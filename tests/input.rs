use cryptty::input::{encode, focus, keypad, mouse, paste};
use winit::keyboard::{Key, ModifiersState as M, NamedKey};

#[test]
fn control_alt_and_unicode_input() {
    assert_eq!(
        encode(&Key::Character("c".into()), Some("c"), M::CONTROL, false),
        vec![3]
    );
    assert_eq!(
        encode(&Key::Character("x".into()), Some("x"), M::ALT, false),
        b"\x1bx"
    );
    assert_eq!(
        encode(&Key::Character("中".into()), Some("中"), M::empty(), false),
        "中".as_bytes()
    );
    assert!(
        encode(&Key::Character("q".into()), Some("q"), M::SUPER, false)
            .is_empty()
    );
}

#[test]
fn cursor_modes_and_modifiers() {
    let key = Key::Named(NamedKey::ArrowUp);
    assert_eq!(encode(&key, None, M::empty(), false), b"\x1b[A");
    assert_eq!(encode(&key, None, M::empty(), true), b"\x1bOA");
    assert_eq!(
        encode(&key, None, M::CONTROL | M::SHIFT, false),
        b"\x1b[1;6A"
    );
}

#[test]
fn paste_frames_multiline_text_and_removes_frame_escape_characters() {
    assert_eq!(paste("one\r\ntwo\n", false), b"one\rtwo\r");
    assert_eq!(
        paste("one\r\ntwo\x1b[201~\u{009b}x", true),
        "\x1b[200~one\ntwo[201~x\x1b[201~".as_bytes()
    );
}

#[test]
fn focus_and_application_keypad_follow_negotiated_modes() {
    assert!(focus(true, false).is_empty());
    assert_eq!(focus(true, true), b"\x1b[I");
    assert_eq!(focus(false, true), b"\x1b[O");
    assert_eq!(
        keypad(&Key::Character("1".into()), M::empty(), true).unwrap(),
        b"\x1bOq"
    );
    assert_eq!(
        keypad(&Key::Named(NamedKey::Enter), M::empty(), true).unwrap(),
        b"\x1bOM"
    );
    assert!(keypad(&Key::Character("1".into()), M::empty(), false).is_none());
    assert!(
        keypad(&Key::Character("1".into()), M::SUPER, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mouse_encodes_press_release_drag_wheel_and_large_coordinates() {
    assert_eq!(mouse(0, false, (2, 3), M::empty(), true), b"\x1b[<0;3;4M");
    assert_eq!(mouse(0, true, (2, 3), M::SHIFT, true), b"\x1b[<4;3;4m");
    assert_eq!(
        mouse(32, false, (299, 249), M::CONTROL, true),
        b"\x1b[<48;300;250M"
    );
    assert_eq!(mouse(64, false, (0, 0), M::ALT, true), b"\x1b[<72;1;1M");
    assert_eq!(mouse(0, true, (2, 3), M::empty(), false), b"\x1b[M##$");
    assert!(mouse(0, false, (223, 0), M::empty(), false).is_empty());
}
