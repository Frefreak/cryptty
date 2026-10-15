use std::process::Command;

use anyhow::Result;
use cryptty::{
    input,
    terminal::{Effect, MouseMode},
};
use winit::event::{ElementState, MouseButton, MouseScrollDelta};

use crate::View;

#[derive(Default)]
pub(super) struct Interaction {
    clipboard: Option<arboard::Clipboard>,
    pointer: (f64, f64),
    buttons: [bool; 3],
    last_motion: Option<(usize, usize)>,
    scroll: (f64, f64),
}

impl Interaction {
    pub(super) fn reset_buttons(&mut self) {
        self.buttons = [false; 3];
        self.last_motion = None;
    }

    fn clipboard(&mut self) -> Result<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new()?);
        }
        Ok(self.clipboard.as_mut().unwrap())
    }
}

impl View {
    pub(super) fn paste(&mut self) {
        let result = (|| -> Result<()> {
            let text = self.interaction.clipboard()?.get_text()?;
            self.send_input(input::paste(&text, self.terminal.bracketed_paste))
        })();
        if let Err(error) = result {
            eprintln!("Cryptty: paste failed: {error}");
        }
    }

    pub(super) fn apply_effects(&mut self) {
        for effect in std::mem::take(&mut self.terminal.effects) {
            let result = match effect {
                Effect::Clipboard(text) => self
                    .interaction
                    .clipboard()
                    .and_then(|clipboard| Ok(clipboard.set_text(text)?)),
            };
            if let Err(error) = result {
                eprintln!("Cryptty: clipboard write failed: {error}");
            }
        }
    }

    fn pointer_cell(&self) -> (usize, usize) {
        let (left, top) = self.renderer.cell_origin(0, 0);
        let (x, y) = self.interaction.pointer;
        let x = (x - left as f64).max(0.0) as usize / self.renderer.cell_width;
        let y = (y - top as f64).max(0.0) as usize / self.renderer.cell_height;
        (
            x.min(self.terminal.cols() - 1),
            y.min(self.terminal.rows.len() - 1),
        )
    }

    pub(super) fn pointer_moved(&mut self, x: f64, y: f64) -> Result<()> {
        self.interaction.pointer = (x, y);
        let position = self.pointer_cell();
        let button = self.interaction.buttons.iter().position(|b| *b);
        let report = match self.terminal.mouse_mode {
            MouseMode::Motion => true,
            MouseMode::Drag => button.is_some(),
            _ => false,
        };
        if report && self.interaction.last_motion != Some(position) {
            self.send_mouse(button.unwrap_or(3) as u8 + 32, false)?;
        }
        self.interaction.last_motion = Some(position);
        Ok(())
    }

    pub(super) fn mouse_button(
        &mut self,
        button: MouseButton,
        state: ElementState,
    ) -> Result<()> {
        let index = match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            _ => return Ok(()),
        };
        let released = state == ElementState::Released;
        let link_modifier = if cfg!(target_os = "macos") {
            self.modifiers.super_key()
        } else {
            self.modifiers.control_key()
        };
        if !released && index == 0 && link_modifier {
            let (x, y) = self.pointer_cell();
            if let Some(uri) = &self.terminal.rows[y][x].hyperlink {
                open_link(uri);
                return Ok(());
            }
        }
        if released && !self.interaction.buttons[index] {
            return Ok(());
        }
        self.interaction.buttons[index] = !released;
        if self.terminal.mouse_mode != MouseMode::Off {
            self.send_mouse(index as u8, released)?;
        }
        Ok(())
    }

    pub(super) fn mouse_wheel(
        &mut self,
        delta: MouseScrollDelta,
    ) -> Result<()> {
        if self.terminal.mouse_mode == MouseMode::Off {
            return Ok(());
        }
        let (x, y) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
            MouseScrollDelta::PixelDelta(p) => (
                p.x / self.renderer.cell_width as f64,
                p.y / self.renderer.cell_height as f64,
            ),
        };
        self.interaction.scroll.0 += x;
        self.interaction.scroll.1 += y;
        let dx = self.interaction.scroll.0.trunc().clamp(-32.0, 32.0) as i32;
        let dy = self.interaction.scroll.1.trunc().clamp(-32.0, 32.0) as i32;
        self.interaction.scroll.0 -= f64::from(dx);
        self.interaction.scroll.1 -= f64::from(dy);
        for _ in 0..dx.unsigned_abs() {
            self.send_mouse(if dx > 0 { 66 } else { 67 }, false)?;
        }
        for _ in 0..dy.unsigned_abs() {
            self.send_mouse(if dy > 0 { 64 } else { 65 }, false)?;
        }
        Ok(())
    }

    fn send_mouse(&mut self, button: u8, released: bool) -> Result<()> {
        self.send_input(input::mouse(
            button,
            released,
            self.pointer_cell(),
            self.modifiers,
            self.terminal.mouse_sgr,
        ))
    }
}

fn open_link(uri: &str) {
    if !["https://", "http://", "mailto:"]
        .iter()
        .any(|s| uri.starts_with(s))
    {
        return;
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match Command::new(program).arg(uri).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => eprintln!("Cryptty: cannot open hyperlink: {error}"),
    }
}
