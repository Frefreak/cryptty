use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{
    Child, CommandBuilder, MasterPty, PtySize, native_pty_system,
};

pub enum Event {
    Output(Vec<u8>),
    Closed,
    Error(String),
}

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    input: SyncSender<Vec<u8>>,
    pub output: Receiver<Event>,
    output_pending: Arc<AtomicBool>,
    pending: VecDeque<Vec<u8>>,
    pending_bytes: usize,
}

impl Pty {
    pub fn spawn(
        command: &[OsString],
        size: PtySize,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self> {
        let pair = native_pty_system().openpty(size)?;
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let mut cmd = if let Some(program) = command.first() {
            let mut cmd = CommandBuilder::new(program);
            cmd.args(&command[1..]);
            cmd
        } else {
            let shell = std::env::var_os("SHELL")
                .context("SHELL is unset; pass a shell after --")?;
            CommandBuilder::new(shell)
        };
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "cryptty");
        cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        let (output_tx, output) = mpsc::sync_channel(64);
        let (input, input_rx) = mpsc::sync_channel::<Vec<u8>>(64);
        let wake = Arc::new(wake);
        let reader_wake = wake.clone();
        let output_pending = Arc::new(AtomicBool::new(false));
        let reader_pending = output_pending.clone();
        let errors = output_tx.clone();
        thread::spawn(move || {
            let mut bytes = [0; 8192];
            loop {
                let event = match reader.read(&mut bytes) {
                    Ok(0) => Event::Closed,
                    Ok(n) => Event::Output(bytes[..n].to_vec()),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                        continue;
                    }
                    // Linux PTYs report EIO when the slave closes.
                    Err(e) if e.raw_os_error() == Some(5) => Event::Closed,
                    Err(e) => Event::Error(e.to_string()),
                };
                let finished = !matches!(event, Event::Output(_));
                if output_tx.send(event).is_err() {
                    break;
                }
                if !reader_pending.swap(true, Ordering::AcqRel) {
                    reader_wake();
                }
                if finished {
                    break;
                }
            }
        });
        thread::spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if let Err(e) = writer.write_all(&bytes) {
                    let _ = errors.send(Event::Error(e.to_string()));
                    wake();
                    break;
                }
                wake();
            }
        });
        Ok(Self {
            master: pair.master,
            child,
            input,
            output,
            output_pending,
            pending: VecDeque::new(),
            pending_bytes: 0,
        })
    }

    // The consumer must schedule another batch if it stops before draining.
    pub fn begin_output_batch(&self) {
        self.output_pending.store(false, Ordering::Release);
    }

    // False means the bounded backlog rejected this input without closing.
    pub fn write(&mut self, bytes: Vec<u8>) -> Result<bool> {
        if self.pending_bytes + bytes.len() > 1024 * 1024 {
            return Ok(false);
        }
        if !bytes.is_empty() {
            self.pending_bytes += bytes.len();
            self.pending.push_back(bytes);
        }
        self.flush_input()?;
        Ok(true)
    }

    pub fn flush_input(&mut self) -> Result<bool> {
        while let Some(bytes) = self.pending.pop_front() {
            let len = bytes.len();
            match self.input.try_send(bytes) {
                Ok(()) => self.pending_bytes -= len,
                Err(TrySendError::Full(bytes)) => {
                    self.pending.push_front(bytes);
                    return Ok(false);
                }
                Err(TrySendError::Disconnected(_)) => {
                    anyhow::bail!("PTY writer disconnected");
                }
            }
        }
        Ok(true)
    }

    pub fn tty_name(&self) -> Option<std::path::PathBuf> {
        self.master.tty_name()
    }

    pub fn resize(&self, size: PtySize) -> Result<()> {
        self.master.resize(size)
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Some(group) = self.master.process_group_leader() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(group),
                nix::sys::signal::Signal::SIGHUP,
            );
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
