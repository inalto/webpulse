//! Processing state of one domain during one collector run
//! (`WebPulse\Collect\DomainRun`): classifies hits (docs/METRICS.md "Hit
//! classification"), feeds the counters and the sessionizer, and flushes
//! everything — including log checkpoints — in one transaction.

use foldhash::HashMap;
use std::rc::Rc;

use anyhow::Result;
use indexmap::IndexMap;
use rusqlite::params;
use serde_json::{Map, Value};

use super::classify::geoip::GeoIp;
use super::classify::path::{PathAnalyzer, PathInfo};
use super::classify::referrer::{Referrer, Utm};
use super::classify::ua::{UaInfo, UserAgent};
use super::counters::{self as cnt, Counters, Event, PV, d, h};
use super::parser::Hit;
use super::session::{DEFAULT_MAX_OPEN, SaltFn, Sessionizer};
use super::store::DomainStore;
use super::store::dims::NF_REF_SEP;
use super::user_settings::DomainSettings;
use crate::config::{Config, php_bool};
use crate::util::{ip, strs, time::DayClock};

/// Hit classes.
pub mod class {
    #![allow(missing_docs)]
    pub const IGNORED: &str = "ignored";
    pub const BOT: &str = "bot";
    pub const ERROR: &str = "error";
    pub const REDIRECT: &str = "redirect";
    pub const DOWNLOAD: &str = "download";
    pub const STATIC: &str = "static";
    pub const PAGEVIEW: &str = "pageview";
    pub const OTHER: &str = "other";
}

/// The classifiers shared by every domain of a run (each holds its own caches).
#[derive(Debug)]
pub struct Classifiers {
    /// User agents and bots.
    pub ua: UserAgent,
    /// Referrers.
    pub referrer: Referrer,
    /// Request targets.
    pub path: PathAnalyzer,
}

impl Classifiers {
    /// Classifiers with the configured `static_prefixes`.
    pub fn new(config: &Config) -> Classifiers {
        let prefixes = config.get_str_list("static_prefixes");
        Classifiers {
            ua: UserAgent::new(),
            referrer: Referrer::new(),
            path: PathAnalyzer::new(prefixes),
        }
    }
}

/// A `checkpoints` row (absolute values; counters are added).
#[derive(Debug, Clone)]
pub struct CheckpointRow {
    #[allow(missing_docs)]
    pub path: String,
    #[allow(missing_docs)]
    pub inode: i64,
    #[allow(missing_docs)]
    pub offset: i64,
    #[allow(missing_docs)]
    pub size: i64,
    #[allow(missing_docs)]
    pub first_line: String,
    #[allow(missing_docs)]
    pub last_ts: Option<i64>,
    #[allow(missing_docs)]
    pub lines: i64,
    #[allow(missing_docs)]
    pub bad_lines: i64,
    #[allow(missing_docs)]
    pub ignored_lines: i64,
    #[allow(missing_docs)]
    pub updated_at: i64,
}

/// An `imported_archives` row.
#[derive(Debug, Clone)]
pub struct ArchiveRow {
    #[allow(missing_docs)]
    pub path: String,
    #[allow(missing_docs)]
    pub size: i64,
    #[allow(missing_docs)]
    pub mtime: i64,
    #[allow(missing_docs)]
    pub lines: i64,
    #[allow(missing_docs)]
    pub imported_at: i64,
    #[allow(missing_docs)]
    pub last_ts: Option<i64>,
}

/// One domain's run.
pub struct DomainRun<'a> {
    store: &'a DomainStore,
    domain: String,
    /// Counters of the run.
    pub counters: Counters,
    /// Sessions of the run.
    pub sessions: Sessionizer,
    clock: DayClock,
    cls: &'a mut Classifiers,
    geo: &'a GeoIp,
    salt: Box<SaltFn<'a>>,
    exclude_ips: Vec<ip::Cidr>,
    exclude_paths: Vec<String>,
    ip_excluded: HashMap<Vec<u8>, bool>,
    path_excluded: HashMap<String, bool>,
    ref_host_cache: HashMap<Vec<u8>, Option<Rc<str>>>,
    count_head: bool,
    store_ip: bool,
    min_day: Option<String>,
    max_day: Option<String>,
    min_ts: Option<i64>,
    lines: i64,
    /// class → hits (run statistics).
    pub classes: IndexMap<&'static str, i64>,
    /// Lines read since the last flush (parsed or not), added to `meta.total_lines`.
    pub pending_lines: i64,
}

impl std::fmt::Debug for DomainRun<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DomainRun")
            .field("domain", &self.domain)
            .field("lines", &self.lines)
            .finish()
    }
}

fn conf_list(config: &Config, key: &str) -> Vec<Value> {
    match config.get(key) {
        Value::Array(a) => a.clone(),
        Value::Null => Vec::new(),
        Value::Object(m) => m.values().cloned().collect(),
        other => vec![other.clone()],
    }
}

impl<'a> DomainRun<'a> {
    /// Prepares the run of `domain` stored in `store`, with the local `timezone`,
    /// the account's settings of the domain and the effective configuration. In
    /// persistent mode the open visits are loaded (those stale at `watermark` are
    /// closed while loading).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: &'a DomainStore,
        domain: &str,
        timezone: &str,
        ds: &DomainSettings,
        config: &Config,
        cls: &'a mut Classifiers,
        geo: &'a GeoIp,
        salt: Box<SaltFn<'a>>,
        persistent: bool,
        watermark: Option<i64>,
    ) -> Result<DomainRun<'a>> {
        let conn = store.conn();
        let timeout = (config.int_or("session_timeout_min", 30) * 60).max(60);
        let max_open = config.int_or("max_open_visits", DEFAULT_MAX_OPEN as i64);
        let next = store
            .get_meta("next_visit_id")
            .map(|s| crate::config::php_str_int(&s))
            .unwrap_or(1);
        let max_id: i64 = conn
            .query_row(
                "SELECT MAX(id) FROM (SELECT MAX(visit_id) AS id FROM visits UNION ALL SELECT MAX(visit_id) FROM open_visits)",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )?
            .unwrap_or(0);
        let mut counters = Counters::new();
        let mut sessions = Sessionizer::new(
            timeout,
            next.max(max_id + 1),
            Some(conn),
            if max_open > 0 {
                max_open as usize
            } else {
                DEFAULT_MAX_OPEN
            },
        )?;
        if persistent {
            sessions.load(&mut counters, conn, watermark)?;
        }
        let mut ips: Vec<String> = conf_list(config, "exclude_ips")
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        ips.extend(ds.exclude_ips.iter().cloned());
        let mut paths: Vec<String> = conf_list(config, "exclude_paths")
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        paths.extend(ds.exclude_paths.iter().cloned());
        let flag = |k: &str| match config.get(k) {
            Value::Null => false,
            v => php_bool(v),
        };
        Ok(DomainRun {
            store,
            domain: domain.to_string(),
            counters,
            sessions,
            clock: DayClock::new(timezone),
            cls,
            geo,
            salt,
            exclude_ips: ip::compile(&ips),
            exclude_paths: paths,
            ip_excluded: HashMap::default(),
            path_excluded: HashMap::default(),
            ref_host_cache: HashMap::default(),
            count_head: flag("count_head_requests"),
            store_ip: flag("store_ip"),
            min_day: None,
            max_day: None,
            min_ts: None,
            lines: 0,
            classes: IndexMap::new(),
            pending_lines: 0,
        })
    }

    /// The day clock of the domain.
    pub fn clock(&mut self) -> &mut DayClock {
        &mut self.clock
    }

    /// Hits processed so far.
    pub fn lines(&self) -> i64 {
        self.lines
    }

    /// Smallest hit timestamp processed in this run.
    pub fn min_ts(&self) -> Option<i64> {
        self.min_ts
    }

    /// Closes stale visits (`now - last >= timeout`).
    pub fn close_stale(&mut self, now: i64) -> usize {
        self.sessions.close_stale(&mut self.counters, now)
    }

    /// Closes every open visit.
    pub fn close_all(&mut self) -> usize {
        self.sessions.close_all(&mut self.counters)
    }

    /// Classifies and accounts one hit; returns its class.
    pub fn process(&mut self, hit: &Hit<'_>, https: bool) -> Result<&'static str> {
        self.lines += 1;
        let c = self.do_process(hit, https)?;
        *self.classes.entry(c).or_insert(0) += 1;
        Ok(c)
    }

    fn do_process(&mut self, hit: &Hit<'_>, https: bool) -> Result<&'static str> {
        let method = hit.method();
        if !self.count_head && (method == b"HEAD" || method == b"OPTIONS") {
            return Ok(class::IGNORED);
        }
        if !self.exclude_ips.is_empty() {
            let ipb = hit.ip();
            let ex = match self.ip_excluded.get(ipb) {
                Some(&e) => e,
                None => {
                    let e = std::str::from_utf8(ipb)
                        .map(|s| ip::matches(s, &self.exclude_ips))
                        .unwrap_or(false);
                    self.ip_excluded.insert(ipb.to_vec(), e);
                    e
                }
            };
            if self.ip_excluded.len() > 50000 {
                self.ip_excluded.clear();
            }
            if ex {
                return Ok(class::IGNORED);
            }
        }
        let p: Rc<PathInfo> = self.cls.path.analyze(hit.target());
        if !self.exclude_paths.is_empty() {
            let ex = match self.path_excluded.get(&p.path) {
                Some(&e) => e,
                None => {
                    if self.path_excluded.len() > 50000 {
                        self.path_excluded.clear();
                    }
                    let e = self
                        .exclude_paths
                        .iter()
                        .any(|pre| p.path.starts_with(pre.as_str()));
                    self.path_excluded.insert(p.path.clone(), e);
                    e
                }
            };
            if ex {
                return Ok(class::IGNORED);
            }
        }
        let ts = hit.ts();
        let (day, hour, midnight) = self.clock.at(ts);
        if self.min_day.as_deref().map(|m| &*day < m).unwrap_or(true) {
            self.min_day = Some(day.to_string());
        }
        if self.max_day.as_deref().map(|m| &*day > m).unwrap_or(true) {
            self.max_day = Some(day.to_string());
        }
        if self.min_ts.map(|m| ts < m).unwrap_or(true) {
            self.min_ts = Some(ts);
        }
        let bytes = hit.bytes();
        let ua: Rc<UaInfo> = self.cls.ua.parse(hit.ua());
        let c = &mut self.counters;
        if let Some(bot) = ua.bot {
            c.day(&day, d::BOT_HITS, 1);
            c.day(&day, d::BOT_BYTES, bytes);
            c.hour(&day, hour, h::BOT_HITS, 1);
            c.dim_hit(&day, cnt::di::BOT, bot, bytes);
            return Ok(class::BOT);
        }
        let status = hit.status();
        let key = p.key.as_str();
        let cls = if status >= 400 {
            class::ERROR
        } else if matches!(status, 301 | 302 | 303 | 307 | 308) {
            class::REDIRECT
        } else if p.is_download && (status == 200 || status == 206) {
            class::DOWNLOAD
        } else if p.is_static {
            class::STATIC
        } else if (method == b"GET" || method == b"POST") && matches!(status, 200 | 203 | 206 | 304)
        {
            class::PAGEVIEW
        } else {
            class::OTHER
        };
        c.day(&day, d::HITS, 1);
        c.day(&day, d::BYTES, bytes);
        c.hour(&day, hour, h::HITS, 1);
        c.hour(&day, hour, h::BYTES, bytes);
        let mut sbuf = [0u8; 20];
        let status_key: &str = if status == 0 {
            "-"
        } else {
            fmt_int(status, &mut sbuf)
        };
        let method_key = if method.is_empty() {
            "(invalid)"
        } else {
            std::str::from_utf8(method).unwrap_or("(invalid)")
        };
        let protocol_key = p
            .scheme
            .as_deref()
            .unwrap_or(if https { "https" } else { "http" });
        let hv = hit.protocol();
        let hv_key = if hv.is_empty() {
            "(none)"
        } else {
            std::str::from_utf8(hv).unwrap_or("(none)")
        };
        for (di, k) in [
            (DIM_STATUS, status_key),
            (DIM_METHOD, method_key),
            (DIM_PROTOCOL, protocol_key),
            (DIM_HTTP_VERSION, hv_key),
            (DIM_FILE_TYPE, p.file_type.as_str()),
        ] {
            c.dim_hit(&day, di, k, bytes);
        }
        let mut ev_type: Option<&'static str> = None;
        match cls {
            class::ERROR => {
                c.day(
                    &day,
                    if status >= 500 {
                        d::ERRORS_5XX
                    } else {
                        d::ERRORS_4XX
                    },
                    1,
                );
                if status == 404 {
                    c.dim_hit(&day, cnt::di::NOT_FOUND, key, bytes);
                    if let Some(rh) = self.cls.referrer.any_host(hit.referrer()) {
                        let k = strs::key255(format!("{key}{NF_REF_SEP}{rh}").as_bytes());
                        c.dim_hit(&day, cnt::di::NOT_FOUND_REFERRER, &k, bytes);
                    }
                } else if status >= 500 {
                    c.dim_hit(&day, cnt::di::SERVER_ERROR, key, bytes);
                }
                ev_type = Some("error");
            }
            class::REDIRECT => ev_type = Some("redirect"),
            class::DOWNLOAD => {
                c.day(&day, d::DOWNLOADS, 1);
                c.dim_hit(&day, cnt::di::DOWNLOAD, key, bytes);
                ev_type = Some("download");
            }
            class::STATIC => c.dim_hit(&day, cnt::di::STATIC_FILE, key, bytes),
            class::PAGEVIEW => {
                c.day(&day, d::PAGEVIEWS, 1);
                c.hour(&day, hour, h::PAGEVIEWS, 1);
                let pi = cnt::di::PAGE;
                c.dim(&day, pi, key, PV, 1);
                c.dim_hit(&day, pi, key, bytes);
                ev_type = Some("pageview");
            }
            _ => {}
        }
        let is_pv = cls == class::PAGEVIEW;
        let domain = &self.domain;
        let store_ip = self.store_ip;
        let geo = self.geo;
        let referrer = &mut self.cls.referrer;
        let ua_ref = &ua;
        let p_ref = &p;
        let mut attrs = || visit_attrs(referrer, geo, hit, p_ref, ua_ref, https, domain, store_ip);
        let visit_id = self.sessions.hit(
            &mut self.counters,
            &mut *self.salt,
            ts,
            &day,
            hour,
            midnight,
            hit.ip(),
            hit.ua(),
            is_pv,
            is_pv || cls == class::DOWNLOAD,
            key,
            bytes,
            &mut attrs,
        )?;
        if let Some(t) = ev_type {
            let rh = match hit.referrer() {
                None => None,
                Some(r) => match self.ref_host_cache.get(r) {
                    Some(v) => v.clone(),
                    None => {
                        if self.ref_host_cache.len() > 20000 {
                            self.ref_host_cache.clear();
                        }
                        let v: Option<Rc<str>> = self
                            .cls
                            .referrer
                            .external_host(Some(r), &self.domain)
                            .map(|s| Rc::from(s.as_str()));
                        self.ref_host_cache.insert(r.to_vec(), v.clone());
                        v
                    }
                },
            };
            self.counters.events.push(Event {
                ts,
                visit_id,
                kind: t,
                path: p.clone(),
                status,
                bytes,
                referrer: rh,
            });
        }
        Ok(cls)
    }

    /// Writes counters, sessions, meta and the given checkpoint/archive rows in ONE
    /// transaction.
    pub fn flush(
        &mut self,
        checkpoints: &[CheckpointRow],
        archives: &[ArchiveRow],
        meta: Vec<(&str, Option<String>)>,
    ) -> Result<()> {
        let conn = self.store.conn();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let res = (|| -> Result<()> {
            self.sessions.commit_all(&mut self.counters);
            self.counters.flush(conn)?;
            self.sessions.flush(conn)?;
            if !checkpoints.is_empty() {
                let mut st = conn.prepare_cached(
                    "INSERT INTO checkpoints (path, inode, offset, size, first_line, last_ts, lines, bad_lines, ignored_lines, updated_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(path) DO UPDATE SET inode = excluded.inode, offset = excluded.offset, size = excluded.size,
                    first_line = excluded.first_line, last_ts = COALESCE(excluded.last_ts, checkpoints.last_ts),
                    lines = checkpoints.lines + excluded.lines, bad_lines = checkpoints.bad_lines + excluded.bad_lines,
                    ignored_lines = checkpoints.ignored_lines + excluded.ignored_lines, updated_at = excluded.updated_at",
                )?;
                for cp in checkpoints {
                    st.execute(params![
                        cp.path,
                        cp.inode,
                        cp.offset,
                        cp.size,
                        cp.first_line,
                        cp.last_ts,
                        cp.lines,
                        cp.bad_lines,
                        cp.ignored_lines,
                        cp.updated_at
                    ])?;
                }
            }
            if !archives.is_empty() {
                let mut st = conn.prepare_cached(
                    "INSERT INTO imported_archives (path, size, mtime, lines, imported_at, last_ts) VALUES (?, ?, ?, ?, ?, ?)
                    ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, lines = imported_archives.lines + excluded.lines,
                    imported_at = excluded.imported_at, last_ts = MAX(COALESCE(imported_archives.last_ts, 0), COALESCE(excluded.last_ts, 0))",
                )?;
                for a in archives {
                    st.execute(params![
                        a.path,
                        a.size,
                        a.mtime,
                        a.lines,
                        a.imported_at,
                        a.last_ts
                    ])?;
                }
            }
            let m = self.store.meta();
            let mut meta = meta;
            meta.push((
                "next_visit_id",
                Some(self.sessions.next_visit_id().to_string()),
            ));
            if let Some(md) = &self.min_day
                && m.get("first_day")
                    .cloned()
                    .flatten()
                    .map(|f| md.as_str() < f.as_str())
                    .unwrap_or(true)
            {
                meta.push(("first_day", Some(md.clone())));
            }
            if let Some(xd) = &self.max_day
                && m.get("last_day")
                    .cloned()
                    .flatten()
                    .map(|l| xd.as_str() > l.as_str())
                    .unwrap_or(true)
            {
                meta.push(("last_day", Some(xd.clone())));
            }
            let total = m
                .get("total_lines")
                .cloned()
                .flatten()
                .map(|s| crate::config::php_str_int(&s))
                .unwrap_or(0);
            meta.push((
                "total_lines",
                Some((total + self.pending_lines).to_string()),
            ));
            self.pending_lines = 0;
            self.store.set_meta_many(&meta)?;
            Ok(())
        })();
        match res {
            Ok(()) => {
                conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                self.store.reload_meta();
                Err(e)
            }
        }
    }
}

const DIM_STATUS: usize = 19;
const DIM_METHOD: usize = 20;
const DIM_PROTOCOL: usize = 21;
const DIM_HTTP_VERSION: usize = 22;
const DIM_FILE_TYPE: usize = 23;

/// Decimal of a non-negative integer written into `buf`.
fn fmt_int(n: i64, buf: &mut [u8; 20]) -> &str {
    let mut v = n.unsigned_abs();
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 || i == 0 {
            break;
        }
    }
    std::str::from_utf8(&buf[i..]).unwrap_or("0")
}

/// Attributes of a visit, from its first hit (`DomainRun::visitAttrs`).
#[allow(clippy::too_many_arguments)]
fn visit_attrs(
    referrer: &mut Referrer,
    geo: &GeoIp,
    hit: &Hit<'_>,
    p: &PathInfo,
    ua: &UaInfo,
    https: bool,
    domain: &str,
    store_ip: bool,
) -> Map<String, Value> {
    let utm = Utm {
        source: p.utm_source.as_deref(),
        medium: p.utm_medium.as_deref(),
        campaign: p.utm_campaign.as_deref(),
        click_id: p.click_id,
    };
    let r = referrer.classify(hit.referrer(), domain, &utm);
    let ip_str = String::from_utf8_lossy(hit.ip()).into_owned();
    let ip_ok = std::str::from_utf8(hit.ip()).is_ok();
    let city = if ip_ok { geo.city(&ip_str) } else { None };
    let mut country = if ip_ok { geo.country(&ip_str) } else { None };
    let mut city_key = None;
    if let Some(cy) = &city {
        let cc = cy
            .country
            .clone()
            .or_else(|| country.clone())
            .unwrap_or_default()
            .to_ascii_uppercase();
        city_key = Some(strs::key255(format!("{}, {cc}", cy.name).as_bytes()));
        if country.is_none() {
            country = cy.country.clone();
        }
    }
    let mut a = Map::new();
    let mut put = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            a.insert(k.to_string(), Value::from(v));
        }
    };
    put("channel", Some(r.channel.to_string()));
    put("referrer_host", r.referrer_host);
    put("referrer_url", r.referrer_url);
    put("search_engine", r.search_engine.map(str::to_string));
    put("keyword", r.keyword);
    put("campaign", p.utm_campaign.clone());
    put("utm_source", p.utm_source.clone());
    put("utm_medium", p.utm_medium.clone());
    put(
        "country",
        Some(country.unwrap_or_else(|| "(unknown)".into())),
    );
    put("city", city_key);
    put("browser", Some(ua.browser.clone()));
    put("browser_version", Some(ua.browser_version.clone()));
    put("os", Some(ua.os.clone()));
    put("os_version", Some(ua.os_version.clone()));
    put("device", Some(ua.device.clone()));
    put(
        "protocol",
        Some(
            p.scheme
                .clone()
                .unwrap_or_else(|| if https { "https".into() } else { "http".into() }),
        ),
    );
    if store_ip {
        put("ip", Some(strs::clean(hit.ip())));
    }
    a
}
