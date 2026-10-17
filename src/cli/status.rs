//! `webpulse status [--json]` (`StatusCommand`): collection health.

use anyhow::Result;
use serde_json::{Value, json};

use super::{Ctx, Kind, RawArgs};
use crate::app::App;
use crate::error::exit;
use crate::util::{fs, lock::Lock, time};

/// One-line description.
pub const SUMMARY: &str = "collection health: paths, accounts, lag, last jobs, GeoIP";

/// `--help` text.
pub const USAGE: &str =
    "Usage: webpulse status [--json]\n\nExit code 1 when the registry cannot be opened.";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, _raw: &RawArgs) -> Result<i32> {
    run(ctx, &Args {})
}

/// Builds and prints the status.
pub fn run(ctx: &Ctx, _args: &Args) -> Result<i32> {
    let app = ctx.app()?;
    let p = &app.paths;
    let now = app.now();
    let geo_db = app.config.get("geoip.country_db").clone();
    let mut st = json!({
        "version": App::version(),
        "php": Value::Null,
        "mode": app.mode().as_str(),
        "config": {"path": app.config_path().map(|p| p.display().to_string()), "error": app.config.load_error()},
        "paths": {},
        "registry": {"ok": false, "error": Value::Null},
        "accounts": {"total": 0, "enabled": 0, "removed": 0},
        "domains": {"total": 0, "enabled": 0, "with_data": 0},
        "collector": {"last_collect_at": Value::Null, "newest_line_ts": Value::Null, "lag_seconds": Value::Null,
            "running": Lock::is_held(&p.run_dir().join("collect.lock")), "hook_flags": 0,
            "cron_installed": p.cron_file().is_file()},
        "geoip": {"enabled": crate::config::php_bool(app.config.get("geoip.enabled")), "db": geo_db, "present": false, "db_date": Value::Null},
        "jobs": [],
        "lines_total": 0,
        "data_bytes": 0,
    });
    for (k, path) in [
        ("data_dir", p.data_dir()),
        ("registry_db", p.registry_db()),
        ("log_dir", p.log_dir()),
        ("run_dir", p.run_dir()),
    ] {
        st["paths"][k] = json!({"path": path.display().to_string(), "exists": path.exists(), "writable": fs::is_writable(path)});
    }
    st["collector"]["hook_flags"] = json!(
        fs::scandir(&p.hook_dir())
            .iter()
            .filter(|f| !f.starts_with('.'))
            .count()
    );
    let mut code = exit::OK;
    let r = (|| -> Result<()> {
        let reg = app.registry()?;
        st["registry"]["ok"] = json!(true);
        let (mut total, mut enabled, mut removed) = (0, 0, 0);
        for a in reg.accounts(None, None)? {
            total += 1;
            enabled += (a.enabled && a.removed_at.is_none()) as i64;
            removed += a.removed_at.is_some() as i64;
        }
        st["accounts"] = json!({"total": total, "enabled": enabled, "removed": removed});
        let (mut dt, mut de) = (0, 0);
        for d in reg.domains(None)? {
            if d.removed_at.is_some() {
                continue;
            }
            dt += 1;
            de += d.enabled as i64;
        }
        st["domains"]["total"] = json!(dt);
        st["domains"]["enabled"] = json!(de);
        let (mut with_data, mut bytes, mut lines) = (0i64, 0i64, 0i64);
        let mut last_collect: Option<i64> = None;
        let mut newest: Option<i64> = None;
        for s in reg.summaries(None)? {
            with_data += 1;
            bytes += s.get("db_bytes").and_then(Value::as_i64).unwrap_or(0);
            lines += s.get("lines_total").and_then(Value::as_i64).unwrap_or(0);
            if let Some(lc) = s.get("last_collect_at").and_then(Value::as_i64)
                && lc > last_collect.unwrap_or(0)
            {
                last_collect = Some(lc);
            }
            if let Some(nl) = s.get("last_line_ts").and_then(Value::as_i64)
                && nl > newest.unwrap_or(0)
            {
                newest = Some(nl);
            }
        }
        st["domains"]["with_data"] = json!(with_data);
        st["data_bytes"] = json!(bytes);
        st["lines_total"] = json!(lines);
        st["collector"]["last_collect_at"] = json!(last_collect);
        st["collector"]["newest_line_ts"] = json!(newest);
        if let Some(n) = newest {
            st["collector"]["lag_seconds"] = json!((now - n).max(0));
        }
        st["jobs"] = Value::Array(reg.jobs(5)?);
        Ok(())
    })();
    if let Err(e) = r {
        st["registry"]["error"] = json!(e.to_string());
        code = exit::ERROR;
    }
    if let Some(db) = st["geoip"]["db"].as_str().map(std::path::PathBuf::from)
        && db.is_file()
    {
        st["geoip"]["present"] = json!(true);
        let epoch = app
            .geoip()
            .metadata()
            .and_then(|m| m["build_epoch"].as_i64());
        let ts = epoch.unwrap_or_else(|| {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(&db).map(|m| m.mtime()).unwrap_or(0)
        });
        st["geoip"]["db_date"] = json!(time::gm_format(ts, "%Y-%m-%d"));
    }
    let tz = time::server_tz();
    let text = || -> String {
        let t = |v: &Value| {
            v.as_i64()
                .map(|ts| time::format_ts(ts, &tz, "%Y-%m-%d %H:%M:%S"))
                .unwrap_or_else(|| "never".into())
        };
        let mut l = vec![format!(
            "WebPulse {} (Rust)",
            st["version"].as_str().unwrap_or("")
        )];
        let cfg_err = st["config"]["error"]
            .as_str()
            .map(|e| format!(" [{e}]"))
            .unwrap_or_default();
        l.push(format!(
            "config:      {}{cfg_err}",
            st["config"]["path"].as_str().unwrap_or("(defaults)")
        ));
        l.push(format!(
            "mode:        {}",
            st["mode"].as_str().unwrap_or("")
        ));
        if let Some(m) = st["paths"].as_object() {
            for (k, v) in m {
                l.push(format!(
                    "{:<12} {}{}",
                    format!("{k}:"),
                    v["path"].as_str().unwrap_or(""),
                    if v["exists"] == json!(true) {
                        ""
                    } else {
                        " [missing]"
                    }
                ));
            }
        }
        l.push(format!(
            "registry:    {}",
            if st["registry"]["ok"] == json!(true) {
                "ok".to_string()
            } else {
                format!("ERROR {}", st["registry"]["error"].as_str().unwrap_or(""))
            }
        ));
        let n = |v: &Value| v.as_i64().unwrap_or(0);
        l.push(format!(
            "accounts:    {} ({} enabled, {} removed)",
            n(&st["accounts"]["total"]),
            n(&st["accounts"]["enabled"]),
            n(&st["accounts"]["removed"])
        ));
        l.push(format!(
            "domains:     {} ({} enabled, {} with data, {:.1} MiB)",
            n(&st["domains"]["total"]),
            n(&st["domains"]["enabled"]),
            n(&st["domains"]["with_data"]),
            n(&st["data_bytes"]) as f64 / 1048576.0
        ));
        l.push(format!(
            "last collect: {}{}",
            t(&st["collector"]["last_collect_at"]),
            if st["collector"]["running"] == json!(true) {
                " (running now)"
            } else {
                ""
            }
        ));
        l.push(format!(
            "newest line: {}{}",
            t(&st["collector"]["newest_line_ts"]),
            st["collector"]["lag_seconds"]
                .as_i64()
                .map(|s| format!(" (lag {s}s)"))
                .unwrap_or_default()
        ));
        l.push(format!(
            "cron:        {}, hook flags pending: {}",
            if st["collector"]["cron_installed"] == json!(true) {
                "installed"
            } else {
                "NOT installed"
            },
            n(&st["collector"]["hook_flags"])
        ));
        l.push(format!(
            "geoip:       {}{}",
            if st["geoip"]["enabled"] == json!(true) {
                "enabled"
            } else {
                "disabled"
            },
            if st["geoip"]["present"] == json!(true) {
                format!(", db {}", st["geoip"]["db_date"].as_str().unwrap_or(""))
            } else {
                ", no database".into()
            }
        ));
        if let Some(jobs) = st["jobs"].as_array().filter(|j| !j.is_empty()) {
            l.push("last jobs:".into());
            for j in jobs {
                let msg = j["message"]
                    .as_str()
                    .filter(|m| !m.is_empty())
                    .map(|m| format!(" — {m}"))
                    .unwrap_or_default();
                l.push(format!(
                    "  #{} {:<8} {:<20} {:<7} {} {} lines{msg}",
                    n(&j["id"]),
                    j["type"].as_str().unwrap_or(""),
                    j["scope"].as_str().unwrap_or(""),
                    j["status"].as_str().unwrap_or(""),
                    t(&j["started_at"]),
                    n(&j["lines"])
                ));
            }
        }
        l.join("\n")
    };
    let s = text();
    ctx.result(&st, || s);
    Ok(code)
}
