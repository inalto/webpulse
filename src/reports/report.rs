//! `a=report&name=<dim>`: ranked table for one dimension over a range, with
//! derived rates, optional previous-period values and an optional per-bucket
//! series of the top keys (port of `Reports\Report`).

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::dimension;
use super::domain_data::{Arg, DomainData, Row};
use super::php;
use super::pretty;
use super::range::Range;
use super::{Query, invalid};
use crate::error::Result;

/// Default page size.
pub const DEFAULT_LIMIT: i64 = 10;
/// Largest page size (larger values are capped, not rejected).
pub const MAX_LIMIT: i64 = 500;
/// Rows of a CSV export.
pub const EXPORT_MAX: i64 = 10000;
/// Keys of the `series=1` breakdown (plus `(other)`).
pub const SERIES_KEYS: usize = 7;
/// Key of the folded remainder.
pub const OTHER: &str = "(other)";

fn sortable(col: &str) -> Option<&'static str> {
    Some(match col {
        "key" => "key",
        "visits" => "visits",
        "pageviews" => "pageviews",
        "hits" => "hits",
        "bytes" => "bytes",
        "bounces" => "bounces",
        "duration" => "duration",
        "bounce_rate" => "CAST(bounces AS REAL) / MAX(visits, 1)",
        "avg_duration" => "CAST(duration AS REAL) / MAX(visits, 1)",
        _ => return None,
    })
}

/// Validated report parameters.
#[derive(Debug, Clone)]
pub struct Params {
    /// Dimension (whitelisted).
    pub dim: String,
    /// Sort column (whitelisted) or `bucket`.
    pub sort: String,
    /// Descending order.
    pub desc: bool,
    /// Page size.
    pub limit: i64,
    /// Offset.
    pub offset: i64,
    /// Substring filter on `key`.
    pub q: String,
    /// Previous-period values.
    pub compare: bool,
    /// Per-bucket series of the top keys.
    pub series: bool,
}

fn int(v: Option<&str>, default: i64, name: &str) -> Result<i64> {
    match v {
        None | Some("") => Ok(default),
        Some(s) => {
            let digits = s.strip_prefix('-').unwrap_or(s);
            if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|c| c.is_ascii_digit())
            {
                return Err(invalid(format!("\"{name}\" must be an integer")));
            }
            Ok(s.parse().unwrap_or(default))
        }
    }
}

/// Validates the report parameters of a query (`export` raises the limits).
pub fn params(q: &Query, export: bool) -> Result<Params> {
    let dim = match q.get("name") {
        Some(d) if dimension::is_valid(d) => d.to_string(),
        other => {
            return Err(invalid(format!(
                "Unknown report \"{}\"",
                php::substr(other.unwrap_or(""), 40)
            )));
        }
    };
    let sort = q
        .get("sort")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| dimension::default_sort(&dim));
    let desc = sort.starts_with('-');
    let mut col = sort.trim_start_matches('-').to_string();
    if col == "share" {
        col = dimension::primary_metric(&dim).to_string();
    }
    if col != "bucket" && sortable(&col).is_none() {
        return Err(invalid(format!(
            "Invalid sort column \"{}\"",
            php::substr(&col, 40)
        )));
    }
    if col == "bucket" && !dimension::is_bucket(&dim) {
        return Err(invalid(
            "Sort \"bucket\" only applies to engagement reports",
        ));
    }
    let max = if export { EXPORT_MAX } else { MAX_LIMIT };
    let limit = int(
        q.get_any("limit"),
        if export { EXPORT_MAX } else { DEFAULT_LIMIT },
        "limit",
    )?;
    if limit < 1 {
        return Err(invalid("\"limit\" must be at least 1"));
    }
    let offset = int(q.get_any("offset"), 0, "offset")?;
    if !(0..=1_000_000).contains(&offset) {
        return Err(invalid("\"offset\" is out of range"));
    }
    let filter = q
        .get("q")
        .map(|f| php::mb_substr(php::trim(f), 200))
        .unwrap_or_default();
    let flag = |n: &str| matches!(q.get(n), Some("1") | Some("true"));
    Ok(Params {
        dim,
        sort: col,
        desc,
        limit: limit.min(max),
        offset,
        q: filter,
        compare: flag("compare"),
        series: flag("series"),
    })
}

/// `addcslashes($s, '%_\\')` for a LIKE pattern with `ESCAPE '\'`.
pub fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '%' || c == '_' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn where_clause(dim: &str, from: &str, to: &str, q: &str) -> (String, Vec<Arg>) {
    let mut w = "dim = ? AND day BETWEEN ? AND ?".to_string();
    let mut args: Vec<Arg> = vec![dim.into(), from.into(), to.into()];
    if !q.is_empty() {
        w.push_str(" AND key LIKE ? ESCAPE '\\'");
        args.push(Arg::S(format!("%{}%", like_escape(q))));
    }
    (w, args)
}

/// Builds the report.
pub fn build(d: &DomainData, r: &Range, p: &Params) -> Result<Value> {
    let dim = p.dim.as_str();
    let (wh, args) = where_clause(dim, &r.from, &r.to, &p.q);
    let trow = d.row(
        "SELECT COALESCE(SUM(visits),0) AS visits, COALESCE(SUM(pageviews),0) AS pageviews, COALESCE(SUM(hits),0) AS hits, \
         COALESCE(SUM(bytes),0) AS bytes, COALESCE(SUM(bounces),0) AS bounces, COALESCE(SUM(duration),0) AS duration \
         FROM dim_daily WHERE dim = ? AND day BETWEEN ? AND ?",
        &[dim.into(), Arg::from(&r.from), Arg::from(&r.to)],
    )?;
    let mut total: Vec<(&'static str, i64)> = dimension::COLUMNS
        .iter()
        .map(|c| (*c, trow.as_ref().map(|t| t.i(c)).unwrap_or(0)))
        .collect();
    let count = d
        .row(
            &format!("SELECT COUNT(DISTINCT key) AS n FROM dim_daily WHERE {wh}"),
            &args,
        )?
        .map(|r| r.i("n"))
        .unwrap_or(0);
    total.push(("rows", count));

    let rows = query(d, &wh, args, &p.sort, p.desc, p.limit + 1, p.offset)?;
    let has_more = rows.len() as i64 > p.limit;
    let rows: Vec<Row> = rows.into_iter().take(p.limit as usize).collect();
    let mut rows = decorate(d, r, dim, &rows, &total)?;

    if p.compare && !rows.is_empty() {
        let keys: Vec<String> = rows
            .iter()
            .map(|o| o["key"].as_str().unwrap_or("").to_string())
            .collect();
        let prev = by_keys(d, dim, &r.prev_from, &r.prev_to, &keys)?;
        for o in rows.iter_mut() {
            let k = o["key"].as_str().unwrap_or("").to_string();
            let pr = prev.get(&k);
            let g = |c: &str| pr.and_then(|m| m.get(c)).copied().unwrap_or(0);
            o.insert("prev".into(), json!({"visits": g("visits"), "pageviews": g("pageviews"), "hits": g("hits"), "bytes": g("bytes")}));
        }
    }

    let mut out = Map::new();
    out.insert("name".into(), json!(dim));
    out.insert("label".into(), json!(dimension::label(dim)));
    out.insert("range".into(), r.to_json());
    out.insert(
        "total".into(),
        Value::Object(
            total
                .iter()
                .map(|(k, v)| (k.to_string(), json!(v)))
                .collect(),
        ),
    );
    out.insert(
        "rows".into(),
        Value::Array(rows.into_iter().map(Value::Object).collect()),
    );
    out.insert("has_more".into(), json!(has_more));
    if p.series {
        out.insert("series".into(), series(d, r, dim)?);
    }
    Ok(Value::Object(out))
}

fn query(
    d: &DomainData,
    wh: &str,
    mut args: Vec<Arg>,
    sort: &str,
    desc: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<Row>> {
    let expr = if sort == "bucket" {
        let mut cases: Vec<(&str, usize)> = Vec::new();
        for (i, k) in dimension::DURATION_BUCKETS
            .iter()
            .chain(dimension::PAGES_BUCKETS.iter())
            .enumerate()
        {
            if !cases.iter().any(|(x, _)| x == k) {
                cases.push((k, i % 8));
            }
        }
        let mut e = "CASE key".to_string();
        for (k, i) in cases {
            e.push_str(&format!(" WHEN '{k}' THEN {i}"));
        }
        e.push_str(" ELSE 99 END");
        e
    } else {
        sortable(sort).unwrap_or("key").to_string()
    };
    let dir = if desc { "DESC" } else { "ASC" };
    let sql = format!(
        "SELECT key, SUM(visits) AS visits, SUM(pageviews) AS pageviews, SUM(hits) AS hits, SUM(bytes) AS bytes, \
         SUM(bounces) AS bounces, SUM(duration) AS duration, MAX(day) AS last_day \
         FROM dim_daily WHERE {wh} GROUP BY key ORDER BY {expr} {dir}, key ASC LIMIT ? OFFSET ?"
    );
    args.push(Arg::I(limit));
    args.push(Arg::I(offset));
    d.rows(&sql, &args)
}

/// Sums per key (visits, pageviews, hits, bytes) for a set of keys.
pub fn by_keys(
    d: &DomainData,
    dim: &str,
    from: &str,
    to: &str,
    keys: &[String],
) -> Result<HashMap<String, HashMap<&'static str, i64>>> {
    let mut out = HashMap::new();
    for chunk in keys.chunks(400) {
        let marks = vec!["?"; chunk.len()].join(",");
        let mut args: Vec<Arg> = vec![dim.into(), from.into(), to.into()];
        args.extend(chunk.iter().map(Arg::from));
        let rows = d.rows(
            &format!(
                "SELECT key, SUM(visits) AS visits, SUM(pageviews) AS pageviews, SUM(hits) AS hits, SUM(bytes) AS bytes \
                 FROM dim_daily WHERE dim = ? AND day BETWEEN ? AND ? AND key IN ({marks}) GROUP BY key"
            ),
            &args,
        )?;
        for row in rows {
            let m: HashMap<&'static str, i64> = ["visits", "pageviews", "hits", "bytes"]
                .iter()
                .map(|c| (*c, row.i(c)))
                .collect();
            out.insert(row.s("key"), m);
        }
    }
    Ok(out)
}

/// Casts numbers and adds share, labels and derived rates.
pub fn decorate(
    d: &DomainData,
    r: &Range,
    dim: &str,
    rows: &[Row],
    total: &[(&'static str, i64)],
) -> Result<Vec<Map<String, Value>>> {
    let share_by = dimension::primary_metric(dim);
    let labels = pretty::has_labels(dim);
    let keys: Vec<String> = rows.iter().map(|x| x.s("key")).collect();
    let mut exits = HashMap::new();
    let mut pageviews = HashMap::new();
    if dim == "page" {
        exits = by_keys(d, "exit_page", &r.from, &r.to, &keys)?;
    } else if dim == "exit_page" {
        pageviews = by_keys(d, "page", &r.from, &r.to, &keys)?;
    }
    let total_of = |c: &str| {
        total
            .iter()
            .find(|(k, _)| *k == c)
            .map(|(_, v)| *v)
            .unwrap_or(0)
    };
    let ratio4 = |a: i64, b: i64| php::fjson(php::round(php::div(a, b).f(), 4));
    let capped4 = |a: i64, b: i64| php::fjson(php::round(php::div(a, b).f().min(1.0), 4));

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let k = row.s("key");
        let mut o = Map::new();
        o.insert("key".into(), json!(k));
        if labels {
            o.insert("label".into(), json!(pretty::label(dim, &k)));
        }
        let mut vals: HashMap<&str, i64> = HashMap::new();
        for c in dimension::COLUMNS {
            let v = row.i(c);
            vals.insert(c, v);
            o.insert(c.into(), json!(v));
        }
        if dim == "bot" && !row.get("last_day").is_null() {
            o.insert("last_day".into(), json!(row.s("last_day")));
        }
        let t = total_of(share_by);
        o.insert(
            "share".into(),
            if t > 0 {
                ratio4(vals[share_by], t)
            } else {
                json!(0)
            },
        );
        if dim == "page" {
            let ex = exits
                .get(&k)
                .and_then(|m| m.get("visits"))
                .copied()
                .unwrap_or(0);
            o.insert("exits".into(), json!(ex));
            o.insert(
                "exit_rate".into(),
                if vals["pageviews"] > 0 {
                    capped4(ex, vals["pageviews"])
                } else {
                    json!(0)
                },
            );
        } else if dim == "exit_page" {
            let pv = pageviews
                .get(&k)
                .and_then(|m| m.get("pageviews"))
                .copied()
                .unwrap_or(0);
            o.insert(
                "exit_rate".into(),
                if pv > 0 {
                    capped4(vals["visits"], pv)
                } else {
                    Value::Null
                },
            );
        }
        if dimension::has_bounces(dim) {
            o.insert(
                "bounce_rate".into(),
                if vals["visits"] > 0 {
                    ratio4(vals["bounces"], vals["visits"])
                } else {
                    json!(0)
                },
            );
        }
        if dimension::is_visit_level(dim) && dimension::has_bounces(dim) {
            let avg = if vals["visits"] > 0 {
                php::round_int(php::div(vals["duration"], vals["visits"]).f())
            } else {
                0
            };
            o.insert("avg_duration".into(), json!(avg));
        }
        out.push(o);
    }
    Ok(out)
}

/// Per-bucket values of the primary metric for the top keys + `(other)`.
pub fn series(d: &DomainData, r: &Range, dim: &str) -> Result<Value> {
    let m = dimension::primary_metric(dim);
    let g = if r.granularity == "hour" {
        "day"
    } else {
        r.granularity
    };
    let labels = r.labels(g);
    let idx = r.day_index(g);
    let n = labels.len();
    let top: Vec<String> = d
        .rows(
            &format!(
                "SELECT key, SUM({m}) AS v FROM dim_daily WHERE dim = ? AND day BETWEEN ? AND ? GROUP BY key ORDER BY v DESC, key LIMIT {SERIES_KEYS}"
            ),
            &[dim.into(), Arg::from(&r.from), Arg::from(&r.to)],
        )?
        .iter()
        .map(|x| x.s("key"))
        .collect();
    let mut values: Vec<(String, Vec<i64>)> = Vec::new();
    for k in &top {
        if !values.iter().any(|(x, _)| x == k) {
            values.push((k.clone(), vec![0; n]));
        }
    }
    let mut other = vec![0i64; n];
    for row in d.rows(
        &format!("SELECT day, key, SUM({m}) AS v FROM dim_daily WHERE dim = ? AND day BETWEEN ? AND ? GROUP BY day, key"),
        &[dim.into(), Arg::from(&r.from), Arg::from(&r.to)],
    )? {
        let Some(&i) = idx.get(&row.s("day")) else { continue };
        let k = row.s("key");
        match values.iter_mut().find(|(x, _)| *x == k) {
            Some((_, v)) => v[i] += row.i("v"),
            None => other[i] += row.i("v"),
        }
    }
    let mut keys = top;
    let mut vals: Vec<Vec<i64>> = values.into_iter().map(|(_, v)| v).collect();
    if other.iter().sum::<i64>() > 0 {
        keys.push(OTHER.to_string());
        vals.push(other);
    }
    let key_labels: Vec<String> = keys
        .iter()
        .map(|k| {
            if k == OTHER {
                "Other".to_string()
            } else {
                pretty::label(dim, k)
            }
        })
        .collect();
    Ok(json!({
        "labels": labels,
        "granularity": g,
        "metric": m,
        "keys": keys,
        "key_labels": key_labels,
        "values": vals,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_validation() {
        let q = |pairs: Vec<(&str, &str)>| params(&Query::from_pairs(pairs), false);
        assert!(q(vec![("name", "nope")]).is_err());
        assert!(q(vec![("name", "page; DROP TABLE daily")]).is_err());
        assert!(q(vec![("name", "page"), ("sort", "key DESC; --")]).is_err());
        assert!(q(vec![("name", "page"), ("limit", "ten")]).is_err());
        assert!(q(vec![("name", "page"), ("sort", "bucket")]).is_err());
        let p = q(vec![("name", "page"), ("limit", "100000")]).unwrap();
        assert_eq!((p.limit, p.sort.as_str(), p.desc), (500, "pageviews", true));
        let p = q(vec![("name", "visit_duration")]).unwrap();
        assert_eq!(p.sort, "bucket");
        let p = q(vec![
            ("name", "page"),
            ("sort", "-share"),
            ("q", "  BLOG "),
            ("compare", "true"),
        ])
        .unwrap();
        assert_eq!(
            (p.sort.as_str(), p.q.as_str(), p.compare),
            ("pageviews", "BLOG", true)
        );
        assert_eq!(like_escape("50%_\\"), "50\\%\\_\\\\");
    }
}
