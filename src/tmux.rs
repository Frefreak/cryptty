//! On-demand, pane-local text snapshots from a local tmux server.

use std::path::Path;

use anyhow::{Context, Result};

use crate::terminal::{Content, Link, Position, Terminal, TextRange};

mod client;
mod integration;
#[cfg(test)]
mod tests;

use client::{Control, quote};
pub use integration::Integration;

const CLIENT_FORMAT: &str = concat!(
    "#{client_tty}\t#{session_id}\t#{window_id}\t",
    "#{client_width}\t#{client_height}\t",
    "#{window_width}\t#{window_height}\t",
    "#{status}\t#{status-position}\t#{pane-border-status}\t",
    "#{window_zoomed_flag}",
);
const PANE_FORMAT: &str = concat!(
    "#{pane_id}\t#{pane_left}\t#{pane_top}\t",
    "#{pane_width}\t#{pane_height}\t#{pane_in_mode}\t#{pane_active}",
);

#[derive(Debug, PartialEq, Eq)]
struct Client {
    session: String,
    window: String,
    cols: usize,
    rows: usize,
    window_cols: usize,
    window_rows: usize,
    status_rows: usize,
    status_top: bool,
    border_status: bool,
    zoomed: bool,
}

impl Client {
    fn find(text: &str, tty: &Path) -> Option<Self> {
        let fields = text.lines().find_map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            (fields.len() == 11 && Path::new(fields[0]) == tty)
                .then_some(fields)
        })?;
        Some(Self {
            session: identifier(fields[1], '$')?,
            window: identifier(fields[2], '@')?,
            cols: fields[3].parse().ok()?,
            rows: fields[4].parse().ok()?,
            window_cols: fields[5].parse().ok()?,
            window_rows: fields[6].parse().ok()?,
            status_rows: match fields[7] {
                "on" => 1,
                "off" => 0,
                value => value.parse().ok()?,
            },
            status_top: fields[8] == "top",
            border_status: fields[9] != "off",
            zoomed: fields[10] == "1",
        })
    }

    fn fits(&self, size: (usize, usize)) -> bool {
        (self.cols, self.rows) == size
            && self.window_cols == self.cols
            && self.window_rows.checked_add(self.status_rows) == Some(self.rows)
            && !self.border_status
    }
}

fn identifier(value: &str, prefix: char) -> Option<String> {
    let digits = value.strip_prefix(prefix)?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.to_owned())
}

#[derive(Debug, PartialEq, Eq)]
pub struct ScreenLink {
    pub uri: String,
    pub ranges: Vec<TextRange>,
}

impl From<Link> for ScreenLink {
    fn from(link: Link) -> Self {
        Self {
            uri: link.uri,
            ranges: vec![link.range],
        }
    }
}

pub enum TextSnapshot {
    Pending,
    Unavailable,
    Changed,
    Failed,
    Ready(Vec<PaneText>),
}

pub enum Lookup {
    Pending,
    Unavailable,
    Ready(Option<ScreenLink>),
}

pub struct PaneText {
    pub origin: Position,
    pub terminal: Terminal,
}

impl PaneText {
    fn local_position(&self, position: Position) -> Option<Position> {
        let row = position.row.checked_sub(self.origin.row)?;
        let col = position.col.checked_sub(self.origin.col)?;
        (row < self.terminal.rows.len() && col < self.terminal.cols())
            .then_some(Position { row, col })
    }

    fn matches(&self, display: &Terminal) -> bool {
        self.terminal.rows.iter().enumerate().all(|(y, row)| {
            display.rows.get(self.origin.row + y).is_some_and(|screen| {
                row.iter().enumerate().all(|(x, cell)| {
                    screen
                        .get(self.origin.col + x)
                        .is_some_and(|other| cell.content == other.content)
                })
            })
        })
    }

    fn screen_link(&self, link: Link) -> ScreenLink {
        let mut ranges = Vec::new();
        for row in link.range.start.row..=link.range.end.row {
            let start = if row == link.range.start.row {
                link.range.start.col
            } else {
                0
            };
            let end = if row == link.range.end.row {
                link.range.end.col
            } else {
                self.terminal.rows[row]
                    .wrapped
                    .unwrap_or(self.terminal.cols())
            };
            if start < end {
                ranges.push(TextRange::between(
                    Position {
                        row: self.origin.row + row,
                        col: self.origin.col + start,
                    },
                    Position {
                        row: self.origin.row + row,
                        col: self.origin.col + end,
                    },
                ));
            }
        }
        ScreenLink {
            uri: link.uri,
            ranges,
        }
    }
}

pub struct Snapshot {
    complete: bool,
    size: (usize, usize),
    pub panes: Vec<PaneText>,
}

impl Snapshot {
    fn text(&self, display: &Terminal) -> TextSnapshot {
        if !self.complete
            || self.size != (display.cols(), display.rows.len())
            || self.panes.is_empty()
            || !self.panes.iter().all(|pane| pane.matches(display))
        {
            return TextSnapshot::Changed;
        }
        TextSnapshot::Ready(
            self.panes
                .iter()
                .map(|pane| PaneText {
                    origin: pane.origin,
                    terminal: pane.terminal.visible_snapshot(),
                })
                .collect(),
        )
    }

    fn capture(
        connection: &mut Control,
        tty: &Path,
        size: (usize, usize),
        current: &impl Fn() -> bool,
    ) -> Result<Self> {
        anyhow::ensure!(current(), "tmux request canceled");
        let clients = format!("list-clients -F {}", quote(CLIENT_FORMAT));
        let text = connection.query(std::slice::from_ref(&clients))?.remove(0);
        let client =
            Client::find(&text, tty).context("tmux client detached")?;
        anyhow::ensure!(client.fits(size), "unsupported tmux viewport");
        let pane_command = format!(
            "list-panes -t {} -F {}",
            quote(&client.window),
            quote(PANE_FORMAT),
        );
        anyhow::ensure!(current(), "tmux request canceled");
        let metadata = connection
            .query(std::slice::from_ref(&pane_command))?
            .remove(0);
        let mut complete = true;
        let mut bounds = Vec::new();
        let mut commands = Vec::new();
        for line in metadata.lines() {
            let fields: Vec<_> = line.split('\t').collect();
            anyhow::ensure!(fields.len() == 7, "invalid tmux pane metadata");
            if client.zoomed && fields[6] != "1" {
                continue;
            }
            if fields[5] != "0" {
                complete = false;
                continue;
            }
            let id = identifier(fields[0], '%').context("invalid pane id")?;
            let left: usize = fields[1].parse()?;
            let top: usize = fields[2].parse()?;
            let width: usize = fields[3].parse()?;
            let height: usize = fields[4].parse()?;
            anyhow::ensure!(
                width > 0
                    && height > 0
                    && left.checked_add(width).is_some_and(|n| n <= size.0)
                    && top
                        .checked_add(height)
                        .is_some_and(|n| n <= client.window_rows),
                "invalid tmux pane bounds",
            );
            let status = if client.status_top {
                client.status_rows
            } else {
                0
            };
            bounds.push((
                Position {
                    row: top + status,
                    col: left,
                },
                width,
                height,
            ));
            commands.push(format!("capture-pane -p -N -t {id}"));
            commands.push(format!("capture-pane -p -J -t {id}"));
        }
        commands.push(pane_command);
        commands.push(clients);
        anyhow::ensure!(current(), "tmux request canceled");
        let mut replies = connection.query(&commands)?;
        let latest = replies.pop().unwrap();
        anyhow::ensure!(
            Client::find(&latest, tty).as_ref() == Some(&client)
                && replies.pop().as_ref() == Some(&metadata),
            "tmux layout changed during capture",
        );
        let mut panes = Vec::new();
        for ((origin, width, height), captures) in
            bounds.into_iter().zip(replies.chunks_exact(2))
        {
            if let Some(terminal) =
                pane_terminal(width, height, &captures[0], &captures[1])
            {
                panes.push(PaneText { origin, terminal });
            } else {
                complete = false;
            }
        }
        Ok(Self {
            size,
            panes,
            complete,
        })
    }

    pub fn lookup(&self, display: &Terminal, position: Position) -> Lookup {
        if self.size != (display.cols(), display.rows.len()) {
            return Lookup::Unavailable;
        }
        for pane in &self.panes {
            if let Some(local) = pane.local_position(position) {
                if !pane.matches(display) {
                    return Lookup::Unavailable;
                }
                return Lookup::Ready(
                    pane.terminal
                        .link_at(local)
                        .map(|link| pane.screen_link(link)),
                );
            }
        }
        Lookup::Unavailable
    }
}

fn pane_terminal(
    width: usize,
    height: usize,
    physical: &str,
    logical: &str,
) -> Option<Terminal> {
    let mut terminal = Terminal::new(width, height);
    let logical = logical.strip_suffix('\n')?;
    terminal.feed(logical.replace('\n', "\r\n").as_bytes());
    let lines: Vec<_> = physical.strip_suffix('\n')?.split('\n').collect();
    if lines.len() != height {
        return None;
    }
    for (row, line) in terminal.rows.iter().zip(lines) {
        let text: String = row
            .iter()
            .filter_map(|cell| match &cell.content {
                Content::Text { text } => Some(text.chars()),
                _ => None,
            })
            .flatten()
            .collect();
        if text.trim_end_matches(' ') != line.trim_end_matches(' ') {
            return None;
        }
    }
    Some(terminal)
}
