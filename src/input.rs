use winit::keyboard::{Key, ModifiersState, NamedKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Paste,
    NewWindow,
}

pub fn shortcut(key: &Key, modifiers: ModifiersState) -> Option<Shortcut> {
    if modifiers.alt_key() || modifiers.super_key() {
        return None;
    }
    if key == &Key::Named(NamedKey::Insert) && modifiers.shift_key() {
        return Some(Shortcut::Paste);
    }
    if modifiers.control_key()
        && modifiers.shift_key()
        && let Key::Character(text) = key
    {
        if text.eq_ignore_ascii_case("v") {
            return Some(Shortcut::Paste);
        }
        if text.eq_ignore_ascii_case("n") {
            return Some(Shortcut::NewWindow);
        }
    }
    None
}

pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\n");
    if bracketed {
        let text: String = text
            .chars()
            .filter(|c| !matches!(*c, '\x1b' | '\u{009b}'))
            .collect();
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace('\n', "\r").into_bytes()
    }
}

pub fn focus(focused: bool, reporting: bool) -> Vec<u8> {
    if !reporting {
        return Vec::new();
    }
    if focused {
        b"\x1b[I".to_vec()
    } else {
        b"\x1b[O".to_vec()
    }
}

pub fn keypad(
    key: &Key,
    modifiers: ModifiersState,
    application: bool,
) -> Option<Vec<u8>> {
    if !application {
        return None;
    }
    if modifiers.super_key() {
        return Some(Vec::new());
    }
    let code = match key {
        Key::Named(NamedKey::Enter) => 'M',
        Key::Character(text) => match text.as_str() {
            "0" => 'p',
            "1" => 'q',
            "2" => 'r',
            "3" => 's',
            "4" => 't',
            "5" => 'u',
            "6" => 'v',
            "7" => 'w',
            "8" => 'x',
            "9" => 'y',
            "." => 'n',
            "," => 'l',
            "+" => 'k',
            "-" => 'm',
            "*" => 'j',
            "/" => 'o',
            "=" => 'X',
            _ => return None,
        },
        _ => return None,
    };
    Some(format!("\x1bO{code}").into_bytes())
}

pub fn mouse(
    button: u8,
    released: bool,
    position: (usize, usize),
    modifiers: ModifiersState,
    sgr: bool,
) -> Vec<u8> {
    let flags = 4 * u8::from(modifiers.shift_key())
        + 8 * u8::from(modifiers.alt_key())
        + 16 * u8::from(modifiers.control_key());
    let (x, y) = (position.0 + 1, position.1 + 1);
    if sgr {
        let end = if released { 'm' } else { 'M' };
        format!("\x1b[<{};{x};{y}{end}", button + flags).into_bytes()
    } else if x < 224 && y < 224 {
        let button = if released { 3 } else { button };
        vec![
            0x1b,
            b'[',
            b'M',
            32 + button + flags,
            32 + x as u8,
            32 + y as u8,
        ]
    } else {
        Vec::new()
    }
}

pub fn encode(
    key: &Key,
    text: Option<&str>,
    modifiers: ModifiersState,
    application_cursor: bool,
) -> Vec<u8> {
    if modifiers.super_key() {
        return Vec::new();
    }
    let modifier = 1
        + u8::from(modifiers.shift_key())
        + 2 * u8::from(modifiers.alt_key())
        + 4 * u8::from(modifiers.control_key());
    let arrow = match key {
        Key::Named(NamedKey::ArrowUp) => Some('A'),
        Key::Named(NamedKey::ArrowDown) => Some('B'),
        Key::Named(NamedKey::ArrowRight) => Some('C'),
        Key::Named(NamedKey::ArrowLeft) => Some('D'),
        Key::Named(NamedKey::Home) => Some('H'),
        Key::Named(NamedKey::End) => Some('F'),
        _ => None,
    };
    if let Some(suffix) = arrow {
        return if modifier != 1 {
            format!("\x1b[1;{modifier}{suffix}").into_bytes()
        } else if application_cursor {
            format!("\x1bO{suffix}").into_bytes()
        } else {
            format!("\x1b[{suffix}").into_bytes()
        };
    }
    let numbered = match key {
        Key::Named(NamedKey::Insert) => Some(2),
        Key::Named(NamedKey::Delete) => Some(3),
        Key::Named(NamedKey::PageUp) => Some(5),
        Key::Named(NamedKey::PageDown) => Some(6),
        Key::Named(NamedKey::F5) => Some(15),
        Key::Named(NamedKey::F6) => Some(17),
        Key::Named(NamedKey::F7) => Some(18),
        Key::Named(NamedKey::F8) => Some(19),
        Key::Named(NamedKey::F9) => Some(20),
        Key::Named(NamedKey::F10) => Some(21),
        Key::Named(NamedKey::F11) => Some(23),
        Key::Named(NamedKey::F12) => Some(24),
        _ => None,
    };
    if let Some(code) = numbered {
        return if modifier == 1 {
            format!("\x1b[{code}~").into_bytes()
        } else {
            format!("\x1b[{code};{modifier}~").into_bytes()
        };
    }
    let mut bytes = match key {
        Key::Named(NamedKey::Enter) => vec![b'\r'],
        Key::Named(NamedKey::Backspace) => vec![0x7f],
        Key::Named(NamedKey::Escape) => vec![0x1b],
        Key::Named(NamedKey::Tab) if modifiers.shift_key() => {
            return b"\x1b[Z".to_vec();
        }
        Key::Named(NamedKey::Tab) => vec![b'\t'],
        Key::Named(NamedKey::F1) => return function_key('P', modifier),
        Key::Named(NamedKey::F2) => return function_key('Q', modifier),
        Key::Named(NamedKey::F3) => return function_key('R', modifier),
        Key::Named(NamedKey::F4) => return function_key('S', modifier),
        Key::Character(value) if modifiers.control_key() => {
            let upper = value.to_ascii_uppercase();
            match upper.as_bytes() {
                [b'@'..=b'_'] => vec![upper.as_bytes()[0] & 0x1f],
                [b' '] | [b'2'] => vec![0],
                [b'?'] | [b'8'] => vec![0x7f],
                _ => Vec::new(),
            }
        }
        _ => text.unwrap_or("").as_bytes().to_vec(),
    };
    if modifiers.alt_key() && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn function_key(suffix: char, modifier: u8) -> Vec<u8> {
    if modifier == 1 {
        format!("\x1bO{suffix}").into_bytes()
    } else {
        format!("\x1b[1;{modifier}{suffix}").into_bytes()
    }
}
