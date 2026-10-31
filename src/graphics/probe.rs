use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use nix::libc;

pub(super) fn write_sequence(
    out: &mut impl Write,
    bytes: &[u8],
    depth: usize,
) -> io::Result<()> {
    let mut bytes = bytes.to_vec();
    for _ in 0..depth {
        let mut wrapped = b"\x1bPtmux;".to_vec();
        for byte in bytes {
            wrapped.push(byte);
            if byte == 0x1b {
                wrapped.push(byte);
            }
        }
        wrapped.extend_from_slice(b"\x1b\\");
        bytes = wrapped;
    }
    out.write_all(&bytes)
}

pub(super) fn detect(out: &mut impl Write) -> Result<usize> {
    let mut tty = ProbeTty::open().context("Prepare graphics support query")?;
    let _focus = PaneFocus::acquire()?;
    for depth in 0..=3 {
        // A bare graphics APC changes tmux's pane title. Ask its identity first.
        write_sequence(out, b"\x1b[>c", depth)?;
        out.flush()?;
        let identity = reply(&mut tty.file, b"\x1b[>", b"c")?;
        if identity.starts_with(b"84;") {
            continue;
        }
        let seed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let id = (seed as u32 & 0xffffff).max(1);
        let query = format!("\x1b_Ga=q,i={id},f=24,s=1,v=1;AAAA\x1b\\");
        write_sequence(out, query.as_bytes(), depth)?;
        out.flush()?;
        let prefix = format!("\x1b_Gi={id};");
        let response = reply(&mut tty.file, prefix.as_bytes(), b"\x1b\\")?;
        ensure!(
            response == b"OK",
            "Graphics query rejected: {}",
            String::from_utf8_lossy(&response)
        );
        return Ok(depth);
    }
    bail!("More than three tmux layers; graphics upload cancelled")
}

fn reply(file: &mut File, prefix: &[u8], end: &[u8]) -> Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let mut poll = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready =
            unsafe { libc::poll(&mut poll, 1, remaining.as_millis() as i32) };
        if ready < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if ready == 0 {
            break;
        }
        let mut chunk = [0; 256];
        let count = file.read(&mut chunk)?;
        ensure!(count > 0, "Terminal closed during graphics support query");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(start) =
            bytes.windows(prefix.len()).position(|w| w == prefix)
        {
            let payload = &bytes[start + prefix.len()..];
            if let Some(stop) =
                payload.windows(end.len()).position(|w| w == end)
            {
                return Ok(payload[..stop].to_vec());
            }
        }
        ensure!(bytes.len() <= 4096, "Excessive input during graphics query");
    }
    bail!(
        "No graphics support response within 2 seconds. \
         In each tmux pane carrying this session, enable \
         'tmux set -p allow-passthrough on', then retry. \
         Over SSH, keep the local tmux pane focused as well."
    )
}

struct ProbeTty {
    file: File,
    saved: libc::termios,
    signals: libc::sigset_t,
}

impl ProbeTty {
    fn open() -> io::Result<Self> {
        let file =
            OpenOptions::new().read(true).write(true).open("/dev/tty")?;
        unsafe {
            let mut saved = std::mem::zeroed();
            if libc::tcgetattr(file.as_raw_fd(), &mut saved) != 0 {
                return Err(io::Error::last_os_error());
            }
            // Defer termination until terminal mode and pane focus are restored.
            let mut blocked = std::mem::zeroed();
            let mut signals = std::mem::zeroed();
            libc::sigemptyset(&mut blocked);
            for signal in [
                libc::SIGINT,
                libc::SIGTERM,
                libc::SIGHUP,
                libc::SIGQUIT,
                libc::SIGTSTP,
            ] {
                libc::sigaddset(&mut blocked, signal);
            }
            let error =
                libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut signals);
            if error != 0 {
                return Err(io::Error::from_raw_os_error(error));
            }
            let guard = Self {
                file,
                saved,
                signals,
            };
            let mut mode = saved;
            mode.c_lflag &= !(libc::ICANON | libc::ECHO);
            mode.c_cc[libc::VMIN] = 1;
            mode.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(guard.file.as_raw_fd(), libc::TCSANOW, &mode)
                != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(guard)
        }
    }
}

impl Drop for ProbeTty {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.file.as_raw_fd(), libc::TCSANOW, &self.saved);
            libc::pthread_sigmask(
                libc::SIG_SETMASK,
                &self.signals,
                std::ptr::null_mut(),
            );
        }
    }
}

struct PaneFocus {
    source: String,
    previous: String,
    window: String,
}

impl PaneFocus {
    fn acquire() -> Result<Option<Self>> {
        if !std::env::var_os("TMUX").is_some_and(|v| !v.is_empty()) {
            return Ok(None);
        }
        let source = std::env::var("TMUX_PANE")?;
        let window =
            tmux(&["display-message", "-p", "-t", &source, "#{window_id}"])?;
        let previous =
            tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"])?;
        if source == previous {
            return Ok(None);
        }
        tmux(&["select-pane", "-t", &source])?;
        Ok(Some(Self {
            source,
            previous,
            window,
        }))
    }
}

impl Drop for PaneFocus {
    fn drop(&mut self) {
        let active =
            tmux(&["display-message", "-p", "-t", &self.window, "#{pane_id}"]);
        if active.as_deref().ok() == Some(self.source.as_str()) {
            let _ = tmux(&["select-pane", "-t", &self.previous]);
        }
    }
}

fn tmux(args: &[&str]) -> Result<String> {
    let result = Command::new("tmux").args(args).output()?;
    ensure!(
        result.status.success(),
        "tmux: {}",
        String::from_utf8_lossy(&result.stderr).trim()
    );
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}
