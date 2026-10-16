//! Shared fixture of the HTTP/report integration tests (R2): a port of
//! `tests/support/seed_db.php` producing the *same* deterministic data (PHP's
//! MT19937 `mt_rand`, `crc32`, `round()` and float arithmetic reproduced), so
//! expected values can be cross-checked against the PHP tests; plus helpers
//! to call the API in-process, run the binary and parse CGI output.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use chrono::{Datelike, Duration, NaiveDate, TimeZone};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use webpulse::app::{App, Global};
use webpulse::http::{Ctx, Principal, Request, Response, api};
use webpulse::reports::Query;
use webpulse::reports::php::round;

/// Fixed clock of the shared fixture: 2026-10-16 12:00 Europe/Rome.
pub const NOW: i64 = 1792144800;

pub const DOMAIN_DDL: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE daily (day TEXT PRIMARY KEY, visits INTEGER NOT NULL DEFAULT 0, visitors INTEGER NOT NULL DEFAULT 0,
  pageviews INTEGER NOT NULL DEFAULT 0, hits INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0,
  bounces INTEGER NOT NULL DEFAULT 0, duration INTEGER NOT NULL DEFAULT 0, downloads INTEGER NOT NULL DEFAULT 0,
  errors_4xx INTEGER NOT NULL DEFAULT 0, errors_5xx INTEGER NOT NULL DEFAULT 0, bot_hits INTEGER NOT NULL DEFAULT 0,
  bot_bytes INTEGER NOT NULL DEFAULT 0);
CREATE TABLE hourly (day TEXT NOT NULL, hour INTEGER NOT NULL, visits INTEGER NOT NULL DEFAULT 0,
  pageviews INTEGER NOT NULL DEFAULT 0, hits INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0,
  bot_hits INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (day, hour));
CREATE TABLE dim_daily (day TEXT NOT NULL, dim TEXT NOT NULL, key TEXT NOT NULL, visits INTEGER NOT NULL DEFAULT 0,
  pageviews INTEGER NOT NULL DEFAULT 0, hits INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0,
  bounces INTEGER NOT NULL DEFAULT 0, duration INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (day, dim, key)) WITHOUT ROWID;
CREATE INDEX dim_daily_dim_day ON dim_daily (dim, day);
CREATE TABLE open_visits (visitor TEXT PRIMARY KEY, visit_id INTEGER NOT NULL, day TEXT NOT NULL, start_ts INTEGER NOT NULL,
  last_ts INTEGER NOT NULL, last_pv_ts INTEGER NOT NULL, pageviews INTEGER NOT NULL, hits INTEGER NOT NULL,
  bytes INTEGER NOT NULL, entry_page TEXT, exit_page TEXT, pages_seen TEXT NOT NULL, attrs TEXT NOT NULL);
CREATE INDEX open_visits_last ON open_visits (last_ts);
CREATE TABLE visits (visit_id INTEGER PRIMARY KEY, visitor TEXT NOT NULL, day TEXT NOT NULL, start_ts INTEGER NOT NULL,
  end_ts INTEGER NOT NULL, duration INTEGER NOT NULL, pageviews INTEGER NOT NULL, hits INTEGER NOT NULL, bytes INTEGER NOT NULL,
  entry_page TEXT, exit_page TEXT, channel TEXT, referrer_host TEXT, search_engine TEXT, keyword TEXT, campaign TEXT,
  country TEXT, city TEXT, browser TEXT, browser_version TEXT, os TEXT, os_version TEXT, device TEXT, ip TEXT);
CREATE INDEX visits_day ON visits (day, start_ts);
CREATE TABLE events (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, visit_id INTEGER, type TEXT NOT NULL, path TEXT NOT NULL,
  status INTEGER NOT NULL, bytes INTEGER NOT NULL, referrer TEXT);
CREATE INDEX events_ts ON events (ts);
CREATE INDEX events_visit ON events (visit_id);
CREATE TABLE visitor_day (day TEXT NOT NULL, visitor TEXT NOT NULL, PRIMARY KEY (day, visitor)) WITHOUT ROWID;
CREATE TABLE checkpoints (path TEXT PRIMARY KEY, inode INTEGER NOT NULL, offset INTEGER NOT NULL, size INTEGER NOT NULL,
  first_line TEXT, last_ts INTEGER, lines INTEGER NOT NULL DEFAULT 0, bad_lines INTEGER NOT NULL DEFAULT 0,
  ignored_lines INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL);
CREATE TABLE imported_archives (path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL, lines INTEGER NOT NULL,
  imported_at INTEGER NOT NULL);
";

pub const REGISTRY_DDL: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE accounts (user TEXT PRIMARY KEY, owner TEXT, enabled INTEGER, timezone TEXT, first_seen INTEGER NOT NULL,
  last_seen INTEGER NOT NULL, removed_at INTEGER);
CREATE TABLE domains (user TEXT NOT NULL, domain TEXT NOT NULL, enabled INTEGER, kind TEXT, first_seen INTEGER NOT NULL,
  last_seen INTEGER NOT NULL, removed_at INTEGER, PRIMARY KEY (user, domain));
CREATE TABLE domain_summary (user TEXT NOT NULL, domain TEXT NOT NULL, db_bytes INTEGER NOT NULL DEFAULT 0,
  last_collect_at INTEGER, last_line_ts INTEGER, lines_total INTEGER NOT NULL DEFAULT 0, visits_today INTEGER NOT NULL DEFAULT 0,
  pageviews_today INTEGER NOT NULL DEFAULT 0, visits_7d INTEGER NOT NULL DEFAULT 0, pageviews_7d INTEGER NOT NULL DEFAULT 0,
  bytes_7d INTEGER NOT NULL DEFAULT 0, visits_30d INTEGER NOT NULL DEFAULT 0, pageviews_30d INTEGER NOT NULL DEFAULT 0,
  bytes_30d INTEGER NOT NULL DEFAULT 0, bot_hits_30d INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (user, domain));
CREATE TABLE salts (day TEXT PRIMARY KEY, salt BLOB NOT NULL);
CREATE TABLE jobs (id INTEGER PRIMARY KEY, type TEXT NOT NULL, scope TEXT, started_at INTEGER NOT NULL, finished_at INTEGER,
  status TEXT NOT NULL, lines INTEGER NOT NULL DEFAULT 0, files INTEGER NOT NULL DEFAULT 0, domains INTEGER NOT NULL DEFAULT 0,
  message TEXT);
";

// ------------------------------------------------------------------ PHP runtime pieces

/// PHP's MT19937 (`mt_srand` / `mt_rand($min, $max)`, `MT_RAND_MT19937` mode).
pub struct Mt {
    s: [u32; 624],
    i: usize,
}

impl Mt {
    pub fn new(seed: u32) -> Mt {
        let mut s = [0u32; 624];
        s[0] = seed;
        for i in 1..624 {
            s[i] = 1812433253u32
                .wrapping_mul(s[i - 1] ^ (s[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        let mut m = Mt { s, i: 0 };
        m.reload();
        m
    }

    fn reload(&mut self) {
        const N: usize = 624;
        const M: usize = 397;
        let twist = |m: u32, u: u32, v: u32| -> u32 {
            let mix = (u & 0x8000_0000) | (v & 0x7fff_ffff);
            m ^ (mix >> 1) ^ (0u32.wrapping_sub(v & 1) & 0x9908_b0df)
        };
        for k in 0..N {
            let m = self.s[(k + M) % N];
            let u = self.s[k];
            let v = self.s[(k + 1) % N];
            self.s[k] = twist(m, u, v);
        }
        self.i = 0;
    }

    fn next(&mut self) -> u32 {
        if self.i >= 624 {
            self.reload();
        }
        let mut y = self.s[self.i];
        self.i += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// `mt_rand($min, $max)`.
    pub fn rand(&mut self, min: i64, max: i64) -> i64 {
        let umax = (max - min) as u32;
        let mut r = self.next();
        if umax == u32::MAX {
            return min + r as i64;
        }
        let n = umax + 1;
        if n & (n - 1) == 0 {
            return min + (r & (n - 1)) as i64;
        }
        let limit = u32::MAX - (u32::MAX % n) - 1;
        while r > limit {
            r = self.next();
        }
        min + (r % n) as i64
    }
}

/// PHP `crc32()`.
pub fn crc32(s: &str) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(s.as_bytes());
    c.sum()
}

/// `(int) round($x)`.
fn ri(x: f64) -> i64 {
    round(x, 0) as i64
}

/// `SeedDb::split()`: splits an integer total over weights (largest remainder).
pub fn split(total: i64, weights: &[f64]) -> Vec<i64> {
    let sum: f64 = weights.iter().fold(0.0, |a, w| a + w);
    if sum <= 0.0 || total <= 0 {
        return vec![0; weights.len()];
    }
    let raw: Vec<f64> = weights.iter().map(|x| (total as f64 * x) / sum).collect();
    let mut out: Vec<i64> = raw.iter().map(|x| x.floor() as i64).collect();
    let mut rem = total - out.iter().sum::<i64>();
    let mut idx: Vec<usize> = (0..raw.len()).collect();
    let frac: Vec<f64> = raw.iter().map(|x| x - x.floor()).collect();
    idx.sort_by(|a, b| {
        frac[*b]
            .partial_cmp(&frac[*a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for i in idx {
        let go = rem > 0;
        rem -= 1;
        if !go {
            break;
        }
        out[i] += 1;
    }
    out
}

type Weights = Vec<(&'static str, f64)>;

fn keys(dim: &str) -> Weights {
    let w: &[(&str, f64)] = match dim {
        "page" => &[
            ("/", 30.0),
            ("/pricing", 12.0),
            ("/blog/", 9.0),
            ("/blog/log-analytics-without-cookies", 7.0),
            ("/contact", 6.0),
            ("/about", 5.0),
            ("/docs/getting-started", 5.0),
            ("/blog/core-web-vitals-2026", 4.0),
            ("/features", 4.0),
            ("/shop/espresso-cups", 3.0),
            ("/careers", 2.0),
            ("/privacy", 2.0),
            ("/docs/api", 2.0),
            ("/search", 1.0),
        ],
        "channel" => &[
            ("direct", 34.0),
            ("organic", 38.0),
            ("social", 9.0),
            ("referral", 11.0),
            ("paid", 3.0),
            ("email", 3.0),
            ("campaign", 2.0),
        ],
        "referrer_host" => &[
            ("google.com", 50.0),
            ("bing.com", 8.0),
            ("duckduckgo.com", 6.0),
            ("news.ycombinator.com", 6.0),
            ("facebook.com", 5.0),
            ("linkedin.com", 4.0),
            ("reddit.com", 4.0),
            ("github.com", 3.0),
            ("t.co", 3.0),
            ("ecosia.org", 2.0),
        ],
        "referrer_url" => &[
            ("news.ycombinator.com/item", 6.0),
            ("reddit.com/r/webdev/comments", 4.0),
            ("github.com/inalto/webpulse", 3.0),
            ("dev.to/someone/log-analytics", 2.0),
            ("blog.example.org/links", 1.0),
        ],
        "search_engine" => &[
            ("google", 75.0),
            ("bing", 11.0),
            ("duckduckgo", 8.0),
            ("ecosia", 3.0),
            ("yahoo", 2.0),
            ("qwant", 1.0),
        ],
        "keyword" => &[
            ("web analytics without cookies", 3.0),
            ("cpanel analytics plugin", 2.0),
            ("awstats alternative", 2.0),
            ("log analyzer", 1.0),
        ],
        "campaign" => &[
            ("autumn-sale", 5.0),
            ("newsletter-oct", 4.0),
            ("launch", 2.0),
        ],
        "utm_source" => &[("newsletter", 5.0), ("twitter", 3.0), ("partner", 2.0)],
        "utm_medium" => &[("email", 5.0), ("social", 3.0), ("cpc", 2.0)],
        "source_medium" => &[
            ("newsletter / email", 5.0),
            ("twitter / social", 3.0),
            ("partner / cpc", 2.0),
        ],
        "country" => &[
            ("it", 30.0),
            ("us", 18.0),
            ("de", 12.0),
            ("fr", 8.0),
            ("gb", 7.0),
            ("es", 6.0),
            ("ch", 4.0),
            ("nl", 3.0),
            ("br", 3.0),
            ("in", 3.0),
            ("jp", 2.0),
            ("ca", 2.0),
            ("(unknown)", 2.0),
        ],
        "city" => &[],
        "browser" => &[
            ("Chrome", 55.0),
            ("Safari", 22.0),
            ("Firefox", 9.0),
            ("Edge", 8.0),
            ("Samsung Internet", 3.0),
            ("Opera", 2.0),
            ("Other", 1.0),
        ],
        "browser_version" => &[
            ("Chrome 129", 35.0),
            ("Chrome 128", 20.0),
            ("Safari 18", 14.0),
            ("Safari 17", 8.0),
            ("Firefox 131", 9.0),
            ("Edge 129", 8.0),
            ("Samsung Internet 26", 3.0),
            ("Opera 113", 2.0),
        ],
        "os" => &[
            ("Windows", 36.0),
            ("Android", 24.0),
            ("iOS", 20.0),
            ("macOS", 14.0),
            ("Linux", 5.0),
            ("ChromeOS", 1.0),
        ],
        "os_version" => &[
            ("Windows 10/11", 36.0),
            ("Android 14", 16.0),
            ("iOS 18", 12.0),
            ("iOS 17", 8.0),
            ("macOS 14", 14.0),
            ("Android 13", 8.0),
        ],
        "device" => &[("desktop", 54.0), ("mobile", 41.0), ("tablet", 5.0)],
        "status" => &[
            ("200", 80.0),
            ("304", 9.0),
            ("301", 3.0),
            ("404", 5.0),
            ("302", 1.0),
            ("403", 1.0),
            ("500", 0.4),
            ("503", 0.2),
        ],
        "method" => &[("GET", 96.0), ("POST", 4.0)],
        "protocol" => &[("https", 93.0), ("http", 7.0)],
        "http_version" => &[("HTTP/2.0", 70.0), ("HTTP/1.1", 29.0), ("HTTP/1.0", 1.0)],
        "file_type" => &[
            ("html", 25.0),
            ("js", 20.0),
            ("css", 12.0),
            ("webp", 15.0),
            ("png", 10.0),
            ("jpg", 9.0),
            ("woff2", 5.0),
            ("svg", 3.0),
            ("pdf", 1.0),
        ],
        "static_file" => &[
            ("/assets/hero-2400.jpg", 30.0),
            ("/assets/app.js", 22.0),
            ("/assets/app.css", 12.0),
            ("/wp-content/uploads/team.png", 10.0),
            ("/fonts/inter.woff2", 8.0),
            ("/assets/logo.svg", 2.0),
        ],
        "download" => &[
            ("/files/brochure-2026.pdf", 6.0),
            ("/files/price-list.pdf", 4.0),
            ("/downloads/webpulse-0.1.0.tar.gz", 3.0),
        ],
        "not_found" => &[
            ("/wp-login.php", 9.0),
            ("/old-pricing", 6.0),
            ("/.env", 5.0),
            ("/blog/2019/hello-world", 3.0),
            ("/favicon.png", 2.0),
        ],
        "server_error" => &[("/search", 3.0), ("/api/checkout", 1.0)],
        "bot" => &[
            ("Googlebot", 30.0),
            ("bingbot", 12.0),
            ("GPTBot", 10.0),
            ("ClaudeBot", 6.0),
            ("AhrefsBot", 9.0),
            ("SemrushBot", 7.0),
            ("Bytespider", 5.0),
            ("Amazonbot", 4.0),
            ("CCBot", 3.0),
            ("Other bot", 6.0),
        ],
        "visit_duration" => &[
            ("0-10s", 46.0),
            ("11-30s", 9.0),
            ("31-60s", 9.0),
            ("1-2m", 10.0),
            ("2-5m", 12.0),
            ("5-10m", 8.0),
            ("10-30m", 5.0),
            ("30m+", 1.0),
        ],
        "pages_per_visit" => &[
            ("1", 46.0),
            ("2", 18.0),
            ("3", 12.0),
            ("4", 8.0),
            ("5", 5.0),
            ("6-10", 7.0),
            ("11-20", 3.0),
            ("21+", 1.0),
        ],
        _ => &[],
    };
    w.to_vec()
}

fn ws(w: &Weights) -> Vec<f64> {
    w.iter().map(|(_, x)| *x).collect()
}

// ------------------------------------------------------------------ domain database

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

/// Options of `SeedDb::domain()`.
pub struct DomainOpts {
    pub days: i64,
    pub timezone: &'static str,
    pub scale: f64,
    pub seed: i64,
    pub user: String,
    pub now: i64,
    pub schema_version: i64,
}

/// Creates a domain database; returns the totals of the daily table.
pub fn domain(path: &Path, domain: &str, o: &DomainOpts) -> HashMap<&'static str, i64> {
    let tz: chrono_tz::Tz = o.timezone.parse().unwrap();
    let end = tz.timestamp_opt(o.now, 0).unwrap().date_naive();
    let mut mt = Mt::new((o.seed + crc32(domain) as i64) as u32);
    let _ = std::fs::remove_file(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=DELETE").unwrap();
    conn.execute_batch(DOMAIN_DDL).unwrap();
    let tx = conn.transaction().unwrap();
    let diurnal: Vec<f64> = [
        2, 1, 1, 1, 1, 2, 4, 7, 10, 12, 13, 12, 11, 12, 13, 13, 12, 11, 10, 10, 9, 8, 6, 4,
    ]
    .iter()
    .map(|x| *x as f64)
    .collect();
    let mut totals: HashMap<&'static str, i64> = DAILY_COLS.iter().map(|c| (*c, 0)).collect();
    let mut first: Option<String> = None;
    let days = o.days;
    for i in (0..days).rev() {
        let dt = end - Duration::days(i);
        let day = dt.format("%Y-%m-%d").to_string();
        if first.is_none() {
            first = Some(day.clone());
        }
        let weekend = dt.weekday().number_from_monday() >= 6;
        let ratio = webpulse::reports::php::div(days - i, days).f();
        let trend = 1.0 + 0.35 * ratio;
        let weekend_factor = if weekend { 0.62 } else { 1.0 };
        let mut visits = ri((((120 + mt.rand(0, 60)) as f64 * trend) * weekend_factor) * o.scale);
        if i == 0 {
            visits = ri(visits as f64 * 0.55);
        }
        let pageviews =
            ri(visits as f64 * (2.4 + webpulse::reports::php::div(mt.rand(0, 60), 100).f()));
        let bounces =
            ri(visits as f64 * (0.40 + webpulse::reports::php::div(mt.rand(0, 10), 100).f()));
        let duration = visits * (80 + mt.rand(0, 50));
        let status_w = keys("status");
        let hits_split = split(pageviews * 3, &ws(&status_w));
        let hits: i64 = hits_split.iter().sum();
        let bytes = hits * (18000 + mt.rand(0, 9000));
        let st: HashMap<&str, i64> = status_w
            .iter()
            .map(|(k, _)| *k)
            .zip(hits_split.iter().copied())
            .collect();
        let e4 = st["404"] + st["403"];
        let e5 = st["500"] + st["503"];
        let downloads = ri(visits as f64 * 0.04);
        let bot_hits =
            ri(hits as f64 * (0.38 + webpulse::reports::php::div(mt.rand(0, 12), 100).f()));
        let bot_bytes = bot_hits * 9000;
        let row: Vec<(&'static str, i64)> = vec![
            ("visits", visits),
            ("visitors", ri(visits as f64 * 0.83)),
            ("pageviews", pageviews),
            ("hits", hits),
            ("bytes", bytes),
            ("bounces", bounces),
            ("duration", duration),
            ("downloads", downloads),
            ("errors_4xx", e4),
            ("errors_5xx", e5),
            ("bot_hits", bot_hits),
            ("bot_bytes", bot_bytes),
        ];
        tx.execute(
            "INSERT INTO daily VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            rusqlite::params_from_iter(
                std::iter::once(rusqlite::types::Value::from(day.clone()))
                    .chain(row.iter().map(|(_, v)| (*v).into())),
            ),
        )
        .unwrap();
        for (k, v) in &row {
            *totals.get_mut(k).unwrap() += v;
        }
        let hv = split(visits, &diurnal);
        let hp = split(pageviews, &diurnal);
        let hh = split(hits, &diurnal);
        let hb = split(bytes, &diurnal);
        let hbot = split(bot_hits, &[1.0; 24]);
        for h in 0..24 {
            tx.execute(
                "INSERT INTO hourly VALUES (?,?,?,?,?,?,?)",
                params![day, h as i64, hv[h], hp[h], hh[h], hb[h], hbot[h]],
            )
            .unwrap();
        }
        let r: HashMap<&str, i64> = row.iter().copied().collect();
        dims(
            &tx,
            &mut mt,
            &day,
            &r,
            &status_w
                .iter()
                .map(|(k, _)| (*k, st[k]))
                .collect::<Vec<_>>(),
        );
    }
    raw(&tx, &mut mt, end, tz, o.now);
    let meta = [
        ("schema_version", o.schema_version.to_string()),
        ("user", o.user.clone()),
        ("domain", domain.to_string()),
        ("created_at", (o.now - 86400 * days).to_string()),
        ("timezone", o.timezone.to_string()),
        ("first_day", first.unwrap_or_default()),
        ("last_day", end.format("%Y-%m-%d").to_string()),
        ("last_collect_at", (o.now - 140).to_string()),
        ("last_rollup_at", (o.now - 3600 * 5).to_string()),
        (
            "total_lines",
            (totals["hits"] + totals["bot_hits"]).to_string(),
        ),
        ("next_visit_id", "100000".to_string()),
    ];
    for (k, v) in meta {
        tx.execute("INSERT INTO meta VALUES (?, ?)", params![k, v])
            .unwrap();
    }
    tx.commit().unwrap();
    totals
}

fn jitter(mt: &mut Mt, w: &Weights) -> Weights {
    w.iter()
        .map(|(k, x)| {
            (
                *k,
                x * (0.75 + webpulse::reports::php::div(mt.rand(0, 50), 100).f()),
            )
        })
        .collect()
}

fn put(
    tx: &rusqlite::Transaction,
    day: &str,
    dim: &str,
    keys: &[String],
    cols: &HashMap<&str, Vec<i64>>,
) {
    for (i, k) in keys.iter().enumerate() {
        let vals: Vec<i64> = [
            "visits",
            "pageviews",
            "hits",
            "bytes",
            "bounces",
            "duration",
        ]
        .iter()
        .map(|c| cols.get(c).and_then(|v| v.get(i)).copied().unwrap_or(0))
        .collect();
        if vals.iter().sum::<i64>() == 0 {
            continue;
        }
        tx.execute(
            "INSERT INTO dim_daily VALUES (?,?,?,?,?,?,?,?,?)",
            params![
                day, dim, k, vals[0], vals[1], vals[2], vals[3], vals[4], vals[5]
            ],
        )
        .unwrap();
    }
}

fn names(w: &Weights) -> Vec<String> {
    w.iter().map(|(k, _)| k.to_string()).collect()
}

fn dims(
    tx: &rusqlite::Transaction,
    mt: &mut Mt,
    day: &str,
    row: &HashMap<&str, i64>,
    status: &[(&str, i64)],
) {
    let v = row["visits"];
    let w = jitter(mt, &keys("page"));
    let pv = split(row["pageviews"], &ws(&w));
    put(
        tx,
        day,
        "page",
        &names(&w),
        &HashMap::from([
            ("pageviews", pv.clone()),
            ("visits", pv.iter().map(|x| ri(*x as f64 * 0.8)).collect()),
            ("hits", pv.clone()),
            ("bytes", pv.iter().map(|x| x * 42000).collect()),
        ]),
    );
    let entries = split(v, &ws(&w));
    put(
        tx,
        day,
        "entry_page",
        &names(&w),
        &HashMap::from([
            ("visits", entries.clone()),
            ("pageviews", entries.clone()),
            ("bounces", split(row["bounces"], &ws(&w))),
            ("duration", split(row["duration"], &ws(&w))),
        ]),
    );
    let w2 = jitter(mt, &keys("page"));
    put(
        tx,
        day,
        "exit_page",
        &names(&w),
        &HashMap::from([("visits", split(v, &ws(&w2)))]),
    );

    let visit_dims = [
        "channel",
        "country",
        "browser",
        "browser_version",
        "os",
        "os_version",
        "device",
        "search_engine",
        "referrer_host",
        "referrer_url",
        "keyword",
        "campaign",
        "utm_source",
        "utm_medium",
        "source_medium",
        "visit_duration",
        "pages_per_visit",
    ];
    let fraction = |d: &str| -> f64 {
        match d {
            "search_engine" => 0.38,
            "referrer_host" => 0.55,
            "referrer_url" => 0.12,
            "keyword" => 0.03,
            "campaign" | "utm_source" | "utm_medium" | "source_medium" => 0.06,
            _ => 1.0,
        }
    };
    for dim in visit_dims {
        let w = if dim == "visit_duration" || dim == "pages_per_visit" {
            keys(dim)
        } else {
            jitter(mt, &keys(dim))
        };
        let tv = ri(v as f64 * fraction(dim));
        let vs = split(tv, &ws(&w));
        let mut cols = HashMap::from([
            ("visits", vs.clone()),
            (
                "pageviews",
                split(
                    ri(webpulse::reports::php::div(row["pageviews"] * tv, v.max(1)).f()),
                    &ws(&w),
                ),
            ),
        ]);
        if [
            "channel",
            "country",
            "device",
            "browser",
            "os",
            "referrer_host",
            "search_engine",
            "campaign",
        ]
        .contains(&dim)
        {
            let mut b = Vec::new();
            let mut dur = Vec::new();
            for (i, (k, _)) in w.iter().enumerate() {
                let rate = 0.3
                    + webpulse::reports::php::div((crc32(&format!("{dim}{k}")) % 40) as i64, 100)
                        .f();
                b.push(ri(vs[i] as f64 * rate));
                dur.push(vs[i] * (50 + (crc32(k) % 120) as i64));
            }
            cols.insert("bounces", b);
            cols.insert("duration", dur);
        }
        put(tx, day, dim, &names(&w), &cols);
    }

    let st_vals: Vec<i64> = status.iter().map(|(_, x)| *x).collect();
    put(
        tx,
        day,
        "status",
        &status
            .iter()
            .map(|(k, _)| k.to_string())
            .collect::<Vec<_>>(),
        &HashMap::from([
            ("hits", st_vals.clone()),
            ("bytes", st_vals.iter().map(|x| x * 15000).collect()),
        ]),
    );
    for dim in ["method", "protocol", "http_version", "file_type"] {
        let w = jitter(mt, &keys(dim));
        let h = split(row["hits"], &ws(&w));
        put(
            tx,
            day,
            dim,
            &names(&w),
            &HashMap::from([("hits", h), ("bytes", split(row["bytes"], &ws(&w)))]),
        );
    }
    let w = jitter(mt, &keys("static_file"));
    let sh = split(ri(row["hits"] as f64 * 0.5), &ws(&w));
    let sizes = [2400000, 180000, 60000, 900000, 48000, 3000];
    put(
        tx,
        day,
        "static_file",
        &names(&w),
        &HashMap::from([
            ("hits", sh.clone()),
            ("bytes", sh.iter().zip(sizes).map(|(x, s)| x * s).collect()),
        ]),
    );
    let w = jitter(mt, &keys("download"));
    let dh = split(row["downloads"], &ws(&w));
    put(
        tx,
        day,
        "download",
        &names(&w),
        &HashMap::from([
            ("hits", dh.clone()),
            ("bytes", dh.iter().map(|x| x * 2100000).collect()),
        ]),
    );
    let w = jitter(mt, &keys("not_found"));
    let st: HashMap<&str, i64> = status.iter().copied().collect();
    let nf = split(st["404"], &ws(&w));
    put(
        tx,
        day,
        "not_found",
        &names(&w),
        &HashMap::from([
            ("hits", nf.clone()),
            ("bytes", nf.iter().map(|x| x * 800).collect()),
        ]),
    );
    let mut refs: Vec<(String, i64)> = Vec::new();
    for (i, (p, _)) in w.iter().enumerate() {
        if *p == "/old-pricing" || *p == "/blog/2019/hello-world" {
            let hosts: Vec<(&str, f64)> = if *p == "/old-pricing" {
                vec![("bing.com", 3.0), ("oldforum.example.net", 2.0)]
            } else {
                vec![("google.com", 1.0)]
            };
            let parts = split(
                (nf[i] as f64 * 0.7).floor() as i64,
                &hosts.iter().map(|(_, x)| *x).collect::<Vec<_>>(),
            );
            for (j, n) in parts.into_iter().enumerate() {
                refs.push((format!("{p} ← {}", hosts[j].0), n));
            }
        }
    }
    put(
        tx,
        day,
        "not_found_referrer",
        &refs.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        &HashMap::from([("hits", refs.iter().map(|(_, n)| *n).collect())]),
    );
    let w = keys("server_error");
    put(
        tx,
        day,
        "server_error",
        &names(&w),
        &HashMap::from([("hits", split(st["500"] + st["503"], &ws(&w)))]),
    );
    let w = jitter(mt, &keys("bot"));
    put(
        tx,
        day,
        "bot",
        &names(&w),
        &HashMap::from([
            ("hits", split(row["bot_hits"], &ws(&w))),
            ("bytes", split(row["bot_bytes"], &ws(&w))),
        ]),
    );
}

fn hex16(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect::<String>()[..16].to_string()
}

type Profile = (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static str,
);

fn raw(tx: &rusqlite::Transaction, mt: &mut Mt, _end: NaiveDate, tz: chrono_tz::Tz, now: i64) {
    let pages: Vec<&str> = keys("page").iter().map(|(k, _)| *k).collect();
    let profiles: [Profile; 7] = [
        (
            "organic",
            Some("google.com"),
            Some("google"),
            None,
            None,
            "it",
            "Chrome",
            "129",
            "Windows",
            Some("10/11"),
            "desktop",
        ),
        (
            "direct",
            None,
            None,
            None,
            None,
            "us",
            "Safari",
            "18",
            "iOS",
            Some("18"),
            "mobile",
        ),
        (
            "social",
            Some("linkedin.com"),
            None,
            None,
            None,
            "de",
            "Firefox",
            "131",
            "Linux",
            None,
            "desktop",
        ),
        (
            "referral",
            Some("news.ycombinator.com"),
            None,
            None,
            None,
            "gb",
            "Chrome",
            "128",
            "Android",
            Some("14"),
            "mobile",
        ),
        (
            "campaign",
            None,
            None,
            None,
            Some("autumn-sale"),
            "fr",
            "Edge",
            "129",
            "Windows",
            Some("10/11"),
            "desktop",
        ),
        (
            "organic",
            Some("duckduckgo.com"),
            Some("duckduckgo"),
            None,
            None,
            "ch",
            "Safari",
            "17",
            "macOS",
            Some("14"),
            "desktop",
        ),
        (
            "email",
            Some("mail.google.com"),
            None,
            None,
            Some("newsletter-oct"),
            "es",
            "Samsung Internet",
            "26",
            "Android",
            Some("13"),
            "tablet",
        ),
    ];
    let day_of = |t: i64| {
        tz.timestamp_opt(t, 0)
            .unwrap()
            .format("%Y-%m-%d")
            .to_string()
    };
    let mut id: i64 = 1000;
    let mut t = now - 3 * 86400;
    while t < now - 2400 {
        let p = profiles[mt.rand(0, 6) as usize];
        let n = if mt.rand(0, 9) < 4 { 1 } else { mt.rand(2, 7) };
        let mut ts = t;
        let mut acts: Vec<(i64, &str, &str, i64, i64)> = Vec::new();
        let mut bytes = 0;
        for k in 0..n {
            let path = if k == 0 {
                pages[mt.rand(0, 4) as usize]
            } else {
                pages[mt.rand(0, pages.len() as i64 - 1) as usize]
            };
            let status = if mt.rand(0, 30) == 0 { 404 } else { 200 };
            let b = mt.rand(8000, 60000);
            acts.push((
                ts,
                if status == 404 { "error" } else { "pageview" },
                path,
                status,
                b,
            ));
            bytes += b;
            ts += mt.rand(15, 200);
        }
        if mt.rand(0, 6) == 0 {
            acts.push((ts, "download", "/files/brochure-2026.pdf", 200, 2100000));
        }
        let last = acts.last().unwrap().0;
        let pv = acts.iter().filter(|a| a.1 == "pageview").count() as i64;
        tx.execute(
            "INSERT INTO visits VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                id,
                hex16(&id.to_string()),
                day_of(t),
                t,
                last,
                if n == 1 { 0 } else { last - t },
                pv,
                acts.len() as i64,
                bytes,
                acts[0].2,
                acts.last().unwrap().2,
                p.0,
                p.1,
                p.2,
                p.3,
                p.4,
                p.5,
                Option::<String>::None,
                p.6,
                p.7,
                p.8,
                p.9,
                p.10,
                Option::<String>::None
            ],
        )
        .unwrap();
        for (i, a) in acts.iter().enumerate() {
            tx.execute(
                "INSERT INTO events (ts, visit_id, type, path, status, bytes, referrer) VALUES (?,?,?,?,?,?,?)",
                params![a.0, id, a.1, a.2, a.3, a.4, if i == 0 { p.1 } else { None }],
            )
            .unwrap();
        }
        id += 1;
        t += 600 + mt.rand(0, 900);
    }
    let mut t = now - 1700;
    while t < now {
        tx.execute(
            "INSERT INTO events (ts, visit_id, type, path, status, bytes, referrer) VALUES (?,?,?,?,?,?,?)",
            params![t, Option::<i64>::None, "bot", "/robots.txt", 200, 120, Option::<String>::None],
        )
        .unwrap();
        t += 97;
    }
    for k in 0..6usize {
        let p = profiles[k % 7];
        let t0 = now - 1500 + k as i64 * 200;
        let n = 1 + (k % 4) as i64;
        let mut ts = t0;
        let mut last = t0;
        for j in 0..n as usize {
            tx.execute(
                "INSERT INTO events (ts, visit_id, type, path, status, bytes, referrer) VALUES (?,?,?,?,?,?,?)",
                params![ts, id, "pageview", pages[(k + j) % pages.len()], 200, 24000, if j == 0 { p.1 } else { None }],
            )
            .unwrap();
            last = ts;
            ts += 90;
        }
        let attrs = json!({"channel": p.0, "referrer_host": p.1, "search_engine": p.2, "campaign": p.4, "country": p.5,
            "browser": p.6, "browser_version": p.7, "os": p.8, "os_version": p.9, "device": p.10});
        tx.execute(
            "INSERT INTO open_visits VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                hex16(&format!("open{k}")),
                id,
                day_of(t0),
                t0,
                last,
                last,
                n,
                n,
                n * 24000,
                pages[k % pages.len()],
                pages[(k + n as usize - 1) % pages.len()],
                "[]",
                serde_json::to_string(&attrs).unwrap()
            ],
        )
        .unwrap();
        id += 1;
    }
}

// ------------------------------------------------------------------ registry and fake server

struct Account {
    user: &'static str,
    owner: &'static str,
    enabled: Option<i64>,
    domains: Vec<(&'static str, &'static str)>,
}

fn accounts() -> Vec<Account> {
    vec![
        Account {
            user: "alice",
            owner: "root",
            enabled: None,
            domains: vec![("example.com", "main"), ("shop.example.com", "sub")],
        },
        Account {
            user: "bob",
            owner: "reseller1",
            enabled: None,
            domains: vec![("blog.net", "main")],
        },
        Account {
            user: "carol",
            owner: "reseller1",
            enabled: None,
            domains: vec![("carol.org", "main")],
        },
        Account {
            user: "dave",
            owner: "root",
            enabled: Some(0),
            domains: vec![("dave.io", "main")],
        },
    ]
}

fn registry(path: &Path, now: i64) {
    let _ = std::fs::remove_file(path);
    let c = Connection::open(path).unwrap();
    c.execute_batch(REGISTRY_DDL).unwrap();
    c.execute_batch(&format!(
        "INSERT INTO meta VALUES ('schema_version', '1'), ('installed_at', '{}'), ('version', '0.1.0')",
        now - 86400 * 50
    ))
    .unwrap();
    for a in accounts() {
        c.execute(
            "INSERT INTO accounts VALUES (?,?,?,?,?,?,NULL)",
            params![
                a.user,
                a.owner,
                a.enabled,
                Option::<String>::None,
                now - 86400 * 50,
                now
            ],
        )
        .unwrap();
        for (d, k) in &a.domains {
            c.execute(
                "INSERT INTO domains VALUES (?,?,?,?,?,?,NULL)",
                params![a.user, d, Option::<i64>::None, k, now - 86400 * 50, now],
            )
            .unwrap();
        }
    }
    for k in (1..=12).rev() {
        let t = now - k * 300;
        c.execute(
            "INSERT INTO jobs (type, scope, started_at, finished_at, status, lines, files, domains, message) VALUES (?,?,?,?,?,?,?,?,?)",
            params!["collect", "all", t, t + 2, "ok", 300 + k * 17, 4, 3, Option::<String>::None],
        )
        .unwrap();
    }
    c.execute(
        "INSERT INTO jobs (type, scope, started_at, finished_at, status, lines, files, domains, message) VALUES (?,?,?,?,?,?,?,?,?)",
        params!["rollup", "all", now - 30000, now - 29990, "ok", 0, 0, 3, Option::<String>::None],
    )
    .unwrap();
}

fn summaries(registry: &Path, dbs: &[(&str, &str, PathBuf)], now: i64) {
    let c = Connection::open(registry).unwrap();
    let tz = chrono_tz::Europe::Rome;
    let today = tz.timestamp_opt(now, 0).unwrap().date_naive();
    let t = today.format("%Y-%m-%d").to_string();
    let d7 = (today - Duration::days(6)).format("%Y-%m-%d").to_string();
    let d30 = (today - Duration::days(29)).format("%Y-%m-%d").to_string();
    for (user, dom, file) in dbs {
        let db = Connection::open(file).unwrap();
        let q = |from: &str| -> (i64, i64, i64, i64, i64) {
            db.query_row(
                "SELECT COALESCE(SUM(visits),0), COALESCE(SUM(pageviews),0), COALESCE(SUM(bytes),0), COALESCE(SUM(bot_hits),0), COALESCE(SUM(hits),0) FROM daily WHERE day >= ? AND day <= ?",
                params![from, t],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap()
        };
        let (a, b, cc) = (q(&t), q(&d7), q(&d30));
        let size = std::fs::metadata(file).unwrap().len() as i64;
        c.execute(
            "INSERT OR REPLACE INTO domain_summary VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                user,
                dom,
                size,
                now - 140,
                now - 200,
                cc.4 + cc.3,
                a.0,
                a.1,
                b.0,
                b.1,
                b.2,
                cc.0,
                cc.1,
                cc.2,
                cc.3
            ],
        )
        .unwrap();
    }
}

/// A seeded fake server.
#[derive(Debug, Clone)]
pub struct Env {
    pub root: PathBuf,
    pub config: PathBuf,
    pub now: i64,
    pub totals: HashMap<String, HashMap<&'static str, i64>>,
}

/// Repository root.
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Builds the fake server of `SeedDb::fakeServer()` under `root`.
pub fn fake_server(root: &Path, now: i64) -> Env {
    let _ = std::fs::remove_dir_all(root);
    for d in [
        "etc",
        "var/cpanel/users",
        "var/webpulse/data",
        "var/webpulse/log",
        "var/webpulse/run",
        "var/webpulse/geoip",
        "home",
    ] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let mut ud = String::new();
    for a in accounts() {
        let mut lines = vec![format!("OWNER={}", a.owner), format!("USER={}", a.user)];
        for (n, (d, _)) in a.domains.iter().enumerate() {
            lines.push(if n == 0 {
                format!("DNS={d}")
            } else {
                format!("DNS{n}={d}")
            });
            ud.push_str(&format!("{d}: {}\n", a.user));
        }
        std::fs::write(
            root.join("var/cpanel/users").join(a.user),
            lines.join("\n") + "\n",
        )
        .unwrap();
    }
    std::fs::write(root.join("etc/userdomains"), ud).unwrap();
    let data = root.join("var/webpulse/data");
    let dbs: Vec<(&str, &str, PathBuf)> = vec![
        ("alice", "example.com", data.join("alice/example.com.db")),
        (
            "alice",
            "shop.example.com",
            data.join("alice/shop.example.com.db"),
        ),
        ("bob", "blog.net", data.join("bob/blog.net.db")),
    ];
    let scale = |d: &str| match d {
        "example.com" => 1.0,
        "shop.example.com" => 0.35,
        _ => 0.6,
    };
    let mut totals = HashMap::new();
    for (user, dom, file) in &dbs {
        let o = DomainOpts {
            days: 75,
            timezone: "Europe/Rome",
            scale: scale(dom),
            seed: 42,
            user: user.to_string(),
            now,
            schema_version: 1,
        };
        totals.insert(dom.to_string(), domain(file, dom, &o));
        std::fs::create_dir_all(data.join(user).join("user")).unwrap();
    }
    std::fs::create_dir_all(data.join("carol/user")).unwrap();
    registry(&root.join("var/webpulse/webpulse.db"), now);
    summaries(&root.join("var/webpulse/webpulse.db"), &dbs, now);
    let r = root.display().to_string();
    let config = json!({
        "version": 1, "default_enabled": true, "timezone": "Europe/Rome", "raw_retention_days": 30,
        "max_rows_per_dim": 1000, "session_timeout_min": 30, "store_ip": false, "exclude_ips": [],
        "exclude_paths": ["/.well-known/"], "count_head_requests": false,
        "geoip": {"enabled": false, "country_db": format!("{r}/var/webpulse/geoip/dbip-country-lite.mmdb"), "city_db": null},
        "php_bin": null,
        "paths": {
            "data_dir": format!("{r}/var/webpulse/data"), "registry_db": format!("{r}/var/webpulse/webpulse.db"),
            "log_dir": format!("{r}/var/webpulse/log"), "run_dir": format!("{r}/var/webpulse/run"),
            "domlogs_dirs": [format!("{r}/etc/apache2/logs/domlogs/{{user}}")], "archive_dir": format!("{r}/home/{{user}}/logs"),
            "userdomains_file": format!("{r}/etc/userdomains"), "cpanel_users_dir": format!("{r}/var/cpanel/users"),
            "cpanel_userdata_dir": format!("{r}/var/cpanel/userdata"), "home_root": format!("{r}/home"),
            "cron_file": format!("{r}/etc/cron.d/webpulse"), "hooks_file": format!("{r}/var/cpanel/hooks.yaml")
        }
    });
    std::fs::write(
        root.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();
    Env {
        root: root.to_path_buf(),
        config: root.join("config.json"),
        now,
        totals,
    }
}

fn tmp(name: &str) -> PathBuf {
    repo().join("tests/tmp").join(name)
}

/// The shared seeded server of one test binary (built once).
pub fn shared(name: &str) -> Env {
    static CELL: OnceLock<std::sync::Mutex<HashMap<String, Env>>> = OnceLock::new();
    let m = CELL.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = g.get(name) {
        return e.clone();
    }
    pin_now();
    let e = fake_server(&tmp(name), NOW);
    g.insert(name.to_string(), e.clone());
    e
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), &to).unwrap();
        }
    }
}

/// A fresh copy of the shared server (for tests that mutate it).
pub fn fresh(shared_name: &str, name: &str) -> Env {
    let src = shared(shared_name);
    let dst = tmp(name);
    let _ = std::fs::remove_dir_all(&dst);
    copy_tree(&src.root, &dst);
    let cfg = std::fs::read_to_string(&src.config)
        .unwrap()
        .replace(&src.root.display().to_string(), &dst.display().to_string());
    std::fs::write(dst.join("config.json"), cfg).unwrap();
    Env {
        root: dst.clone(),
        config: dst.join("config.json"),
        now: src.now,
        totals: src.totals,
    }
}

/// Pins `WEBPULSE_NOW` (same value for every test of the binary).
pub fn pin_now() {
    // SAFETY: every test sets the same value; no test reads another variable concurrently.
    unsafe { std::env::set_var("WEBPULSE_NOW", NOW.to_string()) };
}

/// Principal from `user:alice`, `admin`, `reseller:reseller1` (owned users from the fake cPanel files).
pub fn principal(spec: &str, root: &Path) -> Principal {
    let (kind, name) = spec.split_once(':').unwrap_or((spec, ""));
    match kind {
        "admin" => Principal::admin(),
        "reseller" => {
            let mut owned = Vec::new();
            let mut files: Vec<_> = std::fs::read_dir(root.join("var/cpanel/users"))
                .unwrap()
                .filter_map(|e| e.ok())
                .collect();
            files.sort_by_key(|e| e.file_name());
            for f in files {
                if std::fs::read_to_string(f.path())
                    .unwrap_or_default()
                    .contains(&format!("OWNER={name}\n"))
                {
                    owned.push(f.file_name().to_string_lossy().into_owned());
                }
            }
            Principal::reseller(name, Some(owned))
        }
        _ => Principal::user(name),
    }
}

/// Result of an in-process API call.
pub struct ApiResult {
    pub status: u16,
    pub json: Value,
    pub body: String,
    pub response: Response,
}

/// Boots the application on an environment.
pub fn app(env: &Env) -> App {
    pin_now();
    App::boot(Global {
        config: Some(env.config.clone()),
        ..Default::default()
    })
}

/// Calls the API in-process (POST + `X-WebPulse-Request: 1` when a body is given).
pub fn api(env: &Env, who: &str, query: &[(&str, &str)], body: Option<Value>) -> ApiResult {
    let method = if body.is_some() { "POST" } else { "GET" };
    api_with(
        env,
        who,
        query,
        body.map(|b| b.to_string().into_bytes()),
        method,
        None,
    )
}

/// Calls the API in-process with full control of method, body and headers.
pub fn api_with(
    env: &Env,
    who: &str,
    query: &[(&str, &str)],
    body: Option<Vec<u8>>,
    method: &str,
    headers: Option<Vec<(&str, &str)>>,
) -> ApiResult {
    let a = app(env);
    let ctx = Ctx::new(&a);
    let headers = headers.unwrap_or(if method == "POST" {
        vec![("X-WebPulse-Request", "1")]
    } else {
        vec![]
    });
    let req = Request::new(
        method,
        Query::from_pairs(query.iter().copied()),
        body,
        headers
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        "203.0.113.9",
    );
    let res = api::dispatch(&ctx, &req, &principal(who, &env.root));
    let body = res.text();
    ApiResult {
        status: res.status,
        json: serde_json::from_str(&body).unwrap_or(Value::Null),
        body,
        response: res,
    }
}

/// `data` of a successful call (asserts status 200 and `ok`).
pub fn ok(r: &ApiResult, what: &str) -> Value {
    assert_eq!(
        r.status,
        200,
        "{what}: {}",
        r.body.chars().take(300).collect::<String>()
    );
    assert_eq!(r.json["ok"], json!(true), "{what}: ok flag");
    assert!(r.json["meta"]["took_ms"].is_i64(), "{what}: meta.took_ms");
    r.json["data"].clone()
}

/// Asserts an error response.
pub fn err(r: &ApiResult, status: u16, code: &str, what: &str) {
    assert_eq!(
        r.status,
        status,
        "{what}: {}",
        r.body.chars().take(300).collect::<String>()
    );
    assert_eq!(r.json["ok"], json!(false), "{what}: ok=false");
    assert_eq!(r.json["error"]["code"], json!(code), "{what}: code");
    assert!(r.json["error"]["message"].is_string(), "{what}: message");
}

/// Asserts that every key exists in an object.
pub fn keys_present(expected: &[&str], v: &Value, what: &str) {
    for k in expected {
        assert!(v.get(*k).is_some(), "{what}: missing key '{k}' in {v}");
    }
}

/// Path of the built binary.
pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_webpulse"))
}

/// Output of a binary run.
pub struct Run {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Run {
    pub fn out(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// Runs the binary with arguments, environment and stdin.
pub fn run(args: &[&str], envs: &[(&str, String)], stdin: &[u8]) -> Run {
    use std::io::Write;
    let mut c = Command::new(bin());
    c.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for v in [
        "REQUEST_METHOD",
        "QUERY_STRING",
        "CONTENT_LENGTH",
        "CONTENT_TYPE",
        "REMOTE_USER",
        "REMOTE_ADDR",
        "HTTP_X_WEBPULSE_REQUEST",
        "WEBPULSE_CGI_TESTING",
    ] {
        c.env_remove(v);
    }
    c.env("WEBPULSE_NOW", NOW.to_string()).env("TZ", "UTC");
    for (k, v) in envs {
        c.env(k, v);
    }
    let mut child = c.spawn().expect("spawn webpulse");
    {
        let mut si = child.stdin.take().unwrap();
        let _ = si.write_all(stdin);
    }
    let out = child.wait_with_output().unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A parsed CGI response.
pub struct Cgi {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Cgi {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

/// Parses `Status:` + headers + CRLF CRLF + body.
pub fn parse_cgi(out: &[u8]) -> Cgi {
    let pos = out
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("CGI header terminator");
    let head = String::from_utf8_lossy(&out[..pos]).into_owned();
    let mut status = 200;
    let mut headers = Vec::new();
    for line in head.split("\r\n") {
        let (k, v) = line.split_once(": ").unwrap_or((line, ""));
        if k == "Status" {
            status = v.split(' ').next().unwrap_or("0").parse().unwrap();
        } else {
            headers.push((k.to_string(), v.to_string()));
        }
    }
    Cgi {
        status,
        headers,
        body: out[pos + 4..].to_vec(),
    }
}

/// Runs `webpulse cgi …` for an environment with a CGI request.
pub fn cgi(
    env: &Env,
    args: &[&str],
    method: &str,
    query: &str,
    body: &[u8],
    extra: &[(&str, String)],
) -> (Run, Cgi) {
    let mut envs: Vec<(&str, String)> = vec![
        ("WEBPULSE_CONFIG", env.config.display().to_string()),
        ("REQUEST_METHOD", method.to_string()),
        ("QUERY_STRING", query.to_string()),
        ("CONTENT_LENGTH", body.len().to_string()),
        ("REMOTE_ADDR", "203.0.113.9".to_string()),
    ];
    envs.extend(extra.iter().cloned());
    let r = run(args, &envs, body);
    let c = parse_cgi(&r.stdout);
    (r, c)
}
