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
fn command_markers_join_colors_and_soft_wrapped_rows() {
    for (input, expected) in [
        (
            "\x1b[32mbrew\x1b[0m list \x1b[33m--versions\x1b[0m pinentry pinentry-mac",
            "brew list --versions pinentry pinentry-mac",
        ),
        (
            "paru -S vulkan-swrast chatgpt-desktop",
            "paru -S vulkan-swrast chatgpt-desktop",
        ),
        ("  $ git status  ", "git status"),
        (
            "curl -L 'https://example.com/a?q=1' | head -c 20 > out.txt",
            "curl -L 'https://example.com/a?q=1' | head -c 20 > out.txt",
        ),
    ] {
        for cols in [18, 100] {
            let mut s = session(input, cols, 10, Options::default());
            let target = s
                .targets
                .iter()
                .find(|t| t.text == expected)
                .expect("whole command target")
                .clone();
            if expected.len() > cols {
                assert!(target.segments.len() > 1);
            }
            let mut action = Action::Stay;
            for ch in target.label.chars() {
                action = s.key(Key::Char(ch));
            }
            assert!(matches!(action, Action::Copy(text) if text == expected));
        }
    }
}

#[test]
fn command_marker_suppresses_styled_fragments_but_keeps_paths() {
    let s = session(
        "\x1b[32mcat\x1b[0m \x1b[33m--number\x1b[0m src/main.rs",
        80,
        3,
        Options::default(),
    );
    assert_eq!(texts(&s), vec!["cat --number src/main.rs", "src/main.rs"]);
    let s = session(
        "cat --number src/main.rs",
        80,
        3,
        Options {
            overlap_max: 1.try_into().unwrap(),
            ..Options::default()
        },
    );
    assert_eq!(texts(&s), vec!["cat --number src/main.rs"]);
}

#[test]
fn command_detection_avoids_common_prose_and_incomplete_quotes() {
    for text in [
        "Run brew list --versions pinentry",
        "Use --help for more information.",
        "-S means sync packages",
        "git status",
        "# ordinary comment",
        "paru -S 'unfinished",
        "error: use --help",
        "options --",
    ] {
        let s = session(text, 80, 3, Options::default());
        assert!(texts(&s).is_empty(), "unexpected command: {text}");
    }
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
fn parenthesized_paths_exclude_trailing_chinese_comma() {
    for path in [
        "/Users/even/cryptty/src/main.rs",
        "./中文/a_(b).rs",
        "src/main.rs",
        r"C:\work\src\main.rs",
    ] {
        for suffix in ["", ":12:4"] {
            for cols in [16, 80] {
                let mut s = session(
                    &format!("({path}{suffix})，"),
                    cols,
                    8,
                    Options::default(),
                );
                assert_eq!(texts(&s), vec![path]);
                let label = s.targets[0].label.clone();
                let action = s.key(Key::Char(label.chars().next().unwrap()));
                assert!(matches!(action, Action::Copy(text) if text == path));
            }
        }
    }
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

#[test]
fn marker_opacity_config_defaults_and_bounds() {
    let defaults = crate::config::Config::parse("{}").unwrap().markers;
    assert_eq!(defaults.label_opacity.get(), 0.7);
    assert_eq!(defaults.indicator_opacity.get(), 0.85);
    for value in ["0", "0.7", "0.8", "1"] {
        let config = crate::config::Config::parse(&format!(
            "markers: {{label_opacity: {value}, indicator_opacity: {value}}}"
        ))
        .unwrap();
        assert_eq!(
            config.markers.label_opacity.get(),
            value.parse::<f32>().unwrap()
        );
        assert_eq!(
            config.markers.indicator_opacity.get(),
            value.parse::<f32>().unwrap()
        );
    }
    for field in ["label_opacity", "indicator_opacity"] {
        for value in ["-0.1", "1.1", ".nan", ".inf"] {
            assert!(
                crate::config::Config::parse(&format!(
                    "markers: {{{field}: {value}}}"
                ))
                .is_err()
            );
        }
    }
}

#[test]
fn marker_background_uses_existing_color_format() {
    let defaults = crate::config::Config::parse("{}").unwrap().markers;
    assert_eq!(defaults.label_background.0, 0xfef991);
    for value in ["#e8dca0", "0xE8DCA0"] {
        let config = crate::config::Config::parse(&format!(
            "markers: {{label_background: '{value}'}}"
        ))
        .unwrap();
        assert_eq!(config.markers.label_background.0, 0xe8dca0);
    }
    for value in ["#fff", "#gg0000", "#12345678", "red"] {
        assert!(
            crate::config::Config::parse(&format!(
                "markers: {{label_background: '{value}'}}"
            ))
            .is_err()
        );
    }
}

#[test]
fn leap_requires_empty_prefix_and_later_s_still_copies() {
    let text = (0..650)
        .map(|n| format!("src/file{n}.rs\r\n"))
        .collect::<String>();
    let mut s = session(&text, 24, 651, Options::default());
    assert!(s.starts_leap(Key::Char('s')));
    let target = s
        .targets
        .iter()
        .find(|t| t.label.ends_with('s'))
        .unwrap()
        .clone();
    let first = target.label.chars().next().unwrap();
    assert!(matches!(s.key(Key::Char(first)), Action::Stay));
    assert!(!s.starts_leap(Key::Char('s')));
    assert!(
        matches!(s.key(Key::Char('s')), Action::Copy(t) if t == target.text)
    );
    s.key(Key::Backspace);
    assert!(s.starts_leap(Key::Char('s')));
    let empty = session("ordinary prose", 24, 2, Options::default());
    assert!(empty.starts_leap(Key::Char('s')));
}

#[test]
fn prefix_dims_surroundings_and_backspace_restores_original_styles() {
    let text = (0..30)
        .map(|n| format!("src/中文{n}.rs\r\n"))
        .collect::<String>();
    let mut s = session(
        &format!("\x1b[7;2mcontext\x1b[0m\r\n{text}"),
        12,
        70,
        Options::default(),
    );
    let prefix = s.targets[0].label.chars().next().unwrap();
    let original = s.screen.visible_snapshot();
    s.key(Key::Char(prefix));
    let context = Position { row: 0, col: 0 };
    let (fg, bg) = original.colors(original.rows[0][0].style());
    let (faded, same_bg) = s.screen.colors(s.screen.rows[0][0].style());
    assert_ne!(fg, faded);
    assert_eq!(bg, same_bg);
    for target in &s.targets {
        let matching = target.label.starts_with(prefix);
        for segment in &target.segments {
            let row = segment.start.row;
            for col in segment.start.col..segment.start.col + segment.columns {
                let before = original.rows[row][col].style();
                let after = s.screen.rows[row][col].style();
                assert_eq!(before == after, matching);
            }
        }
    }
    s.key(Key::Backspace);
    for (before, after) in original.rows.iter().zip(&s.screen.rows) {
        assert!(before.iter().eq(after.iter()));
    }
    let vim = s.into_vim();
    assert_eq!(
        vim.screen.rows[context.row][context.col].style(),
        original.rows[context.row][context.col].style()
    );
}
