//! The whitelist of dimension names stored in `dim_daily.dim` and the
//! per-dimension metadata used by the API, the exporter and the UI
//! (port of `Reports\Dimension`). `ALL` must equal the engine's list
//! (`engine::store::dims::ALL`, docs/DATA-MODEL.md).

use serde_json::{Map, Value, json};

/// Every dimension name, in the documented order.
pub const ALL: [&str; 32] = [
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

/// Separator inside `not_found_referrer` keys: `path ← referrer_host`.
pub const NF_REF_SEP: &str = " ← ";

/// Fixed order of the visit duration buckets.
pub const DURATION_BUCKETS: [&str; 8] = [
    "0-10s", "11-30s", "31-60s", "1-2m", "2-5m", "5-10m", "10-30m", "30m+",
];
/// Fixed order of the pages-per-visit buckets.
pub const PAGES_BUCKETS: [&str; 8] = ["1", "2", "3", "4", "5", "6-10", "11-20", "21+"];

const WITH_BOUNCES: [&str; 9] = [
    "entry_page",
    "channel",
    "country",
    "device",
    "browser",
    "os",
    "referrer_host",
    "search_engine",
    "campaign",
];

const VISIT_LEVEL: [&str; 20] = [
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
    "visit_duration",
    "pages_per_visit",
];

const HIT_LEVEL: [&str; 11] = [
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
];

/// Base columns of `dim_daily` that can be summed.
pub const COLUMNS: [&str; 6] = [
    "visits",
    "pageviews",
    "hits",
    "bytes",
    "bounces",
    "duration",
];

/// True when `dim` is a known dimension name.
pub fn is_valid(dim: &str) -> bool {
    ALL.contains(&dim)
}

/// Human label of a dimension (`Pages`, `Countries`).
pub fn label(dim: &str) -> &str {
    match dim {
        "page" => "Pages",
        "entry_page" => "Entry pages",
        "exit_page" => "Exit pages",
        "referrer_host" => "Referrers",
        "referrer_url" => "Referrer URLs",
        "channel" => "Channels",
        "search_engine" => "Search engines",
        "keyword" => "Keywords",
        "campaign" => "Campaigns",
        "utm_source" => "UTM sources",
        "utm_medium" => "UTM media",
        "source_medium" => "Source / medium",
        "country" => "Countries",
        "city" => "Cities",
        "browser" => "Browsers",
        "browser_version" => "Browser versions",
        "os" => "Operating systems",
        "os_version" => "OS versions",
        "device" => "Devices",
        "status" => "HTTP status",
        "method" => "Methods",
        "protocol" => "Protocols",
        "http_version" => "HTTP versions",
        "file_type" => "File types",
        "static_file" => "Static files",
        "download" => "Downloads",
        "not_found" => "Not found (404)",
        "not_found_referrer" => "404 referrers",
        "server_error" => "Server errors (5xx)",
        "bot" => "Crawlers",
        "visit_duration" => "Visit duration",
        "pages_per_visit" => "Pages per visit",
        other => other,
    }
}

/// Visit-level dims (one row per visit, counted in `visits`).
pub fn is_visit_level(dim: &str) -> bool {
    VISIT_LEVEL.contains(&dim)
}

/// Hit-level dims (counted in `hits`/`bytes`).
pub fn is_hit_level(dim: &str) -> bool {
    HIT_LEVEL.contains(&dim)
}

/// Visit-level dims that also carry bounces and duration.
pub fn has_bounces(dim: &str) -> bool {
    WITH_BOUNCES.contains(&dim)
}

/// Engagement bucket dims are ordered by bucket, not by value.
pub fn is_bucket(dim: &str) -> bool {
    dim == "visit_duration" || dim == "pages_per_visit"
}

/// Metric used for `share` and the default sort.
pub fn primary_metric(dim: &str) -> &'static str {
    if dim == "page" {
        "pageviews"
    } else if dim == "static_file" || dim == "file_type" {
        "bytes"
    } else if is_hit_level(dim) {
        "hits"
    } else {
        "visits"
    }
}

/// Default `sort` value.
pub fn default_sort(dim: &str) -> String {
    if is_bucket(dim) {
        "bucket".into()
    } else {
        format!("-{}", primary_metric(dim))
    }
}

/// Columns (base and derived) that are meaningful for the dim, in display order.
pub fn metrics(dim: &str) -> Vec<&'static str> {
    if dim == "page" {
        vec!["pageviews", "visits", "hits", "bytes", "exit_rate"]
    } else if dim == "entry_page" {
        vec![
            "visits",
            "pageviews",
            "bounces",
            "bounce_rate",
            "avg_duration",
        ]
    } else if dim == "exit_page" {
        vec!["visits", "exit_rate"]
    } else if is_bucket(dim) {
        vec!["visits"]
    } else if has_bounces(dim) {
        vec!["visits", "pageviews", "bounce_rate", "avg_duration"]
    } else if is_visit_level(dim) {
        vec!["visits", "pageviews"]
    } else {
        vec!["hits", "bytes"]
    }
}

/// Prettifier hint: path|url|host|country|status|channel|device|bucket|engine|upper|text.
pub fn pretty(dim: &str) -> &'static str {
    match dim {
        "page" | "entry_page" | "exit_page" | "not_found" | "server_error" | "static_file"
        | "download" => "path",
        "referrer_url" => "url",
        "referrer_host" => "host",
        "country" => "country",
        "status" => "status",
        "channel" => "channel",
        "device" => "device",
        "visit_duration" | "pages_per_visit" => "bucket",
        "search_engine" => "engine",
        "protocol" | "method" => "upper",
        _ => "text",
    }
}

/// Metadata of every dim, shipped to the UI in the page bootstrap.
pub fn meta() -> Value {
    let mut out = Map::new();
    for dim in ALL {
        out.insert(
            dim.to_string(),
            json!({
                "label": label(dim),
                "visit_level": is_visit_level(dim),
                "bounces": has_bounces(dim),
                "sort": default_sort(dim),
                "metrics": metrics(dim),
                "pretty": pretty(dim),
                "share": primary_metric(dim),
            }),
        );
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list must equal the one documented in docs/DATA-MODEL.md (and the
    /// engine's `Dims::ALL`, checked by tests/http_dims.rs once it exists).
    #[test]
    fn all_matches_data_model_doc() {
        let doc = include_str!("../../docs/DATA-MODEL.md");
        let start = doc
            .find("`Dimension::ALL`:")
            .expect("list in DATA-MODEL.md");
        let rest = &doc[start..];
        let open = rest.find("\n  `").expect("list start") + 4;
        let close = rest[open..].find('`').expect("list end") + open;
        let names: Vec<&str> = rest[open..close].split_whitespace().collect();
        assert_eq!(names, ALL.to_vec());
        assert_eq!(meta().as_object().map(|m| m.len()), Some(32));
        assert_eq!(default_sort("visit_duration"), "bucket");
        assert_eq!(primary_metric("static_file"), "bytes");
    }
}
