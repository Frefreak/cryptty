use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use cryptty::{
    input,
    pty::{Event, Pty},
    terminal::{Content, Terminal},
};
use portable_pty::PtySize;

struct Fixture {
    directory: PathBuf,
    socket: String,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket = format!("cryptty-{name}-{}-{unique}", std::process::id());
        let directory = std::env::temp_dir().join(&socket);
        std::fs::create_dir(&directory).unwrap();
        Self { directory, socket }
    }

    fn tmux(&self, args: &[&str]) {
        let output = Command::new("tmux")
            .args(["-L", &self.socket])
            .args(args)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .env_remove("TMUX")
            .output();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn screen(terminal: &Terminal) -> String {
    let mut text = String::new();
    for row in &terminal.rows {
        for cell in row {
            if let Content::Text { text: value, .. } = &cell.content {
                text.extend(value.chars());
            }
        }
        text.push('\n');
    }
    text
}

fn drive(
    pty: &mut Pty,
    terminal: &mut Terminal,
    step: impl FnMut(&mut Pty, &mut Terminal),
) {
    drive_with_delay(pty, terminal, Duration::ZERO, step);
}

fn drive_with_delay(
    pty: &mut Pty,
    terminal: &mut Terminal,
    delay: Duration,
    step: impl FnMut(&mut Pty, &mut Terminal),
) {
    drive_with_replies(pty, terminal, delay, |replies| replies, step);
}

fn drive_with_replies(
    pty: &mut Pty,
    terminal: &mut Terminal,
    delay: Duration,
    mut map_reply: impl FnMut(Vec<u8>) -> Vec<u8>,
    mut step: impl FnMut(&mut Pty, &mut Terminal),
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match pty
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("application must respond before the deadline")
        {
            Event::Output(bytes) => {
                let replies = map_reply(terminal.feed(&bytes));
                if !replies.is_empty() {
                    std::thread::sleep(delay);
                }
                assert!(pty.write(replies).unwrap());
                step(pty, terminal);
            }
            Event::Closed => break,
            Event::Error(error) => panic!("{error}"),
        }
    }
}

#[test]
#[ignore = "requires an interactive Neovim on PATH"]
fn neovim_paste_resize_write_and_restore_shell() {
    let fixture = Fixture::new("nvim");
    let file = fixture.directory.join("edited.txt");
    let command: Vec<OsString> = vec![
        "/bin/sh".into(), "-c".into(),
        "printf 'SHELL_BEFORE\\r\\n'; nvim --clean -i NONE -n \"$1\"; printf 'SHELL_AFTER\\r\\n'".into(),
        "cryptty-test".into(), file.clone().into(),
    ];
    let mut pty = Pty::spawn(&command, PtySize::default(), || {}).unwrap();
    let mut terminal = Terminal::new(80, 24);
    let mut stage = 0;
    drive(&mut pty, &mut terminal, |pty, terminal| {
        if stage == 0 && terminal.alternate_screen() && terminal.bracketed_paste
        {
            let mut bytes = b"i".to_vec();
            bytes.extend(input::paste("hello 中\nsecond line", true));
            assert!(pty.write(bytes).unwrap());
            stage = 1;
        } else if stage == 1 && screen(terminal).contains("second line") {
            terminal.resize(66, 16);
            pty.resize(PtySize {
                cols: 66,
                rows: 16,
                ..PtySize::default()
            })
            .unwrap();
            assert!(pty.write(b"\x1b:wq\r".to_vec()).unwrap());
            stage = 2;
        }
    });
    assert_eq!(stage, 2);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "hello 中\nsecond line\n"
    );
    assert!(!terminal.alternate_screen());
    assert!(!terminal.bracketed_paste);
    assert!(screen(&terminal).contains("SHELL_BEFORE"));
    assert!(screen(&terminal).contains("SHELL_AFTER"));
}

#[test]
#[ignore = "requires an interactive tmux on PATH"]
fn tmux_split_resize_and_restore_shell() {
    let fixture = Fixture::new("tmux");
    let config = fixture.directory.join("tmux.conf");
    std::fs::write(
        &config,
        "set -g status off\nset -g mouse on\nset -g default-shell /bin/sh\n",
    )
    .unwrap();
    let command: Vec<OsString> = vec![
        "/bin/sh".into(), "-c".into(),
        "unset TMUX; printf 'SHELL_BEFORE\\r\\n'; tmux -L \"$1\" -f \"$2\" new-session -s test /bin/sh; printf 'SHELL_AFTER\\r\\n'".into(),
        "cryptty-test".into(), fixture.socket.clone().into(), config.into(),
    ];
    let mut pty = Pty::spawn(&command, PtySize::default(), || {}).unwrap();
    let mut terminal = Terminal::new(80, 24);
    let mut stage = 0;
    drive(&mut pty, &mut terminal, |pty, terminal| {
        if stage == 0 && terminal.alternate_screen() {
            assert!(pty.write(b"printf '\\nTMUX_LEFT\\n'\r".to_vec()).unwrap());
            stage = 1;
        } else if stage == 1 && screen(terminal).contains("TMUX_LEFT") {
            fixture.tmux(&["split-window", "-h", "-t", "test:0", "/bin/sh"]);
            terminal.resize(100, 30);
            pty.resize(PtySize {
                cols: 100,
                rows: 30,
                ..PtySize::default()
            })
            .unwrap();
            assert!(
                pty.write(b"printf '\\nTMUX_RIGHT\\n'\r".to_vec()).unwrap()
            );
            stage = 2;
        } else if stage == 2 {
            let visible = screen(terminal);
            if visible.contains("TMUX_LEFT") && visible.contains("TMUX_RIGHT") {
                fixture.tmux(&["kill-session", "-t", "test"]);
                stage = 3;
            }
        }
    });
    assert_eq!(stage, 3, "{}", screen(&terminal));
    assert!(!terminal.alternate_screen());
    assert!(screen(&terminal).contains("SHELL_BEFORE"));
    assert!(screen(&terminal).contains("SHELL_AFTER"));
}

#[test]
#[ignore = "requires tmux with extended-key support on PATH"]
fn tmux_extended_keys_preserve_shift_enter_and_prefix() {
    use winit::event::ElementState;
    use winit::keyboard::{
        Key, KeyCode, KeyLocation, ModifiersState, NamedKey, PhysicalKey,
    };
    let key = |terminal: &Terminal, key: Key, modifiers| {
        input::encode_key(
            &input::kitty::KeyInput {
                key: &key,
                unmodified: key.clone(),
                physical: PhysicalKey::Code(KeyCode::Enter),
                location: KeyLocation::Standard,
                text: None,
                modifiers,
                locks: 0,
                state: ElementState::Pressed,
                repeat: false,
            },
            terminal.keyboard.flags(),
            terminal.modify_other_keys_2,
            terminal.application_cursor,
            terminal.application_keypad,
        )
    };
    for (format, extended, expected) in [
        ("xterm", true, b"\x1b[27;2;13~\r".as_slice()),
        ("csi-u", true, b"\x1b[13;2u\r".as_slice()),
        ("xterm", false, b"\x1b\r\r".as_slice()),
    ] {
        let fixture = Fixture::new("tmux-keys");
        let config = fixture.directory.join("tmux.conf");
        let capture = fixture.directory.join("keys.bin");
        let reader = fixture.directory.join("read.sh");
        std::fs::write(
            &config,
            format!(
                "set -g status off\nset -s extended-keys {}\n\
             set -s extended-keys-format {format}\n\
             set -as terminal-features ',xterm-256color:extkeys'\n",
                if extended { "on" } else { "off" },
            ),
        )
        .unwrap();
        std::fs::write(
            &reader,
            "stty raw -echo\n\
             printf '\\033[>4;2mKEY_READY'\n\
             dd bs=1 count=\"$2\" of=\"$1\" 2>/dev/null\n\
             printf '\\r\\nKEY_DONE'\n\
             sleep 30\n",
        )
        .unwrap();
        let command: Vec<OsString> = vec![
            "/bin/sh".into(), "-c".into(),
            "unset TMUX; tmux -L \"$1\" -f \"$2\" new-session -s test /bin/sh \"$3\" \"$4\" \"$5\"; printf 'SHELL_AFTER\\r\\n'".into(),
            "cryptty-test".into(), fixture.socket.clone().into(),
            config.into(), reader.into(), capture.clone().into(),
            expected.len().to_string().into(),
        ];
        let mut pty = Pty::spawn(&command, PtySize::default(), || {}).unwrap();
        let mut terminal = Terminal::new(80, 24);
        let mut stage = 0;
        drive(&mut pty, &mut terminal, |pty, terminal| {
            let visible = screen(terminal);
            if stage == 0 && visible.contains("KEY_READY") {
                assert_eq!(terminal.modify_other_keys_2, extended);
                let mut bytes = if extended {
                    let bytes = key(
                        terminal,
                        Key::Named(NamedKey::Enter),
                        ModifiersState::SHIFT,
                    );
                    assert_eq!(bytes, b"\x1b[27;2;13~");
                    bytes
                } else {
                    let config = cryptty::config::Config::parse(
                        r#"
keyboard:
  bindings:
    - {key: Enter, modifiers: [Shift], text: "\u001b\r"}
"#,
                    )
                    .unwrap();
                    config
                        .keyboard
                        .text_for(
                            &Key::Named(NamedKey::Enter),
                            ModifiersState::SHIFT,
                        )
                        .unwrap()
                        .as_bytes()
                        .to_vec()
                };
                bytes.extend(key(
                    terminal,
                    Key::Named(NamedKey::Enter),
                    ModifiersState::empty(),
                ));
                assert!(pty.write(bytes).unwrap());
                stage = 1;
            } else if stage == 1 && visible.contains("KEY_DONE") {
                assert_eq!(std::fs::read(&capture).unwrap(), expected);
                let mut bytes = key(
                    terminal,
                    Key::Character("b".into()),
                    ModifiersState::CONTROL,
                );
                bytes.push(b'd');
                assert!(pty.write(bytes).unwrap());
                stage = 2;
            }
        });
        assert_eq!(stage, 2, "{}", screen(&terminal));
        assert!(!terminal.modify_other_keys_2);
        assert!(screen(&terminal).contains("SHELL_AFTER"));
    }
}

#[test]
#[ignore = "requires tmux on PATH and a native PTY"]
fn tmux_image_sender_survives_pane_switch_resize_and_clear() {
    use cryptty::graphics::Placeholder;
    let fixture = Fixture::new("images");
    let config = fixture.directory.join("tmux.conf");
    std::fs::write(
        &config,
        concat!(
            "set -g allow-passthrough off\n",
            "set -g status off\n",
            "set -g default-terminal tmux-256color\n",
        ),
    )
    .unwrap();
    let image = fixture.directory.join("image.png");
    image::RgbaImage::from_fn(32, 32, |x, y| {
        image::Rgba([x as u8 * 7, y as u8 * 7, 128, 255])
    })
    .save(&image)
    .unwrap();
    let script = fixture.directory.join("show.sh");
    std::fs::write(
        &script,
        format!(
            concat!(
                "#!/bin/sh\ntmux split-window -h /bin/sh\n",
                "'{}' image '{}'\nprintf IMAGE_READY\nread done\n"
            ),
            env!("CARGO_BIN_EXE_cryptty"),
            image.display()
        ),
    )
    .unwrap();
    let command: Vec<OsString> = vec![
        "tmux".into(),
        "-u".into(),
        "-L".into(),
        fixture.socket.clone().into(),
        "-f".into(),
        config.into_os_string(),
        "new-session".into(),
        "/bin/sh".into(),
        script.into_os_string(),
    ];
    let mut pty = Pty::spawn(
        &command,
        PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 800,
            pixel_height: 480,
        },
        || {},
    )
    .unwrap();
    let mut terminal = Terminal::new(80, 24);
    terminal.cell_pixels = (10, 20);
    let mut phase = 0;
    drive(&mut pty, &mut terminal, |pty, terminal| {
        let visible = terminal.rows.iter().any(|row| {
            row.iter()
                .any(|cell| Placeholder::read(cell, None).is_some())
        });
        let ready = screen(terminal).contains("IMAGE_READY");
        match phase {
            0 if ready && visible => {
                assert_eq!(terminal.graphics.images.len(), 1);
                let option = |args: &[&str]| {
                    let output = Command::new("tmux")
                        .args(["-L", &fixture.socket])
                        .args(args)
                        .env_remove("TMUX")
                        .output()
                        .unwrap();
                    assert!(output.status.success());
                    String::from_utf8(output.stdout).unwrap()
                };
                assert_eq!(
                    option(&["display-message", "-p", "#{pane_index}"]).trim(),
                    "1",
                    "probing must restore the pane focused by split-window",
                );
                assert_eq!(
                    option(&[
                        "show-options",
                        "-pAv",
                        "-t",
                        "0:0.0",
                        "allow-passthrough"
                    ]),
                    "on\n"
                );
                assert_eq!(
                    option(&["show-options", "-gv", "allow-passthrough"]),
                    "off\n"
                );
                assert_eq!(
                    option(&[
                        "show-options",
                        "-pAv",
                        "-t",
                        "0:0.1",
                        "allow-passthrough"
                    ]),
                    "off\n"
                );
                phase = 1;
                fixture.tmux(&["new-window", "-n", "other", "/bin/sh"]);
                assert_eq!(
                    option(&[
                        "show-options",
                        "-pAv",
                        "-t",
                        "0:1.0",
                        "allow-passthrough"
                    ]),
                    "off\n"
                );
            }
            1 if !visible && !ready => {
                phase = 2;
                fixture.tmux(&["select-window", "-t", "0"]);
            }
            2 if ready && visible => {
                phase = 3;
                terminal.resize(60, 20);
                pty.resize(PtySize {
                    rows: 20,
                    cols: 60,
                    pixel_width: 600,
                    pixel_height: 400,
                })
                .unwrap();
                fixture.tmux(&["refresh-client"]);
            }
            3 if ready && visible => {
                phase = 4;
                fixture.tmux(&["send-keys", "-t", "0", "Enter"]);
                fixture.tmux(&["kill-server"]);
            }
            _ => (),
        }
    });
    assert_eq!(phase, 4, "image did not survive tmux lifecycle");
}

#[test]
#[ignore = "requires tmux on PATH and a native PTY"]
fn image_pane_preserves_paths_and_closes_on_enter() {
    let fixture = Fixture::new("image-pane");
    let config = fixture.directory.join("tmux.conf");
    std::fs::write(
        &config,
        concat!(
            "set -g allow-passthrough off\nset -g status off\n",
            "set -g remain-on-exit on\n"
        ),
    )
    .unwrap();
    let image = fixture.directory.join("a ' $(exit 73).png");
    image::RgbaImage::from_pixel(336, 1200, image::Rgba([70, 90, 130, 255]))
        .save(&image)
        .unwrap();
    let script = fixture.directory.join("preview.sh");
    std::fs::write(
        &script,
        concat!(
            "#!/bin/sh\n",
            "tmux split-window -h -l 60% /bin/sh\n",
            "tmux display-message -p -t \"$TMUX_PANE\" ",
            "'#{window_layout}' > \"$3.layout\"\n",
            "\"$1\" image --pane \"$2\" > \"$3\"\n",
            "printf PREVIEW_LAUNCHED\nread done\n"
        ),
    )
    .unwrap();
    let report = fixture.directory.join("launch.txt");
    let command: Vec<OsString> = vec![
        "tmux".into(),
        "-u".into(),
        "-L".into(),
        fixture.socket.clone().into(),
        "-f".into(),
        config.into_os_string(),
        "new-session".into(),
        "/bin/sh".into(),
        script.into_os_string(),
        env!("CARGO_BIN_EXE_cryptty").into(),
        image.into_os_string(),
        report.clone().into_os_string(),
    ];
    let mut pty = Pty::spawn(
        &command,
        PtySize {
            rows: 48,
            cols: 201,
            pixel_width: 4020,
            pixel_height: 1920,
        },
        || {},
    )
    .unwrap();
    let mut terminal = Terminal::new(201, 48);
    let mut phase = 0;
    drive(&mut pty, &mut terminal, |_, terminal| {
        let text = screen(terminal);
        if phase == 0 && text.contains("Press Enter to close") {
            assert_eq!(terminal.graphics.images.len(), 1);
            assert!(
                std::fs::read_to_string(&report)
                    .unwrap()
                    .contains("Opened image preview in pane")
            );
            phase = 1;
            fixture.tmux(&["send-keys", "-t", "0:0.2", "Enter"]);
        } else if phase == 1
            && text.contains("PREVIEW_LAUNCHED")
            && !text.contains("Press Enter to close")
        {
            let output = Command::new("tmux")
                .args(["-L", &fixture.socket, "list-panes", "-F", "#{pane_id}"])
                .env_remove("TMUX")
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().lines().count(),
                2
            );
            let layout = Command::new("tmux")
                .args([
                    "-L",
                    &fixture.socket,
                    "display-message",
                    "-p",
                    "-t",
                    "0:0",
                    "#{window_layout}",
                ])
                .env_remove("TMUX")
                .output()
                .unwrap();
            assert!(layout.status.success());
            assert_eq!(
                String::from_utf8(layout.stdout).unwrap(),
                std::fs::read_to_string(report.with_extension("txt.layout"))
                    .unwrap()
            );
            phase = 2;
            fixture.tmux(&["kill-server"]);
        }
    });
    assert_eq!(phase, 2, "preview did not open and close");
}

#[test]
#[ignore = "requires tmux on PATH and a native PTY"]
fn image_preview_uploads_after_focus_returns_to_source_pane() {
    check_preview_upload(false);
}

#[test]
#[ignore = "requires tmux on PATH and a native PTY"]
fn image_preview_uploads_from_a_hidden_window() {
    check_preview_upload(true);
}

fn check_preview_upload(hidden_window: bool) {
    let fixture = Fixture::new(if hidden_window {
        "image-preview-hidden"
    } else {
        "image-preview-focus"
    });
    let config = fixture.directory.join("tmux.conf");
    std::fs::write(
        &config,
        concat!(
            "set -g status off\nset -g allow-passthrough off\n",
            "set-hook -g after-split-window 'select-pane -t 0'\n"
        ),
    )
    .unwrap();
    let image = fixture.directory.join("image.png");
    image::RgbaImage::from_pixel(1254, 1254, image::Rgba([70, 90, 130, 255]))
        .save(&image)
        .unwrap();
    let script = fixture.directory.join("preview.sh");
    std::fs::write(
        &script,
        format!(
            concat!(
                "{}",
                "\"$1\" image --pane \"$2\" > \"$3\"\n",
                "printf PREVIEW_LAUNCHED\nread done\n"
            ),
            if hidden_window {
                "tmux new-window /bin/sh\n"
            } else {
                ""
            }
        ),
    )
    .unwrap();
    let report = fixture.directory.join("launch.txt");
    let command: Vec<OsString> = vec![
        "tmux".into(),
        "-u".into(),
        "-L".into(),
        fixture.socket.clone().into(),
        "-f".into(),
        config.into_os_string(),
        "new-session".into(),
        "/bin/sh".into(),
        script.into_os_string(),
        env!("CARGO_BIN_EXE_cryptty").into(),
        image.into_os_string(),
        report.into_os_string(),
    ];
    let mut pty = Pty::spawn(
        &command,
        PtySize {
            rows: 48,
            cols: 201,
            pixel_width: 4020,
            pixel_height: 1920,
        },
        || {},
    )
    .unwrap();
    let mut terminal = Terminal::new(201, 48);
    let mut checked = false;
    drive(&mut pty, &mut terminal, |_, terminal| {
        let text = screen(terminal);
        if !checked
            && text.contains("Press Enter to close")
            && text.contains("PREVIEW_LAUNCHED")
        {
            assert_eq!(terminal.graphics.images.len(), 1);
            assert!(text.contains(cryptty::graphics::MARKER));
            let mut found = 0;
            for row in &terminal.rows {
                let mut previous = None;
                let mut image_col = 0;
                for cell in row {
                    previous =
                        cryptty::graphics::Placeholder::read(cell, previous);
                    if let Some(placeholder) = previous {
                        assert_eq!(
                            placeholder.col, image_col,
                            "row={}, cell={:?}",
                            placeholder.row, cell
                        );
                        image_col += 1;
                        assert!(
                            terminal
                                .graphics
                                .images
                                .contains_key(&u64::from(placeholder.image)),
                            "placeholder id {} missing from {:?}",
                            placeholder.image,
                            terminal.graphics.images.keys().collect::<Vec<_>>()
                        );
                        found += 1;
                    }
                }
            }
            assert!(found > 0, "image placeholders must be drawable");
            assert!(!text.contains("Timed out"));
            let output = Command::new("tmux")
                .args([
                    "-L",
                    &fixture.socket,
                    "list-panes",
                    "-F",
                    "#{pane_index}:#{pane_active}:#{allow-passthrough}",
                ])
                .env_remove("TMUX")
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                "0:1:off\n1:0:all\n"
            );
            checked = true;
            fixture.tmux(&["kill-server"]);
        }
    });
    assert!(checked, "inactive preview must finish uploading");
}

#[test]
#[ignore = "requires tmux on PATH and native PTYs"]
fn image_probe_finds_remote_and_nested_tmux_paths() {
    for depth in 0..=3 {
        image_probe_case(depth, true, Duration::ZERO, ProbeReply::Normal);
    }
}

#[test]
#[ignore = "requires tmux on PATH and a native PTY"]
fn image_probe_reports_blocked_passthrough_and_restores_tty() {
    image_probe_case(1, false, Duration::ZERO, ProbeReply::Normal);
}

#[test]
#[ignore = "requires a native PTY"]
fn image_probe_accepts_slow_ssh_responses() {
    image_probe_case(0, true, Duration::from_millis(900), ProbeReply::Normal);
}

#[derive(Clone, Copy)]
enum ProbeReply {
    Normal,
    MissingIdentity,
    MissingGraphics,
    RejectedGraphics,
}

#[test]
#[ignore = "requires native PTYs"]
fn image_probe_diagnostics_distinguish_query_failures() {
    for reply in [
        ProbeReply::MissingIdentity,
        ProbeReply::MissingGraphics,
        ProbeReply::RejectedGraphics,
    ] {
        image_probe_case(0, true, Duration::ZERO, reply);
    }
}

fn image_probe_case(
    depth: usize,
    passthrough: bool,
    delay: Duration,
    reply: ProbeReply,
) {
    let succeeds = passthrough && matches!(reply, ProbeReply::Normal);
    let fixture = Fixture::new("probe");
    let layers: Vec<_> = (0..depth).map(|_| Fixture::new("layer")).collect();
    let image = fixture.directory.join("image.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([20, 40, 80, 255]))
        .save(&image)
        .unwrap();
    let script = fixture.directory.join("sender.sh");
    std::fs::write(
        &script,
        format!(
            concat!(
                "before=$(stty -g)\n",
                "env -u TMUX -u TMUX_PANE '{}' image '{}'\n",
                "result=$?\n",
                "test \"$(stty -g)\" = \"$before\" || exit 73\n",
                "printf 'PROBE_EXIT_%s\\n' \"$result\"\nread done\n"
            ),
            env!("CARGO_BIN_EXE_cryptty"),
            image.display(),
        ),
    )
    .unwrap();
    let mut command: Vec<OsString> = vec!["/bin/sh".into(), script.into()];
    for layer in &layers {
        let config = layer.directory.join("tmux.conf");
        std::fs::write(
            &config,
            format!(
                "set -g status off\nset -g allow-passthrough {}\nset-hook -g after-new-session 'select-pane -T PROBE_TITLE'\n",
                if passthrough { "on" } else { "off" },
            ),
        )
        .unwrap();
        let mut outer: Vec<OsString> = vec![
            "env".into(),
            "-u".into(),
            "TMUX".into(),
            "tmux".into(),
            "-u".into(),
            "-L".into(),
            layer.socket.clone().into(),
            "-f".into(),
            config.into(),
            "new-session".into(),
        ];
        outer.extend(command);
        command = outer;
    }
    let mut pty = Pty::spawn(&command, PtySize::default(), || {}).unwrap();
    let mut terminal = Terminal::new(80, 24);
    let mut checked = false;
    drive_with_replies(
        &mut pty,
        &mut terminal,
        delay,
        |bytes| match reply {
            ProbeReply::MissingIdentity if bytes.starts_with(b"\x1b[>") => {
                b"UNRELATED_INPUT".to_vec()
            }
            ProbeReply::MissingGraphics if bytes.starts_with(b"\x1b_G") => {
                Vec::new()
            }
            ProbeReply::RejectedGraphics if bytes.starts_with(b"\x1b_G") => {
                String::from_utf8(bytes)
                    .unwrap()
                    .replace(";OK", ";ENOTSUP: test rejection")
                    .into_bytes()
            }
            _ => bytes,
        },
        |pty, terminal| {
            let text = screen(terminal).replace('\n', "");
            if !checked && text.contains("PROBE_EXIT_") {
                assert!(
                    text.contains(if succeeds {
                        "PROBE_EXIT_0"
                    } else {
                        "PROBE_EXIT_1"
                    }),
                    "{text}"
                );
                assert_eq!(
                    terminal.graphics.images.len(),
                    usize::from(succeeds)
                );
                if !passthrough {
                    assert!(
                        text.contains(
                            "DA2 identity query failed at passthrough depth 1"
                        ) && text
                            .contains("No matching reply within 2 seconds")
                            && text.contains("0 input bytes received"),
                        "{text}"
                    );
                }
                match reply {
                    ProbeReply::Normal => {}
                    ProbeReply::MissingIdentity => {
                        assert!(
                            text.contains(
                                "DA2 identity query failed at passthrough depth 0"
                            ),
                            "{text}"
                        );
                        assert!(
                            text.contains("15 input bytes received"),
                            "{text}"
                        );
                        assert!(!text.contains("UNRELATED_INPUT"), "{text}");
                    }
                    ProbeReply::MissingGraphics
                    | ProbeReply::RejectedGraphics => {
                        assert!(
                            text.contains(
                                "Kitty graphics query failed at passthrough depth 0"
                            ),
                            "{text}"
                        );
                        assert!(text.contains("DA2 reply:"), "{text}");
                        let expected = match reply {
                            ProbeReply::MissingGraphics => {
                                "0 input bytes received"
                            }
                            _ => "Terminal rejected graphics query: ENOTSUP",
                        };
                        assert!(text.contains(expected), "{text}");
                    }
                }
                for layer in &layers {
                    let title = Command::new("tmux")
                        .args([
                            "-L",
                            &layer.socket,
                            "display-message",
                            "-p",
                            "#{pane_title}",
                        ])
                        .output()
                        .unwrap();
                    assert_eq!(
                        String::from_utf8_lossy(&title.stdout).trim(),
                        "PROBE_TITLE"
                    );
                }
                checked = true;
                pty.write(b"\r".to_vec()).unwrap();
            }
        },
    );
    assert!(checked, "probe depth {depth} did not finish");
}
