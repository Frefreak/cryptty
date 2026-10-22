use super::{Content, Style, Terminal, Underline};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRange {
    pub start: Position,
    pub end: Position,
}

impl TextRange {
    pub fn between(a: Position, b: Position) -> Self {
        Self {
            start: a.min(b),
            end: a.max(b),
        }
    }

    pub fn contains(self, position: Position) -> bool {
        self.start <= position && position < self.end
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub uri: String,
    pub range: TextRange,
}

impl Terminal {
    pub(super) fn scroll_selection(
        &mut self,
        top: usize,
        count: usize,
        up: bool,
    ) {
        let Some(mut range) = self.selection else {
            return;
        };
        let last = range.end.row - usize::from(range.end.col == 0);
        let bottom = self.scroll_bottom;
        if last < top || range.start.row > bottom {
            return;
        }
        if range.start.row < top
            || last > bottom
            || (up && range.start.row < top + count)
            || (!up && last + count > bottom)
        {
            self.selection = None;
        } else {
            if up {
                range.start.row -= count;
                range.end.row -= count;
            } else {
                range.start.row += count;
                range.end.row += count;
            }
            self.selection = Some(range);
        }
    }

    fn logical_rows(&self, row: usize) -> (usize, usize) {
        let mut first = row;
        while first > 0 && self.rows[first - 1].wrapped.is_some() {
            first -= 1;
        }
        let mut last = row;
        while last + 1 < self.rows.len() && self.rows[last].wrapped.is_some() {
            last += 1;
        }
        (first, last)
    }

    pub fn line_selection(&self, position: Position) -> TextRange {
        let (first, last) = self.logical_rows(position.row);
        TextRange::between(
            Position { row: first, col: 0 },
            Position {
                row: last + 1,
                col: 0,
            },
        )
    }

    pub fn word_selection(&self, position: Position) -> TextRange {
        let (first, last) = self.logical_rows(position.row);
        let mut cells = Vec::new();
        for y in first..=last {
            let row = &self.rows[y];
            for x in 0..row.wrapped.unwrap_or(row.len()) {
                if let Content::Text { text } = &row[x].content {
                    cells.push((
                        Position { row: y, col: x },
                        Position {
                            row: y,
                            col: x + text.columns(),
                        },
                        text,
                    ));
                }
            }
        }
        let Some(index) = cells
            .iter()
            .position(|(start, end, _)| *start <= position && position < *end)
        else {
            return TextRange::between(position, position);
        };
        let class = |i: usize| {
            let ch = cells[i].2.base();
            if ch.is_whitespace() {
                0
            } else if ch.is_alphanumeric() || self.word_characters.contains(ch)
            {
                1
            } else {
                2
            }
        };
        let mut start = index;
        while start > 0 && class(start - 1) == class(index) {
            start -= 1;
        }
        let mut end = index + 1;
        while end < cells.len() && class(end) == class(index) {
            end += 1;
        }
        let text: String = cells[start..end]
            .iter()
            .flat_map(|(_, _, text)| text.chars())
            .collect();
        if text.char_indices().any(|(i, _)| supported_uri(&text[i..])) {
            let suffix = text.len() - text.trim_end_matches("^C").len();
            let boundary = end - suffix;
            if index < boundary {
                end = boundary;
            } else {
                start = boundary;
            }
        }
        TextRange::between(cells[start].0, cells[end - 1].1)
    }

    pub fn selection_between(&self, a: Position, b: Position) -> TextRange {
        let mut range = TextRange::between(a, b);
        if a == b {
            return range;
        }
        if let Some(Content::Continuation) = self
            .rows
            .get(range.start.row)
            .and_then(|row| row.get(range.start.col))
            .map(|c| &c.content)
        {
            range.start.col = range.start.col.saturating_sub(1);
        }
        if let Some(Content::Continuation) = self
            .rows
            .get(range.end.row)
            .and_then(|row| row.get(range.end.col))
            .map(|c| &c.content)
        {
            range.end.col += 1;
        }
        range
    }

    pub fn cell_style(&self, x: usize, y: usize) -> Style {
        let position = Position { row: y, col: x };
        let mut style = self.rows[y][x].style();
        if self.selection.is_some_and(|r| r.contains(position)) {
            style.inverse = !style.inverse;
        }
        if self.hovered_link.iter().any(|r| r.contains(position)) {
            style.underline = Underline::Single;
        }
        style
    }

    pub fn selected_text(&self, range: TextRange) -> String {
        let mut text = String::new();
        for y in range.start.row..=range.end.row.min(self.rows.len() - 1) {
            let row = &self.rows[y];
            let start = if y == range.start.row {
                range.start.col
            } else {
                0
            };
            let end = if y == range.end.row {
                range.end.col
            } else {
                row.len()
            };
            let end = end.min(row.wrapped.unwrap_or(row.len()));
            let offset = text.len();
            for x in start.min(row.len())..end {
                match &row[x].content {
                    Content::Text { text: cell, .. } => {
                        text.extend(cell.chars());
                    }
                    Content::Continuation if x == start && x > 0 => {
                        if let Content::Text { text: cell, .. } =
                            &row[x - 1].content
                        {
                            text.extend(cell.chars());
                        }
                    }
                    _ => (),
                }
            }
            if row.wrapped.is_none() && end == row.len() {
                text.truncate(
                    offset + text[offset..].trim_end_matches(' ').len(),
                );
            }
            if y < range.end.row && row.wrapped.is_none() {
                text.push('\n');
            }
        }
        text
    }

    pub fn link_at(&self, position: Position) -> Option<Link> {
        let cell = self.rows.get(position.row)?.get(position.col)?;
        if let Some(uri) = &cell.hyperlink {
            if !supported_uri(uri) {
                return None;
            }
            let cols = self.cols();
            let index = position.row * cols + position.col;
            let matches = |i: usize| {
                self.rows[i / cols][i % cols].hyperlink.as_ref() == Some(uri)
            };
            let mut start = index;
            while start > 0 && matches(start - 1) {
                start -= 1;
            }
            let mut end = index + 1;
            while end < cols * self.rows.len() && matches(end) {
                end += 1;
            }
            return Some(Link {
                uri: uri.to_string(),
                range: TextRange {
                    start: Position {
                        row: start / cols,
                        col: start % cols,
                    },
                    end: Position {
                        row: end / cols,
                        col: end % cols,
                    },
                },
            });
        }
        let (first, last) = self.logical_rows(position.row);
        let mut text = String::new();
        let mut cells = Vec::new();
        for y in first..=last {
            let row = &self.rows[y];
            for x in 0..row.wrapped.unwrap_or(row.len()) {
                if let Content::Text { text: cell } = &row[x].content {
                    let start = text.len();
                    text.extend(cell.chars());
                    cells.push((
                        start..text.len(),
                        Position { row: y, col: x },
                        Position {
                            row: y,
                            col: x + cell.columns(),
                        },
                    ));
                }
            }
        }
        let hovered = cells
            .iter()
            .find(|(_, start, end)| {
                start.row == position.row
                    && start.col <= position.col
                    && position.col < end.col
            })?
            .0
            .start;
        for (start, _) in text
            .char_indices()
            .filter(|(i, _)| supported_uri(&text[*i..]))
        {
            let tail = &text[start..];
            let end = tail
                .find(|c: char| {
                    c.is_whitespace()
                        || c.is_control()
                        || matches!(c, '<' | '>' | '"')
                })
                .unwrap_or(tail.len());
            let uri = trim_url(&tail[..end]);
            let end = start + uri.len();
            if start <= hovered && hovered < end {
                let first = cells
                    .iter()
                    .find(|(bytes, _, _)| bytes.contains(&start))?;
                let last = cells
                    .iter()
                    .rev()
                    .find(|(bytes, _, _)| bytes.start < end)?;
                return Some(Link {
                    uri: uri.to_owned(),
                    range: TextRange {
                        start: first.1,
                        end: last.2,
                    },
                });
            }
        }
        None
    }
}

pub fn supported_uri(uri: &str) -> bool {
    ["https://", "http://", "file://"].iter().any(|scheme| {
        uri.get(..scheme.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
    })
}

fn trim_url(mut uri: &str) -> &str {
    uri = uri.trim_end_matches("^C");
    loop {
        let Some(last) = uri.chars().last() else {
            return uri;
        };
        let unmatched = match last {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        }
        .is_some_and(|open| {
            uri.matches(last).count() > uri.matches(open).count()
        });
        if matches!(last, '.' | ',' | '，' | ';' | ':' | '!' | '\'')
            || unmatched
        {
            uri = &uri[..uri.len() - last.len_utf8()];
        } else {
            return uri;
        }
    }
}
