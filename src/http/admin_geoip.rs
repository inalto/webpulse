//! `admin.geoip` (status of the GeoIP databases, admin and resellers) and
//! `admin.run {type: "geoip-update"}` (root): the WHM GeoIP card.

use std::path::Path;

use serde_json::{Map, Value, json};

use super::api::ApiError;
use super::ctx::Ctx;
use super::job_runner;
use super::principal::Principal;
use super::resolver;
use crate::cli::geoip_update::{DEFAULT_URL, JOB_TYPE};
use crate::engine::classify::geoip;
use crate::reports::domain_data::{self, Arg, date_c, date_ymd};
use crate::util::time;

/// `admin.run` type of the download.
pub const JOB: &str = JOB_TYPE;
/// Where the free country database comes from, and its licence.
pub const PROVIDER_URL: &str = "https://db-ip.com/db/download/ip-to-country-lite";
/// Licence of the DB-IP lite databases.
pub const LICENSE: &str = "CC BY 4.0";
/// Attribution the licence asks for wherever the data is shown.
pub const ATTRIBUTION: &str = "IP Geolocation by DB-IP";
/// Link of the attribution.
pub const ATTRIBUTION_URL: &str = "https://db-ip.com";
/// Domains (by visits, last 30 days) sampled for the known-country share.
pub const SAMPLE_DOMAINS: usize = 25;

fn iso_or_null(ts: i64) -> Value {
    if ts != 0 {
        json!(date_c(ts))
    } else {
        Value::Null
    }
}

fn mtime(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One database file: path, presence, size, mtime and MMDB metadata.
pub fn db_info(path: &str) -> Value {
    let p = Path::new(path);
    let meta = std::fs::metadata(p).ok().filter(|m| m.is_file());
    let mut v = json!({
        "path": path,
        "present": meta.is_some(),
        "bytes": meta.as_ref().map(|m| json!(m.len())).unwrap_or(Value::Null),
        "mtime": meta.as_ref().map(|m| iso_or_null(mtime(m))).unwrap_or(Value::Null),
        "metadata": Value::Null,
        "error": Value::Null,
    });
    if meta.is_some() {
        match geoip::read_metadata(p) {
            Ok(m) => {
                v["metadata"] = json!({
                    "database_type": m.database_type,
                    "build_epoch": m.build_epoch,
                    "build_date": time::gm_format(m.build_epoch as i64, "%Y-%m-%d"),
                    "ip_version": m.ip_version,
                    "node_count": m.node_count,
                    "record_size": m.record_size,
                    "languages": m.languages,
                    "description": m.description.get("en").cloned(),
                    "dbip": geoip::is_dbip(&m),
                });
            }
            Err(_) => v["error"] = json!("not a valid MaxMind DB file"),
        }
    }
    v
}

fn cfg_path(ctx: &Ctx, key: &str) -> Option<String> {
    match ctx.cfg(key) {
        Value::String(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// The download URL template in effect (`geoip.url`, else the built-in one).
pub fn url_template(ctx: &Ctx) -> String {
    match ctx.cfg("geoip.url") {
        Value::String(s) if !s.trim().is_empty() => s,
        _ => DEFAULT_URL.to_string(),
    }
}

/// True when the dashboards must show the DB-IP attribution: a DB-IP database is
/// installed and GeoIP is on (or the settings cannot be read, as in a cPanel
/// account's dashboard, where the registry is root-only).
pub fn attribution_needed(ctx: &Ctx) -> bool {
    let unknown = ctx.app.config.load_error().is_some()
        || !super::registry::access(ctx.paths.registry_db(), libc::R_OK);
    if !(unknown || ctx.cfg_bool("geoip.enabled", false)) {
        return false;
    }
    ["geoip.country_db", "geoip.city_db"].iter().any(|k| {
        cfg_path(ctx, k)
            .filter(|p| Path::new(p).is_file())
            .and_then(|p| geoip::read_metadata(Path::new(&p)).ok())
            .is_some_and(|m| geoip::is_dbip(&m))
    })
}

fn cron(ctx: &Ctx) -> Value {
    let file = ctx.paths.cron_file();
    let text = std::fs::File::open(file)
        .ok()
        .and_then(|f| {
            use std::io::Read;
            let mut buf = Vec::new();
            f.take(1 << 20).read_to_end(&mut buf).ok()?;
            Some(String::from_utf8_lossy(&buf).into_owned())
        })
        .filter(|_| file.is_file());
    let line = text.as_deref().and_then(|t| {
        t.lines()
            .map(str::trim)
            .find(|l| !l.starts_with('#') && l.contains(JOB_TYPE))
            .map(|l| l.chars().take(300).collect::<String>())
    });
    json!({
        "file": file.display().to_string(),
        "present": text.is_some(),
        "scheduled": line.is_some(),
        "line": line,
    })
}

fn last_job(ctx: &Ctx) -> anyhow::Result<Value> {
    let found = ctx
        .registry()?
        .jobs(200)?
        .into_iter()
        .find(|j| j.get("type").and_then(Value::as_str) == Some(JOB_TYPE));
    Ok(match found {
        Some(mut j) => {
            let int =
                |j: &Map<String, Value>, k: &str| j.get(k).and_then(Value::as_i64).unwrap_or(0);
            let (s, f) = (int(&j, "started_at"), int(&j, "finished_at"));
            j.insert("started_at_iso".into(), iso_or_null(s));
            j.insert("finished_at_iso".into(), iso_or_null(f));
            Value::Object(j)
        }
        None => Value::Null,
    })
}

/// Share of visits (last 30 days) whose country is known, over the busiest
/// domains in scope (`dim_daily`, dimension `country`).
fn known_country(ctx: &Ctx, p: &Principal) -> anyhow::Result<Value> {
    let reg = ctx.registry()?;
    let users = if p.is_admin() {
        None
    } else {
        Some(resolver::visible_users(p, Some(reg), &ctx.paths))
    };
    let mut sums = reg.summaries(users.as_deref())?;
    let v30 = |s: &Map<String, Value>| s.get("visits_30d").and_then(Value::as_i64).unwrap_or(0);
    sums.retain(|s| v30(s) > 0);
    sums.sort_by_key(|s| std::cmp::Reverse(v30(s)));
    let since = date_ymd(ctx.now() - 29 * 86400);
    let (mut total, mut unknown, mut n, mut partial) =
        (0i64, 0i64, 0usize, sums.len() > SAMPLE_DOMAINS);
    for s in sums.iter().take(SAMPLE_DOMAINS) {
        let user = s.get("user").and_then(Value::as_str).unwrap_or("");
        let domain = s.get("domain").and_then(Value::as_str).unwrap_or("");
        let res = domain_data::open_read_only(&ctx.paths, user, domain).and_then(|st| {
            let rows = domain_data::query_rows(
                &st.conn,
                "SELECT COALESCE(SUM(visits),0) AS t, COALESCE(SUM(CASE WHEN key = '(unknown)' THEN visits ELSE 0 END),0) AS u FROM dim_daily WHERE dim = 'country' AND day >= ?",
                &[Arg::S(since.clone())],
            )?;
            Ok(rows.first().map(|r| (r.i("t"), r.i("u"))).unwrap_or((0, 0)))
        });
        match res {
            Ok((t, u)) => {
                total += t;
                unknown += u;
                n += 1;
            }
            Err(_) => partial = true,
        }
    }
    let share = if total > 0 {
        json!(((total - unknown) as f64 / total as f64 * 10000.0).round() / 10000.0)
    } else {
        Value::Null
    };
    Ok(json!({
        "share": share,
        "visits": total,
        "known": total - unknown,
        "domains": n,
        "partial": partial,
    }))
}

/// `admin.geoip`: everything the GeoIP card shows.
pub fn status(ctx: &Ctx, p: &Principal) -> anyhow::Result<Value> {
    let country = cfg_path(ctx, "geoip.country_db").unwrap_or_default();
    let dir = ctx.app.config.geoip_dir();
    let tpl = url_template(ctx);
    let now = ctx.now();
    let (y, m) = (time::gm_format(now, "%Y"), time::gm_format(now, "%m"));
    let country_info = db_info(&country);
    let city_info = cfg_path(ctx, "geoip.city_db").map(|c| db_info(&c));
    let dbip = [Some(&country_info), city_info.as_ref()]
        .into_iter()
        .flatten()
        .any(|d| d["metadata"]["dbip"] == json!(true));
    let enabled = ctx.cfg_bool("geoip.enabled", false);
    let over = ctx.overridden_keys();
    Ok(json!({
        "enabled": enabled,
        "dir": dir.display().to_string(),
        "country": country_info,
        "city": city_info,
        "url": tpl,
        "url_preview": tpl.replace("{YYYY}", &y).replace("{MM}", &m),
        "url_default": DEFAULT_URL,
        "url_overridden": over.iter().any(|k| k == "geoip.url"),
        "cron": cron(ctx),
        "last_job": last_job(ctx)?,
        "known_country_30d": known_country(ctx, p)?,
        "attribution": if enabled && dbip { json!(ATTRIBUTION) } else { Value::Null },
        "provider": {
            "name": "DB-IP",
            "url": PROVIDER_URL,
            "license": LICENSE,
            "attribution": ATTRIBUTION,
            "attribution_url": ATTRIBUTION_URL,
        },
        "can_edit": p.is_admin(),
    }))
}

/// `admin.run {type: "geoip-update"}` (root): starts `webpulse geoip-update --force
/// --job <id>`; the download works with GeoIP off (it is switched on separately).
pub fn run_update(ctx: &Ctx, p: &Principal) -> anyhow::Result<Value> {
    if !p.is_admin() {
        return Err(ApiError::forbidden(
            "Only the server administrator can download the GeoIP database",
        )
        .into());
    }
    let reg = ctx.registry()?;
    let now = ctx.now();
    let job_id = reg.start_job_limited(
        JOB_TYPE,
        "all",
        now - super::admin_api::JOB_STALE_SECONDS,
        super::admin_api::MAX_RUNNING_JOBS,
        now,
    )?;
    let mut args = vec![
        JOB_TYPE.to_string(),
        "--force".into(),
        "--job".into(),
        job_id.to_string(),
        "--quiet".into(),
    ];
    let cfg = ctx.config_path().filter(|c| c.is_file());
    if let Some(c) = &cfg {
        args.extend(["--config".into(), c.to_string_lossy().into_owned()]);
    }
    if job_runner::spawn(ctx, &args, cfg.as_deref()).is_none() {
        let _ = reg.finish_job(
            job_id,
            "error",
            Some("could not start the download process"),
            now,
        );
        return Err(ApiError::new(500, "spawn_failed", "Could not start the download").into());
    }
    ctx.app
        .log
        .info(format!("GeoIP download started from WHM (job {job_id})"));
    Ok(json!({"job_id": job_id, "enabled": ctx.cfg_bool("geoip.enabled", false)}))
}
