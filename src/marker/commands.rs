use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock, mpsc};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

const HISTORY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_NAMES: usize = 8192;

pub(super) fn predefined_commands() -> &'static HashSet<&'static str> {
    static NAMES: OnceLock<HashSet<&str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        "awk bash cargo cat cd chmod chown cp curl cut date diff echo env \
         find fish git grep head journalctl jq kill kubectl less ln ls make \
         mkdir mv npm podman printf ps pwd python python3 rg rm rsync sed \
         sh sort ssh sudo systemctl tail tar tee test touch uv wc wget which \
         xargs zsh gpg-connect-agent update-alternatives \
         dbus-update-activation-environment"
            .split_whitespace()
            .collect()
    })
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HistoryShell {
    #[default]
    Fish,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryOptions {
    pub enabled: bool,
    pub shell: HistoryShell,
    pub path: Option<PathBuf>,
    pub refresh_interval_seconds: NonZeroU32,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            shell: HistoryShell::Fish,
            path: None,
            refresh_interval_seconds: 600.try_into().unwrap(),
        }
    }
}

pub struct CommandHistory {
    options: HistoryOptions,
    _stop: mpsc::Sender<()>,
    names: Arc<RwLock<Arc<HashSet<String>>>>,
}

impl CommandHistory {
    pub fn new(options: &HistoryOptions) -> Self {
        let (updates, receiver) = mpsc::channel();
        let names = Arc::new(RwLock::new(Arc::new(HashSet::new())));
        let output = names.clone();
        let initial = options.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("command-history".into())
            .spawn(move || run(initial, receiver, output))
        {
            eprintln!("Cryptty: cannot start command history worker: {error}");
        }
        Self {
            options: options.clone(),
            _stop: updates,
            names,
        }
    }

    pub fn configure(&mut self, options: &HistoryOptions) {
        if self.options != *options {
            *self = Self::new(options);
        }
    }

    pub fn snapshot(&self) -> Arc<HashSet<String>> {
        self.names.read().unwrap().clone()
    }
}

fn run(
    options: HistoryOptions,
    stop: mpsc::Receiver<()>,
    output: Arc<RwLock<Arc<HashSet<String>>>>,
) {
    let Some(source) = Source::new(&options) else {
        return;
    };
    let interval =
        Duration::from_secs(options.refresh_interval_seconds.get().into());
    refresh_loop(source, interval, stop, output);
}

fn refresh_loop(
    mut source: Source,
    interval: Duration,
    stop: mpsc::Receiver<()>,
    output: Arc<RwLock<Arc<HashSet<String>>>>,
) {
    if let Some(names) = source.load_cache() {
        *output.write().unwrap() = Arc::new(names);
    }
    loop {
        match source.refresh() {
            Ok(Some(names)) => *output.write().unwrap() = Arc::new(names),
            Ok(None) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => {
                eprintln!("Cryptty: command history refresh failed: {error}");
            }
        }
        match stop.recv_timeout(interval) {
            Ok(()) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

struct Source {
    path: PathBuf,
    cache: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
}

impl Source {
    fn new(options: &HistoryOptions) -> Option<Self> {
        if !options.enabled {
            return None;
        }
        let path = match options.shell {
            HistoryShell::Fish => options.path.clone().or_else(fish_path)?,
        };
        let mut hash = DefaultHasher::new();
        path.hash(&mut hash);
        let cache = xdg_directory("XDG_CACHE_HOME", ".cache").map(|root| {
            root.join("cryptty")
                .join(format!("commands-fish-{:016x}.txt", hash.finish()))
        });
        Some(Self {
            path,
            cache,
            stamp: None,
        })
    }

    fn load_cache(&self) -> Option<HashSet<String>> {
        let bytes = read_tail(self.cache.as_ref()?, HISTORY_BYTES).ok()?;
        Some(
            String::from_utf8_lossy(&bytes)
                .lines()
                .filter(|name| valid_name(name))
                .take(MAX_NAMES)
                .map(str::to_owned)
                .collect(),
        )
    }

    fn refresh(&mut self) -> io::Result<Option<HashSet<String>>> {
        let metadata = std::fs::metadata(&self.path)?;
        let stamp = (metadata.modified()?, metadata.len());
        if self.stamp == Some(stamp) {
            return Ok(None);
        }
        let bytes = read_tail(&self.path, HISTORY_BYTES)?;
        let names = fish_names(&bytes);
        if let Some(cache) = &self.cache
            && let Err(error) = write_cache(cache, &names)
        {
            eprintln!("Cryptty: command name cache write failed: {error}");
        }
        self.stamp = Some(stamp);
        Ok(Some(names))
    }
}

fn xdg_directory(variable: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(fallback))
        })
}

fn fish_path() -> Option<PathBuf> {
    let session =
        std::env::var("fish_history").unwrap_or_else(|_| "fish".into());
    if session.is_empty()
        || !session.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
    {
        return None;
    }
    Some(
        xdg_directory("XDG_DATA_HOME", ".local/share")?
            .join("fish")
            .join(format!("{session}_history")),
    )
}

fn read_tail(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(io::Error::other("history source must be a regular file"));
    }
    let mut file = std::fs::File::open(path)?;
    let offset = file.metadata()?.len().saturating_sub(limit);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    if offset > 0 {
        let end = bytes
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(bytes.len(), |index| index + 1);
        bytes.drain(..end);
    }
    Ok(bytes)
}

fn fish_names(bytes: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    for line in String::from_utf8_lossy(bytes).lines().rev() {
        let Some(command) = line.strip_prefix("- cmd: ") else {
            continue;
        };
        if command.len() > 16 * 1024 {
            continue;
        }
        let mut decoded = String::new();
        let mut chars = command.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('n') => decoded.push('\n'),
                    Some('\\') => decoded.push('\\'),
                    Some(other) => {
                        decoded.push('\\');
                        decoded.push(other);
                    }
                    None => decoded.push('\\'),
                }
            } else {
                decoded.push(ch);
            }
        }
        let Ok(words) = shell_words::split(&decoded) else {
            continue;
        };
        let Some(first) = words.first() else { continue };
        if !super::recognize::plausible_executable(first)
            || first.contains(['|', '<', '>', ';', '&'])
        {
            continue;
        }
        let name = first.rsplit('/').next().unwrap();
        if valid_name(name) {
            names.insert(name.to_owned());
        }
        if names.len() >= MAX_NAMES {
            break;
        }
    }
    names
}

fn valid_name(name: &str) -> bool {
    name.len() <= 128
        && name.starts_with(|ch: char| ch.is_ascii_lowercase())
        && name.bytes().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-' | b'.')
        })
}

fn write_cache(path: &Path, names: &HashSet<String>) -> io::Result<()> {
    let parent = path.parent().unwrap();
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    let mut sorted: Vec<_> = names.iter().collect();
    sorted.sort();
    for name in sorted {
        writeln!(file, "{name}")?;
    }
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
