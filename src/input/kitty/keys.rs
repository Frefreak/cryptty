use winit::keyboard::{Key, KeyCode, KeyLocation, NamedKey, PhysicalKey};

use super::{KeyInput, single_character};

pub(super) fn code(event: &KeyInput<'_>) -> Option<(u32, char)> {
    if event.location == KeyLocation::Numpad
        && let Some(code) = keypad(event)
    {
        return Some((code, 'u'));
    }
    let Key::Named(key) = event.key else {
        return single_character(&event.unmodified).map(|ch| (ch as u32, 'u'));
    };
    use NamedKey::*;
    let value = match key {
        Escape => (27, 'u'),
        Enter => (13, 'u'),
        Tab => (9, 'u'),
        Backspace => (127, 'u'),
        Insert => (2, '~'),
        Delete => (3, '~'),
        ArrowLeft => (1, 'D'),
        ArrowRight => (1, 'C'),
        ArrowUp => (1, 'A'),
        ArrowDown => (1, 'B'),
        PageUp => (5, '~'),
        PageDown => (6, '~'),
        Home => (1, 'H'),
        End => (1, 'F'),
        CapsLock => (57358, 'u'),
        ScrollLock => (57359, 'u'),
        NumLock => (57360, 'u'),
        PrintScreen => (57361, 'u'),
        Pause => (57362, 'u'),
        ContextMenu => (57363, 'u'),
        F1 => (1, 'P'),
        F2 => (1, 'Q'),
        F3 => (13, '~'),
        F4 => (1, 'S'),
        F5 => (15, '~'),
        F6 => (17, '~'),
        F7 => (18, '~'),
        F8 => (19, '~'),
        F9 => (20, '~'),
        F10 => (21, '~'),
        F11 => (23, '~'),
        F12 => (24, '~'),
        Shift | Control | Alt | Super | Hyper | Meta => {
            let offset = if event.location == KeyLocation::Right {
                6
            } else {
                0
            };
            let base = match key {
                Shift => 57441,
                Control => 57442,
                Alt => 57443,
                Super => 57444,
                Hyper => 57445,
                _ => 57446,
            };
            (base + offset, 'u')
        }
        _ => {
            let keys = [
                F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24,
                F25, F26, F27, F28, F29, F30, F31, F32, F33, F34, F35,
            ];
            (
                57376 + keys.iter().position(|entry| entry == key)? as u32,
                'u',
            )
        }
    };
    Some(value)
}

pub(super) fn is_modifier(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::Super
                | NamedKey::Hyper
                | NamedKey::Meta
                | NamedKey::CapsLock
                | NamedKey::NumLock
        )
    )
}

fn keypad(event: &KeyInput<'_>) -> Option<u32> {
    if let Key::Named(key) = event.key {
        use NamedKey::*;
        let code = match key {
            Enter => 57414,
            ArrowLeft => 57417,
            ArrowRight => 57418,
            ArrowUp => 57419,
            ArrowDown => 57420,
            PageUp => 57421,
            PageDown => 57422,
            Home => 57423,
            End => 57424,
            Insert => 57425,
            Delete => 57426,
            Clear => 57427,
            _ => return None,
        };
        return Some(code);
    }
    use KeyCode::*;
    let PhysicalKey::Code(key) = event.physical else {
        return None;
    };
    Some(match key {
        Numpad0 => 57399,
        Numpad1 => 57400,
        Numpad2 => 57401,
        Numpad3 => 57402,
        Numpad4 => 57403,
        Numpad5 => 57404,
        Numpad6 => 57405,
        Numpad7 => 57406,
        Numpad8 => 57407,
        Numpad9 => 57408,
        NumpadDecimal => 57409,
        NumpadDivide => 57410,
        NumpadMultiply => 57411,
        NumpadSubtract => 57412,
        NumpadAdd => 57413,
        NumpadEnter => 57414,
        NumpadEqual => 57415,
        NumpadComma => 57416,
        _ => return None,
    })
}

pub(super) fn base_layout(physical: PhysicalKey) -> Option<char> {
    use KeyCode::*;
    let PhysicalKey::Code(key) = physical else {
        return None;
    };
    Some(match key {
        KeyA => 'a',
        KeyB => 'b',
        KeyC => 'c',
        KeyD => 'd',
        KeyE => 'e',
        KeyF => 'f',
        KeyG => 'g',
        KeyH => 'h',
        KeyI => 'i',
        KeyJ => 'j',
        KeyK => 'k',
        KeyL => 'l',
        KeyM => 'm',
        KeyN => 'n',
        KeyO => 'o',
        KeyP => 'p',
        KeyQ => 'q',
        KeyR => 'r',
        KeyS => 's',
        KeyT => 't',
        KeyU => 'u',
        KeyV => 'v',
        KeyW => 'w',
        KeyX => 'x',
        KeyY => 'y',
        KeyZ => 'z',
        Digit0 => '0',
        Digit1 => '1',
        Digit2 => '2',
        Digit3 => '3',
        Digit4 => '4',
        Digit5 => '5',
        Digit6 => '6',
        Digit7 => '7',
        Digit8 => '8',
        Digit9 => '9',
        Space => ' ',
        Backquote => '`',
        Minus => '-',
        Equal => '=',
        BracketLeft => '[',
        BracketRight => ']',
        Backslash => '\\',
        Semicolon => ';',
        Quote => '\'',
        Comma => ',',
        Period => '.',
        Slash => '/',
        _ => return None,
    })
}
