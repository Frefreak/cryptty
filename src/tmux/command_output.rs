use std::path::Path;
use unicode_width::UnicodeWidthStr;

use anyhow::{Context, Result, bail, ensure};

use super::client::{Server, output, tmux_running};
use crate::config::OutputLimits;
use crate::terminal::command_output::check_size;

pub fn last_command_output(
    tty: &Path,
    limits: OutputLimits,
) -> Result<Option<String>> {
    if !tmux_running(tty) {
        return Ok(None);
    }
    let server = Server::default();
    from_client(&server, tty, limits).map(Some)
}

fn from_client(
    server: &Server,
    tty: &Path,
    limits: OutputLimits,
) -> Result<String> {
    let clients = output(server.command().args([
        "list-clients",
        "-F",
        "#{client_tty}\t#{pane_id}",
    ]))?;
    let pane = clients
        .lines()
        .find_map(|line| {
            let (client, pane) = line.split_once('\t')?;
            (Path::new(client) == tty).then(|| super::identifier(pane, '%'))?
        })
        .context("Cannot identify the active tmux pane")?;
    capture(server, &pane, limits)
}

fn capture(
    server: &Server,
    pane: &str,
    limits: OutputLimits,
) -> Result<String> {
    let state = || {
        let format = concat!(
            "#{pane_current_command} #{pane_width} #{pane_height} ",
            "#{history_size} #{pane_output_generation}",
        );
        output(server.command().args([
            "display-message",
            "-p",
            "-t",
            pane,
            format,
        ]))
    };
    let before = state()?;
    let fields: Vec<_> = before.split_whitespace().collect();
    let ["fish", width, height, backlog, _generation] = fields.as_slice()
    else {
        bail!("Copy command output requires a fish prompt in tmux 3.8");
    };
    let width = width.parse::<usize>()?;
    let height = height.parse::<usize>()?;
    let backlog = backlog.parse::<usize>()?;
    let total = height + backlog;
    let mut history = 128usize.min(backlog);
    let text = loop {
        let rows = output(server.command().args([
            "capture-pane",
            "-pFTN",
            "-S",
            &format!("-{history}"),
            "-t",
            pane,
        ]))?;
        let lines: Vec<_> = rows.split_terminator('\n').collect();
        ensure!(
            lines.len() == height + history,
            "tmux pane changed during capture"
        );
        if let Some((start, end)) = output_rows(&lines)? {
            let mut text = String::new();
            for row in &lines[start..end - 1] {
                let (flags, content) =
                    row.split_once(' ').context("Missing tmux row flags")?;
                text.push_str(content);
                if !flags.contains('W') {
                    text.push('\n');
                }
                ensure!(
                    text.len() <= limits.max_bytes,
                    "Command output exceeds {} bytes",
                    limits.max_bytes
                );
            }
            let (_, final_line) = lines[end - 1]
                .split_once(' ')
                .context("Missing tmux row flags")?;
            let row = backlog - history + end - 1;
            let (suffix, newline) =
                fish_last_line(server, pane, width, total, row, final_line)?;
            text.push_str(suffix);
            if newline {
                text.push('\n');
            }
            check_size(&text, limits)?;
            break text;
        }
        ensure!(
            history < backlog,
            "No complete command output markers in tmux history"
        );
        history = history.saturating_mul(2).min(backlog);
    };
    ensure!(state()? == before, "tmux pane changed during capture");
    Ok(text)
}

fn output_rows(lines: &[&str]) -> Result<Option<(usize, usize)>> {
    let mut end = None;
    for (i, line) in lines.iter().enumerate().rev() {
        let (flags, _) =
            line.split_once(' ').context("Missing tmux row flags")?;
        if end.is_none() {
            if flags.contains('P') {
                ensure!(
                    !flags.contains('O'),
                    "Command produced no separate output"
                );
                end = Some(i);
            }
        } else if flags.contains('P') {
            bail!("No output marker for the last command");
        } else if flags.contains('O') {
            return Ok(end.map(|end| (i, end)));
        }
    }
    Ok(None)
}

fn fish_last_line<'a>(
    server: &Server,
    pane: &str,
    width: usize,
    rows: usize,
    row: usize,
    line: &'a str,
) -> Result<(&'a str, bool)> {
    // Fish fills the last row after unterminated output. Its recorded end
    // column distinguishes the indicator and padding from program text.
    let marker = line.trim_end_matches(' ').strip_suffix('⏎');
    let columns = |text: &str| {
        let mut parts = text.split('\t');
        let first = parts.next().unwrap_or_default().width();
        parts.fold(first, |col, part| col + (8 - col % 8) + part.width())
    };
    if marker.is_some()
        || columns(line) >= width
        || (!line.is_ascii() && line.ends_with(' '))
    {
        if let Some(col) = end_column(server, pane, width, rows, row)? {
            if columns(line) == col {
                return Ok((line, false));
            }
            if let Some(body) = marker
                && columns(body) == col
            {
                return Ok((body, false));
            }
            bail!("Cannot verify the fish output boundary");
        }
    }
    Ok((line, true))
}

fn end_column(
    server: &Server,
    pane: &str,
    width: usize,
    rows: usize,
    row: usize,
) -> Result<Option<usize>> {
    // tmux's diagnostic capture always expands every grid cell.
    ensure!(
        width.saturating_mul(rows) <= 250_000,
        "Pane history too large to verify the final output column"
    );
    let dump = super::client::output_with_limit(
        server.command().args(["capture-pane", "-pR", "-t", pane]),
        64 * 1024 * 1024,
    )?;
    let prefix = format!("\tL {row} (");
    let header = dump
        .lines()
        .find(|line| line.starts_with(&prefix))
        .context("Missing tmux output boundary")?;
    if !header.contains("END_OUTPUT") {
        return Ok(None);
    }
    let (_, columns) = header
        .split_once(" osc133=")
        .context("Missing OSC 133 columns")?;
    Ok(Some(
        columns
            .split(',')
            .nth(3)
            .context("Missing command end column")?
            .parse()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::Pty;
    use portable_pty::PtySize;
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "requires tmux 3.8 and fish with OSC 133 integration"]
    fn attached_client_copies_only_complete_fish_output() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("socket");
        let server = Server {
            socket: Some(socket.clone()),
        };
        let tmux = |args: &[&str]| {
            let r = Command::new("tmux")
                .arg("-S")
                .arg(&socket)
                .args(args)
                .env_remove("TMUX")
                .output()
                .unwrap();
            assert!(r.status.success(), "{:?}", r.stderr);
            String::from_utf8(r.stdout).unwrap()
        };
        tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "test",
            "fish --no-config --private",
        ]);
        tmux(&["split-window", "-dh", "printf 'NEIGHBOUR'; sleep 20"]);
        let args = [
            "tmux".into(),
            "-S".into(),
            socket.clone().into_os_string(),
            "attach-session".into(),
            "-t".into(),
            "test".into(),
        ];
        let pty = Pty::spawn(
            &args,
            PtySize {
                rows: 16,
                cols: 80,
                ..PtySize::default()
            },
            || {},
        )
        .unwrap();
        let tty = pty.tty_name().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !tmux(&["list-clients", "-F", "#{client_tty}"])
            .contains(tty.to_str().unwrap())
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(tmux_running(&tty), "attached tmux client must be detected");
        let width: usize =
            tmux(&["display-message", "-pt", "test:0.0", "#{pane_width}"])
                .trim()
                .parse()
                .unwrap();
        let full_row = "x".repeat(width);
        let exact = format!("printf '{full_row}'");
        let wrapped = format!("echo '{full_row}x'");
        let wrapped_text = format!("{full_row}x\n");
        for (cmd, expected) in [
            (exact.as_str(), full_row.as_str()),
            (wrapped.as_str(), wrapped_text.as_str()),
            ("printf 'literal⏎\\n'", "literal⏎\n"),
            ("echo -n asdf", "asdf"),
            ("printf 'abc\\tdef'", "abc\tdef"),
            ("printf 'one\\n\\ntwo  \\n'", "one\n\ntwo  \n"),
        ] {
            tmux(&["send-keys", "-t", "test:0.0", cmd, "Enter"]);
            loop {
                let result =
                    from_client(&server, &tty, OutputLimits::default());
                if result.as_deref().ok() == Some(expected) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "expected {expected:?}, got {result:?}"
                );
                std::thread::sleep(Duration::from_millis(30));
            }
        }
        let small = OutputLimits {
            max_lines: 2,
            max_bytes: 1024,
        };
        assert!(
            from_client(&server, &tty, small)
                .unwrap_err()
                .to_string()
                .contains("2 lines")
        );
        tmux(&["kill-server"]);
    }
}
