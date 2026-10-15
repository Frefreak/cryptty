use std::ffi::OsString;
use std::time::{Duration, Instant};

use cryptty::pty::{Event, Pty};
use portable_pty::PtySize;

#[test]
fn real_shell_receives_input_and_resize() {
    let command: Vec<OsString> = ["/bin/sh", "-c",
        "printf 'READY\\n'; read value; stty size; printf 'VALUE=%s\\n' \"$value\""
    ].into_iter().map(Into::into).collect();
    let initial = PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 800,
        pixel_height: 480,
    };
    let mut pty = Pty::spawn(&command, initial, || {}).unwrap();
    let mut output = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let event = pty
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        match event {
            Event::Output(bytes) => {
                output.push_str(&String::from_utf8_lossy(&bytes));
                if output.contains("READY") {
                    break;
                }
            }
            Event::Error(e) => panic!("{e}"),
            Event::Closed => panic!("shell exited before reading input"),
        }
    }
    pty.resize(PtySize {
        rows: 33,
        cols: 101,
        ..initial
    })
    .unwrap();
    pty.write(b"cryptty\n".to_vec()).unwrap();
    loop {
        match pty
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            Event::Output(bytes) => {
                output.push_str(&String::from_utf8_lossy(&bytes))
            }
            Event::Closed => break,
            Event::Error(e) => panic!("{e}"),
        }
    }
    assert!(output.contains("33 101"), "{output}");
    assert!(output.contains("VALUE=cryptty"), "{output}");
}

#[test]
fn blocked_input_is_retained_and_retried() {
    let command: Vec<OsString> = [
        "/bin/sh", "-c",
        "stty raw -echo; printf READY; sleep 0.2; dd bs=1024 count=640 iflag=fullblock of=/dev/null 2>/dev/null; printf DONE",
    ].into_iter().map(Into::into).collect();
    // macOS dd has no iflag=fullblock; use a byte count there.
    let mut command = command;
    if cfg!(target_os = "macos") {
        command[2] = "stty raw -echo; printf READY; sleep 0.2; dd bs=1 count=655360 of=/dev/null 2>/dev/null; printf DONE".into();
    }
    let (wake_tx, wake_rx) = std::sync::mpsc::channel();
    let mut pty = Pty::spawn(&command, PtySize::default(), move || {
        let _ = wake_tx.send(());
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = String::new();
    while !output.contains("READY") {
        if let Event::Output(bytes) =
            pty.output.recv_timeout(Duration::from_secs(2)).unwrap()
        {
            output.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    for _ in 0..80 {
        assert!(pty.write(vec![b'x'; 8192]).unwrap());
    }
    assert!(!pty.flush_input().unwrap());
    while !output.contains("DONE") {
        assert!(Instant::now() < deadline, "{output}");
        let _ = wake_rx.recv_timeout(Duration::from_millis(50));
        pty.flush_input().unwrap();
        pty.begin_output_batch();
        while let Ok(event) = pty.output.try_recv() {
            match event {
                Event::Output(bytes) => {
                    output.push_str(&String::from_utf8_lossy(&bytes))
                }
                Event::Error(e) => panic!("{e}"),
                Event::Closed => (),
            }
        }
    }
}

#[test]
fn output_keeps_flowing_while_input_waits() {
    let input_reader = if cfg!(target_os = "macos") {
        "dd of=/dev/null bs=1 count=409600 2>/dev/null"
    } else {
        "dd of=/dev/null bs=4096 count=100 iflag=fullblock 2>/dev/null"
    };
    let script = format!(
        "stty raw -echo; printf READY; sleep 0.1; dd if=/dev/zero bs=8192 count=100 2>/dev/null; {input_reader}; printf DONE"
    );
    let command = [OsString::from("/bin/sh"), "-c".into(), script.into()];
    let mut pty = Pty::spawn(&command, PtySize::default(), || {}).unwrap();
    let mut output = Vec::new();
    while !output.ends_with(b"READY") {
        if let Event::Output(bytes) =
            pty.output.recv_timeout(Duration::from_secs(2)).unwrap()
        {
            output.extend(bytes);
        }
    }
    for _ in 0..100 {
        assert!(pty.write(vec![b'x'; 4096]).unwrap());
    }
    assert!(!pty.flush_input().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !output.ends_with(b"DONE") {
        assert!(Instant::now() < deadline);
        pty.flush_input().unwrap();
        match pty.output.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Output(bytes)) => output.extend(bytes),
            Ok(Event::Error(e)) => panic!("{e}"),
            _ => (),
        }
    }
    assert_eq!(output.len(), 5 + 8192 * 100 + 4);
}

#[test]
fn coalesced_output_wakes_resume_after_each_bounded_batch() {
    let command: Vec<OsString> = [
        "/bin/sh",
        "-c",
        "stty raw -echo; head -c 1048576 /dev/zero; printf DONE",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    let (wake_tx, wake_rx) = std::sync::mpsc::channel();
    let pty = Pty::spawn(&command, PtySize::default(), move || {
        let _ = wake_tx.send(());
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = Vec::new();
    let mut scheduled = false;
    'receive: loop {
        if !scheduled {
            wake_rx
                .recv_timeout(
                    deadline.saturating_duration_since(Instant::now()),
                )
                .unwrap();
        }
        assert!(Instant::now() < deadline);
        pty.begin_output_batch();
        scheduled = false;
        for index in 0..32 {
            match pty.output.try_recv() {
                Ok(Event::Output(bytes)) => {
                    output.extend_from_slice(&bytes);
                    scheduled = index == 31;
                }
                Ok(Event::Closed) => break 'receive,
                Ok(Event::Error(error)) => panic!("{error}"),
                Err(_) => break,
            }
        }
    }
    assert_eq!(output.len(), 1048580);
    assert!(output[..1048576].iter().all(|byte| *byte == 0));
    assert_eq!(&output[1048576..], b"DONE");
}

#[test]
fn output_notifies_again_after_an_idle_interval() {
    let directory = tempfile::tempdir().unwrap();
    let gate = directory.path().join("continue");
    let command: Vec<OsString> = vec![
        "/bin/sh".into(),
        "-c".into(),
        "printf FIRST; while ! test -e \"$1\"; do sleep 0.01; done; printf SECOND".into(),
        "test".into(),
        gate.as_os_str().into(),
    ];
    let (wake_tx, wake_rx) = std::sync::mpsc::channel();
    let pty = Pty::spawn(&command, PtySize::default(), move || {
        let _ = wake_tx.send(());
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = Vec::new();
    for expected in [b"FIRST".as_slice(), b"FIRSTSECOND".as_slice()] {
        while output.len() < expected.len() {
            wake_rx
                .recv_timeout(
                    deadline.saturating_duration_since(Instant::now()),
                )
                .unwrap();
            pty.begin_output_batch();
            while let Ok(event) = pty.output.try_recv() {
                match event {
                    Event::Output(bytes) => output.extend_from_slice(&bytes),
                    Event::Error(error) => panic!("{error}"),
                    Event::Closed => (),
                }
            }
        }
        assert_eq!(output, expected);
        std::fs::write(&gate, []).unwrap();
    }
}
