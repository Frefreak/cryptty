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
fn same_start_keeps_longest_target_and_other_starts_remain_available() {
    for cols in [18, 100] {
        let command = "/tmp/cryptty --renderer gpu -- /bin/sh";
        let mut s = session(
            "\x1b[32m/tmp/cryptty\x1b[0m --renderer gpu -- /bin/sh",
            cols,
            8,
            Options::default(),
        );
        assert_eq!(texts(&s), vec![command, "/bin/sh"]);
        let label = s.targets[0].label.clone();
        let mut action = Action::Stay;
        for ch in label.chars() {
            action = s.key(Key::Char(ch));
        }
        assert!(matches!(action, Action::Copy(text) if text == command));
    }
    let s = session(
        "\x1b[1m./file.rs some explanation\x1b[0m",
        80,
        3,
        Options::default(),
    );
    assert_eq!(texts(&s), vec!["./file.rs some explanation"]);
    let s = session(
        "\x1b[1m./file.rs\x1b[0m\x1b[8m/secret\x1b[0m",
        80,
        3,
        Options::default(),
    );
    assert_eq!(texts(&s), vec!["./file.rs"]);
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
fn command_markers_use_structure_without_command_names() {
    for text in [
        "sudo dnf upgrade 'kf6-*' 'qt6-*' 'plasma-*' 'kwin*' kdecoration sddm",
        "widget collect '*.log'",
        "widget collect *.log",
        "widget collect|gadget consume",
        "widget|gadget",
        "widget>result",
        "widget; gadget",
        "widget&&gadget",
        "widget collect > result",
        "widget collect < input",
        "widget collect && gadget consume",
        "widget collect; gadget consume",
    ] {
        for cols in [18, 100] {
            let mut s = session(text, cols, 10, Options::default());
            let target = s
                .targets
                .iter()
                .find(|t| t.text == text)
                .unwrap_or_else(|| panic!("missing command: {text}"))
                .clone();
            let mut action = Action::Stay;
            for ch in target.label.chars() {
                action = s.key(Key::Char(ch));
            }
            assert!(matches!(action, Action::Copy(copy) if copy == text));
        }
    }
}

#[test]
fn command_markers_use_leading_token_color() {
    for (input, expected) in [
        (
            "\x1b[35mand \x1b[32msudo\x1b[0m systemctl restart sddm",
            "and sudo systemctl restart sddm",
        ),
        (
            "\x1b[32mwidget\x1b[0m collect records",
            "widget collect records",
        ),
    ] {
        for cols in [18, 100] {
            let s = session(input, cols, 10, Options::default());
            assert_eq!(texts(&s), vec![expected]);
        }
    }
}

#[test]
fn command_structure_ignores_literal_operators_and_plain_phrases() {
    for text in [
        "widget 'a | b'",
        "widget \"a > b\"",
        "widget a\\|b",
        "widget 'a && b'",
        "widget 'a; b'",
        "widget '*'",
        "widget 'some * words'",
        "please read the instructions",
        "systemctl restart sddm",
        "and sudo systemctl restart sddm",
        "widget collect |",
        "widget collect >",
        "widget '*.log",
    ] {
        let s = session(text, 100, 3, Options::default());
        assert!(texts(&s).is_empty(), "unexpected target: {text}");
    }
}

#[test]
fn commands_after_prefixes_copy_the_chain_without_styled_fragments() {
    let command = concat!(
        "\x1b[32mcat\x1b[0m Makefile; ",
        "\x1b[32mcat\x1b[0m scripts/build-pgo; ",
        "\x1b[32mmake\x1b[0m help; ",
        "\x1b[32mmake\x1b[0m \x1b[33m-n\x1b[0m install-pgo ",
        "PREFIX=\x1b[33m'/tmp/cryptty prefix'\x1b[0m; ",
        "\x1b[32mcargo\x1b[0m fmt \x1b[33m--check\x1b[0m; ",
        "\x1b[32mgit\x1b[0m diff \x1b[33m--check\x1b[0m",
    );
    let expected = "cat Makefile; cat scripts/build-pgo; make help; \
        make -n install-pgo PREFIX='/tmp/cryptty prefix'; \
        cargo fmt --check; git diff --check";
    for prefix in ["Ran ", "Running ", "Preview: ", "└ ", "执行： "] {
        for cols in [40, 240] {
            let mut s = session(
                &format!("\x1b[1m{prefix}\x1b[0m{command}"),
                cols,
                10,
                Options::default(),
            );
            let target = s
                .targets
                .iter()
                .find(|t| t.text == expected)
                .expect("command without prefix")
                .clone();
            for fragment in ["cargo", "--check", "-n"] {
                assert!(!texts(&s).contains(&fragment));
            }
            assert!(texts(&s).contains(&"scripts/build-pgo"));
            let mut action = Action::Stay;
            for ch in target.label.chars() {
                action = s.key(Key::Char(ch));
            }
            assert!(matches!(action, Action::Copy(copy) if copy == expected));
        }
    }
}

#[test]
fn prefixed_commands_require_a_highlighted_leading_token() {
    for input in [
        "Running \x1b[32mwidget\x1b[0m collect records",
        "Preview: \x1b[32mwidget\x1b[0m collect records",
    ] {
        let s = session(input, 80, 3, Options::default());
        assert!(texts(&s).contains(&"widget collect records"));
    }
    for input in [
        "Run widget collect --all",
        "Read \x1b[34mthe instructions\x1b[0m carefully",
        "Note \x1b[33m--check\x1b[0m is an option",
    ] {
        let s = session(input, 80, 3, Options::default());
        assert!(!s.targets.iter().any(|t| {
            t.text.ends_with("--all")
                || t.text.ends_with("carefully")
                || t.text.ends_with("is an option")
        }));
    }
}

#[test]
fn prefixed_commands_keep_compact_operators() {
    for (input, expected) in [
        ("Ran \x1b[32mwidget\x1b[0m|gadget", "widget|gadget"),
        (
            "Ran \x1b[32mcat\x1b[0m; \x1b[32mmake\x1b[0m help",
            "cat; make help",
        ),
    ] {
        let s = session(input, 80, 3, Options::default());
        assert_eq!(texts(&s), vec![expected]);
    }
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
    assert_eq!(defaults.label_opacity.get(), 0.8);
    assert_eq!(defaults.label_embolden.pixels(), 0.3);
    assert_eq!(defaults.indicator_opacity.get(), 0.85);
    for value in ["0", "0.7", "0.8", "1"] {
        let config = crate::config::Config::parse(&format!(
            "markers: {{label_opacity: {value}, indicator_opacity: {value}, \
             label_embolden: {value}}}"
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
        assert_eq!(
            config.markers.label_embolden.pixels(),
            value.parse::<f32>().unwrap()
        );
    }
    for field in ["label_opacity", "indicator_opacity", "label_embolden"] {
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
fn marker_accents_accept_custom_and_empty_palettes() {
    let config = crate::config::Config::parse(
        "markers: {accent_colors: ['#102030', '0xaabbcc']}",
    )
    .unwrap();
    assert_eq!(
        config
            .markers
            .accent_colors
            .iter()
            .map(|c| c.0)
            .collect::<Vec<_>>(),
        vec![0x102030, 0xaabbcc]
    );
    let config =
        crate::config::Config::parse("markers: {accent_colors: []}").unwrap();
    assert!(config.markers.accent_colors.is_empty());
    assert!(
        crate::config::Config::parse("markers: {accent_colors: ['invalid']}",)
            .is_err()
    );
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
