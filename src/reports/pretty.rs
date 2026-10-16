//! Human-readable labels for dimension keys (country names, HTTP status
//! texts, channel/device/bucket names) and number formatting for the CLI
//! (port of `Reports\Pretty`).

use super::data::country_name;
use super::dimension;
use super::php;

/// Label for a key of the given dim.
pub fn label(dim: &str, key: &str) -> String {
    match dimension::pretty(dim) {
        "country" => country(key),
        "status" => status(key),
        "channel" => match key {
            "direct" => "Direct".into(),
            "organic" => "Organic search".into(),
            "social" => "Social".into(),
            "referral" => "Referral".into(),
            "paid" => "Paid".into(),
            "email" => "Email".into(),
            "campaign" => "Campaign".into(),
            _ => php::ucfirst(key),
        },
        "device" => match key {
            "desktop" => "Desktop".into(),
            "mobile" => "Mobile".into(),
            "tablet" => "Tablet".into(),
            "other" => "Other".into(),
            _ => php::ucfirst(key),
        },
        "engine" => match key {
            "google" => "Google".into(),
            "bing" => "Bing".into(),
            "duckduckgo" => "DuckDuckGo".into(),
            "yahoo" => "Yahoo".into(),
            "yandex" => "Yandex".into(),
            "baidu" => "Baidu".into(),
            "ecosia" => "Ecosia".into(),
            "qwant" => "Qwant".into(),
            "startpage" => "Startpage".into(),
            "brave" => "Brave Search".into(),
            "naver" => "Naver".into(),
            "seznam" => "Seznam".into(),
            "ask" => "Ask".into(),
            "aol" => "AOL".into(),
            _ => php::ucfirst(key),
        },
        "bucket" => bucket(dim, key),
        "upper" => key.to_ascii_uppercase(),
        _ => key.to_string(),
    }
}

/// True when the dim's label differs from the key (worth sending to the UI).
pub fn has_labels(dim: &str) -> bool {
    matches!(
        dimension::pretty(dim),
        "country" | "status" | "channel" | "device" | "engine" | "bucket" | "upper"
    )
}

/// English country name for a lowercase alpha-2 code, `Unknown` for `(unknown)`.
pub fn country(code: &str) -> String {
    let c = code.to_ascii_lowercase();
    match country_name(&c) {
        Some(n) => n.to_string(),
        None if c.is_empty() || c.starts_with('(') => "Unknown".into(),
        None => code.to_ascii_uppercase(),
    }
}

/// `404 Not Found`.
pub fn status(code: &str) -> String {
    let n = php::intval(code);
    match status_text(n) {
        Some(t) => format!("{n} {t}"),
        None => code.to_string(),
    }
}

/// Reason phrase of the status codes 0.1.0 knows.
pub fn status_text(n: i64) -> Option<&'static str> {
    Some(match n {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        418 => "I'm a teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        444 => "No Response",
        451 => "Unavailable For Legal Reasons",
        499 => "Client Closed Request",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        508 => "Loop Detected",
        _ => return None,
    })
}

/// Engagement bucket label (`2–5 min`, `6–10 pages`).
pub fn bucket(dim: &str, key: &str) -> String {
    if dim == "pages_per_visit" {
        return if key == "1" {
            "1 page".into()
        } else {
            format!("{} pages", key.replace('-', "–"))
        };
    }
    match key {
        "0-10s" => "0–10 s",
        "11-30s" => "11–30 s",
        "31-60s" => "31–60 s",
        "1-2m" => "1–2 min",
        "2-5m" => "2–5 min",
        "5-10m" => "5–10 min",
        "10-30m" => "10–30 min",
        "30m+" => "30 min +",
        other => other,
    }
    .to_string()
}

/// `1.2 MB` (1024-based).
pub fn bytes(n: f64) -> String {
    const U: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut i = 0;
    let mut v = n;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    let num = if i == 0 {
        php::float_to_int(v).to_string()
    } else {
        php::number_format(v, if v >= 100.0 { 0 } else { 1 })
    };
    format!("{num} {}", U[i])
}

/// `1m 32s`.
pub fn duration(s: f64) -> String {
    let s = php::round_int(s);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(label("country", "it"), "Italy");
        assert_eq!(label("country", "(unknown)"), "Unknown");
        assert_eq!(label("country", "xx"), "XX");
        assert_eq!(label("status", "404"), "404 Not Found");
        assert_eq!(label("status", "299"), "299");
        assert_eq!(label("pages_per_visit", "6-10"), "6–10 pages");
        assert_eq!(label("visit_duration", "30m+"), "30 min +");
        assert_eq!(label("method", "get"), "GET");
        assert_eq!(label("channel", "weird"), "Weird");
        assert_eq!(bytes(1536.0), "1.5 KB");
        assert_eq!(bytes(512.0), "512 B");
        assert_eq!(duration(92.0), "1m 32s");
        assert_eq!(duration(3700.0), "1h 1m");
    }
}
