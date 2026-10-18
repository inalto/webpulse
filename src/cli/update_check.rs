//! `webpulse update-check [--force] [--json]`: asks GitHub for the latest release
//! (`update.repo`) and stores the answer in the registry for the WHM bell
//! (see [`crate::engine::update`]). The nightly `webpulse rollup` runs the same
//! check when `update.check` is on.

use anyhow::Result;
use serde_json::{Value, json};

use super::{Ctx, Kind, RawArgs};
use crate::app::{App, is_production_config};
use crate::engine::update;
use crate::error::exit;

/// One-line description.
pub const SUMMARY: &str = "check GitHub for a newer WebPulse release (shown in WHM)";

/// `--help` text.
pub const USAGE: &str = "Usage: webpulse update-check [--force] [--json]

Asks the GitHub API for the latest release of update.repo (default
inalto/webpulse), compares it with this version and stores the result for
the bell in WHM. The network is used at most once every 20 hours unless
--force. Does nothing when update.check is false, unless --force.
Disable the nightly check with: webpulse config set update.check false";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[("force", Kind::Flag)];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Ask GitHub even when the last check is recent (or update.check is false)
    #[arg(long)]
    pub force: bool,
}

/// `update.url` may be used by this process.
pub fn override_allowed(app: &App) -> bool {
    update::override_allowed(
        app.is_root(),
        app.config_path().is_some_and(is_production_config),
    )
}

/// Runs a check against the registry of `app`; returns the stored status and
/// whether the network was used.
pub fn run_check(app: &App, force: bool) -> Result<(Value, bool)> {
    let reg = app.registry()?;
    let prev = reg.settings().get(update::SETTING_KEY).cloned();
    let url = update::source_url(&app.config, override_allowed(app));
    let (st, fetched) = update::check(url, prev.as_ref(), App::version(), app.now(), force);
    if fetched {
        reg.set_setting(update::SETTING_KEY, &st)?;
        match st["error"].as_str() {
            Some(e) => app.log.warn(format!("update check failed: {e}")),
            None => app.log.info(format!(
                "update check: latest release {} (running {})",
                st["latest"].as_str().unwrap_or("?"),
                App::version()
            )),
        }
    }
    Ok((st, fetched))
}

/// The nightly check of `webpulse rollup`: only when `update.check` is on and,
/// on a non-production configuration (tests, dev), only with an `update.url`.
/// Never fails.
pub fn nightly(app: &App) {
    if !update::check_enabled(&app.config) {
        return;
    }
    let production = app.config_path().is_some_and(is_production_config);
    if !production && !update::has_override(&app.config, override_allowed(app)) {
        return;
    }
    if let Err(e) = run_check(app, false) {
        app.log.warn(format!("update check: {e:#}"));
    }
}

fn human(st: &Value) -> String {
    let cur = st["current"].as_str().unwrap_or("");
    if let Some(e) = st["error"].as_str() {
        return format!("update check failed: {e}");
    }
    match st["latest"].as_str() {
        None => format!("WebPulse {cur}: no release information yet"),
        Some(l) if st["is_newer"] == json!(true) => format!(
            "WebPulse {l} is available (this is {cur}){}\nUpgrade from WHM (bell in the header) or run: install.sh --update",
            st["html_url"]
                .as_str()
                .map(|u| format!(": {u}"))
                .unwrap_or_default()
        ),
        Some(l) => format!("WebPulse {cur} is up to date (latest release {l})"),
    }
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let app = ctx.app()?;
    let force = raw.flag("force");
    if !force && !update::check_enabled(&app.config) {
        ctx.info("update checks are disabled (update.check = false); use --force to check anyway");
        if ctx.json() {
            let prev = app.registry()?.settings().get(update::SETTING_KEY).cloned();
            let mut v = update::view(prev.as_ref(), App::version());
            v["checked"] = json!(false);
            ctx.result(&v, String::new);
        }
        return Ok(exit::OK);
    }
    let (st, fetched) = run_check(app, force)?;
    let mut v = update::view(Some(&st), App::version());
    v["checked"] = json!(fetched);
    ctx.result(&v, || human(&v));
    Ok(if fetched && st["error"].is_string() {
        exit::ERROR
    } else {
        exit::OK
    })
}

/// Longest an upgrade may run before install.sh is killed.
pub const UPGRADE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// `webpulse dev run-upgrade --job ID --tag TAG [--repo OWNER/NAME]` (hidden; started
/// detached by the WHM `admin.run {type: "upgrade"}` action): runs
/// `bash <install_root>/install.sh --update --ref TAG -y` with a clean environment,
/// appends its output to `<log_dir>/upgrade.log`, waits for it and finishes job ID
/// `ok` or `error` from its exit status.
pub fn run_upgrade(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let app = ctx.app()?;
    let tag = raw.opt("tag").unwrap_or("");
    if !update::valid_tag(tag) {
        return Err(super::usage("--tag must be a release tag such as v0.4.0"));
    }
    let repo = raw.opt("repo");
    if let Some(r) = repo
        && !update::valid_repo(r)
    {
        return Err(super::usage("--repo must be owner/name"));
    }
    let id = raw
        .opt("job")
        .and_then(|j| j.parse::<i64>().ok())
        .ok_or_else(|| super::usage("--job ID is required"))?;
    let reg = app.registry()?;
    if reg.job(id)?.is_none() {
        return Err(super::usage(format!("job {id} does not exist")));
    }
    let finish = |status: &str, msg: &str| {
        if let Err(e) = reg.finish_job(id, status, 0, 0, 0, Some(msg)) {
            app.log
                .error(format!("upgrade: cannot finish job {id}: {e:#}"));
        }
    };
    let Some(installer) = update::installer(app.paths.root()) else {
        let msg =
            "install.sh was not found in the installation directory (or is not owned by root)";
        app.log.error(format!("upgrade: {msg}"));
        finish("error", msg);
        return Ok(exit::ERROR);
    };
    let log_path = app.paths.log_dir().join("upgrade.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);
    let (out, err) = match log.and_then(|mut f| {
        use std::os::unix::fs::PermissionsExt;
        let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
        writeln!(
            f,
            "\n=== {} upgrade from {} to {tag} (job #{id}): {} --update --ref {tag} -y",
            crate::util::time::gm_format(crate::util::time::now_ts(), "%Y-%m-%d %H:%M:%S UTC"),
            App::version(),
            installer.display()
        )?;
        Ok((f.try_clone()?, f))
    }) {
        Ok((o, e)) => (Stdio::from(o), Stdio::from(e)),
        Err(e) => {
            app.log
                .warn(format!("upgrade: cannot write {}: {e}", log_path.display()));
            (Stdio::null(), Stdio::null())
        }
    };
    let bash = if std::path::Path::new("/bin/bash").is_file() {
        "/bin/bash"
    } else {
        "bash"
    };
    let mut cmd = Command::new(bash);
    cmd.arg(&installer)
        .args(["--update", "--ref", tag, "-y"])
        .env_clear()
        .env(
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        )
        .env("LANG", "C")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .current_dir("/");
    if let Some(r) = repo {
        cmd.args(["--repo", r]);
    }
    for k in ["HOME", "TMPDIR"] {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    app.log.info(format!(
        "upgrade: running {} --update --ref {tag} (job {id})",
        installer.display()
    ));
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("could not start install.sh: {e}");
            app.log.error(format!("upgrade: {msg}"));
            finish("error", &msg);
            return Ok(exit::ERROR);
        }
    };
    let t0 = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if t0.elapsed() > UPGRADE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(250)),
            Err(_) => break None,
        }
    };
    let logs = log_path.display();
    match status {
        Some(s) if s.success() => {
            app.log
                .info(format!("upgrade: install.sh finished, now {tag}"));
            finish("ok", &format!("Upgraded to {tag}"));
            // the cached release status predates this version: drop it so the next
            // page load or check fetches a fresh one instead of showing an older "latest"
            if let Ok(reg) = app.registry() {
                let _ = reg.delete_setting(update::SETTING_KEY);
            }
            Ok(exit::OK)
        }
        Some(s) => {
            let code = s
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "a signal".into());
            let msg = format!("install.sh exited with status {code}; see {logs}");
            app.log.error(format!("upgrade: {msg}"));
            finish("error", &msg);
            Ok(exit::ERROR)
        }
        None => {
            let msg =
                format!("install.sh did not finish in 30 minutes and was stopped; see {logs}");
            app.log.error(format!("upgrade: {msg}"));
            finish("error", &msg);
            Ok(exit::ERROR)
        }
    }
}
