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
    let _focus = PaneFocus::acquire().context("Focus graphics query pane")?;
    let clients = reply_clients()?;
    for depth in 0..=3 {
        let expected = if depth == 0 { 1 } else { clients };
        // A bare graphics APC changes tmux's pane title. Ask its identity first.
        let identities = (|| -> Result<Vec<Vec<u8>>> {
            write_sequence(out, b"\x1b[>c", depth)?;
            out.flush()?;
            reply(&mut tty.file, b"\x1b[>", b"c", expected)
        })()
        .with_context(|| {
            format!("DA2 identity query failed at passthrough depth {depth}")
        })?;
        if identities.iter().all(|reply| reply.starts_with(b"84;")) {
            continue;
        }
        ensure!(
            identities.iter().all(|reply| !reply.starts_with(b"84;")),
            "Attached terminals have different tmux nesting depths"
        );
        let seed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let id = (seed as u32 & 0xffffff).max(1);
        let query = format!("\x1b_Ga=q,i={id},f=24,s=1,v=1;AAAA\x1b\\");
        (|| -> Result<()> {
            write_sequence(out, query.as_bytes(), depth)?;
            out.flush()?;
            let prefix = format!("\x1b_Gi={id};");
            let responses =
                reply(&mut tty.file, prefix.as_bytes(), b"\x1b\\", expected)?;
            for response in responses {
                ensure!(
                    response == b"OK",
                    "Terminal rejected graphics query: {}",
                    response.escape_ascii()
                );
            }
            Ok(())
        })()
        .with_context(|| {
            format!(
                "Kitty graphics query failed at passthrough depth {depth} \
                 (DA2 reply: {})",
                identities[0].escape_ascii()
            )
        })?;
        return Ok(depth);
    }
    bail!("More than three tmux layers; graphics upload cancelled")
}

fn reply(
    file: &mut File,
    prefix: &[u8],
    end: &[u8],
    expected: usize,
) -> Result<Vec<Vec<u8>>> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut responses = Vec::new();
    let mut consumed = 0;
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
        ensure!(count > 0, "Terminal closed while waiting for query reply");
        bytes.extend_from_slice(&chunk[..count]);
        while let Some(start) = bytes[consumed..]
            .windows(prefix.len())
            .position(|w| w == prefix)
        {
            let start = consumed + start + prefix.len();
            let payload = &bytes[start..];
            if let Some(stop) =
                payload.windows(end.len()).position(|w| w == end)
            {
                responses.push(payload[..stop].to_vec());
                consumed = start + stop + end.len();
                if responses.len() == expected {
                    return Ok(responses);
                }
            } else {
                break;
            }
        }
        ensure!(
            bytes.len() <= 4096,
            "Excessive input while waiting for query reply"
        );
    }
    bail!(
        "Missing terminal replies within 2 seconds ({}/{} replies, \
         {} input bytes received). \
         Check terminal DA2/Kitty query support and reply routing. \
         For tmux, check 'tmux set -p allow-passthrough on' in each pane \
         carrying this session. Over SSH, keep the local SSH pane focused. \
         A timeout alone does not identify the cause.",
        responses.len(),
        expected,
        bytes.len()
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

// Passthrough queries reach every writable client displaying this window.
fn reply_clients() -> Result<usize> {
    if !std::env::var_os("TMUX").is_some_and(|v| !v.is_empty()) {
        return Ok(1);
    }
    let pane = std::env::var("TMUX_PANE")?;
    let window = tmux(&["display-message", "-p", "-t", &pane, "#{window_id}"])?;
    let clients =
        tmux(&["list-clients", "-F", "#{window_id} #{client_flags}"])?;
    Ok(visible_clients(&clients, &window).max(1))
}

fn visible_clients(clients: &str, window: &str) -> usize {
    clients
        .lines()
        .filter(|line| {
            let Some((current, flags)) = line.split_once(' ') else {
                return false;
            };
            current == window
                && !flags.split(',').any(|flag| {
                    matches!(flag, "read-only" | "control-mode" | "suspended")
                })
        })
        .count()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    const PREFIX: &[u8] = b"\x1b_Gi=123;";
    const END: &[u8] = b"\x1b\\";
    const OK: &[u8] = b"\x1b_Gi=123;OK\x1b\\";

    fn pair() -> (File, File) {
        let (reader, writer) = nix::unistd::pipe().unwrap();
        (File::from(reader), File::from(writer))
    }

    #[test]
    fn delayed_duplicate_is_consumed_before_returning() {
        let (mut reader, mut writer) = pair();
        let sender = thread::spawn(move || {
            writer.write_all(OK).unwrap();
            thread::sleep(Duration::from_millis(150));
            writer.write_all(&OK[..8]).unwrap();
            thread::sleep(Duration::from_millis(20));
            writer.write_all(&OK[8..]).unwrap();
        });
        let responses = reply(&mut reader, PREFIX, END, 2).unwrap();
        sender.join().unwrap();
        assert_eq!(responses, [b"OK".to_vec(), b"OK".to_vec()]);
        let mut remaining = Vec::new();
        reader.read_to_end(&mut remaining).unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn batched_replies_are_all_parsed() {
        let (mut reader, mut writer) = pair();
        writer.write_all(&[OK, OK].concat()).unwrap();
        assert_eq!(reply(&mut reader, PREFIX, END, 2).unwrap().len(), 2);
    }

    #[test]
    fn single_client_needs_one_reply() {
        let (mut reader, mut writer) = pair();
        writer.write_all(OK).unwrap();
        assert_eq!(reply(&mut reader, PREFIX, END, 1).unwrap(), [b"OK"]);
    }

    #[test]
    fn missing_reply_reports_counts_without_input_contents() {
        let (mut reader, mut writer) = pair();
        writer.write_all(OK).unwrap();
        writer.write_all(b"private input").unwrap();
        let error = reply(&mut reader, PREFIX, END, 2).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("1/2 replies"), "{message}");
        assert!(!message.contains("private input"));
    }

    #[test]
    fn only_clients_that_can_reply_in_this_window_are_counted() {
        let clients = "@1 attached,UTF-8\n\
                       @1 attached,focused,UTF-8\n\
                       @2 attached,UTF-8\n\
                       @1 attached,read-only,UTF-8\n\
                       @1 attached,control-mode\n\
                       @1 attached,suspended,UTF-8";
        assert_eq!(visible_clients(clients, "@1"), 2);
    }
}
