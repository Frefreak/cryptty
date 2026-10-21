use std::ffi::OsString;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use cryptty::{
    config::{Config, Font, Keyboard, Padding},
    options::{self, Action, RendererChoice},
    pty,
    render::Renderer,
    terminal::Terminal,
    tmux,
};
use portable_pty::PtySize;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{Ime, WindowEvent};
use winit::event_loop::{
    ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy,
};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowId};

mod diagnostics;
mod display;
mod interaction;
mod keyboard_input;
#[cfg(target_os = "macos")]
mod macos_input;
#[cfg(target_os = "macos")]
mod macos_reopen;
mod notifications;
mod redraw;
mod reload;
mod vim_mode;
#[cfg(target_os = "linux")]
mod wayland_blur;

struct View {
    display: display::Display,
    window: Arc<Window>,
    renderer: Renderer,
    terminal: Terminal,
    pty: pty::Pty,
    tmux: tmux::Integration,
    vim: Option<vim_mode::Mode>,
    modifiers: ModifiersState,
    keyboard_input: keyboard_input::State,
    keyboard: Keyboard,
    markers: cryptty::marker::Options,
    preedit: String,
    padding: Padding,
    font: Font,
    interaction: interaction::Interaction,
    redraw: redraw::Redraw,
    next_blink: Instant,
    next_text_blink: Instant,
    notifications: notifications::Notifications,
    displayed_title: String,
    backlog_full: bool,
    config_reload_failed: bool,
    notice: Option<(String, Instant)>,
}

struct App {
    view: Option<View>,
    proxy: EventLoopProxy<()>,
    command: Vec<OsString>,
    config: Config,
    renderer_choice: RendererChoice,
    error: Option<anyhow::Error>,
}

impl App {
    fn create(&mut self, events: &ActiveEventLoop) -> Result<()> {
        let attributes = Window::default_attributes()
            .with_title("Cryptty")
            .with_visible(false)
            .with_transparent(self.config.window.opacity.transparent())
            .with_inner_size(LogicalSize::new(900.0, 560.0))
            .with_min_inner_size(LogicalSize::new(160.0, 100.0));
        #[cfg(target_os = "linux")]
        let attributes = {
            use winit::platform::wayland::WindowAttributesExtWayland;
            attributes.with_name("org.cryptty", "cryptty")
        };
        let window = Arc::new(events.create_window(attributes)?);
        window.set_ime_allowed(true);
        window.set_cursor(CursorIcon::Text);
        #[cfg(target_os = "macos")]
        macos_input::install(&window)?;
        #[cfg(target_os = "macos")]
        macos_input::configure(&window, self.config.window.option_as_alt);
        let display = display::Display::new(
            window.clone(),
            self.renderer_choice,
            self.config.window,
        )?;
        let renderer = Renderer::with_options(
            self.config.font.size.pixels(window.scale_factor()),
            self.config.window.padding.physical(window.scale_factor()),
            self.config.font.hinting,
            &self.config.font.family,
            &self.config.font.codepoint_map,
            &self.config.font.fallback,
        )?;
        let size = window.inner_size();
        let (cols, rows) = renderer.grid_size(size.width, size.height);
        let mut terminal =
            Terminal::with_theme(cols, rows, self.config.colors.theme());
        terminal.word_characters =
            self.config.selection.word_characters.clone();
        terminal.cell_pixels =
            (renderer.cell_width as u32, renderer.cell_height as u32);
        let proxy = self.proxy.clone();
        let pty = pty::Pty::spawn(
            &self.command,
            pty_size(cols, rows, &renderer),
            move || {
                let _ = proxy.send_event(());
            },
        )?;
        let proxy = self.proxy.clone();
        let tmux = tmux::Integration::new(pty.tty_name(), move || {
            let _ = proxy.send_event(());
        });
        self.view = Some(View {
            display,
            window,
            renderer,
            terminal,
            pty,
            tmux,
            vim: None,
            modifiers: ModifiersState::empty(),
            keyboard_input: keyboard_input::State::default(),
            keyboard: self.config.keyboard.clone(),
            markers: self.config.markers.clone(),
            preedit: String::new(),
            padding: self.config.window.padding,
            font: self.config.font.clone(),
            interaction: interaction::Interaction::default(),
            redraw: redraw::Redraw::default(),
            next_blink: Instant::now() + Duration::from_millis(500),
            next_text_blink: Instant::now() + Duration::from_millis(250),
            notifications: notifications::Notifications::default(),
            displayed_title: "Cryptty".into(),
            backlog_full: false,
            config_reload_failed: false,
            notice: None,
        });
        self.view.as_mut().unwrap().draw()?;
        self.view.as_ref().unwrap().window.set_visible(true);
        Ok(())
    }

    fn fail(&mut self, events: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        events.exit();
    }
}

impl View {
    fn send_input(&mut self, bytes: Vec<u8>) -> Result<()> {
        if !self.pty.write(bytes)? {
            self.backlog_full = true;
            self.update_title();
            eprintln!("Cryptty: input backlog full; input rejected");
        }
        Ok(())
    }

    fn update_title(&mut self) {
        let mut title = self.terminal.title.clone();
        if let Some(mode) = &self.vim {
            title.push_str(" — ");
            title.push_str(&mode.status());
        }
        if self.backlog_full {
            title.push_str(" — input backlog full");
        }
        if self.config_reload_failed {
            title.push_str(" — config reload failed");
        }
        if title != self.displayed_title {
            self.window.set_title(&title);
            self.displayed_title = title;
        }
    }

    fn reset_blink(&mut self) -> bool {
        let changed = self.terminal.reset_cursor_blink();
        self.next_blink = Instant::now() + Duration::from_millis(500);
        changed
    }

    fn resize(&mut self) -> Result<()> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let (cols, rows) = self.renderer.grid_size(size.width, size.height);
        self.terminal.cell_pixels = (
            self.renderer.cell_width as u32,
            self.renderer.cell_height as u32,
        );
        self.leave_vim();
        self.terminal.resize(cols, rows);
        self.tmux.invalidate();
        self.pty.resize(pty_size(cols, rows, &self.renderer))?;
        self.renderer.invalidate();
        self.clear_selection();
        self.refresh_pointer();
        self.defer_redraw();
        Ok(())
    }

    fn input_priority(&self, now: Instant) -> bool {
        cfg!(target_os = "linux")
            && self.display.is_gpu()
            && self.redraw.input_active(now)
    }

    fn defer_redraw(&mut self) {
        let now = Instant::now();
        self.redraw.defer(now);
        if self.redraw.deadline().is_some_and(|t| t <= now) {
            self.window.request_redraw();
        }
    }

    fn draw(&mut self) -> Result<()> {
        let now = Instant::now();
        if (self.vim.is_none()
            && self.terminal.synchronized_until().is_some_and(|t| t > now))
            || self.redraw.deadline().is_some_and(|t| t > now)
        {
            return Ok(());
        }
        let request_frame_callback = !self.input_priority(now);
        let targets = match &self.vim {
            Some(vim_mode::Mode::Marker(session)) => session.visible_targets(),
            _ => Vec::new(),
        };
        self.renderer.set_marker_opacity(&self.markers);
        self.renderer.set_markers(targets);
        self.display.draw(
            &self.window,
            &mut self.renderer,
            self.vim
                .as_ref()
                .map_or(&self.terminal, |mode| mode.screen()),
            &self.preedit,
            request_frame_callback,
        )?;
        self.redraw.presented(
            Instant::now(),
            self.display.frame_interval(&self.window),
        );
        let (x, y) = self
            .vim
            .as_ref()
            .map_or(self.terminal.cursor, |mode| mode.screen().cursor);
        let (left, top) = self.renderer.cell_origin(x, y);
        self.window.set_ime_cursor_area(
            PhysicalPosition::new(left as i32, top as i32),
            PhysicalSize::new(
                self.renderer.cell_width as u32,
                self.renderer.cell_height as u32,
            ),
        );
        Ok(())
    }

    fn event(
        &mut self,
        events: &ActiveEventLoop,
        event: WindowEvent,
    ) -> Result<()> {
        match event {
            WindowEvent::CloseRequested => events.exit(),
            WindowEvent::Resized(_) => self.resize()?,
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.renderer = Renderer::with_options(
                    self.font.size.pixels(scale_factor),
                    self.padding.physical(scale_factor),
                    self.font.hinting,
                    &self.font.family,
                    &self.font.codepoint_map,
                    &self.font.fallback,
                )?;
                if let Some((message, _)) = &self.notice {
                    self.renderer.set_notice(Some(message));
                }
                self.display.clear_glyphs();
                self.resize()?;
            }
            WindowEvent::RedrawRequested => self.draw()?,
            WindowEvent::Occluded(occluded) => {
                if occluded {
                    self.redraw.cancel_input();
                } else {
                    self.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                self.refresh_pointer();
            }
            WindowEvent::Focused(focused) => {
                self.send_input(cryptty::input::focus(
                    focused,
                    self.terminal.focus_reporting,
                ))?;
                if !focused {
                    self.redraw.cancel_input();
                    self.modifiers = ModifiersState::empty();
                    self.preedit.clear();
                    self.interaction.reset_buttons();
                    self.keyboard_input.reset();
                } else {
                    self.window.request_user_attention(None);
                }
                self.refresh_pointer();
                self.reset_blink();
                self.renderer.invalidate();
                self.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer_moved(position.x, position.y)?;
            }
            WindowEvent::CursorLeft { .. } => self.pointer_left(),
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_button(button, state)?;
                self.refresh_pointer();
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                self.mouse_wheel(delta, phase)?;
            }
            WindowEvent::Ime(Ime::Preedit(text, _)) => {
                self.preedit = text;
                self.renderer.invalidate();
                self.window.request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                if self.vim.is_some() {
                    self.preedit.clear();
                    for ch in text.chars() {
                        if self.vim.is_none() {
                            break;
                        }
                        self.vim_key(cryptty::vim::Key::Char(ch));
                    }
                    return Ok(());
                }
                if !text.is_empty() {
                    self.redraw.input(Instant::now());
                }
                self.reset_blink();
                self.preedit.clear();
                self.renderer.invalidate();
                self.send_input(text.into_bytes())?;
                self.window.request_redraw();
            }
            WindowEvent::Ime(Ime::Disabled) => {
                self.preedit.clear();
                self.renderer.invalidate();
                self.window.request_redraw();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => self.key_input(event)?,
            _ => (),
        }
        Ok(())
    }
}

impl ApplicationHandler<()> for App {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        if self.view.is_none()
            && let Err(error) = self.create(events)
        {
            self.fail(events, error);
        }
    }

    fn window_event(
        &mut self,
        events: &ActiveEventLoop,
        _: WindowId,
        event: WindowEvent,
    ) {
        if let Some(view) = self.view.as_mut()
            && let Err(error) = view.event(events, event)
        {
            self.fail(events, error);
        }
    }

    fn user_event(&mut self, events: &ActiveEventLoop, _: ()) {
        #[cfg(target_os = "macos")]
        for _ in 0..macos_reopen::take_requests() {
            if let Some(view) = &self.view {
                view.new_window();
            }
        }
        let Some(view) = self.view.as_mut() else {
            return;
        };
        match view.pty.flush_input() {
            Ok(false) => (),
            Ok(true) => view.backlog_full = false,
            Err(error) => {
                self.fail(events, error);
                return;
            }
        }
        view.pty.begin_output_batch();
        // Bound each batch so keyboard and window events can run under load.
        let mut changed = false;
        let mut processed = 0;
        for _ in 0..32 {
            match view.pty.output.try_recv() {
                Ok(pty::Event::Output(bytes)) => {
                    processed += 1;
                    let selection = view.terminal.selection;
                    if view.vim.is_none()
                        && !view.terminal.hovered_link.is_empty()
                    {
                        view.renderer.invalidate();
                    }
                    if view.vim.is_none() {
                        view.tmux.invalidate();
                    }
                    let replies = view.terminal.feed(&bytes);
                    view.selection_after_output(selection);
                    view.apply_effects();
                    if let Err(error) = view.send_input(replies) {
                        self.fail(events, error);
                        return;
                    }
                    changed = true;
                    match view.pty.flush_input() {
                        Ok(_) => (),
                        Err(error) => {
                            self.fail(events, error);
                            return;
                        }
                    }
                }
                Ok(pty::Event::Closed) => {
                    events.exit();
                    return;
                }
                Ok(pty::Event::Error(error)) => {
                    self.fail(events, anyhow::anyhow!("PTY: {error}"));
                    return;
                }
                Err(_) => break,
            }
        }
        if processed == 32 {
            let _ = self.proxy.send_event(());
        }
        view.update_title();
        view.refresh_pointer();
        if changed && view.vim.is_none() {
            view.defer_redraw();
        }
    }

    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        let Some(view) = self.view.as_mut() else {
            return;
        };
        let now = Instant::now();
        if view.tmux.deadline().is_some_and(|deadline| deadline <= now) {
            view.refresh_pointer();
        }
        view.refresh_vim();
        if view.notice.is_none() && view.tmux.take_missing_binary_notice() {
            let message = "tmux detected, but its executable was not found.\n\
                Add tmux to Cryptty's PATH, then relaunch Cryptty.\n\
                Pane-aware features are unavailable; Vim selection spans \
                the full window.";
            eprintln!("Cryptty: {message}");
            view.renderer.set_notice(Some(message));
            view.notice = Some((message.into(), now + Duration::from_secs(10)));
            view.window.request_redraw();
        }
        let mut wake = if view.vim.is_some() {
            None
        } else {
            view.terminal.synchronized_until()
        };
        if let Some((_, deadline)) = view.notice.as_ref() {
            let deadline = *deadline;
            if deadline <= now {
                view.notice = None;
                view.renderer.set_notice(None);
                view.window.request_redraw();
            }
        }
        if wake.is_some_and(|t| t <= now) {
            view.terminal.finish_synchronized_update();
            view.window.request_redraw();
            wake = None;
        }
        if let Some(deadline) = view.redraw.deadline() {
            let deadline = wake.map_or(deadline, |sync| sync.max(deadline));
            if deadline <= now {
                if view.input_priority(now) {
                    if let Err(error) = view.draw() {
                        self.fail(events, error);
                        return;
                    }
                } else {
                    view.window.request_redraw();
                }
            } else {
                wake = Some(wake.map_or(deadline, |t| t.min(deadline)));
            }
        }
        if view.vim.is_none()
            && view.terminal.cursor_style.blinking
            && view.terminal.cursor_visible
        {
            if now >= view.next_blink {
                view.terminal.cursor_blink_on = !view.terminal.cursor_blink_on;
                view.next_blink = now + Duration::from_millis(500);
                view.window.request_redraw();
            }
            wake =
                Some(wake.map_or(view.next_blink, |t| t.min(view.next_blink)));
        }
        if view.vim.is_none() && view.terminal.has_blinking_text() {
            if now >= view.next_text_blink {
                view.terminal.text_blink_phase =
                    (view.terminal.text_blink_phase + 1) % 4;
                view.next_text_blink = now + Duration::from_millis(250);
                view.window.request_redraw();
            }
            wake =
                Some(wake.map_or(view.next_text_blink, |t| {
                    t.min(view.next_text_blink)
                }));
        } else {
            view.terminal.text_blink_phase = 0;
            view.next_text_blink = now + Duration::from_millis(250);
        }
        if let Some(deadline) =
            view.vim.as_ref().and_then(|mode| mode.deadline())
        {
            wake = Some(wake.map_or(deadline, |t| t.min(deadline)));
        }
        if let Some(deadline) = view.tmux.deadline() {
            wake = Some(wake.map_or(deadline, |t| t.min(deadline)));
        }
        if let Some((_, deadline)) = view.notice.as_ref() {
            let deadline = *deadline;
            wake = Some(wake.map_or(deadline, |t| t.min(deadline)));
        }
        events.set_control_flow(
            wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        );
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.view.take();
    }
}

fn pty_size(cols: usize, rows: usize, renderer: &Renderer) -> PtySize {
    PtySize {
        rows: rows.min(u16::MAX as usize) as u16,
        cols: cols.min(u16::MAX as usize) as u16,
        pixel_width: (cols * renderer.cell_width).min(u16::MAX as usize) as u16,
        pixel_height: (rows * renderer.cell_height).min(u16::MAX as usize)
            as u16,
    }
}

fn main() -> Result<()> {
    let (renderer_choice, command) =
        match options::parse(std::env::args_os().skip(1))? {
            Action::Image { paths, pane } => {
                if pane {
                    return cryptty::graphics::sender::open_pane(&paths);
                }
                return cryptty::graphics::sender::run(&paths);
            }
            Action::Help => {
                println!("{}", options::HELP);
                return Ok(());
            }
            Action::ListGpus => {
                cryptty::render::gpu::list_adapters();
                return Ok(());
            }
            Action::Run { renderer, command } => (renderer, command),
        };
    let config = Config::load()?;
    let events = EventLoop::with_user_event()
        .build()
        .context("Cannot open a desktop display")?;
    events.set_control_flow(ControlFlow::Wait);
    #[cfg(target_os = "macos")]
    macos_reopen::install(events.create_proxy())?;
    let mut app = App {
        view: None,
        proxy: events.create_proxy(),
        command,
        config,
        renderer_choice,
        error: None,
    };
    events.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error);
    }
    Ok(())
}
