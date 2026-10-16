//! `webpulse rollup [--user U [--domain D]] [--job [ID]]` (`RollupCommand`).

use std::time::Instant;

use anyhow::Result;
use serde_json::{Map, Value, json};

use super::{Ctx, Job, Kind, RawArgs, usage};
use crate::engine::rollup::Rollup;
use crate::error::exit;
use crate::paths::{is_valid_domain, is_valid_user};
use crate::util::lock::Lock;

/// One-line description.
pub const SUMMARY: &str = "nightly compaction: close visits, cap dimensions, retention, vacuum";

/// `--help` text.
pub const USAGE: &str = "Usage: webpulse rollup [--user USER [--domain DOMAIN]] [--job [ID]]

Closes stale visits, folds the tail of high-cardinality dimensions into
\"(other)\" (max_rows_per_dim per day), purges raw visits/events older than
raw_retention_days, prunes visitor_day and salts, refreshes the WHM summary,
runs PRAGMA optimize and VACUUM when more than 25 % of the pages are free.";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[
    ("user", Kind::Value),
    ("domain", Kind::Value),
    ("job", Kind::Optional),
];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Only this account
    #[arg(long)]
    pub user: Option<String>,
    /// Only this domain of --user
    #[arg(long)]
    pub domain: Option<String>,
    /// Record the run in registry.jobs
    #[arg(long)]
    pub job: Option<Option<String>>,
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let user = raw.opt("user");
    let domain = raw.opt("domain");
    if domain.is_some() && user.is_none() {
        return Err(usage("--domain requires --user"));
    }
    if let Some(u) = user
        && !is_valid_user(u)
    {
        return Err(usage(format!("invalid user name '{u}'")));
    }
    if let Some(d) = domain
        && !is_valid_domain(&d.to_ascii_lowercase())
    {
        return Err(usage(format!("invalid domain '{d}'")));
    }
    let args = Args {
        user: user.map(str::to_string),
        domain: domain.map(|d| d.to_ascii_lowercase()),
        job: None,
    };
    run(ctx, raw, &args)
}

/// Runs the rollup.
pub fn run(ctx: &Ctx, raw: &RawArgs, args: &Args) -> Result<i32> {
    let app = ctx.app()?;
    let Some(mut global) = Lock::acquire(&ctx.lock_path("rollup")?, 0.0)? else {
        if raw.has("job") {
            Job::new(ctx, "rollup", None, raw)?.finish(
                "skipped",
                0,
                0,
                0,
                Some("another rollup is running"),
                false,
            );
        }
        ctx.info("another rollup is running");
        return Ok(exit::OK);
    };
    let t0 = Instant::now();
    let scope = match (&args.user, &args.domain) {
        (None, _) => "all".to_string(),
        (Some(u), None) => u.clone(),
        (Some(u), Some(d)) => format!("{u}/{d}"),
    };
    let mut job = Job::new(ctx, "rollup", Some(&scope), raw)?;
    let mut rollup = Rollup::new(app);
    let mut tot = Map::new();
    for k in [
        "domains",
        "closed",
        "folded",
        "purged_visits",
        "purged_events",
        "pruned_visitor_day",
        "vacuum",
        "errors",
        "salts_pruned",
    ] {
        tot.insert(k.into(), Value::from(0));
    }
    let inc = |tot: &mut Map<String, Value>, k: &str, n: i64| {
        let v = tot.get(k).and_then(Value::as_i64).unwrap_or(0) + n;
        tot.insert(k.into(), Value::from(v));
    };
    let users = match &args.user {
        Some(u) => vec![u.clone()],
        None => rollup.users(),
    };
    for u in users {
        let Some(mut lock) = ctx.user_lock(&u, 600.0)? else {
            inc(&mut tot, "errors", 1);
            app.log
                .warn(format!("rollup: account {u} stayed locked, skipped"));
            continue;
        };
        for d in rollup.domains_with_data(&u) {
            if args.domain.as_deref().map(|x| x != d).unwrap_or(false) {
                continue;
            }
            match rollup.rollup_domain(&u, &d) {
                Ok(s) => {
                    inc(&mut tot, "closed", s.closed);
                    inc(&mut tot, "folded", s.folded);
                    inc(&mut tot, "purged_visits", s.purged_visits);
                    inc(&mut tot, "purged_events", s.purged_events);
                    inc(&mut tot, "pruned_visitor_day", s.pruned_visitor_day);
                    inc(&mut tot, "vacuum", s.vacuum);
                    inc(&mut tot, "domains", 1);
                }
                Err(e) => {
                    inc(&mut tot, "errors", 1);
                    app.log.error(format!("rollup {u}/{d} failed: {e:#}"));
                }
            }
        }
        lock.release();
    }
    match rollup.prune_salts() {
        Ok(n) => {
            tot.insert("salts_pruned".into(), Value::from(n as i64));
        }
        Err(e) => {
            inc(&mut tot, "errors", 1);
            app.log.error(format!("salt pruning failed: {e:#}"));
        }
    }
    global.release();
    let elapsed = (t0.elapsed().as_secs_f64() * 1000.0).round() / 1000.0;
    tot.insert("elapsed".into(), json!(elapsed));
    let g = |k: &str| tot.get(k).and_then(Value::as_i64).unwrap_or(0);
    let errors = g("errors");
    let msg = (errors > 0).then(|| format!("{errors} error(s)"));
    job.finish(
        if errors > 0 { "error" } else { "ok" },
        0,
        0,
        g("domains"),
        msg.as_deref(),
        true,
    );
    let text = format!(
        "rollup of {} domain(s): {} visit(s) closed, {} row(s) folded, {} visit(s) and {} event(s) purged, {} vacuum(s), {} error(s) in {:.2}s",
        g("domains"),
        g("closed"),
        g("folded"),
        g("purged_visits"),
        g("purged_events"),
        g("vacuum"),
        g("errors"),
        elapsed
    );
    ctx.result(&Value::Object(tot), || text);
    Ok(exit::OK)
}
