use super::Terminal;
use crate::config::OutputLimits;
use anyhow::{Result, ensure};

pub fn check_size(text: &str, limits: OutputLimits) -> Result<()> {
    ensure!(
        text.len() <= limits.max_bytes,
        "Command output exceeds {} bytes",
        limits.max_bytes
    );
    ensure!(
        text.lines().count() <= limits.max_lines,
        "Command output exceeds {} lines",
        limits.max_lines
    );
    ensure!(!text.is_empty(), "Command produced no output");
    Ok(())
}

#[derive(Default)]
pub(super) struct Capture {
    active: Option<Result<String>>,
    completed: Option<Result<String>>,
    newlines: usize,
    pub(super) carriage_return: bool,
}

impl Capture {
    pub(super) fn append_ascii(&mut self, bytes: &[u8], limits: OutputLimits) {
        if matches!(self.active, Some(Ok(_))) {
            self.append(
                std::str::from_utf8(bytes).expect("printable ASCII"),
                limits,
            );
        }
    }

    pub(super) fn append(&mut self, text: &str, limits: OutputLimits) {
        let Some(Ok(body)) = self.active.as_mut() else {
            return;
        };
        let newline = text == "\n";
        if self.carriage_return
            && !newline
            && !body.is_empty()
            && !body.ends_with('\n')
        {
            self.unsupported();
        } else if text.len() > limits.max_bytes.saturating_sub(body.len()) {
            self.active = Some(Err(anyhow::anyhow!(
                "Command output exceeds {} bytes",
                limits.max_bytes
            )));
        } else if self.newlines >= limits.max_lines {
            self.active = Some(Err(anyhow::anyhow!(
                "Command output exceeds {} lines",
                limits.max_lines
            )));
        } else {
            body.push_str(text);
            self.newlines += usize::from(newline);
        }
        self.carriage_return = false;
    }

    pub(super) fn unsupported(&mut self) {
        if matches!(self.active, Some(Ok(_))) {
            self.active = Some(Err(anyhow::anyhow!(
                "Command redrew the screen; complete output unavailable"
            )));
        }
    }
}

impl Terminal {
    pub(super) fn command_marker(&mut self, marker: &[u8]) {
        if self.alternate {
            return;
        }
        let capture = &mut self.command_output;
        match marker {
            b"C" => {
                capture.active = Some(Ok(String::new()));
                capture.newlines = 0;
                capture.carriage_return = false;
            }
            b"D" | b"A" if capture.active.is_some() => {
                capture.completed = capture.active.take();
            }
            _ => (),
        }
    }

    pub fn last_command_output(&self) -> Result<&str> {
        let output =
            self.command_output.completed.as_ref().ok_or_else(|| {
                anyhow::anyhow!("No completed command output")
            })?;
        let text = output.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        check_size(text, self.output_limits)?;
        Ok(text)
    }
}
