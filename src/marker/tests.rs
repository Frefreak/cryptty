use super::*;

fn session(text: &str, cols: usize, rows: usize, options: Options) -> Session {
    let mut terminal = Terminal::new(cols, rows);
    terminal.feed(text.as_bytes());
    Session::new(terminal.visible_snapshot(), Vec::new(), &options)
}

fn texts(session: &Session) -> Vec<&str> {
    session.targets.iter().map(|t| t.text.as_str()).collect()
}

#[test]
fn recognizes_requested_targets_without_plain_prose() {
    let s = session(
        "https://example.com/a/b src/main.rs:42:3 192.168.1.2 ::1\r\n\
         550e8400-e29b-41d4-a716-446655440000 deadbee 1234567\r\n\
         connection refused user_id=abc-123",
        100,
        5,
        Options::default(),
    );
    assert_eq!(
        texts(&s),
        vec![
            "https://example.com/a/b",
            "src/main.rs",
            "192.168.1.2",
            "::1",
            "550e8400-e29b-41d4-a716-446655440000",
            "deadbee",
        ]
    );
}

#[test]
fn styled_phrase_and_inner_path_get_distinct_immediate_labels() {
    let s = session(
        "\x1b[32mOpen src/main.rs to continue\x1b[0m",
        80,
        3,
        Options::default(),
    );
    assert_eq!(
        texts(&s),
        vec!["Open src/main.rs to continue", "src/main.rs"]
    );
    assert_ne!(s.targets[0].label, s.targets[1].label);
    let options = Options {
        overlap_max: 1.try_into().unwrap(),
        ..Options::default()
    };
    let s =
        session("\x1b[1mOpen src/main.rs to continue\x1b[0m", 80, 3, options);
    assert_eq!(texts(&s), vec!["src/main.rs"]);
}

#[test]
fn styled_threshold_counts_unicode_and_internal_spaces() {
    let s = session(
        "\x1b[32ma bc\x1b[0m \x1b[1ma b c\x1b[0m \x1b[31m你好 世界\x1b[0m",
        80,
        3,
        Options::default(),
    );
    assert_eq!(texts(&s), vec!["a b c", "你好 世界"]);
    assert_eq!(s.targets[1].segments[0].columns, 9);
}

#[test]
fn wrapped_unicode_path_preserves_combining_marks_and_coordinates() {
    let s = session("./中文/e\u{301}.rs:12:4", 8, 4, Options::default());
    assert_eq!(texts(&s), vec!["./中文/e\u{301}.rs"]);
    assert_eq!(s.targets[0].segments.len(), 2);
    assert_eq!(s.targets[0].segments[0].columns, 8);
}

#[test]
fn pane_boundaries_do_not_join_text() {
    let mut a = Terminal::new(20, 2);
    a.feed(b"src/main.rs");
    let mut b = Terminal::new(20, 2);
    b.feed(b"other/test.rs");
    let s = Session::new(
        Terminal::new(50, 4),
        vec![
            PaneText {
                origin: Position { row: 1, col: 2 },
                terminal: a,
            },
            PaneText {
                origin: Position { row: 1, col: 25 },
                terminal: b,
            },
        ],
        &Options::default(),
    );
    assert_eq!(texts(&s), vec!["src/main.rs", "other/test.rs"]);
    assert_eq!(s.targets[1].segments[0].start, Position { row: 1, col: 25 });
}

#[test]
fn labels_are_prefix_free_and_copy_exactly_then_escape_cancels() {
    let text = (0..30)
        .map(|n| format!("src/file{n}.rs\r\n"))
        .collect::<String>();
    let mut s = session(&text, 40, 40, Options::default());
    assert_eq!(s.targets.len(), 30);
    let target = s.targets[26].clone();
    assert_eq!(target.label.len(), 2);
    let mut chars = target.label.chars();
    assert!(matches!(
        s.key(Key::Char(chars.next().unwrap())),
        Action::Stay
    ));
    assert!(s.visible_targets().len() < 30);
    assert!(
        matches!(s.key(Key::Char(chars.next().unwrap())), Action::Copy(t) if t == target.text)
    );
    assert!(matches!(s.key(Key::Backspace), Action::Stay));
    assert_eq!(s.visible_targets().len(), 30);
    assert!(matches!(s.key(Key::Escape), Action::Exit));
}

#[test]
fn punctuation_and_url_internals_do_not_create_extra_targets() {
    let s = session(
        "(https://example.com/path_(x)), ./src/a.rs:2:9; [::1]",
        100,
        3,
        Options::default(),
    );
    assert_eq!(
        texts(&s),
        vec!["https://example.com/path_(x)", "./src/a.rs", "::1"]
    );
}

#[test]
fn invalid_options_rejected_at_parse_time() {
    assert!(
        crate::config::Config::parse("markers:\n  overlap_max: 0").is_err()
    );
    assert!(
        crate::config::Config::parse("markers:\n  hash_min_length: 0").is_err()
    );
}

#[test]
fn styled_url_fragment_is_suppressed_but_containing_phrase_survives() {
    let s = session(
        "https://example.com/\x1b[32mhello\x1b[0m",
        80,
        3,
        Options::default(),
    );
    assert_eq!(texts(&s), vec!["https://example.com/hello"]);
    let s = session(
        "\x1b[32mVisit https://example.com/hello now\x1b[0m",
        80,
        3,
        Options::default(),
    );
    assert_eq!(
        texts(&s),
        vec![
            "Visit https://example.com/hello now",
            "https://example.com/hello"
        ]
    );
}

#[test]
fn ip_fragments_inside_code_identifiers_are_not_targets() {
    let s = session(
        "even-cloud-core::connection::Connection abc::defg foo192.168.1.2\r\n\
         [2001:db8::1] ::1 192.168.1.2",
        100,
        4,
        Options::default(),
    );
    assert_eq!(texts(&s), vec!["2001:db8::1", "::1", "192.168.1.2"]);
}

#[test]
fn ip_boundaries_allow_ports_and_sentence_punctuation() {
    let s = session(
        "192.168.1.2:8080 192.168.1.3. [::1]:80 ::2. \r\n\
         192.168.1.2.3 192.168.1.2:port code::face",
        100,
        4,
        Options::default(),
    );
    assert_eq!(texts(&s), vec!["192.168.1.2", "192.168.1.3", "::1", "::2"]);
}

#[test]
fn explicit_default_foreground_does_not_mark_body_prose() {
    let s = session(
        "\x1b[37mRequests carry a service identifier\x1b[0m\r\n\
         \x1b[38;2;248;248;242mResponses carry the final status\x1b[0m\r\n\
         \x1b[33mContent-Encoding\x1b[0m \x1b[1;37mImportant\x1b[0m",
        80,
        5,
        Options::default(),
    );
    assert_eq!(texts(&s), vec!["Content-Encoding", "Important"]);
}

#[test]
fn styled_detection_uses_frozen_display_palette_for_plain_tmux_capture() {
    let mut display = Terminal::new(80, 3);
    display.feed(
        b"\x1b]10;#aabbcc\x1b\\\x1b]4;1;#aabbcc\x1b\\\
        \x1b[31mOrdinary text\x1b[0m \x1b[33mHighlighted\x1b[0m",
    );
    let mut pane = display.visible_snapshot();
    pane.set_theme(crate::terminal::Theme::default());
    let s = Session::new(
        display,
        vec![PaneText {
            origin: Position { row: 0, col: 0 },
            terminal: pane,
        }],
        &Options::default(),
    );
    assert_eq!(texts(&s), vec!["Highlighted"]);
}
