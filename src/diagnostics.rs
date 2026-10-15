use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_LIMIT: u64 = 128 * 1024;

pub(super) fn unknown_osc(command: Option<u32>, bytes: usize, count: u64) {
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
        let path = directory.join("unsupported-osc.log");
        if fs::metadata(&path).is_ok_and(|meta| meta.len() >= LOG_LIMIT) {
            fs::rename(&path, directory.join("unsupported-osc.log.1"))?;
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
        writeln!(
            file,
            "{time} pid={} unknown OSC {command:?} bytes={bytes} count={count}",
            std::process::id()
        )
    })();
    if let Err(error) = result {
        eprintln!("Cryptty: cannot write OSC diagnostic: {error}");
    }
}
