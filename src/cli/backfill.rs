//! `webpulse backfill --user U [--domain D] [--archive-dir DIR] [--job [ID]]`
//! (`WebPulse\Cli\Command\BackfillCommand`).

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use serde_json::{Value, json};

use super::collect::USER_LOCK_WAIT;
use super::{Ctx, Job, Kind, RawArgs, usage};
use crate::engine::collector::Collector;
use crate::error::exit;
use crate::paths::{is_valid_domain, is_valid_user};

/// One-line description.
pub const SUMMARY: &str = "import archived logs (/home/<user>/logs/*.gz), never twice";

/// `--help` text.
pub const USAGE: &str =
    "Usage: webpulse backfill --user USER [--domain DOMAIN] [--archive-dir DIR] [--job [ID]]

  --user U           account whose archives are imported (required)
  --domain D         only archives of this domain
  --archive-dir DIR  read archives from DIR instead of paths.archive_dir
  --job [ID]         record the run in registry.jobs (adopt job ID when given)

Archives already imported (same path, size and mtime) are skipped; lines
newer than the start of live collection are not imported again.";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[
    ("user", Kind::Value),
    ("domain", Kind::Value),
    ("archive-dir", Kind::Value),
    ("job", Kind::Optional),
];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Account whose archives are imported
    #[arg(long)]
    pub user: Option<String>,
    /// Only archives of this domain
    #[arg(long)]
    pub domain: Option<String>,
    /// Read archives from DIR instead of paths.archive_dir
    #[arg(long = "archive-dir")]
    pub archive_dir: Option<PathBuf>,
    /// Record the run in registry.jobs
    #[arg(long)]
    pub job: Option<Option<String>>,
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let Some(user) = raw.opt("user") else {
        return Err(usage("--user is required"));
    };
    if !is_valid_user(user) {
        return Err(usage(format!("invalid user name '{user}'")));
    }
    let domain = raw.opt("domain");
    if let Some(d) = domain
        && !is_valid_domain(&d.to_ascii_lowercase())
    {
        return Err(usage(format!("invalid domain '{d}'")));
    }
    let domain = domain.map(|d| d.to_ascii_lowercase());
    let dir = raw.opt("archive-dir").map(PathBuf::from);
    if let Some(d) = &dir
        && !d.is_dir()
    {
        return Err(usage(format!(
            "archive directory {} does not exist",
            d.display()
        )));
    }
    let args = Args {
        user: Some(user.to_string()),
        domain,
        archive_dir: dir,
        job: None,
    };
    run(ctx, raw, &args)
}

/// Runs the backfill.
pub fn run(ctx: &Ctx, raw: &RawArgs, args: &Args) -> Result<i32> {
    let app = ctx.app()?;
    let user = args.user.clone().unwrap_or_default();
    let t0 = Instant::now();
    let scope = match &args.domain {
        Some(d) => format!("{user}/{d}"),
        None => user.clone(),
    };
    let mut job = Job::new(ctx, "backfill", Some(&scope), raw)?;
    let Some(mut lock) = ctx.user_lock(&user, USER_LOCK_WAIT)? else {
        job.finish("skipped", 0, 0, 0, Some("account locked"), true);
        ctx.err(&format!("account {user} is locked by another process"));
        return Ok(exit::ERROR);
    };
    let mut collector = Collector::new(app);
    collector.sync_registry(Some(&user))?;
    let t = collector.backfill_user(&user, args.domain.as_deref(), args.archive_dir.as_deref())?;
    lock.release();
    let elapsed = t0.elapsed().as_secs_f64();
    let mut data = json!({
        "domains": t.domains, "files": t.files, "lines": t.lines, "imported": t.imported, "bad": t.bad,
        "ignored": t.ignored, "skipped_files": t.skipped_files, "errors": t.errors, "skipped": t.skipped,
        "elapsed": (elapsed * 1000.0).round() / 1000.0,
    });
    let msg = (t.errors > 0).then(|| format!("{} error(s), see webpulse.log", t.errors));
    job.finish(
        if t.errors > 0 { "error" } else { "ok" },
        t.lines,
        t.files,
        t.domains,
        msg.as_deref(),
        true,
    );
    data["job_id"] = job.id().map(Value::from).unwrap_or(Value::Null);
    ctx.result(&data, || {
        format!(
            "backfilled {} archive(s) of {} domain(s): {} line(s) read, {} imported, {} archive(s) already imported, {} error(s) in {:.2}s",
            t.files, t.domains, t.lines, t.imported, t.skipped_files, t.errors, elapsed
        )
    });
    Ok(exit::OK)
}
