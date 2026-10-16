use winit::keyboard::{Key, NamedKey};

use super::kitty::KeyInput;

pub(super) fn encode(event: &KeyInput<'_>) -> Option<Vec<u8>> {
    if event.modifiers.super_key() {
        return Some(Vec::new());
    }
    let text = event.text.unwrap_or("");
    let printable = !text.is_empty() && !text.chars().any(char::is_control);
    let modifiers = u8::from(event.modifiers.shift_key())
        | (u8::from(event.modifiers.alt_key()) << 1)
        | (u8::from(event.modifiers.control_key()) << 2);
    #[cfg(target_os = "macos")]
    let modifiers = {
        let mut modifiers = modifiers;
        if printable && character(&event.unmodified) != character_text(text) {
            modifiers &= !2;
            if modifiers & !1 == 0 && event.modifiers.alt_key() {
                return Some(text.as_bytes().to_vec());
            }
        }
        modifiers
    };
    if modifiers == 0 {
        return None;
    }
    let code = match event.key {
        Key::Named(NamedKey::Enter) if printable => {
            return Some(text.as_bytes().to_vec());
        }
        Key::Named(NamedKey::Backspace) if printable => {
            return Some(Vec::new());
        }
        Key::Named(NamedKey::Enter) => 13,
        Key::Named(NamedKey::Tab) => 9,
        Key::Named(NamedKey::Backspace) => 127,
        Key::Named(NamedKey::Escape) => 27,
        Key::Character(_) => {
            let ch = if printable {
                // Multi-character text belongs to input-method commits.
                let Some(ch) = character_text(text) else {
                    return Some(text.as_bytes().to_vec());
                };
                ch
            } else {
                character(event.key)?
            };
            if modifiers == 1 && ch != ' ' && !('@'..='\u{7f}').contains(&ch) {
                return None;
            }
            ch as u32
        }
        _ => return None,
    };
    Some(format!("\x1b[27;{};{code}~", modifiers + 1).into_bytes())
}

fn character(key: &Key) -> Option<char> {
    let Key::Character(text) = key else {
        return None;
    };
    character_text(text)
}

fn character_text(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}
