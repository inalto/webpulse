//! File logger (`<log_dir>/webpulse.log`, rotated by size 5 × 5 MiB) with central
//! sanitising of messages: attacker-controlled strings (paths, UAs, file names)
//! reach the log, so control characters are replaced and lines are capped.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

const MAX_BYTES: u64 = 5 * 1024 * 1024;
const KEEP: u32 = 5;
const MAX_MSG: usize = 8 * 1024;

#[derive(Debug)]
pub struct Log {
    file: Option<PathBuf>,
    min_level: Level,
    mirror_stderr: bool,
    inner: Mutex<Option<File>>,
}

impl Log {
    /// `file = None` logs to stderr only (cPanel user context, tests).
    pub fn new(file: Option<PathBuf>, min_level: Level, mirror_stderr: bool) -> Log {
        Log { file, min_level, mirror_stderr, inner: Mutex::new(None) }
    }
    pub fn disabled() -> Log { Log::new(None, Level::Error, false) }
    pub fn min_level(&self) -> Level { self.min_level }

    pub fn debug(&self, msg: impl AsRef<str>) { self.log(Level::Debug, msg.as_ref()) }
    pub fn info(&self, msg: impl AsRef<str>) { self.log(Level::Info, msg.as_ref()) }
    pub fn warn(&self, msg: impl AsRef<str>) { self.log(Level::Warn, msg.as_ref()) }
    pub fn error(&self, msg: impl AsRef<str>) { self.log(Level::Error, msg.as_ref()) }

    pub fn log(&self, level: Level, msg: &str) {
        if level < self.min_level {
            return;
        }
        let line = format!("{} [{}] {}\n", Utc::now().format("%Y-%m-%dT%H:%M:%SZ"), level.tag(), sanitize(msg));
        if self.mirror_stderr || self.file.is_none() {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        if let Some(path) = &self.file {
            if let Ok(mut guard) = self.inner.lock() {
                if guard.is_none() {
                    *guard = open(path);
                }
                if let Some(f) = guard.as_mut() {
                    let _ = f.write_all(line.as_bytes());
                    if f.metadata().map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
                        drop(guard.take());
                        rotate(path);
                        *guard = open(path);
                    }
                }
            }
        }
    }
}

/// Replaces control characters (incl. newlines) with `?` and caps the length.
pub fn sanitize(msg: &str) -> String {
    let mut out: String = msg.chars().map(|c| if c.is_control() { '?' } else { c }).collect();
    if out.len() > MAX_MSG {
        let mut cut = MAX_MSG;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push('…');
    }
    out
}

fn open(path: &Path) -> Option<File> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let f = OpenOptions::new().create(true).append(true).open(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Some(f)
}

fn rotate(path: &Path) {
    let name = |i: u32| if i == 0 { path.to_path_buf() } else { PathBuf::from(format!("{}.{i}", path.display())) };
    let _ = std::fs::remove_file(name(KEEP));
    for i in (1..KEEP).rev() {
        let _ = std::fs::rename(name(i), name(i + 1));
    }
    let _ = std::fs::rename(name(0), name(1));
}

#[cfg(test)]
mod tests {
    #[test]
    fn sanitizes() {
        assert_eq!(super::sanitize("a\nb\x1b[31mc"), "a?b?[31mc");
    }
}
