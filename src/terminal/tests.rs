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
    let oldest = terminal.history.front().unwrap().storage_id();
    assert!(terminal.history.front().unwrap().wrapped.is_some());
    terminal.feed(b"\x1b[1;31;44m\r\n");
    let blank = terminal.rows.back().unwrap();
    assert_eq!(blank.storage_id(), oldest);
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
        let oldest = terminal.history.front().unwrap().storage_id();
        terminal.feed(b"\r\n");
        assert_eq!(terminal.rows.back().unwrap().storage_id(), oldest);
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
        "\x1b[1;4H",
        "\x1b[1;2He\u{301}\x1b[1;6H",
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
        let scrolled =
            terminal.rows.iter().take(3).cloned().collect::<Vec<_>>();
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
        assert!(terminal.rows.iter().skip(1).all(|row| {
            row.wrapped.is_none()
                && row.iter().all(|cell| *cell == Cell::blank(terminal.style))
        }));
    }
}

#[test]
fn lazy_rows_hide_recycled_cells_and_materialize_gaps() {
    let mut row = Row::new(8, Style::default());
    let mut styled = Cell::blank(Style {
        fg: Color::Rgb(3, 4, 5),
        bg: Color::Indexed(6),
        ..Style::default()
    });
    styled.content = Content::Text {
        text: CellText::new('中'),
        wide: true,
    };
    styled.hyperlink = Some(Arc::new("https://example.org".into()));
    row.fill(styled.clone());
    row.wrapped = Some(8);
    let erase = Style {
        bg: Color::Indexed(4),
        bold: true,
        ..Style::default()
    };
    row.clear(erase);
    let blank = Cell::blank(erase);
    assert!(row.iter().all(|cell| cell == &blank));
    assert!(row.iter().rev().all(|cell| cell == &blank));
    assert_eq!(row.get(8), None);
    assert_eq!(row.wrapped, None);
    let mut expected = vec![blank.clone(); 8];
    row.set(3, styled.clone());
    expected[3] = styled.clone();
    row[6].style = styled.style;
    expected[6].style = styled.style;
    assert!(row.iter().eq(expected.iter()));
    assert_eq!(row.iter().len(), 8);
    let mut clone = row.clone();
    clone.materialized_mut()[2..].rotate_left(2);
    expected[2..].rotate_left(2);
    assert!(clone.iter().eq(expected.iter()));
    assert_ne!(clone, row);
    clone.clear(Style::default());
    assert_eq!(clone, Row::new(8, Style::default()));
    assert_eq!(clone.iter().nth(7), Some(&Cell::blank(Style::default())));
}

#[test]
fn row_snapshots_copy_logical_cells_across_widths_and_clears() {
    for source_width in [4, 8, 12] {
        for target_width in [2, 8, 16] {
            let mut source = Row::new(source_width, Style::default());
            let mut cell = Cell::blank(Style::default());
            cell.content = Content::Text {
                text: CellText::new('e'),
                wide: false,
            };
            if let Content::Text { text, .. } = &mut cell.content {
                text.push_combining('\u{301}');
            }
            cell.hyperlink = Some(Arc::new("https://example.org".into()));
            source.fill(cell.clone());
            let mut target = Row::new(target_width, Style::default());
            target.fill(cell.clone());
            target.clone_from(&source);
            assert_eq!(target, source);
            source.clear(Style {
                bg: Color::Indexed(3),
                ..Style::default()
            });
            source.set(1, cell.clone());
            target.clone_from(&source);
            assert_eq!(target, source);
            assert!(target.cells[2..].iter().all(|cell| {
                cell.hyperlink.is_none()
                    && cell.content == Cell::blank(Style::default()).content
            }));
            source.clear(Style::default());
            target.clone_from(&source);
            assert_eq!(target, Row::new(source_width, Style::default()));
            assert!(target.cells.iter().all(|cell| cell.hyperlink.is_none()));
        }
    }
}

impl Terminal {
    fn feed_unbuffered(&mut self, mut bytes: &[u8]) -> Vec<u8> {
        let mut parser = self.parser.take().unwrap();
        while let Some(index) = memchr::memchr2(0x18, 0x1a, bytes) {
            parser.advance(self, &bytes[..index]);
            self.cancelling = true;
            parser.advance(self, &bytes[index..index + 1]);
            self.cancelling = false;
            bytes = &bytes[index + 1..];
        }
        parser.advance(self, bytes);
        self.parser = Some(parser);
        std::mem::take(&mut self.replies)
    }
}

#[test]
fn buffered_parser_matches_direct_callbacks_at_every_chunk_boundary() {
    let mut bytes = "a".repeat(600).into_bytes();
    bytes.extend_from_slice(
        concat!(
            "\r\n中e\u{301}😀\rABCDE\x1b[2Dabc",
            "\x1b[1;31;44mstyled\x1b[0m\x1b[6n",
            "\x1b[?7l0123456789abcdef\x1b[?7h",
            "\x1b[4hinsert\x1b[4l\x1b(0lqk\x1b(B",
            "\x1b]8;;https://example.org\x1b\\linked\x1b]8;;\x1b\\",
            "\x1b[?1049halternate\x1b[?1049l\x1b[3b",
            "\x1bP$qm\x1b\\\x1b[2J\x1b[2;3r\x1b[3;1Hbottom\n",
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(b"\xe4ab\x1b]2;cancel\x18after\x1b[6n");
    bytes.extend_from_slice(b"before\x1bcreset\x1bP$qm\x18cancelled\x1b[6n");
    for size in [1, 2, 3, 7, 16, 255, 256, 257, 1024] {
        let mut buffered = Terminal::new(8, 4);
        let mut direct = Terminal::new(8, 4);
        for (index, chunk) in bytes.chunks(size).enumerate() {
            let replies = buffered.feed(chunk);
            assert_eq!(replies, direct.feed_unbuffered(chunk));
            assert_eq!(buffered.rows, direct.rows, "{size}: {index}");
            assert_eq!(buffered.history, direct.history);
            assert_eq!(buffered.cursor, direct.cursor);
            assert_eq!(buffered.pending_wrap, direct.pending_wrap);
            assert_eq!(buffered.preceding, direct.preceding);
            assert_eq!(buffered.style, direct.style);
            assert_eq!(buffered.title, direct.title);
        }
    }
}
