use cryptty::terminal::{Cell, Content, Position, Terminal};
use cryptty::tmux::{Lookup, ScreenLink};

#[derive(Default)]
pub(super) struct Hover(Option<Snapshot>);

struct Snapshot {
    position: Position,
    size: (usize, usize),
    link: ScreenLink,
    spans: Vec<Span>,
    bounded: bool,
}

struct Span {
    row: usize,
    start: usize,
    cells: Vec<Cell>,
    wrapped: Option<usize>,
}

impl Hover {
    pub fn clear(&mut self) {
        self.0 = None;
    }

    pub fn update(
        &mut self,
        lookup: Lookup,
        position: Position,
        terminal: &Terminal,
    ) -> Option<&ScreenLink> {
        match lookup {
            Lookup::Ready(Some(link)) => {
                self.0 = Some(Snapshot::new(link, position, terminal));
            }
            Lookup::Pending => {
                if !self.0.as_ref().is_some_and(|snapshot| {
                    snapshot.position == position && snapshot.matches(terminal)
                }) {
                    self.clear();
                }
            }
            _ => self.clear(),
        }
        self.0.as_ref().map(|snapshot| &snapshot.link)
    }
}

impl Snapshot {
    fn new(link: ScreenLink, position: Position, terminal: &Terminal) -> Self {
        let mut spans = Vec::new();
        for range in &link.ranges {
            for row in range.start.row..=range.end.row {
                let start = if row == range.start.row {
                    range.start.col
                } else {
                    0
                };
                let end = if row == range.end.row {
                    range.end.col
                } else {
                    terminal.cols()
                };
                if start == end {
                    continue;
                }
                // Include token context, including trimmed URL punctuation.
                let mut start = start;
                while start > 0 {
                    start -= 1;
                    if delimiter(&terminal.rows[row][start]) {
                        break;
                    }
                }
                let mut end = end;
                while end < terminal.cols() {
                    end += 1;
                    if delimiter(&terminal.rows[row][end - 1]) {
                        break;
                    }
                }
                spans.push(Span {
                    row,
                    start,
                    cells: (start..end)
                        .map(|col| terminal.rows[row][col].clone())
                        .collect(),
                    wrapped: terminal.rows[row].wrapped,
                });
            }
        }
        // A pane edge can hide more of the token on the next row.
        let bounded = spans
            .last()
            .and_then(|span| span.cells.last())
            .is_some_and(token_end)
            && spans.first().is_some_and(|span| {
                span.cells.first().is_some_and(token_end)
                    || (span.start == 0
                        && (span.row == 0
                            || terminal.rows[span.row - 1].wrapped.is_none()))
            });
        Self {
            position,
            size: (terminal.cols(), terminal.rows.len()),
            link,
            spans,
            bounded,
        }
    }

    fn matches(&self, terminal: &Terminal) -> bool {
        self.bounded
            && self.size == (terminal.cols(), terminal.rows.len())
            && self.spans.first().is_some_and(|span| {
                span.start != 0
                    || span.row == 0
                    || terminal.rows[span.row - 1].wrapped.is_none()
            })
            && self.spans.iter().all(|span| {
                let row = &terminal.rows[span.row];
                row.wrapped == span.wrapped
                    && span.cells.iter().enumerate().all(|(offset, before)| {
                        let after = &row[span.start + offset];
                        before.content == after.content
                            && before.hyperlink == after.hyperlink
                    })
            })
    }
}

fn delimiter(cell: &Cell) -> bool {
    token_end(cell)
        || matches!(&cell.content, Content::Text { text }
            if text.chars().any(|ch| matches!(ch, '\u{2500}'..='\u{257f}')))
}

fn token_end(cell: &Cell) -> bool {
    match &cell.content {
        Content::Text { text } => text.chars().any(|ch| {
            ch.is_whitespace()
                || ch.is_control()
                || matches!(ch, '<' | '>' | '"')
        }),
        Content::Continuation => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Terminal, Hover, Position) {
        let mut terminal = Terminal::new(60, 4);
        terminal.feed(b"https://example.com/path other text");
        let position = Position { row: 0, col: 5 };
        let link = terminal.link_at(position).unwrap().into();
        let mut hover = Hover::default();
        hover.update(Lookup::Ready(Some(link)), position, &terminal);
        (terminal, hover, position)
    }

    #[test]
    fn unrelated_output_and_identical_redraw_preserve_pending_hover() {
        let (mut terminal, mut hover, position) = setup();
        for tick in 0..20 {
            terminal.feed(format!("\x1b[3;1Htick {tick}").as_bytes());
            terminal.feed(b"\x1b[H\x1b[31mhttps://example.com/path");
            let link = hover.update(Lookup::Pending, position, &terminal);
            assert_eq!(link.unwrap().uri, "https://example.com/path");
        }
    }

    #[test]
    fn changed_text_target_and_boundaries_clear_pending_hover() {
        for edit in [
            &b"\x1b[1;10HX"[..],
            &b"\x1b[1;25Hx"[..],
            &b"\x1b[H\x1b]8;;https://other.test\x07https://example.com/path"[..],
            &b"\x1b[H\x1b[K"[..],
        ] {
            let (mut terminal, mut hover, position) = setup();
            terminal.feed(edit);
            assert!(
                hover.update(Lookup::Pending, position, &terminal).is_none()
            );
        }
    }

    #[test]
    fn trimmed_punctuation_becoming_part_of_url_clears_hover() {
        let mut terminal = Terminal::new(60, 4);
        terminal.feed(b"https://example.com/path). ");
        let position = Position { row: 0, col: 5 };
        let mut hover = Hover::default();
        hover.update(
            Lookup::Ready(Some(terminal.link_at(position).unwrap().into())),
            position,
            &terminal,
        );
        terminal.feed(b"\x1b[1;26Hx");
        assert_eq!(
            terminal.link_at(position).unwrap().uri,
            "https://example.com/path)x"
        );
        assert!(hover.update(Lookup::Pending, position, &terminal).is_none());
    }

    #[test]
    fn unresolved_token_at_wrapped_edge_requires_fresh_lookup() {
        let mut terminal = Terminal::new(24, 4);
        terminal.feed(b"https://example.com/path). ");
        let position = Position { row: 0, col: 5 };
        let mut hover = Hover::default();
        hover.update(
            Lookup::Ready(Some(terminal.link_at(position).unwrap().into())),
            position,
            &terminal,
        );
        terminal.feed(b"\x1b[2;2Hx");
        assert_eq!(
            terminal.link_at(position).unwrap().uri,
            "https://example.com/path)x"
        );
        assert!(hover.update(Lookup::Pending, position, &terminal).is_none());
    }

    #[test]
    fn preceding_wrapped_token_requires_fresh_lookup() {
        let mut terminal = Terminal::new(24, 4);
        terminal.feed(b"aaaaaaaaaaaaaaaaaaaaaaaahttps://example.com/path ");
        let position = Position { row: 1, col: 5 };
        let mut hover = Hover::default();
        hover.update(
            Lookup::Ready(Some(terminal.link_at(position).unwrap().into())),
            position,
            &terminal,
        );
        terminal.feed(b"\x1b[Hhttp://");
        assert!(
            terminal
                .link_at(position)
                .unwrap()
                .uri
                .starts_with("http://")
        );
        assert!(hover.update(Lookup::Pending, position, &terminal).is_none());
    }

    #[test]
    fn newly_wrapped_preceding_row_clears_pending_hover() {
        let mut terminal = Terminal::new(40, 4);
        terminal.feed(b"prefix\r\nhttps://example.com/path ");
        let position = Position { row: 1, col: 5 };
        let mut hover = Hover::default();
        hover.update(
            Lookup::Ready(Some(terminal.link_at(position).unwrap().into())),
            position,
            &terminal,
        );
        assert!(hover.update(Lookup::Pending, position, &terminal).is_some());
        terminal.rows[0].wrapped = Some(40);
        assert!(hover.update(Lookup::Pending, position, &terminal).is_none());
    }

    #[test]
    fn pointer_motion_resize_and_completed_lookup_clear_old_hover() {
        let (terminal, mut hover, _) = setup();
        assert!(hover.update(
            Lookup::Pending,
            Position { row: 0, col: 6 },
            &terminal,
        ).is_none());
        let (mut terminal, mut hover, position) = setup();
        terminal.resize(50, 4);
        assert!(hover.update(Lookup::Pending, position, &terminal).is_none());
        let (terminal, mut hover, position) = setup();
        assert!(
            hover
                .update(Lookup::Ready(None), position, &terminal)
                .is_none()
        );
        assert!(hover.update(Lookup::Pending, position, &terminal).is_none());
    }

    #[test]
    fn wrapped_pane_link_checks_every_segment_without_other_pane_text() {
        use cryptty::terminal::TextRange;
        let mut terminal = Terminal::new(40, 4);
        terminal.feed(
            b"https://example.com/ |other pane\r\npath                |status",
        );
        let position = Position { row: 1, col: 1 };
        let mut hover = Hover::default();
        hover.update(
            Lookup::Ready(Some(ScreenLink {
                uri: "https://example.com/path".into(),
                ranges: vec![
                    TextRange::between(
                        Position { row: 0, col: 0 },
                        Position { row: 0, col: 19 },
                    ),
                    TextRange::between(
                        Position { row: 1, col: 0 },
                        Position { row: 1, col: 4 },
                    ),
                ],
            })),
            position,
            &terminal,
        );
        terminal.feed(b"\x1b[1;22Hnew status");
        assert!(hover.update(Lookup::Pending, position, &terminal).is_some());
        terminal.feed(b"\x1b[1;10HX");
        assert!(hover.update(Lookup::Pending, position, &terminal).is_none());
    }
}
