use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

use super::{Options, Segment, Target};
use crate::terminal::{Color, Content, Position, Style, Terminal};
use crate::tmux::PaneText;

struct Glyph {
    bytes: Range<usize>,
    position: Position,
    columns: usize,
    style: Style,
}

#[derive(Default)]
struct Line {
    text: String,
    glyphs: Vec<Glyph>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Command,
    Url,
    Path,
    Address,
    Uuid,
    Hash,
    Styled,
}

struct Candidate {
    range: Range<usize>,
    kind: Kind,
}

fn patterns() -> &'static [(Kind, Regex)] {
    static PATTERNS: OnceLock<Vec<(Kind, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            (Kind::Url, r#"(?i)\b(?:https?|ftp|file|ssh)://[^\s<>\"']+"#),
            (Kind::Path, r#"(?:[~.]?/|[A-Za-z]:[\\/])[^\s<>\"'|]+|[\pL\pN_.@-]+(?:/[^\s<>\"'|]+)+"#),
            (Kind::Address, r"\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}\b|[0-9A-Fa-f:]*:[0-9A-Fa-f:]+(?:\.[0-9]+)*"),
            (Kind::Uuid, r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b"),
            (Kind::Hash, r"\b[0-9a-fA-F]+\b"),
        ].into_iter().map(|(kind, pattern)| {
            (kind, Regex::new(pattern).unwrap())
        }).collect()
    })
}

pub(super) fn targets(
    pane: &PaneText,
    display: &Terminal,
    options: &Options,
    history: &HashSet<String>,
) -> Vec<Target> {
    let mut result = Vec::new();
    let mut line = Line::default();
    for (row, cells) in pane.terminal.rows.iter().enumerate() {
        let end = cells.wrapped.unwrap_or(cells.len());
        for (col, cell) in cells.iter().enumerate().take(end) {
            if let Content::Text { text } = &cell.content {
                let start = line.text.len();
                line.text.extend(text.chars());
                line.glyphs.push(Glyph {
                    bytes: start..line.text.len(),
                    position: Position {
                        row: pane.origin.row + row,
                        col: pane.origin.col + col,
                    },
                    columns: text.columns(),
                    style: cell.style(),
                });
            }
        }
        if cells.wrapped.is_none() {
            let end = line.text.trim_end_matches([' ', '\t']).len();
            if row + 1 < pane.terminal.rows.len()
                && continues_command(&line.text[..end])
            {
                line.text.truncate(end);
                line.glyphs.retain(|glyph| glyph.bytes.end <= end);
                line.text.push('\n');
                continue;
            }
            result.extend(recognize(&line, display, options, history));
            line = Line::default();
        }
    }
    result.extend(recognize(&line, display, options, history));
    result
}

fn continues_command(text: &str) -> bool {
    let mut quote = None;
    let mut escaped = false;
    let mut word_start = true;
    for ch in text.chars() {
        if escaped {
            escaped = false;
            if ch != '\n' {
                word_start = false;
            }
            continue;
        } else if ch == '#' && quote.is_none() && word_start {
            return false;
        } else if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        } else if Some(ch) == quote {
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
        }
        word_start = quote.is_none()
            && (ch.is_whitespace()
                || matches!(ch, ';' | '|' | '&' | '(' | ')'));
    }
    escaped
}

fn recognize(
    line: &Line,
    display: &Terminal,
    options: &Options,
    history: &HashSet<String>,
) -> Vec<Target> {
    let mut candidates = Vec::new();
    if let Some(range) = command_range(line, display, options, history) {
        candidates.push(Candidate {
            range,
            kind: Kind::Command,
        });
    }
    candidates.extend(token_candidates(line, options));
    let mut styled = styled_candidates(line, display, options);
    styled.retain(|candidate| {
        let text = &line.text[candidate.range.clone()];
        !candidates.iter().any(|token| {
            matches!(token.kind, Kind::Url | Kind::Path)
                && token.range.start == candidate.range.start
                && token.range.end
                    == candidate.range.start
                        + trim_token(
                            text,
                            token.kind,
                            &line.text[..candidate.range.start],
                        )
                        .len()
                && token.range.end < candidate.range.end
        })
    });
    candidates.extend(styled);
    candidates.retain(|candidate| {
        !line.glyphs.iter().any(|glyph| {
            glyph.style.hidden
                && glyph.bytes.start < candidate.range.end
                && glyph.bytes.end > candidate.range.start
        })
    });
    resolve(candidates, options.overlap_max.get())
        .into_iter()
        .filter_map(|candidate| target(line, candidate.range))
        .collect()
}

fn token_candidates(line: &Line, options: &Options) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for (kind, pattern) in patterns() {
        for hit in pattern.find_iter(&line.text) {
            let mut range = hit.range();
            let text = hit.as_str();
            match kind {
                Kind::Url | Kind::Path => {
                    range.end = range.start
                        + trim_token(text, *kind, &line.text[..range.start])
                            .len();
                    if *kind == Kind::Path && range.len() < 2 {
                        continue;
                    }
                }
                Kind::Address => {
                    if !address_boundaries(&line.text, &range)
                        || text.parse::<std::net::IpAddr>().is_err()
                    {
                        continue;
                    }
                }
                Kind::Hash => {
                    if text.len() < options.hash_min_length.get()
                        || !text.bytes().any(|b| b.is_ascii_alphabetic())
                    {
                        continue;
                    }
                }
                _ => (),
            }
            if range.is_empty() {
                continue;
            }
            candidates.push(Candidate { range, kind: *kind });
        }
    }
    candidates
}

fn styled_candidates(
    line: &Line,
    display: &Terminal,
    options: &Options,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let mut start = 0;
    while start < line.glyphs.len() {
        let style = line.glyphs[start].style;
        let mut end = start + 1;
        while end < line.glyphs.len() && line.glyphs[end].style == style {
            end += 1;
        }
        let emphasized = style.bold
            || display.color(style.fg) != display.color(Color::Foreground);
        if !style.hidden && emphasized {
            let range =
                line.glyphs[start].bytes.start..line.glyphs[end - 1].bytes.end;
            let text = &line.text[range.clone()];
            let trimmed = text.trim();
            if trimmed.chars().count() >= options.styled_min_length.get() {
                let left = text.len() - text.trim_start().len();
                candidates.push(Candidate {
                    range: range.start + left
                        ..range.start + left + trimmed.len(),
                    kind: Kind::Styled,
                });
            }
        }
        start = end;
    }
    candidates
}

#[derive(Clone, Copy)]
enum CommandContext {
    LineStart,
    AfterPrefix,
}

fn command_range(
    line: &Line,
    display: &Terminal,
    options: &Options,
    history: &HashSet<String>,
) -> Option<Range<usize>> {
    if let Some(range) = command_from(
        line,
        display,
        0,
        CommandContext::LineStart,
        options,
        history,
    ) {
        return Some(range);
    }
    let mut previous_color = None;
    for glyph in &line.glyphs {
        if line.text[glyph.bytes.clone()].trim().is_empty() {
            continue;
        }
        let color = display.color(glyph.style.fg);
        let begins_color = previous_color != Some(color);
        previous_color = Some(color);
        if !begins_color || color == display.color(Color::Foreground) {
            continue;
        }
        let start = glyph.bytes.start;
        let boundary = line.text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        if boundary
            && let Some(range) = command_from(
                line,
                display,
                start,
                CommandContext::AfterPrefix,
                options,
                history,
            )
        {
            return Some(range);
        }
    }
    None
}

fn command_from(
    line: &Line,
    display: &Terminal,
    offset: usize,
    context: CommandContext,
    options: &Options,
    history: &HashSet<String>,
) -> Option<Range<usize>> {
    let text = &line.text;
    let suffix = &text[offset..];
    let trimmed = suffix.trim();
    let (command, prompted) = match trimmed.strip_prefix("$ ") {
        Some(command) => (command.trim_start(), true),
        None => (trimmed, false),
    };
    let words = shell_words::split(command).ok()?;
    if !plausible_executable(words.first()?) {
        return None;
    }
    let start = offset + suffix.len() - suffix.trim_start().len()
        + trimmed.len()
        - command.len();
    let range = start..start + command.len();
    let executable = words[0].split(['|', '<', '>', ';', '&']).next()?;
    let color_eligible = executable.contains('/')
        || executable.len() <= options.command_name_max_length.get()
        || super::commands::predefined_commands().contains(executable)
        || options.known_commands.contains(executable)
        || history.contains(executable);
    let highlighted =
        color_eligible && highlighted_executable(line, display, range.clone());
    let accepted = match context {
        CommandContext::LineStart => {
            prompted || highlighted || has_command_structure(command, &words)
        }
        CommandContext::AfterPrefix => highlighted,
    };
    accepted.then_some(range)
}

pub(super) fn plausible_executable(word: &str) -> bool {
    let executable = word.split(['|', '<', '>', ';', '&']).next().unwrap();
    let name = executable.rsplit('/').next().unwrap();
    name.starts_with(|ch: char| ch.is_ascii_lowercase())
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
        })
        && (executable == name
            || executable.starts_with('/')
            || executable.starts_with("./")
            || executable.starts_with("~/"))
}

fn has_command_structure(command: &str, words: &[String]) -> bool {
    let has_option = words.iter().skip(1).any(|word| {
        word.strip_prefix("--")
            .or_else(|| word.strip_prefix('-'))
            .is_some_and(|option| {
                option.starts_with(|ch: char| ch.is_ascii_alphanumeric())
            })
    });
    let has_glob = words.iter().skip(1).any(|word| {
        word.contains('*')
            && word.chars().any(|ch| ch.is_ascii_alphanumeric())
            && !word.chars().any(char::is_whitespace)
    });
    has_option || has_glob || has_shell_operator(command)
}

fn highlighted_executable(
    line: &Line,
    display: &Terminal,
    range: Range<usize>,
) -> bool {
    let text = &line.text;
    let command = &text[range.clone()];
    let start = range.start;
    let token_end = start
        + command
            .find(|ch: char| {
                ch.is_whitespace() || matches!(ch, '|' | '<' | '>' | ';' | '&')
            })
            .unwrap_or(command.len());
    let mut last_color = None;
    for glyph in line.glyphs.iter().filter(|glyph| {
        glyph.bytes.start >= start && glyph.bytes.start < token_end
    }) {
        let color = display.color(glyph.style.fg);
        if color == display.color(Color::Foreground) {
            return false;
        }
        last_color = Some(color);
    }
    last_color.is_some_and(|color| {
        line.glyphs
            .iter()
            .find(|glyph| {
                glyph.bytes.start >= token_end
                    && glyph.bytes.start < range.end
                    && !text[glyph.bytes.clone()].trim().is_empty()
            })
            .is_some_and(|glyph| display.color(glyph.style.fg) != color)
    })
}

fn has_shell_operator(command: &str) -> bool {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in command.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if matches!(ch, '|' | '<' | '>' | ';')
            || (ch == '&' && command[index..].starts_with("&&"))
        {
            let left = command[..index].trim();
            let right = command[index..]
                .trim_start_matches(['|', '<', '>', ';', '&'])
                .trim();
            if !left.is_empty() && !right.is_empty() {
                return true;
            }
        }
    }
    false
}

fn address_boundaries(text: &str, range: &Range<usize>) -> bool {
    let continues = |ch: char| {
        ch.is_alphanumeric() || matches!(ch, '_' | ':' | '.' | '-' | '%')
    };
    if text[..range.start]
        .chars()
        .next_back()
        .is_some_and(continues)
    {
        return false;
    }
    let mut after = &text[range.end..];
    if text[range.clone()].contains('.')
        && !text[range.clone()].contains(':')
        && let Some(port) = after.strip_prefix(':')
    {
        let digits = port.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return false;
        }
        after = &port[digits..];
    }
    if let Some(rest) = after.strip_prefix('.') {
        after = rest;
    }
    !after.chars().next().is_some_and(continues)
}

fn trim_token<'a>(text: &'a str, kind: Kind, before: &str) -> &'a str {
    match kind {
        Kind::Url => crate::text_token::trim_url(text),
        Kind::Path => crate::text_token::trim_path(text, before),
        _ => text,
    }
}

fn resolve(mut candidates: Vec<Candidate>, max: usize) -> Vec<Candidate> {
    candidates.sort_by_key(|c| {
        (c.range.start, std::cmp::Reverse(c.range.len()), c.kind)
    });
    candidates.dedup_by_key(|c| c.range.start);
    candidates.sort_by_key(|c| {
        (c.kind, c.range.start, std::cmp::Reverse(c.range.len()))
    });
    let mut chosen: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        if chosen.iter().any(|other| {
            let suppresses_nested = match other.kind {
                Kind::Command => candidate.kind == Kind::Styled,
                Kind::Url => true,
                Kind::Styled => false,
                _ => candidate.kind != Kind::Styled,
            };
            other.range == candidate.range
                || (other.kind == Kind::Url
                    && candidate.kind == Kind::Path
                    && other.range.contains(&candidate.range.start))
                || (suppresses_nested
                    && other.range.start <= candidate.range.start
                    && other.range.end >= candidate.range.end)
        }) {
            continue;
        }
        let mut boundaries = vec![candidate.range.start];
        boundaries.extend(
            chosen
                .iter()
                .map(|c| c.range.start)
                .filter(|&start| candidate.range.contains(&start)),
        );
        if boundaries.iter().any(|point| {
            chosen.iter().filter(|c| c.range.contains(point)).count() >= max
        }) {
            continue;
        }
        chosen.push(candidate);
    }
    chosen.sort_by_key(|c| (c.range.start, c.kind));
    chosen
}

fn target(line: &Line, range: Range<usize>) -> Option<Target> {
    let mut segments: Vec<Segment> = Vec::new();
    for glyph in line
        .glyphs
        .iter()
        .filter(|g| g.bytes.start < range.end && g.bytes.end > range.start)
    {
        if glyph.style.hidden {
            return None;
        }
        if let Some(last) = segments.last_mut()
            && last.start.row == glyph.position.row
            && last.start.col + last.columns == glyph.position.col
        {
            last.columns += glyph.columns;
        } else {
            segments.push(Segment {
                start: glyph.position,
                columns: glyph.columns,
            });
        }
    }
    (!segments.is_empty()).then(|| Target {
        text: line.text[range].into(),
        segments,
        label: String::new(),
    })
}
