//! `a=health`: the technical panel (GoAccess inspired): status codes, error
//! trend, 404s with their referrers, 5xx pages, bandwidth by file type, heavy
//! static files, downloads, protocol / HTTP version / method and peak hour
//! (port of `Reports\Health`).

use std::collections::HashMap;

use serde_json::{Value, json};

use super::dimension::NF_REF_SEP;
use super::domain_data::{Arg, DomainData};
use super::overview;
use super::range::Range;
use super::report::{self, Params, like_escape};
use crate::error::Result;

/// Builds the technical panel.
pub fn build(d: &DomainData, r: &Range) -> Result<Value> {
    let series = overview::series(
        d,
        r,
        if r.granularity == "hour" {
            Some("day")
        } else {
            None
        },
    )?;

    let mut not_found = top(d, r, "not_found", "hits", 10)?;
    let paths: Vec<String> = not_found
        .iter()
        .map(|x| x["key"].as_str().unwrap_or("").to_string())
        .collect();
    let refs = not_found_referrers(d, r, &paths)?;
    for row in not_found.iter_mut() {
        let k = row["key"].as_str().unwrap_or("").to_string();
        let list = refs.get(&k).cloned().unwrap_or_default();
        if let Some(o) = row.as_object_mut() {
            o.insert("referrers".into(), Value::Array(list));
        }
    }

    let fr = [Arg::from(&r.from), Arg::from(&r.to)];
    let peak = d.row("SELECT day, hour, hits FROM hourly WHERE day BETWEEN ? AND ? ORDER BY hits DESC, day DESC LIMIT 1", &fr)?;
    let peak_hour =
        d.row("SELECT hour, SUM(hits) AS hits FROM hourly WHERE day BETWEEN ? AND ? GROUP BY hour ORDER BY hits DESC LIMIT 1", &fr)?;

    let get = |m: &str| series.json(m);
    Ok(json!({
        "range": r.to_json(),
        "status": top(d, r, "status", "hits", 50)?,
        "errors_series": {
            "labels": series.labels,
            "errors_4xx": get("errors_4xx"),
            "errors_5xx": get("errors_5xx"),
            "hits": hits_series(d, r, series.labels.len())?,
            "bot_hits": get("bot_hits"),
        },
        "not_found": not_found,
        "server_error": top(d, r, "server_error", "hits", 10)?,
        "file_type": top(d, r, "file_type", "bytes", 30)?,
        "static_file": top(d, r, "static_file", "bytes", 10)?,
        "download": top(d, r, "download", "hits", 10)?,
        "protocol": top(d, r, "protocol", "hits", 10)?,
        "http_version": top(d, r, "http_version", "hits", 10)?,
        "method": top(d, r, "method", "hits", 10)?,
        "peak": peak.map(|p| json!({"day": p.s("day"), "hour": p.i("hour"), "hits": p.i("hits")})),
        "peak_hour": peak_hour.map(|p| json!({"hour": p.i("hour"), "hits": p.i("hits")})),
    }))
}

/// Top rows of a dim by a metric, decorated like `report` rows.
pub fn top(d: &DomainData, r: &Range, dim: &str, metric: &str, limit: i64) -> Result<Vec<Value>> {
    let p = Params {
        dim: dim.into(),
        sort: metric.into(),
        desc: true,
        limit,
        offset: 0,
        q: String::new(),
        compare: false,
        series: false,
    };
    let mut rep = report::build(d, r, &p)?;
    Ok(rep
        .get_mut("rows")
        .and_then(|v| v.as_array_mut())
        .map(std::mem::take)
        .unwrap_or_default())
}

fn hits_series(d: &DomainData, r: &Range, n: usize) -> Result<Vec<i64>> {
    let g = if r.granularity == "hour" {
        "day"
    } else {
        r.granularity
    };
    let idx = r.day_index(g);
    let mut out = vec![0i64; n];
    for row in d.rows(
        "SELECT day, hits FROM daily WHERE day BETWEEN ? AND ?",
        &[Arg::from(&r.from), Arg::from(&r.to)],
    )? {
        if let Some(&i) = idx.get(&row.s("day"))
            && i < n
        {
            out[i] += row.i("hits");
        }
    }
    Ok(out)
}

/// Top 5 referrer hosts per 404 path from the `not_found_referrer` dim.
pub fn not_found_referrers(
    d: &DomainData,
    r: &Range,
    paths: &[String],
) -> Result<HashMap<String, Vec<Value>>> {
    let mut out: HashMap<String, Vec<Value>> = HashMap::new();
    for p in paths {
        let prefix = format!("{p}{NF_REF_SEP}");
        let like = format!("{}%", like_escape(&prefix));
        let rows = d.rows(
            "SELECT key, SUM(hits) AS hits FROM dim_daily WHERE dim = 'not_found_referrer' AND day BETWEEN ? AND ? \
             AND key LIKE ? ESCAPE '\\' GROUP BY key ORDER BY hits DESC LIMIT 5",
            &[Arg::from(&r.from), Arg::from(&r.to), Arg::S(like)],
        )?;
        for row in rows {
            let k = row.s("key");
            // LIKE is case-insensitive; keep exact prefixes only
            if !k.as_bytes().starts_with(prefix.as_bytes()) {
                continue;
            }
            let host = String::from_utf8_lossy(&k.as_bytes()[prefix.len()..]).into_owned();
            out.entry(p.clone())
                .or_default()
                .push(json!({"key": host, "hits": row.i("hits")}));
        }
    }
    Ok(out)
}
