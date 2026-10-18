use std::collections::{BTreeMap, HashSet};

use super::text::Pane;
use super::{Key, Location};
use crate::terminal::Position;

const SAFE: &str = "sfnut/SFNLHMUGTZ?";
const LABELS: &str = "sfnjklhodweimbuyvrgtaqpcxz/SFNJKLHODWEIMBUYVRGTAQPCXZ?";

#[derive(Clone)]
struct Target {
    location: Location,
    second: char,
    preview: Position,
    label: Option<char>,
    group: usize,
}

enum Phase {
    First,
    Second {
        first: char,
        targets: Vec<Target>,
    },
    Labels {
        targets: Vec<Target>,
        group: usize,
        automatic: bool,
        selected: usize,
    },
}

#[derive(Clone, Copy)]
pub(super) enum Query {
    One(char),
    Two([char; 2]),
}

pub(super) struct Search {
    pub origin: Location,
    scope: Option<usize>,
    phase: Phase,
    pub query: Option<Query>,
    no_matches: bool,
}

pub(super) enum Action {
    Stay,
    Jump(Location),
    Auto(Location),
    Forward(Key),
    Cancel,
}

fn equivalent(ch: char) -> char {
    match ch {
        ' ' | '\t' | '\r' | '\n' => ' ',
        '(' | '[' | '{' => '(',
        ')' | ']' | '}' => ')',
        '\'' | '"' | '`' => '\'',
        _ => ch,
    }
}

fn matches(actual: char, input: char, sensitive: bool) -> bool {
    let actual = equivalent(actual);
    let input = equivalent(input);
    if sensitive {
        actual == input
    } else {
        actual.to_lowercase().eq(input.to_lowercase())
    }
}

impl Search {
    pub fn new(origin: Location, scope: Option<usize>) -> Self {
        Self {
            origin,
            scope,
            phase: Phase::First,
            query: None,
            no_matches: false,
        }
    }

    fn targets(
        &self,
        panes: &[Pane],
        first: char,
        sensitive: bool,
    ) -> Vec<Target> {
        let cursor = panes[self.origin.pane].screen(self.origin.position);
        let mut targets = Vec::new();
        for (pane_id, pane) in panes.iter().enumerate() {
            if self.scope.is_some_and(|scope| scope != pane_id) {
                continue;
            }
            for (i, glyph) in pane.glyphs.iter().enumerate() {
                let location = Location {
                    pane: pane_id,
                    position: glyph.position,
                };
                if location == self.origin
                    || !matches(glyph.ch, first, sensitive)
                {
                    continue;
                }
                let next =
                    pane.glyphs.get(i + 1).filter(|_| pane.connected(i, i + 1));
                let second = next.map_or('\n', |g| g.ch);
                if matches(second, first, sensitive)
                    && i > 0
                    && pane.connected(i - 1, i)
                    && matches(pane.glyphs[i - 1].ch, first, sensitive)
                {
                    continue;
                }
                let preview_col = next
                    .map_or(glyph.position.col + glyph.width, |g| {
                        g.position.col + g.width
                    });
                let preview = Position {
                    row: next.map_or(glyph.position.row, |g| g.position.row),
                    col: preview_col.min(pane.terminal.cols() - 1),
                };
                targets.push(Target {
                    location,
                    second,
                    preview,
                    label: None,
                    group: 0,
                });
            }
        }
        targets.sort_by(|a, b| {
            let rank = |target: &Target| {
                let p = panes[target.location.pane]
                    .screen(target.location.position);
                let dx = p.col.abs_diff(cursor.col) as f64 * 0.3;
                let dy = p.row.abs_diff(cursor.row) as f64;
                let same_pane = target.location.pane == self.origin.pane;
                let same_line = same_pane && p.row == cursor.row;
                (dx * dx + dy * dy).sqrt()
                    - if same_pane { 30.0 } else { 0.0 }
                    - if same_line { 999.0 } else { 0.0 }
                    - if same_line && p.col > cursor.col {
                        999.0
                    } else {
                        0.0
                    }
            };
            rank(a).total_cmp(&rank(b))
        });
        targets
    }

    fn assign(&self, targets: &mut [Target]) -> bool {
        let automatic = targets.len() <= SAFE.chars().count() + 1;
        let labels: Vec<_> =
            if automatic { SAFE } else { LABELS }.chars().collect();
        for (i, target) in targets.iter_mut().enumerate() {
            if automatic && i == 0 {
                target.label = None;
                target.group = 0;
            } else {
                let i = i - usize::from(automatic);
                target.label = Some(labels[i % labels.len()]);
                target.group = i / labels.len();
            }
        }
        automatic
    }

    fn first(&mut self, panes: &[Pane], first: char) {
        let mut targets = self.targets(panes, first, first.is_uppercase());
        let mut groups: BTreeMap<char, Vec<usize>> = BTreeMap::new();
        for (i, target) in targets.iter().enumerate() {
            let second =
                equivalent(target.second).to_lowercase().next().unwrap();
            groups.entry(second).or_default().push(i);
        }
        for indexes in groups.values() {
            let mut group: Vec<_> =
                indexes.iter().map(|&i| targets[i].clone()).collect();
            self.assign(&mut group);
            for (&index, target) in indexes.iter().zip(group) {
                targets[index] = target;
            }
        }
        self.no_matches = targets.is_empty();
        self.phase = Phase::Second { first, targets };
    }

    fn second(&mut self, first: char, second: char, panes: &[Pane]) -> Action {
        let sensitive = first.is_uppercase() || second.is_uppercase();
        let mut targets: Vec<_> = self
            .targets(panes, first, sensitive)
            .into_iter()
            .filter(|target| matches(target.second, second, sensitive))
            .collect();
        if targets.is_empty() {
            self.no_matches = true;
            self.phase = Phase::First;
            return Action::Stay;
        }
        self.no_matches = false;
        self.query = Some(Query::Two([first, second]));
        if targets.len() == 1 {
            return Action::Jump(targets[0].location);
        }
        let automatic = self.assign(&mut targets);
        let first = targets[0].location;
        self.phase = Phase::Labels {
            targets,
            group: 0,
            automatic,
            selected: 0,
        };
        if automatic {
            Action::Auto(first)
        } else {
            Action::Stay
        }
    }

    pub fn key(
        &mut self,
        key: Key,
        panes: &[Pane],
        last: Option<Query>,
    ) -> Action {
        if key == Key::Escape {
            return Action::Cancel;
        }
        match &mut self.phase {
            Phase::First => match key {
                Key::Char(first) => {
                    self.first(panes, first);
                    Action::Stay
                }
                Key::Enter => match last {
                    Some(Query::Two([first, second])) => {
                        self.second(first, second, panes)
                    }
                    Some(Query::One(first)) => {
                        self.first(panes, first);
                        self.key(Key::Enter, panes, None)
                    }
                    None => Action::Stay,
                },
                _ => Action::Stay,
            },
            Phase::Second { first, targets } => match key {
                Key::Char(second) => {
                    let first = *first;
                    self.second(first, second, panes)
                }
                Key::Enter => {
                    if let Some(target) = targets.first() {
                        self.query = Some(Query::One(*first));
                        Action::Jump(target.location)
                    } else {
                        Action::Stay
                    }
                }
                Key::Backspace => {
                    self.phase = Phase::First;
                    Action::Stay
                }
                _ => Action::Stay,
            },
            Phase::Labels {
                targets,
                group,
                automatic,
                selected,
            } => {
                match key {
                    Key::Char(' ') => {
                        let last = targets.last().unwrap().group;
                        *group = (*group + 1).min(last);
                    }
                    Key::Backspace if *group > 0 => *group -= 1,
                    Key::Enter | Key::Backspace if *automatic => {
                        let next = if key == Key::Enter {
                            (*selected + 1).min(targets.len() - 1)
                        } else {
                            selected.saturating_sub(1)
                        };
                        let label = targets[next].label.take();
                        targets[*selected].label = label;
                        *selected = next;
                        return Action::Auto(targets[*selected].location);
                    }
                    Key::Enter => {
                        return Action::Jump(
                            targets
                                .iter()
                                .find(|t| t.group == *group)
                                .unwrap()
                                .location,
                        );
                    }
                    Key::Char(label) => {
                        if let Some(target) = targets.iter().find(|t| {
                            t.group == *group && t.label == Some(label)
                        }) {
                            return Action::Jump(target.location);
                        }
                        if *automatic {
                            return Action::Forward(key);
                        }
                    }
                    _ if *automatic => return Action::Forward(key),
                    _ => (),
                }
                Action::Stay
            }
        }
    }

    pub fn status(&self) -> String {
        if self.no_matches {
            return "LEAP — no matches; try again".into();
        }
        match &self.phase {
            Phase::First => "LEAP — first character".into(),
            Phase::Second { first, .. } => {
                format!("LEAP {first} — second character")
            }
            Phase::Labels { group, .. } => {
                format!("LEAP — label, group {}", group + 1)
            }
        }
    }

    pub fn beacons(&self, panes: &[Pane]) -> Vec<(Location, Option<char>)> {
        let (targets, preview, group, selected) = match &self.phase {
            Phase::First => return Vec::new(),
            Phase::Second { targets, .. } => (targets, true, 0, None),
            Phase::Labels {
                targets,
                group,
                automatic,
                selected,
            } => (
                targets,
                false,
                *group,
                automatic.then_some(targets[*selected].location),
            ),
        };
        let mut result = Vec::new();
        let mut occupied = HashSet::new();
        for target in targets {
            if target.group != group || Some(target.location) == selected {
                continue;
            }
            result.push((target.location, None));
        }
        for target in targets {
            if target.group != group || Some(target.location) == selected {
                continue;
            }
            let Some(label) = target.label else {
                continue;
            };
            let position = if preview {
                target.preview
            } else {
                target.location.position
            };
            let pane = &panes[target.location.pane];
            let position = pane.normalize(position);
            let location = Location {
                pane: target.location.pane,
                position,
            };
            if preview && targets.iter().any(|t| t.location == location) {
                continue;
            }
            if occupied.insert((location.pane, position.row, position.col)) {
                result.push((location, Some(label)));
            }
        }
        result
    }
}
