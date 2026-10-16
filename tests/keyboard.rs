use cryptty::input::kitty::{KeyInput, encode};
use cryptty::keyboard::Flags;
use cryptty::terminal::Terminal;
use winit::event::ElementState;
use winit::keyboard::{
    Key, KeyCode, KeyLocation, ModifiersState as M, NamedKey, PhysicalKey,
};

fn event(key: &Key) -> KeyInput<'_> {
    KeyInput {
        key,
        unmodified: key.clone(),
        physical: PhysicalKey::Code(KeyCode::Enter),
        location: KeyLocation::Standard,
        text: None,
        modifiers: M::empty(),
        locks: 0,
        state: ElementState::Pressed,
        repeat: false,
    }
}

fn negotiated_key(terminal: &Terminal, event: &KeyInput<'_>) -> Vec<u8> {
    cryptty::input::encode_key(
        event,
        terminal.keyboard.flags(),
        terminal.modify_other_keys_2,
        terminal.application_cursor,
        terminal.application_keypad,
    )
}

#[test]
fn tmux_mode_two_negotiation_and_kitty_precedence() {
    let mut terminal = Terminal::new(80, 24);
    let key = Key::Named(NamedKey::Enter);
    let mut e = event(&key);
    e.modifiers = M::SHIFT;
    assert_eq!(negotiated_key(&terminal, &e), b"\r");
    for byte in b"\x1b[>4;2m" {
        terminal.feed(&[*byte]);
    }
    assert_eq!(terminal.feed(b"\x1b[?4m"), b"\x1b[>4;2m");
    assert_eq!(negotiated_key(&terminal, &e), b"\x1b[27;2;13~");
    terminal.feed(b"\x1b[?1049h");
    assert_eq!(negotiated_key(&terminal, &e), b"\x1b[27;2;13~");
    terminal.feed(b"\x1b[>5u");
    assert_eq!(negotiated_key(&terminal, &e), b"\x1b[13;2u");
    terminal.feed(b"\x1b[<u\x1b[?1049l");
    assert_eq!(negotiated_key(&terminal, &e), b"\x1b[27;2;13~");
    e.modifiers = M::empty();
    assert_eq!(negotiated_key(&terminal, &e), b"\r");
    e.state = ElementState::Released;
    assert!(negotiated_key(&terminal, &e).is_empty());
    for reset in ["\x1b[>m", "\x1b[>4m", "\x1b[>4;0m", "\x1b[>4n", "\x1bc"] {
        terminal.feed(b"\x1b[>4;2m");
        terminal.feed(reset.as_bytes());
        assert_eq!(terminal.feed(b"\x1b[?4m"), b"\x1b[>4;0m");
    }
}

#[test]
fn tmux_mode_two_preserves_text_and_encodes_modifiers() {
    let mut terminal = Terminal::new(80, 24);
    terminal.feed(b"\x1b[>4;2m");
    for (key, text, modifiers, expected) in [
        (Key::Character("a".into()), "a", M::empty(), "a"),
        (Key::Character("A".into()), "A", M::SHIFT, "\x1b[27;2;65~"),
        (Key::Character("!".into()), "!", M::SHIFT, "!"),
        (
            Key::Character("b".into()),
            "\x02",
            M::CONTROL,
            "\x1b[27;5;98~",
        ),
        (
            Key::Character("H".into()),
            "\x08",
            M::CONTROL | M::SHIFT,
            "\x1b[27;6;72~",
        ),
        (Key::Named(NamedKey::Tab), "\t", M::SHIFT, "\x1b[27;2;9~"),
        (Key::Named(NamedKey::ArrowUp), "", M::SHIFT, "\x1b[1;2A"),
        (Key::Named(NamedKey::Enter), "你好", M::SHIFT, "你好"),
    ] {
        let mut e = event(&key);
        e.text = Some(text);
        e.modifiers = modifiers;
        assert_eq!(negotiated_key(&terminal, &e), expected.as_bytes());
    }
    #[cfg(target_os = "macos")]
    {
        let key = Key::Character("{".into());
        let mut e = event(&key);
        e.unmodified = Key::Character("8".into());
        e.text = Some("{");
        e.modifiers = M::ALT;
        assert_eq!(negotiated_key(&terminal, &e), b"{");
    }
    terminal.feed(b"\x1b=");
    let key = Key::Named(NamedKey::Enter);
    let mut e = event(&key);
    e.modifiers = M::SHIFT;
    e.location = KeyLocation::Numpad;
    assert_eq!(negotiated_key(&terminal, &e), b"\x1bOM");
}

#[test]
fn shift_enter_and_safe_unmodified_keys_match_ghostty() {
    for (key, plain, shifted) in [
        (NamedKey::Enter, b"\r".as_slice(), b"\x1b[13;2u".as_slice()),
        (NamedKey::Tab, b"\t", b"\x1b[9;2u"),
        (NamedKey::Backspace, b"\x7f", b"\x1b[127;2u"),
    ] {
        let key = Key::Named(key);
        let mut e = event(&key);
        assert_eq!(encode(&e, Flags::new(5)), plain);
        e.modifiers = M::SHIFT;
        assert_eq!(encode(&e, Flags::new(5)), shifted);
        e.state = ElementState::Released;
        assert_eq!(encode(&e, Flags::new(7)), b"");
        assert!(!encode(&e, Flags::new(31)).is_empty());
    }
}

#[test]
fn alternate_keys_repeat_release_and_associated_text() {
    let key = Key::Character("A".into());
    let mut e = event(&key);
    e.unmodified = Key::Character("a".into());
    e.physical = PhysicalKey::Code(KeyCode::KeyA);
    e.text = Some("A");
    e.modifiers = M::SHIFT;
    assert_eq!(encode(&e, Flags::new(5)), b"A");
    e.modifiers |= M::CONTROL;
    assert_eq!(encode(&e, Flags::new(5)), b"\x1b[97:65;6u");
    e.repeat = true;
    assert_eq!(encode(&e, Flags::new(7)), b"\x1b[97:65;6:2u");
    e.state = ElementState::Released;
    assert_eq!(encode(&e, Flags::new(7)), b"\x1b[97:65;6:3u");
    e.state = ElementState::Pressed;
    e.repeat = false;
    e.modifiers = M::SHIFT;
    assert_eq!(encode(&e, Flags::new(31)), b"\x1b[97:65;2;65u");
}

#[test]
fn produced_text_and_post_event_modifier_state_are_preserved() {
    let key = Key::Character("a".into());
    let mut e = event(&key);
    e.physical = PhysicalKey::Code(KeyCode::KeyA);
    e.text = Some("a");
    e.modifiers = M::SHIFT;
    e.locks = 64;
    assert_eq!(encode(&e, Flags::new(5)), b"a");

    let key = Key::Named(NamedKey::Control);
    let mut e = event(&key);
    e.state = ElementState::Released;
    e.modifiers = M::CONTROL;
    assert_eq!(encode(&e, Flags::new(31)), b"\x1b[57442;5:3u");
    e.modifiers = M::empty();
    assert_eq!(encode(&e, Flags::new(31)), b"\x1b[57442;1:3u");

    let key = Key::Named(NamedKey::Enter);
    let mut e = event(&key);
    e.text = Some("中文");
    assert_eq!(encode(&e, Flags::new(31)), "中文".as_bytes());
}

#[cfg(target_os = "macos")]
#[test]
fn option_composed_text_survives_negotiated_modes() {
    let key = Key::Character("{".into());
    let mut e = event(&key);
    e.unmodified = Key::Character("8".into());
    e.physical = PhysicalKey::Code(KeyCode::Digit8);
    e.text = Some("{");
    e.modifiers = M::ALT;
    assert_eq!(encode(&e, Flags::new(5)), b"{");
    assert_eq!(encode(&e, Flags::new(31)), b"\x1b[56;3;123u");
}

#[test]
fn function_keypad_and_release_encoding_match_ghostty() {
    let key = Key::Named(NamedKey::F3);
    let mut e = event(&key);
    assert_eq!(encode(&e, Flags::new(5)), b"\x1b[13~");
    e.modifiers = M::CONTROL;
    assert_eq!(encode(&e, Flags::new(7)), b"\x1b[13;5~");
    let key = Key::Named(NamedKey::Enter);
    let mut e = event(&key);
    e.location = KeyLocation::Numpad;
    e.physical = PhysicalKey::Code(KeyCode::NumpadEnter);
    assert_eq!(encode(&e, Flags::new(5)), b"\x1b[57414u");
    e.state = ElementState::Released;
    assert_eq!(encode(&e, Flags::new(7)), b"\x1b[57414;1:3u");
}

#[test]
fn protocol_queries_nested_modes_and_legacy_cursor_restore() {
    let mut t = Terminal::new(80, 24);
    assert_eq!(t.feed(b"\x1b[?u\x1b[c"), b"\x1b[?0u\x1b[?1;0c");
    assert_eq!(t.feed(b"\x1b[>5u\x1b[?u"), b"\x1b[?5u");
    assert_eq!(t.feed(b"\x1b[>31u\x1b[?u"), b"\x1b[?31u");
    assert_eq!(t.feed(b"\x1b[<u\x1b[?u"), b"\x1b[?5u");
    assert_eq!(t.feed(b"\x1b[=2;2u\x1b[?u"), b"\x1b[?7u");
    assert_eq!(t.feed(b"\x1b[=2;3u\x1b[?u"), b"\x1b[?5u");
    assert_eq!(t.feed(b"\x1b[=31;0u\x1b[?u"), b"\x1b[?5u");
    t.feed(b"\x1b[3;4H\x1b[s\x1b[H\x1b[u");
    assert_eq!(t.cursor, (3, 2));
    assert_eq!(t.feed(b"\x1b[<99u\x1b[?u"), b"\x1b[?0u");
}

#[test]
fn screen_stacks_are_independent_and_reset_is_complete() {
    let mut t = Terminal::new(80, 24);
    t.feed(b"\x1b[>5u\x1b[?1049h");
    assert_eq!(t.feed(b"\x1b[?u"), b"\x1b[?0u");
    t.feed(b"\x1b[>31u\x1b[?1049l");
    assert_eq!(t.feed(b"\x1b[?u"), b"\x1b[?5u");
    t.feed(b"\x1b[?1049h");
    assert_eq!(t.feed(b"\x1b[?u"), b"\x1b[?31u");
    t.feed(b"\x1bc");
    assert_eq!(t.feed(b"\x1b[?u"), b"\x1b[?0u");
}

#[test]
fn split_sequences_and_bounded_stack_keep_the_latest_modes() {
    let mut t = Terminal::new(80, 24);
    for byte in b"\x1b[>5u" {
        t.feed(&[*byte]);
    }
    assert_eq!(t.feed(b"\x1b[?u"), b"\x1b[?5u");
    for _ in 0..100 {
        t.feed(b"\x1b[>7u");
    }
    assert_eq!(t.feed(b"\x1b[<31u\x1b[?u"), b"\x1b[?7u");
    assert_eq!(t.feed(b"\x1b[<99u\x1b[?u"), b"\x1b[?0u");
    assert_eq!(t.feed(b"\x1b[=65535u\x1b[?u"), b"\x1b[?31u");
}
