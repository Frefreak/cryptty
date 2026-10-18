use super::*;
use crate::{App, RendererChoice};
use cryptty::config::Config;
use cryptty::pty::Event;
use cryptty::terminal::{Content, Terminal};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::ModifiersState;
use winit::platform::wayland::EventLoopBuilderExtWayland;

#[test]
fn pending_click_rejects_changed_link_meaning() {
    let mut terminal = Terminal::new(20, 3);
    terminal.feed(b"https://example.com");
    let pending = PendingLink {
        position: Position { row: 0, col: 2 },
        rows: terminal.rows.clone(),
    };
    terminal.feed(b"\x1b[H\x1b[31mhttps://example.com");
    assert!(pending.matches(&terminal));
    terminal.feed(b"\x1b[H\x1b]8;;https://other.test\x07https://example.com");
    assert!(!pending.matches(&terminal));
    let pending = PendingLink {
        position: Position { row: 0, col: 2 },
        rows: terminal.rows.clone(),
    };
    terminal.rows[0].wrapped = Some(20);
    assert!(!pending.matches(&terminal));
}

#[test]
#[ignore = "requires tmux and a native Wayland desktop"]
fn quick_ctrl_click_opens_complete_tmux_url_and_reaps_helper() {
    const CHILD: &str = "CRYPTTY_TMUX_CLICK_TEST";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let opener = directory.path().join("xdg-open");
        std::fs::write(
            &opener,
            "#!/bin/sh\nprintf '%s' \"$1\" > \"$CRYPTTY_LINK_OUTPUT\"\n",
        )
        .unwrap();
        std::fs::set_permissions(
            &opener,
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        for (capture_mouse, scheme) in [
            ("off", "https"),
            ("on", "https"),
            ("off", "file"),
            ("on", "file"),
        ] {
            let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                concat!(
                    "interaction::tmux_tests::",
                    "quick_ctrl_click_opens_complete_tmux_url_and_reaps_helper"
                ),
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("CRYPTTY_TEST_MOUSE", capture_mouse)
            .env("CRYPTTY_TEST_SCHEME", scheme)
            .env("TMUX_TMPDIR", directory.path())
            .env("CRYPTTY_LINK_OUTPUT", directory.path().join(format!("opened-url-{capture_mouse}-{scheme}")))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    directory.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env_remove("TMUX")
            .output()
            .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    }
    fn tmux(args: &[&str]) -> String {
        let output = Command::new("tmux")
            .args(["-L", "default"])
            .args(args)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    struct ServerGuard;
    impl Drop for ServerGuard {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .args(["-L", "default", "kill-server"])
                .env_remove("TMUX")
                .output();
        }
    }
    let _guard = ServerGuard;
    tmux(&[
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-s",
        "test",
        "-x",
        "100",
        "-y",
        "30",
        "sleep 30",
    ]);
    tmux(&["set-option", "-g", "status-position", "top"]);
    tmux(&[
        "set-option",
        "-g",
        "mouse",
        &std::env::var("CRYPTTY_TEST_MOUSE").unwrap(),
    ]);
    let prefix = if std::env::var("CRYPTTY_TEST_SCHEME").unwrap() == "file" {
        "file:///tmp/"
    } else {
        "https://example.com/"
    };
    let uri = format!("{prefix}{}", "abcdefghij/".repeat(8));
    let command = format!("printf '%s' '{uri}'; sleep 30");
    tmux(&["split-window", "-h", "-t", "test", &command]);

    struct Check {
        proxy: winit::event_loop::EventLoopProxy<()>,
        uri: String,
    }
    fn drain(view: &mut View) {
        while let Ok(Event::Output(bytes)) = view.pty.output.try_recv() {
            view.tmux.invalidate();
            let replies = view.terminal.feed(&bytes);
            view.send_input(replies).unwrap();
        }
        view.refresh_pointer();
    }
    impl ApplicationHandler<()> for Check {
        fn resumed(&mut self, events: &ActiveEventLoop) {
            let mut app = App {
                view: None,
                proxy: self.proxy.clone(),
                command: [
                    "tmux",
                    "-L",
                    "default",
                    "attach-session",
                    "-t",
                    "test",
                ]
                .into_iter()
                .map(Into::into)
                .collect(),
                config: Config::parse("font: {size: 14}").unwrap(),
                renderer_choice: RendererChoice::Software,
                error: None,
            };
            app.create(events).unwrap();
            let view = app.view.as_mut().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let position = loop {
                assert!(Instant::now() < deadline, "tmux redraw timed out");
                drain(view);
                let found = view.terminal.rows.iter().enumerate().find_map(
                    |(row, cells)| {
                        let line: String = cells
                            .iter()
                            .filter_map(|cell| match &cell.content {
                                Content::Text { text } => Some(text.chars()),
                                _ => None,
                            })
                            .flatten()
                            .collect();
                        line.find(&self.uri[..16]).map(|col| Position {
                            row,
                            col: line[..col].chars().count() + 2,
                        })
                    },
                );
                if let Some(position) = found {
                    break position;
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            // Let the first tmux redraw settle before accepting a click.
            std::thread::sleep(Duration::from_millis(30));
            drain(view);
            assert_ne!(view.terminal.link_at(position).unwrap().uri, self.uri);
            let (x, y) = view.renderer.cell_origin(position.col, position.row);
            view.pointer_moved(x as f64 + 1.0, y as f64 + 1.0).unwrap();
            loop {
                drain(view);
                if view.terminal.hovered_link.len() > 1 {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "unmodified hover timed out"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!view.renderer.set_link_tooltip(
                Some(&self.uri),
                (position.col, position.row),
            ));
            view.draw().unwrap();
            view.pointer_left();
            view.pointer_moved(x as f64 + 1.0, y as f64 + 1.0).unwrap();
            view.modifiers = ModifiersState::CONTROL;
            if std::env::var("CRYPTTY_TEST_MOUSE").unwrap() == "on" {
                view.modifiers |= ModifiersState::SHIFT;
            }
            view.refresh_pointer();
            view.mouse_button(MouseButton::Left, ElementState::Pressed)
                .unwrap();
            assert!(view.interaction.pending_link.is_some());
            view.modifiers = ModifiersState::empty();
            view.refresh_pointer();
            let opened =
                PathBuf::from(std::env::var_os("CRYPTTY_LINK_OUTPUT").unwrap());
            loop {
                assert!(
                    Instant::now() < deadline,
                    "pending URL click timed out"
                );
                drain(view);
                if opened.exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(std::fs::read_to_string(opened).unwrap(), self.uri);
            assert!(view.interaction.pending_link.is_none());
            view.modifiers = ModifiersState::CONTROL;
            if std::env::var("CRYPTTY_TEST_MOUSE").unwrap() == "on" {
                view.modifiers |= ModifiersState::SHIFT;
            }
            view.refresh_pointer();
            loop {
                drain(view);
                if view.terminal.hovered_link.len() > 1 {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "wrapped highlight timed out"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                view.terminal
                    .hovered_link
                    .iter()
                    .all(|range| range.start.col > view.terminal.cols() / 3)
            );
            view.modifiers = ModifiersState::empty();
            view.refresh_pointer();
            assert!(!view.renderer.set_link_tooltip(
                Some(&self.uri),
                (position.col, position.row),
            ));
            view.pointer_left();
            assert!(view.terminal.hovered_link.is_empty());
            assert!(!view.renderer.set_link_tooltip(None, (0, 0)));
            let size = (view.terminal.cols(), view.terminal.rows.len());
            view.tmux.update(
                false,
                size,
                Instant::now() + Duration::from_secs(6),
            );
            loop {
                let clients =
                    tmux(&["list-clients", "-F", "#{client_control_mode}"]);
                if clients.lines().all(|line| line == "0") {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "control client did not exit"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            app.view.take();
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
        uri,
    };
    events.run_app(&mut check).unwrap();
}
