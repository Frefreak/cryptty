use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const TIMEOUT: Duration = Duration::from_millis(500);
const MAX_REPLY: u64 = 4 * 1024 * 1024;

#[derive(Clone, Default)]
pub(super) struct Server {
    pub socket: Option<PathBuf>,
}

impl Server {
    fn command(&self) -> Command {
        let mut command = Command::new("tmux");
        if let Some(socket) = &self.socket {
            command.arg("-S").arg(socket);
        } else {
            command.args(["-L", "default"]);
        }
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }

    pub fn connect(
        &self,
        tty: &Path,
        current: &impl Fn() -> bool,
    ) -> Result<Control> {
        anyhow::ensure!(current(), "tmux request canceled");
        let mut child = self
            .command()
            .args(["list-clients", "-F", super::CLIENT_FORMAT])
            .spawn()?;
        let stdout = child.stdout.take().context("missing tmux stdout")?;
        let (send, receive) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut text = String::new();
            let result = stdout.take(MAX_REPLY).read_to_string(&mut text);
            let _ = send.send(result.map(|_| text));
        });
        let result = receive.recv_timeout(TIMEOUT);
        let _ = child.kill();
        let _ = child.wait();
        let text = result??;
        let target = super::Client::find(&text, tty)
            .context("no tmux client on this terminal")?;
        anyhow::ensure!(current(), "tmux request canceled");
        let child = self
            .command()
            .args([
                "-C",
                "attach-session",
                "-f",
                "no-output,ignore-size",
                "-t",
                &target.session,
            ])
            .stdin(Stdio::piped())
            .spawn()?;
        Control::new(child)
    }
}

pub(super) struct Control {
    child: Child,
    input: ChildStdin,
    replies: Receiver<Result<String>>,
}

impl Control {
    fn new(mut child: Child) -> Result<Self> {
        let input = child.stdin.take().context("missing tmux stdin")?;
        let output = child.stdout.take().context("missing tmux stdout")?;
        let (send, replies) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(output);
            let mut block: Option<(String, String)> = None;
            loop {
                let mut line = String::new();
                match reader.by_ref().take(MAX_REPLY).read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() as u64 >= MAX_REPLY => break,
                    Ok(_) => (),
                }
                let line = line.strip_suffix('\n').unwrap_or(&line);
                if let Some((header, text)) = &mut block {
                    if line == format!("%end {header}") {
                        let (_, text) = block.take().unwrap();
                        if send.send(Ok(text)).is_err() {
                            break;
                        }
                    } else if line == format!("%error {header}") {
                        if send
                            .send(Err(anyhow::anyhow!("tmux command failed")))
                            .is_err()
                        {
                            break;
                        }
                        block = None;
                    } else {
                        text.push_str(line);
                        text.push('\n');
                        if text.len() as u64 > MAX_REPLY {
                            break;
                        }
                    }
                } else if let Some(header) = line.strip_prefix("%begin ") {
                    block = Some((header.to_owned(), String::new()));
                } else if line.starts_with("%exit") {
                    break;
                }
            }
        });
        let connection = Self {
            child,
            input,
            replies,
        };
        // Consume attach-session's response before sending any queries.
        connection.replies.recv_timeout(TIMEOUT)??;
        Ok(connection)
    }

    pub fn query(&mut self, commands: &[String]) -> Result<Vec<String>> {
        if commands.is_empty() {
            return Ok(Vec::new());
        }
        if self.child.try_wait()?.is_some() {
            bail!("tmux control client exited");
        }
        writeln!(self.input, "{}", commands.join(" ; "))?;
        self.input.flush()?;
        let deadline = Instant::now() + TIMEOUT;
        commands
            .iter()
            .map(|_| {
                self.replies.recv_timeout(
                    deadline.saturating_duration_since(Instant::now()),
                )?
            })
            .collect()
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
