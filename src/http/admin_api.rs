//! `admin.*` actions (WHM): server overview, account/domain switches,
//! settings, jobs and purge (port of `Http\AdminApi`). Root sees everything;
//! a reseller is limited to the accounts it owns and cannot change global
//! settings, run rollups or purge data.

use serde_json::{Map, Value, json};

use super::api::ApiError;
use super::cpanel;
use super::ctx::Ctx;
use super::job_runner;
use super::json as pjson;
use super::principal::Principal;
use super::request::{Request, entries, field, field_nn, is_array, is_list};
use super::resolver::{self, glob_db, glob_dirs, valid_domain, valid_user};
use super::user_settings_api::is_valid_cidr;
use crate::app::App;
use crate::engine::update;
use crate::reports::data::is_timezone_identifier;
use crate::reports::domain_data::{Arg, date_c, date_ymd, domain_db_checked, php_default_tz};
use crate::reports::{invalid, php};

/// Settings exposed to the WHM settings form (dotted keys).
pub const SETTINGS: [&str; 9] = [
    "default_enabled",
    "raw_retention_days",
    "max_rows_per_dim",
    "session_timeout_min",
    "store_ip",
    "exclude_ips",
    "exclude_paths",
    "geoip.enabled",
    "timezone",
];
/// Settings the WHM GeoIP card saves through `admin.save_settings` (not listed by
/// `admin.settings`, whose shape is the one of 0.1.0; `admin.geoip` reports them).
pub const GEOIP_SETTINGS: [&str; 2] = ["geoip.url", "geoip.city_db"];
/// Job types WHM may start.
pub const JOB_TYPES: [&str; 3] = ["collect", "backfill", "rollup"];
/// Jobs started from WHM that may run at the same time.
pub const MAX_RUNNING_JOBS: usize = 3;
/// A job still "running" after this many seconds is considered dead.
pub const JOB_STALE_SECONDS: i64 = 6 * 3600;

type R = anyhow::Result<Value>;

/// Dispatches `admin.<action>`.
pub fn handle(ctx: &Ctx, r: &Request, p: &Principal, action: &str) -> R {
    match action {
        "overview" => overview(ctx, r, p),
        "accounts" => accounts(ctx, r, p),
        "set_account" => set_account(ctx, r, p),
        "set_domain" => set_domain(ctx, r, p),
        "settings" => settings(ctx, p),
        "save_settings" => save_settings(ctx, r, p),
        "run" => run(ctx, r, p),
        "jobs" => Ok(json!({"jobs": jobs(ctx, p, 50)?})),
        "purge" => purge(ctx, r, p),
        "update" => update_info(ctx, p),
        "update_check" => update_check(ctx, p),
        "geoip" => super::admin_geoip::status(ctx, p),
        other => Err(ApiError::bad_request(&format!(
            "Unknown action \"admin.{}\"",
            php::substr(other, 40)
        ))
        .into()),
    }
}

/// Users in scope, or `None` for "all" (root).
fn scope(ctx: &Ctx, p: &Principal) -> anyhow::Result<Option<Vec<String>>> {
    if p.is_admin() {
        return Ok(None);
    }
    Ok(Some(resolver::visible_users(
        p,
        Some(ctx.registry()?),
        &ctx.paths,
    )))
}

fn int_of(m: &Map<String, Value>, k: &str) -> i64 {
    m.get(k).and_then(Value::as_i64).unwrap_or(0)
}

fn str_of(m: &Map<String, Value>, k: &str) -> String {
    m.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn iso_or_null(ts: i64) -> Value {
    if ts != 0 {
        json!(date_c(ts))
    } else {
        Value::Null
    }
}

fn overview(ctx: &Ctx, r: &Request, p: &Principal) -> R {
    // `top` (1..=500, default 10): rows of `top_domains` (the server report asks for all).
    let top_n = r.get("top").map(php::intval).unwrap_or(10).clamp(1, 500) as usize;
    let reg = ctx.registry()?;
    let users = scope(ctx, p)?;
    let sums = reg.summaries(users.as_deref())?;
    let now = ctx.now();
    let mut t = [0i64; 9];
    let mut last_collect: i64 = 0;
    let mut last_line: i64 = 0;
    for s in &sums {
        for (i, k) in [
            "visits_today",
            "pageviews_today",
            "visits_7d",
            "pageviews_7d",
            "bytes_7d",
            "visits_30d",
            "pageviews_30d",
            "bytes_30d",
            "bot_hits_30d",
        ]
        .iter()
        .enumerate()
        {
            t[i] += int_of(s, k);
        }
        last_collect = last_collect.max(int_of(s, "last_collect_at"));
        last_line = last_line.max(int_of(s, "last_line_ts"));
    }

    // Bot share needs human hits, which the summary does not carry.
    let (mut human, mut bots, mut partial) = (0i64, 0i64, false);
    let refs: Vec<_> = resolver::visible_domains(p, Some(reg), &ctx.paths)
        .into_iter()
        .filter(|d| d.has_data)
        .collect();
    if refs.len() <= 300 {
        let since = date_ymd(now - 29 * 86400);
        for r in &refs {
            let res = crate::reports::domain_data::open_read_only(&ctx.paths, &r.user, &r.domain).and_then(|st| {
                let rows = crate::reports::domain_data::query_rows(
                    &st.conn,
                    "SELECT COALESCE(SUM(hits),0) AS h, COALESCE(SUM(bot_hits),0) AS b FROM daily WHERE day >= ?",
                    &[Arg::S(since.clone())],
                )?;
                Ok(rows.first().map(|x| (x.i("h"), x.i("b"))).unwrap_or((0, 0)))
            });
            match res {
                Ok((h, b)) => {
                    human += h;
                    bots += b;
                }
                Err(_) => partial = true,
            }
        }
    } else {
        partial = true;
    }

    let mut top = sums.clone();
    top.sort_by_key(|s| std::cmp::Reverse(int_of(s, "visits_30d")));
    let top: Vec<Value> = top
        .iter()
        .take(top_n)
        .map(|s| {
            json!({
                "user": s.get("user"), "domain": s.get("domain"), "visits_30d": s.get("visits_30d"),
                "pageviews_30d": s.get("pageviews_30d"), "bytes_30d": s.get("bytes_30d"), "bot_hits_30d": s.get("bot_hits_30d"),
                "visits_today": s.get("visits_today"), "db_bytes": s.get("db_bytes"),
                "last_collect_at": iso_or_null(int_of(s, "last_collect_at")),
            })
        })
        .collect();

    let mut accounts = reg.accounts(
        if p.is_reseller() {
            Some(p.name.as_str())
        } else {
            None
        },
        None,
    )?;
    if let Some(u) = &users {
        accounts.retain(|a| u.contains(&str_of(a, "user")));
    }
    let enabled = accounts
        .iter()
        .filter(|a| {
            a.get("enabled") == Some(&json!(true)) && a.get("removed_at") == Some(&Value::Null)
        })
        .count();

    let geo_db = match ctx.cfg("geoip.country_db") {
        Value::String(s) => s,
        _ => String::new(),
    };
    let hooks = match ctx.cfg("paths.hooks_file") {
        Value::String(s) => s,
        _ => "/var/cpanel/hooks.yaml".into(),
    };
    let hook_installed = std::path::Path::new(&hooks).is_file()
        && std::fs::File::open(&hooks)
            .ok()
            .and_then(|f| {
                use std::io::Read;
                let mut buf = Vec::new();
                f.take(4 << 20).read_to_end(&mut buf).ok()?;
                Some(buf.windows(8).any(|w| w == b"webpulse"))
            })
            .unwrap_or(false);
    let db_date = if !geo_db.is_empty() && std::path::Path::new(&geo_db).is_file() {
        std::fs::metadata(&geo_db)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let ts = t
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                json!(date_ymd(ts))
            })
    } else {
        None
    };

    let bot_share = if human + bots > 0 {
        php::fjson(php::round(php::div(bots, human + bots).f(), 4))
    } else {
        Value::Null
    };
    Ok(json!({
        "totals": {
            "today": {"visits": t[0], "pageviews": t[1]},
            "7d": {"visits": t[2], "pageviews": t[3], "bytes": t[4]},
            "30d": {"visits": t[5], "pageviews": t[6], "bytes": t[7], "bot_hits": t[8]},
        },
        "bot_share_30d": bot_share,
        "bot_share_partial": partial,
        "top_domains": top,
        "accounts": {"total": accounts.len(), "enabled": enabled, "disabled": accounts.len() - enabled},
        "domains": {"total": sums.len(), "with_data": refs.len()},
        "disk": disk(ctx, users.as_deref()),
        "jobs": jobs(ctx, p, 8)?,
        "collector": {
            "last_collect_at": iso_or_null(last_collect),
            "lag_seconds": if last_line != 0 { json!((now - last_line).max(0)) } else { Value::Null },
            "cron_installed": ctx.paths.cron_file().is_file(),
            "hook_installed": hook_installed,
            "geoip": {
                "enabled": ctx.cfg_bool("geoip.enabled", false),
                "db_date": db_date,
            },
        },
        "version": App::version(),
        "can_edit_settings": p.is_admin(),
    }))
}

fn is_symlink(p: &std::path::Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Bytes used by domain databases (users in scope, or all) and the registry.
fn disk(ctx: &Ctx, users: Option<&[String]>) -> Value {
    let dirs: Vec<std::path::PathBuf> = match users {
        None => glob_dirs(ctx.paths.data_dir())
            .into_iter()
            .map(|(_, p)| p)
            .collect(),
        Some(u) => u
            .iter()
            .filter(|x| valid_user(x))
            .map(|x| ctx.paths.user_dir(x))
            .collect(),
    };
    let (mut bytes, mut n) = (0u64, 0u64);
    for dir in dirs {
        for (_, f) in glob_db(&dir) {
            if !is_symlink(&f) {
                bytes += std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                n += 1;
            }
        }
    }
    let reg = ctx.paths.registry_db();
    let reg_bytes = if users.is_none() && reg.is_file() {
        json!(std::fs::metadata(reg).map(|m| m.len()).unwrap_or(0))
    } else {
        Value::Null
    };
    json!({"data_bytes": bytes, "registry_bytes": reg_bytes, "databases": n})
}

fn accounts(ctx: &Ctx, r: &Request, p: &Principal) -> R {
    let reg = ctx.registry()?;
    let q = php::mb_substr(php::trim(r.get("q").unwrap_or("")), 100);
    let limit = r.get("limit").map(php::intval).unwrap_or(50).clamp(1, 500);
    // clamped like the reports' offset (an absurd value cannot overflow `offset + limit`)
    let offset = r
        .get("offset")
        .map(php::intval)
        .unwrap_or(0)
        .clamp(0, 1_000_000);
    let users = scope(ctx, p)?;

    let mut accounts = reg.accounts(
        if p.is_reseller() {
            Some(p.name.as_str())
        } else {
            None
        },
        if q.is_empty() { None } else { Some(&q) },
    )?;
    if let Some(u) = &users {
        accounts.retain(|a| u.contains(&str_of(a, "user")));
    }
    let total = accounts.len() as i64;
    let page: Vec<Map<String, Value>> = accounts
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    let names: Vec<String> = page.iter().map(|a| str_of(a, "user")).collect();

    let all_domains = reg.domains(None)?;
    let sums = reg.summaries(Some(&names))?;
    let sum_of = |u: &str, d: &str| {
        sums.iter()
            .find(|s| str_of(s, "user") == u && str_of(s, "domain") == d)
    };

    let mut out = Vec::new();
    for a in &page {
        let u = str_of(a, "user");
        let mut doms: Vec<(String, Map<String, Value>)> = Vec::new();
        for d in &all_domains {
            if str_of(d, "user") == u && d.get("removed_at") == Some(&Value::Null) {
                let dn = str_of(d, "domain");
                match doms.iter_mut().find(|(x, _)| *x == dn) {
                    Some(e) => e.1 = d.clone(),
                    None => doms.push((dn, d.clone())),
                }
            }
        }
        if valid_user(&u) {
            for (name, _) in glob_db(&ctx.paths.user_dir(&u)) {
                let dn = name[..name.len() - 3].to_string();
                if !doms.iter().any(|(x, _)| *x == dn) && valid_domain(&dn) {
                    let mut m = Map::new();
                    m.insert("domain".into(), json!(dn));
                    m.insert("kind".into(), json!("unknown"));
                    m.insert(
                        "enabled".into(),
                        a.get("enabled").cloned().unwrap_or(Value::Null),
                    );
                    m.insert("enabled_override".into(), Value::Null);
                    doms.push((dn, m));
                }
            }
        }
        let mut list: Vec<Map<String, Value>> = Vec::new();
        for (dn, d) in &doms {
            let file = domain_db_checked(&ctx.paths, &u, dn)?;
            let s = sum_of(&u, dn);
            let has = file.is_file();
            let mut m = Map::new();
            m.insert("domain".into(), json!(dn));
            m.insert(
                "kind".into(),
                json!(d.get("kind").and_then(Value::as_str).unwrap_or("unknown")),
            );
            m.insert(
                "enabled".into(),
                json!(d.get("enabled").map(truthy).unwrap_or(false)),
            );
            m.insert(
                "enabled_override".into(),
                d.get("enabled_override").cloned().unwrap_or(Value::Null),
            );
            m.insert("has_data".into(), json!(has));
            m.insert(
                "db_bytes".into(),
                json!(if has {
                    std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                }),
            );
            m.insert(
                "visits_30d".into(),
                s.map(|s| s.get("visits_30d").cloned().unwrap_or(json!(0)))
                    .unwrap_or(json!(0)),
            );
            m.insert(
                "last_collect_at".into(),
                s.map(|s| iso_or_null(int_of(s, "last_collect_at")))
                    .unwrap_or(Value::Null),
            );
            list.push(m);
        }
        list.sort_by(|x, y| {
            resolver::by_kind_then_domain(
                (&str_of(x, "kind"), &str_of(x, "domain")),
                (&str_of(y, "kind"), &str_of(y, "domain")),
            )
        });
        let db_bytes: i64 = list.iter().map(|m| int_of(m, "db_bytes")).sum();
        let visits: i64 = list.iter().map(|m| int_of(m, "visits_30d")).sum();
        out.push(json!({
            "user": u,
            "owner": a.get("owner"),
            "enabled": a.get("enabled"),
            "enabled_override": a.get("enabled_override"),
            "timezone": a.get("timezone"),
            "removed": a.get("removed_at").map(|v| !v.is_null()).unwrap_or(false),
            "db_bytes": db_bytes,
            "visits_30d": visits,
            "domains": list,
        }));
    }
    Ok(json!({
        "accounts": out,
        "total": total,
        "has_more": offset.saturating_add(limit) < total,
        "default_enabled": ctx.cfg_bool("default_enabled", true),
    }))
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !(s.is_empty() || s == "0"),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Validates `user` from a body and checks the principal's scope (404 otherwise).
fn scoped_user(p: &Principal, user: Option<&Value>) -> Result<String, ApiError> {
    match user {
        Some(Value::String(u)) if valid_user(u) && p.can_see_user(u) => Ok(u.clone()),
        _ => Err(ApiError::not_found("Unknown account")),
    }
}

/// True when `domain` is a domain of `user`: a live registry row, a domain
/// cPanel lists for the account, or an existing database.
pub fn belongs_to(ctx: &Ctx, user: &str, domain: &str) -> anyhow::Result<bool> {
    if let Ok(reg) = ctx.registry()
        && let Ok(rows) = reg.domains(Some(user))
        && rows
            .iter()
            .any(|d| str_of(d, "domain") == domain && d.get("removed_at") == Some(&Value::Null))
    {
        return Ok(true);
    }
    if cpanel::domains_of(&ctx.paths, user).contains(domain) {
        return Ok(true);
    }
    let db = domain_db_checked(&ctx.paths, user, domain)?;
    Ok(db.is_file() && !is_symlink(&db))
}

/// Validates `domain` from a body and checks it belongs to `user`.
fn owned_domain(ctx: &Ctx, user: &str, domain: Option<&Value>) -> anyhow::Result<String> {
    let d = match domain {
        Some(Value::String(d)) if valid_domain(&d.to_ascii_lowercase()) => d.to_ascii_lowercase(),
        _ => return Err(ApiError::bad_request("Invalid domain").into()),
    };
    if !belongs_to(ctx, user, &d)? {
        return Err(ApiError::not_found("Unknown domain").into());
    }
    Ok(d)
}

/// `true|false|null` (key required).
fn tri(body: &Value) -> Result<Option<bool>, ApiError> {
    match field(body, "enabled") {
        None => Err(ApiError::bad_request(
            "\"enabled\" is required (true, false or null)",
        )),
        Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(ApiError::bad_request(
            "\"enabled\" must be true, false or null",
        )),
    }
}

fn set_account(ctx: &Ctx, r: &Request, p: &Principal) -> R {
    let b = r.json()?;
    let user = scoped_user(p, field_nn(&b, "user"))?;
    let enabled = tri(&b)?;
    let tz = match field_nn(&b, "timezone") {
        None => None,
        Some(Value::String(s)) if s.is_empty() || is_timezone_identifier(s) => Some(s.clone()),
        Some(_) => return Err(ApiError::bad_request("Invalid timezone").into()),
    };
    let reg = ctx.registry()?;
    reg.set_account(
        &user,
        enabled,
        tz.as_deref(),
        cpanel::owner(&ctx.paths, &user).as_deref(),
        ctx.now(),
    )?;
    Ok(json!({"account": reg.account(&user)?}))
}

fn set_domain(ctx: &Ctx, r: &Request, p: &Principal) -> R {
    let b = r.json()?;
    let user = scoped_user(p, field_nn(&b, "user"))?;
    let domain = owned_domain(ctx, &user, field_nn(&b, "domain"))?;
    let enabled = tri(&b)?;
    let reg = ctx.registry()?;
    reg.set_domain(
        &user,
        &domain,
        enabled,
        cpanel::owner(&ctx.paths, &user).as_deref(),
        ctx.now(),
    )?;
    let row = reg
        .domains(Some(&user))?
        .into_iter()
        .find(|d| str_of(d, "domain") == domain);
    Ok(json!({"domain": row}))
}

fn settings(ctx: &Ctx, p: &Principal) -> R {
    if !p.is_admin() {
        return Err(
            ApiError::forbidden("Only the server administrator can see global settings").into(),
        );
    }
    let mut values = Map::new();
    for k in SETTINGS {
        values.insert(k.into(), ctx.cfg(k));
    }
    let over = ctx.overridden_keys();
    let overridden: Vec<&str> = SETTINGS
        .iter()
        .copied()
        .filter(|k| over.iter().any(|o| o == k))
        .collect();
    Ok(json!({
        "settings": values,
        "overridden": overridden,
        "editable": p.is_admin(),
        "server_timezone": php_default_tz().name(),
    }))
}

/// `Config::validate()` of PHP 0.1.0 for the WHM-editable keys.
pub fn validate_setting(key: &str, v: &Value) -> anyhow::Result<Value> {
    let int = |min: i64, max: i64| -> anyhow::Result<Value> {
        let n = match v {
            Value::String(s)
                if !s.is_empty()
                    && s.strip_prefix('-')
                        .unwrap_or(s)
                        .bytes()
                        .all(|c| c.is_ascii_digit())
                    && s != "-" =>
            {
                Some(php::intval(s))
            }
            Value::Number(n) => n.as_i64(),
            _ => None,
        };
        match n {
            Some(n) if (min..=max).contains(&n) => Ok(json!(n)),
            _ => Err(invalid(format!(
                "{key} must be an integer between {min} and {max}"
            ))),
        }
    };
    let boolean = || -> anyhow::Result<Value> {
        match v {
            Value::Bool(b) => Ok(json!(b)),
            Value::String(s) if ["1", "true", "on", "yes"].contains(&s.as_str()) => Ok(json!(true)),
            Value::String(s) if ["0", "false", "off", "no"].contains(&s.as_str()) => {
                Ok(json!(false))
            }
            Value::Number(n) if n.as_i64() == Some(1) => Ok(json!(true)),
            Value::Number(n) if n.as_i64() == Some(0) => Ok(json!(false)),
            _ => Err(invalid(format!("{key} must be a boolean"))),
        }
    };
    let list = |max: usize, ok: &dyn Fn(&str) -> bool, what: &str| -> anyhow::Result<Value> {
        if !is_array(v) || !is_list(v) {
            return Err(invalid(format!("{key} must be a list")));
        }
        let items = entries(v);
        if items.len() > max {
            return Err(invalid(format!("{key} accepts at most {max} entries")));
        }
        let mut out: Vec<String> = Vec::new();
        for (_, e) in items {
            match e {
                Value::String(s) if ok(php::trim(s)) => {
                    let t = php::trim(s).to_string();
                    if !out.contains(&t) {
                        out.push(t);
                    }
                }
                other => {
                    return Err(invalid(format!(
                        "{key}: invalid {what} {}",
                        pjson::encode(other, pjson::DEFAULT)
                    )));
                }
            }
        }
        Ok(json!(out))
    };
    let path_prefix =
        |p: &str| p.starts_with('/') && p.len() <= 255 && !p.bytes().any(|c| c < 0x20);
    match key {
        "default_enabled" | "store_ip" | "count_head_requests" | "geoip.enabled" => boolean(),
        "raw_retention_days" => int(1, 3650),
        "max_rows_per_dim" => int(10, 100_000),
        "session_timeout_min" => int(5, 240),
        "timezone" => match v {
            Value::Null => Ok(Value::Null),
            Value::String(s) if s.is_empty() || s == "null" => Ok(Value::Null),
            Value::String(s) if is_timezone_identifier(s) => Ok(json!(s)),
            _ => Err(invalid(
                "timezone must be a valid identifier such as Europe/Rome",
            )),
        },
        "exclude_ips" => list(500, &|s| is_valid_cidr(s), "IP/CIDR"),
        "exclude_paths" | "static_prefixes" => list(200, &path_prefix, "path prefix"),
        other => Err(invalid(format!(
            "setting '{other}' cannot be changed at run time (edit config.json)"
        ))),
    }
}

fn save_settings(ctx: &Ctx, r: &Request, p: &Principal) -> R {
    if !p.is_admin() {
        return Err(ApiError::forbidden(
            "Only the server administrator can change global settings",
        )
        .into());
    }
    let b = r.json()?;
    let src = match field(&b, "settings") {
        Some(s) if is_array(s) => s,
        _ => &b,
    };
    let mut input: Vec<(String, Value)> = entries(src)
        .into_iter()
        .map(|(k, v)| (k, v.clone()))
        .collect();
    if let Some((_, geo)) = input.iter().find(|(k, _)| k == "geoip")
        && is_array(geo)
        && let Some(en) = field(geo, "enabled")
    {
        let en = en.clone();
        match input.iter_mut().find(|(k, _)| k == "geoip.enabled") {
            Some(e) => e.1 = en,
            None => input.push(("geoip.enabled".into(), en)),
        }
    }
    input.retain(|(k, _)| k != "geoip");
    let mut clean = Vec::new();
    for (k, v) in &input {
        if GEOIP_SETTINGS.contains(&k.as_str()) {
            let v = ctx
                .app
                .config
                .validate_here(k, v)
                .map_err(|e| invalid(e.to_string()))?;
            clean.push((k.clone(), v));
            continue;
        }
        if !SETTINGS.contains(&k.as_str()) {
            return Err(ApiError::bad_request(&format!(
                "Setting \"{}\" cannot be changed here",
                php::substr(k, 40)
            ))
            .into());
        }
        clean.push((k.clone(), validate_setting(k, v)?));
    }
    let reg = ctx.registry()?;
    for (k, v) in &clean {
        if GEOIP_SETTINGS.contains(&k.as_str()) {
            reg.set_setting(k, v)?;
        } else {
            reg.set_setting(k, &validate_setting(k, v)?)?;
        }
    }
    ctx.reload_overrides();
    settings(ctx, p)
}

fn run(ctx: &Ctx, r: &Request, p: &Principal) -> R {
    let b = r.json()?;
    if matches!(field_nn(&b, "type"), Some(Value::String(t)) if t == "upgrade") {
        return upgrade(ctx, p);
    }
    if matches!(field_nn(&b, "type"), Some(Value::String(t)) if t == super::admin_geoip::JOB) {
        return super::admin_geoip::run_update(ctx, p);
    }
    let kind = match field_nn(&b, "type") {
        Some(Value::String(t)) if JOB_TYPES.contains(&t.as_str()) => t.clone(),
        _ => {
            return Err(ApiError::bad_request(&format!(
                "\"type\" must be one of {}",
                JOB_TYPES.join(", ")
            ))
            .into());
        }
    };
    let user = match field_nn(&b, "user") {
        Some(Value::String(s)) if s.is_empty() => None,
        None => None,
        Some(u) => Some(scoped_user(p, Some(u))?),
    };
    let domain = match field_nn(&b, "domain") {
        Some(Value::String(s)) if s.is_empty() => None,
        None => None,
        Some(d) => {
            let ok = matches!(d, Value::String(s) if valid_domain(&s.to_ascii_lowercase()));
            match (&user, ok) {
                (Some(u), true) => Some(owned_domain(ctx, u, Some(d))?),
                _ => {
                    return Err(
                        ApiError::bad_request("\"domain\" requires a valid \"user\"").into(),
                    );
                }
            }
        }
    };
    if kind == "rollup" && !p.is_admin() {
        return Err(ApiError::forbidden("Only the server administrator can run the rollup").into());
    }
    if kind == "backfill" && user.is_none() {
        return Err(ApiError::bad_request("Backfill needs an account (\"user\")").into());
    }
    if !p.is_admin() && user.is_none() {
        return Err(ApiError::bad_request("Pick one of your accounts").into());
    }
    let scope = match (&user, &domain) {
        (None, _) => "all".to_string(),
        (Some(u), None) => u.clone(),
        (Some(u), Some(d)) => format!("{u}/{d}"),
    };
    let reg = ctx.registry()?;
    let now = ctx.now();
    let job_id = reg.start_job_limited(
        &kind,
        &scope,
        now - JOB_STALE_SECONDS,
        MAX_RUNNING_JOBS,
        now,
    )?;
    let mut args = vec![
        kind.clone(),
        "--job".into(),
        job_id.to_string(),
        "--quiet".into(),
    ];
    if let Some(u) = &user {
        args.extend(["--user".into(), u.clone()]);
    }
    if let Some(d) = &domain {
        args.extend(["--domain".into(), d.clone()]);
    }
    let cfg = ctx.config_path().filter(|c| c.is_file());
    if let Some(c) = &cfg {
        args.extend(["--config".into(), c.to_string_lossy().into_owned()]);
    }
    if job_runner::spawn(ctx, &args, cfg.as_deref()).is_none() {
        let _ = reg.finish_job(
            job_id,
            "error",
            Some("could not start the job process"),
            now,
        );
        return Err(ApiError::new(500, "spawn_failed", "Could not start the job").into());
    }
    Ok(json!({"job_id": job_id}))
}

/// Last jobs (resellers only see jobs of their accounts and global ones).
fn jobs(ctx: &Ctx, p: &Principal, limit: usize) -> anyhow::Result<Vec<Value>> {
    let mut out = Vec::new();
    for mut j in ctx
        .registry()?
        .jobs(if p.is_admin() { limit as i64 } else { 200 })?
    {
        let scope = j
            .get("scope")
            .and_then(php::scalar_string)
            .unwrap_or_default();
        if !p.is_admin()
            && scope != "all"
            && !scope.is_empty()
            && !p.can_see_user(scope.split('/').next().unwrap_or(""))
        {
            continue;
        }
        let started = int_of(&j, "started_at");
        let finished = int_of(&j, "finished_at");
        j.insert("started_at_iso".into(), iso_or_null(started));
        j.insert("finished_at_iso".into(), iso_or_null(finished));
        out.push(Value::Object(j));
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

fn purge(ctx: &Ctx, r: &Request, p: &Principal) -> R {
    if !p.is_admin() {
        return Err(ApiError::forbidden("Only the server administrator can purge data").into());
    }
    let b = r.json()?;
    if field(&b, "confirm") != Some(&json!("DELETE")) {
        return Err(ApiError::bad_request("Type DELETE to confirm").into());
    }
    let user = scoped_user(p, field_nn(&b, "user"))?;
    let data_dir = std::fs::canonicalize(ctx.paths.data_dir()).ok();
    let (domain, files) = match field_nn(&b, "domain") {
        Some(Value::String(s)) if s.is_empty() => (
            None,
            glob_db(&ctx.paths.user_dir(&user))
                .into_iter()
                .map(|(_, p)| p)
                .collect(),
        ),
        None => (
            None,
            glob_db(&ctx.paths.user_dir(&user))
                .into_iter()
                .map(|(_, p)| p)
                .collect::<Vec<_>>(),
        ),
        Some(d) => {
            let d = owned_domain(ctx, &user, Some(d))?;
            let f = domain_db_checked(&ctx.paths, &user, &d)?;
            (Some(d), vec![f])
        }
    };
    let mut deleted = Vec::new();
    for f in files {
        if !f.is_file() || is_symlink(&f) {
            continue;
        }
        let (Some(dd), Ok(real)) = (&data_dir, std::fs::canonicalize(&f)) else {
            continue;
        };
        if !real.starts_with(dd) || real == *dd {
            continue;
        }
        if std::fs::remove_file(&real).is_ok() {
            let mut j = real.as_os_str().to_owned();
            j.push("-journal");
            let _ = std::fs::remove_file(j);
            let name = real
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            deleted.push(name.strip_suffix(".db").unwrap_or(&name).to_string());
        }
    }
    if let Ok(reg) = ctx.registry() {
        let _ = reg.delete_summary(&user, domain.as_deref());
    }
    Ok(json!({"deleted": deleted}))
}

// ------------------------------------------------------------------ updates

/// An upgrade job still "running" after this many seconds no longer blocks a new one.
pub const UPGRADE_STALE_SECONDS: i64 = 3600;

fn stored_update(ctx: &Ctx) -> Option<Value> {
    ctx.registry()
        .ok()?
        .settings()
        .ok()?
        .into_iter()
        .find(|(k, _)| k == update::SETTING_KEY)
        .map(|(_, v)| v)
}

fn update_override_allowed(ctx: &Ctx) -> bool {
    update::override_allowed(
        ctx.app.is_root(),
        ctx.app
            .config_path()
            .is_some_and(crate::app::is_production_config),
    )
}

/// The newest upgrade job (root only), `None` when there is none.
fn last_upgrade_job(ctx: &Ctx) -> anyhow::Result<Option<Map<String, Value>>> {
    Ok(ctx
        .registry()?
        .jobs(100)?
        .into_iter()
        .find(|j| str_of(j, "type") == "upgrade"))
}

/// `admin.update`: the stored release check, re-evaluated against this binary.
fn update_info(ctx: &Ctx, p: &Principal) -> R {
    let prev = stored_update(ctx);
    let mut v = update::view(prev.as_ref(), App::version());
    let at = update::checked_at(prev.as_ref());
    let install_sh = p.is_admin() && update::installer(ctx.paths.root()).is_some();
    let newer = v["is_newer"] == json!(true);
    let tag_ok = v["tag"].as_str().is_some_and(update::valid_tag);
    v["checked_at_iso"] = iso_or_null(at);
    v["check_enabled"] = json!(update::check_enabled(&ctx.app.config));
    v["install_sh"] = json!(install_sh);
    v["can_upgrade"] = json!(p.is_admin() && newer && tag_ok && install_sh);
    v["upgrade_job"] = if p.is_admin() {
        match last_upgrade_job(ctx)? {
            Some(mut j) => {
                let started = int_of(&j, "started_at");
                let finished = int_of(&j, "finished_at");
                j.insert("started_at_iso".into(), iso_or_null(started));
                j.insert("finished_at_iso".into(), iso_or_null(finished));
                Value::Object(j)
            }
            None => Value::Null,
        }
    } else {
        Value::Null
    };
    Ok(v)
}

/// `admin.update_check` (root, POST): asks GitHub again unless the last check is
/// less than an hour old (then the cached status comes back with `throttled`).
fn update_check(ctx: &Ctx, p: &Principal) -> R {
    if !p.is_admin() {
        return Err(
            ApiError::forbidden("Only the server administrator can check for updates").into(),
        );
    }
    let prev = stored_update(ctx);
    let now = ctx.now();
    let throttled = !update::is_stale(prev.as_ref(), now, update::THROTTLE_SECONDS);
    if !throttled {
        let url = update::source_url(&ctx.app.config, update_override_allowed(ctx));
        let (st, _) = update::check(url, prev.as_ref(), App::version(), now, true);
        ctx.registry()?.set_setting(update::SETTING_KEY, &st)?;
        if let Some(e) = st["error"].as_str() {
            ctx.app.log.warn(format!("update check failed: {e}"));
        }
    }
    let mut v = update_info(ctx, p)?;
    v["throttled"] = json!(throttled);
    Ok(v)
}

/// `admin.run {type: "upgrade"}` (root): starts `webpulse dev run-upgrade`, which runs
/// `install.sh --update --ref <tag> -y` and finishes the job from its exit status.
fn upgrade(ctx: &Ctx, p: &Principal) -> R {
    if !p.is_admin() {
        return Err(
            ApiError::forbidden("Only the server administrator can upgrade WebPulse").into(),
        );
    }
    let st = update::view(stored_update(ctx).as_ref(), App::version());
    let tag = st["tag"].as_str().unwrap_or("").to_string();
    if st["is_newer"] != json!(true) || !update::valid_tag(&tag) {
        return Err(
            ApiError::bad_request("No newer release is known; check for updates first").into(),
        );
    }
    if update::installer(ctx.paths.root()).is_none() {
        return Err(ApiError::new(
            409,
            "conflict",
            "The installer (install.sh) was not found in the installation directory",
        )
        .into());
    }
    let reg = ctx.registry()?;
    let now = ctx.now();
    if let Some(j) = reg.jobs(100)?.into_iter().find(|j| {
        str_of(j, "type") == "upgrade"
            && str_of(j, "status") == "running"
            && int_of(j, "started_at") >= now - UPGRADE_STALE_SECONDS
    }) {
        return Err(super::registry::Conflict(format!(
            "An upgrade is already running (#{})",
            int_of(&j, "id")
        ))
        .into());
    }
    let job_id = reg.start_job_limited(
        "upgrade",
        &tag,
        now - JOB_STALE_SECONDS,
        MAX_RUNNING_JOBS,
        now,
    )?;
    let mut args = vec![
        "dev".to_string(),
        "run-upgrade".into(),
        "--job".into(),
        job_id.to_string(),
        "--tag".into(),
        tag.clone(),
        "--quiet".into(),
    ];
    let repo = update::repo_of(&ctx.app.config);
    if repo != update::DEFAULT_REPO && update::valid_repo(&repo) {
        args.extend(["--repo".into(), repo]);
    }
    let cfg = ctx.config_path().filter(|c| c.is_file());
    if let Some(c) = &cfg {
        args.extend(["--config".into(), c.to_string_lossy().into_owned()]);
    }
    if job_runner::spawn(ctx, &args, cfg.as_deref()).is_none() {
        let _ = reg.finish_job(
            job_id,
            "error",
            Some("could not start the upgrade process"),
            now,
        );
        return Err(ApiError::new(500, "spawn_failed", "Could not start the upgrade").into());
    }
    ctx.app
        .log
        .info(format!("upgrade to {tag} started from WHM (job {job_id})"));
    Ok(json!({"job_id": job_id, "tag": tag}))
}
