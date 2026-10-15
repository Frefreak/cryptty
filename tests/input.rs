use cryptty::input::encode;
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
