//! Clock helpers. `WEBPULSE_NOW` (unix seconds or RFC 3339) freezes "now" for tests.
//!
//! The frozen clock is a test hook: it is honoured when the process is not root, or
//! when it runs on a configuration other than the production `/etc/webpulse/config.json`
//! (decided by `App` at boot through [`set_fake_clock_allowed`]). Before that decision,
//! only unprivileged processes honour it.

use std::sync::atomic::{AtomicU8, Ordering};

use chrono::{DateTime, TimeZone, Utc};

/// 0 = undecided (honour only when not root), 1 = allowed, 2 = denied.
static FAKE_CLOCK: AtomicU8 = AtomicU8::new(0);

/// Decides whether `WEBPULSE_NOW` is honoured by this process.
pub fn set_fake_clock_allowed(allowed: bool) {
    FAKE_CLOCK.store(if allowed { 1 } else { 2 }, Ordering::Relaxed);
}

/// Whether `WEBPULSE_NOW` may be honoured: when not root, or when the configuration
/// in use is not the production file.
pub fn fake_clock_policy(root: bool, production_config: bool) -> bool {
    !root || !production_config
}

fn fake_clock_allowed() -> bool {
    match FAKE_CLOCK.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => !crate::util::fs::is_root(),
    }
}

/// Current unix time, honouring `WEBPULSE_NOW` (see the module documentation).
pub fn now_ts() -> i64 {
    if !fake_clock_allowed() {
        return Utc::now().timestamp();
    }
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
    Utc.timestamp_opt(now_ts(), 0)
        .single()
        .unwrap_or_else(Utc::now)
}

/// Parses an IANA timezone name, falling back to the system zone (`TZ` env or UTC).
pub fn tz_or_server(name: Option<&str>) -> chrono_tz::Tz {
    if let Some(n) = name
        && let Ok(tz) = n.parse::<chrono_tz::Tz>()
    {
        return tz;
    }
    server_tz()
}

/// The server timezone: `TZ` env, else `/etc/localtime` → zoneinfo name, else UTC.
pub fn server_tz() -> chrono_tz::Tz {
    if let Ok(tz) = std::env::var("TZ")
        && let Ok(t) = tz.trim_start_matches(':').parse::<chrono_tz::Tz>()
    {
        return t;
    }
    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        let s = link.to_string_lossy();
        if let Some(pos) = s.find("zoneinfo/")
            && let Ok(t) = s[pos + 9..].parse::<chrono_tz::Tz>()
        {
            return t;
        }
    }
    chrono_tz::UTC
}

use chrono::{Datelike, LocalResult, NaiveDate, NaiveDateTime, Offset, Timelike};
use foldhash::HashMap;
use std::sync::Arc;

/// Continents of PHP's `DateTimeZone::listIdentifiers()` (plus `UTC`).
const TZ_GROUPS: &[&str] = &[
    "Africa/",
    "America/",
    "Antarctica/",
    "Arctic/",
    "Asia/",
    "Atlantic/",
    "Australia/",
    "Europe/",
    "Indian/",
    "Pacific/",
];

/// True for a timezone identifier PHP lists in `DateTimeZone::listIdentifiers()`
/// (an IANA zone of one of the continent groups, or `UTC`).
pub fn is_valid_tz_identifier(name: &str) -> bool {
    if name == "UTC" {
        return true;
    }
    TZ_GROUPS.iter().any(|g| name.starts_with(g)) && name.parse::<chrono_tz::Tz>().is_ok()
}

/// Unix timestamp of a local wall-clock time in `tz`, resolved like PHP's
/// `new DateTimeImmutable('Y-m-d H:i:s', $tz)`: the earlier instant when ambiguous,
/// moved forward by the gap when the time does not exist.
pub fn local_to_ts(tz: &chrono_tz::Tz, naive: NaiveDateTime) -> i64 {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(t) => t.timestamp(),
        LocalResult::Ambiguous(a, b) => a.timestamp().min(b.timestamp()),
        LocalResult::None => {
            // offset in effect before the gap
            let before = naive - chrono::Duration::hours(24);
            let off = match tz.from_local_datetime(&before) {
                LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => {
                    t.offset().fix().local_minus_utc() as i64
                }
                LocalResult::None => 0,
            };
            naive.and_utc().timestamp() - off
        }
    }
}

/// Parses `YYYY-MM-DD`.
pub fn parse_day(day: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

/// Unix timestamp of local midnight of `day` in `tz` (0 for an invalid day).
pub fn local_midnight(day: &str, tz: &chrono_tz::Tz) -> i64 {
    match parse_day(day) {
        Some(d) => local_to_ts(tz, d.and_hms_opt(0, 0, 0).unwrap_or_default()),
        None => 0,
    }
}

/// `$day` shifted by `n` days (`DayClock::addDays`).
pub fn add_days(day: &str, n: i64) -> String {
    match parse_day(day) {
        Some(d) => (d + chrono::Duration::days(n))
            .format("%Y-%m-%d")
            .to_string(),
        None => day.to_string(),
    }
}

/// The day before `day` (`DayClock::prevDay`).
pub fn prev_day(day: &str) -> String {
    add_days(day, -1)
}

/// Fast unix timestamp → local (day, hour, local-midnight) conversion for one
/// timezone, cached per 15-minute bucket (`WebPulse\Util\DayClock`; exact for every
/// real timezone since offsets and transitions are multiples of 15 minutes).
#[derive(Debug, Clone)]
pub struct DayClock {
    tz: chrono_tz::Tz,
    cache: HashMap<i64, (Arc<str>, u32, i64)>,
}

impl DayClock {
    /// Invalid identifiers fall back to the server timezone.
    pub fn new(timezone: &str) -> DayClock {
        DayClock {
            tz: tz_or_server(Some(timezone)),
            cache: HashMap::default(),
        }
    }

    /// The timezone in use.
    pub fn tz(&self) -> &chrono_tz::Tz {
        &self.tz
    }

    /// Name of the timezone in use.
    pub fn timezone(&self) -> String {
        self.tz.name().to_string()
    }

    /// `(Y-m-d, hour 0-23, unix ts of local midnight)` of `ts`.
    pub fn at(&mut self, ts: i64) -> (Arc<str>, u32, i64) {
        let b = ts.div_euclid(900);
        if let Some(v) = self.cache.get(&b) {
            return v.clone();
        }
        if self.cache.len() > 20000 {
            self.cache.clear();
        }
        let local = self
            .tz
            .timestamp_opt(ts, 0)
            .single()
            .unwrap_or_else(|| self.tz.timestamp_opt(0, 0).unwrap());
        let d = local.date_naive();
        let day: Arc<str> =
            Arc::from(format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()).as_str());
        let mid = local_to_ts(&self.tz, d.and_hms_opt(0, 0, 0).unwrap_or_default());
        let v = (day, local.hour(), mid);
        self.cache.insert(b, v.clone());
        v
    }

    /// Local day of `ts`.
    pub fn day(&mut self, ts: i64) -> Arc<str> {
        self.at(ts).0
    }
}

/// Formats `ts` in `tz` with a chrono format string.
pub fn format_ts(ts: i64, tz: &chrono_tz::Tz, fmt: &str) -> String {
    match tz.timestamp_opt(ts, 0).single() {
        Some(t) => t.format(fmt).to_string(),
        None => String::new(),
    }
}

/// `gmdate($fmt, $ts)` with a chrono format string.
pub fn gm_format(ts: i64, fmt: &str) -> String {
    match Utc.timestamp_opt(ts, 0).single() {
        Some(t) => t.format(fmt).to_string(),
        None => String::new(),
    }
}

/// CPU time (user + system, seconds) used so far by the terminated and waited-for
/// children of this process (`getrusage(RUSAGE_CHILDREN)`); load-independent timing
/// for tests and benchmarks.
pub fn children_cpu_seconds() -> f64 {
    // SAFETY: a zeroed rusage is a valid out-parameter; getrusage only writes it.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: valid pointer to a local rusage.
    if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut ru) } != 0 {
        return 0.0;
    }
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn day_clock_and_days() {
        let mut c = DayClock::new("Europe/Rome");
        // 2026-10-10 23:30 UTC = 2026-10-11 01:30 Rome
        let (d, h, mid) = c.at(1_791_675_000);
        assert_eq!(&*d, "2026-10-11");
        assert_eq!(h, 1);
        assert_eq!(mid, 1_791_669_600);
        assert_eq!(prev_day("2026-03-01"), "2026-02-28");
        assert_eq!(add_days("2026-12-31", 1), "2027-01-01");
        assert!(is_valid_tz_identifier("Europe/Rome"));
        assert!(!is_valid_tz_identifier("Mars/Base"));
        assert!(!is_valid_tz_identifier("Etc/GMT+1"));
    }
}
