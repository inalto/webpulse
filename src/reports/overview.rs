//! `a=overview`: KPIs with previous-period deltas, the main time series,
//! hour-of-day / weekday distributions and the weekday × hour heatmap
//! (port of `Reports\Overview`).

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use serde_json::{Map, Value, json};

use super::domain_data::{Arg, DomainData};
use super::php::{self, Num};
use super::range::Range;
use crate::error::Result;

const DAILY_COLS: [&str; 12] = [
    "visits",
    "visitors",
    "pageviews",
    "hits",
    "bytes",
    "bounces",
    "duration",
    "downloads",
    "errors_4xx",
    "errors_5xx",
    "bot_hits",
    "bot_bytes",
];

/// Series returned in `series`.
pub const SERIES: [&str; 10] = [
    "visits",
    "visitors",
    "pageviews",
    "hits",
    "bytes",
    "bot_hits",
    "bounces",
    "duration",
    "errors_4xx",
    "errors_5xx",
];

/// Series the `hourly` table can provide.
const HOURLY: [&str; 5] = ["visits", "pageviews", "hits", "bytes", "bot_hits"];

/// A time series: labels plus one vector (or `null`) per metric.
pub struct Series {
    /// Bucket labels.
    pub labels: Vec<String>,
    /// Metric → values (`None` = not available at this granularity).
    pub values: Vec<(&'static str, Option<Vec<i64>>)>,
}

impl Series {
    /// Values of one metric (`None` when unavailable).
    pub fn get(&self, metric: &str) -> Option<&Vec<i64>> {
        self.values
            .iter()
            .find(|(k, _)| *k == metric)
            .and_then(|(_, v)| v.as_ref())
    }

    fn get_mut(&mut self, metric: &str) -> Option<&mut Vec<i64>> {
        self.values
            .iter_mut()
            .find(|(k, _)| *k == metric)
            .and_then(|(_, v)| v.as_mut())
    }

    /// Values of one metric as JSON (`null` when unavailable).
    pub fn json(&self, metric: &str) -> Value {
        match self.get(metric) {
            Some(v) => json!(v),
            None => Value::Null,
        }
    }

    /// `{"labels": …, <metric>: …}` in SERIES order.
    pub fn to_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("labels".into(), json!(self.labels));
        for s in SERIES {
            m.insert(s.into(), self.json(s));
        }
        m
    }
}

/// Builds the overview.
pub fn build(d: &DomainData, r: &Range, compare: bool) -> Result<Value> {
    let cur = totals(d, &r.from, &r.to)?;
    let prev = totals(d, &r.prev_from, &r.prev_to)?;

    let mut series = series(d, r, None)?.to_json();
    if compare {
        let p = series_of(d, &r.previous(), r.granularity)?;
        let mut pm = Map::new();
        pm.insert("labels".into(), json!(p.labels));
        for k in [
            "visits",
            "pageviews",
            "visitors",
            "hits",
            "bytes",
            "bot_hits",
            "bounces",
            "duration",
            "errors_4xx",
            "errors_5xx",
        ] {
            pm.insert(k.into(), p.json(k));
        }
        series.insert("prev".into(), Value::Object(pm));
    }
    let (hours, heat) = hourly(d, r)?;
    Ok(json!({
        "range": r.to_json(),
        "kpi": kpis(&cur, &prev),
        "series": Value::Object(series),
        "hours": hours,
        "weekdays": weekdays(d, r)?,
        "heatmap": heat,
        "meta": d.meta_block(),
    }))
}

/// Sums of the `daily` table between two days.
pub fn totals(d: &DomainData, from: &str, to: &str) -> Result<HashMap<&'static str, i64>> {
    let sel = DAILY_COLS
        .iter()
        .map(|c| format!("COALESCE(SUM({c}),0) AS {c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let row = d.row(
        &format!("SELECT {sel} FROM daily WHERE day BETWEEN ? AND ?"),
        &[from.into(), to.into()],
    )?;
    let mut out = HashMap::new();
    for c in DAILY_COLS {
        out.insert(c, row.as_ref().map(|r| r.i(c)).unwrap_or(0));
    }
    Ok(out)
}

fn derive(t: &HashMap<&'static str, i64>) -> Vec<(&'static str, Num)> {
    let g = |k: &str| t.get(k).copied().unwrap_or(0);
    let v = g("visits");
    let all_hits = g("hits") + g("bot_hits");
    let ratio = |a: i64, b: i64, places: i32| -> Num {
        if b > 0 {
            Num::F(php::round(php::div(a, b).f(), places))
        } else {
            Num::I(0)
        }
    };
    vec![
        ("visits", Num::I(v)),
        ("visitors", Num::I(g("visitors"))),
        ("pageviews", Num::I(g("pageviews"))),
        ("pages_per_visit", ratio(g("pageviews"), v, 2)),
        (
            "avg_duration",
            if v > 0 {
                Num::I(php::round_int(php::div(g("duration"), v).f()))
            } else {
                Num::I(0)
            },
        ),
        ("bounce_rate", ratio(g("bounces"), v, 4)),
        ("bytes", Num::I(g("bytes"))),
        ("bot_share", ratio(g("bot_hits"), all_hits, 4)),
        ("downloads", Num::I(g("downloads"))),
        ("errors_4xx", Num::I(g("errors_4xx"))),
        ("errors_5xx", Num::I(g("errors_5xx"))),
        ("hits", Num::I(g("hits"))),
        ("bot_hits", Num::I(g("bot_hits"))),
    ]
}

/// KPI objects `{value, prev, change}`.
pub fn kpis(c: &HashMap<&'static str, i64>, p: &HashMap<&'static str, i64>) -> Value {
    let a = derive(c);
    let b = derive(p);
    let mut out = Map::new();
    for ((k, v), (_, pv)) in a.into_iter().zip(b) {
        out.insert(
            k.into(),
            json!({"value": v.json(), "prev": pv.json(), "change": change(v, pv)}),
        );
    }
    Value::Object(out)
}

/// `(value - prev) / prev` rounded to 4 places, `null` when prev is 0.
pub fn change(value: Num, prev: Num) -> Value {
    if prev.is_zero() {
        return Value::Null;
    }
    let q = match (value, prev) {
        (Num::I(a), Num::I(b)) => php::div(a - b, b).f(),
        _ => (value.f() - prev.f()) / prev.f(),
    };
    php::fjson(php::round(q, 4))
}

/// Main series bucketed by the range's granularity.
pub fn series(d: &DomainData, r: &Range, granularity: Option<&str>) -> Result<Series> {
    series_of(d, r, granularity.unwrap_or(r.granularity))
}

fn series_of(d: &DomainData, r: &Range, g: &str) -> Result<Series> {
    let labels = r.labels(g);
    let n = labels.len();
    if g == "hour" {
        let values = SERIES
            .iter()
            .map(|s| {
                (
                    *s,
                    if HOURLY.contains(s) {
                        Some(vec![0; n])
                    } else {
                        None
                    },
                )
            })
            .collect();
        let mut out = Series { labels, values };
        let pos: HashMap<String, usize> = r
            .day_list()
            .into_iter()
            .enumerate()
            .map(|(i, day)| (day, i))
            .collect();
        let rows = d.rows(
            "SELECT day, hour, visits, pageviews, hits, bytes, bot_hits FROM hourly WHERE day BETWEEN ? AND ?",
            &[Arg::from(&r.from), Arg::from(&r.to)],
        )?;
        for row in rows {
            let h = row.i("hour");
            let Some(&p) = pos.get(&row.s("day")) else {
                continue;
            };
            if !(0..=23).contains(&h) {
                continue;
            }
            let i = p * 24 + h as usize;
            for s in HOURLY {
                if let Some(v) = out.get_mut(s) {
                    v[i] += row.i(s);
                }
            }
        }
        return Ok(out);
    }
    let mut out = Series {
        labels,
        values: SERIES.iter().map(|s| (*s, Some(vec![0; n]))).collect(),
    };
    let idx = r.day_index(g);
    let cols = SERIES.join(", ");
    for row in d.rows(
        &format!("SELECT day, {cols} FROM daily WHERE day BETWEEN ? AND ?"),
        &[Arg::from(&r.from), Arg::from(&r.to)],
    )? {
        let Some(&i) = idx.get(&row.s("day")) else {
            continue;
        };
        for s in SERIES {
            if let Some(v) = out.get_mut(s) {
                v[i] += row.i(s);
            }
        }
    }
    Ok(out)
}

/// Hour-of-day totals and the weekday × hour heatmap (visits), Monday first.
fn hourly(d: &DomainData, r: &Range) -> Result<(Value, Value)> {
    let mut hv = [0i64; 24];
    let mut hp = [0i64; 24];
    let mut heat = [[0i64; 24]; 7];
    let rows = d.rows(
        "SELECT day, hour, SUM(visits) AS visits, SUM(pageviews) AS pageviews FROM hourly WHERE day BETWEEN ? AND ? GROUP BY day, hour",
        &[Arg::from(&r.from), Arg::from(&r.to)],
    )?;
    let mut dow: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let h = row.i("hour");
        if !(0..=23).contains(&h) {
            continue;
        }
        let h = h as usize;
        let day = row.s("day");
        let w = *dow.entry(day.clone()).or_insert_with(|| weekday(&day));
        hv[h] += row.i("visits");
        hp[h] += row.i("pageviews");
        heat[w][h] += row.i("visits");
    }
    Ok((
        json!({"visits": hv.to_vec(), "pageviews": hp.to_vec()}),
        json!(heat.iter().map(|r| r.to_vec()).collect::<Vec<_>>()),
    ))
}

fn weekdays(d: &DomainData, r: &Range) -> Result<Value> {
    let mut v = [0i64; 7];
    let mut p = [0i64; 7];
    for row in d.rows(
        "SELECT day, visits, pageviews FROM daily WHERE day BETWEEN ? AND ?",
        &[Arg::from(&r.from), Arg::from(&r.to)],
    )? {
        let w = weekday(&row.s("day"));
        v[w] += row.i("visits");
        p[w] += row.i("pageviews");
    }
    Ok(json!({"visits": v.to_vec(), "pageviews": p.to_vec()}))
}

/// 0 = Monday … 6 = Sunday (PHP `gmmktime()` semantics for odd input:
/// out-of-range parts roll over).
pub fn weekday(day: &str) -> usize {
    let num = |a: usize, b: usize| day.get(a..b).map(php::intval).unwrap_or(0);
    let (y, m, dd) = (num(0, 4), num(5, 7), num(8, 10));
    // normalise like mktime(): months and days may overflow
    let base = NaiveDate::from_ymd_opt(
        y as i32 + ((m - 1).div_euclid(12)) as i32,
        ((m - 1).rem_euclid(12) + 1) as u32,
        1,
    );
    let Some(base) = base else { return 0 };
    let date = base + chrono::Duration::days(dd - 1);
    date.weekday().num_days_from_monday() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_and_change() {
        assert_eq!(weekday("2026-10-16"), 4);
        assert_eq!(weekday("2026-10-19"), 0);
        assert_eq!(change(Num::I(1234), Num::I(1100)), json!(0.1218));
        assert_eq!(change(Num::I(2), Num::I(0)), Value::Null);
        assert_eq!(change(Num::I(200), Num::I(100)).to_string(), "1.0");
    }
}
