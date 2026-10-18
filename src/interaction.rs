use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;
use cryptty::{
    input,
    terminal::{Effect, MouseMode, Position, Row, TextRange, supported_uri},
    tmux::{Lookup, ScreenLink},
};
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::window::CursorIcon;

use crate::View;

#[derive(Default)]
pub(super) struct Interaction {
    clipboard: Option<arboard::Clipboard>,
    pointer: (f64, f64),
    buttons: [bool; 3],
    last_motion: Option<(usize, usize)>,
    scroll: (f64, f64),
    selection_anchor: Option<SelectionAnchor>,
    clicks: Clicks,
    pointer_inside: bool,
    pending_link: Option<PendingLink>,
}

struct PendingLink {
    position: Position,
    rows: std::collections::VecDeque<Row>,
}

impl PendingLink {
    fn matches(&self, terminal: &cryptty::terminal::Terminal) -> bool {
        self.rows.len() == terminal.rows.len()
            && self.rows.iter().zip(&terminal.rows).all(|(before, after)| {
                before.len() == after.len()
                    && before.wrapped == after.wrapped
                    && before.iter().zip(after).all(|(a, b)| {
                        a.content == b.content && a.hyperlink == b.hyperlink
                    })
            })
    }
}

#[derive(Clone, Copy)]
enum SelectionAnchor {
    Character(Position),
    Word(TextRange),
    Line(TextRange),
}

#[derive(Default)]
struct Clicks {
    last: Option<(Instant, Position)>,
    count: u8,
}

impl Clicks {
    fn press(&mut self, now: Instant, position: Position) -> u8 {
        self.count = if self.last.is_some_and(|(time, cell)| {
            cell == position
                && now.duration_since(time) < Duration::from_millis(400)
        }) {
            self.count % 3 + 1
        } else {
            1
        };
        self.last = Some((now, position));
        self.count
    }
}

impl Interaction {
    pub(super) fn reset_buttons(&mut self) {
        self.buttons = [false; 3];
        self.last_motion = None;
        self.selection_anchor = None;
        self.clicks = Clicks::default();
        self.pending_link = None;
    }

    fn selection_after_output(
        &mut self,
        previous: Option<TextRange>,
        current: Option<TextRange>,
    ) -> bool {
        if previous == current {
            return false;
        }
        self.clicks = Clicks::default();
        if let (Some(before), Some(after)) = (previous, current) {
            let delta = after.start.row as isize - before.start.row as isize;
            let shift = |position: &mut Position| {
                position.row = position.row.saturating_add_signed(delta);
            };
            match &mut self.selection_anchor {
                Some(SelectionAnchor::Character(position)) => shift(position),
                Some(
                    SelectionAnchor::Word(range) | SelectionAnchor::Line(range),
                ) => {
                    shift(&mut range.start);
                    shift(&mut range.end);
                }
                None => (),
            }
        } else {
            self.selection_anchor = None;
        }
        true
    }

    pub(super) fn clipboard(&mut self) -> Result<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new()?);
        }
        Ok(self.clipboard.as_mut().unwrap())
    }
}

impl View {
    pub(super) fn new_window(&self) {
        let result = (|| -> Result<()> {
            let mut child = Command::new(std::env::current_exe()?)
                .args(std::env::args_os().skip(1).take_while(|arg| arg != "--"))
                .stdin(Stdio::null())
                .spawn()?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Cryptty: cannot open new window: {error}");
        }
    }

    pub(super) fn clear_selection(&mut self) {
        if self.terminal.selection.take().is_some() {
            self.renderer.invalidate();
        }
        self.interaction.selection_anchor = None;
        self.interaction.clicks = Clicks::default();
    }

    pub(super) fn selection_after_output(
        &mut self,
        previous: Option<TextRange>,
    ) {
        if self
            .interaction
            .selection_after_output(previous, self.terminal.selection)
        {
            self.renderer.invalidate();
        }
    }

    pub(super) fn pointer_left(&mut self) {
        self.interaction.pointer_inside = false;
        self.refresh_pointer();
    }

    pub(super) fn refresh_pointer(&mut self) {
        if self.vim.is_some() {
            if self.renderer.set_link_tooltip(None, (0, 0)) {
                self.window.request_redraw();
            }
            self.refresh_vim();
            self.window.set_cursor(CursorIcon::Default);
            return;
        }
        let available = self.terminal.mouse_mode == MouseMode::Off
            || self.modifiers.shift_key()
            || self.interaction.selection_anchor.is_some();
        self.tmux.update(
            self.interaction.pending_link.is_some()
                || self.interaction.pointer_inside,
            (self.terminal.cols(), self.terminal.rows.len()),
            Instant::now(),
        );
        let (x, y) = self.pointer_cell();
        let position = Position { row: y, col: x };
        if let Some(pending) = &self.interaction.pending_link {
            let clicked = pending.position;
            if !pending.matches(&self.terminal) {
                self.interaction.pending_link = None;
            } else {
                match self.lookup_link(clicked) {
                    Lookup::Pending => (),
                    result => {
                        self.interaction.pending_link = None;
                        if let Lookup::Ready(Some(link)) = result {
                            open_link(&link.uri);
                        }
                    }
                }
            }
            if self.interaction.pending_link.is_none()
                && !self.interaction.pointer_inside
            {
                self.tmux.update(
                    false,
                    (self.terminal.cols(), self.terminal.rows.len()),
                    Instant::now(),
                );
            }
        }
        let link = if self.interaction.pointer_inside {
            self.lookup_link(position)
        } else {
            Lookup::Ready(None)
        };
        let link = match link {
            Lookup::Ready(link) => link,
            _ => None,
        };
        let hovered = link
            .as_ref()
            .map(|link| link.ranges.clone())
            .unwrap_or_default();
        let tooltip_changed = self.renderer.set_link_tooltip(
            link.as_ref()
                .filter(|_| self.modifiers.control_key())
                .map(|link| link.uri.as_str()),
            (x, y),
        );
        if self.terminal.hovered_link != hovered || tooltip_changed {
            self.terminal.hovered_link = hovered;
            self.renderer.invalidate();
            self.window.request_redraw();
        }
        self.window.set_cursor(
            if available && link.is_some() && self.modifiers.control_key() {
                CursorIcon::Pointer
            } else if available {
                CursorIcon::Text
            } else {
                CursorIcon::Default
            },
        );
    }

    fn lookup_link(&self, position: Position) -> Lookup {
        let explicit = self.terminal.rows[position.row][position.col]
            .hyperlink
            .is_some();
        if !explicit {
            match self.tmux.lookup(&self.terminal, position) {
                Lookup::Unavailable => (),
                result => return result,
            }
        }
        Lookup::Ready(self.terminal.link_at(position).map(ScreenLink::from))
    }

    fn selection_position(&self) -> Position {
        let (left, _) = self.renderer.cell_origin(0, 0);
        let col = ((self.interaction.pointer.0 - left as f64)
            / self.renderer.cell_width as f64)
            .round()
            .max(0.0) as usize;
        Position {
            row: self.pointer_cell().1,
            col: col.min(self.terminal.cols()),
        }
    }

    fn update_selection(&mut self, anchor: SelectionAnchor) {
        let (col, row) = self.pointer_cell();
        let position = Position { row, col };
        let range = match anchor {
            SelectionAnchor::Character(start) => self
                .terminal
                .selection_between(start, self.selection_position()),
            SelectionAnchor::Word(start) => {
                let end = self.terminal.word_selection(position);
                TextRange::between(
                    start.start.min(end.start),
                    start.end.max(end.end),
                )
            }
            SelectionAnchor::Line(start) => {
                let end = self.terminal.line_selection(position);
                TextRange::between(
                    start.start.min(end.start),
                    start.end.max(end.end),
                )
            }
        };
        self.terminal.selection = (range.start != range.end).then_some(range);
        self.renderer.invalidate();
        self.window.request_redraw();
    }

    fn copy_selection(&mut self) {
        let Some(range) = self.terminal.selection else {
            return;
        };
        let text = self.terminal.selected_text(range);
        if text.is_empty() {
            return;
        }
        if let Err(error) = self
            .interaction
            .clipboard()
            .and_then(|clipboard| Ok(clipboard.set_text(text)?))
        {
            eprintln!("Cryptty: selection copy failed: {error}");
        }
    }

    pub(super) fn paste(&mut self) {
        let result = (|| -> Result<()> {
            let text = self.interaction.clipboard()?.get_text()?;
            self.clear_selection();
            self.window.request_redraw();
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
                Effect::Bell => {
                    self.notifications.notify(&self.window, None);
                    Ok(())
                }
                Effect::Notification(message) => {
                    self.notifications.notify(&self.window, Some(message));
                    Ok(())
                }
                Effect::UnknownOsc {
                    command,
                    bytes,
                    count,
                } => {
                    crate::diagnostics::unknown_osc(command, bytes, count);
                    Ok(())
                }
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
        self.interaction.pointer_inside = true;
        self.refresh_pointer();
        if self.vim.is_some() {
            return Ok(());
        }
        if let Some(anchor) = self.interaction.selection_anchor {
            let (col, row) = self.pointer_cell();
            if self
                .interaction
                .clicks
                .last
                .is_some_and(|(_, cell)| cell != Position { row, col })
            {
                self.interaction.clicks = Clicks::default();
            }
            self.update_selection(anchor);
            return Ok(());
        }
        if self.modifiers.shift_key() {
            return Ok(());
        }
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
        if self.vim.is_some() {
            return Ok(());
        }
        let index = match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            _ => return Ok(()),
        };
        let released = state == ElementState::Released;
        let available = self.terminal.mouse_mode == MouseMode::Off
            || self.modifiers.shift_key();
        if released
            && index == 0
            && let Some(anchor) = self.interaction.selection_anchor.take()
        {
            self.update_selection(anchor);
            self.copy_selection();
            return Ok(());
        }
        if !released && index == 0 && available && self.modifiers.control_key()
        {
            let (col, row) = self.pointer_cell();
            let position = Position { row, col };
            self.interaction.clicks = Clicks::default();
            match self.lookup_link(position) {
                Lookup::Ready(Some(link)) => {
                    open_link(&link.uri);
                    return Ok(());
                }
                Lookup::Pending => {
                    self.interaction.pending_link = Some(PendingLink {
                        position,
                        rows: self.terminal.rows.clone(),
                    });
                    return Ok(());
                }
                _ => (),
            }
        }

        if !released && index == 0 && available {
            let (col, row) = self.pointer_cell();
            let position = Position { row, col };
            let count = self.interaction.clicks.press(Instant::now(), position);
            let anchor = match count {
                2 => SelectionAnchor::Word(
                    self.terminal.word_selection(position),
                ),
                3 => SelectionAnchor::Line(
                    self.terminal.line_selection(position),
                ),
                _ => SelectionAnchor::Character(self.selection_position()),
            };
            self.interaction.selection_anchor = Some(anchor);
            self.update_selection(anchor);
            return Ok(());
        }
        if !released && index == 1 && available {
            self.paste();
            return Ok(());
        }
        if !released && available {
            return Ok(());
        }
        if released && !self.interaction.buttons[index] {
            return Ok(());
        }
        self.interaction.buttons[index] = !released;
        if self.terminal.mouse_mode != MouseMode::Off
            && (released || !self.modifiers.shift_key())
        {
            self.send_mouse(index as u8, released)?;
        }
        Ok(())
    }

    pub(super) fn mouse_wheel(
        &mut self,
        delta: MouseScrollDelta,
    ) -> Result<()> {
        if self.vim.is_some() {
            return Ok(());
        }
        if self.terminal.mouse_mode == MouseMode::Off
            || self.modifiers.shift_key()
        {
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
    if !supported_uri(uri) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_preserves_pending_drag_and_double_clicks() {
        let mut interaction = Interaction::default();
        let cell = Position { row: 1, col: 2 };
        let now = Instant::now();
        assert_eq!(interaction.clicks.press(now, cell), 1);
        interaction.selection_anchor = Some(SelectionAnchor::Character(cell));
        assert!(!interaction.selection_after_output(None, None));
        assert!(matches!(
            interaction.selection_anchor,
            Some(SelectionAnchor::Character(position)) if position == cell
        ));
        assert_eq!(
            interaction
                .clicks
                .press(now + Duration::from_millis(100), cell),
            2
        );
    }

    #[test]
    fn output_moves_an_active_word_drag_with_the_selection() {
        let before = TextRange::between(
            Position { row: 2, col: 1 },
            Position { row: 2, col: 5 },
        );
        let after = TextRange::between(
            Position { row: 1, col: 1 },
            Position { row: 1, col: 5 },
        );
        let mut interaction = Interaction {
            selection_anchor: Some(SelectionAnchor::Word(before)),
            ..Interaction::default()
        };
        assert!(interaction.selection_after_output(Some(before), Some(after)));
        let Some(SelectionAnchor::Word(anchor)) = interaction.selection_anchor
        else {
            panic!("Word drag must remain active");
        };
        assert_eq!(anchor, after);
        assert!(interaction.selection_after_output(Some(after), None));
        assert!(interaction.selection_anchor.is_none());
    }

    #[test]
    fn multiple_clicks_require_the_same_cell_and_a_short_interval() {
        let mut clicks = Clicks::default();
        let now = Instant::now();
        let cell = Position { row: 1, col: 2 };
        assert_eq!(clicks.press(now, cell), 1);
        assert_eq!(clicks.press(now + Duration::from_millis(100), cell), 2);
        assert_eq!(clicks.press(now + Duration::from_millis(200), cell), 3);
        assert_eq!(clicks.press(now + Duration::from_millis(300), cell), 1);
        assert_eq!(clicks.press(now + Duration::from_secs(1), cell), 1);
        assert_eq!(
            clicks.press(
                now + Duration::from_millis(1100),
                Position { row: 1, col: 3 }
            ),
            1
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "interaction/tmux_tests.rs"]
mod tmux_tests;
