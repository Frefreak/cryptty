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
    "#{pane_width}\t#{pane_height}\t#{pane_mode}\t#{pane_active}\t",
    "#{scroll_position}",
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

enum CapturedPane {
    Application(PaneText),
    CopyMode(PaneText),
}

impl CapturedPane {
    fn pane(&self) -> &PaneText {
        match self {
            Self::Application(pane) | Self::CopyMode(pane) => pane,
        }
    }

    fn visible(&self, display: &Terminal) -> Option<PaneText> {
        let pane = self.pane();
        let application = matches!(self, Self::Application(_));
        if application && !pane.matches(display) {
            return None;
        }
        let mut terminal = pane.terminal.visible_snapshot();
        let mut matching = Vec::new();
        for (y, row) in terminal.rows.iter_mut().enumerate() {
            let source = display.rows.get(pane.origin.row + y)?;
            let mut same = true;
            for x in 0..row.len() {
                let visible = source.get(pane.origin.col + x)?;
                same &= row[x].content == visible.content;
                row[x].clone_from(visible);
            }
            matching.push(same);
        }
        // Copy-mode headers and prompts replace backing text on screen.
        for (y, row) in terminal.rows.iter_mut().enumerate() {
            if !application
                && (!matching[y]
                    || !matching.get(y + 1).copied().unwrap_or(false))
            {
                row.wrapped = None;
            }
        }
        Some(PaneText {
            origin: pane.origin,
            terminal,
        })
    }
}

pub struct Snapshot {
    complete: bool,
    size: (usize, usize),
    panes: Vec<CapturedPane>,
}

impl Snapshot {
    fn text(&self, display: &Terminal) -> TextSnapshot {
        if !self.complete
            || self.size != (display.cols(), display.rows.len())
            || self.panes.is_empty()
        {
            return TextSnapshot::Changed;
        }
        match self
            .panes
            .iter()
            .map(|pane| pane.visible(display))
            .collect()
        {
            Some(panes) => TextSnapshot::Ready(panes),
            None => TextSnapshot::Changed,
        }
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
            anyhow::ensure!(fields.len() == 8, "invalid tmux pane metadata");
            if client.zoomed && fields[6] != "1" {
                continue;
            }
            let in_mode = match fields[5] {
                "" => false,
                "copy-mode" => true,
                _ => {
                    complete = false;
                    continue;
                }
            };
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
                in_mode,
            ));
            let mode = if in_mode {
                let scroll = if fields[7].is_empty() {
                    0
                } else {
                    fields[7].parse::<u32>()?
                };
                let first = -i64::from(scroll);
                let last = first + height as i64 - 1;
                format!(" -M -S {first} -E {last}")
            } else {
                String::new()
            };
            commands.push(format!("capture-pane -p -N{mode} -t {id}"));
            commands.push(format!("capture-pane -p -J{mode} -t {id}"));
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
        for ((origin, width, height, in_mode), captures) in
            bounds.into_iter().zip(replies.chunks_exact(2))
        {
            if let Some(terminal) =
                pane_terminal(width, height, &captures[0], &captures[1])
            {
                let pane = PaneText { origin, terminal };
                panes.push(if in_mode {
                    CapturedPane::CopyMode(pane)
                } else {
                    CapturedPane::Application(pane)
                });
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
        for captured in &self.panes {
            let pane = captured.pane();
            if let Some(local) = pane.local_position(position) {
                let visible;
                let pane = match captured {
                    CapturedPane::Application(pane) => {
                        if !pane.matches(display) {
                            return Lookup::Unavailable;
                        }
                        pane
                    }
                    CapturedPane::CopyMode(_) => {
                        let Some(text) = captured.visible(display) else {
                            return Lookup::Unavailable;
                        };
                        visible = text;
                        &visible
                    }
                };
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
