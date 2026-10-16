use cryptty::terminal::{
    Color, Content, CursorShape, Effect, MouseMode, PALETTE, Terminal,
    Underline,
};

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

#[test]
fn alternate_screen_restores_shell_cursor_style_and_pending_wrap() {
    let mut t = Terminal::new(4, 3);
    t.feed(b"\x1b[31mbase\x1b[?1049h\x1b[0m\x1b[Hedit");
    assert!(t.alternate_screen());
    assert_eq!(row(&t, 0), "edit");
    t.feed(b"\x1b[?1049h\x1b[?1049l");
    assert!(!t.alternate_screen());
    assert_eq!(row(&t, 0), "base");
    assert_eq!(t.cursor, (3, 0));
    t.feed(b"Z");
    assert_eq!(row(&t, 1), "Z   ");
    assert_eq!(t.rows[1][0].style().fg, Color::Indexed(1));
    t.feed(b"\x1b[?1049l");
    assert_eq!(t.cursor, (1, 1));
}

#[test]
fn alternate_and_primary_grids_both_resize() {
    let mut t = Terminal::new(6, 4);
    t.feed(b"one\r\ntwo\r\nthree\r\nfour\x1b[?1049h\x1b[Halt");
    t.resize(4, 2);
    t.feed(b"\x1b[?1049l");
    assert_eq!(row(&t, 0), "e   ");
    assert_eq!(row(&t, 1), "four");
    assert_eq!(t.cursor, (3, 1));
    t.feed(b"\x1b[?47h");
    assert_eq!(t.rows.len(), 2);
    assert!(t.rows.iter().all(|r| r.len() == 4));
}

#[test]
fn alternate_47_retains_contents_and_1047_clears_on_exit() {
    let mut t = Terminal::new(5, 2);
    t.feed(b"shell\x1b[?47halt\x1b[?47l");
    assert_eq!(row(&t, 0), "shell");
    t.feed(b"\x1b[?47h");
    assert_eq!(row(&t, 0), "alt  ");
    t.feed(b"\x1b[?1047l\x1b[?47h");
    assert!(t.rows.iter().flatten().all(|cell| matches!(
        &cell.content, Content::Text { text, .. } if text.chars().eq([' '])
    )));
}

#[test]
fn origin_mode_and_margins_bound_cursor_and_reports() {
    let mut t = Terminal::new(10, 5);
    t.feed(b"\x1b[2;4r\x1b[?6h");
    assert_eq!(t.cursor, (0, 1));
    assert_eq!(t.feed(b"\x1b[3;2H\x1b[6n"), b"\x1b[3;2R");
    assert_eq!(t.cursor, (1, 3));
    t.feed(b"\x1b[99B");
    assert_eq!(t.cursor.1, 3);
    t.feed(b"\x1b[99A");
    assert_eq!(t.cursor.1, 1);
    t.feed(b"\x1b[?6l");
    assert_eq!(t.cursor, (0, 0));
    t.feed(b"\x1b[5;1H\x1b[99B");
    assert_eq!(t.cursor.1, 4);
}

#[test]
fn tabs_charsets_insert_mode_and_repeat_work_together() {
    let mut t = Terminal::new(20, 3);
    t.feed(b"\x1b[3g\x1b[4G\x1bH\r\tX");
    assert_eq!(t.cursor, (4, 0));
    t.feed(b"\x1b[Z");
    assert_eq!(t.cursor, (3, 0));
    t.feed(b"\x1b[2;1H\x1b(0lqk\x1b(B");
    assert!(row(&t, 1).starts_with("┌─┐"));
    t.feed(b"\x1b[3;1HA\x1b[3b");
    assert!(row(&t, 2).starts_with("AAAA"));
    let mut t = Terminal::new(6, 2);
    t.feed("中abcd\r\x1b[4hX\x1b[4l".as_bytes());
    assert_eq!(row(&t, 0), "X中abc");
    assert_eq!(t.rows[0][2].content, Content::Continuation);
}

#[test]
fn saved_cursor_restores_origin_charset_and_attributes() {
    let mut t = Terminal::new(8, 4);
    t.feed(b"\x1b[2;4r\x1b[?6h\x1b[2;3H\x1b[3;31m\x1b(0\x1b7");
    t.feed(b"\x1b[?6l\x1b(B\x1b[0m\x1b8q");
    assert_eq!(t.cursor, (3, 2));
    assert!(row(&t, 2).starts_with("  ─"));
    assert!(t.rows[2][2].style().italic);
    assert_eq!(t.feed(b"\x1b[6n"), b"\x1b[2;4R");
}

#[test]
fn saved_cursor_restores_autowrap() {
    let mut t = Terminal::new(3, 2);
    t.feed(b"\x1b7\x1b[?7l\x1b8abcd");
    assert_eq!(row(&t, 0), "abc");
    assert_eq!(row(&t, 1), "d  ");
}

#[test]
fn sgr_colon_colors_and_independent_attribute_resets() {
    let mut t = Terminal::new(20, 2);
    t.feed(b"\x1b[1;2;3;4:3;9;53;38:2::1:2:3;48:5:4;58:2::4:5:6mA");
    let a = t.rows[0][0].style();
    assert!(a.bold && a.dim && a.italic && a.strike && a.overline);
    assert_eq!(a.underline, Underline::Curly);
    assert_eq!(a.fg, Color::Rgb(1, 2, 3));
    assert_eq!(a.bg, Color::Indexed(4));
    assert_eq!(a.underline_color, Some(Color::Rgb(4, 5, 6)));
    t.feed(b"\x1b[22;23;24;29;55;59mB");
    let b = t.rows[0][1].style();
    assert!(!b.bold && !b.dim && !b.italic && !b.strike && !b.overline);
    assert_eq!(b.underline, Underline::None);
    assert_eq!(b.underline_color, None);
    assert_eq!(b.fg, a.fg);
    t.feed(b"\x1b[58;5;2;4:2mC\x1b[0mD");
    assert_eq!(
        t.rows[0][2].style().underline_color,
        Some(Color::Indexed(2))
    );
    assert_eq!(t.rows[0][2].style().underline, Underline::Double);
    assert_eq!(t.rows[0][3].style(), Default::default());
}

#[test]
fn erase_uses_colors_without_drawing_current_decorations() {
    let mut t = Terminal::new(5, 2);
    t.feed(b"\x1b[4:3;3;44m\x1b[2J");
    for cell in t.rows.iter().flatten() {
        assert_eq!(cell.style().underline, Underline::None);
        assert!(!cell.style().italic);
        assert_eq!(cell.style().bg, Color::Indexed(4));
    }
}

#[test]
fn osc_updates_existing_indexed_and_default_colors() {
    let mut t = Terminal::new(8, 2);
    t.feed(b"A\x1b[31mB\x1b[38;2;255;0;0mC");
    t.feed(b"\x1b]4;1;#123456\x07\x1b]10;rgb:ab/cd/ef\x1b\\");
    assert_eq!(t.color(t.rows[0][0].style().fg), 0xabcdef);
    assert_eq!(t.color(t.rows[0][1].style().fg), 0x123456);
    assert_eq!(t.color(t.rows[0][2].style().fg), 0xff0000);
    assert_eq!(
        t.feed(b"\x1b]4;1;?\x1b\\"),
        b"\x1b]4;1;rgb:1212/3434/5656\x1b\\"
    );
    assert_eq!(t.feed(b"\x1b]10;?\x07"), b"\x1b]10;rgb:abab/cdcd/efef\x07");
    t.feed(b"\x1b]104;1\x07\x1b]110\x07");
    assert_eq!(t.color(t.rows[0][1].style().fg), PALETTE[1]);
    assert_eq!(t.color(t.rows[0][0].style().fg), t.theme().foreground);
}

#[test]
fn titles_hyperlinks_and_clipboard_survive_chunk_boundaries() {
    let bytes = b"\x1b]2;project;name\x1b\\\x1b]8;id=test;https://example.org/a;b\x1b\\link\x1b]8;;\x07 plain\x1b]52;c;aGVsbG8=\x07";
    for split in 0..=bytes.len() {
        let mut t = Terminal::new(20, 2);
        t.feed(&bytes[..split]);
        t.feed(&bytes[split..]);
        assert_eq!(t.title, "project;name");
        assert_eq!(
            t.rows[0][0].hyperlink.as_deref().map(String::as_str),
            Some("https://example.org/a;b")
        );
        assert!(t.rows[0][4].hyperlink.is_none());
        assert!(
            matches!(t.effects.as_slice(), [Effect::Clipboard(text)] if text == "hello")
        );
    }
}

#[test]
fn oversized_and_cancelled_osc_do_not_apply_partial_commands() {
    let mut t = Terminal::new(8, 2);
    t.feed(b"\x1b]2;");
    t.feed(&vec![b'x'; 70000]);
    t.feed(b"\x07A");
    assert_eq!(t.title, "Cryptty");
    assert_eq!(row(&t, 0), "A       ");
    t.feed(b"\x1b]2;cancelled\x18B");
    assert_eq!(t.title, "Cryptty");
    assert_eq!(row(&t, 0), "AB      ");
    assert!(t.feed(b"\x1b]52;c;?\x07").is_empty());
    assert!(t.effects.is_empty());
}

#[test]
fn input_mode_queries_and_cursor_style_report_actual_state() {
    let mut t = Terminal::new(8, 2);
    assert_eq!(t.feed(b"\x1b[?2004$p"), b"\x1b[?2004;2$y");
    t.feed(b"\x1b[?2004;1004;1002;1006h\x1b=\x1b[5 q");
    assert!(t.bracketed_paste && t.focus_reporting && t.mouse_sgr);
    assert!(t.application_keypad);
    assert_eq!(t.mouse_mode, MouseMode::Drag);
    assert_eq!(t.cursor_style.shape, CursorShape::Beam);
    assert!(t.cursor_style.blinking);
    assert_eq!(t.feed(b"\x1bP$q q\x1b\\"), b"\x1bP1$r5 q\x1b\\");
    assert_eq!(
        t.feed(b"\x1b[?2004;9999$p"),
        b"\x1b[?2004;1$y\x1b[?9999;0$y"
    );
    t.feed(b"\x1b[?2026h");
    assert!(t.synchronized_until().is_some());
    assert_eq!(t.feed(b"\x1b[?2026$p"), b"\x1b[?2026;1$y");
    t.feed(b"\x1b[?2026l\x1b[?2004;1004;1002;1006l\x1b>");
    assert!(t.synchronized_until().is_none());
    assert_eq!(t.mouse_mode, MouseMode::Off);
    assert!(!t.application_keypad && !t.bracketed_paste && !t.focus_reporting);
}

#[test]
fn parser_handles_extended_styles_and_strings_at_every_boundary() {
    let bytes = "中文\x1b[38:2::1:2:3;4:3mstyled\x1b]11;?\x1b\\\x1bP$qm\x1b\\"
        .as_bytes();
    let mut expected = Terminal::new(40, 3);
    let reply = expected.feed(bytes);
    for split in 0..=bytes.len() {
        let mut t = Terminal::new(40, 3);
        let mut actual = t.feed(&bytes[..split]);
        actual.extend(t.feed(&bytes[split..]));
        assert_eq!(t.rows, expected.rows);
        assert_eq!(actual, reply);
    }
}

#[test]
fn notifications_preserve_text_and_distinguish_bell_terminators() {
    let mut t = Terminal::new(20, 3);
    t.feed(b"\x07\x07\x1b]9;Task finished; details\x07");
    assert_eq!(t.effects.len(), 2);
    assert!(matches!(t.effects[0], Effect::Bell));
    assert!(matches!(&t.effects[1], Effect::Notification(s)
        if s == "Task finished; details"));
    t.effects.clear();
    t.feed(b"\x1b]9;cancelled\x18\x1b]9;4;1;50\x1b\\");
    assert!(
        t.effects
            .iter()
            .all(|e| !matches!(e, Effect::Notification(_)))
    );
    t.feed(format!("\x1b]9;{}\x1b\\", "x".repeat(2000)).as_bytes());
    assert!(matches!(t.effects.last(), Some(Effect::Notification(s))
        if s.len() == 1024));
}

#[test]
fn unknown_osc_reports_metadata_at_powers_of_two() {
    let mut t = Terminal::new(20, 3);
    for _ in 0..9 {
        t.feed(b"\x1b]999;private payload\x07");
    }
    let counts: Vec<_> = t
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::UnknownOsc {
                command: Some(999),
                count,
                ..
            } => Some(*count),
            _ => None,
        })
        .collect();
    assert_eq!(counts, [1, 2, 4, 8]);
}

#[test]
fn slow_and_rapid_blink_have_distinct_phases_and_reset() {
    use cryptty::terminal::Blink;
    let mut t = Terminal::new(20, 3);
    t.feed(b"\x1b[5ma\x1b[6mb\x1b[25mc");
    assert_eq!(t.rows[0][0].style().blink, Blink::Slow);
    assert_eq!(t.rows[0][1].style().blink, Blink::Rapid);
    assert_eq!(t.rows[0][2].style().blink, Blink::None);
    assert_eq!(
        (0..4).map(|p| Blink::Slow.visible(p)).collect::<Vec<_>>(),
        [true, true, false, false]
    );
    assert_eq!(
        (0..4).map(|p| Blink::Rapid.visible(p)).collect::<Vec<_>>(),
        [true, false, true, false]
    );
}
