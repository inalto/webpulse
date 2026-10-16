//! `webpulse setup [--no-sync]` (`SetupCommand`): creates config, directories and
//! registry with the right modes (idempotent).

use anyhow::Result;
use serde_json::{Value, json};

use super::{Ctx, Kind, RawArgs};
use crate::app::config_path_for;
use crate::config::FILE_DEFAULT_JSON;
use crate::engine::collector::Collector;
use crate::engine::store::DomainStore;
use crate::error::exit;
use crate::util::fs;

/// One-line description.
pub const SUMMARY: &str =
    "create config, directories and registry with the right modes (idempotent)";

/// `--help` text.
pub const USAGE: &str = "Usage: webpulse setup [--no-sync]

Creates (or repairs) the configuration file from etc/config.json.default,
the data/log/run/geoip directories, the hook flag directory (1777), the
registry database (0600) and, unless --no-sync, one data directory per
cPanel account (0750 root:<user>, with a user-owned \"user/\" sub-directory).";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[("no-sync", Kind::Flag)];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Skip creating the per-account directories
    #[arg(long = "no-sync")]
    pub no_sync: bool,
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let (cfg_path, _) = config_path_for(&ctx.global);
    let mut done: Vec<String> = Vec::new();
    if !cfg_path.is_file() {
        if let Some(dir) = cfg_path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::ensure_dir(dir, 0o755, Some("root"), Some("root"))?;
        }
        let default_file = crate::app::install_root().join("etc/config.json.default");
        let data =
            std::fs::read(&default_file).unwrap_or_else(|_| FILE_DEFAULT_JSON.as_bytes().to_vec());
        fs::write_atomic(&cfg_path, &data, 0o600)?;
        done.push(format!("created {}", cfg_path.display()));
    } else {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&cfg_path, std::fs::Permissions::from_mode(0o600));
    }
    let app = ctx.app()?;
    let p = &app.paths;
    let registry_dir = p
        .registry_db()
        .parent()
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| ".".into());
    let dirs: [(std::path::PathBuf, u32); 6] = [
        (registry_dir, 0o711),
        (p.data_dir().to_path_buf(), 0o711),
        (p.log_dir().to_path_buf(), 0o700),
        (p.run_dir().to_path_buf(), 0o711),
        (p.hook_dir(), 0o1777),
        (p.geoip_dir().to_path_buf(), 0o755),
    ];
    // only directories created here get their mode/ownership; existing ones are left
    // alone (a configured path may point at a directory the administrator manages)
    for (dir, mode) in dirs {
        if fs::ensure_dir(&dir, mode, Some("root"), Some("root"))? {
            done.push(format!("created {} ({:o})", dir.display(), mode));
        }
    }
    let reg = app.registry()?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p.registry_db(), std::fs::Permissions::from_mode(0o600));
    }
    let mut users: Vec<String> = Vec::new();
    if !raw.flag("no-sync") {
        users = Collector::new(app).sync_registry(None)?;
        for u in &users {
            if let Err(e) = DomainStore::prepare_user_dir(p, u) {
                ctx.err(&format!("cannot prepare data directory of {u}: {e}"));
            }
        }
    }
    let version = reg.meta()?.get("schema_version").cloned().flatten();
    let summary = json!({
        "config": cfg_path.display().to_string(),
        "registry": p.registry_db().display().to_string(),
        "data_dir": p.data_dir().display().to_string(),
        "accounts": users.len(),
        "registry_version": version.map(Value::from).unwrap_or(Value::Null),
        "actions": done,
    });
    ctx.result(&summary, || {
        let mut l = done.clone();
        l.push(format!(
            "setup complete: config {}, registry {}, {} account(s)",
            cfg_path.display(),
            p.registry_db().display(),
            users.len()
        ));
        l.join("\n")
    });
    Ok(exit::OK)
}
