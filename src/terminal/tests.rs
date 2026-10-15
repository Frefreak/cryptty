use super::*;

#[test]
fn recycled_history_rows_clear_text_links_and_wrapping() {
    let mut terminal = Terminal::new(8, 2);
    terminal.feed(
        "\x1b[1m\x1b]8;;https://example.org\x1b\\中e\u{301}abcdef".as_bytes(),
    );
    terminal.feed(b"\x1b]8;;\x1b\\\x1b[0m");
    while terminal.history.len() < reflow::HISTORY_LIMIT {
        terminal.feed(b"\r\nline");
    }
    let oldest = terminal.history.front().unwrap().as_ptr();
    assert!(terminal.history.front().unwrap().wrapped.is_some());
    terminal.feed(b"\x1b[1;31;44m\r\n");
    let blank = terminal.rows.last().unwrap();
    assert_eq!(blank.as_ptr(), oldest);
    assert_eq!(blank.wrapped, None);
    assert!(
        blank
            .iter()
            .all(|cell| *cell == Cell::blank(terminal.style))
    );
    assert_eq!(terminal.history.len(), reflow::HISTORY_LIMIT);
    assert!(
        terminal.history.back().unwrap()[0]
            .content
            .eq(&Content::Text {
                text: CellText::new('l'),
                wide: false
            })
    );
    for alternate in [false, true] {
        if alternate {
            terminal.feed(b"\x1b[?1049h");
        }
        terminal.resize(12 + usize::from(alternate), 4);
        if alternate {
            terminal.feed(b"\x1b[?1049l");
        }
        while terminal.history.len() < reflow::HISTORY_LIMIT {
            terminal.feed(b"\r\nline");
        }
        let oldest = terminal.history.front().unwrap().as_ptr();
        terminal.feed(b"\r\n");
        assert_eq!(terminal.rows.last().unwrap().as_ptr(), oldest);
        assert!(
            terminal
                .rows
                .iter()
                .chain(&terminal.history)
                .all(|row| row.len() == terminal.cols())
        );
    }
}

#[test]
fn printable_fast_path_matches_general_placement() {
    let prefixes = [
        "",
        "abcdefg",
        "abcdefgh",
        "\x1b[?7labcdefgh",
        "\x1b[4habcd\r",
        "中x\r",
        "x中\r",
        "中x\x1b[1;2H",
        "e\u{301}\r",
        "\x1b[1;3;4;31;44m",
        "\x1b]8;;https://example.org\x1b\\",
        "\x1b[2;4r\x1b[4;8H",
    ];
    for prefix in prefixes {
        for ch in (' '..='~').chain(['é', '中', '😀', '\u{301}']) {
            let mut fast = Terminal::new(8, 4);
            let mut general = Terminal::new(8, 4);
            fast.feed(prefix.as_bytes());
            general.feed(prefix.as_bytes());
            fast.put(ch);
            general.put_slow(ch);
            assert_eq!(fast.rows, general.rows, "{prefix:?}, {ch:?}");
            assert_eq!(fast.cursor, general.cursor);
            assert_eq!(fast.pending_wrap, general.pending_wrap);
            assert_eq!(fast.history, general.history);
            let suffix = "\u{301}Z\r\nnext".as_bytes();
            fast.feed(suffix);
            general.feed(suffix);
            assert_eq!(fast.rows, general.rows);
            assert_eq!(fast.cursor, general.cursor);
        }
    }
}

#[test]
fn combining_text_preserves_the_byte_limit_with_four_byte_scalars() {
    let mut text = CellText::new('a');
    for _ in 0..31 {
        text.push_combining('\u{301}');
    }
    text.push_combining('\u{e0100}');
    let expected: String = text.chars().collect();
    assert_eq!(expected.len(), 67);
    text.push_combining('\u{301}');
    assert_eq!(text.chars().collect::<String>(), expected);
}

#[test]
fn multiple_scrolled_rows_preserve_history_order_at_capacity() {
    for initial_len in [reflow::HISTORY_LIMIT - 1, reflow::HISTORY_LIMIT] {
        let mut terminal = Terminal::new(8, 4);
        let mut index = 0;
        while terminal.history.len() < initial_len {
            terminal.feed(format!("{index:05}\r\n").as_bytes());
            index += 1;
        }
        let removed = initial_len + 3 - reflow::HISTORY_LIMIT;
        let first_retained = terminal.history[removed].clone();
        let scrolled = terminal.rows[..3].to_vec();
        terminal.feed(b"\x1b[3S");
        assert_eq!(terminal.history.len(), reflow::HISTORY_LIMIT);
        assert_eq!(terminal.history.front().unwrap(), &first_retained);
        assert!(
            terminal
                .history
                .iter()
                .rev()
                .take(3)
                .eq(scrolled.iter().rev())
        );
        assert!(terminal.rows[1..].iter().all(|row| {
            row.wrapped.is_none()
                && row.iter().all(|cell| *cell == Cell::blank(terminal.style))
        }));
    }
}
