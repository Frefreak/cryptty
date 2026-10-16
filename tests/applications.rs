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
                let replies = terminal.feed(&bytes);
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
