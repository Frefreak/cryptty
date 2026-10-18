use std::collections::{HashMap, HashSet};
use std::time::Instant;

use anyhow::Result;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::Fullscreen;

use cryptty::input::{self, kitty::KeyInput};

use crate::View;

enum Consumed {
    Paste,
    OneShot,
}

#[derive(Default)]
pub(super) struct State {
    consumed: HashMap<PhysicalKey, Consumed>,
    bound_keys: HashMap<PhysicalKey, Vec<u8>>,
    held_modifiers: HashMap<PhysicalKey, ModifiersState>,
    locks: u8,
    forwarded: HashSet<PhysicalKey>,
    modal_keys: HashSet<PhysicalKey>,
}

impl State {
    pub(super) fn reset(&mut self) {
        self.consumed.clear();
        self.bound_keys.clear();
        self.held_modifiers.clear();
        self.forwarded.clear();
        self.modal_keys.clear();
    }

    fn consume_shortcut_event(
        &mut self,
        physical: PhysicalKey,
        state: ElementState,
        repeat: bool,
    ) -> bool {
        match state {
            ElementState::Released => self.consumed.remove(&physical).is_some(),
            ElementState::Pressed => {
                repeat
                    && matches!(
                        self.consumed.get(&physical),
                        Some(Consumed::OneShot)
                    )
            }
        }
    }

    fn claim_modal(&mut self, key: PhysicalKey) {
        if !self.forwarded.contains(&key)
            && !self.consumed.contains_key(&key)
            && !self.bound_keys.contains_key(&key)
        {
            self.modal_keys.insert(key);
        }
    }

    fn consume_modal_event(
        &mut self,
        key: PhysicalKey,
        state: ElementState,
        active: bool,
    ) -> bool {
        if state == ElementState::Released {
            return self.modal_keys.remove(&key);
        }
        if active {
            self.claim_modal(key);
            false
        } else {
            self.modal_keys.contains(&key)
        }
    }

    fn binding_input(
        &mut self,
        keyboard: &cryptty::config::Keyboard,
        event: &KeyInput<'_>,
        composing: bool,
    ) -> Option<Vec<u8>> {
        if event.state == ElementState::Released {
            return self.bound_keys.remove(&event.physical).map(|_| Vec::new());
        }
        if let Some(bytes) = self.bound_keys.get(&event.physical) {
            return Some(if composing { Vec::new() } else { bytes.clone() });
        }
        if event.repeat || composing {
            return None;
        }
        let text = keyboard.text_for(&event.unmodified, event.modifiers)?;
        let bytes = text.as_bytes().to_vec();
        self.bound_keys.insert(event.physical, bytes.clone());
        Some(bytes)
    }

    fn modifiers(
        &mut self,
        event: &KeyEvent,
        mut modifiers: ModifiersState,
    ) -> ModifiersState {
        let bit = match event.logical_key {
            Key::Named(NamedKey::Shift) => ModifiersState::SHIFT,
            Key::Named(NamedKey::Control) => ModifiersState::CONTROL,
            Key::Named(NamedKey::Alt) => ModifiersState::ALT,
            Key::Named(NamedKey::Super) => ModifiersState::SUPER,
            _ => return modifiers,
        };
        if event.state == ElementState::Pressed {
            self.held_modifiers.insert(event.physical_key, bit);
        } else {
            self.held_modifiers.remove(&event.physical_key);
        }
        modifiers
            .set(bit, self.held_modifiers.values().any(|&held| held == bit));
        modifiers
    }

    fn locks(&mut self, event: &KeyEvent) -> u8 {
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::{NSEvent, NSEventModifierFlags};
            // AppKit exposes the current lock state on the window thread.
            let flags = unsafe { NSEvent::modifierFlags_class() };
            self.locks = if flags
                .contains(NSEventModifierFlags::NSEventModifierFlagCapsLock)
            {
                64
            } else {
                0
            };
            let _ = event;
        }
        #[cfg(not(target_os = "macos"))]
        if event.state == ElementState::Pressed && !event.repeat {
            match event.logical_key {
                Key::Named(NamedKey::CapsLock) => self.locks ^= 64,
                Key::Named(NamedKey::NumLock) => self.locks ^= 128,
                _ => (),
            }
        }
        self.locks
    }
}

impl View {
    pub(super) fn key_input(&mut self, event: KeyEvent) -> Result<()> {
        let pressed = event.state == ElementState::Pressed;
        let locks = self.keyboard_input.locks(&event);
        let modifiers = self.keyboard_input.modifiers(&event, self.modifiers);
        let input_event = KeyInput {
            key: &event.logical_key,
            unmodified: event.key_without_modifiers(),
            physical: event.physical_key,
            location: event.location,
            text: event.text.as_deref(),
            modifiers,
            locks,
            state: event.state,
            repeat: event.repeat,
        };
        if self.keyboard_input.consume_modal_event(
            event.physical_key,
            event.state,
            self.vim.is_some(),
        ) {
            return Ok(());
        }
        if self.keyboard_input.consume_shortcut_event(
            event.physical_key,
            event.state,
            event.repeat,
        ) {
            return Ok(());
        }
        let forwarded_release = !pressed
            && (self.keyboard_input.forwarded.remove(&event.physical_key)
                || self
                    .keyboard_input
                    .bound_keys
                    .contains_key(&event.physical_key));
        if !forwarded_release && self.vim_input(&input_event) {
            if pressed {
                self.keyboard_input.claim_modal(event.physical_key);
            }
            return Ok(());
        }
        let cursor_changed = pressed && self.reset_blink();
        if let Some(bytes) = self.keyboard_input.binding_input(
            &self.keyboard,
            &input_event,
            !self.preedit.is_empty(),
        ) {
            if pressed && !bytes.is_empty() {
                self.redraw.input(Instant::now());
            }
            if pressed && !bytes.is_empty() {
                self.keyboard_input.forwarded.insert(event.physical_key);
            }
            self.send_input(bytes)?;
            if cursor_changed {
                self.window.request_redraw();
            }
            return Ok(());
        }
        if pressed {
            if let Some(shortcut) =
                input::shortcut(&event.logical_key, self.modifiers)
            {
                let consumed = match shortcut {
                    input::Shortcut::Paste => Consumed::Paste,
                    _ => Consumed::OneShot,
                };
                self.keyboard_input
                    .consumed
                    .insert(event.physical_key, consumed);
                match shortcut {
                    input::Shortcut::Paste => self.paste(),
                    input::Shortcut::NewWindow if !event.repeat => {
                        self.new_window()
                    }
                    input::Shortcut::NewWindow => (),
                    input::Shortcut::ReloadConfig if !event.repeat => {
                        self.reload_config();
                    }
                    input::Shortcut::ReloadConfig => (),
                }
                self.window.request_redraw();
                return Ok(());
            }
            if event.logical_key == Key::Named(NamedKey::Enter)
                && self.modifiers.alt_key()
                && !self.modifiers.control_key()
                && !self.modifiers.super_key()
            {
                self.keyboard_input
                    .consumed
                    .insert(event.physical_key, Consumed::OneShot);
                if !event.repeat {
                    let fullscreen = if self.window.fullscreen().is_some() {
                        None
                    } else {
                        Some(Fullscreen::Borderless(None))
                    };
                    self.window.set_fullscreen(fullscreen);
                }
                return Ok(());
            }
        }
        if !self.preedit.is_empty()
            && !input::kitty::is_modifier(&event.logical_key)
        {
            return Ok(());
        }
        let flags = self.terminal.keyboard.flags();
        let bytes = input::encode_key(
            &input_event,
            flags,
            self.terminal.modify_other_keys_2,
            self.terminal.application_cursor,
            self.terminal.application_keypad,
        );
        if pressed && !bytes.is_empty() {
            self.redraw.input(Instant::now());
        }
        if pressed && !bytes.is_empty() {
            self.keyboard_input.forwarded.insert(event.physical_key);
        }
        self.send_input(bytes)?;
        if cursor_changed {
            self.window.request_redraw();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cryptty::config::Config;
    use winit::keyboard::{KeyCode, KeyLocation};

    #[test]
    fn modal_repeats_stop_at_exit_but_preexisting_keys_keep_their_release() {
        let mut state = State::default();
        let key = PhysicalKey::Code(KeyCode::KeyY);
        assert!(!state.consume_modal_event(key, ElementState::Pressed, true));
        assert!(state.consume_modal_event(key, ElementState::Pressed, false));
        assert!(state.consume_modal_event(key, ElementState::Released, false));
        assert!(!state.consume_modal_event(key, ElementState::Pressed, false));
        state.forwarded.insert(key);
        assert!(!state.consume_modal_event(key, ElementState::Pressed, true));
        assert!(!state.consume_modal_event(key, ElementState::Released, true));
        assert!(state.forwarded.remove(&key));
    }

    #[test]
    fn shortcuts_and_empty_bindings_keep_release_ownership_in_vim_mode() {
        let mut state = State::default();
        let key = PhysicalKey::Code(KeyCode::KeyR);
        state.consumed.insert(key, Consumed::OneShot);
        assert!(!state.consume_modal_event(key, ElementState::Pressed, true));
        assert!(!state.consume_modal_event(key, ElementState::Released, true));
        assert!(state.consume_shortcut_event(
            key,
            ElementState::Released,
            false
        ));
        assert!(!state.consumed.contains_key(&key));
        state.bound_keys.insert(key, Vec::new());
        state.claim_modal(key);
        assert!(!state.consume_modal_event(key, ElementState::Released, true));
        assert!(state.bound_keys.contains_key(&key));
    }

    #[test]
    fn shortcut_repeats_stay_consumed_until_release() {
        let key = PhysicalKey::Code(KeyCode::KeyR);
        let mut state = State::default();
        assert!(!state.consume_shortcut_event(
            key,
            ElementState::Pressed,
            false
        ));
        state.consumed.insert(key, Consumed::OneShot);
        assert!(state.consume_shortcut_event(key, ElementState::Pressed, true));
        assert!(state.consume_shortcut_event(
            key,
            ElementState::Released,
            false
        ));
        assert!(!state.consume_shortcut_event(
            key,
            ElementState::Pressed,
            false
        ));
        assert!(!state.consume_shortcut_event(
            key,
            ElementState::Released,
            false
        ));
    }

    #[test]
    fn paste_shortcuts_keep_their_repeat_behavior() {
        let key = PhysicalKey::Code(KeyCode::KeyV);
        let mut state = State::default();
        state.consumed.insert(key, Consumed::Paste);
        assert!(!state.consume_shortcut_event(
            key,
            ElementState::Pressed,
            true
        ));
        assert!(state.consume_shortcut_event(
            key,
            ElementState::Released,
            false
        ));
    }

    #[test]
    fn binding_choice_survives_modifier_changes_until_release() {
        let keyboard = Config::parse(
            r#"
keyboard:
  bindings:
    - {key: Enter, modifiers: [Shift], text: "\u001b\r"}
"#,
        )
        .unwrap()
        .keyboard;
        let key = Key::Named(NamedKey::Enter);
        let mut event = KeyInput {
            key: &key,
            unmodified: key.clone(),
            physical: PhysicalKey::Code(KeyCode::Enter),
            location: KeyLocation::Standard,
            text: Some("\r"),
            modifiers: ModifiersState::SHIFT,
            locks: 0,
            state: ElementState::Pressed,
            repeat: false,
        };
        let mut state = State::default();
        assert_eq!(
            state.binding_input(&keyboard, &event, false),
            Some(b"\x1b\r".to_vec())
        );
        event.modifiers = ModifiersState::empty();
        event.repeat = true;
        assert_eq!(
            state.binding_input(&keyboard, &event, false),
            Some(b"\x1b\r".to_vec())
        );
        assert_eq!(
            state.binding_input(&keyboard, &event, true),
            Some(Vec::new())
        );
        event.state = ElementState::Released;
        assert_eq!(
            state.binding_input(&keyboard, &event, false),
            Some(Vec::new())
        );

        event.state = ElementState::Pressed;
        event.repeat = false;
        assert_eq!(state.binding_input(&keyboard, &event, false), None);
        event.modifiers = ModifiersState::SHIFT;
        event.repeat = true;
        assert_eq!(state.binding_input(&keyboard, &event, false), None);
        event.state = ElementState::Released;
        assert_eq!(state.binding_input(&keyboard, &event, false), None);

        event.state = ElementState::Pressed;
        event.repeat = false;
        assert_eq!(state.binding_input(&keyboard, &event, true), None);
        assert!(state.bound_keys.is_empty());
    }
}
