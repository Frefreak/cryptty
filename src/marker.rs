//! On-demand copy targets from a frozen, pane-aware viewport.
use serde::Deserialize;

use crate::terminal::{Position, Terminal};
use crate::tmux::PaneText;
use crate::vim::{Action, Key};

mod recognize;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub styled_min_length: std::num::NonZeroUsize,
    pub hash_min_length: std::num::NonZeroUsize,
    pub overlap_max: std::num::NonZeroUsize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
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
        for pane in panes {
            targets.extend(recognize::targets(&pane, &screen, options));
        }
        // Fixed-width labels remain prefix-free at every target count.
        let alphabet = b"asdfghjklqwertyuiopzxcvbnm";
        let mut digits = 1;
        let mut capacity = alphabet.len();
        while capacity < targets.len() {
            digits += 1;
            capacity = capacity.saturating_mul(alphabet.len());
        }
        for (index, target) in targets.iter_mut().enumerate() {
            let mut n = index;
            let mut label = vec![b'a'; digits];
            for ch in label.iter_mut().rev() {
                *ch = alphabet[n % alphabet.len()];
                n /= alphabet.len();
            }
            target.label = String::from_utf8(label).unwrap();
        }
        Self {
            screen,
            targets,
            prefix: String::new(),
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
            "MARK — no targets · Esc to exit".into()
        } else {
            format!("MARK — {} targets · {}", self.targets.len(), self.prefix)
        }
    }

    pub fn key(&mut self, key: Key) -> Action {
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
        Action::Stay
    }
}
