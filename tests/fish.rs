use std::ffi::OsString;
use std::time::{Duration, Instant};

use cryptty::pty::{Event, Pty};
use cryptty::terminal::Terminal;
use portable_pty::PtySize;

#[test]
#[ignore = "requires fish on PATH"]
fn interactive_fish_handshake_and_command() {
    let command: Vec<OsString> = [
        "fish",
        "--private",
        "--no-config",
        "--init-command",
        "function fish_prompt; printf READY_PROMPT; end",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    let mut pty = Pty::spawn(&command, PtySize::default(), || {}).unwrap();
    let mut terminal = Terminal::new(80, 24);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = String::new();
    let mut sent = false;
    loop {
        match pty
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            Event::Output(bytes) => {
                output.push_str(&String::from_utf8_lossy(&bytes));
                assert!(pty.write(terminal.feed(&bytes)).unwrap());
                if !sent && output.contains("READY_PROMPT") {
                    assert!(
                        pty.write(
                            b"printf '\\nFISH_EXECUTED=%s\\n' cryptty; exit\n"
                                .to_vec()
                        )
                        .unwrap()
                    );
                    sent = true;
                }
            }
            Event::Closed => break,
            Event::Error(e) => panic!("{e}"),
        }
    }
    assert!(sent);
    assert!(output.contains("FISH_EXECUTED=cryptty"), "{output:?}");
}
