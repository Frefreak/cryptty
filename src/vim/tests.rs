use super::*;
use crate::terminal::{Content, TextRange};

fn session(text: &str, cols: usize, rows: usize) -> Session {
    let mut terminal = Terminal::new(cols, rows);
    terminal.feed(text.as_bytes());
    let mut session = Session::new(terminal.visible_snapshot(), Vec::new());
    session.phase = Phase::Navigate;
    session.first_key = false;
    session.render();
    session.cursor.position = Position { row: 0, col: 0 };
    session
}

fn keys(session: &mut Session, text: &str) -> Action {
    let mut result = Action::Stay;
    for ch in text.chars() {
        result = session.key(Key::Char(ch), Instant::now());
    }
    result
}

fn copied(action: Action) -> String {
    match action {
        Action::Copy(text) => text,
        _ => panic!("expected copy"),
    }
}

#[test]
fn personal_selection_keys_and_single_cell_yank() {
    let mut s = session("abc def", 20, 3);
    keys(&mut s, "v");
    assert_eq!(s.selection.unwrap().kind, SelectionKind::Block);
    s.key(Key::Control('v'), Instant::now());
    assert_eq!(s.selection.unwrap().kind, SelectionKind::Character);
    assert_eq!(copied(keys(&mut s, "y")), "a");
    keys(&mut s, "V");
    assert_eq!(s.selection.unwrap().kind, SelectionKind::Line);
    assert_eq!(copied(keys(&mut s, "y")), "abc def\n");
}

#[test]
fn word_counts_aliases_and_yank_operators() {
    let mut s = session("  alpha,beta gamma", 30, 3);
    keys(&mut s, "H");
    assert_eq!(s.cursor.position.col, 2);
    keys(&mut s, "2w");
    assert_eq!(s.cursor.position.col, 8);
    keys(&mut s, "b");
    assert_eq!(s.cursor.position.col, 7);
    keys(&mut s, "L");
    assert_eq!(s.cursor.position.col, 17);
    keys(&mut s, "H");
    assert_eq!(copied(keys(&mut s, "y2w")), "alpha,");
    let mut s = session("alpha beta gamma", 30, 3);
    assert_eq!(copied(keys(&mut s, "2yw")), "alpha beta ");
    let mut s = session("one\r\ntwo\r\nthree", 12, 4);
    assert_eq!(copied(keys(&mut s, "2yy")), "one\ntwo\n");
}

#[test]
fn wide_characters_are_atomic_and_soft_wraps_copy_without_newlines() {
    let mut s = session("你e\u{301}好 xyz", 5, 3);
    keys(&mut s, "l");
    assert_eq!(s.cursor.position.col, 2);
    keys(&mut s, "l");
    assert_eq!(s.cursor.position.col, 3);
    keys(&mut s, "l");
    assert_eq!(s.cursor.position, Position { row: 1, col: 0 });
    keys(&mut s, "0");
    assert_eq!(s.cursor.position, Position { row: 0, col: 0 });
    s.key(Key::Control('v'), Instant::now());
    keys(&mut s, "L");
    assert_eq!(copied(keys(&mut s, "y")), "你e\u{301}好 xyz");
}

#[test]
fn block_selection_keeps_rectangular_rows_and_expands_wide_edges() {
    let mut s = session("a你cd\r\nab好d", 8, 3);
    keys(&mut s, "lvjl");
    assert_eq!(copied(keys(&mut s, "y")), "你\nb好");
    assert!(s.screen.rows[0][1].style().inverse);
    assert!(s.screen.rows[0][2].style().inverse);
}

#[test]
fn vertical_navigation_retains_preferred_column_and_stays_in_pane() {
    let mut s = session("abcdef\r\nx\r\nabcdef", 8, 3);
    keys(&mut s, "4lj");
    assert_eq!(s.cursor.position.col, 0);
    keys(&mut s, "j");
    assert_eq!(s.cursor.position.col, 4);
    keys(&mut s, "9999j9999l");
    assert_eq!(s.cursor.position, Position { row: 2, col: 5 });
}

#[test]
fn kj_always_navigates_without_exiting() {
    let mut s = session("one\r\ntwo", 10, 3);
    keys(&mut s, "j");
    assert!(matches!(keys(&mut s, "kj"), Action::Stay));
    assert_eq!(s.cursor.position.row, 1);
    keys(&mut s, "v");
    assert!(matches!(keys(&mut s, "kj"), Action::Stay));
    assert_eq!(s.cursor.position.row, 1);
    assert!(s.selection.is_some());
}

#[test]
fn escape_unwinds_leap_then_selection_then_mode() {
    let mut s = session("alpha beta", 20, 2);
    keys(&mut s, "vs");
    assert!(matches!(s.key(Key::Escape, Instant::now()), Action::Stay));
    assert!(s.selection.is_some());
    assert!(matches!(s.key(Key::Escape, Instant::now()), Action::Stay));
    assert!(s.selection.is_none());
    assert!(matches!(s.key(Key::Escape, Instant::now()), Action::Exit));
}

#[test]
fn unique_leap_lands_then_navigation_and_copy_use_that_cursor() {
    let mut s = session("alpha beta gamma", 30, 2);
    keys(&mut s, "sbe");
    assert_eq!(s.cursor.position.col, 6);
    s.key(Key::Control('v'), Instant::now());
    keys(&mut s, "e");
    assert_eq!(copied(keys(&mut s, "y")), "beta");
}

#[test]
fn safe_auto_jump_reuses_non_label_input_as_navigation() {
    let mut s = session("zero abcd abcd", 30, 2);
    keys(&mut s, "sab");
    assert_eq!(s.cursor.position.col, 5);
    assert!(matches!(s.phase, Phase::Leap(_)));
    keys(&mut s, "l");
    assert_eq!(s.cursor.position.col, 6);
    assert!(matches!(s.phase, Phase::Navigate));
    keys(&mut s, "sab");
    keys(&mut s, "s");
    assert_eq!(s.cursor.position.col, 5);
}

#[test]
fn frozen_screen_does_not_follow_live_output_or_labels() {
    let mut live = Terminal::new(20, 3);
    live.feed(b"alpha beta");
    let frozen = live.visible_snapshot();
    let rows = frozen.rows.clone();
    let mut s = Session::new(frozen, Vec::new());
    live.feed(b"\r\nnew output");
    keys(&mut s, "be");
    assert_eq!(s.base.rows, rows);
    assert_eq!(s.screen.rows, rows);
    assert_ne!(s.base.rows, live.rows);
}

#[test]
fn leap_searches_both_panes_but_selection_and_motions_stay_local() {
    let mut display = Terminal::new(21, 3);
    display.feed(b"left text |right text");
    let mut left = Terminal::new(10, 3);
    left.feed(b"left text");
    let mut right = Terminal::new(10, 3);
    right.feed(b"right text");
    let mut s = Session::new(
        display,
        vec![
            PaneText {
                origin: Position { row: 0, col: 0 },
                terminal: left,
            },
            PaneText {
                origin: Position { row: 0, col: 11 },
                terminal: right,
            },
        ],
    );
    keys(&mut s, "le");
    assert_eq!(s.cursor.pane, 0);
    keys(&mut s, "sri");
    assert_eq!(s.cursor.pane, 1);
    keys(&mut s, "9999h");
    assert_eq!(s.cursor.position.col, 0);
    s.key(Key::Control('v'), Instant::now());
    keys(&mut s, "L");
    assert_eq!(copied(keys(&mut s, "y")), "right text");
    assert!(!s.screen.rows[0][10].style().inverse);
    keys(&mut s, "sle");
    assert_eq!(s.cursor.pane, 1);
}

#[test]
fn labels_do_not_modify_copied_text_or_break_wide_cells() {
    let mut s = session("start 你好 你好", 24, 2);
    keys(&mut s, "s你");
    for row in &s.screen.rows {
        for (col, cell) in row.iter().enumerate() {
            if matches!(cell.content, Content::Continuation) {
                assert!(col > 0);
                assert!(matches!(&row[col - 1].content,
                    Content::Text { text } if text.columns() == 2));
            }
        }
    }
    assert_eq!(
        s.panes[0].terminal.selected_text(TextRange::between(
            Position { row: 0, col: 0 },
            Position { row: 0, col: 15 },
        )),
        "start 你好 你好"
    );
}

#[test]
fn successful_leap_retry_clears_failure_status() {
    let mut s = session("zero abcd abcd", 30, 2);
    keys(&mut s, "szz");
    assert!(s.status().contains("no matches"));
    keys(&mut s, "ab");
    assert!(!s.status().contains("no matches"));
}

#[test]
fn leap_smartcase_applies_to_the_whole_pair() {
    let mut s = session("zero Ab aB", 20, 2);
    keys(&mut s, "saB");
    assert_eq!(s.cursor.position.col, 8);
}

#[test]
fn leap_label_groups_reach_targets_beyond_the_first_page() {
    let mut s = session(&"ab ".repeat(90), 300, 2);
    keys(&mut s, "sab ");
    assert!(s.status().contains("group 2"));
    keys(&mut s, "s");
    assert!(s.cursor.position.col > 120);
    assert!(matches!(s.phase, Phase::Navigate));
}

#[test]
fn one_character_enter_and_repeat_keep_the_search_cursor() {
    let mut s = session("zero alpha beta", 30, 2);
    keys(&mut s, "sb");
    s.key(Key::Enter, Instant::now());
    assert_eq!(s.cursor.position.col, 11);
    keys(&mut s, "sal");
    assert_eq!(s.cursor.position.col, 5);
    keys(&mut s, "0s");
    s.key(Key::Enter, Instant::now());
    assert_eq!(s.cursor.position.col, 5);
}

#[test]
fn one_character_shortcut_replaces_the_previous_repeat_query() {
    let mut s = session("zero alpha beta", 30, 2);
    keys(&mut s, "sal");
    assert_eq!(s.cursor.position.col, 5);
    keys(&mut s, "sb");
    s.key(Key::Enter, Instant::now());
    assert_eq!(s.cursor.position.col, 11);
    keys(&mut s, "0s");
    s.key(Key::Enter, Instant::now());
    assert_eq!(s.cursor.position.col, 11);
}

#[test]
fn find_directions_counts_and_repeat() {
    let mut s = session("a x b x c x", 20, 3);
    keys(&mut s, "2fx");
    assert_eq!(s.cursor.position.col, 6);
    keys(&mut s, ";");
    assert_eq!(s.cursor.position.col, 10);
    keys(&mut s, ",");
    assert_eq!(s.cursor.position.col, 6);
    keys(&mut s, ";");
    assert_eq!(s.cursor.position.col, 10);
    keys(&mut s, "2Fx");
    assert_eq!(s.cursor.position.col, 2);
    keys(&mut s, "0tx");
    assert_eq!(s.cursor.position.col, 1);
    keys(&mut s, ";");
    assert_eq!(s.cursor.position.col, 5);
    keys(&mut s, "2; fz");
    assert_eq!(s.cursor.position.col, 5);
    keys(&mut s, "$Tx");
    assert_eq!(s.cursor.position.col, 7);
    keys(&mut s, ";");
    assert_eq!(s.cursor.position.col, 3);
}

#[test]
fn finds_extend_every_selection_kind_and_yank_motion() {
    for key in [Key::Char('v'), Key::Control('v'), Key::Char('V')] {
        let mut s = session("a x b x", 20, 3);
        s.key(key, Instant::now());
        keys(&mut s, "2fx");
        assert_eq!(s.cursor.position.col, 6);
        keys(&mut s, "Tx");
        assert_eq!(s.cursor.position.col, 3);
        keys(&mut s, "tx");
        assert_eq!(s.cursor.position.col, 5);
        keys(&mut s, "Fx");
        assert_eq!(s.cursor.position.col, 2);
        let expected = if key == Key::Char('V') {
            "a x b x\n"
        } else {
            "a x"
        };
        assert_eq!(copied(keys(&mut s, "y")), expected);
    }
    let mut s = session("a x b x", 20, 3);
    assert_eq!(copied(keys(&mut s, "y2fx")), "a x b x");
    let mut s = session("a x b x", 20, 3);
    assert_eq!(copied(keys(&mut s, "ytx")), "a ");
    keys(&mut s, "$");
    assert_eq!(copied(keys(&mut s, "yFx")), "x b ");
    assert_eq!(copied(keys(&mut s, "yTx")), " b ");
}

#[test]
fn find_is_literal_case_sensitive_and_cancellable() {
    let mut s = session("a1kjxX", 20, 3);
    keys(&mut s, "f1");
    assert_eq!(s.cursor.position.col, 1);
    keys(&mut s, "fk");
    assert!(matches!(keys(&mut s, "j"), Action::Stay));
    keys(&mut s, "k0fX");
    assert_eq!(s.cursor.position.col, 5);
    keys(&mut s, "vf");
    assert!(s.status().contains("character"));
    assert!(matches!(s.key(Key::Escape, Instant::now()), Action::Stay));
    assert!(s.selection.is_some());
    keys(&mut s, "Fz");
    assert_eq!(s.cursor.position.col, 5);
    assert!(s.selection.is_some());
}

#[test]
fn find_respects_logical_lines_and_wide_cells() {
    let mut s = session("a你b好c\r\nx", 5, 4);
    keys(&mut s, "t好");
    assert_eq!(s.cursor.position, Position { row: 0, col: 3 });
    keys(&mut s, "f好");
    assert_eq!(s.cursor.position, Position { row: 1, col: 0 });
    keys(&mut s, "fx");
    assert_eq!(s.cursor.position, Position { row: 1, col: 0 });
    keys(&mut s, "T你");
    assert_eq!(s.cursor.position, Position { row: 0, col: 3 });
    keys(&mut s, "F你");
    assert_eq!(s.cursor.position, Position { row: 0, col: 1 });
}

#[test]
fn backward_till_yank_with_adjacent_target_keeps_the_mode() {
    let mut s = session("axb", 10, 2);
    keys(&mut s, "$");
    assert!(matches!(keys(&mut s, "yTx"), Action::Stay));
    assert_eq!(s.cursor.position.col, 2);
    assert!(s.selection.is_none());
    assert!(matches!(s.phase, Phase::Navigate));
}

#[test]
fn autojump_and_traversal_cross_panes_but_selection_stays_local() {
    let mut display = Terminal::new(21, 3);
    display.cursor = (0, 0);
    let mut left = Terminal::new(10, 3);
    left.feed(b"a ab");
    let mut right = Terminal::new(10, 3);
    right.feed(b"b ab ab");
    let mut s = Session::new(
        display,
        vec![
            PaneText {
                origin: Position { row: 0, col: 0 },
                terminal: left,
            },
            PaneText {
                origin: Position { row: 0, col: 11 },
                terminal: right,
            },
        ],
    );
    keys(&mut s, "ab");
    assert_eq!(s.cursor.pane, 0);
    assert_eq!(s.cursor.position.col, 2);
    s.key(Key::Enter, Instant::now());
    assert_eq!(s.cursor.pane, 1);
    assert_eq!(s.cursor.position.col, 2);
    s.key(Key::Enter, Instant::now());
    assert_eq!(s.cursor.position.col, 5);
    s.key(Key::Backspace, Instant::now());
    assert_eq!(s.cursor.position.col, 2);
    s.key(Key::Escape, Instant::now());
    assert_eq!(s.cursor.pane, 0);
    assert_eq!(s.cursor.position.col, 0);
    keys(&mut s, "sabs");
    assert_eq!(s.cursor.pane, 1);
    assert_eq!(s.cursor.position.col, 2);
    s.key(Key::Control('v'), Instant::now());
    keys(&mut s, "sab");
    assert_eq!(s.cursor.pane, 1);
    assert_eq!(s.cursor.position.col, 5);
    assert_eq!(copied(keys(&mut s, "y")), "ab a");
}

#[test]
fn autojump_retains_the_safe_label_limit() {
    for (count, expected_col) in [(18, 5), (19, 0)] {
        let mut s = session(&format!("zero {}", "ab ".repeat(count)), 80, 3);
        keys(&mut s, "sab");
        assert_eq!(s.cursor.position.col, expected_col);
    }
}

#[test]
fn autojump_cursor_keeps_original_glyph_and_other_targets_keep_labels() {
    let mut s = session("zero ab ab ab", 24, 3);
    keys(&mut s, "sab");
    let check = |s: &Session, col: usize| {
        assert_eq!(s.cursor.position.col, col);
        assert_eq!(s.screen.cursor, (col, 0));
        assert_eq!(s.screen.rows[0][col], s.base.rows[0][col]);
        let Phase::Leap(search) = &s.phase else {
            panic!("expected Leap")
        };
        let beacons = search.beacons(&s.panes);
        assert!(!beacons.iter().any(|(location, _)| *location == s.cursor));
        assert_eq!(
            beacons.iter().filter(|(_, label)| label.is_some()).count(),
            2
        );
    };
    check(&s, 5);
    s.key(Key::Enter, Instant::now());
    check(&s, 8);
    s.key(Key::Enter, Instant::now());
    check(&s, 11);
    s.key(Key::Enter, Instant::now());
    check(&s, 11);
    s.key(Key::Backspace, Instant::now());
    check(&s, 8);
    // The vacated first match receives the selected match's old label.
    keys(&mut s, "s");
    assert_eq!(s.cursor.position.col, 5);
    assert!(matches!(s.phase, Phase::Navigate));
}

#[test]
fn selected_wide_leap_match_preserves_character_and_selection_style() {
    let mut s = session("zero 你好 你好", 24, 3);
    s.key(Key::Control('v'), Instant::now());
    keys(&mut s, "s你好");
    assert_eq!(s.cursor.position.col, 5);
    for col in [5, 6] {
        assert_eq!(s.screen.rows[0][col].content, s.base.rows[0][col].content);
        assert!(s.screen.rows[0][col].style().inverse);
    }
    s.key(Key::Enter, Instant::now());
    assert_eq!(s.cursor.position.col, 10);
    for col in [10, 11] {
        assert_eq!(s.screen.rows[0][col].content, s.base.rows[0][col].content);
        assert!(s.screen.rows[0][col].style().inverse);
    }
}

#[test]
fn first_escape_exits_fresh_leap_session() {
    let mut terminal = Terminal::new(30, 3);
    terminal.feed(b"alpha beta gamma");
    let mut s = Session::new(terminal.visible_snapshot(), Vec::new());
    assert!(matches!(s.phase, Phase::Leap(_)));
    assert!(matches!(s.key(Key::Escape, Instant::now()), Action::Exit));
}

#[test]
fn escape_after_search_input_cancels_before_exiting() {
    let mut terminal = Terminal::new(30, 3);
    terminal.feed(b"alpha beta gamma");
    let mut s = Session::new(terminal.visible_snapshot(), Vec::new());
    assert!(matches!(
        s.key(Key::Char('b'), Instant::now()),
        Action::Stay
    ));
    assert!(matches!(s.key(Key::Escape, Instant::now()), Action::Stay));
    assert!(matches!(s.phase, Phase::Navigate));
    assert!(matches!(s.key(Key::Escape, Instant::now()), Action::Exit));
}

#[test]
fn q_unwinds_selection_then_exits_navigation() {
    let mut s = session("alpha beta", 20, 2);
    keys(&mut s, "v2");
    assert!(matches!(keys(&mut s, "q"), Action::Stay));
    assert!(s.selection.is_none());
    assert_eq!(s.count, 0);
    assert!(matches!(keys(&mut s, "q"), Action::Exit));
}

#[test]
fn q_remains_a_search_character() {
    let mut s = session("a quick fox", 20, 2);
    assert!(matches!(keys(&mut s, "squ"), Action::Stay));
    assert_eq!(s.cursor.position.col, 2);
    for (command, column) in [("fq", 2), ("tq", 1)] {
        let mut s = session("a quick fox", 20, 2);
        assert!(matches!(keys(&mut s, command), Action::Stay));
        assert_eq!(s.cursor.position.col, column);
    }
}
