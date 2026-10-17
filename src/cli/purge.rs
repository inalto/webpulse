//! `webpulse purge --user U [--domain D] --yes` (`PurgeCommand`).

use anyhow::Result;
use serde_json::json;

use super::collect::USER_LOCK_WAIT;
use super::{Ctx, Kind, RawArgs, usage};
use crate::error::exit;
use crate::log::Level;
use crate::paths::{is_valid_domain, is_valid_user};
use crate::util::fs;

/// One-line description.
pub const SUMMARY: &str = "delete the statistics of an account or domain (irreversible)";

/// `--help` text.
pub const USAGE: &str = "Usage: webpulse purge --user USER [--domain DOMAIN] --yes

Deletes the domain database(s). Without --domain every database of the
account is deleted; when the account no longer exists in cPanel its data
directory and registry rows are removed too. --yes is mandatory.";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[
    ("user", Kind::Value),
    ("domain", Kind::Value),
    ("yes", Kind::Flag),
];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Account
    #[arg(long)]
    pub user: Option<String>,
    /// Only this domain
    #[arg(long)]
    pub domain: Option<String>,
    /// Confirm the deletion
    #[arg(long)]
    pub yes: bool,
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let user = match raw.opt("user") {
        Some(u) if is_valid_user(u) => u.to_string(),
        _ => return Err(usage("--user USER is required")),
    };
    let domain = raw.opt("domain").map(|d| d.to_ascii_lowercase());
    if let Some(d) = &domain
        && !is_valid_domain(d)
    {
        return Err(usage(format!("invalid domain '{d}'")));
    }
    if !raw.flag("yes") {
        return Err(usage("refusing to delete data without --yes"));
    }
    let app = ctx.app()?;
    let Some(mut lock) = ctx.user_lock(&user, USER_LOCK_WAIT)? else {
        ctx.err(&format!("account {user} is locked by another process"));
        return Ok(exit::ERROR);
    };
    let reg = app.registry()?;
    let dir = app.paths.try_user_dir(&user)?;
    let files: Vec<std::path::PathBuf> = match &domain {
        Some(d) => vec![app.paths.try_domain_db(&user, d)?],
        None => fs::scandir(&dir)
            .into_iter()
            .filter(|n| !n.starts_with('.') && n.ends_with(".db"))
            .map(|n| dir.join(n))
            .collect(),
    };
    let mut deleted: Vec<String> = Vec::new();
    for f in files {
        if f.is_file() || fs::is_link(&f) {
            let _ = std::fs::remove_file(&f);
            let _ = std::fs::remove_file(format!("{}-journal", f.display()));
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            deleted.push(name.strip_suffix(".db").unwrap_or(&name).to_string());
        }
    }
    match &domain {
        Some(d) => reg.delete_summary(&user, Some(d))?,
        None => {
            reg.delete_summary(&user, None)?;
            if let Some(acc) = reg.account(&user)?
                && acc.removed_at.is_some()
                && !app.provider().exists(&user)
            {
                fs::remove_tree(&dir, None);
                reg.forget_account(&user)?;
            }
        }
    }
    lock.release();
    let what = match &domain {
        Some(d) => format!("purged {user}/{d}"),
        None => format!("purged {user}"),
    };
    app.log
        .log_ctx(Level::Info, &what, &json!({"deleted": deleted}));
    ctx.result(&json!({"user": user, "deleted": deleted}), || {
        if deleted.is_empty() {
            "nothing to delete".into()
        } else {
            format!("deleted: {}", deleted.join(", "))
        }
    });
    Ok(exit::OK)
}
