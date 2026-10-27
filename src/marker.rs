//! On-demand copy targets from a frozen, pane-aware viewport.
use serde::Deserialize;

use crate::terminal::{Color, Content, Position, Terminal};
use crate::tmux::PaneText;
use crate::vim::{Action, Key};

mod labels;
mod recognize;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub label_background: crate::config::Color,
    pub label_embolden: crate::config::FontEmbolden,
    pub label_opacity: crate::config::Opacity,
    pub indicator_opacity: crate::config::Opacity,
    pub styled_min_length: std::num::NonZeroUsize,
    pub hash_min_length: std::num::NonZeroUsize,
    pub overlap_max: std::num::NonZeroUsize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            label_background: crate::config::Color(0xfef991),
            label_embolden: 0.3.try_into().unwrap(),
            label_opacity: 0.8.try_into().unwrap(),
            indicator_opacity: 0.85.try_into().unwrap(),
            styled_min_length: 5.try_into().unwrap(),
            hash_min_length: 7.try_into().unwrap(),
            overlap_max: 2.try_into().unwrap(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: Position,
    pub columns: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub text: String,
    pub segments: Vec<Segment>,
    pub label: String,
}

pub struct Session {
    pub screen: Terminal,
    base: Terminal,
    panes: Vec<PaneText>,
    targets: Vec<Target>,
    prefix: String,
}

impl Session {
    pub fn new(
        screen: Terminal,
        panes: Vec<PaneText>,
        options: &Options,
    ) -> Self {
        let panes = if panes.is_empty() {
            vec![PaneText {
                origin: Position { row: 0, col: 0 },
                terminal: screen.visible_snapshot(),
            }]
        } else {
            panes
        };
        let mut targets = Vec::new();
        for pane in &panes {
            targets.extend(recognize::targets(pane, &screen, options));
        }
        let labels = labels::generate(targets.len());
        for (target, label) in targets.iter_mut().zip(labels) {
            target.label = label;
        }
        Self {
            base: screen.visible_snapshot(),
            screen,
            panes,
            targets,
            prefix: String::new(),
        }
    }

    pub fn starts_leap(&self, key: Key) -> bool {
        key == Key::Char('s') && self.prefix.is_empty()
    }

    pub fn into_vim(self) -> crate::vim::Session {
        crate::vim::Session::new(self.base, self.panes)
    }

    fn render(&mut self) {
        self.screen = self.base.visible_snapshot();
        self.screen.vim_mode = true;
        self.screen.cursor_visible = false;
        if self.prefix.is_empty() {
            return;
        }
        for (row, cells) in self.base.rows.iter().enumerate() {
            for (col, cell) in cells.iter().enumerate() {
                if !matches!(cell.content, Content::Text { .. }) {
                    continue;
                }
                let mut style = cell.style();
                let (fg, bg) = self.base.colors(style);
                let blend = |shift: u32| {
                    let fg = (fg >> shift) & 255;
                    let bg = (bg >> shift) & 255;
                    ((fg * 3 + bg * 2) / 5) as u8
                };
                let faded = Color::Rgb(blend(16), blend(8), blend(0));
                if style.inverse ^ self.base.reverse_video {
                    style.bg = faded;
                } else {
                    style.fg = faded;
                }
                style.dim = false;
                self.screen.overlay(Position { row, col }, None, style);
            }
        }
        for target in self
            .targets
            .iter()
            .filter(|t| t.label.starts_with(&self.prefix))
        {
            for segment in &target.segments {
                let row = segment.start.row;
                for col in
                    segment.start.col..segment.start.col + segment.columns
                {
                    let style = self.base.rows[row][col].style();
                    self.screen.overlay(Position { row, col }, None, style);
                }
            }
        }
    }

    pub fn visible_targets(&self) -> Vec<Target> {
        self.targets
            .iter()
            .filter(|t| t.label.starts_with(&self.prefix))
            .cloned()
            .collect()
    }

    pub fn status(&self) -> String {
        if self.targets.is_empty() {
            "MARK — no targets · s to Leap · Esc to exit".into()
        } else if self.prefix.is_empty() {
            format!("MARK — {} targets · s to Leap", self.targets.len())
        } else {
            format!("MARK — {} targets · {}", self.targets.len(), self.prefix)
        }
    }

    pub fn key(&mut self, key: Key) -> Action {
        let previous = self.prefix.clone();
        match key {
            Key::Escape => return Action::Exit,
            Key::Backspace => {
                self.prefix.pop();
            }
            Key::Char(ch) if ch.is_ascii_alphabetic() => {
                let mut next = self.prefix.clone();
                next.push(ch.to_ascii_lowercase());
                if let Some(target) =
                    self.targets.iter().find(|target| target.label == next)
                {
                    return Action::Copy(target.text.clone());
                }
                if self.targets.iter().any(|t| t.label.starts_with(&next)) {
                    self.prefix = next;
                }
            }
            _ => (),
        }
        if self.prefix != previous {
            self.render();
        }
        Action::Stay
    }
}
