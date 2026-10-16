//! `webpulse collect [--user U [--domain D]] [--hooked] [--job [ID]]`
//! (`WebPulse\Cli\Command\CollectCommand`).
//!
//! The global run (no `--user`) collects every account in its own child process
//! (`collect --user U --child --json`): a fatal error in one account cannot stop
//! the accounts after it. Each domain is committed transactionally, so a child that
//! dies leaves its checkpoints consistent and the next run resumes from there.

use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::Result;
use serde_json::{Map, Value, json};

use super::{Ctx, Job, Kind, RawArgs, usage};
use crate::engine::collector::{Collector, UserTotals};
use crate::error::exit;
use crate::log::Level;
use crate::paths::{is_valid_domain, is_valid_user};
use crate::util::{fs, lock::Lock, strs};

/// Seconds an explicit `--user` / hooked run waits for the user lock.
pub const USER_LOCK_WAIT: f64 = 300.0;

/// One-line description.
pub const SUMMARY: &str = "tail the access logs and update the statistics (cron runs this)";

/// `--help` text.
pub const USAGE: &str =
    "Usage: webpulse collect [--user USER [--domain DOMAIN]] [--hooked] [--job [ID]]

  --user U      only this account (waits for its lock)
  --domain D    only this domain of --user
  --hooked      process the accounts flagged by the Stats::RunUser hook
                (<run_dir>/hook/<user>) and remove the flags
  --job [ID]    record the run in registry.jobs (adopt job ID when given)
  --child       (internal) one account of a global run: skip it when
                locked, never record a job row

Without --user every enabled account is processed, each in a child
process; a second concurrent run exits 0 immediately, and accounts whose
lock is held are skipped.";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[
    ("user", Kind::Value),
    ("domain", Kind::Value),
    ("hooked", Kind::Flag),
    ("job", Kind::Optional),
    ("child", Kind::Flag),
];

/// Arguments (clap description; parsed with the 0.1.0 rules by [`main`]).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Only this account
    #[arg(long)]
    pub user: Option<String>,
    /// Only this domain of --user
    #[arg(long)]
    pub domain: Option<String>,
    /// Drain the accounts flagged by the Stats::RunUser hook
    #[arg(long)]
    pub hooked: bool,
    /// Record the run in registry.jobs (adopt job ID when given)
    #[arg(long)]
    pub job: Option<Option<String>>,
    /// (internal) one account of a global run
    #[arg(long)]
    pub child: bool,
}

/// Running totals of a collect run (JSON key order of 0.1.0).
pub(crate) struct Totals {
    pub map: Map<String, Value>,
}

impl Totals {
    pub(crate) fn new() -> Totals {
        let mut map = Map::new();
        for k in [
            "users", "domains", "files", "lines", "bad", "ignored", "errors",
        ] {
            map.insert(k.into(), Value::from(0));
        }
        map.insert("skipped".into(), Value::Array(Vec::new()));
        Totals { map }
    }
    pub(crate) fn get(&self, k: &str) -> i64 {
        self.map.get(k).and_then(Value::as_i64).unwrap_or(0)
    }
    pub(crate) fn inc(&mut self, k: &str, n: i64) {
        let v = self.get(k) + n;
        self.map.insert(k.into(), Value::from(v));
    }
    pub(crate) fn skip(&mut self, s: String) {
        if let Some(Value::Array(a)) = self.map.get_mut("skipped") {
            a.push(Value::from(s));
        }
    }
    pub(crate) fn add_user(&mut self, r: &UserTotals) {
        self.inc("domains", r.domains);
        self.inc("files", r.files);
        self.inc("lines", r.lines);
        self.inc("bad", r.bad);
        self.inc("ignored", r.ignored);
        self.inc("errors", r.errors);
        for s in &r.skipped {
            self.skip(s.clone());
        }
    }
    fn add_json(&mut self, r: &Value) {
        for k in ["domains", "files", "lines", "bad", "ignored", "errors"] {
            self.inc(k, r.get(k).map(crate::config::php_int).unwrap_or(0));
        }
        if let Some(Value::Array(a)) = r.get("skipped") {
            for s in a {
                self.skip(
                    s.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| crate::util::json::encode(s)),
                );
            }
        }
    }
}

/// Parses and validates the arguments.
pub fn parse(raw: &RawArgs) -> Result<Args> {
    if let Some(p) = raw.positional().first() {
        return Err(usage(format!("unexpected argument {p}")));
    }
    let user = raw.opt("user").map(str::to_string);
    let domain = raw.opt("domain").map(str::to_string);
    if domain.is_some() && user.is_none() {
        return Err(usage("--domain requires --user"));
    }
    let child = raw.flag("child");
    if child && user.is_none() {
        return Err(usage("--child requires --user"));
    }
    if let Some(u) = &user
        && !is_valid_user(u)
    {
        return Err(usage(format!("invalid user name '{u}'")));
    }
    if let Some(d) = &domain
        && !is_valid_domain(&d.to_ascii_lowercase())
    {
        return Err(usage(format!("invalid domain '{d}'")));
    }
    let hooked = raw.flag("hooked");
    if hooked && user.is_some() {
        return Err(usage("--hooked cannot be combined with --user"));
    }
    Ok(Args {
        user,
        domain: domain.map(|d| d.to_ascii_lowercase()),
        hooked,
        job: raw.get("job").map(|v| match v {
            super::OptVal::Str(s) => Some(s.clone()),
            super::OptVal::True => None,
        }),
        child,
    })
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let args = parse(raw)?;
    if args.hooked {
        return hooked(ctx, raw);
    }
    run(ctx, raw, &args)
}

/// Runs `collect` (not `--hooked`).
pub fn run(ctx: &Ctx, raw: &RawArgs, args: &Args) -> Result<i32> {
    let app = ctx.app()?;
    let t0 = Instant::now();
    let scope = match (&args.user, &args.domain) {
        (None, _) => "all".to_string(),
        (Some(u), None) => u.clone(),
        (Some(u), Some(d)) => format!("{u}/{d}"),
    };
    let mut job = Job::new(ctx, "collect", Some(&scope), raw)?;
    let mut collector = Collector::new(app);
    let mut tot = Totals::new();

    if let Some(user) = &args.user {
        let lock = ctx.user_lock(user, if args.child { 0.0 } else { USER_LOCK_WAIT })?;
        let Some(mut lock) = lock else {
            if args.child {
                tot.skip(format!("{user} (locked)"));
                return Ok(report(ctx, &mut job, tot, t0, false));
            }
            ctx.err(&format!("account {user} is locked by another process"));
            job.finish("skipped", 0, 0, 0, Some("account locked"), true);
            return Ok(exit::ERROR);
        };
        collector.sync_registry(Some(user))?;
        let r = collector.collect_user(user, args.domain.as_deref())?;
        tot.add_user(&r);
        tot.map.insert("users".into(), Value::from(1));
        lock.release();
        if args.child {
            return Ok(report(ctx, &mut job, tot, t0, false));
        }
    } else {
        let Some(mut global) = Lock::acquire(&ctx.lock_path("collect")?, 0.0)? else {
            ctx.info("another collect is already running; nothing to do");
            job.finish(
                "skipped",
                0,
                0,
                0,
                Some("another collect is running"),
                false,
            );
            return Ok(exit::OK);
        };
        let users = collector.sync_registry(None)?;
        let reg = app.registry()?;
        for u in users {
            if let Some(acc) = reg.account(&u)?
                && !acc.enabled
                && !reg.domains(Some(&u))?.iter().any(|d| d.enabled)
            {
                continue;
            }
            match collect_in_child(ctx, &u) {
                None => tot.inc("errors", 1),
                Some(r) => {
                    tot.add_json(&r);
                    tot.inc(
                        "users",
                        r.get("users").map(crate::config::php_int).unwrap_or(0),
                    );
                    let peak = tot
                        .map
                        .get("peak_memory")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    let child = r
                        .get("peak_memory")
                        .map(crate::config::php_int)
                        .unwrap_or(0);
                    tot.map
                        .insert("peak_memory".into(), Value::from(peak.max(child)));
                }
            }
        }
        global.release();
    }
    let record = tot.get("lines") > 0 || tot.get("errors") > 0;
    Ok(report(ctx, &mut job, tot, t0, record))
}

/// `WEBPULSE_CHILD_EXE` (tests): the program to run for per-account children. Ignored
/// when running as root, so the environment cannot make the root collector execute an
/// arbitrary program.
pub fn child_exe_override() -> Option<std::path::PathBuf> {
    child_exe_from(
        crate::util::fs::is_root(),
        std::env::var_os("WEBPULSE_CHILD_EXE"),
    )
}

/// Policy of [`child_exe_override`]: the value of `WEBPULSE_CHILD_EXE` when not root.
pub fn child_exe_from(root: bool, value: Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    if root {
        return None;
    }
    value
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
}

/// Collects one account in a child process (`collect --user U --child --json`) and
/// returns its JSON totals; `None` when the child failed (logged, never fatal here).
pub fn collect_in_child(ctx: &Ctx, user: &str) -> Option<Value> {
    let app = ctx.app().ok()?;
    // test hook: honoured only for unprivileged runs, never when collecting as root
    let exe = match child_exe_override() {
        Some(e) => e,
        None => match std::env::current_exe() {
            Ok(e) => e,
            Err(_) => {
                app.log.error(format!(
                    "collect {user} failed: cannot start a child process"
                ));
                return None;
            }
        },
    };
    let mut cmd = Command::new(&exe);
    cmd.args(["collect", "--user", user, "--child", "--json"]);
    if ctx.quiet() {
        cmd.arg("--quiet");
    }
    if ctx.verbose() {
        cmd.arg("--verbose");
    }
    if let Some(cfg) = app.config_path()
        && cfg.is_file()
    {
        let real = std::fs::canonicalize(cfg).unwrap_or_else(|_| cfg.to_path_buf());
        cmd.arg("--config").arg(real);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => {
            app.log.error(format!(
                "collect {user} failed: cannot start a child process"
            ));
            return None;
        }
    };
    let code = output.status.code().unwrap_or(-1);
    let text = String::from_utf8_lossy(&output.stdout);
    let parsed: Option<Value> = serde_json::from_str(&text)
        .ok()
        .filter(|v: &Value| v.is_object() || v.is_array());
    match parsed {
        Some(v) if code == 0 => Some(v),
        _ => {
            app.log.log_ctx(
                Level::Error,
                &format!("collect {user} failed: child process exited with code {code}"),
                &json!({"output": strs::truncate(&strs::clean(text.trim().as_bytes()), 500, "")}),
            );
            None
        }
    }
}

/// `collect --hooked`: drains the accounts flagged by the Stats::RunUser hook.
pub fn hooked(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let app = ctx.app()?;
    let dir = app.paths.hook_dir();
    let mut flags: Vec<(String, i64)> = Vec::new();
    for f in fs::scandir(&dir) {
        if f.starts_with('.') || !is_valid_user(&f) {
            continue;
        }
        let p = dir.join(&f);
        match std::fs::symlink_metadata(&p) {
            Ok(st) if st.file_type().is_file() => {
                use std::os::unix::fs::MetadataExt;
                flags.push((f, st.mtime()));
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&p);
            }
            Err(_) => {}
        }
    }
    if flags.is_empty() {
        if raw.has("job") {
            Job::new(ctx, "hook", None, raw)?.finish(
                "skipped",
                0,
                0,
                0,
                Some("no hook flags"),
                false,
            );
        }
        ctx.result(&json!({"users": 0, "lines": 0}), || "no hook flags".into());
        return Ok(exit::OK);
    }
    let Some(mut global) = Lock::acquire(&ctx.lock_path("hooked")?, 0.0)? else {
        if raw.has("job") {
            Job::new(ctx, "hook", None, raw)?.finish(
                "skipped",
                0,
                0,
                0,
                Some("another hooked collect is running"),
                false,
            );
        }
        ctx.info("another hooked collect is running");
        return Ok(exit::OK);
    };
    let t0 = Instant::now();
    let scope = flags
        .iter()
        .map(|(u, _)| u.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut job = Job::new(ctx, "hook", Some(&scope), raw)?;
    let mut collector = Collector::new(app);
    let mut tot = Totals::new();
    for (u, mtime) in flags {
        let flag = dir.join(&u);
        if !app.cpanel_users().exists(&u) {
            let _ = std::fs::remove_file(&flag);
            continue;
        }
        let Some(mut lock) = ctx.user_lock(&u, USER_LOCK_WAIT)? else {
            tot.skip(format!("{u} (locked)"));
            continue;
        };
        let r = (|| -> Result<UserTotals> {
            collector.sync_registry(Some(&u))?;
            collector.collect_user(&u, None)
        })();
        match r {
            Ok(r) => {
                tot.add_user(&r);
                tot.inc("users", 1);
                use std::os::unix::fs::MetadataExt;
                if let Ok(st) = std::fs::symlink_metadata(&flag)
                    && st.mtime() == mtime
                {
                    let _ = std::fs::remove_file(&flag);
                }
            }
            Err(e) => {
                tot.inc("errors", 1);
                app.log.error(format!("hooked collect {u} failed: {e:#}"));
            }
        }
        lock.release();
    }
    global.release();
    Ok(report(ctx, &mut job, tot, t0, true))
}

/// Prints the totals and finishes the job.
pub(crate) fn report(
    ctx: &Ctx,
    job: &mut Job<'_>,
    mut tot: Totals,
    t0: Instant,
    record: bool,
) -> i32 {
    let elapsed = t0.elapsed().as_secs_f64();
    let lines = tot.get("lines");
    tot.map
        .insert("elapsed".into(), json!((elapsed * 1000.0).round() / 1000.0));
    tot.map.insert(
        "lines_per_sec".into(),
        Value::from(if elapsed > 0.0 {
            (lines as f64 / elapsed).round() as i64
        } else {
            0
        }),
    );
    let peak = tot
        .map
        .get("peak_memory")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(fs::peak_rss() as i64);
    tot.map.insert("peak_memory".into(), Value::from(peak));
    tot.map.insert(
        "job_id".into(),
        job.id().map(Value::from).unwrap_or(Value::Null),
    );
    let errors = tot.get("errors");
    let status = if errors > 0 { "error" } else { "ok" };
    let msg = (errors > 0).then(|| format!("{errors} error(s), see webpulse.log"));
    job.finish(
        status,
        lines,
        tot.get("files"),
        tot.get("domains"),
        msg.as_deref(),
        record,
    );
    tot.map.insert(
        "job_id".into(),
        job.id().map(Value::from).unwrap_or(Value::Null),
    );
    let data = Value::Object(tot.map);
    ctx.result(&data, || {
        let g = |k: &str| data.get(k).and_then(Value::as_i64).unwrap_or(0);
        let skipped: Vec<String> = data["skipped"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        format!(
            "collected {} line(s) from {} file(s) in {} domain(s) of {} account(s) in {:.2}s ({} lines/s, {} bad, {} ignored, {} error(s)){}",
            g("lines"),
            g("files"),
            g("domains"),
            g("users"),
            elapsed,
            g("lines_per_sec"),
            g("bad"),
            g("ignored"),
            g("errors"),
            if skipped.is_empty() { String::new() } else { format!("\nskipped: {}", skipped.join(", ")) }
        )
    });
    exit::OK
}
