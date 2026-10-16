//! `a=visits_log`: Matomo-style visitor log, newest first, cursor-paginated
//! by start timestamp (`before`, plus `before_id` to break ties). Includes
//! still-open visits (flagged `live`). Port of `Reports\VisitsLog`.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::domain_data::{Arg, DomainData, Row};
use super::php;
use super::range::Range;
use super::{Query, invalid};
use crate::error::Result;

/// Default page size.
pub const DEFAULT_LIMIT: i64 = 20;
/// Largest page size.
pub const MAX_LIMIT: i64 = 100;
/// Actions listed per visit.
pub const MAX_ACTIONS: usize = 100;

const ATTRS: [&str; 13] = [
    "channel",
    "referrer_host",
    "search_engine",
    "keyword",
    "campaign",
    "country",
    "city",
    "browser",
    "browser_version",
    "os",
    "os_version",
    "device",
    "ip",
];

fn int(v: Option<&str>, default: i64, name: &str) -> Result<i64> {
    match v {
        None | Some("") => Ok(default),
        Some(s) if !s.is_empty() && s.len() <= 19 && s.bytes().all(|c| c.is_ascii_digit()) => {
            Ok(s.parse().unwrap_or(i64::MAX))
        }
        Some(_) => Err(invalid(format!(
            "\"{name}\" must be a non-negative integer"
        ))),
    }
}

/// `Chrome 129` from browser `Chrome` and version `129` or `Chrome 129`.
pub fn with_version(name: Option<&str>, version: Option<&str>) -> Option<String> {
    let version = version.filter(|v| !v.is_empty());
    match name.filter(|n| !n.is_empty()) {
        None => version.map(str::to_string),
        Some(n) => match version {
            None => Some(n.to_string()),
            Some(v) if v.starts_with(n) => Some(v.to_string()),
            Some(v) => Some(format!("{n} {v}")),
        },
    }
}

struct Visit {
    id: i64,
    start: i64,
    end: i64,
    fields: Map<String, Value>,
}

fn base(id: i64, start: i64, end: i64, duration: i64, row: &Row) -> (Visit, Map<String, Value>) {
    let mut m = Map::new();
    m.insert("visit_id".into(), json!(id));
    m.insert("duration".into(), json!(duration));
    m.insert("pageviews".into(), json!(row.i("pageviews")));
    m.insert("hits".into(), json!(row.i("hits")));
    m.insert("bytes".into(), json!(row.i("bytes")));
    m.insert("entry_page".into(), json!(row.os("entry_page")));
    m.insert("exit_page".into(), json!(row.os("exit_page")));
    (
        Visit {
            id,
            start,
            end,
            fields: Map::new(),
        },
        m,
    )
}

fn finish(
    mut v: Visit,
    mut m: Map<String, Value>,
    attrs: HashMap<&str, Option<String>>,
    live: bool,
) -> Visit {
    let a = |k: &str| attrs.get(k).cloned().flatten();
    for k in ATTRS {
        match k {
            "browser" => {
                m.insert(
                    k.into(),
                    json!(with_version(
                        a("browser").as_deref(),
                        a("browser_version").as_deref()
                    )),
                );
            }
            "os" => {
                m.insert(
                    k.into(),
                    json!(with_version(a("os").as_deref(), a("os_version").as_deref())),
                );
            }
            "browser_version" | "os_version" => {}
            "ip" => {
                if let Some(ip) = a("ip") {
                    m.insert(k.into(), json!(ip));
                }
            }
            _ => {
                m.insert(k.into(), json!(a(k)));
            }
        }
    }
    m.insert("live".into(), json!(live));
    v.fields = m;
    v
}

fn from_closed(row: &Row) -> Visit {
    let (v, m) = base(
        row.i("visit_id"),
        row.i("start_ts"),
        row.i("end_ts"),
        row.i("duration"),
        row,
    );
    let attrs = ATTRS
        .iter()
        .map(|k| (*k, row.os(k).filter(|s| !s.is_empty())))
        .collect();
    finish(v, m, attrs, false)
}

fn from_open(row: &Row) -> Visit {
    let start = row.i("start_ts");
    let (v, m) = base(
        row.i("visit_id"),
        start,
        row.i("last_ts"),
        (row.i("last_pv_ts") - start).max(0),
        row,
    );
    let raw = if row.has("attrs") {
        row.s("attrs")
    } else {
        "{}".into()
    };
    let decoded: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let attrs = ATTRS
        .iter()
        .map(|k| {
            (
                *k,
                decoded
                    .get(k)
                    .and_then(php::scalar_string)
                    .filter(|s| !s.is_empty()),
            )
        })
        .collect();
    finish(v, m, attrs, true)
}

/// Builds one page of the visitor log.
pub fn build(d: &DomainData, r: &Range, q: &Query) -> Result<Value> {
    let limit = int(q.get_any("limit"), DEFAULT_LIMIT, "limit")?;
    if limit < 1 {
        return Err(invalid("\"limit\" must be at least 1"));
    }
    let limit = limit.min(MAX_LIMIT);
    let before = int(q.get_any("before"), i64::MAX, "before")?;
    let before_id = int(q.get_any("before_id"), i64::MAX, "before_id")?;

    let cursor = "(start_ts < ? OR (start_ts = ? AND visit_id < ?))";
    let args: Vec<Arg> = vec![
        Arg::from(&r.from),
        Arg::from(&r.to),
        before.into(),
        before.into(),
        before_id.into(),
    ];
    let closed = d.rows(
        &format!("SELECT * FROM visits WHERE day BETWEEN ? AND ? AND {cursor} ORDER BY start_ts DESC, visit_id DESC LIMIT {}", limit + 1),
        &args,
    )?;
    let open = d.rows(
        &format!(
            "SELECT * FROM open_visits WHERE day BETWEEN ? AND ? AND {cursor} ORDER BY start_ts DESC, visit_id DESC LIMIT {}",
            limit + 1
        ),
        &args,
    )?;
    let mut all: Vec<Visit> = closed
        .iter()
        .map(from_closed)
        .chain(open.iter().map(from_open))
        .collect();
    all.sort_by_key(|v| std::cmp::Reverse((v.start, v.id)));
    let more = all.len() as i64 > limit;
    all.truncate(limit as usize);

    let ids: Vec<i64> = all.iter().map(|v| v.id).collect();
    let actions = actions(d, &ids)?;
    let mut visits = Vec::with_capacity(all.len());
    for v in &all {
        let mut m = v.fields.clone();
        let acts: Vec<Value> = actions
            .get(&v.id)
            .map(|rows| {
                rows.iter()
                    .map(|a| {
                        json!({
                            "ts": d.iso_ts(a.i("ts")),
                            "type": a.s("type"),
                            "path": a.s("path"),
                            "status": a.i("status"),
                            "bytes": a.i("bytes"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        m.insert("actions".into(), Value::Array(acts));
        m.insert("start".into(), json!(d.iso_ts(v.start)));
        m.insert("end".into(), json!(d.iso_ts(v.end)));
        visits.push(Value::Object(m));
    }
    let last = all.last();
    Ok(json!({
        "visits": visits,
        "next_before": if more { last.map(|l| l.start) } else { None },
        "next_before_id": if more { last.map(|l| l.id) } else { None },
        "retention_days": d.raw_retention_days,
    }))
}

fn actions(d: &DomainData, ids: &[i64]) -> Result<HashMap<i64, Vec<Row>>> {
    let mut out: HashMap<i64, Vec<Row>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let marks = vec!["?"; ids.len()].join(",");
    let args: Vec<Arg> = ids.iter().map(|i| Arg::I(*i)).collect();
    for row in d.rows(&format!("SELECT visit_id, ts, type, path, status, bytes FROM events WHERE visit_id IN ({marks}) ORDER BY ts, id"), &args)? {
        let e = out.entry(row.i("visit_id")).or_default();
        if e.len() < MAX_ACTIONS {
            e.push(row);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_cursor() {
        assert_eq!(
            with_version(Some("Chrome"), Some("129")).as_deref(),
            Some("Chrome 129")
        );
        assert_eq!(
            with_version(Some("Chrome"), Some("Chrome 129")).as_deref(),
            Some("Chrome 129")
        );
        assert_eq!(with_version(None, Some("129")).as_deref(), Some("129"));
        assert_eq!(with_version(Some("Linux"), None).as_deref(), Some("Linux"));
        assert_eq!(with_version(None, None), None);
        assert!(int(Some("-5"), 0, "before").is_err());
        assert!(int(Some("x"), 0, "before").is_err());
        assert_eq!(int(Some("99999999999999999999"), 0, "b").ok(), None);
        assert_eq!(int(Some("9999999999999999999"), 0, "b").unwrap(), i64::MAX);
    }
}
