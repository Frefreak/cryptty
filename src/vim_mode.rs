use std::time::{Duration, Instant};

use cryptty::input::kitty::KeyInput;
use cryptty::terminal::Terminal;
use cryptty::tmux::TextSnapshot;
use cryptty::vim::{Action, Key, Session};
use winit::event::ElementState;
use winit::keyboard::{Key as WindowKey, ModifiersState, NamedKey};

use crate::View;

pub(super) enum Mode {
    Loading {
        screen: Box<Terminal>,
        keys: Vec<Key>,
        deadline: Instant,
        kind: Kind,
    },
    Active(Box<Session>),
    Marker(Box<cryptty::marker::Session>),
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Vim,
    Marker,
}

impl Mode {
    pub fn screen(&self) -> &Terminal {
        match self {
            Self::Loading { screen, .. } => screen,
            Self::Active(session) => &session.screen,
            Self::Marker(session) => &session.screen,
        }
    }

    pub fn status(&self) -> String {
        match self {
            Self::Loading { kind, .. } => match kind {
                Kind::Vim => "VIM — preparing".into(),
                Kind::Marker => "MARK — preparing".into(),
            },
            Self::Active(session) => session.status(),
            Self::Marker(session) => session.status(),
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        match self {
            Self::Loading { deadline, .. } => Some(*deadline),
            Self::Active(_) | Self::Marker(_) => None,
        }
    }
}

fn mode_key(input: &KeyInput<'_>) -> Option<Key> {
    if input
        .modifiers
        .intersects(ModifiersState::ALT | ModifiersState::SUPER)
    {
        return None;
    }
    if input.modifiers.control_key() {
        if input.modifiers != ModifiersState::CONTROL {
            return None;
        }
        return match &input.unmodified {
            WindowKey::Character(text) if text.eq_ignore_ascii_case("v") => {
                Some(Key::Control('v'))
            }
            _ => None,
        };
    }
    match input.key {
        WindowKey::Character(text) => {
            let mut chars = text.chars();
            let ch = chars.next()?;
            chars.next().is_none().then_some(Key::Char(ch))
        }
        WindowKey::Named(key) => match key {
            NamedKey::Escape => Some(Key::Escape),
            NamedKey::Enter => Some(Key::Enter),
            NamedKey::Backspace => Some(Key::Backspace),
            NamedKey::Space => Some(Key::Char(' ')),
            NamedKey::ArrowLeft => Some(Key::Left),
            NamedKey::ArrowRight => Some(Key::Right),
            NamedKey::ArrowUp => Some(Key::Up),
            NamedKey::ArrowDown => Some(Key::Down),
            _ => None,
        },
        _ => None,
    }
}

impl View {
    pub(super) fn vim_input(&mut self, input: &KeyInput<'_>) -> bool {
        let marker = self
            .keyboard
            .enters_marker(&input.unmodified, input.modifiers);
        if marker && self.preedit.is_empty() {
            if input.state == ElementState::Pressed
                && !input.repeat
                && self.vim.is_none()
            {
                self.enter_mode(Kind::Marker);
            }
            return true;
        }
        let trigger = self.preedit.is_empty()
            && (!input.repeat || self.vim.is_some())
            && self.keyboard.enters_vim(&input.unmodified, input.modifiers);
        if trigger {
            if input.state == ElementState::Pressed && !input.repeat {
                if matches!(
                    self.vim,
                    Some(Mode::Marker(_))
                        | Some(Mode::Loading {
                            kind: Kind::Marker,
                            ..
                        })
                ) {
                    return true;
                }
                if self.vim.is_some() {
                    self.vim_key(Key::Char('s'));
                } else {
                    self.enter_mode(Kind::Vim);
                }
            }
            return true;
        }
        if self.vim.is_none() {
            return false;
        }
        if input.state == ElementState::Pressed {
            if !self.preedit.is_empty() {
                if input.key == &WindowKey::Named(NamedKey::Escape) {
                    self.preedit.clear();
                    self.renderer.invalidate();
                    self.window.request_redraw();
                }
                return true;
            }
            if let Some(key) = mode_key(input) {
                self.vim_key(key);
            }
        }
        true
    }

    fn enter_mode(&mut self, kind: Kind) {
        self.renderer.set_link_tooltip(None, (0, 0));
        self.interaction.reset_buttons();
        self.clear_selection();
        self.preedit.clear();
        self.redraw.cancel_input();
        let mut screen = self.terminal.visible_snapshot();
        screen.cursor_visible = false;
        screen.vim_mode = true;
        self.vim = Some(Mode::Loading {
            screen: Box::new(screen),
            keys: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(3),
            kind,
        });
        self.refresh_vim();
        self.update_title();
        self.renderer.invalidate();
        self.window.request_redraw();
    }

    pub(super) fn leave_vim(&mut self) {
        if self.vim.take().is_some() {
            self.preedit.clear();
            self.window.set_ime_allowed(true);
            self.interaction.reset_buttons();
            self.tmux.update(
                false,
                (self.terminal.cols(), self.terminal.rows.len()),
                Instant::now(),
            );
            self.renderer.invalidate();
            self.update_title();
            self.window.request_redraw();
        }
    }

    pub(super) fn refresh_vim(&mut self) {
        let Some(mode) = &self.vim else {
            return;
        };
        let now = Instant::now();
        let loading = matches!(mode, Mode::Loading { .. });
        self.tmux.update(
            loading,
            (self.terminal.cols(), self.terminal.rows.len()),
            now,
        );
        if !loading {
            return;
        }
        if mode.deadline().is_some_and(|t| t <= now) {
            eprintln!("Cryptty: Vim mode capture timed out; try again");
            self.leave_vim();
            return;
        }
        let panes = match self.tmux.text(mode.screen()) {
            TextSnapshot::Pending => return,
            TextSnapshot::Unavailable => Vec::new(),
            TextSnapshot::Failed => {
                eprintln!("Cryptty: tmux viewport unavailable for Vim mode");
                self.leave_vim();
                return;
            }
            TextSnapshot::Changed => {
                eprintln!(
                    "Cryptty: pane text changed during Vim capture; try again"
                );
                self.leave_vim();
                return;
            }
            TextSnapshot::Ready(panes) => panes,
        };
        let Some(Mode::Loading {
            screen, keys, kind, ..
        }) = self.vim.take()
        else {
            unreachable!();
        };
        self.vim = Some(match kind {
            Kind::Vim => Mode::Active(Box::new(Session::new(*screen, panes))),
            Kind::Marker => Mode::Marker(Box::new(
                cryptty::marker::Session::new(*screen, panes, &self.markers),
            )),
        });
        self.tmux.update(
            false,
            (self.terminal.cols(), self.terminal.rows.len()),
            now,
        );
        for key in keys {
            if self.vim.is_none() {
                break;
            }
            self.vim_key(key);
        }
        self.update_title();
        self.renderer.invalidate();
        self.window.request_redraw();
    }

    pub(super) fn vim_key(&mut self, key: Key) {
        let action = match &mut self.vim {
            Some(Mode::Loading { keys, .. }) => {
                if key == Key::Escape {
                    Action::Exit
                } else {
                    if keys.len() < 64 {
                        keys.push(key);
                    }
                    Action::Stay
                }
            }
            Some(Mode::Active(session)) => session.key(key, Instant::now()),
            Some(Mode::Marker(session)) => session.key(key),
            None => return,
        };
        match action {
            Action::Exit => self.leave_vim(),
            Action::Copy(text) => {
                match self
                    .interaction
                    .clipboard()
                    .and_then(|clipboard| Ok(clipboard.set_text(text)?))
                {
                    Ok(()) => self.leave_vim(),
                    Err(error) => {
                        eprintln!("Cryptty: Vim copy failed: {error}")
                    }
                }
            }
            Action::Stay => (),
        }
        self.update_title();
        self.renderer.invalidate();
        self.window.request_redraw();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
