use anyhow::Result;
use cryptty::{config::Config, render::Renderer};

use crate::{View, pty_size};

impl View {
    pub(super) fn reload_config(&mut self) {
        let result =
            Config::load().and_then(|config| self.apply_config(config));
        self.config_reload_failed = result.is_err();
        match result {
            Ok(()) => eprintln!("Cryptty: configuration reloaded"),
            Err(error) => eprintln!("Cryptty: config reload failed: {error:#}"),
        }
        self.update_title();
    }

    fn apply_config(&mut self, config: Config) -> Result<()> {
        let scale = self.window.scale_factor();
        let renderer = Renderer::with_options(
            config.font.size.pixels(scale),
            config.window.padding.physical(scale),
            config.font.hinting,
            &config.font.family,
            &config.font.codepoint_map,
        )?;
        let size = self.window.inner_size();
        let (cols, rows) = if size.width == 0 || size.height == 0 {
            (self.terminal.cols(), self.terminal.rows.len())
        } else {
            renderer.grid_size(size.width, size.height)
        };
        self.pty.resize(pty_size(cols, rows, &renderer))?;

        #[cfg(target_os = "macos")]
        crate::macos_input::configure(
            &self.window,
            config.window.option_as_alt,
        );
        self.renderer = renderer;
        self.display.clear_glyphs();
        self.terminal.resize(cols, rows);
        self.terminal.set_theme(config.colors.theme());
        self.font = config.font;
        self.padding = config.window.padding;
        self.keyboard = config.keyboard;
        self.clear_selection();
        self.refresh_pointer();
        self.window.request_redraw();
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::{App, RendererChoice};
    use std::process::Command;
    use winit::application::ApplicationHandler;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::keyboard::{Key, ModifiersState, NamedKey};
    use winit::platform::wayland::EventLoopBuilderExtWayland;

    #[test]
    #[ignore = "requires a native Wayland desktop"]
    fn running_window_reloads_config_and_recovers_from_invalid_file() {
        const CHILD: &str = "CRYPTTY_RELOAD_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let directory = tempfile::tempdir().unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "reload::tests::running_window_reloads_config_and_recovers_from_invalid_file",
                    "--ignored",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("XDG_CONFIG_HOME", directory.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        struct Check {
            proxy: winit::event_loop::EventLoopProxy<()>,
        }
        impl ApplicationHandler<()> for Check {
            fn resumed(&mut self, events: &ActiveEventLoop) {
                let root = std::env::var_os("XDG_CONFIG_HOME").unwrap();
                let path =
                    std::path::PathBuf::from(root).join("cryptty/cryptty.yaml");
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let valid = concat!(
                    "font: {size: 24}\n",
                    "window: {padding: {x: 6, y: 7}}\n",
                    "colors: {primary: {background: '#123456'}}\n",
                    "keyboard:\n  bindings:\n",
                    "    - {key: Enter, modifiers: [Shift], text: abc}\n",
                );
                for renderer_choice in
                    [RendererChoice::Software, RendererChoice::Auto]
                {
                    let proxy = self.proxy.clone();
                    let mut app = App {
                        view: None,
                        proxy,
                        command: vec!["/bin/cat".into()],
                        config: Config::parse("font: {size: 17}").unwrap(),
                        renderer_choice,
                        error: None,
                    };
                    app.create(events).unwrap();
                    let view = app.view.as_mut().unwrap();
                    view.terminal.feed(b"keep");
                    let old_columns = view.terminal.cols();
                    std::fs::write(&path, valid).unwrap();
                    view.reload_config();
                    assert!(!view.config_reload_failed);
                    assert_eq!(view.padding.x, 6);
                    assert_eq!(view.padding.y, 7);
                    assert_eq!(view.terminal.theme().background, 0x123456);
                    assert!(view.terminal.cols() < old_columns);
                    assert_eq!(
                        view.keyboard.text_for(
                            &Key::Named(NamedKey::Enter),
                            ModifiersState::SHIFT
                        ),
                        Some("abc")
                    );
                    let text: String = view.terminal.rows[0]
                        .iter()
                        .filter_map(|cell| match &cell.content {
                            cryptty::terminal::Content::Text { text } => {
                                Some(text.chars())
                            }
                            _ => None,
                        })
                        .flatten()
                        .collect();
                    assert!(text.starts_with("keep"));
                    let rows = view.terminal.rows.clone();
                    let cursor = view.terminal.cursor;
                    view.redraw.presented(
                        std::time::Instant::now(),
                        std::time::Duration::ZERO,
                    );
                    view.draw().unwrap();
                    let size = view.window.inner_size();
                    view.renderer.draw(
                        &view.terminal,
                        size.width,
                        size.height,
                        "",
                    );
                    assert_eq!(view.renderer.pixels[0], 0x123456);
                    for invalid in ["font: {size: 0}", "keyboard: ["] {
                        std::fs::write(&path, invalid).unwrap();
                        view.reload_config();
                        assert!(view.config_reload_failed);
                        assert_eq!(view.terminal.rows, rows);
                        assert_eq!(view.terminal.cursor, cursor);
                        assert_eq!(view.terminal.theme().background, 0x123456);
                        assert_eq!(
                            view.keyboard.text_for(
                                &Key::Named(NamedKey::Enter),
                                ModifiersState::SHIFT
                            ),
                            Some("abc")
                        );
                        assert!(
                            view.displayed_title
                                .ends_with(" — config reload failed")
                        );
                    }
                    std::fs::write(&path, valid.replace("abc", "xyz")).unwrap();
                    view.reload_config();
                    assert!(!view.config_reload_failed);
                    assert_eq!(
                        view.keyboard.text_for(
                            &Key::Named(NamedKey::Enter),
                            ModifiersState::SHIFT
                        ),
                        Some("xyz")
                    );
                    assert!(
                        !view
                            .displayed_title
                            .ends_with(" — config reload failed")
                    );
                }
                events.exit();
            }

            fn window_event(
                &mut self,
                _: &ActiveEventLoop,
                _: winit::window::WindowId,
                _: winit::event::WindowEvent,
            ) {
            }
        }
        let mut builder = EventLoop::<()>::with_user_event();
        builder.with_any_thread(true);
        let events = builder.build().unwrap();
        let mut check = Check {
            proxy: events.create_proxy(),
        };
        events.run_app(&mut check).unwrap();
    }
}
