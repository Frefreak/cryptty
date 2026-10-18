use std::fmt::Write;

use winit::event::ElementState;
use winit::keyboard::{
    Key, KeyLocation, ModifiersState, NamedKey, PhysicalKey,
};

use crate::keyboard::{
    Flags, REPORT_ALL, REPORT_ALTERNATES, REPORT_EVENTS, REPORT_TEXT,
};

mod keys;

pub struct KeyInput<'a> {
    pub key: &'a Key,
    pub unmodified: Key,
    pub physical: PhysicalKey,
    pub location: KeyLocation,
    pub text: Option<&'a str>,
    pub modifiers: ModifiersState,
    pub locks: u8,
    pub state: ElementState,
    pub repeat: bool,
}

pub fn encode(event: &KeyInput<'_>, flags: Flags) -> Vec<u8> {
    let released = event.state == ElementState::Released;
    let all = flags.contains(REPORT_ALL);
    let special = matches!(
        event.key,
        Key::Named(NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace)
    ) && event.location != KeyLocation::Numpad;
    if released && (!flags.contains(REPORT_EVENTS) || (special && !all)) {
        return Vec::new();
    }
    let text = event.text.unwrap_or("");
    let printable = !text.is_empty() && !text.chars().any(char::is_control);
    let mut modifiers = u8::from(event.modifiers.shift_key())
        | (u8::from(event.modifiers.alt_key()) << 1)
        | (u8::from(event.modifiers.control_key()) << 2)
        | (u8::from(event.modifiers.super_key()) << 3);
    let mut binding_modifiers = modifiers;
    if printable && matches!(event.key, Key::Character(_)) {
        binding_modifiers &= !1;
    }
    #[cfg(target_os = "macos")]
    if printable && single_character(&event.unmodified) != text.chars().next() {
        binding_modifiers &= !2;
    }
    if printable && !released {
        match event.key {
            Key::Named(NamedKey::Enter) => return text.as_bytes().to_vec(),
            Key::Named(NamedKey::Backspace) => return Vec::new(),
            _ => (),
        }
    }
    if !all && binding_modifiers == 0 && !released {
        if special {
            return match event.key {
                Key::Named(NamedKey::Enter) => vec![b'\r'],
                Key::Named(NamedKey::Tab) => vec![b'\t'],
                _ => vec![0x7f],
            };
        }
        if printable {
            return text.as_bytes().to_vec();
        }
    }
    let Some((code, suffix)) = keys::code(event) else {
        return if released || !printable {
            Vec::new()
        } else {
            text.as_bytes().to_vec()
        };
    };
    if keys::is_modifier(event.key) && !all {
        return Vec::new();
    }
    modifiers |= event.locks;
    let modifier = u16::from(modifiers) + 1;
    let event_type = if released {
        3
    } else if event.repeat {
        2
    } else {
        1
    };
    let report_events = flags.contains(REPORT_EVENTS);
    if !matches!(suffix, 'u' | '~') {
        return if report_events {
            format!("\x1b[1;{modifier}:{event_type}{suffix}").into_bytes()
        } else if modifier != 1 {
            format!("\x1b[1;{modifier}{suffix}").into_bytes()
        } else {
            format!("\x1b[{suffix}").into_bytes()
        };
    }
    let mut output = format!("\x1b[{code}");
    if flags.contains(REPORT_ALTERNATES) && suffix == 'u' && code >= 32 {
        let shifted = event
            .modifiers
            .shift_key()
            .then(|| {
                single_character(event.key).or_else(|| text.chars().next())
            })
            .flatten()
            .filter(|&ch| ch as u32 != code && !ch.is_control());
        if let Some(ch) = shifted {
            write!(output, ":{}", ch as u32).unwrap();
        }
        if let Some(base) = keys::base_layout(event.physical)
            && base as u32 != code
            && !text.starts_with(base)
        {
            if shifted.is_none() {
                output.push(':');
            }
            write!(output, ":{}", base as u32).unwrap();
        }
    }
    let explicit_modifiers =
        modifier != 1 || (report_events && event_type != 1);
    if explicit_modifiers {
        write!(output, ";{modifier}").unwrap();
    }
    if report_events && event_type != 1 {
        write!(output, ":{event_type}").unwrap();
    }
    if flags.contains(REPORT_TEXT) && !released && binding_modifiers & 14 == 0 {
        let mut chars = text.chars().filter(|ch| !ch.is_control()).peekable();
        if chars.peek().is_some() {
            if !explicit_modifiers {
                output.push(';');
            }
            output.push(';');
            for (index, ch) in chars.enumerate() {
                if index > 0 {
                    output.push(':');
                }
                write!(output, "{}", ch as u32).unwrap();
            }
        }
    }
    output.push(suffix);
    output.into_bytes()
}

pub fn is_modifier(key: &Key) -> bool {
    keys::is_modifier(key)
}

fn single_character(key: &Key) -> Option<char> {
    let Key::Character(text) = key else {
        return None;
    };
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}
