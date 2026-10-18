use crate::terminal::{Content, Position, Terminal, TextRange};
use crate::tmux::PaneText;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SelectionKind {
    Character,
    Line,
    Block,
}

#[derive(Clone, Copy)]
pub(super) struct Selection {
    pub anchor: Position,
    pub kind: SelectionKind,
}

pub(super) struct Glyph {
    pub position: Position,
    pub ch: char,
    pub width: usize,
}

pub(super) struct Pane {
    pub origin: Position,
    pub terminal: Terminal,
    pub glyphs: Vec<Glyph>,
}

impl From<PaneText> for Pane {
    fn from(text: PaneText) -> Self {
        let mut glyphs = Vec::new();
        for (row, cells) in text.terminal.rows.iter().enumerate() {
            let end = cells.wrapped.unwrap_or_else(|| {
                cells
                    .iter()
                    .rposition(|cell| match &cell.content {
                        Content::Text { text } => {
                            text.chars().any(|c| c != ' ')
                        }
                        Content::Continuation => true,
                    })
                    .map_or(1, |col| col + 1)
            });
            for col in 0..end {
                if let Content::Text { text } = &cells[col].content {
                    glyphs.push(Glyph {
                        position: Position { row, col },
                        ch: text.chars().next().unwrap(),
                        width: text.columns(),
                    });
                }
            }
        }
        Self {
            origin: text.origin,
            terminal: text.terminal,
            glyphs,
        }
    }
}

impl Pane {
    pub fn contains(&self, screen: Position) -> bool {
        screen.row >= self.origin.row
            && screen.col >= self.origin.col
            && screen.row < self.origin.row + self.terminal.rows.len()
            && screen.col < self.origin.col + self.terminal.cols()
    }

    pub fn screen(&self, local: Position) -> Position {
        Position {
            row: self.origin.row + local.row,
            col: self.origin.col + local.col,
        }
    }

    pub fn normalize(&self, position: Position) -> Position {
        let row = position.row.min(self.terminal.rows.len() - 1);
        self.glyphs
            .iter()
            .filter(|g| g.position.row == row)
            .take_while(|g| g.position.col <= position.col)
            .last()
            .map_or(Position { row, col: 0 }, |g| g.position)
    }

    pub fn index(&self, position: Position) -> usize {
        self.glyphs
            .binary_search_by_key(&position, |g| g.position)
            .unwrap_or_else(|i| i.min(self.glyphs.len() - 1))
    }

    pub fn connected(&self, a: usize, b: usize) -> bool {
        let first = self.glyphs[a].position;
        let next = self.glyphs[b].position;
        first.row == next.row
            || (next.row == first.row + 1
                && self.terminal.rows[first.row].wrapped.is_some())
    }

    fn class(&self, index: usize, big: bool) -> u8 {
        let ch = self.glyphs[index].ch;
        if ch.is_whitespace() {
            0
        } else if big || ch.is_alphanumeric() || ch == '_' {
            1
        } else {
            2
        }
    }

    fn word(&self, mut i: usize, motion: char) -> usize {
        let big = motion.is_ascii_uppercase();
        let same = |a, b| {
            self.class(a, big) == self.class(b, big) && self.connected(a, b)
        };
        let last = self.glyphs.len() - 1;
        match motion.to_ascii_lowercase() {
            'w' => {
                while i < last && same(i, i + 1) {
                    i += 1;
                }
                i = (i + 1).min(last);
                while i < last && self.class(i, big) == 0 {
                    i += 1;
                }
            }
            'b' => {
                i = i.saturating_sub(1);
                while i > 0 && self.class(i, big) == 0 {
                    i -= 1;
                }
                while i > 0 && same(i - 1, i) {
                    i -= 1;
                }
            }
            'e' => {
                i = (i + 1).min(last);
                while i < last && self.class(i, big) == 0 {
                    i += 1;
                }
                while i < last && same(i, i + 1) {
                    i += 1;
                }
            }
            _ => unreachable!(),
        }
        i
    }

    pub fn logical_rows(&self, row: usize) -> (usize, usize) {
        let mut first = row;
        let mut last = row;
        while first > 0 && self.terminal.rows[first - 1].wrapped.is_some() {
            first -= 1;
        }
        while last + 1 < self.terminal.rows.len()
            && self.terminal.rows[last].wrapped.is_some()
        {
            last += 1;
        }
        (first, last)
    }

    pub fn motion(
        &self,
        position: Position,
        key: char,
        count: usize,
        preferred: &mut Option<usize>,
    ) -> Option<Position> {
        let count = count.min(self.glyphs.len());
        if matches!(key, 'j' | 'k') {
            let col = *preferred.get_or_insert(position.col);
            let row = if key == 'j' {
                position
                    .row
                    .saturating_add(count)
                    .min(self.terminal.rows.len() - 1)
            } else {
                position.row.saturating_sub(count)
            };
            return Some(self.normalize(Position { row, col }));
        }
        *preferred = None;
        let mut i = self.index(position);
        let (first, last) = self.logical_rows(position.row);
        match key {
            'h' | 'l' => {
                for _ in 0..count {
                    let next = if key == 'h' {
                        i.saturating_sub(1)
                    } else {
                        (i + 1).min(self.glyphs.len() - 1)
                    };
                    if !self.connected(i.min(next), i.max(next)) {
                        break;
                    }
                    i = next;
                }
            }
            'w' | 'b' | 'e' | 'W' | 'B' | 'E' => {
                for _ in 0..count {
                    i = self.word(i, key);
                }
            }
            '0' => return Some(Position { row: first, col: 0 }),
            '^' | 'H' => {
                return Some(
                    self.glyphs
                        .iter()
                        .find(|g| {
                            g.position.row >= first
                                && g.position.row <= last
                                && !g.ch.is_whitespace()
                        })
                        .map_or(Position { row: first, col: 0 }, |g| {
                            g.position
                        }),
                );
            }
            '$' | 'L' => {
                let mut row = last;
                for _ in 1..count {
                    if row + 1 == self.terminal.rows.len() {
                        break;
                    }
                    row = self.logical_rows(row + 1).1;
                }
                return Some(self.normalize(Position {
                    row,
                    col: usize::MAX,
                }));
            }
            _ => return None,
        }
        Some(self.glyphs[i].position)
    }

    pub fn ranges(
        &self,
        selection: Selection,
        cursor: Position,
    ) -> Vec<TextRange> {
        let a = selection.anchor.min(cursor);
        let b = selection.anchor.max(cursor);
        match selection.kind {
            SelectionKind::Character => vec![TextRange::between(
                a,
                Position {
                    row: b.row,
                    col: b.col + self.glyphs[self.index(b)].width,
                },
            )],
            SelectionKind::Line => vec![TextRange::between(
                Position {
                    row: self.logical_rows(a.row).0,
                    col: 0,
                },
                Position {
                    row: self.logical_rows(b.row).1 + 1,
                    col: 0,
                },
            )],
            SelectionKind::Block => {
                let left = selection.anchor.col.min(cursor.col);
                let right = selection.anchor.col.max(cursor.col) + 1;
                (a.row..=b.row)
                    .map(|row| {
                        self.terminal.selection_between(
                            Position { row, col: left },
                            Position { row, col: right },
                        )
                    })
                    .collect()
            }
        }
    }

    pub fn copy(&self, selection: Selection, cursor: Position) -> String {
        let ranges = self.ranges(selection, cursor);
        ranges
            .into_iter()
            .map(|range| self.terminal.selected_text(range))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
