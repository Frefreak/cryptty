use cryptty::terminal::{Position, Terminal, TextRange};

fn p(row: usize, col: usize) -> Position {
    Position { row, col }
}

#[test]
fn long_url_survives_soft_wrapping_at_every_cell() {
    let uri = format!(
        "https://example.com/auth?scope={}&redirect=https%3A%2F%2Fexample.com%2Fcallback&state=test#result",
        "read%20write%20".repeat(30),
    );
    let mut terminal = Terminal::new(31, 30);
    terminal.feed(uri.as_bytes());
    for index in 0..uri.len() {
        let link = terminal.link_at(p(index / 31, index % 31)).unwrap();
        assert_eq!(link.uri, uri);
    }
    let range = TextRange::between(p(0, 0), p(uri.len() / 31, uri.len() % 31));
    assert_eq!(terminal.selected_text(range), uri);
}

#[test]
fn hard_line_breaks_stay_separate_even_with_matching_underlines() {
    let mut terminal = Terminal::new(40, 4);
    terminal.feed(b"\x1b[4;34m https://example.com/first\r\n continuation?state=second\x1b[0m");
    assert_eq!(
        terminal.link_at(p(0, 10)).unwrap().uri,
        "https://example.com/first"
    );
    assert!(terminal.link_at(p(1, 5)).is_none());
    assert_eq!(
        terminal.selected_text(TextRange::between(p(0, 0), p(1, 26))),
        " https://example.com/first\n continuation?state=second"
    );
}

#[test]
fn osc8_target_is_independent_of_display_text() {
    let mut terminal = Terminal::new(40, 4);
    terminal.feed(b"\x1b]8;;https://example.com/full?one=1&two=2\x1b\\Open login page\x1b]8;;\x1b\\");
    let link = terminal.link_at(p(0, 5)).unwrap();
    assert_eq!(link.uri, "https://example.com/full?one=1&two=2");
    assert_eq!(terminal.selected_text(link.range), "Open login page");
    terminal.feed(b"\r\n\x1b]8;;mailto:test@example.com\x07mail\x1b]8;;\x07");
    assert!(terminal.link_at(p(1, 1)).is_none());
}

#[test]
fn punctuation_and_balanced_parentheses_preserve_the_url() {
    let mut terminal = Terminal::new(80, 4);
    terminal.feed(b"See (https://example.com/a_(b)?q=x&other=y#part).\r\nmailto:test@example.com");
    assert_eq!(
        terminal.link_at(p(0, 15)).unwrap().uri,
        "https://example.com/a_(b)?q=x&other=y#part"
    );
    assert!(
        terminal
            .link_at(p(
                0,
                5 + "https://example.com/a_(b)?q=x&other=y#part".len()
            ))
            .is_none()
    );
    assert!(terminal.link_at(p(1, 5)).is_none());
}

#[test]
fn selection_includes_wide_and_combining_characters_without_wrap_padding() {
    let mut terminal = Terminal::new(4, 4);
    terminal.feed("abc中e\u{301}".as_bytes());
    assert_eq!(
        terminal.selected_text(TextRange::between(p(0, 0), p(1, 3))),
        "abc中e\u{301}"
    );
    let range = terminal.selection_between(p(1, 1), p(1, 2));
    assert_eq!(range, TextRange::between(p(1, 0), p(1, 2)));
    assert_eq!(terminal.selected_text(range), "中");
}

#[test]
fn wrap_metadata_follows_scrolling_and_alternate_screen() {
    let mut terminal = Terminal::new(20, 3);
    terminal.feed(b"header\r\nhttps://example.com/a-long-url");
    terminal.feed(b"\r\nnext");
    assert_eq!(
        terminal.link_at(p(0, 10)).unwrap().uri,
        "https://example.com/a-long-url"
    );
    terminal.feed(b"\x1b[?1049hother\x1b[?1049l");
    assert_eq!(
        terminal.link_at(p(0, 10)).unwrap().uri,
        "https://example.com/a-long-url"
    );
    terminal.feed(b"\x1b[2J");
    assert!(terminal.rows.iter().all(|row| row.wrapped.is_none()));
}

#[test]
fn editing_rows_breaks_links_at_the_changed_boundary() {
    for edit in [b'L', b'M'] {
        let mut terminal = Terminal::new(20, 4);
        terminal.feed(b"https://example.com/a-long-url\r\nunrelated");
        terminal.feed(&[27, b'[', b'2', b';', b'1', b'H', 27, b'[', edit]);
        assert_eq!(
            terminal.link_at(p(0, 10)).unwrap().uri,
            "https://example.com/"
        );
    }
}

#[test]
fn clicking_a_wide_character_without_drag_keeps_selection_empty() {
    let mut terminal = Terminal::new(20, 3);
    terminal.feed("中".as_bytes());
    let range = terminal.selection_between(p(0, 1), p(0, 1));
    assert_eq!(range.start, range.end);
}

#[test]
fn automatic_wrap_after_scrolling_keeps_link_continuous() {
    let mut terminal = Terminal::new(20, 2);
    terminal.feed(b"header\r\nhttps://example.com/a-long-url");
    assert_eq!(
        terminal.link_at(p(1, 3)).unwrap().uri,
        "https://example.com/a-long-url"
    );
}

#[test]
fn shrinking_then_growing_does_not_join_unrelated_rows() {
    let mut terminal = Terminal::new(10, 3);
    terminal.feed(b"https://a/bbbbb\x1b[H");
    terminal.resize(10, 1);
    terminal.resize(10, 3);
    terminal.feed(b"\x1b[2;1Hsuffix");
    assert_eq!(terminal.link_at(p(0, 3)).unwrap().uri, "https://a/");
}
