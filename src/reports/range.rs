//! An inclusive range of local days in a domain's timezone, plus the
//! immediately preceding period of the same length and the bucket
//! granularity used for time series (port of `Reports\Range`).

use std::collections::HashMap;

use chrono::{Datelike, Duration, Months, NaiveDate, TimeZone};
use chrono_tz::Tz;
use serde_json::{Value, json};

use super::{Query, invalid, php};
use crate::error::Result;

/// Shortcuts accepted by the `range` query parameter.
pub const SHORTCUTS: [&str; 10] = [
    "today",
    "yesterday",
    "7d",
    "30d",
    "90d",
    "12m",
    "month",
    "lastmonth",
    "year",
    "all",
];
/// Default shortcut.
pub const DEFAULT: &str = "30d";
/// Longest custom range accepted (days).
pub const MAX_DAYS: i64 = 3700;

/// A validated range of local days.
#[derive(Debug, Clone)]
pub struct Range {
    /// First local day, `YYYY-MM-DD`.
    pub from: String,
    /// Last local day (inclusive).
    pub to: String,
    /// The shortcut that produced the range, or `custom`.
    pub shortcut: String,
    /// Number of days.
    pub days: i64,
    /// `hour`, `day`, `week` or `month`.
    pub granularity: &'static str,
    /// First day of the previous period.
    pub prev_from: String,
    /// Last day of the previous period.
    pub prev_to: String,
    from_d: NaiveDate,
    to_d: NaiveDate,
}

fn fmt(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// True for a syntactically and calendrically valid `YYYY-MM-DD` (year ≥ 1).
pub fn is_date(s: &str) -> bool {
    parse_date(s).is_some()
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        let p = &s[r];
        if p.bytes().all(|c| c.is_ascii_digit()) {
            p.parse().ok()
        } else {
            None
        }
    };
    let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if y < 1 {
        return None;
    }
    NaiveDate::from_ymd_opt(y as i32, m, d)
}

fn date(s: &str) -> Result<NaiveDate> {
    parse_date(s).ok_or_else(|| {
        invalid(format!(
            "Invalid date \"{}\" (expected YYYY-MM-DD)",
            php::substr(s, 20)
        ))
    })
}

/// Granularity rule from API.md.
pub fn granularity_for(days: i64) -> &'static str {
    if days <= 2 {
        "hour"
    } else if days <= 120 {
        "day"
    } else if days <= 400 {
        "week"
    } else {
        "month"
    }
}

/// Every local day between two dates, inclusive.
pub fn days_between(from: &str, to: &str) -> Result<Vec<String>> {
    let mut cur = date(from)?;
    let end = date(to)?;
    let mut out = Vec::new();
    while cur <= end {
        out.push(fmt(cur));
        cur += Duration::days(1);
    }
    Ok(out)
}

impl Range {
    /// A range between two local days (validated).
    pub fn new(from: &str, to: &str, shortcut: &str) -> Result<Range> {
        let a = date(from)?;
        let b = date(to)?;
        if a > b {
            return Err(invalid("Range start is after its end"));
        }
        let days = (b - a).num_days() + 1;
        if days > MAX_DAYS {
            return Err(invalid(format!("Range is too long (max {MAX_DAYS} days)")));
        }
        let prev_to = a - Duration::days(1);
        let prev_from = prev_to - Duration::days(days - 1);
        Ok(Range {
            from: fmt(a),
            to: fmt(b),
            shortcut: shortcut.to_string(),
            days,
            granularity: granularity_for(days),
            prev_from: fmt(prev_from),
            prev_to: fmt(prev_to),
            from_d: a,
            to_d: b,
        })
    }

    /// From request parameters (`range` shortcut or `from`/`to`); `now` is a
    /// unix timestamp, `first_day` the first day with data (for `all`).
    pub fn from_query(q: &Query, tz: Tz, now: i64, first_day: Option<&str>) -> Result<Range> {
        let from = q.get("from").filter(|s| !s.is_empty());
        let to = q.get("to").filter(|s| !s.is_empty());
        if from.is_some() || to.is_some() {
            return match (from, to) {
                (Some(f), Some(t)) => Range::new(f, t, "custom"),
                _ => Err(invalid(
                    "Both \"from\" and \"to\" are required for a custom range",
                )),
            };
        }
        let range = q.get("range").filter(|s| !s.is_empty()).unwrap_or(DEFAULT);
        Range::shortcut(range, tz, now, first_day)
    }

    /// A named shortcut evaluated at `now` in `tz`.
    pub fn shortcut(name: &str, tz: Tz, now: i64, first_day: Option<&str>) -> Result<Range> {
        let today = tz
            .timestamp_opt(now, 0)
            .single()
            .map(|d| d.date_naive())
            .unwrap_or_default();
        let back = |n: i64| today - Duration::days(n);
        let first_of_month = today.with_day(1).unwrap_or(today);
        match name {
            "today" => Range::new(&fmt(today), &fmt(today), name),
            "yesterday" => Range::new(&fmt(back(1)), &fmt(back(1)), name),
            "7d" => Range::new(&fmt(back(6)), &fmt(today), name),
            "30d" => Range::new(&fmt(back(29)), &fmt(today), name),
            "90d" => Range::new(&fmt(back(89)), &fmt(today), name),
            "12m" => {
                let start = first_of_month
                    .checked_sub_months(Months::new(11))
                    .unwrap_or(first_of_month);
                Range::new(&fmt(start), &fmt(today), name)
            }
            "month" => Range::new(&fmt(first_of_month), &fmt(today), name),
            "lastmonth" => {
                let first = first_of_month
                    .checked_sub_months(Months::new(1))
                    .unwrap_or(first_of_month);
                let last = first_of_month - Duration::days(1);
                Range::new(&fmt(first), &fmt(last), name)
            }
            "year" => Range::new(&format!("{:04}-01-01", today.year()), &fmt(today), name),
            "all" => {
                let t = fmt(today);
                let start = match first_day {
                    Some(f) if is_date(f) && f <= t.as_str() => f.to_string(),
                    _ => t.clone(),
                };
                let min_start = fmt(back(MAX_DAYS - 1));
                let from = if start >= min_start { start } else { min_start };
                Range::new(&from, &t, name)
            }
            _ => Err(invalid(format!("Unknown range \"{name}\""))),
        }
    }

    /// The preceding period as its own range.
    pub fn previous(&self) -> Range {
        Range::new(&self.prev_from, &self.prev_to, "custom").unwrap_or_else(|_| self.clone())
    }

    /// Every local day in the range, in order.
    pub fn day_list(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.days as usize);
        let mut cur = self.from_d;
        while cur <= self.to_d {
            out.push(fmt(cur));
            cur += Duration::days(1);
        }
        out
    }

    /// Bucket labels: `YYYY-MM-DD HH:00` (hour), `YYYY-MM-DD` (day), the first
    /// day of the ISO week clipped to the range (week), `YYYY-MM` (month).
    pub fn labels(&self, granularity: &str) -> Vec<String> {
        if granularity == "hour" {
            let mut out = Vec::with_capacity(self.days as usize * 24);
            for day in self.day_list() {
                for h in 0..24 {
                    out.push(format!("{day} {h:02}:00"));
                }
            }
            return out;
        }
        let mut out: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for day in self.day_list() {
            let k = self.bucket_key(&day, granularity);
            if seen.insert(k.clone()) {
                out.push(k);
            }
        }
        out
    }

    /// Map of local day → bucket index for day/week/month granularity.
    pub fn day_index(&self, granularity: &str) -> HashMap<String, usize> {
        let g = if granularity == "hour" {
            "day"
        } else {
            granularity
        };
        let mut map = HashMap::new();
        let mut idx: HashMap<String, usize> = HashMap::new();
        for day in self.day_list() {
            let k = self.bucket_key(&day, g);
            let n = idx.len();
            let i = *idx.entry(k).or_insert(n);
            map.insert(day, i);
        }
        map
    }

    fn bucket_key(&self, day: &str, g: &str) -> String {
        match g {
            "month" => day[..7].to_string(),
            "week" => {
                let d = parse_date(day).unwrap_or(self.from_d);
                let monday = fmt(d - Duration::days(d.weekday().num_days_from_monday() as i64));
                if monday >= self.from {
                    monday
                } else {
                    self.from.clone()
                }
            }
            _ => day.to_string(),
        }
    }

    /// `{"from","to","days","granularity","prev":{"from","to"}}`.
    pub fn to_json(&self) -> Value {
        json!({
            "from": self.from,
            "to": self.to,
            "days": self.days,
            "granularity": self.granularity,
            "prev": {"from": self.prev_from, "to": self.prev_to},
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn at(s: &str, tz: Tz) -> i64 {
        let n = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        tz.from_local_datetime(&n).single().unwrap().timestamp()
    }
    fn r(name: &str, now: &str) -> Range {
        Range::shortcut(
            name,
            chrono_tz::Europe::Rome,
            at(now, chrono_tz::Europe::Rome),
            None,
        )
        .unwrap()
    }

    #[test]
    fn shortcuts() {
        let exp = [
            ("today", "2026-10-16", "2026-10-16", 1, "hour"),
            ("yesterday", "2026-10-15", "2026-10-15", 1, "hour"),
            ("7d", "2026-10-10", "2026-10-16", 7, "day"),
            ("30d", "2026-09-17", "2026-10-16", 30, "day"),
            ("90d", "2026-07-19", "2026-10-16", 90, "day"),
            ("12m", "2025-11-01", "2026-10-16", 350, "week"),
            ("month", "2026-10-01", "2026-10-16", 16, "day"),
            ("lastmonth", "2026-09-01", "2026-09-30", 30, "day"),
            ("year", "2026-01-01", "2026-10-16", 289, "week"),
        ];
        for (name, from, to, days, g) in exp {
            let x = r(name, "2026-10-16 12:00");
            assert_eq!(
                (x.from.as_str(), x.to.as_str(), x.days, x.granularity),
                (from, to, days, g),
                "{name}"
            );
        }
        let x = r("30d", "2026-10-16 09:00");
        assert_eq!(
            (x.prev_from.as_str(), x.prev_to.as_str()),
            ("2026-08-18", "2026-09-16")
        );
    }

    #[test]
    fn all_and_boundaries() {
        let rome = chrono_tz::Europe::Rome;
        let now = at("2026-10-16 10:00", rome);
        assert_eq!(
            Range::shortcut("all", rome, now, Some("2026-09-02"))
                .unwrap()
                .from,
            "2026-09-02"
        );
        assert_eq!(
            Range::shortcut("all", rome, now, None).unwrap().from,
            "2026-10-16"
        );
        assert_eq!(
            Range::shortcut("all", rome, now, Some("garbage"))
                .unwrap()
                .from,
            "2026-10-16"
        );
        let x = r("lastmonth", "2026-03-01 00:30");
        assert_eq!(
            (x.from.as_str(), x.to.as_str(), x.days),
            ("2026-02-01", "2026-02-28", 28)
        );
        let x = r("lastmonth", "2024-03-31 23:00");
        assert_eq!(
            (x.from.as_str(), x.to.as_str()),
            ("2024-02-01", "2024-02-29")
        );
        let x = r("lastmonth", "2026-01-15 12:00");
        assert_eq!(
            (x.from.as_str(), x.to.as_str()),
            ("2025-12-01", "2025-12-31")
        );
        let x = r("month", "2026-05-01 08:00");
        assert_eq!(
            (x.granularity, x.prev_from.as_str(), x.prev_to.as_str()),
            ("hour", "2026-04-30", "2026-04-30")
        );
        assert_eq!(r("12m", "2026-03-31 12:00").from, "2025-04-01");
    }

    #[test]
    fn timezone_decides_today() {
        let utc = chrono_tz::UTC;
        let late = at("2026-10-16 23:30", utc);
        assert_eq!(
            Range::shortcut("today", chrono_tz::Europe::Rome, late, None)
                .unwrap()
                .from,
            "2026-10-17"
        );
        assert_eq!(
            Range::shortcut("today", chrono_tz::America::New_York, late, None)
                .unwrap()
                .from,
            "2026-10-16"
        );
        assert_eq!(
            Range::shortcut("today", chrono_tz::Pacific::Kiritimati, late, None)
                .unwrap()
                .from,
            "2026-10-17"
        );
        let early = at("2026-10-16 00:30", utc);
        assert_eq!(
            Range::shortcut("yesterday", utc, early, None).unwrap().from,
            "2026-10-15"
        );
        assert_eq!(
            Range::shortcut("today", chrono_tz::America::Los_Angeles, early, None)
                .unwrap()
                .from,
            "2026-10-15"
        );
    }

    #[test]
    fn labels_and_index() {
        let x = Range::new("2026-10-20", "2026-10-30", "custom").unwrap();
        assert_eq!(
            (x.days, x.prev_from.as_str(), x.prev_to.as_str()),
            (11, "2026-10-09", "2026-10-19")
        );
        let x = Range::new("2026-10-15", "2026-10-16", "custom").unwrap();
        let l = x.labels(x.granularity);
        assert_eq!(
            (l.len(), l[0].as_str(), l[47].as_str()),
            (48, "2026-10-15 00:00", "2026-10-16 23:00")
        );
        let w = Range::new("2026-06-03", "2026-10-16", "custom").unwrap();
        assert_eq!(w.granularity, "week");
        let wl = w.labels("week");
        assert_eq!(
            (wl[0].as_str(), wl[1].as_str()),
            ("2026-06-03", "2026-06-08")
        );
        let idx = w.day_index("week");
        assert_eq!(
            (idx["2026-06-07"], idx["2026-06-08"], idx["2026-10-16"]),
            (0, 1, wl.len() - 1)
        );
        let m = Range::new("2025-01-15", "2026-10-16", "custom").unwrap();
        assert_eq!(
            (
                m.granularity,
                m.labels("month")[0].as_str(),
                m.labels("month")[21].as_str()
            ),
            ("month", "2025-01", "2026-10")
        );
        assert_eq!(granularity_for(2), "hour");
        assert_eq!(granularity_for(121), "week");
        assert_eq!(granularity_for(401), "month");
    }

    #[test]
    fn validation() {
        let rome = chrono_tz::Europe::Rome;
        let now = at("2026-10-16 12:00", rome);
        assert_eq!(
            Range::from_query(&Query::new(), rome, now, None)
                .unwrap()
                .shortcut,
            "30d"
        );
        let q = Query::from_pairs([
            ("from", "2026-02-01"),
            ("to", "2026-02-28"),
            ("range", "7d"),
        ]);
        assert_eq!(
            Range::from_query(&q, rome, now, None).unwrap().shortcut,
            "custom"
        );
        for bad in [
            vec![("range", "fortnight")],
            vec![("from", "2026-02-01")],
            vec![("from", "2026-02-30"), ("to", "2026-03-01")],
            vec![("from", "2026-03-02"), ("to", "2026-03-01")],
            vec![("from", "2026-03-01'--"), ("to", "2026-03-02")],
            vec![("from", "1990-01-01"), ("to", "2026-03-02")],
        ] {
            assert!(
                Range::from_query(&Query::from_pairs(bad.clone()), rome, now, None).is_err(),
                "{bad:?}"
            );
        }
        assert!(!is_date("2026-10-16\n"));
        assert!(is_date("2026-10-16"));
        assert!(!is_date("0000-01-01"));
    }
}
