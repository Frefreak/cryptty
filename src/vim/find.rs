use super::text::{Pane, Selection, SelectionKind};
use super::{Action, Session};
use crate::terminal::Position;

#[derive(Clone, Copy)]
pub(super) enum FindKind {
    Forward,
    Backward,
    TillForward,
    TillBackward,
}

impl FindKind {
    pub fn from_key(key: char) -> Option<Self> {
        Some(match key {
            'f' => Self::Forward,
            'F' => Self::Backward,
            't' => Self::TillForward,
            'T' => Self::TillBackward,
            _ => return None,
        })
    }

    pub fn key(self) -> char {
        match self {
            Self::Forward => 'f',
            Self::Backward => 'F',
            Self::TillForward => 't',
            Self::TillBackward => 'T',
        }
    }

    pub fn reverse(self) -> Self {
        match self {
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
            Self::TillForward => Self::TillBackward,
            Self::TillBackward => Self::TillForward,
        }
    }

    fn backward(self) -> bool {
        matches!(self, Self::Backward | Self::TillBackward)
    }

    fn till(self) -> bool {
        matches!(self, Self::TillForward | Self::TillBackward)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Find {
    pub kind: FindKind,
    pub target: char,
}

impl Pane {
    fn find(
        &self,
        position: Position,
        find: Find,
        mut count: usize,
        repeat: bool,
    ) -> Option<Position> {
        let origin = self.index(position);
        let mut i = origin;
        let (first, last) = self.logical_rows(position.row);
        loop {
            i = if find.kind.backward() {
                i.checked_sub(1)?
            } else {
                i + 1
            };
            let glyph = self.glyphs.get(i)?;
            if glyph.position.row < first || glyph.position.row > last {
                return None;
            }
            if glyph.ch != find.target {
                continue;
            }
            let destination = if find.kind.till() {
                if find.kind.backward() { i + 1 } else { i - 1 }
            } else {
                i
            };
            // Repeating a till motion must pass its adjacent target.
            if repeat && destination == origin {
                continue;
            }
            count -= 1;
            if count == 0 {
                return Some(self.glyphs[destination].position);
            }
        }
    }
}

impl Session {
    pub(super) fn find(
        &mut self,
        find: Find,
        count: usize,
        repeat: bool,
        operator: bool,
    ) -> Action {
        let pane = &self.panes[self.cursor.pane];
        let Some(next) = pane.find(self.cursor.position, find, count, repeat)
        else {
            return Action::Stay;
        };
        self.preferred_column = None;
        if operator {
            let mut anchor = self.cursor.position;
            if find.kind.backward() {
                if next == anchor {
                    return Action::Stay;
                }
                anchor =
                    pane.glyphs[pane.index(anchor).saturating_sub(1)].position;
            }
            return Action::Copy(pane.copy(
                Selection {
                    anchor,
                    kind: SelectionKind::Character,
                },
                next,
            ));
        }
        self.cursor.position = next;
        Action::Stay
    }
}
