//! Starts `webpulse <type> --job <id> …` in the background, detached from the
//! web request (new session, `nohup`, stdin from /dev/null, output appended
//! to `<log_dir>/jobs.out`), so that WHM can trigger collection, backfill and
//! rollup (port of `Http\JobRunner`).

use std::path::Path;
use std::process::{Command, Stdio};

use super::ctx::Ctx;
use super::registry::access;
use crate::reports::php::escapeshellarg;

/// Spawns the current executable with `args`; returns the command line, or
/// `None` when it could not be started.
pub fn spawn(ctx: &Ctx, args: &[String], config: Option<&Path>) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.to_string_lossy().into_owned();
    let mut parts = vec![escapeshellarg(&exe)];
    parts.extend(args.iter().map(|a| escapeshellarg(a)));
    let cmd = parts.join(" ");
    let log = ctx.paths.log_dir().join("jobs.out");
    let log_dir = ctx.paths.log_dir();
    let redirect = if log_dir.is_dir() && (access(log_dir, libc::W_OK) || access(&log, libc::W_OK))
    {
        escapeshellarg(&log.to_string_lossy())
    } else {
        "/dev/null".to_string()
    };
    let shell = format!("nohup {cmd} >>{redirect} 2>&1 </dev/null &");
    let mut c = Command::new("/bin/sh");
    c.arg("-c")
        .arg(&shell)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let root = ctx.paths.root();
    if root.is_dir() {
        c.current_dir(root);
    }
    if let Some(cfg) = config {
        c.env("WEBPULSE_CONFIG", cfg);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid() is async-signal-safe; it detaches the shell (and the
        // job it starts) from the session of the web server's request.
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    let status = c.status().ok()?;
    if status.success() { Some(cmd) } else { None }
}
