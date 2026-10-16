//! File logger (`<log_dir>/webpulse.log`, rotated by size 5 × 5 MiB) with central
//! sanitising of messages (`WebPulse\Log`): attacker-controlled strings (paths, UAs,
//! file names, settings keys) reach the log, so every message is made one line of
//! valid UTF-8 (control characters replaced with `?`) and capped at 8 KiB.
//!
//! Line format: `2026-10-16T08:00:00+02:00 [warn] 1234 message {"json":"context"}`.
//! Never fails: when the file cannot be written (e.g. the cPanel plugin runs as the
//! account) messages are dropped, or mirrored to stderr when verbose.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use serde_json::Value;

use crate::util::{fs, json, strs};

/// Log levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Details (only with `--verbose`).
    Debug = 0,
    /// Normal events.
    Info = 1,
    /// Problems that do not stop the run (mirrored to stderr by the CLI).
    Warn = 2,
    /// Failures (mirrored to stderr by the CLI).
    Error = 3,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// Longest message kept (bytes).
pub const MAX_MESSAGE: usize = 8192;
const MAX_BYTES: u64 = 5 * 1024 * 1024;
const KEEP: u32 = 5;

/// The logger.
#[derive(Debug)]
pub struct Log {
    file: Option<PathBuf>,
    min_level: AtomicU8,
    mirror_stderr: AtomicBool,
    stderr_errors: AtomicBool,
    mkdir_tried: AtomicBool,
    lock: Mutex<()>,
}

impl Log {
    /// Logs to `file`; `None` logs nothing to disk (stderr only when mirroring).
    pub fn new(file: Option<PathBuf>, min_level: Level, mirror_stderr: bool) -> Log {
        Log {
            file,
            min_level: AtomicU8::new(min_level as u8),
            mirror_stderr: AtomicBool::new(mirror_stderr),
            stderr_errors: AtomicBool::new(false),
            mkdir_tried: AtomicBool::new(false),
            lock: Mutex::new(()),
        }
    }

    /// A logger that writes nothing.
    pub fn disabled() -> Log {
        Log::new(None, Level::Error, false)
    }

    /// Lowest level written.
    pub fn min_level(&self) -> Level {
        match self.min_level.load(Ordering::Relaxed) {
            0 => Level::Debug,
            1 => Level::Info,
            2 => Level::Warn,
            _ => Level::Error,
        }
    }

    /// `--verbose`: mirror every message (debug included) to stderr.
    pub fn set_verbose(&self, on: bool) {
        self.mirror_stderr.store(on, Ordering::Relaxed);
        self.min_level
            .store(if on { 0 } else { 1 }, Ordering::Relaxed);
    }

    /// Mirror warnings and errors to stderr (CLI default when not `--quiet`).
    pub fn set_stderr_errors(&self, on: bool) {
        self.stderr_errors.store(on, Ordering::Relaxed);
    }

    /// Path of the log file.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Debug message.
    pub fn debug(&self, msg: impl AsRef<str>) {
        self.write(Level::Debug, msg.as_ref(), None)
    }
    /// Informational message.
    pub fn info(&self, msg: impl AsRef<str>) {
        self.write(Level::Info, msg.as_ref(), None)
    }
    /// Warning.
    pub fn warn(&self, msg: impl AsRef<str>) {
        self.write(Level::Warn, msg.as_ref(), None)
    }
    /// Error.
    pub fn error(&self, msg: impl AsRef<str>) {
        self.write(Level::Error, msg.as_ref(), None)
    }

    /// Message at `level` without context.
    pub fn log(&self, level: Level, msg: &str) {
        self.write(level, msg, None)
    }

    /// Message with a JSON context appended (PHP `$log->warn($msg, $ctx)`).
    pub fn log_ctx(&self, level: Level, msg: &str, ctx: &Value) {
        self.write(level, msg, Some(ctx))
    }

    /// Writes one line.
    pub fn write(&self, level: Level, msg: &str, ctx: Option<&Value>) {
        if (level as u8) < self.min_level.load(Ordering::Relaxed) {
            return;
        }
        let tz = crate::util::time::server_tz();
        let now = crate::util::time::format_ts(
            chrono::Utc::now().timestamp(),
            &tz,
            "%Y-%m-%dT%H:%M:%S%:z",
        );
        let mut line = format!(
            "{now} [{}] {} {}",
            level.tag(),
            std::process::id(),
            sanitize(msg)
        );
        if let Some(c) = ctx
            && !matches!(c, Value::Object(m) if m.is_empty())
            && !c.is_null()
        {
            line.push(' ');
            line.push_str(&json::encode(c));
        }
        line.push('\n');
        if self.mirror_stderr.load(Ordering::Relaxed)
            || (self.stderr_errors.load(Ordering::Relaxed) && level >= Level::Warn)
        {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        let Some(file) = &self.file else {
            return;
        };
        let _guard = self.lock.lock();
        let dir = file.parent().unwrap_or(Path::new("."));
        if !dir.is_dir() {
            let parent = dir.parent().unwrap_or(Path::new("/"));
            if self.mkdir_tried.load(Ordering::Relaxed)
                || !parent.is_dir()
                || !fs::is_writable(parent)
            {
                return;
            }
            self.mkdir_tried.store(true, Ordering::Relaxed);
            use std::os::unix::fs::DirBuilderExt;
            if std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .is_err()
            {
                return;
            }
        }
        if !fs::is_writable(dir) {
            return;
        }
        rotate(file);
        let fresh = !file.exists();
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(file)
        {
            crate::util::lock::flock(&f, libc::LOCK_EX);
            let _ = f.write_all(line.as_bytes());
            crate::util::lock::flock(&f, libc::LOCK_UN);
            if fresh {
                let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}

/// One log line's worth of text: invalid UTF-8 and control characters (CR, LF,
/// ESC…) replaced with `?`, truncated to [`MAX_MESSAGE`] bytes (with `…`).
pub fn sanitize(msg: &str) -> String {
    sanitize_bytes(msg.as_bytes())
}

/// [`sanitize`] for raw bytes.
pub fn sanitize_bytes(msg: &[u8]) -> String {
    strs::truncate(&strs::clean(msg), MAX_MESSAGE, strs::ELLIPSIS)
}

fn rotate(path: &Path) {
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if size < MAX_BYTES {
        return;
    }
    let name = |i: u32| PathBuf::from(format!("{}.{i}", path.display()));
    for i in (1..KEEP).rev() {
        if name(i).exists() {
            let _ = std::fs::rename(name(i), name(i + 1));
        }
    }
    let _ = std::fs::rename(path, name(1));
}

#[cfg(test)]
mod tests {
    #[test]
    fn sanitizes() {
        assert_eq!(super::sanitize("a\nb\x1b[31mc"), "a?b?[31mc");
        assert_eq!(
            super::sanitize(&"x".repeat(100_000)).len(),
            super::MAX_MESSAGE
        );
    }
}
