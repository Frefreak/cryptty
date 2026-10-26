use super::*;
use crate::{App, RendererChoice};
use cryptty::config::Config;
use cryptty::pty::Event;
use cryptty::terminal::{Content, MouseMode};
use std::io::Write;
use std::process::{Command, Stdio};
use winit::application::ApplicationHandler;
use winit::event::{
    Ime, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, KeyLocation, PhysicalKey};
use winit::platform::wayland::EventLoopBuilderExtWayland;

fn press(view: &mut View, key: WindowKey, modifiers: ModifiersState) {
    let input = KeyInput {
        key: &key,
        unmodified: key.clone(),
        physical: PhysicalKey::Code(KeyCode::KeyJ),
        location: KeyLocation::Standard,
        text: None,
        modifiers,
        locks: 0,
        state: ElementState::Pressed,
        repeat: false,
    };
    assert!(view.vim_input(&input));
}

#[test]
fn marker_to_leap_preserves_panes_and_supports_selection_and_copy() {
    use cryptty::terminal::Position;
    use cryptty::tmux::PaneText;

    let mut display = Terminal::new(60, 4);
    display.feed(b"left text\x1b[1;31Hbrew list");
    let mut left = Terminal::new(20, 4);
    left.feed(b"left text");
    let mut right = Terminal::new(20, 4);
    right.feed(b"brew list");
    let marker = cryptty::marker::Session::new(
        display,
        vec![
            PaneText {
                origin: Position { row: 0, col: 0 },
                terminal: left,
            },
            PaneText {
                origin: Position { row: 0, col: 30 },
                terminal: right,
            },
        ],
        &Default::default(),
    );
    let (mut mode, action) = Mode::Marker(Box::new(marker)).key(Key::Char('s'));
    assert!(matches!(action, Action::Stay));
    assert!(matches!(mode, Mode::Active(_)));
    assert!(mode.status().contains("first character"));
    for key in ['b', 'r', 'v', 'e'] {
        let (next, action) = mode.key(Key::Char(key));
        assert!(matches!(action, Action::Stay));
        mode = next;
    }
    let (_, action) = mode.key(Key::Char('y'));
    assert!(matches!(action, Action::Copy(text) if text == "brew"));
}

#[test]
fn marker_prefix_keeps_s_in_label_input() {
    let mut display = Terminal::new(24, 651);
    let text = (0..650)
        .map(|n| format!("src/{n}.rs\r\n"))
        .collect::<String>();
    display.feed(text.as_bytes());
    let marker =
        cryptty::marker::Session::new(display, Vec::new(), &Default::default());
    let target = marker
        .visible_targets()
        .into_iter()
        .find(|t| t.label.ends_with('s'))
        .unwrap();
    let first = target.label.chars().next().unwrap();
    let (mode, _) = Mode::Marker(Box::new(marker)).key(Key::Char(first));
    let (mode, action) = mode.key(Key::Char('s'));
    assert!(matches!(mode, Mode::Marker(_)));
    assert!(matches!(action, Action::Copy(text) if text == target.text));
    let (mode, _) = mode.key(Key::Backspace);
    let (mode, _) = mode.key(Key::Char('s'));
    assert!(matches!(mode, Mode::Active(_)));
}

#[test]
#[ignore = "requires tmux and a native Wayland desktop"]
fn frozen_tmux_vim_mode_copies_and_returns_to_live_output() {
    const CHILD: &str = "CRYPTTY_VIM_WINDOW_TEST";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        for renderer in ["software", "auto"] {
            let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact",
                "vim_mode::tests::frozen_tmux_vim_mode_copies_and_returns_to_live_output",
                "--ignored", "--nocapture"])
            .env(CHILD, "1")
            .env("CRYPTTY_VIM_RENDERER", renderer)
            .env("TMUX_TMPDIR", directory.path())
            .env_remove("TMUX")
            .output().unwrap();
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
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .args(["-L", "default", "kill-server"])
                .env_remove("TMUX")
                .output();
        }
    }
    let _guard = Guard;
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
        "printf 'left beta'; sleep 60",
    ]);
    tmux(&["set-option", "-g", "status-position", "top"]);
    tmux(&["set-option", "-g", "mouse", "on"]);
    let directory = std::env::var("TMUX_TMPDIR").unwrap();
    let input_file = format!("{directory}/received");
    let fixture = format!(
        "printf 'right beta'; stty -echo -icanon; cat > '{input_file}'"
    );
    tmux(&["split-window", "-h", "-t", "test", &fixture]);
    struct Check {
        proxy: winit::event_loop::EventLoopProxy<()>,
    }
    fn drain(view: &mut View) {
        while let Ok(Event::Output(bytes)) = view.pty.output.try_recv() {
            if view.vim.is_none() {
                view.tmux.invalidate();
            }
            let replies = view.terminal.feed(&bytes);
            view.send_input(replies).unwrap();
        }
        view.refresh_pointer();
    }
    fn has_text(terminal: &Terminal, needle: &str) -> bool {
        terminal.rows.iter().any(|row| {
            let text: String = row
                .iter()
                .filter_map(|cell| match &cell.content {
                    Content::Text { text } => Some(text.chars()),
                    _ => None,
                })
                .flatten()
                .collect();
            text.contains(needle)
        })
    }
    impl ApplicationHandler<()> for Check {
        fn resumed(&mut self, events: &ActiveEventLoop) {
            let mut clipboard = arboard::Clipboard::new().unwrap();
            let saved_text = clipboard.get_text().ok();
            let saved_image = Command::new("wl-paste")
                .args(["--type", "image/png"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| output.stdout);
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
                renderer_choice: if std::env::var("CRYPTTY_VIM_RENDERER")
                    .unwrap()
                    == "auto"
                {
                    RendererChoice::Auto
                } else {
                    RendererChoice::Software
                },
                opengl_color_format: None,
                error: None,
            };
            app.create(events).unwrap();
            let view = app.view.as_mut().unwrap();
            view.keyboard = Config::parse(
                "keyboard: {bindings: [{key: F6, modifiers: [Alt], action: VimMode}]}",
            ).unwrap().keyboard;
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                drain(view);
                if has_text(&view.terminal, "right beta") {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(30));
            drain(view);
            let entry = WindowKey::Named(NamedKey::F6);
            let mut repeat = KeyInput {
                key: &entry,
                unmodified: entry.clone(),
                physical: PhysicalKey::Code(KeyCode::F6),
                location: KeyLocation::Standard,
                text: None,
                modifiers: ModifiersState::ALT,
                locks: 0,
                state: ElementState::Pressed,
                repeat: true,
            };
            // A held key keeps its existing route when modifiers change.
            assert!(!view.vim_input(&repeat));
            assert!(view.vim.is_none());
            repeat.repeat = false;
            view.preedit = "composing".into();
            assert!(!view.vim_input(&repeat));
            assert!(view.vim.is_none());
            view.preedit.clear();
            press(view, WindowKey::Named(NamedKey::F6), ModifiersState::ALT);
            // Fast typing must survive the asynchronous pane capture.
            press(
                view,
                WindowKey::Character("b".into()),
                ModifiersState::empty(),
            );
            press(
                view,
                WindowKey::Character("e".into()),
                ModifiersState::empty(),
            );
            while matches!(view.vim, Some(Mode::Loading { .. })) {
                drain(view);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(matches!(view.vim, Some(Mode::Active(_))));
            assert!(view.vim.as_ref().unwrap().screen().vim_mode);
            let first = view.vim.as_ref().unwrap().screen().cursor;
            assert!(first.0 > view.terminal.cols() / 2);
            press(
                view,
                WindowKey::Named(NamedKey::Enter),
                ModifiersState::empty(),
            );
            assert!(
                view.vim.as_ref().unwrap().screen().cursor.0
                    < view.terminal.cols() / 2
            );
            press(
                view,
                WindowKey::Named(NamedKey::Backspace),
                ModifiersState::empty(),
            );
            assert_eq!(view.vim.as_ref().unwrap().screen().cursor, first);
            let frozen = view.vim.as_ref().unwrap().screen().rows.clone();
            assert!(has_text(
                view.vim.as_ref().unwrap().screen(),
                "right beta"
            ));
            press(
                view,
                WindowKey::Character("v".into()),
                ModifiersState::CONTROL,
            );
            press(
                view,
                WindowKey::Character("f".into()),
                ModifiersState::empty(),
            );
            press(
                view,
                WindowKey::Character("a".into()),
                ModifiersState::empty(),
            );
            view.terminal.mouse_mode = MouseMode::Motion;
            view.pointer_moved(20.0, 20.0).unwrap();
            view.mouse_button(MouseButton::Middle, ElementState::Pressed)
                .unwrap();
            view.mouse_wheel(
                MouseScrollDelta::LineDelta(0.0, 1.0),
                TouchPhase::Moved,
            )
            .unwrap();
            press(
                view,
                WindowKey::Character("v".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
            );
            view.pty.flush_input().unwrap();
            std::thread::sleep(Duration::from_millis(20));
            let received = std::fs::read(format!(
                "{}/received",
                std::env::var("TMUX_TMPDIR").unwrap()
            ))
            .unwrap();
            assert!(received.is_empty(), "modal input reached the pane");
            tmux(&[
                "respawn-pane",
                "-k",
                "-t",
                "test:0.1",
                "printf 'right CHANGED'; sleep 60",
            ]);
            loop {
                drain(view);
                if has_text(&view.terminal, "CHANGED") {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!has_text(view.vim.as_ref().unwrap().screen(), "CHANGED"));
            // Selection rendering changes styles, while frozen text stays put.
            assert_ne!(frozen, view.terminal.rows);
            view.redraw.presented(Instant::now(), Duration::ZERO);
            view.draw().unwrap();
            let screen = view.vim.as_ref().unwrap().screen();
            view.renderer.draw(screen, 900, 560, "");
            assert!(view.renderer.pixels.iter().any(|&p| p != 0x282a36));
            press(
                view,
                WindowKey::Character("y".into()),
                ModifiersState::empty(),
            );
            assert!(view.vim.is_none());
            assert_eq!(clipboard.get_text().unwrap(), "beta");
            assert!(has_text(&view.terminal, "CHANGED"));
            // Enter from a scrolled tmux copy viewport using the default key.
            tmux(&[
                "respawn-pane",
                "-k",
                "-t",
                "test:0.1",
                "printf 'history target\\n'; seq 0 79 | sed 's/^/filler /'; sleep 60",
            ]);
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                drain(view);
                if has_text(&view.terminal, "filler 79") {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            tmux(&["copy-mode", "-t", "test:0.1"]);
            tmux(&["send-keys", "-t", "test:0.1", "-X", "history-top"]);
            loop {
                drain(view);
                if has_text(&view.terminal, "history target") {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(30));
            drain(view);
            let scroll = tmux(&[
                "display-message",
                "-p",
                "-t",
                "test:0.1",
                "#{scroll_position}",
            ]);
            view.keyboard = Config::default().keyboard;
            press(
                view,
                WindowKey::Character("j".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
            );
            press(
                view,
                WindowKey::Character("t".into()),
                ModifiersState::empty(),
            );
            press(
                view,
                WindowKey::Character("a".into()),
                ModifiersState::empty(),
            );
            while matches!(view.vim, Some(Mode::Loading { .. })) {
                drain(view);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(matches!(view.vim, Some(Mode::Active(_))));
            press(
                view,
                WindowKey::Character("y".into()),
                ModifiersState::empty(),
            );
            press(
                view,
                WindowKey::Character("w".into()),
                ModifiersState::empty(),
            );
            assert!(view.vim.is_none());
            assert_eq!(clipboard.get_text().unwrap().trim_end(), "target");
            assert_eq!(
                tmux(&[
                    "display-message",
                    "-p",
                    "-t",
                    "test:0.1",
                    "#{pane_mode}",
                ])
                .trim(),
                "copy-mode"
            );
            assert_eq!(
                tmux(&[
                    "display-message",
                    "-p",
                    "-t",
                    "test:0.1",
                    "#{scroll_position}",
                ]),
                scroll
            );
            // IME text is routed to Leap, not the PTY.
            view.vim = Some(Mode::Active(Box::new(Session::new(
                {
                    let mut terminal = Terminal::new(
                        view.terminal.cols(),
                        view.terminal.rows.len(),
                    );
                    terminal.feed("中文测试".as_bytes());
                    terminal
                },
                Vec::new(),
            ))));
            view.event(
                events,
                WindowEvent::Ime(Ime::Preedit("zhong".into(), None)),
            )
            .unwrap();
            press(
                view,
                WindowKey::Character("z".into()),
                ModifiersState::empty(),
            );
            assert!(
                view.vim
                    .as_ref()
                    .unwrap()
                    .status()
                    .contains("first character")
            );
            view.event(events, WindowEvent::Ime(Ime::Commit("中文".into())))
                .unwrap();
            assert!(view.preedit.is_empty());
            assert!(view.vim.as_ref().unwrap().status().contains("NORMAL"));
            view.resize().unwrap();
            assert!(view.vim.is_none());
            if let Some(image) = saved_image {
                let mut copy = Command::new("wl-copy")
                    .args(["--type", "image/png"])
                    .stdin(Stdio::piped())
                    .spawn()
                    .unwrap();
                copy.stdin.take().unwrap().write_all(&image).unwrap();
                assert!(copy.wait().unwrap().success());
            } else if let Some(text) = saved_text {
                clipboard.set_text(text).unwrap();
            }
            app.view.take();
            events.exit();
        }
        fn window_event(
            &mut self,
            _: &ActiveEventLoop,
            _: winit::window::WindowId,
            _: WindowEvent,
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
