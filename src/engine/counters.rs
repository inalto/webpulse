//! In-memory accumulator of one domain (`WebPulse\Collect\Counters`): daily, hourly
//! and dim_daily counters plus the raw rows (events, closed visits, visitor_day).
//! Flushed with upserts inside the caller's transaction, so each (day, dim, key) row
//! is written at most once per flush; all-zero rows are skipped.

use foldhash::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use rusqlite::{Connection, params};

use super::classify::path::PathInfo;
use super::store::dims;

/// `daily` value columns, in storage order.
pub const DAILY: [&str; 12] = [
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
/// `hourly` value columns, in storage order.
pub const HOURLY: [&str; 5] = ["visits", "pageviews", "hits", "bytes", "bot_hits"];

/// Index of a `daily` column.
pub mod d {
    #![allow(missing_docs)]
    pub const VISITS: usize = 0;
    pub const VISITORS: usize = 1;
    pub const PAGEVIEWS: usize = 2;
    pub const HITS: usize = 3;
    pub const BYTES: usize = 4;
    pub const BOUNCES: usize = 5;
    pub const DURATION: usize = 6;
    pub const DOWNLOADS: usize = 7;
    pub const ERRORS_4XX: usize = 8;
    pub const ERRORS_5XX: usize = 9;
    pub const BOT_HITS: usize = 10;
    pub const BOT_BYTES: usize = 11;
}

/// Index of an `hourly` column.
pub mod h {
    #![allow(missing_docs)]
    pub const VISITS: usize = 0;
    pub const PAGEVIEWS: usize = 1;
    pub const HITS: usize = 2;
    pub const BYTES: usize = 3;
    pub const BOT_HITS: usize = 4;
}

/// dim_daily column: visits.
pub const V: usize = 0;
/// dim_daily column: pageviews.
pub const PV: usize = 1;
/// dim_daily column: hits.
pub const H: usize = 2;
/// dim_daily column: bytes.
pub const B: usize = 3;
/// dim_daily column: bounces.
pub const BO: usize = 4;
/// dim_daily column: duration.
pub const DU: usize = 5;

/// Indexes of the dimensions in [`dims::ALL`].
pub mod di {
    #![allow(missing_docs)]
    pub const PAGE: usize = 0;
    pub const ENTRY_PAGE: usize = 1;
    pub const EXIT_PAGE: usize = 2;
    pub const REFERRER_HOST: usize = 3;
    pub const REFERRER_URL: usize = 4;
    pub const CHANNEL: usize = 5;
    pub const SEARCH_ENGINE: usize = 6;
    pub const KEYWORD: usize = 7;
    pub const CAMPAIGN: usize = 8;
    pub const UTM_SOURCE: usize = 9;
    pub const UTM_MEDIUM: usize = 10;
    pub const SOURCE_MEDIUM: usize = 11;
    pub const COUNTRY: usize = 12;
    pub const CITY: usize = 13;
    pub const BROWSER: usize = 14;
    pub const BROWSER_VERSION: usize = 15;
    pub const OS: usize = 16;
    pub const OS_VERSION: usize = 17;
    pub const DEVICE: usize = 18;
    pub const STATUS: usize = 19;
    pub const METHOD: usize = 20;
    pub const PROTOCOL: usize = 21;
    pub const HTTP_VERSION: usize = 22;
    pub const FILE_TYPE: usize = 23;
    pub const STATIC_FILE: usize = 24;
    pub const DOWNLOAD: usize = 25;
    pub const NOT_FOUND: usize = 26;
    pub const NOT_FOUND_REFERRER: usize = 27;
    pub const SERVER_ERROR: usize = 28;
    pub const BOT: usize = 29;
    pub const VISIT_DURATION: usize = 30;
    pub const PAGES_PER_VISIT: usize = 31;
}

/// Index of a dimension name in [`dims::ALL`].
pub fn dim_index(name: &str) -> usize {
    dims::ALL.iter().position(|d| *d == name).unwrap_or(0)
}

/// A raw event (pageview, download, error, redirect).
#[derive(Debug, Clone)]
pub struct Event {
    /// Timestamp.
    pub ts: i64,
    /// Visit id (`None` when no visit).
    pub visit_id: Option<i64>,
    /// pageview | download | error | redirect
    pub kind: &'static str,
    /// Path info of the hit (its key is stored).
    pub path: Rc<PathInfo>,
    /// HTTP status.
    pub status: i64,
    /// Bytes.
    pub bytes: i64,
    /// External referrer host.
    pub referrer: Option<Rc<str>>,
}

/// A closed visit (`visits` row).
#[derive(Debug, Clone, Default)]
pub struct VisitRow {
    #[allow(missing_docs)]
    pub visit_id: i64,
    #[allow(missing_docs)]
    pub visitor: String,
    #[allow(missing_docs)]
    pub day: String,
    #[allow(missing_docs)]
    pub start_ts: i64,
    #[allow(missing_docs)]
    pub end_ts: i64,
    #[allow(missing_docs)]
    pub duration: i64,
    #[allow(missing_docs)]
    pub pageviews: i64,
    #[allow(missing_docs)]
    pub hits: i64,
    #[allow(missing_docs)]
    pub bytes: i64,
    #[allow(missing_docs)]
    pub entry_page: Option<String>,
    #[allow(missing_docs)]
    pub exit_page: Option<String>,
    /// channel, referrer_host, search_engine, keyword, campaign, country, city,
    /// browser, browser_version, os, os_version, device, ip (in this order).
    pub attrs: [Option<String>; 13],
}

/// Attribute names of [`VisitRow::attrs`].
pub const VISIT_ATTRS: [&str; 13] = [
    "channel",
    "referrer_host",
    "search_engine",
    "keyword",
    "campaign",
    "country",
    "city",
    "browser",
    "browser_version",
    "os",
    "os_version",
    "device",
    "ip",
];

struct DayCounters {
    day: Arc<str>,
    daily: [i64; 12],
    hourly: [[i64; 5]; 24],
    dims: Vec<HashMap<Box<str>, [i64; 6]>>,
}

impl DayCounters {
    fn new(day: Arc<str>) -> DayCounters {
        DayCounters {
            day,
            daily: [0; 12],
            hourly: [[0; 5]; 24],
            dims: (0..dims::ALL.len()).map(|_| HashMap::default()).collect(),
        }
    }
}

/// Rows per multi-row INSERT.
const BATCH: usize = 100;

fn flush_dims(conn: &Connection, batch: &[(&str, usize, &str, &[i64; 6])]) -> Result<()> {
    let groups = vec!["(?, ?, ?, ?, ?, ?, ?, ?, ?)"; batch.len()].join(", ");
    let mut st = conn.prepare_cached(&format!(
        "INSERT INTO dim_daily (day, dim, key, visits, pageviews, hits, bytes, bounces, duration) VALUES {groups}
                ON CONFLICT(day, dim, key) DO UPDATE SET visits = visits + excluded.visits, pageviews = pageviews + excluded.pageviews, hits = hits + excluded.hits, bytes = bytes + excluded.bytes, bounces = bounces + excluded.bounces, duration = duration + excluded.duration"
    ))?;
    let mut i = 1;
    for (day, di, key, v) in batch {
        st.raw_bind_parameter(i, *day)?;
        st.raw_bind_parameter(i + 1, dims::ALL[*di])?;
        st.raw_bind_parameter(i + 2, *key)?;
        for (j, x) in v.iter().enumerate() {
            st.raw_bind_parameter(i + 3 + j, *x)?;
        }
        i += 9;
    }
    st.raw_execute()?;
    Ok(())
}

/// The accumulator.
#[derive(Default)]
pub struct Counters {
    days: Vec<DayCounters>,
    index: HashMap<Arc<str>, usize>,
    last: Option<usize>,
    /// Buffered events.
    pub events: Vec<Event>,
    /// Buffered closed visits.
    pub visits: Vec<VisitRow>,
    /// Buffered `(day, visitor)` rows.
    pub visitor_day: Vec<(Arc<str>, String)>,
}

impl std::fmt::Debug for Counters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Counters")
            .field("days", &self.days.len())
            .field("events", &self.events.len())
            .finish()
    }
}

impl Counters {
    /// Empty counters.
    pub fn new() -> Counters {
        Counters::default()
    }

    fn slot(&mut self, day: &str) -> &mut DayCounters {
        if let Some(i) = self.last {
            let d = self.days[i].day.as_bytes();
            let b = day.as_bytes();
            // short inline comparison (days are 10 bytes): no memcmp call per counter update
            if d.len() == b.len() && d.iter().zip(b).all(|(x, y)| x == y) {
                return &mut self.days[i];
            }
        }
        let i = match self.index.get(day) {
            Some(&i) => i,
            None => {
                let a: Arc<str> = Arc::from(day);
                self.days.push(DayCounters::new(a.clone()));
                self.index.insert(a, self.days.len() - 1);
                self.days.len() - 1
            }
        };
        self.last = Some(i);
        &mut self.days[i]
    }

    /// Adds `n` to a daily column.
    pub fn day(&mut self, day: &str, field: usize, n: i64) {
        self.slot(day).daily[field] += n;
    }

    /// Adds `n` to an hourly column.
    pub fn hour(&mut self, day: &str, hour: u32, field: usize, n: i64) {
        self.slot(day).hourly[(hour as usize).min(23)][field] += n;
    }

    /// Adds `n` to column `col` (V, PV, H, B, BO, DU) of a dim row.
    pub fn dim(&mut self, day: &str, dim: usize, key: &str, col: usize, n: i64) {
        let m = &mut self.slot(day).dims[dim];
        match m.get_mut(key) {
            Some(r) => r[col] += n,
            None => {
                let mut r = [0i64; 6];
                r[col] += n;
                m.insert(key.into(), r);
            }
        }
    }

    /// Adds one hit (hits+1, bytes) to a dim row.
    pub fn dim_hit(&mut self, day: &str, dim: usize, key: &str, bytes: i64) {
        let m = &mut self.slot(day).dims[dim];
        match m.get_mut(key) {
            Some(r) => {
                r[H] += 1;
                r[B] += bytes;
            }
            None => {
                let mut r = [0i64; 6];
                r[H] = 1;
                r[B] = bytes;
                m.insert(key.into(), r);
            }
        }
    }

    /// Records a first visit of `visitor` on `day`.
    pub fn visitor_day(&mut self, day: &Arc<str>, visitor: &str) {
        self.visitor_day.push((day.clone(), visitor.to_string()));
    }

    /// Number of buffered raw rows (triggers intermediate flushes).
    pub fn raw_rows(&self) -> usize {
        self.events.len() + self.visits.len() + self.visitor_day.len()
    }

    /// Writes everything to `conn` (the caller owns the transaction) and clears the
    /// buffers; returns the number of rows written.
    pub fn flush(&mut self, conn: &Connection) -> Result<usize> {
        let mut rows = 0;
        if !self.days.is_empty() {
            let mut st = conn.prepare_cached(
                "INSERT INTO daily (day, visits, visitors, pageviews, hits, bytes, bounces, duration, downloads, errors_4xx, errors_5xx, bot_hits, bot_bytes) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(day) DO UPDATE SET visits = visits + excluded.visits, visitors = visitors + excluded.visitors, pageviews = pageviews + excluded.pageviews, hits = hits + excluded.hits, bytes = bytes + excluded.bytes, bounces = bounces + excluded.bounces, duration = duration + excluded.duration, downloads = downloads + excluded.downloads, errors_4xx = errors_4xx + excluded.errors_4xx, errors_5xx = errors_5xx + excluded.errors_5xx, bot_hits = bot_hits + excluded.bot_hits, bot_bytes = bot_bytes + excluded.bot_bytes",
            )?;
            for dc in &self.days {
                let f = &dc.daily;
                if f.iter().all(|&v| v == 0) {
                    continue;
                }
                st.execute(params![
                    &*dc.day, f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[8], f[9], f[10],
                    f[11]
                ])?;
                rows += 1;
            }
            let mut st = conn.prepare_cached(
                "INSERT INTO hourly (day, hour, visits, pageviews, hits, bytes, bot_hits) VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(day, hour) DO UPDATE SET visits = visits + excluded.visits, pageviews = pageviews + excluded.pageviews, hits = hits + excluded.hits, bytes = bytes + excluded.bytes, bot_hits = bot_hits + excluded.bot_hits",
            )?;
            for dc in &self.days {
                for (hour, f) in dc.hourly.iter().enumerate() {
                    if f.iter().all(|&v| v == 0) {
                        continue;
                    }
                    st.execute(params![&*dc.day, hour as i64, f[0], f[1], f[2], f[3], f[4]])?;
                    rows += 1;
                }
            }
            let mut batch: Vec<(&str, usize, &str, &[i64; 6])> = Vec::with_capacity(BATCH);
            for dc in &self.days {
                for (di, m) in dc.dims.iter().enumerate() {
                    for (k, v) in m {
                        if v.iter().all(|&x| x == 0) {
                            continue;
                        }
                        batch.push((&dc.day, di, k, v));
                        rows += 1;
                        if batch.len() == BATCH {
                            flush_dims(conn, &batch)?;
                            batch.clear();
                        }
                    }
                }
            }
            if !batch.is_empty() {
                flush_dims(conn, &batch)?;
            }
        }
        for chunk in self.events.chunks(BATCH) {
            let groups = vec!["(?, ?, ?, ?, ?, ?, ?)"; chunk.len()].join(", ");
            let mut st = conn.prepare_cached(&format!("INSERT INTO events (ts, visit_id, type, path, status, bytes, referrer) VALUES {groups}"))?;
            let mut i = 1;
            for e in chunk {
                st.raw_bind_parameter(i, e.ts)?;
                st.raw_bind_parameter(i + 1, e.visit_id)?;
                st.raw_bind_parameter(i + 2, e.kind)?;
                st.raw_bind_parameter(i + 3, e.path.key.as_str())?;
                st.raw_bind_parameter(i + 4, e.status)?;
                st.raw_bind_parameter(i + 5, e.bytes)?;
                st.raw_bind_parameter(i + 6, e.referrer.as_deref())?;
                i += 7;
            }
            st.raw_execute()?;
            rows += chunk.len();
        }
        if !self.visits.is_empty() {
            let mut st = conn.prepare_cached(
                "INSERT OR REPLACE INTO visits (visit_id, visitor, day, start_ts, end_ts, duration, pageviews, hits, bytes, entry_page, exit_page, channel, referrer_host, search_engine, keyword, campaign, country, city, browser, browser_version, os, os_version, device, ip) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for v in &self.visits {
                let a = &v.attrs;
                st.execute(params![
                    v.visit_id,
                    v.visitor,
                    v.day,
                    v.start_ts,
                    v.end_ts,
                    v.duration,
                    v.pageviews,
                    v.hits,
                    v.bytes,
                    v.entry_page,
                    v.exit_page,
                    a[0],
                    a[1],
                    a[2],
                    a[3],
                    a[4],
                    a[5],
                    a[6],
                    a[7],
                    a[8],
                    a[9],
                    a[10],
                    a[11],
                    a[12]
                ])?;
            }
            rows += self.visits.len();
        }
        if !self.visitor_day.is_empty() {
            let mut st = conn
                .prepare_cached("INSERT OR IGNORE INTO visitor_day (day, visitor) VALUES (?, ?)")?;
            for (d, v) in &self.visitor_day {
                st.execute(params![&**d, v])?;
            }
            rows += self.visitor_day.len();
        }
        self.reset();
        Ok(rows)
    }

    /// Clears every buffer.
    pub fn reset(&mut self) {
        self.days.clear();
        self.index.clear();
        self.last = None;
        self.events.clear();
        self.visits.clear();
        self.visitor_day.clear();
    }
}
