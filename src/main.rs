use std::ffi::OsString;
use std::num::NonZeroU32;
use std::rc::Rc;

use anyhow::{Context as _, Result};
use cryptty::{
    config::{Config, Padding},
    input, pty,
    render::Renderer,
    terminal::Terminal,
};
use portable_pty::PtySize;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, WindowEvent};
use winit::event_loop::{
    ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy,
};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

const FONT_SIZE_PT: f32 = 17.0;

fn font_size_pixels(scale_factor: f64) -> f32 {
    FONT_SIZE_PT * 96.0 / 72.0 * scale_factor as f32
}

struct View {
    surface: Surface<Rc<Window>, Rc<Window>>,
    window: Rc<Window>,
    renderer: Renderer,
    terminal: Terminal,
    pty: pty::Pty,
    modifiers: ModifiersState,
    preedit: String,
    padding: Padding,
    font_hinting: bool,
}

struct App {
    view: Option<View>,
    proxy: EventLoopProxy<()>,
    command: Vec<OsString>,
    config: Config,
    error: Option<anyhow::Error>,
}

impl App {
    fn create(&mut self, events: &ActiveEventLoop) -> Result<()> {
        let window = Rc::new(
            events.create_window(
                Window::default_attributes()
                    .with_title("Cryptty")
                    .with_visible(false)
                    .with_inner_size(LogicalSize::new(900.0, 560.0))
                    .with_min_inner_size(LogicalSize::new(160.0, 100.0)),
            )?,
        );
        window.set_ime_allowed(true);
        let context = Context::new(window.clone())
            .map_err(|e| anyhow::anyhow!("software display: {e}"))?;
        let surface = Surface::new(&context, window.clone())
            .map_err(|e| anyhow::anyhow!("software surface: {e}"))?;
        let renderer = Renderer::with_options(
            font_size_pixels(window.scale_factor()),
            self.config.window.padding.physical(window.scale_factor()),
            self.config.font.hinting,
        )?;
        let size = window.inner_size();
        let (cols, rows) = renderer.grid_size(size.width, size.height);
        let terminal =
            Terminal::with_theme(cols, rows, self.config.colors.theme());
        let proxy = self.proxy.clone();
        let pty = pty::Pty::spawn(
            &self.command,
            pty_size(cols, rows, &renderer),
            move || {
                let _ = proxy.send_event(());
            },
        )?;
        self.view = Some(View {
            surface,
            window,
            renderer,
            terminal,
            pty,
            modifiers: ModifiersState::empty(),
            preedit: String::new(),
            padding: self.config.window.padding,
            font_hinting: self.config.font.hinting,
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
            self.window.set_title("Cryptty — input backlog full");
            eprintln!("Cryptty: input backlog full; input rejected");
        }
        Ok(())
    }

    fn resize(&mut self) -> Result<()> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let (cols, rows) = self.renderer.grid_size(size.width, size.height);
        self.terminal.resize(cols, rows);
        self.pty.resize(pty_size(cols, rows, &self.renderer))?;
        self.renderer.invalidate();
        self.window.request_redraw();
        Ok(())
    }

    fn draw(&mut self) -> Result<()> {
        let size = self.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        self.surface
            .resize(width, height)
            .map_err(|e| anyhow::anyhow!("resize software surface: {e}"))?;
        self.renderer.draw(
            &self.terminal,
            size.width,
            size.height,
            &self.preedit,
        );
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|e| anyhow::anyhow!("acquire software buffer: {e}"))?;
        buffer.copy_from_slice(&self.renderer.pixels);
        self.window.pre_present_notify();
        buffer
            .present()
            .map_err(|e| anyhow::anyhow!("present frame: {e}"))?;
        let (x, y) = self.terminal.cursor;
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
                    font_size_pixels(scale_factor),
                    self.padding.physical(scale_factor),
                    self.font_hinting,
                )?;
                self.resize()?;
            }
            WindowEvent::RedrawRequested => self.draw()?,
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::Focused(false) => {
                self.modifiers = ModifiersState::empty();
                self.preedit.clear();
                self.renderer.invalidate();
                self.window.request_redraw();
            }
            WindowEvent::Ime(Ime::Preedit(text, _)) => {
                self.preedit = text;
                self.renderer.invalidate();
                self.window.request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
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
            } if event.state == ElementState::Pressed => {
                if event.logical_key == Key::Named(NamedKey::Enter)
                    && self.modifiers.alt_key()
                    && !self.modifiers.control_key()
                    && !self.modifiers.super_key()
                {
                    let fullscreen = if self.window.fullscreen().is_some() {
                        None
                    } else {
                        Some(Fullscreen::Borderless(None))
                    };
                    self.window.set_fullscreen(fullscreen);
                } else if self.preedit.is_empty() {
                    self.send_input(input::encode(
                        &event.logical_key,
                        event.text.as_deref(),
                        self.modifiers,
                        self.terminal.application_cursor,
                    ))?;
                }
            }
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
        let Some(view) = self.view.as_mut() else {
            return;
        };
        match view.pty.flush_input() {
            Ok(false) => (),
            Ok(true) => view.window.set_title("Cryptty"),
            Err(error) => {
                self.fail(events, error);
                return;
            }
        }
        // Bound each batch so keyboard and window events can run under load.
        for _ in 0..32 {
            match view.pty.output.try_recv() {
                Ok(pty::Event::Output(bytes)) => {
                    let replies = view.terminal.feed(&bytes);
                    if let Err(error) = view.send_input(replies) {
                        self.fail(events, error);
                        return;
                    }
                    view.window.request_redraw();
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
    let mut args = std::env::args_os().skip(1);
    let command = match args.next() {
        Some(arg) if arg == "--" => args.collect(),
        Some(arg) if arg == "--help" || arg == "-h" => {
            println!(
                "Cryptty — software-rendered terminal\n\n\
                Usage: cryptty [-- PROGRAM [ARG...]]\n\n\
                Runs $SHELL by default. Alt+Enter toggles fullscreen."
            );
            return Ok(());
        }
        Some(arg) => anyhow::bail!("Unknown argument {arg:?}; use --help"),
        None => Vec::new(),
    };
    let config = Config::load()?;
    let events = EventLoop::with_user_event()
        .build()
        .context("Cannot open a desktop display")?;
    events.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        view: None,
        proxy: events.create_proxy(),
        command,
        config,
        error: None,
    };
    events.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error);
    }
    Ok(())
}
