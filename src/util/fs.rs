//! Filesystem helpers aware of root vs. unprivileged execution (`WebPulse\Util\Fs`),
//! plus the few libc calls the engine needs (euid, user/group lookups).

use std::ffi::CString;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

/// Effective uid of the process.
pub fn euid() -> u32 {
    // SAFETY: geteuid() has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// True when running as root.
pub fn is_root() -> bool {
    euid() == 0
}

/// Password-database entry of a user: (uid, gid, home directory).
pub fn passwd(user: &str) -> Option<(u32, u32, String)> {
    let name = CString::new(user).ok()?;
    let mut buf = vec![0 as libc::c_char; 16384];
    // SAFETY: zeroed passwd struct is a valid out-parameter for getpwnam_r.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: all pointers are valid for the duration of the call; buf is large enough
    // or the call reports ERANGE (treated as "unknown user").
    let rc = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut pw,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    let dir = if pw.pw_dir.is_null() {
        String::new()
    } else {
        // SAFETY: pw_dir points into buf, NUL-terminated by getpwnam_r.
        unsafe { std::ffi::CStr::from_ptr(pw.pw_dir) }
            .to_string_lossy()
            .into_owned()
    };
    Some((pw.pw_uid, pw.pw_gid, dir))
}

/// uid of a user name, or `None` when unknown.
pub fn user_id(user: &str) -> Option<u32> {
    passwd(user).map(|p| p.0)
}

/// gid of a group name, or `None` when unknown.
pub fn group_id(group: &str) -> Option<u32> {
    let name = CString::new(group).ok()?;
    let mut buf = vec![0 as libc::c_char; 16384];
    // SAFETY: zeroed group struct is a valid out-parameter for getgrnam_r.
    let mut gr: libc::group = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::group = std::ptr::null_mut();
    // SAFETY: see `passwd`.
    let rc = unsafe {
        libc::getgrnam_r(
            name.as_ptr(),
            &mut gr,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    Some(gr.gr_gid)
}

/// Peak resident set size of this process in bytes (`VmHWM`), 0 when unknown.
pub fn peak_rss() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kb: u64 = rest
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse()
                .unwrap_or(0);
            return kb * 1024;
        }
    }
    0
}

/// System directories a configured WebPulse directory may never be: creating or
/// re-moding them would damage the server.
pub const PROTECTED_DIRS: &[&str] = &[
    "/", "/etc", "/var", "/var/log", "/usr", "/home", "/root", "/tmp",
];

/// True when `dir` is (or resolves through symlinks / `..` to) one of [`PROTECTED_DIRS`].
pub fn is_protected_dir(dir: &Path) -> bool {
    let lexical = {
        let s = dir.to_string_lossy();
        let t = s.trim_end_matches('/');
        PathBuf::from(if t.is_empty() && s.starts_with('/') {
            "/"
        } else {
            t
        })
    };
    let canon = std::fs::canonicalize(dir).ok();
    PROTECTED_DIRS.iter().any(|p| {
        let p = Path::new(p);
        lexical == p || canon.as_deref() == Some(p)
    })
}

fn check_dir_target(dir: &Path) -> Result<()> {
    if is_protected_dir(dir) {
        return Err(anyhow!(
            "refusing to use the system directory {} as a WebPulse directory",
            dir.display()
        ));
    }
    if std::fs::symlink_metadata(dir)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(anyhow!(
            "refusing to use symlinked directory {}",
            dir.display()
        ));
    }
    Ok(())
}

/// Creates `dir` (recursively) when missing, then gives the new directory `mode` and,
/// when running as root, `owner`/`group` (if they exist). A directory that already
/// exists is left exactly as it is (its mode and ownership are the administrator's).
/// Returns whether the directory was created. Refuses symlinks and the system
/// directories of [`PROTECTED_DIRS`].
pub fn ensure_dir(dir: &Path, mode: u32, owner: Option<&str>, group: Option<&str>) -> Result<bool> {
    check_dir_target(dir)?;
    if dir.is_dir() {
        return Ok(false);
    }
    if std::fs::DirBuilder::new()
        .recursive(true)
        .mode(mode)
        .create(dir)
        .is_err()
        && !dir.is_dir()
    {
        return Err(anyhow!("cannot create directory {}", dir.display()));
    }
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode));
    own(dir, owner, group);
    Ok(true)
}

/// Like [`ensure_dir`] but also enforces `mode` and ownership on an existing
/// directory. Only for directories WebPulse owns outright (the per-account
/// directories under the data directory), never for a configured path.
pub fn enforce_dir(dir: &Path, mode: u32, owner: Option<&str>, group: Option<&str>) -> Result<()> {
    if !ensure_dir(dir, mode, owner, group)? {
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode));
        own(dir, owner, group);
    }
    Ok(())
}

/// Creates the empty file `path` with mode 0600 (exclusive, never following a
/// symlink) so that a database SQLite is about to create is never world-readable,
/// not even for an instant (SQLite gives its `-journal` the database's mode).
/// An existing file is left alone. True when the file was created.
pub fn precreate_private(path: &Path) -> std::io::Result<bool> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(_) => {
            // umask cannot widen 0600, but it could narrow it; make it exact
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

/// chown/chgrp `path` when root and the principals exist (best effort).
pub fn own(path: &Path, owner: Option<&str>, group: Option<&str>) {
    if !is_root() {
        return;
    }
    let uid = owner.and_then(user_id);
    let gid = group.and_then(group_id);
    if uid.is_some() || gid.is_some() {
        let _ = std::os::unix::fs::chown(path, uid, gid);
    }
}

/// Atomically writes `data` to `file` (temporary file + rename) with `mode`.
pub fn write_atomic(file: &Path, data: &[u8], mode: u32) -> Result<()> {
    let dir = file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let base = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut rnd = [0u8; 4];
    rand::fill(&mut rnd);
    let tmp = dir.join(format!(".{base}.{}.{}.tmp", std::process::id(), hex(&rnd)));
    let write = || -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(data)?;
        Ok(())
    };
    if write().is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow!("cannot write {}", tmp.display()));
    }
    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    if std::fs::rename(&tmp, file).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow!(
            "cannot rename {} to {}",
            tmp.display(),
            file.display()
        ));
    }
    Ok(())
}

/// Lowercase hex of bytes.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn is_regular_mode(mode: u32) -> bool {
    mode & 0o170000 == 0o100000
}

/// Opens an existing regular file for reading without ever blocking and without
/// following a file swapped in after the caller's `lstat()`: the open is
/// non-blocking (a FIFO renamed into place returns at once) and the descriptor must
/// be a regular file with the same device and inode as `lst`. For files in
/// directories writable by an account.
pub fn open_regular(path: &Path, lst: Option<&Metadata>) -> Result<File> {
    let owned;
    let lst = match lst {
        Some(m) => m,
        None => {
            owned = std::fs::symlink_metadata(path)
                .map_err(|_| anyhow!("cannot stat {}", path.display()))?;
            &owned
        }
    };
    if !is_regular_mode(lst.mode()) {
        return Err(anyhow!("{} is not a regular file", path.display()));
    }
    let f = open_nonblocking(path).map_err(|_| anyhow!("cannot open {}", path.display()))?;
    let st = f
        .metadata()
        .map_err(|_| anyhow!("{} changed while opening", path.display()))?;
    if !is_regular_mode(st.mode()) || st.ino() != lst.ino() || st.dev() != lst.dev() {
        return Err(anyhow!("{} changed while opening", path.display()));
    }
    Ok(f)
}

/// `open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC)`.
pub fn open_nonblocking(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}

/// Reads a whole regular file of at most `max` bytes through [`open_regular`].
pub fn read_regular(path: &Path, max: u64, lst: Option<&Metadata>) -> Result<Vec<u8>> {
    let f = open_regular(path, lst)?;
    let too_large = || anyhow!("{} is larger than {max} bytes", path.display());
    let st = f.metadata().map_err(|_| too_large())?;
    if st.len() > max {
        return Err(too_large());
    }
    let mut data = Vec::new();
    let mut lim = f.take(max + 1);
    // a non-blocking read of a regular file never returns EAGAIN
    let _ = lim.read_to_end(&mut data);
    if data.len() as u64 > max {
        return Err(too_large());
    }
    Ok(data)
}

/// Removes a file or directory tree without following symlinks. Directories not
/// owned by `euid` (default: the effective uid) are handed to
/// `rm -rf --one-file-system`, which walks them race-free. True when `path` is gone.
pub fn remove_tree(path: &Path, euid_override: Option<u32>) -> bool {
    let st = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    if !st.file_type().is_dir() {
        return std::fs::remove_file(path).is_ok();
    }
    let me = euid_override.unwrap_or_else(euid);
    if st.uid() != me {
        return rm_rf(path);
    }
    if let Ok(rd) = std::fs::read_dir(path) {
        let mut names: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
        names.sort();
        for n in names {
            remove_tree(&path.join(n), Some(me));
        }
    }
    std::fs::remove_dir(path).is_ok()
}

fn rm_rf(path: &Path) -> bool {
    let ok = std::process::Command::new("rm")
        .arg("-rf")
        .arg("--one-file-system")
        .arg("--")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok();
    ok && std::fs::symlink_metadata(path).is_err()
}

/// Sorted entry names of a directory (PHP `scandir()` without `.` and `..`);
/// empty when unreadable.
pub fn scandir(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    v.sort();
    v
}

/// True when `p` is a symlink (PHP `is_link`).
pub fn is_link(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// PHP `is_writable()`: `access(W_OK)` with the real uid.
pub fn is_writable(p: &Path) -> bool {
    let Ok(c) = CString::new(p.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    // SAFETY: c is a valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// PHP `is_readable()`: `access(R_OK)`.
pub fn is_readable(p: &Path) -> bool {
    let Ok(c) = CString::new(p.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    // SAFETY: c is a valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::R_OK) == 0 }
}
