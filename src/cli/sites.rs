//! `webpulse sites [--json]`: what discovery found — every site with its source
//! (`config`, `apache`, `nginx`; `cpanel` on cPanel servers), owner, aliases,
//! log files (exists, readable, format, first line parsed?) and archives, plus
//! the warnings that explain why a site is not collected.

use std::io::Read;
use std::path::Path;

use anyhow::Result;
use serde_json::{Value, json};

use super::{Ctx, Kind, RawArgs};
use crate::engine::discovery::Mode;
use crate::engine::parser::LogParser;
use crate::engine::standalone::{self, FormatKind, Site};
use crate::error::exit;

/// One-line description.
pub const SUMMARY: &str = "show the sites discovery found (config, Apache, nginx) and their logs";

/// `--help` text.
pub const USAGE: &str = "Usage: webpulse sites [--json]

Lists what discovery found: on a standalone server the sites of the `sites`
array of config.json, of the Apache configuration (/etc/httpd, /etc/apache2)
and of the nginx configuration (/etc/nginx/nginx.conf), in that order of
precedence; on a cPanel server the accounts' domains. For every log file:
whether it exists and is readable, its declared format and whether its first
line is a combined-format line. Sites that are found but not collected are
listed with the reason. Run it as root to check the log files.";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {}

/// First line of a log: `ok`, `unparseable`, `empty`; `None` when unreadable.
pub fn first_line_check(path: &Path, now: i64) -> (bool, Option<&'static str>) {
    let Ok(f) = crate::util::fs::open_nonblocking(path) else {
        return (false, None);
    };
    if !f.metadata().map(|m| m.is_file()).unwrap_or(false) {
        return (false, None);
    }
    let mut buf = Vec::new();
    if f.take(65536).read_to_end(&mut buf).is_err() {
        return (false, None);
    }
    let line = buf.split(|&b| b == b'\n').next().unwrap_or(&[]);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() {
        return (true, Some("empty"));
    }
    let mut p = LogParser::new();
    p.set_window(now);
    let mut scratch = Vec::new();
    (
        true,
        Some(if p.parse(line, &mut scratch).is_some() {
            "ok"
        } else {
            "unparseable"
        }),
    )
}

fn log_json(path: &Path, https: bool, format: &str, kind: FormatKind, now: i64) -> Value {
    let exists = standalone::exists(path);
    let (readable, first) = if exists == Some(true) {
        first_line_check(path, now)
    } else {
        (false, None)
    };
    json!({
        "path": path.display().to_string(),
        "https": https,
        "format": format,
        "compatible": kind.as_str(),
        "exists": exists,
        "readable": readable,
        "first_line": first,
    })
}

fn site_json(site: &Site, collected: bool, live: &[std::path::PathBuf], now: i64) -> Value {
    let archives = standalone::site_archives(site, live);
    let pattern = match &site.archives {
        Some(p) => p.clone(),
        None => site
            .logs
            .iter()
            .map(|l| format!("{}-*, {}.N", l.path.display(), l.path.display()))
            .collect::<Vec<_>>()
            .join("; "),
    };
    json!({
        "source": site.source.as_str(),
        "domain": site.domain,
        "aliases": site.aliases,
        "owner": site.owner,
        "collected": collected,
        "origin": site.origin,
        "logs": site.logs.iter().map(|l| log_json(&l.path, l.https, &l.format, l.kind, now)).collect::<Vec<_>>(),
        "archives": {"pattern": pattern, "configured": site.archives.is_some(), "found": archives.len()},
        "warnings": site.warnings,
    })
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    if let Some(p) = raw.positional().first() {
        return Err(super::usage(format!("unexpected argument {p}")));
    }
    let app = ctx.app()?;
    let now = app.now();
    let provider = app.provider();
    let mut data = json!({
        "mode": provider.mode().as_str(),
        "hostname": standalone::machine_hostname(),
        "sites": [],
        "warnings": [],
    });
    match provider.standalone() {
        Some(d) => {
            let live: Vec<std::path::PathBuf> = d
                .sites
                .iter()
                .chain(d.ignored.iter())
                .flat_map(|s| s.logs.iter().map(|l| l.path.clone()))
                .collect();
            let mut sites: Vec<Value> = d
                .sites
                .iter()
                .map(|s| site_json(s, true, &live, now))
                .collect();
            sites.extend(d.ignored.iter().map(|s| site_json(s, false, &live, now)));
            data["sites"] = Value::Array(sites);
            data["warnings"] = json!(d.warnings);
            data["config_files"] = json!(
                d.files
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
            );
        }
        None => {
            let mut sites = Vec::new();
            for (user, info) in provider.all() {
                for (domain, kind) in &info.kinds {
                    let logs: Vec<Value> = provider
                        .live_logs(&user, domain)
                        .iter()
                        .map(|l| log_json(&l.path, l.https, "combined", FormatKind::Combined, now))
                        .collect();
                    let mut warnings = Vec::new();
                    if logs.is_empty() {
                        warnings.push("no log file in the domlog directories (yet)".to_string());
                    }
                    sites.push(json!({
                        "source": "cpanel", "domain": domain, "aliases": [], "owner": user, "kind": kind,
                        "collected": true, "origin": Value::Null, "logs": logs,
                        "archives": {"pattern": app.paths.archive_dir(&user).0.display().to_string(), "configured": false, "found": Value::Null},
                        "warnings": warnings,
                    }));
                }
            }
            data["sites"] = Value::Array(sites);
        }
    }
    ctx.result(&data, || human(&data));
    Ok(exit::OK)
}

fn human(d: &Value) -> String {
    let mut out: Vec<String> = Vec::new();
    let mode = d["mode"].as_str().unwrap_or("");
    out.push(format!(
        "Mode: {mode}{}",
        if mode == Mode::Standalone.as_str() {
            format!(
                " (default virtual host: {})",
                d["hostname"].as_str().unwrap_or("")
            )
        } else {
            String::new()
        }
    ));
    let empty = Vec::new();
    let sites = d["sites"].as_array().unwrap_or(&empty);
    let mut collected = 0;
    let mut ignored = 0;
    for (pass, title) in [
        (true, "Collected sites"),
        (false, "Found but not collected"),
    ] {
        let list: Vec<&Value> = sites
            .iter()
            .filter(|s| s["collected"].as_bool() == Some(pass))
            .collect();
        if list.is_empty() {
            continue;
        }
        if pass {
            collected = list.len();
        } else {
            ignored = list.len();
        }
        out.push(String::new());
        out.push(format!("{title}:"));
        for s in list {
            out.push(format!(
                "  {}  [{}, owner {}]{}",
                s["domain"].as_str().unwrap_or(""),
                s["source"].as_str().unwrap_or(""),
                s["owner"].as_str().unwrap_or(""),
                s["origin"]
                    .as_str()
                    .map(|o| format!("  {o}"))
                    .unwrap_or_default()
            ));
            let aliases: Vec<&str> = s["aliases"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if !aliases.is_empty() {
                out.push(format!("      aliases   {}", aliases.join(" ")));
            }
            for l in s["logs"].as_array().unwrap_or(&empty) {
                let state = match (l["exists"].as_bool(), l["readable"].as_bool()) {
                    (Some(false), _) => "missing".to_string(),
                    (None, _) => "cannot check (permission denied; run as root)".to_string(),
                    (Some(true), Some(false)) => "not readable (run as root)".to_string(),
                    _ => match l["first_line"].as_str() {
                        Some("ok") => "first line ok".to_string(),
                        Some("empty") => "empty".to_string(),
                        Some("unparseable") => {
                            "first line is not a combined-format line".to_string()
                        }
                        _ => "unknown".to_string(),
                    },
                };
                out.push(format!(
                    "      log       {}  ({}{}, {})",
                    l["path"].as_str().unwrap_or(""),
                    l["format"].as_str().unwrap_or(""),
                    if l["https"].as_bool() == Some(true) {
                        ", https"
                    } else {
                        ""
                    },
                    state
                ));
            }
            // archives: the configured pattern, or the number of rotated files found
            let a = &s["archives"];
            let found = a["found"].as_u64();
            if a["configured"].as_bool() == Some(true) {
                out.push(format!(
                    "      archives  {}  ({} found)",
                    a["pattern"].as_str().unwrap_or(""),
                    found.unwrap_or(0)
                ));
            } else if let Some(n) = found.filter(|n| *n > 0) {
                out.push(format!("      archives  {n} rotated file(s) for backfill"));
            }
            for w in s["warnings"].as_array().unwrap_or(&empty) {
                out.push(format!("      warning   {}", w.as_str().unwrap_or("")));
            }
        }
    }
    let warnings = d["warnings"].as_array().unwrap_or(&empty);
    if !warnings.is_empty() {
        out.push(String::new());
        out.push("Warnings:".into());
        for w in warnings {
            out.push(format!("  {}", w.as_str().unwrap_or("")));
        }
    }
    out.push(String::new());
    out.push(format!(
        "{collected} site(s) collected, {ignored} found but not collected"
    ));
    out.join("\n")
}
