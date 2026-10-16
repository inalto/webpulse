//! `flock()` based lock files (`WebPulse\Util\Lock`). The lock is released when the
//! value is dropped or [`Lock::release`] is called.

use std::fs::{File, OpenOptions};
use std::io::{Seek, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

/// `flock(fd, op)` on an open file; true on success.
pub fn flock(f: &File, op: libc::c_int) -> bool {
    // SAFETY: the descriptor is owned by `f` and valid for the call.
    unsafe { libc::flock(f.as_raw_fd(), op) == 0 }
}

/// An exclusive lock held on a lock file.
#[derive(Debug)]
pub struct Lock {
    file: Option<File>,
    path: PathBuf,
}

impl Lock {
    /// Tries to take an exclusive lock on `path`: non-blocking when `wait_seconds` is
    /// 0, otherwise polls every 100 ms up to that long. `Ok(None)` when another
    /// process holds it; `Err` when the lock file cannot be opened.
    pub fn acquire(path: &Path, wait_seconds: f64) -> Result<Option<Lock>> {
        if let Some(dir) = path.parent()
            && !dir.is_dir()
        {
            let _ = std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir);
        }
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(|_| anyhow!("cannot open lock file {}", path.display()))?;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        let deadline = Instant::now() + Duration::from_secs_f64(wait_seconds.max(0.0));
        loop {
            if flock(&f, libc::LOCK_EX | libc::LOCK_NB) {
                let _ = f.set_len(0);
                let _ = f.rewind();
                let _ = f.write_all(std::process::id().to_string().as_bytes());
                let _ = f.flush();
                return Ok(Some(Lock {
                    file: Some(f),
                    path: path.to_path_buf(),
                }));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// True when somebody currently holds the lock on `path`.
    pub fn is_held(path: &Path) -> bool {
        if !path.is_file() {
            return false;
        }
        let Ok(f) = File::open(path) else {
            return false;
        };
        let free = flock(&f, libc::LOCK_SH | libc::LOCK_NB);
        if free {
            flock(&f, libc::LOCK_UN);
        }
        !free
    }

    /// Lock file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Releases the lock (idempotent).
    pub fn release(&mut self) {
        if let Some(f) = self.file.take() {
            flock(&f, libc::LOCK_UN);
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.release();
    }
}
