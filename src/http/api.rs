//! The JSON API (docs/API.md), port of `Http\Api`: one entry point shared by
//! the cPanel shim, the WHM shim and the dev server; the caller supplies the
//! principal. `dispatch` never fails and never panics.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

use serde_json::{Value, json};

use super::ctx::Ctx;
use super::principal::Principal;
use super::registry::Registry;
use super::request::Request;
use super::resolver::{self, DomainRef};
use super::response::Response;
use super::{admin_api, user_settings_api};
use crate::app::App;
use crate::error::WpError;
use crate::reports::domain_data::{DomainData, iso_in, php_default_tz};
use crate::reports::{
    Invalid, export, health, overview, php, range::Range, realtime, report, visits_log,
};

/// Actions that change state: POST + `X-WebPulse-Request: 1` required.
pub const MUTATING: [&str; 7] = [
    "save_user_settings",
    "admin.update_check",
    "admin.set_account",
    "admin.set_domain",
    "admin.save_settings",
    "admin.run",
    "admin.purge",
];

const DOMAIN_ACTIONS: [&str; 6] = [
    "overview",
    "report",
    "realtime",
    "visits_log",
    "health",
    "export",
];

/// An error that maps to a JSON error response.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ApiError {
    /// HTTP status.
    pub status: u16,
    /// Error code (`not_found`, `forbidden`, …).
    pub code: String,
    /// Human message.
    pub message: String,
}

impl ApiError {
    /// Any status/code.
    pub fn new(status: u16, code: &str, message: &str) -> ApiError {
        ApiError {
            status,
            code: code.into(),
            message: message.into(),
        }
    }
    /// 404 `not_found`.
    pub fn not_found(message: &str) -> ApiError {
        ApiError::new(404, "not_found", message)
    }
    /// 403 `forbidden`.
    pub fn forbidden(message: &str) -> ApiError {
        ApiError::new(403, "forbidden", message)
    }
    /// 400 `bad_request`.
    pub fn bad_request(message: &str) -> ApiError {
        ApiError::new(400, "bad_request", message)
    }
}

/// Result of an action: JSON data or a complete response (CSV export).
pub enum Outcome {
    /// Wrapped in the `{"ok":true,"data":…}` envelope.
    Data(Value),
    /// Sent as is.
    Raw(Response),
}

/// Handles one API request; never fails.
pub fn dispatch(ctx: &Ctx, r: &Request, p: &Principal) -> Response {
    let t0 = Instant::now();
    let action = r.get("a").unwrap_or("").to_string();
    let res = catch_unwind(AssertUnwindSafe(|| handle(ctx, r, p, &action)));
    match res {
        Ok(Ok(Outcome::Data(v))) => Response::json(v, t0),
        Ok(Ok(Outcome::Raw(resp))) => resp,
        Ok(Err(e)) => error_response(ctx, &action, e),
        Err(_) => {
            ctx.log_error(&format!("api {action}: panic"));
            Response::error(500, "internal_error", "Internal error")
        }
    }
}

/// Maps an error to the API's error responses.
pub fn error_response(ctx: &Ctx, action: &str, e: anyhow::Error) -> Response {
    if let Some(a) = e.downcast_ref::<ApiError>() {
        return Response::error(a.status, &a.code, &a.message);
    }
    if let Some(i) = e.downcast_ref::<Invalid>() {
        return Response::error(400, "bad_request", &i.0);
    }
    if let Some(c) = e.downcast_ref::<super::registry::Conflict>() {
        return Response::error(409, "conflict", &c.0);
    }
    match e.downcast_ref::<WpError>() {
        Some(WpError::NotFound(_)) => {
            return Response::error(
                404,
                "no_data",
                "No data has been collected for this domain yet",
            );
        }
        Some(WpError::SchemaTooNew { .. }) => {
            return Response::error(
                503,
                "schema_too_new",
                "The database is being upgraded; try again in a few minutes",
            );
        }
        Some(WpError::Usage(m)) => return Response::error(400, "bad_request", m),
        _ => {}
    }
    ctx.log_error(&format!("api {action}: {e:#}"));
    Response::error(500, "internal_error", "Internal error")
}

fn handle(ctx: &Ctx, r: &Request, p: &Principal, action: &str) -> anyhow::Result<Outcome> {
    if MUTATING.contains(&action) {
        if r.method != "POST" {
            return Err(
                ApiError::new(405, "method_not_allowed", "This action requires POST").into(),
            );
        }
        if r.header("X-WebPulse-Request") != Some("1") {
            return Err(ApiError::forbidden("Missing X-WebPulse-Request header").into());
        }
    } else if r.method != "GET" && r.method != "HEAD" && !action.is_empty() {
        return Err(ApiError::new(405, "method_not_allowed", "This action requires GET").into());
    }

    if DOMAIN_ACTIONS.contains(&action) {
        return domain_action(ctx, r, p, action);
    }
    if let Some(sub) = action.strip_prefix("admin.") {
        if !p.has_admin_scope() {
            return Err(ApiError::forbidden("Administrator access required").into());
        }
        return admin_api::handle(ctx, r, p, sub).map(Outcome::Data);
    }
    let data = match action {
        "domains" => json!({"domains": domains(ctx, p)}),
        "user_settings" => user_settings_api::get(ctx, r, p)?,
        "save_user_settings" => user_settings_api::save(ctx, r, p)?,
        "ping" => json!({"version": App::version(), "principal": p.to_json()}),
        "" => return Err(ApiError::bad_request("Parameter \"a\" is required").into()),
        other => {
            return Err(ApiError::bad_request(&format!(
                "Unknown action \"{}\"",
                php::substr(other, 40)
            ))
            .into());
        }
    };
    Ok(Outcome::Data(data))
}

/// The registry, or `None` when it is not readable (cPanel accounts).
pub fn registry<'c>(ctx: &'c Ctx, p: &Principal) -> Option<&'c Registry> {
    if p.is_user() {
        let file = ctx.paths.registry_db();
        if !file.is_file() || !super::registry::access(file, libc::R_OK) {
            return None;
        }
    }
    ctx.registry().ok()
}

/// Opens a visible domain read-only (404 when not visible or without data).
pub fn open(ctx: &Ctx, r: &Request, p: &Principal) -> anyhow::Result<DomainData> {
    let reference = resolver::resolve(
        p,
        registry(ctx, p),
        &ctx.paths,
        r.get("domain"),
        r.get("user"),
    )?;
    if !reference.has_data {
        return Err(ApiError::new(
            404,
            "no_data",
            "No data has been collected for this domain yet",
        )
        .into());
    }
    let retention = ctx.cfg_i64("raw_retention_days", 30);
    ctx.domain_data(&reference.user, &reference.domain, retention)
}

fn domain_action(ctx: &Ctx, r: &Request, p: &Principal, action: &str) -> anyhow::Result<Outcome> {
    let d = open(ctx, r, p)?;
    if action == "realtime" {
        return Ok(Outcome::Data(realtime::build(&d)?));
    }
    let first = d.first_day();
    let range = Range::from_query(&r.query, d.tz, d.now(), first.as_deref())?;
    let compare = matches!(r.get("compare"), Some("1") | Some("true"));
    let v = match action {
        "overview" => overview::build(&d, &range, compare)?,
        "report" => report::build(&d, &range, &report::params(&r.query, false)?)?,
        "visits_log" => visits_log::build(&d, &range, &r.query)?,
        "health" => health::build(&d, &range)?,
        "export" => {
            let format = r.get("format").unwrap_or("csv");
            if format != "csv" {
                return Err(ApiError::bad_request(&format!(
                    "Unsupported format \"{}\"",
                    php::substr(format, 10)
                ))
                .into());
            }
            let mut params = report::params(&r.query, true)?;
            params.compare = false;
            params.series = false;
            let rep = report::build(&d, &range, &params)?;
            let name = export::filename(&d.domain, &params.dim, &range);
            return Ok(Outcome::Raw(Response::new(
                200,
                "text/csv; charset=utf-8",
                export::csv(&rep),
                vec![
                    (
                        "Content-Disposition".into(),
                        format!("attachment; filename=\"{name}\""),
                    ),
                    ("Cache-Control".into(), "no-store".into()),
                    ("X-Content-Type-Options".into(), "nosniff".into()),
                ],
            )));
        }
        _ => return Err(ApiError::bad_request("Unknown action").into()),
    };
    Ok(Outcome::Data(v))
}

/// `a=domains`: visible domains with a tiny summary.
pub fn domains(ctx: &Ctx, p: &Principal) -> Vec<Value> {
    let reg = registry(ctx, p);
    let refs: Vec<DomainRef> = resolver::visible_domains(p, reg, &ctx.paths);
    let mut summaries = std::collections::HashMap::new();
    if let (Some(reg), false) = (reg, p.is_user())
        && let Ok(rows) = reg.summaries(None)
    {
        for s in rows {
            let key = format!(
                "{}/{}",
                s.get("user").and_then(Value::as_str).unwrap_or(""),
                s.get("domain").and_then(Value::as_str).unwrap_or("")
            );
            summaries.insert(key, s);
        }
    }
    let n = refs.len();
    let mut out = Vec::with_capacity(n);
    for r in &refs {
        let mut row = serde_json::Map::new();
        row.insert("domain".into(), json!(r.domain));
        row.insert("user".into(), json!(r.user));
        row.insert("kind".into(), json!(r.kind));
        row.insert("enabled".into(), json!(r.enabled.unwrap_or(true)));
        row.insert("has_data".into(), json!(r.has_data));
        row.insert("first_day".into(), Value::Null);
        row.insert("last_day".into(), Value::Null);
        row.insert("last_collect_at".into(), Value::Null);
        row.insert("visits_today".into(), json!(0));
        row.insert("pageviews_today".into(), json!(0));
        row.insert("timezone".into(), Value::Null);
        row.insert("db_bytes".into(), json!(r.db_bytes()));
        if let Some(s) = summaries.get(&format!("{}/{}", r.user, r.domain)) {
            let int = |k: &str| s.get(k).and_then(Value::as_i64).unwrap_or(0);
            row.insert("visits_today".into(), json!(int("visits_today")));
            row.insert("pageviews_today".into(), json!(int("pageviews_today")));
            let lc = int("last_collect_at");
            row.insert(
                "last_collect_at".into(),
                if lc != 0 {
                    json!(iso_in(lc, php_default_tz()))
                } else {
                    Value::Null
                },
            );
        } else if r.has_data && (n <= 200 || p.is_user()) {
            match ctx.domain_data(&r.user, &r.domain, 30).and_then(|d| {
                let today = d.today();
                let t = d.row(
                    "SELECT visits, pageviews FROM daily WHERE day = ?",
                    &[today.as_str().into()],
                )?;
                Ok((d, t))
            }) {
                Ok((d, t)) => {
                    row.insert(
                        "visits_today".into(),
                        json!(t.as_ref().map(|t| t.i("visits")).unwrap_or(0)),
                    );
                    row.insert(
                        "pageviews_today".into(),
                        json!(t.as_ref().map(|t| t.i("pageviews")).unwrap_or(0)),
                    );
                    row.insert("first_day".into(), json!(d.first_day()));
                    row.insert("last_day".into(), json!(d.last_day()));
                    row.insert(
                        "last_collect_at".into(),
                        json!(d.iso(d.meta_string("last_collect_at").as_deref())),
                    );
                    row.insert("timezone".into(), json!(d.tz.name()));
                }
                Err(_) => {
                    row.insert("has_data".into(), json!(false));
                }
            }
        }
        out.push(Value::Object(row));
    }
    out
}
