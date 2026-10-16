//! `a=realtime`: the last 30 minutes, from `open_visits` and `events`
//! (port of `Reports\Realtime`). "Real time" is bounded by the collector
//! cadence (every 5 minutes).

use std::collections::HashMap;

use serde_json::{Value, json};

use super::domain_data::{Arg, DomainData};
use super::php;
use super::visits_log::with_version;
use crate::error::Result;

/// Window in seconds.
pub const WINDOW: i64 = 1800;
/// Recent events listed.
pub const RECENT: i64 = 20;

/// Country/device/browser/referrer/pageviews of a visit.
#[derive(Debug, Clone, Default)]
pub struct VisitAttrs {
    country: Option<String>,
    device: Option<String>,
    browser: Option<String>,
    referrer_host: Option<String>,
    pageviews: i64,
}

/// Builds the realtime panel.
pub fn build(d: &DomainData) -> Result<Value> {
    let now = d.now();
    let since = now - WINDOW;
    let active = d
        .row(
            "SELECT COUNT(*) AS n FROM open_visits WHERE last_ts >= ?",
            &[since.into()],
        )?
        .map(|r| r.i("n"))
        .unwrap_or(0);

    let mut per_minute = [0i64; 30];
    let mut pv = 0i64;
    for row in d.rows(
        "SELECT ts FROM events WHERE ts >= ? AND ts <= ? AND type = 'pageview'",
        &[since.into(), now.into()],
    )? {
        let i = (row.i("ts") - since) / 60;
        per_minute[i.clamp(0, 29) as usize] += 1;
        pv += 1;
    }

    let recent = d.rows(
        &format!(
            "SELECT ts, path, status, type, referrer, visit_id FROM events WHERE ts >= ? AND type <> 'bot' ORDER BY ts DESC, id DESC LIMIT {RECENT}"
        ),
        &[since.into()],
    )?;
    let mut ids: Vec<i64> = Vec::new();
    for r in &recent {
        if !r.get("visit_id").is_null() {
            let id = r.i("visit_id");
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let attrs = visit_attrs(d, &ids)?;
    let mut out = Vec::with_capacity(recent.len());
    for row in &recent {
        let a = attrs.get(&row.i("visit_id"));
        let referrer = match row.os("referrer").filter(|s| !s.is_empty()) {
            Some(r) => Some(r),
            None => a.and_then(|a| a.referrer_host.clone()),
        };
        out.push(json!({
            "ts": d.iso_ts(row.i("ts")),
            "path": row.s("path"),
            "status": row.i("status"),
            "type": row.s("type"),
            "country": a.and_then(|a| a.country.clone()),
            "device": a.and_then(|a| a.device.clone()),
            "browser": a.and_then(|a| a.browser.clone()),
            "referrer": referrer,
            "visit_pageviews": a.map(|a| a.pageviews),
        }));
    }
    let last = d
        .row("SELECT MAX(ts) AS ts FROM events", &[])?
        .filter(|r| !r.get("ts").is_null())
        .map(|r| d.iso_ts(r.i("ts")));
    Ok(json!({
        "active_visits": active,
        "pageviews_30m": pv,
        "per_minute": per_minute.to_vec(),
        "recent": out,
        "last_event_at": last,
        "last_collect_at": d.iso(d.meta_string("last_collect_at").as_deref()),
    }))
}

fn nonempty_scalar(v: Option<&Value>) -> Option<String> {
    v.and_then(php::scalar_string).filter(|s| !s.is_empty())
}

/// Attributes of visits from `visits` or (winning) `open_visits`.
pub fn visit_attrs(d: &DomainData, ids: &[i64]) -> Result<HashMap<i64, VisitAttrs>> {
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let marks = vec!["?"; ids.len()].join(",");
    let args: Vec<Arg> = ids.iter().map(|i| Arg::I(*i)).collect();
    for r in d.rows(
        &format!("SELECT visit_id, country, device, browser, browser_version, referrer_host, pageviews FROM visits WHERE visit_id IN ({marks})"),
        &args,
    )? {
        out.insert(
            r.i("visit_id"),
            VisitAttrs {
                country: r.os("country"),
                device: r.os("device"),
                browser: with_version(r.os("browser").as_deref(), r.os("browser_version").as_deref()),
                referrer_host: r.os("referrer_host"),
                pageviews: r.i("pageviews"),
            },
        );
    }
    for r in d.rows(
        &format!("SELECT visit_id, pageviews, attrs FROM open_visits WHERE visit_id IN ({marks})"),
        &args,
    )? {
        let a: Value = serde_json::from_str(&r.s("attrs")).unwrap_or(Value::Null);
        let a = if a.is_object() { a } else { Value::Null };
        out.insert(
            r.i("visit_id"),
            VisitAttrs {
                country: nonempty_scalar(a.get("country")),
                device: nonempty_scalar(a.get("device")),
                browser: with_version(
                    nonempty_scalar(a.get("browser")).as_deref(),
                    nonempty_scalar(a.get("browser_version")).as_deref(),
                ),
                referrer_host: nonempty_scalar(a.get("referrer_host")),
                pageviews: r.i("pageviews"),
            },
        );
    }
    Ok(out)
}
