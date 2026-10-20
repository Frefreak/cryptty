use super::*;

fn p(row: usize, col: usize) -> Position {
    Position { row, col }
}

#[test]
fn captures_distinguish_wrapping_from_explicit_newlines() {
    let physical = "https://a/\n1234567890\nend       \n          \n";
    let joined = "https://a/1234567890end\n\n";
    let terminal = pane_terminal(10, 4, physical, joined).unwrap();
    assert_eq!(
        terminal.link_at(p(2, 1)).unwrap().uri,
        "https://a/1234567890end"
    );
    let terminal = pane_terminal(10, 4, physical, physical).unwrap();
    assert_eq!(terminal.link_at(p(0, 2)).unwrap().uri, "https://a/");
    assert!(terminal.link_at(p(1, 1)).is_none());
}

#[test]
fn inconsistent_captures_are_rejected() {
    assert!(
        pane_terminal(10, 2, "https://a/\nchanged   \n", "https://a/old\n")
            .is_none()
    );
}

#[test]
fn unicode_wide_padding_and_combining_marks_keep_coordinates() {
    let terminal =
        pane_terminal(4, 3, "abc \n中e\u{301} \n尾  \n", "abc中e\u{301}\n尾\n")
            .unwrap();
    assert_eq!(terminal.rows[0].wrapped, Some(3));
    assert_eq!(
        terminal.selected_text(TextRange::between(p(0, 0), p(1, 3))),
        "abc中e\u{301}"
    );
}

#[test]
fn pane_links_and_highlights_stay_inside_their_rectangle() {
    let physical = "https://a/\n1234567890\nend       \n          \n";
    let terminal =
        pane_terminal(10, 4, physical, "https://a/1234567890end\n\n").unwrap();
    let mut display = Terminal::new(22, 6);
    for (row, line) in physical.lines().enumerate() {
        display.feed(format!("\x1b[{};12H{line}", row + 2).as_bytes());
    }
    let snapshot = Snapshot {
        complete: true,
        size: (22, 6),
        panes: vec![CapturedPane::Application(PaneText {
            origin: p(1, 11),
            terminal,
        })],
    };
    let Lookup::Ready(Some(link)) = snapshot.lookup(&display, p(3, 12)) else {
        panic!("wrapped suffix must resolve");
    };
    assert_eq!(link.uri, "https://a/1234567890end");
    assert_eq!(
        link.ranges,
        vec![
            TextRange::between(p(1, 11), p(1, 21)),
            TextRange::between(p(2, 11), p(2, 21)),
            TextRange::between(p(3, 11), p(3, 14)),
        ]
    );
    assert!(matches!(
        snapshot.lookup(&display, p(3, 5)),
        Lookup::Unavailable
    ));
    display.feed(b"\x1b[2;12HX");
    assert!(matches!(
        snapshot.lookup(&display, p(3, 12)),
        Lookup::Unavailable
    ));
}

#[test]
fn client_selection_uses_exact_tty_and_checks_viewport() {
    let text = concat!(
        "/dev/pts/9\t$1\t@2\t80\t24\t80\t23\ton\tbottom\toff\t0\n",
        "/dev/pts/1\t$3\t@4\t80\t24\t80\t22\t2\ttop\toff\t1\n",
    );
    let client = Client::find(text, Path::new("/dev/pts/1")).unwrap();
    assert_eq!(client.window, "@4");
    assert!(client.status_top);
    assert!(client.zoomed);
    assert!(client.fits((80, 24)));
    assert!(!client.fits((80, 25)));
    assert!(Client::find(text, Path::new("/dev/pts/10")).is_none());
}

#[test]
#[ignore = "requires tmux and a local PTY"]
fn real_tmux_split_recovers_url_and_tracks_layout_changes() {
    use crate::pty::{Event, Pty};
    use portable_pty::PtySize;
    use std::ffi::OsString;
    use std::process::Command;
    use std::time::{Duration, Instant};

    struct ServerGuard(std::path::PathBuf);
    impl Drop for ServerGuard {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .arg("-S")
                .arg(&self.0)
                .arg("kill-server")
                .env_remove("TMUX")
                .output();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("tmux.sock");
    let _guard = ServerGuard(socket.clone());
    let run = |args: &[&str]| {
        let output = Command::new("tmux")
            .arg("-S")
            .arg(&socket)
            .args(args)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&[
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-s",
        "test",
        "-x",
        "81",
        "-y",
        "12",
        "sleep 30",
    ]);
    run(&["set-option", "-g", "status-position", "top"]);
    let uri = concat!(
        "https://carso.nz/1111111111111111/",
        "222222222222222222/3333333333333333333333/",
        "444444444444444444444444444/5555555555555"
    );
    let command = format!("printf '%s' {}; sleep 30", quote(uri));
    run(&["split-window", "-h", "-l", "40", "-t", "test", &command]);
    let command: Vec<OsString> = vec![
        "tmux".into(),
        "-S".into(),
        socket.clone().into(),
        "attach-session".into(),
        "-t".into(),
        "test".into(),
    ];
    let mut pty = Pty::spawn(
        &command,
        PtySize {
            cols: 81,
            rows: 12,
            ..PtySize::default()
        },
        || {},
    )
    .unwrap();
    let tty = pty.tty_name().unwrap();
    let mut terminal = Terminal::new(81, 12);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let event = pty
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        let Event::Output(bytes) = event else {
            panic!("tmux exited")
        };
        let replies = terminal.feed(&bytes);
        pty.write(replies).unwrap();
        if matches!(&terminal.rows[3][41].content,
                    Content::Text { text } if text.chars().next() == Some('4'))
        {
            break;
        }
    }
    assert_eq!(terminal.rows[1].wrapped, None);
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tmux::integration::tests::missing_binary_with_live_tmux_shows_notice",
            "--ignored",
            "--nocapture",
        ])
        .env("PATH", "")
        .env("CRYPTTY_TEST_TMUX_TTY", &tty)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert_ne!(terminal.link_at(p(1, 45)).unwrap().uri, uri);
    let mut connection = client::Server {
        socket: Some(socket.clone()),
    }
    .connect(&tty, &|| true)
    .unwrap();
    let snapshot =
        Snapshot::capture(&mut connection, &tty, (81, 12), &|| true).unwrap();
    for point in [p(1, 45), p(2, 42), p(3, 42)] {
        let Lookup::Ready(Some(link)) = snapshot.lookup(&terminal, point)
        else {
            panic!("tmux link at {point:?} missing");
        };
        assert_eq!(link.uri, uri);
        assert!(
            link.ranges
                .iter()
                .all(|range| range.start.col >= 41 && range.end.col <= 81)
        );
    }
    let before = Instant::now();
    for _ in 0..50 {
        Snapshot::capture(&mut connection, &tty, (81, 12), &|| true).unwrap();
    }
    eprintln!("tmux snapshot average: {:?}", before.elapsed() / 50);
    run(&["resize-pane", "-Z", "-t", "test:0.1"]);
    let zoomed =
        Snapshot::capture(&mut connection, &tty, (81, 12), &|| true).unwrap();
    assert_eq!(zoomed.panes.len(), 1);
    assert_eq!(zoomed.panes[0].pane().origin, p(1, 0));
    assert_eq!(zoomed.panes[0].pane().terminal.cols(), 81);
    run(&["copy-mode", "-t", "test:0.1"]);
    let copy_mode =
        Snapshot::capture(&mut connection, &tty, (81, 12), &|| true).unwrap();
    assert!(copy_mode.complete);
    assert_eq!(copy_mode.panes.len(), 1);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !terminal.rows.iter().any(|row| row.iter().any(|cell| {
        matches!(&cell.content, Content::Text { text } if text.chars().next() == Some('['))
    })) {
        let event = pty
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        let Event::Output(bytes) = event else {
            panic!("tmux exited")
        };
        let replies = terminal.feed(&bytes);
        pty.write(replies).unwrap();
    }
    assert!(matches!(copy_mode.text(&terminal), TextSnapshot::Ready(_)));
}

#[test]
fn vim_snapshot_rejects_partial_pane_coverage() {
    let display = Terminal::new(10, 3);
    let snapshot = Snapshot {
        complete: false,
        size: (10, 3),
        panes: vec![CapturedPane::Application(PaneText {
            origin: p(0, 0),
            terminal: display.visible_snapshot(),
        })],
    };
    assert!(matches!(snapshot.text(&display), TextSnapshot::Changed));
}

#[test]
fn copy_mode_uses_visible_headers_and_preserves_matching_wraps() {
    let physical =
        "headerxxxx\nhttps://a/\n1234567890\nend       \npromptxxxx\n";
    let backing = pane_terminal(
        10,
        5,
        physical,
        "headerxxxx\nhttps://a/1234567890end\npromptxxxx\n",
    )
    .unwrap();
    let mut display = Terminal::new(22, 7);
    for (row, line) in physical.lines().enumerate() {
        display.feed(format!("\x1b[{};12H{line}", row + 2).as_bytes());
    }
    display.feed(b"\x1b[2;12H[copy top]\x1b[6;12H/search   ");
    let captured = CapturedPane::CopyMode(PaneText {
        origin: p(1, 11),
        terminal: backing,
    });
    let visible = captured.visible(&display).unwrap();
    assert_eq!(
        visible.terminal.link_at(p(2, 2)).unwrap().uri,
        "https://a/1234567890end"
    );
    assert_eq!(
        visible
            .terminal
            .selected_text(TextRange::between(p(0, 0), p(0, 10))),
        "[copy top]"
    );
    assert_eq!(
        visible
            .terminal
            .selected_text(TextRange::between(p(4, 0), p(4, 10))),
        "/search"
    );
    display.feed(b"\x1b[4;12H/changed  ");
    let visible = captured.visible(&display).unwrap();
    assert_eq!(visible.terminal.rows[1].wrapped, None);
    assert_eq!(visible.terminal.rows[2].wrapped, None);
    assert_eq!(visible.terminal.link_at(p(1, 2)).unwrap().uri, "https://a/");
}

#[test]
fn application_capture_uses_visible_styles_for_marker_targets() {
    let plain = "even-cloud-core::connection::Connection TerminalEndpoint";
    let mut capture = Terminal::new(30, 3);
    capture.feed(plain.as_bytes());
    let mut display = Terminal::new(34, 5);
    for (y, row) in capture.rows.iter().enumerate() {
        let text: String = row
            .iter()
            .filter_map(|cell| match &cell.content {
                Content::Text { text } => Some(text.chars()),
                _ => None,
            })
            .flatten()
            .collect();
        display
            .feed(format!("\x1b[{};3H\x1b[33m{text}\x1b[0m", y + 2).as_bytes());
    }
    let captured = CapturedPane::Application(PaneText {
        origin: p(1, 2),
        terminal: capture,
    });
    let pane = captured.visible(&display).unwrap();
    assert_eq!(
        pane.terminal.rows[0][0].style().fg,
        display.rows[1][2].style().fg
    );
    assert_eq!(pane.terminal.rows[0].wrapped, Some(30));
    let session = crate::marker::Session::new(
        display,
        vec![pane],
        &crate::marker::Options::default(),
    );
    let targets = session.visible_targets();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].text, plain);
    assert_eq!(targets[0].segments[0].start, p(1, 2));
    assert_eq!(targets[0].segments.len(), 2);
}

#[test]
fn captured_tabs_preserve_pane_text_and_reject_changes() {
    let capture = "origin\thttps://example.com\n中e\u{301}\tpath\n";
    let terminal = pane_terminal(40, 2, capture, capture).unwrap();
    assert_eq!(
        terminal.link_at(p(0, 8)).unwrap().uri,
        "https://example.com"
    );
    assert_eq!(
        terminal.selected_text(TextRange::between(p(1, 0), p(1, 12))),
        "中e\u{301}     path"
    );
    assert!(
        pane_terminal(
            40,
            2,
            capture,
            "origin\thttps://example.com\n中e\u{301}\tother\n"
        )
        .is_none()
    );
}
