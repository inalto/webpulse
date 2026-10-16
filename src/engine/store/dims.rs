//! Dimension names written to `dim_daily.dim` (docs/DATA-MODEL.md), `WebPulse\Store\Dims`.
//! Mirrored by the read side; the values are fixed.

/// Every dimension, in the documented order.
pub const ALL: &[&str] = &[
    "page",
    "entry_page",
    "exit_page",
    "referrer_host",
    "referrer_url",
    "channel",
    "search_engine",
    "keyword",
    "campaign",
    "utm_source",
    "utm_medium",
    "source_medium",
    "country",
    "city",
    "browser",
    "browser_version",
    "os",
    "os_version",
    "device",
    "status",
    "method",
    "protocol",
    "http_version",
    "file_type",
    "static_file",
    "download",
    "not_found",
    "not_found_referrer",
    "server_error",
    "bot",
    "visit_duration",
    "pages_per_visit",
];

/// Visit-level dimensions: attributed once per visit (first hit) on the visit's day;
/// they carry visits, pageviews, bounces and duration.
pub const VISIT_LEVEL: &[&str] = &[
    "channel",
    "referrer_host",
    "referrer_url",
    "search_engine",
    "keyword",
    "campaign",
    "utm_source",
    "utm_medium",
    "source_medium",
    "country",
    "city",
    "browser",
    "browser_version",
    "os",
    "os_version",
    "device",
];

/// Separator used in `not_found_referrer` keys: `"/path ← host"`.
pub const NF_REF_SEP: &str = " \u{2190} ";

/// Key used when rollup folds the tail of a dimension.
pub const OTHER: &str = "(other)";

/// Engagement bucket keys (docs/METRICS.md).
pub const DURATION_BUCKETS: &[&str] = &[
    "0-10s", "11-30s", "31-60s", "1-2m", "2-5m", "5-10m", "10-30m", "30m+",
];
/// Pages-per-visit bucket keys.
pub const PAGES_BUCKETS: &[&str] = &["1", "2", "3", "4", "5", "6-10", "11-20", "21+"];

/// `dim_daily` value columns, in storage order.
pub const COLUMNS: &[&str] = &[
    "visits",
    "pageviews",
    "hits",
    "bytes",
    "bounces",
    "duration",
];

/// Bucket key for a visit duration in seconds.
pub fn duration_bucket(sec: i64) -> &'static str {
    match sec {
        s if s <= 10 => "0-10s",
        s if s <= 30 => "11-30s",
        s if s <= 60 => "31-60s",
        s if s <= 120 => "1-2m",
        s if s <= 300 => "2-5m",
        s if s <= 600 => "5-10m",
        s if s <= 1800 => "10-30m",
        _ => "30m+",
    }
}

/// Bucket key for a pageview count (>= 1).
pub fn pages_bucket(pv: i64) -> &'static str {
    match pv {
        p if p <= 1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        p if p <= 10 => "6-10",
        p if p <= 20 => "11-20",
        _ => "21+",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lists_and_buckets() {
        assert_eq!(ALL.len(), 32);
        let mut u = ALL.to_vec();
        u.sort();
        u.dedup();
        assert_eq!(u.len(), 32);
        assert!(VISIT_LEVEL.iter().all(|d| ALL.contains(d)));
        let d: Vec<&str> = [0, 10, 11, 60, 61, 300, 600, 1800, 1801]
            .iter()
            .map(|&s| duration_bucket(s))
            .collect();
        assert_eq!(
            d,
            [
                "0-10s", "0-10s", "11-30s", "31-60s", "1-2m", "2-5m", "5-10m", "10-30m", "30m+"
            ]
        );
        let p: Vec<&str> = [1, 2, 5, 6, 20, 21]
            .iter()
            .map(|&n| pages_bucket(n))
            .collect();
        assert_eq!(p, ["1", "2", "5", "6-10", "11-20", "21+"]);
    }
}
