//! Clock helpers. `WEBPULSE_NOW` (unix seconds or RFC 3339) freezes "now" for tests.

use chrono::{DateTime, TimeZone, Utc};

/// Current unix time, honouring `WEBPULSE_NOW`.
pub fn now_ts() -> i64 {
    if let Ok(v) = std::env::var("WEBPULSE_NOW") {
        let v = v.trim();
        if let Ok(n) = v.parse::<i64>() {
            return n;
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(v) {
            return dt.timestamp();
        }
    }
    Utc::now().timestamp()
}

/// Current time as a UTC `DateTime`, honouring `WEBPULSE_NOW`.
pub fn now_utc() -> DateTime<Utc> {
    Utc.timestamp_opt(now_ts(), 0).single().unwrap_or_else(Utc::now)
}

/// Parses an IANA timezone name, falling back to the system zone (`TZ` env or UTC).
pub fn tz_or_server(name: Option<&str>) -> chrono_tz::Tz {
    if let Some(n) = name {
        if let Ok(tz) = n.parse::<chrono_tz::Tz>() {
            return tz;
        }
    }
    server_tz()
}

/// The server timezone: `TZ` env, else `/etc/localtime` → zoneinfo name, else UTC.
pub fn server_tz() -> chrono_tz::Tz {
    if let Ok(tz) = std::env::var("TZ") {
        if let Ok(t) = tz.trim_start_matches(':').parse::<chrono_tz::Tz>() {
            return t;
        }
    }
    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        let s = link.to_string_lossy();
        if let Some(pos) = s.find("zoneinfo/") {
            if let Ok(t) = s[pos + 9..].parse::<chrono_tz::Tz>() {
                return t;
            }
        }
    }
    chrono_tz::UTC
}
