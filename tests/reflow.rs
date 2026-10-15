use cryptty::terminal::{Content, Position, Terminal, TextRange};

fn row(t: &Terminal, y: usize) -> String {
    t.rows[y]
        .iter()
        .filter_map(|cell| match &cell.content {
            Content::Text { text, .. } => Some(text.chars()),
            Content::Continuation => None,
        })
        .flatten()
        .collect()
}

fn selected(t: &Terminal, end: Position) -> String {
    t.selected_text(TextRange::between(Position { row: 0, col: 0 }, end))
}

#[test]
fn narrow_then_wide_preserves_long_url_and_cursor() {
    let uri = "https://example.com/auth?scope=read%20write&state=abcdef";
    let mut t = Terminal::new(80, 4);
    t.feed(uri.as_bytes());
    t.resize(12, 4);
    t.resize(80, 4);
    assert_eq!(row(&t, 0).trim_end(), uri);
    assert_eq!(t.cursor, (uri.len(), 0));
    assert_eq!(t.link_at(Position { row: 0, col: 10 }).unwrap().uri, uri);
    t.feed(b"!");
    assert_eq!(row(&t, 0).trim_end(), format!("{uri}!"));
}

#[test]
fn hard_lines_remain_separate_and_styles_and_links_survive() {
    let mut t = Terminal::new(20, 6);
    t.feed(b"\x1b[31mfirst-line\r\n\x1b]8;;https://example.com\x07second-line\x1b]8;;\x07");
    let style = t.rows[0][0].style;
    t.resize(7, 6);
    t.resize(20, 6);
    assert_eq!(row(&t, 0).trim_end(), "first-line");
    assert_eq!(row(&t, 1).trim_end(), "second-line");
    assert_eq!(t.rows[0][0].style, style);
    assert_eq!(
        t.link_at(Position { row: 1, col: 3 }).unwrap().uri,
        "https://example.com"
    );
}

#[test]
fn wide_and_combining_characters_survive_repeated_reflow() {
    let text = "abc中e\u{301}文def";
    let mut t = Terminal::new(20, 5);
    t.feed(text.as_bytes());
    for width in [4, 7, 3, 20] {
        t.resize(width, 5);
        for cells in &t.rows {
            for (x, cell) in cells.iter().enumerate() {
                match cell.content {
                    Content::Text { wide: true, .. } => assert!(matches!(
                        cells[x + 1].content,
                        Content::Continuation
                    )),
                    Content::Continuation => assert!(
                        x > 0
                            && matches!(
                                cells[x - 1].content,
                                Content::Text { wide: true, .. }
                            )
                    ),
                    _ => (),
                }
            }
        }
    }
    assert_eq!(row(&t, 0).trim_end(), text);
    assert_eq!(t.cursor, (11, 0));
}

#[test]
fn pending_wrap_survives_resize_and_next_character() {
    let mut t = Terminal::new(8, 4);
    t.feed(b"abcdefgh");
    t.resize(4, 4);
    t.resize(8, 4);
    t.feed(b"i");
    assert_eq!(selected(&t, Position { row: 1, col: 1 }), "abcdefghi");
    assert_eq!(t.cursor, (1, 1));
}

#[test]
fn height_growth_restores_scrolled_primary_rows() {
    let mut t = Terminal::new(10, 3);
    t.feed(b"one\r\ntwo\r\nthree\r\nfour");
    t.resize(10, 4);
    assert_eq!(row(&t, 0).trim_end(), "one");
    assert_eq!(row(&t, 3).trim_end(), "four");
    assert_eq!(t.cursor, (4, 3));
    t.resize(10, 2);
    t.resize(10, 4);
    assert_eq!(row(&t, 0).trim_end(), "one");
    assert_eq!(row(&t, 3).trim_end(), "four");
}

#[test]
fn alternate_grid_stays_fixed_while_primary_reflows() {
    let mut t = Terminal::new(12, 4);
    t.feed(b"abcdefghijkl\x1b[?1049h\x1b[HABCDEFGHIJKL");
    t.resize(6, 4);
    assert_eq!(row(&t, 0), "ABCDEF");
    assert_eq!(row(&t, 1).trim_end(), "");
    t.feed(b"\x1b[?1049lX");
    t.resize(20, 4);
    assert_eq!(row(&t, 0).trim_end(), "abcdefghijklX");
}

#[test]
fn saved_cursor_tracks_reflowed_text() {
    let mut t = Terminal::new(20, 5);
    t.feed(b"abcdefghij\x1b7\r\nnext");
    t.resize(6, 5);
    t.feed(b"\x1b8X");
    t.resize(20, 5);
    assert_eq!(row(&t, 0).trim_end(), "abcdefghijX");
}

#[test]
fn clear_saved_lines_discards_retained_history() {
    let mut t = Terminal::new(10, 2);
    t.feed(b"one\r\ntwo\r\nthree\x1b[3J");
    t.resize(10, 4);
    assert_eq!(row(&t, 0).trim_end(), "two");
}

#[test]
fn cursor_on_wide_continuation_keeps_its_column() {
    let mut t = Terminal::new(6, 2);
    t.feed("中ab\x1b[1;2H".as_bytes());
    t.resize(8, 2);
    assert_eq!(t.cursor, (1, 0));
}

#[test]
fn cursor_at_soft_wrap_boundary_stays_before_next_character() {
    let mut t = Terminal::new(4, 4);
    t.feed(b"abcdefgh\x1b[2;1H");
    t.resize(2, 4);
    t.feed(b"X");
    t.resize(8, 4);
    assert_eq!(row(&t, 0).trim_end(), "abcdXfgh");
}

#[test]
fn inserting_a_top_row_severs_the_history_wrap_connection() {
    let mut t = Terminal::new(4, 2);
    t.feed(b"abcdefghij\x1b[H\x1b[L");
    t.resize(12, 2);
    assert_eq!(row(&t, 0).trim_end(), "");
    assert_eq!(row(&t, 1).trim_end(), "efgh");
    assert_eq!(t.cursor, (0, 0));
}

#[test]
fn alternate_saved_cursor_does_not_wrap_after_widening() {
    let mut t = Terminal::new(4, 2);
    t.feed(b"\x1b[?47habcd\x1b7");
    t.resize(8, 2);
    t.feed(b"\x1b8X");
    assert_eq!(row(&t, 0).trim_end(), "abcX");
    assert_eq!(t.cursor, (4, 0));
}

#[test]
fn minimum_width_preserves_wide_characters() {
    let mut t = Terminal::new(8, 3);
    t.feed("中abc".as_bytes());
    t.resize(1, 3);
    assert_eq!(t.cols(), 2);
    t.resize(8, 3);
    assert_eq!(row(&t, 0).trim_end(), "中abc");
}
