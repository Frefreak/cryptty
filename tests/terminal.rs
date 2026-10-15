use cryptty::terminal::{
    Content, FOREGROUND, PALETTE, Terminal, Underline, indexed_color,
};

fn row(t: &Terminal, y: usize) -> String {
    t.rows[y]
        .iter()
        .filter_map(|c| match &c.content {
            Content::Text { text, .. } => Some(text.chars()),
            Content::Continuation => None,
        })
        .flatten()
        .collect()
}

fn valid(t: &Terminal) {
    assert!(t.cursor.0 < t.cols());
    assert!(t.cursor.1 < t.rows.len());
    for row in &t.rows {
        assert_eq!(row.len(), t.cols());
        for (x, cell) in row.iter().enumerate() {
            match &cell.content {
                Content::Continuation => {
                    assert!(x > 0);
                    assert!(matches!(
                        row[x - 1].content,
                        Content::Text { wide: true, .. }
                    ));
                }
                Content::Text { wide: true, .. } => {
                    assert!(x + 1 < row.len());
                    assert_eq!(row[x + 1].content, Content::Continuation);
                }
                _ => (),
            }
        }
    }
}

#[test]
fn utf8_and_escape_sequences_survive_every_chunk_boundary() {
    let bytes = "hello中文\x1b[31mred\x1b[0m\r\nnext".as_bytes();
    let mut expected = Terminal::new(30, 4);
    expected.feed(bytes);
    for split in 0..=bytes.len() {
        let mut actual = Terminal::new(30, 4);
        actual.feed(&bytes[..split]);
        actual.feed(&bytes[split..]);
        assert_eq!(actual.rows, expected.rows);
        assert_eq!(actual.cursor, expected.cursor);
    }
}

#[test]
fn malformed_utf8_preserves_following_ascii_and_controls() {
    let mut t = Terminal::new(20, 2);
    t.feed(&[0xe4, b'A', 0xe4, b'\r', b'B']);
    assert!(row(&t, 0).starts_with("BA�"));
}

#[test]
fn delayed_wrap_and_carriage_return() {
    let mut t = Terminal::new(3, 2);
    t.feed(b"abc");
    assert_eq!(t.cursor, (2, 0));
    t.feed(b"\rx");
    assert_eq!(row(&t, 0), "xbc");
    t.feed(b"yzq");
    assert_eq!(row(&t, 0), "xyz");
    assert_eq!(row(&t, 1), "q  ");
}

#[test]
fn sgr_preserves_delayed_wrap() {
    let mut t = Terminal::new(3, 2);
    t.feed(b"abc\x1b[31md");
    assert_eq!(row(&t, 1), "d  ");
    assert_eq!(t.color(t.rows[1][0].style.fg), PALETTE[1]);
}

#[test]
fn linefeed_preserves_column_and_scrolls_at_bottom() {
    let mut t = Terminal::new(4, 2);
    t.feed(b"ab\nx\ny");
    assert_eq!(row(&t, 0), "  x ");
    assert_eq!(row(&t, 1), "   y");
}

#[test]
fn colors_and_resets() {
    let mut t = Terminal::new(10, 2);
    t.feed(b"\x1b[1;4;31;44mA\x1b[38;5;196mB\x1b[38;2;1;2;3mC\x1b[0mD");
    let a = t.rows[0][0].style;
    assert!(a.bold && a.underline == Underline::Single);
    assert_eq!(t.color(a.fg), PALETTE[1]);
    assert_eq!(t.color(a.bg), PALETTE[4]);
    assert_eq!(t.color(t.rows[0][1].style.fg), 0xff0000);
    assert_eq!(t.color(t.rows[0][2].style.fg), 0x010203);
    assert_eq!(t.color(t.rows[0][3].style.fg), FOREGROUND);
    assert!(!t.rows[0][3].style.bold);
    assert_eq!(indexed_color(232), 0x080808);
    assert_eq!(indexed_color(255), 0xeeeeee);
}

#[test]
fn cursor_position_erasure_and_saved_cursor() {
    let mut t = Terminal::new(6, 3);
    t.feed(b"abcdef\x1b[2;3H\x1b7X\x1b[3;6HY\x1b8Z");
    assert_eq!(row(&t, 1), "  Z   ");
    t.feed(b"\x1b[1;3H\x1b[K");
    assert_eq!(row(&t, 0), "ab    ");
    t.feed(b"\x1b[2J");
    assert!(t.rows.iter().all(|r| r.iter().all(|c| matches!(
        &c.content, Content::Text { text, .. } if text.chars().eq([' '])
    ))));
    assert_eq!(t.cursor, (2, 0));
}

#[test]
fn scrolling_region_preserves_surrounding_rows() {
    let mut t = Terminal::new(4, 4);
    t.feed(b"AAAA\r\nBBBB\r\nCCCC\r\nDDDD");
    t.feed(b"\x1b[2;3r\x1b[3;1H\n");
    assert_eq!(row(&t, 0), "AAAA");
    assert_eq!(row(&t, 1), "CCCC");
    assert_eq!(row(&t, 2), "    ");
    assert_eq!(row(&t, 3), "DDDD");
}

#[test]
fn deleting_and_inserting_characters() {
    let mut t = Terminal::new(6, 2);
    t.feed(b"abcdef\x1b[1;3H\x1b[2P");
    assert_eq!(row(&t, 0), "abef  ");
    t.feed(b"\x1b[2@");
    assert_eq!(row(&t, 0), "ab  ef");
}

#[test]
fn wide_characters_remain_consistent_after_overwrite_and_resize() {
    let mut t = Terminal::new(6, 3);
    t.feed("中文x".as_bytes());
    t.feed(b"\x1b[1;2HA");
    assert_eq!(row(&t, 0), " A文x ");
    valid(&t);
    t.resize(3, 2);
    valid(&t);
    assert_eq!(row(&t, 0), "文x");
    t.feed("\x1b[2;1H中\x1b[1P".as_bytes());
    valid(&t);
}

#[test]
fn wide_character_wraps_before_last_column() {
    let mut t = Terminal::new(3, 2);
    t.feed("ab中".as_bytes());
    assert_eq!(row(&t, 0), "ab ");
    assert_eq!(row(&t, 1), "中 ");
    valid(&t);
}

#[test]
fn combining_mark_does_not_advance_cursor() {
    let mut t = Terminal::new(10, 2);
    t.feed("e\u{301}".as_bytes());
    assert_eq!(t.cursor, (1, 0));
    assert!(row(&t, 0).starts_with("e\u{301}"));
}

#[test]
fn scrolling_reuses_rows_and_clears_combining_marks() {
    let mut t = Terminal::new(4, 4);
    t.feed("AAAA\r\ne\u{301}bbb\r\nCCCC\r\nDDDD".as_bytes());
    let mut allocations: Vec<_> = t.rows.iter().map(|r| r.as_ptr()).collect();
    allocations.sort();
    t.feed(b"\x1b[2;3r\x1b[31m\x1b[99T");
    assert_eq!(row(&t, 0), "AAAA");
    assert_eq!(row(&t, 1), "    ");
    assert_eq!(row(&t, 2), "    ");
    assert_eq!(row(&t, 3), "DDDD");
    assert!(
        t.rows[1..3]
            .iter()
            .flatten()
            .all(|c| t.color(c.style.fg) == PALETTE[1])
    );
    t.feed(b"\x1b[99S");
    let mut reused: Vec<_> = t.rows.iter().map(|r| r.as_ptr()).collect();
    reused.sort();
    assert_eq!(allocations, reused);
    valid(&t);
}

#[test]
fn csi_capacity_cancellation_and_restart_preserve_parser_state() {
    let mut t = Terminal::new(8, 2);
    t.feed(b"\x1b[31m");
    t.feed(b"\x1b[");
    t.feed(&[b';'; 128]);
    t.feed(b"mA");
    assert_eq!(t.color(t.rows[0][0].style.fg), PALETTE[1]);
    t.feed(b"\x1b[0m");
    t.feed(b"\x1b[");
    t.feed(&[b'1'; 200]);
    t.feed(b"mB");
    assert_eq!(row(&t, 0), "AB      ");
    t.feed(b"\x1b[31\x18C\x1b[32\x1b[34mD");
    assert_eq!(row(&t, 0), "ABCD    ");
    assert_eq!(t.color(t.rows[0][2].style.fg), FOREGROUND);
    assert_eq!(t.color(t.rows[0][3].style.fg), PALETTE[4]);
}

#[test]
fn terminal_queries_and_private_modes() {
    let mut t = Terminal::new(10, 3);
    assert_eq!(t.feed(b"\x1b[2;4H\x1b[6n\x1b[5n"), b"\x1b[2;4R\x1b[0n");
    t.feed(b"\x1b[?1h\x1b[?25l");
    assert!(t.application_cursor);
    assert!(!t.cursor_visible);
    t.feed(b"\x1b[?1l\x1b[?25h");
    assert!(!t.application_cursor);
    assert!(t.cursor_visible);
}

#[test]
fn string_sequences_are_discarded_without_printing_payload() {
    let mut t = Terminal::new(20, 2);
    t.feed(b"A\x1b]0;title\x1b\\B\x1bPpayload\x1b\\C");
    assert!(row(&t, 0).starts_with("ABC"));
}

#[test]
fn shrinking_height_keeps_cursor_visible() {
    let mut t = Terminal::new(6, 4);
    t.feed(b"one\r\ntwo\r\nthree\r\nfour");
    t.resize(6, 2);
    assert_eq!(row(&t, 0), "three ");
    assert_eq!(row(&t, 1), "four  ");
    assert_eq!(t.cursor, (4, 1));
}

#[test]
fn untrusted_input_and_resize_keep_screen_invariants() {
    let mut t = Terminal::new(12, 5);
    let mut random = 1234567u64;
    let sequences: &[&[u8]] = &[
        b"\x1b[999999999999999999999999999999999999H",
        b"\x1b[999999999999P",
        b"\x1b[2J",
        b"\x1b[3@",
        b"\x1b[2L",
        b"\x1b[1M",
        b"\x1b[2;4r",
        b"\x1b[r",
        b"\x1b[1;1H",
        b"\x1b[3;2H",
        "中".as_bytes(),
        b"abc",
        b"\n",
        b"\r",
        b"\x1bM",
        b"\x1bc",
    ];
    for i in 0..10000 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        t.feed(sequences[(random >> 32) as usize % sequences.len()]);
        if i % 17 == 0 {
            t.resize((random % 20 + 1) as usize, (random % 8 + 1) as usize);
        }
        valid(&t);
    }
}

#[test]
fn one_row_terminal_scrolls_in_both_directions() {
    let mut t = Terminal::new(3, 1);
    t.feed(b"abcx");
    assert_eq!(row(&t, 0), "x  ");
    t.feed(b"\x1bM");
    assert_eq!(row(&t, 0), "   ");
}

#[test]
fn fish_startup_queries_receive_a_primary_attributes_reply() {
    let mut t = Terminal::new(80, 24);
    let query = b"\x1b[?u\x1b[>0q\x1b]11;?\x1b\\\x1b[?1049h\x1bP+q696e646e\x1b\\\x1bP+q71756572792d6f732d6e616d65\x1b\\\x1b[?1049l\x1b[0c";
    assert_eq!(
        t.feed(query),
        concat!(
            "\x1bP>|Cryptty(",
            env!("CARGO_PKG_VERSION"),
            ")\x1b\\",
            "\x1b]11;rgb:2828/2a2a/3636\x1b\\",
            "\x1bP0+r696e646e\x1b\\",
            "\x1bP0+r71756572792d6f732d6e616d65\x1b\\",
            "\x1b[?1;0c",
        )
        .as_bytes()
    );
    assert_eq!(t.feed(b"\x1b[c"), b"\x1b[?1;0c");
    assert!(row(&t, 0).trim().is_empty());
    assert_eq!(t.cursor, (0, 0));
}

#[test]
fn attributes_query_preserves_delayed_wrap() {
    let mut t = Terminal::new(3, 2);
    t.feed(b"abc");
    assert_eq!(t.feed(b"\x1b["), b"");
    assert_eq!(t.feed(b"0c"), b"\x1b[?1;0c");
    t.feed(b"d");
    assert_eq!(row(&t, 0), "abc");
    assert_eq!(row(&t, 1), "d  ");
}

#[test]
fn configured_theme_survives_color_resets_resize_and_terminal_reset() {
    use cryptty::config::Config;
    let config: Config = Config::parse(
        r##"
colors:
  primary:
    foreground: "#123456"
    background: "#654321"
  normal:
    blue: "#102030"
  bright:
    blue: "#405060"
"##,
    )
    .unwrap();
    let theme = config.colors.theme();
    let mut t = Terminal::with_theme(12, 2, theme);
    t.feed(b"\x1b[34mA\x1b[94mB\x1b[38;5;4mC\x1b[48;5;12mD");
    assert_eq!(t.color(t.rows[0][0].style.fg), 0x102030);
    assert_eq!(t.color(t.rows[0][1].style.fg), 0x405060);
    assert_eq!(t.color(t.rows[0][2].style.fg), 0x102030);
    assert_eq!(t.color(t.rows[0][3].style.bg), 0x405060);
    t.feed(b"\x1b[39;49mE\x1b[31;44m\x1b[0mF");
    for x in [4, 5] {
        assert_eq!(t.color(t.rows[0][x].style.fg), theme.foreground);
        assert_eq!(t.color(t.rows[0][x].style.bg), theme.background);
    }
    t.feed(b"\x1b[38;5;196mG\x1b[38;2;1;2;3mH");
    assert_eq!(t.color(t.rows[0][6].style.fg), 0xff0000);
    assert_eq!(t.color(t.rows[0][7].style.fg), 0x010203);
    t.resize(14, 3);
    assert_eq!(t.color(t.rows[0][13].style.bg), theme.background);
    assert_eq!(t.color(t.rows[2][0].style.fg), theme.foreground);
    t.feed(b"\x1bc");
    assert_eq!(t.theme(), &theme);
    for cell in t.rows.iter().flatten() {
        assert_eq!(t.color(cell.style.fg), theme.foreground);
        assert_eq!(t.color(cell.style.bg), theme.background);
    }
    t.feed(b"\x1b[34mZ");
    assert_eq!(t.color(t.rows[0][0].style.fg), 0x102030);
}
