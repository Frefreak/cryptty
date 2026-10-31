use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_LIMIT: u64 = 128 * 1024;

pub(super) fn unknown_osc(command: Option<u32>, bytes: usize, count: u64) {
    append(
        "unsupported-osc.log",
        &format!("unknown OSC {command:?} bytes={bytes} count={count}"),
    );
}

pub(super) fn mode_failure(mode: &str, size: (usize, usize), reason: &str) {
    append(
        "diagnostics.log",
        &format!("{mode} capture failed grid={}x{}: {reason}", size.0, size.1,),
    );
}

fn append(name: &str, message: &str) {
    let result = (|| -> io::Result<()> {
        let root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local/state"))
            })
            .ok_or_else(|| io::Error::other("No state directory available"))?;
        let directory = root.join("cryptty");
        fs::create_dir_all(&directory)?;
        let path = directory.join(name);
        if fs::metadata(&path).is_ok_and(|meta| meta.len() >= LOG_LIMIT) {
            fs::rename(&path, directory.join(format!("{name}.1")))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        writeln!(file, "{time} pid={} {message}", std::process::id())
    })();
    if let Err(error) = result {
        eprintln!("Cryptty: cannot write {name} diagnostic: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn mode_failures_append_and_rotate_with_stderr_discarded() {
        const CHILD: &str = "CRYPTTY_DIAGNOSTIC_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            mode_failure("Vim", (80, 24), "capture timed out");
            mode_failure("Marker", (80, 24), "pane at col=40 row=0 differs");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let run = || {
            let status = std::process::Command::new(
                std::env::current_exe().unwrap(),
            )
            .args([
                "--exact",
                "diagnostics::tests::mode_failures_append_and_rotate_with_stderr_discarded",
            ])
            .env(CHILD, "1")
            .env("XDG_STATE_HOME", directory.path())
            .stderr(std::process::Stdio::null())
            .output()
            .unwrap();
            assert!(status.status.success());
        };
        run();
        let path = directory.path().join("cryptty/diagnostics.log");
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(
            text.contains("Vim capture failed grid=80x24: capture timed out")
        );
        assert!(
            text.contains("Marker capture failed grid=80x24: pane at col=40")
        );
        assert!(text.lines().all(|line| {
            line.split_whitespace()
                .next()
                .unwrap()
                .parse::<u64>()
                .is_ok()
                && line.contains(" pid=")
        }));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(LOG_LIMIT)
            .unwrap();
        run();
        assert_eq!(
            fs::metadata(path.with_extension("log.1")).unwrap().len(),
            LOG_LIMIT
        );
        assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 2);
    }
}
