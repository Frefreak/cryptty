//! Frozen, visible-text navigation. Coordinates are local to one pane.

use std::time::Instant;

use crate::terminal::{Color, CursorShape, CursorStyle, Position, Terminal};
use crate::tmux::PaneText;

mod find;
mod leap;
#[cfg(test)]
mod tests;
mod text;

use find::{Find, FindKind};
use text::{Pane, Selection, SelectionKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Control(char),
    Enter,
    Escape,
    Backspace,
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Location {
    pane: usize,
    position: Position,
}

enum Phase {
    Navigate,
    Leap(leap::Search),
    Yank(usize),
    Find {
        kind: FindKind,
        count: usize,
        operator: bool,
    },
}

pub enum Action {
    Stay,
    Exit,
    Copy(String),
}

pub struct Session {
    base: Terminal,
    pub screen: Terminal,
    panes: Vec<Pane>,
    cursor: Location,
    selection: Option<Selection>,
    phase: Phase,
    count: usize,
    preferred_column: Option<usize>,
    last_query: Option<leap::Query>,
    last_find: Option<Find>,
}

impl Session {
    pub fn new(base: Terminal, panes: Vec<PaneText>) -> Self {
        let panes: Vec<Pane> = if panes.is_empty() {
            vec![PaneText {
                origin: Position { row: 0, col: 0 },
                terminal: base.visible_snapshot(),
            }]
        } else {
            panes
        }
        .into_iter()
        .map(Pane::from)
        .collect();
        let screen = base.visible_snapshot();
        let screen_cursor = Position {
            row: base.cursor.1,
            col: base.cursor.0,
        };
        let pane = panes
            .iter()
            .position(|p| p.contains(screen_cursor))
            .unwrap_or(0);
        let origin = panes[pane].origin;
        let position = panes[pane].normalize(Position {
            row: screen_cursor.row.saturating_sub(origin.row),
            col: screen_cursor.col.saturating_sub(origin.col),
        });
        let cursor = Location { pane, position };
        let mut session = Self {
            base,
            screen,
            panes,
            cursor,
            selection: None,
            phase: Phase::Leap(leap::Search::new(cursor, None)),
            count: 0,
            preferred_column: None,
            last_query: None,
            last_find: None,
        };
        session.render();
        session
    }

    pub fn status(&self) -> String {
        let mode = match &self.phase {
            Phase::Leap(search) => search.status(),
            Phase::Yank(_) => "YANK — motion".into(),
            Phase::Find { kind, .. } => {
                format!("{} — character", kind.key())
            }
            Phase::Navigate => match self.selection.map(|s| s.kind) {
                Some(SelectionKind::Character) => "CHARACTER".into(),
                Some(SelectionKind::Line) => "LINE".into(),
                Some(SelectionKind::Block) => "BLOCK".into(),
                None => "NORMAL".into(),
            },
        };
        let count = if self.count == 0 {
            String::new()
        } else {
            format!(" — {}", self.count)
        };
        format!("VIM — {mode}{count}")
    }

    pub fn key(&mut self, key: Key, _now: Instant) -> Action {
        let action = self.handle(key);
        self.render();
        action
    }

    fn handle(&mut self, key: Key) -> Action {
        if let Phase::Leap(search) = &mut self.phase {
            let action = search.key(key, &self.panes, self.last_query);
            if search.query.is_some() {
                self.last_query = search.query;
            }
            match action {
                leap::Action::Stay => return Action::Stay,
                leap::Action::Auto(location) => {
                    self.cursor = location;
                    return Action::Stay;
                }
                leap::Action::Jump(location) => {
                    self.cursor = location;
                    self.phase = Phase::Navigate;
                    self.preferred_column = None;
                    return Action::Stay;
                }
                leap::Action::Cancel => {
                    self.cursor = search.origin;
                    self.phase = Phase::Navigate;
                    return Action::Stay;
                }
                leap::Action::Forward(key) => {
                    self.phase = Phase::Navigate;
                    return self.handle(key);
                }
            }
        }
        if let Phase::Find {
            kind,
            count,
            operator,
        } = self.phase
        {
            self.phase = Phase::Navigate;
            if let Key::Char(target) = key {
                let find = Find { kind, target };
                self.last_find = Some(find);
                return self.find(find, count, false, operator);
            }
            return Action::Stay;
        }
        if key == Key::Escape {
            self.count = 0;
            if matches!(self.phase, Phase::Yank(_)) {
                self.phase = Phase::Navigate;
            } else if self.selection.take().is_none() {
                return Action::Exit;
            }
            return Action::Stay;
        }
        if let Key::Char(ch @ '0'..='9') = key
            && (ch != '0' || self.count != 0)
        {
            self.count =
                (self.count * 10 + ch.to_digit(10).unwrap() as usize).min(9999);
            return Action::Stay;
        }
        let count = std::mem::take(&mut self.count).max(1);
        if key == Key::Char('s') && matches!(self.phase, Phase::Navigate) {
            self.phase = Phase::Leap(leap::Search::new(
                self.cursor,
                self.selection.map(|_| self.cursor.pane),
            ));
            self.preferred_column = None;
            return Action::Stay;
        }
        let kind = match key {
            Key::Char('v') => Some(SelectionKind::Block),
            Key::Control('v') => Some(SelectionKind::Character),
            Key::Char('V') => Some(SelectionKind::Line),
            _ => None,
        };
        if let Some(kind) = kind {
            self.phase = Phase::Navigate;
            self.selection = match self.selection {
                Some(selection) if selection.kind == kind => None,
                Some(selection) => Some(Selection { kind, ..selection }),
                None => Some(Selection {
                    anchor: self.cursor.position,
                    kind,
                }),
            };
            return Action::Stay;
        }
        if key == Key::Char('y') {
            let pane = &self.panes[self.cursor.pane];
            if let Some(selection) = self.selection {
                return Action::Copy(
                    pane.copy(selection, self.cursor.position),
                );
            }
            if let Phase::Yank(operator_count) = self.phase {
                let mut end = self.cursor.position;
                for _ in 1..(count * operator_count).min(pane.glyphs.len()) {
                    end.row = (pane.logical_rows(end.row).1 + 1)
                        .min(pane.terminal.rows.len() - 1);
                }
                return Action::Copy(pane.copy(
                    Selection {
                        anchor: self.cursor.position,
                        kind: SelectionKind::Line,
                    },
                    end,
                ));
            }
            self.phase = Phase::Yank(count);
            return Action::Stay;
        }
        let motion = match key {
            Key::Char(ch) => ch,
            Key::Left => 'h',
            Key::Right => 'l',
            Key::Up => 'k',
            Key::Down => 'j',
            _ => return Action::Stay,
        };
        let operator = match self.phase {
            Phase::Yank(n) => Some(n),
            _ => None,
        };
        if let Some(kind) = FindKind::from_key(motion) {
            self.phase = Phase::Find {
                kind,
                count: count * operator.unwrap_or(1),
                operator: operator.is_some(),
            };
            return Action::Stay;
        }
        if matches!(motion, ';' | ',') {
            self.phase = Phase::Navigate;
            if let Some(mut find) = self.last_find {
                if motion == ',' {
                    find.kind = find.kind.reverse();
                }
                return self.find(
                    find,
                    count * operator.unwrap_or(1),
                    true,
                    operator.is_some(),
                );
            }
            return Action::Stay;
        }
        let pane = &self.panes[self.cursor.pane];
        let Some(next) = pane.motion(
            self.cursor.position,
            motion,
            count * operator.unwrap_or(1),
            &mut self.preferred_column,
        ) else {
            self.phase = Phase::Navigate;
            return Action::Stay;
        };
        if operator.is_some() {
            let mut selection = Selection {
                anchor: self.cursor.position,
                kind: SelectionKind::Character,
            };
            if matches!(motion, 'j' | 'k') {
                selection.kind = SelectionKind::Line;
            }
            let mut end = next;
            if matches!(motion, 'w' | 'W' | 'l') && next > selection.anchor {
                end = pane.glyphs[pane.index(next).saturating_sub(1)].position;
            } else if matches!(motion, 'b' | 'B' | 'h' | '0' | '^' | 'H')
                && next < selection.anchor
            {
                selection.anchor = pane.glyphs
                    [pane.index(selection.anchor).saturating_sub(1)]
                .position;
            }
            return Action::Copy(pane.copy(selection, end));
        }
        self.cursor.position = next;
        Action::Stay
    }

    fn render(&mut self) {
        self.screen.rows = self.base.rows.clone();
        self.screen.vim_mode = true;
        self.screen.selection = None;
        self.screen.hovered_link.clear();
        let pane = &self.panes[self.cursor.pane];
        let position = pane.screen(self.cursor.position);
        self.screen.cursor = (position.col, position.row);
        self.screen.cursor_visible = true;
        self.screen.cursor_blink_on = true;
        self.screen.cursor_style = CursorStyle {
            shape: CursorShape::Block,
            blinking: false,
        };
        if let Some(selection) = self.selection {
            for range in pane.ranges(selection, self.cursor.position) {
                for row in range.start.row
                    ..=range.end.row.min(pane.terminal.rows.len() - 1)
                {
                    let start = if row == range.start.row {
                        range.start.col
                    } else {
                        0
                    };
                    let end = if row == range.end.row {
                        range.end.col
                    } else {
                        pane.terminal.cols()
                    };
                    for col in start..end {
                        let position = pane.screen(Position { row, col });
                        let mut style =
                            self.base.rows[position.row][position.col].style();
                        style.inverse = !style.inverse;
                        self.screen.overlay(position, None, style);
                    }
                }
            }
        }
        if let Phase::Leap(search) = &self.phase {
            for (location, label) in search.beacons(&self.panes) {
                let position =
                    self.panes[location.pane].screen(location.position);
                let mut style =
                    self.base.rows[position.row][position.col].style();
                style.fg = Color::Rgb(0x28, 0x2a, 0x36);
                style.bg = if label.is_some() {
                    Color::Rgb(0xff, 0x79, 0xc6)
                } else {
                    Color::Rgb(0x50, 0xfa, 0x7b)
                };
                style.bold = true;
                style.inverse = false;
                self.screen.overlay(position, label, style);
            }
        }
    }
}
