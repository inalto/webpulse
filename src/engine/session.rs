//! Turns hits into visits (docs/METRICS.md "Visitor and visit"),
//! `WebPulse\Collect\Sessionizer` and `Visit`.
//!
//! * visitor hash = first 16 hex of sha256(salt(local day) . ip . "\n" . ua);
//! * a visit starts on a pageview or download when the visitor has no open visit or
//!   the gap since its last hit is >= the session timeout; other hits only extend an
//!   open visit;
//! * a visit crossing local midnight continues: during the first `timeout` seconds
//!   of a day the previous day's hash is tried too;
//! * visit-level counters (visits, visitors, entry/exit, bounce +1/−1, running
//!   duration) go to the visit's day; the visit-level dimension rows receive their
//!   pageview, bounce and duration deltas at every flush and at close;
//! * engagement buckets and the `visits` row are written at close.
//!
//! In persistent mode open visits are loaded from / saved to `open_visits`. Memory
//! is bounded: at most `max_open` visits are kept open; beyond it the visits idle
//! the longest are closed early (oldest first, ties in opening order).

use std::collections::HashMap as StdHashMap;

use foldhash::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use indexmap::IndexSet;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, params};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::counters::{self as cnt, BO, Counters, DU, PV, V, VISIT_ATTRS, VisitRow, d, h};
use super::store::dims;
use crate::config::php_int;
use crate::util::{json as pjson, time};

/// Distinct pages remembered per visit.
pub const MAX_PAGES_SEEN: usize = 200;
/// Default cap of open visits held in memory (`max_open_visits`).
pub const DEFAULT_MAX_OPEN: usize = 50000;
/// Rows read per batch when closing stored visits at load time.
const LOAD_BATCH: usize = 2000;

/// The visit-level attribute names, in the order they produce dimension rows.
/// Dimension indexes of [`VDIM_ATTRS`].
const VDIM_IDX: [usize; 15] = [5, 3, 4, 6, 7, 8, 9, 10, 12, 13, 14, 15, 16, 17, 18];

const VDIM_ATTRS: [&str; 15] = [
    "channel",
    "referrer_host",
    "referrer_url",
    "search_engine",
    "keyword",
    "campaign",
    "utm_source",
    "utm_medium",
    "country",
    "city",
    "browser",
    "browser_version",
    "os",
    "os_version",
    "device",
];

/// Mutable state of one open visit.
#[derive(Debug, Clone)]
pub struct Visit {
    /// Visitor hash.
    pub key: String,
    /// Visit id.
    pub id: i64,
    /// Attribution day (local day of the first hit).
    pub day: Arc<str>,
    /// First hit.
    pub start: i64,
    /// Last hit of any class.
    pub last: i64,
    /// Attributes (channel, country, browser…), in insertion order.
    pub attrs: Attrs,
    /// Distinct page keys seen (capped, insertion order).
    pub pages: IndexSet<String>,
    /// First pageview.
    pub first_pv: Option<i64>,
    /// Last pageview.
    pub last_pv: i64,
    /// Pageviews.
    pub pv: i64,
    /// Hits.
    pub hits: i64,
    /// Bytes.
    pub bytes: i64,
    /// Entry page key.
    pub entry: Option<String>,
    /// Exit page key.
    pub exit: Option<String>,
    /// Must be saved at the next flush.
    pub dirty: bool,
    c_pv: i64,
    c_bounce: i64,
    c_duration: i64,
    seq: u64,
}

impl Visit {
    fn new(key: String, id: i64, day: Arc<str>, start: i64, attrs: Attrs) -> Visit {
        Visit {
            key,
            id,
            day,
            start,
            last: start,
            attrs,
            pages: IndexSet::new(),
            first_pv: None,
            last_pv: start,
            pv: 0,
            hits: 0,
            bytes: 0,
            entry: None,
            exit: None,
            dirty: true,
            c_pv: 0,
            c_bounce: 0,
            c_duration: 0,
            seq: 0,
        }
    }

    /// Duration between the first and the last pageview (0 for bounces).
    pub fn duration(&self) -> i64 {
        match (self.pv > 0, self.first_pv) {
            (true, Some(f)) => self.last_pv - f,
            _ => 0,
        }
    }

    /// Visit-level dimension rows derived from the attributes.
    pub fn vdims(&self) -> Vec<(usize, String)> {
        visit_dims(&self.attrs)
    }
}

/// The 16 lowercase hex digits of a visitor hash as `&str`.
fn hex_str(h: &[u8; 16]) -> &str {
    std::str::from_utf8(h).unwrap_or("")
}

/// PHP `strval()` of a decoded JSON scalar.
pub fn php_strval(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => {
            if *b {
                "1".into()
            } else {
                String::new()
            }
        }
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.to_string(),
            None => n.as_u64().map(|u| u.to_string()).unwrap_or_else(|| {
                pjson::php_float(n.as_f64().unwrap_or(0.0))
                    .replace("e+", "E+")
                    .replace("e-", "E-")
            }),
        },
        Value::String(s) => s.clone(),
        _ => "Array".into(),
    }
}

/// A visit attribute value: an interned string (the usual case) or any other JSON
/// value read back from `open_visits.attrs`.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrVal {
    /// String value (shared between visits).
    Str(Rc<str>),
    /// Any other JSON value.
    Json(Value),
}

impl AttrVal {
    fn truthy(&self) -> bool {
        match self {
            AttrVal::Str(s) => !(s.is_empty() || &**s == "0"),
            AttrVal::Json(v) => !v.is_null() && crate::config::php_bool(v),
        }
    }
    fn strval(&self) -> String {
        match self {
            AttrVal::Str(s) => s.to_string(),
            AttrVal::Json(v) => php_strval(v),
        }
    }
    fn to_json(&self) -> Value {
        match self {
            AttrVal::Str(s) => Value::from(&**s),
            AttrVal::Json(v) => v.clone(),
        }
    }
}

/// Visit attributes in insertion order, compact (keys and values interned): tens
/// of thousands of visits may be open at once.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attrs {
    items: Vec<(Rc<str>, AttrVal)>,
}

/// String interner shared by the visits of a run.
#[derive(Debug, Default)]
struct Pool {
    set: HashSet<Rc<str>>,
}

impl Pool {
    fn intern(&mut self, s: &str) -> Rc<str> {
        if let Some(r) = self.set.get(s) {
            return r.clone();
        }
        if self.set.len() > 100_000 {
            self.set.clear();
        }
        let r: Rc<str> = Rc::from(s);
        self.set.insert(r.clone());
        r
    }
}

impl Attrs {
    fn from_map(m: Map<String, Value>, pool: &mut Pool) -> Attrs {
        let items = m
            .into_iter()
            .map(|(k, v)| {
                let val = match v {
                    Value::String(s) => AttrVal::Str(pool.intern(&s)),
                    other => AttrVal::Json(other),
                };
                (pool.intern(&k), val)
            })
            .collect();
        Attrs { items }
    }
    /// Value of an attribute.
    pub fn get(&self, k: &str) -> Option<&AttrVal> {
        self.items.iter().find(|(x, _)| &**x == k).map(|(_, v)| v)
    }
    /// JSON object (insertion order).
    pub fn to_map(&self) -> Map<String, Value> {
        self.items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_json()))
            .collect()
    }
}

/// Visit-level dimension rows of a visit, from its attributes (`Sessionizer::visitDims`).
pub fn visit_dims(a: &Attrs) -> Vec<(usize, String)> {
    let mut out = Vec::with_capacity(17);
    for_each_vdim(a, |d, k| out.push((d, k.to_string())));
    out
}

/// Calls `f(dimension index, key)` for every visit-level dimension row of a visit.
pub fn for_each_vdim(a: &Attrs, mut f: impl FnMut(usize, &str)) {
    for (i, d) in VDIM_ATTRS.iter().enumerate() {
        if let Some(AttrVal::Str(s)) = a.get(d)
            && !s.is_empty()
        {
            f(VDIM_IDX[i], s);
        }
    }
    let src = a.get("utm_source");
    let med = a.get("utm_medium");
    let truthy = |v: Option<&AttrVal>| v.map(AttrVal::truthy).unwrap_or(false);
    if truthy(src) || truthy(med) {
        let g = |v: Option<&AttrVal>| match v {
            Some(x) if x.truthy() => x.strval(),
            _ => "(not set)".to_string(),
        };
        f(cnt::di::SOURCE_MEDIUM, &format!("{} / {}", g(src), g(med)));
    }
}

/// The sessionizer of one domain run.
#[derive(Debug)]
pub struct Sessionizer {
    open: HashMap<String, Visit>,
    deleted: HashSet<String>,
    seen_day: HashMap<String, HashSet<String>>,
    closed_count: usize,
    started_count: usize,
    evicted_count: usize,
    salts: HashMap<String, Vec<u8>>,
    hash_cache: HashMap<String, HashMap<Vec<u8>, [u8; 16]>>,
    pool: Pool,
    hash_cache_len: usize,
    key_buf: Vec<u8>,
    timeout: i64,
    next_visit_id: i64,
    max_open: usize,
    seq: u64,
}

/// Source of the daily salts (the registry in production).
pub type SaltFn<'a> = dyn FnMut(&str) -> Result<Vec<u8>> + 'a;

impl Sessionizer {
    /// `timeout` in seconds; `max_open` is raised to at least 10. With a connection,
    /// the `visitor_day` table is loaded (unique visitors already counted).
    pub fn new(
        timeout: i64,
        next_visit_id: i64,
        conn: Option<&Connection>,
        max_open: usize,
    ) -> Result<Sessionizer> {
        let mut s = Sessionizer {
            open: HashMap::default(),
            deleted: HashSet::default(),
            seen_day: HashMap::default(),
            closed_count: 0,
            started_count: 0,
            evicted_count: 0,
            salts: HashMap::default(),
            hash_cache: HashMap::default(),
            pool: Pool::default(),
            hash_cache_len: 0,
            key_buf: Vec::new(),
            timeout,
            next_visit_id,
            max_open: max_open.max(10),
            seq: 0,
        };
        if let Some(c) = conn {
            let mut st = c.prepare("SELECT day, visitor FROM visitor_day")?;
            let rows = st.query_map([], |r| {
                Ok((r.get::<_, SqlValue>(0)?, r.get::<_, SqlValue>(1)?))
            })?;
            for row in rows {
                let (dv, vv) = row?;
                let dd = sql_str(&dv);
                let vk = sql_str(&vv);
                s.seen_day.entry(dd).or_default().insert(vk);
            }
        }
        Ok(s)
    }

    /// Visitor hash for a local day.
    pub fn hash(
        &mut self,
        salt: &mut SaltFn<'_>,
        day: &str,
        ip: &[u8],
        ua: &[u8],
    ) -> Result<[u8; 16]> {
        if !self.salts.contains_key(day) {
            let s = salt(day)?;
            self.salts.insert(day.to_string(), s);
        }
        self.key_buf.clear();
        self.key_buf.extend_from_slice(ip);
        self.key_buf.push(b'\n');
        self.key_buf.extend_from_slice(ua);
        if let Some(h) = self
            .hash_cache
            .get(day)
            .and_then(|m| m.get(self.key_buf.as_slice()))
        {
            return Ok(*h);
        }
        let mut hh = Sha256::new();
        hh.update(&self.salts[day]);
        hh.update(&self.key_buf);
        let digest = hh.finalize();
        let mut hex = [0u8; 16];
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for (i, b) in digest[..8].iter().enumerate() {
            hex[2 * i] = HEX[(b >> 4) as usize];
            hex[2 * i + 1] = HEX[(b & 15) as usize];
        }
        if self.hash_cache_len > 100_000 {
            self.hash_cache.clear();
            self.hash_cache_len = 0;
        }
        self.hash_cache_len += 1;
        self.hash_cache
            .entry(day.to_string())
            .or_default()
            .insert(self.key_buf.clone(), hex);
        Ok(hex)
    }

    /// Next visit id to allocate (persisted in `meta.next_visit_id`).
    pub fn next_visit_id(&self) -> i64 {
        self.next_visit_id
    }
    /// Visits open in memory.
    pub fn open_count(&self) -> usize {
        self.open.len()
    }
    /// Visits started during this run.
    pub fn started_count(&self) -> usize {
        self.started_count
    }
    /// Visits closed during this run (including those closed while loading).
    pub fn closed_count(&self) -> usize {
        self.closed_count
    }
    /// Visits closed early because of the cap.
    pub fn evicted_count(&self) -> usize {
        self.evicted_count
    }
    /// The open-visit cap.
    pub fn max_open(&self) -> usize {
        self.max_open
    }

    fn insert_open(&mut self, mut v: Visit) {
        self.seq += 1;
        v.seq = self.seq;
        self.open.insert(v.key.clone(), v);
    }

    /// Loads the open visits of a domain database (persistent mode), before the first
    /// hit. Stored visits stale at `watermark` (and those beyond the cap, oldest
    /// first) are closed in committed batches; then at most `max_open` visits, the
    /// most recent, are loaded.
    pub fn load(
        &mut self,
        c: &mut Counters,
        conn: &Connection,
        watermark: Option<i64>,
    ) -> Result<()> {
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM open_visits", [], |r| r.get(0))?;
        let excess = (total - self.max_open as i64).max(0) as usize;
        let stale_at = match watermark {
            None => i64::MIN,
            Some(w) => w - self.timeout,
        };
        let first: Option<i64> = conn
            .query_row("SELECT MIN(last_ts) FROM open_visits", [], |r| {
                r.get::<_, SqlValue>(0)
            })
            .ok()
            .and_then(|v| sql_int_opt(&v));
        if excess > 0 || first.map(|f| f <= stale_at).unwrap_or(false) {
            self.close_stored(c, conn, excess, stale_at)?;
        }
        let rows = select_rows(
            conn,
            "SELECT * FROM open_visits ORDER BY last_ts DESC LIMIT ?",
            self.max_open as i64,
        )?;
        for r in rows {
            if let Some(v) = from_row(&r, &mut self.pool) {
                if v.id >= self.next_visit_id {
                    self.next_visit_id = v.id + 1;
                }
                self.insert_open(v);
            }
        }
        Ok(())
    }

    fn close_stored(
        &mut self,
        c: &mut Counters,
        conn: &Connection,
        excess: usize,
        stale_at: i64,
    ) -> Result<usize> {
        let mut closed = 0usize;
        loop {
            let rows = select_rows(
                conn,
                "SELECT * FROM open_visits ORDER BY last_ts ASC, visitor ASC LIMIT ?",
                LOAD_BATCH as i64,
            )?;
            let mut keys: Vec<String> = Vec::new();
            let mut done = rows.len() < LOAD_BATCH;
            for r in &rows {
                let last = r.get("last_ts").and_then(sql_int_opt).unwrap_or(0);
                if closed >= excess && last > stale_at {
                    done = true;
                    break;
                }
                if let Some(v) = from_row(r, &mut self.pool) {
                    self.close_visit(c, v);
                }
                keys.push(r.get("visitor").map(sql_str).unwrap_or_default());
                closed += 1;
            }
            if keys.is_empty() {
                break;
            }
            conn.execute_batch("BEGIN IMMEDIATE")?;
            let res = (|| -> Result<()> {
                c.flush(conn)?;
                let mut del = conn.prepare_cached("DELETE FROM open_visits WHERE visitor = ?")?;
                for k in &keys {
                    del.execute(params![k])?;
                    self.deleted.remove(k);
                }
                Ok(())
            })();
            match res {
                Ok(()) => conn.execute_batch("COMMIT")?,
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
            if done {
                break;
            }
        }
        Ok(closed)
    }

    /// Feeds one human hit; returns the visit id it was attributed to (`None`: no
    /// visit). `day`/`hour`/`midnight` are the local values of `ts`; `attrs` computes
    /// the visit attributes lazily (first hit of a new visit).
    #[allow(clippy::too_many_arguments)]
    pub fn hit(
        &mut self,
        c: &mut Counters,
        salt: &mut SaltFn<'_>,
        ts: i64,
        day: &Arc<str>,
        hour: u32,
        midnight: i64,
        ip: &[u8],
        ua: &[u8],
        is_pageview: bool,
        can_start: bool,
        key: &str,
        bytes: i64,
        attrs: &mut dyn FnMut() -> Map<String, Value>,
    ) -> Result<Option<i64>> {
        let mut vkb = self.hash(salt, day, ip, ua)?;
        let mut found = self.open.contains_key(hex_str(&vkb));
        if !found && ts - midnight < self.timeout {
            let prev = self.hash(salt, &time::prev_day(day), ip, ua)?;
            if let Some(pv) = self.open.get(hex_str(&prev))
                && ts - pv.last < self.timeout
            {
                vkb = prev;
                found = true;
            }
        }
        let vk = hex_str(&vkb);
        if found {
            let stale = self
                .open
                .get(vk)
                .map(|v| ts - v.last >= self.timeout)
                .unwrap_or(false);
            if stale {
                if let Some(v) = self.open.remove(vk) {
                    self.close_visit(c, v);
                }
                found = false;
            }
        }
        if !found {
            if !can_start {
                return Ok(None);
            }
            let a = attrs();
            self.start(c, vk.to_string(), ts, day, hour, a);
        }
        let Some(v) = self.open.get_mut(vk) else {
            return Ok(None);
        };
        if ts > v.last {
            v.last = ts;
        }
        v.hits = v.hits.saturating_add(1);
        v.bytes = v.bytes.saturating_add(bytes);
        v.dirty = true;
        if is_pageview {
            Sessionizer::pageview(c, v, ts, day, key);
        }
        Ok(Some(v.id))
    }

    fn start(
        &mut self,
        c: &mut Counters,
        vk: String,
        ts: i64,
        day: &Arc<str>,
        hour: u32,
        attrs: Map<String, Value>,
    ) {
        let attrs = Attrs::from_map(attrs, &mut self.pool);
        if self.open.len() >= self.max_open {
            self.evict_oldest(c);
        }
        let v = Visit::new(vk.clone(), self.next_visit_id, day.clone(), ts, attrs);
        self.next_visit_id += 1;
        self.started_count += 1;
        c.day(day, d::VISITS, 1);
        c.hour(day, hour, h::VISITS, 1);
        if self.new_visitor_day(c, day, &vk) {
            c.day(day, d::VISITORS, 1);
        }
        for_each_vdim(&v.attrs, |dim, k| c.dim(day, dim, k, V, 1));
        self.deleted.remove(&vk);
        self.insert_open(v);
    }

    fn pageview(c: &mut Counters, v: &mut Visit, ts: i64, hit_day: &str, key: &str) {
        let vday = v.day.clone();
        let entry_dim = cnt::di::ENTRY_PAGE;
        let exit_dim = cnt::di::EXIT_PAGE;
        v.pv += 1;
        if v.pv == 1 {
            v.first_pv = Some(ts);
            v.last_pv = ts;
            v.entry = Some(key.to_string());
            v.exit = Some(key.to_string());
            c.dim(&vday, entry_dim, key, V, 1);
            c.dim(&vday, entry_dim, key, BO, 1);
            c.dim(&vday, exit_dim, key, V, 1);
            c.day(&vday, d::BOUNCES, 1);
        } else {
            let entry = v.entry.clone().unwrap_or_default();
            if v.pv == 2 {
                c.day(&vday, d::BOUNCES, -1);
                c.dim(&vday, entry_dim, &entry, BO, -1);
            }
            let first = v.first_pv.unwrap_or(0);
            let old = v.last_pv - first;
            let latest = ts >= v.last_pv;
            if ts < first {
                v.first_pv = Some(ts);
            }
            if latest {
                v.last_pv = ts;
            }
            let delta = (v.last_pv - v.first_pv.unwrap_or(0)) - old;
            if delta != 0 {
                c.day(&vday, d::DURATION, delta);
                c.dim(&vday, entry_dim, &entry, DU, delta);
            }
            if latest && v.exit.as_deref() != Some(key) {
                let old_exit = v.exit.clone().unwrap_or_default();
                c.dim(&vday, exit_dim, &old_exit, V, -1);
                c.dim(&vday, exit_dim, key, V, 1);
                v.exit = Some(key.to_string());
            }
        }
        if !v.pages.contains(key) && v.pages.len() < MAX_PAGES_SEEN {
            v.pages.insert(key.to_string());
            c.dim(hit_day, cnt::di::PAGE, key, V, 1);
        }
    }

    /// Writes the pending pageview / bounce / duration deltas of a visit to its
    /// visit-level dims.
    fn commit_dims(c: &mut Counters, v: &mut Visit) {
        let dpv = v.pv - v.c_pv;
        let bounce = (v.pv == 1) as i64;
        let dbo = bounce - v.c_bounce;
        let dur = v.duration();
        let ddu = dur - v.c_duration;
        if dpv == 0 && dbo == 0 && ddu == 0 {
            return;
        }
        let day = v.day.clone();
        for_each_vdim(&v.attrs, |dim, k| {
            if dpv != 0 {
                c.dim(&day, dim, k, PV, dpv);
            }
            if dbo != 0 {
                c.dim(&day, dim, k, BO, dbo);
            }
            if ddu != 0 {
                c.dim(&day, dim, k, DU, ddu);
            }
        });
        v.c_pv = v.pv;
        v.c_bounce = bounce;
        v.c_duration = dur;
        v.dirty = true;
    }

    /// Commits the visit-level dim deltas of every open visit (before flushing the counters).
    pub fn commit_all(&mut self, c: &mut Counters) {
        for v in self.open.values_mut() {
            if v.dirty {
                Sessionizer::commit_dims(c, v);
            }
        }
    }

    fn new_visitor_day(&mut self, c: &mut Counters, day: &Arc<str>, vk: &str) -> bool {
        let set = self.seen_day.entry(day.to_string()).or_default();
        if set.contains(vk) {
            return false;
        }
        set.insert(vk.to_string());
        c.visitor_day(day, vk);
        true
    }

    fn sorted_keys(&self, mut pred: impl FnMut(&Visit) -> bool) -> Vec<String> {
        let mut v: Vec<(u64, &String)> = self
            .open
            .values()
            .filter(|v| pred(v))
            .map(|v| (v.seq, &v.key))
            .collect();
        v.sort_unstable();
        v.into_iter().map(|(_, k)| k.clone()).collect()
    }

    /// Closes the visits idle the longest (a tenth of the cap, at least one) so
    /// that a new one fits.
    fn evict_oldest(&mut self, c: &mut Counters) {
        let n = 1usize
            .max(self.max_open / 10)
            .max((self.open.len() + 1).saturating_sub(self.max_open));
        let mut order: Vec<(i64, u64, String)> = self
            .open
            .values()
            .map(|v| (v.last, v.seq, v.key.clone()))
            .collect();
        order.sort_unstable();
        for (_, _, k) in order.into_iter().take(n) {
            if let Some(v) = self.open.remove(&k) {
                self.close_visit(c, v);
                self.evicted_count += 1;
            }
        }
    }

    /// Closes one visit (already removed from the open set): engagement buckets +
    /// `visits` row.
    fn close_visit(&mut self, c: &mut Counters, mut v: Visit) {
        Sessionizer::commit_dims(c, &mut v);
        self.open.remove(&v.key);
        self.deleted.insert(v.key.clone());
        self.closed_count += 1;
        let duration = v.duration();
        c.dim(
            &v.day,
            cnt::di::VISIT_DURATION,
            dims::duration_bucket(duration),
            V,
            1,
        );
        if v.pv > 0 {
            c.dim(
                &v.day,
                cnt::di::PAGES_PER_VISIT,
                dims::pages_bucket(v.pv),
                V,
                1,
            );
        }
        let a = &v.attrs;
        let mut attrs: [Option<String>; 13] = Default::default();
        for (i, name) in VISIT_ATTRS.iter().enumerate() {
            attrs[i] = match a.get(name) {
                None | Some(AttrVal::Json(Value::Null)) => None,
                Some(AttrVal::Str(s)) => Some(s.to_string()),
                Some(AttrVal::Json(other)) => Some(php_strval(other)),
            };
        }
        c.visits.push(VisitRow {
            visit_id: v.id,
            visitor: v.key.clone(),
            day: v.day.to_string(),
            start_ts: v.start,
            end_ts: v.last,
            duration,
            pageviews: v.pv,
            hits: v.hits,
            bytes: v.bytes,
            entry_page: v.entry.clone(),
            exit_page: v.exit.clone(),
            attrs,
        });
    }

    /// Closes the visits whose last hit is at least `timeout` before `now`.
    pub fn close_stale(&mut self, c: &mut Counters, now: i64) -> usize {
        let timeout = self.timeout;
        let keys = self.sorted_keys(|v| now - v.last >= timeout);
        let n = keys.len();
        for k in keys {
            if let Some(v) = self.open.remove(&k) {
                self.close_visit(c, v);
            }
        }
        n
    }

    /// Closes every open visit (end of a backfill).
    pub fn close_all(&mut self, c: &mut Counters) -> usize {
        let keys = self.sorted_keys(|_| true);
        let n = keys.len();
        for k in keys {
            if let Some(v) = self.open.remove(&k) {
                self.close_visit(c, v);
            }
        }
        n
    }

    /// Frees the "seen visitor" memos of days before `day`.
    pub fn forget_days_before(&mut self, day: &str) {
        self.seen_day.retain(|d, _| d.as_str() >= day);
        if self.salts.len() > 8 {
            self.salts.clear();
        }
    }

    /// Persists open visits (persistent mode) inside the caller's transaction.
    pub fn flush(&mut self, conn: &Connection) -> Result<()> {
        if !self.deleted.is_empty() {
            let mut del = conn.prepare_cached("DELETE FROM open_visits WHERE visitor = ?")?;
            for k in self.deleted.drain() {
                if !self.open.contains_key(&k) {
                    del.execute(params![k])?;
                }
            }
        }
        let mut dirty: Vec<(u64, String)> = self
            .open
            .values()
            .filter(|v| v.dirty)
            .map(|v| (v.seq, v.key.clone()))
            .collect();
        if dirty.is_empty() {
            return Ok(());
        }
        dirty.sort_unstable();
        let mut ins = conn.prepare_cached(
            "INSERT OR REPLACE INTO open_visits (visitor, visit_id, day, start_ts, last_ts, last_pv_ts, pageviews, hits, bytes,
                entry_page, exit_page, pages_seen, attrs) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for (_, k) in dirty {
            let Some(v) = self.open.get_mut(&k) else {
                continue;
            };
            let mut attrs = v.attrs.to_map();
            let mut internal = Map::new();
            internal.insert(
                "first_pv".into(),
                v.first_pv.map(Value::from).unwrap_or(Value::Null),
            );
            internal.insert("c_pv".into(), Value::from(v.c_pv));
            internal.insert("c_bounce".into(), Value::from(v.c_bounce));
            internal.insert("c_duration".into(), Value::from(v.c_duration));
            attrs.insert("_".into(), Value::Object(internal));
            let pages = Value::Array(v.pages.iter().map(|p| Value::from(p.as_str())).collect());
            ins.execute(params![
                v.key,
                v.id,
                &*v.day,
                v.start,
                v.last,
                v.last_pv,
                v.pv,
                v.hits,
                v.bytes,
                v.entry,
                v.exit,
                pjson::encode(&pages),
                pjson::encode(&Value::Object(attrs))
            ])?;
            v.dirty = false;
        }
        Ok(())
    }
}

fn sql_str(v: &SqlValue) -> String {
    match v {
        SqlValue::Null => String::new(),
        SqlValue::Integer(i) => i.to_string(),
        SqlValue::Real(f) => pjson::php_float(*f),
        SqlValue::Text(s) => s.clone(),
        SqlValue::Blob(b) => String::from_utf8_lossy(b).into_owned(),
    }
}

fn sql_int_opt(v: &SqlValue) -> Option<i64> {
    match v {
        SqlValue::Null => None,
        SqlValue::Integer(i) => Some(*i),
        SqlValue::Real(f) => Some(*f as i64),
        SqlValue::Text(s) => Some(crate::config::php_str_int(s)),
        SqlValue::Blob(_) => Some(0),
    }
}

fn select_rows(
    conn: &Connection,
    sql: &str,
    limit: i64,
) -> Result<Vec<StdHashMap<String, SqlValue>>> {
    let mut st = conn.prepare_cached(sql)?;
    let names: Vec<String> = st.column_names().into_iter().map(str::to_string).collect();
    let rows = st.query_map(params![limit], |r| {
        let mut m = StdHashMap::new();
        for (i, n) in names.iter().enumerate() {
            m.insert(n.clone(), r.get::<_, SqlValue>(i)?);
        }
        Ok(m)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// JSON container entries as PHP arrays see them.
fn json_entries(v: Value) -> Option<Map<String, Value>> {
    match v {
        Value::Object(m) => Some(m),
        Value::Array(a) => Some(
            a.into_iter()
                .enumerate()
                .map(|(i, v)| (i.to_string(), v))
                .collect(),
        ),
        _ => None,
    }
}

/// Rebuilds a visit from its `open_visits` row (`None` when unreadable).
fn from_row(r: &StdHashMap<String, SqlValue>, pool: &mut Pool) -> Option<Visit> {
    let g = |k: &str| r.get(k).cloned().unwrap_or(SqlValue::Null);
    let attrs: Value = serde_json::from_str(&sql_str(&g("attrs"))).ok()?;
    let pages: Value = serde_json::from_str(&sql_str(&g("pages_seen"))).ok()?;
    let mut attrs = json_entries(attrs)?;
    let pages = json_entries(pages)?;
    let internal = match attrs.get("_") {
        Some(Value::Object(m)) => m.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v.clone()))
            .collect(),
        _ => Map::new(),
    };
    attrs.shift_remove("_");
    let attrs = Attrs::from_map(attrs, pool);
    let ival = |k: &str| sql_int_opt(&g(k)).unwrap_or(0);
    let mut v = Visit::new(
        sql_str(&g("visitor")),
        ival("visit_id"),
        Arc::from(sql_str(&g("day")).as_str()),
        ival("start_ts"),
        attrs,
    );
    v.last = ival("last_ts");
    v.pv = ival("pageviews");
    v.hits = ival("hits");
    v.bytes = ival("bytes");
    v.last_pv = ival("last_pv_ts");
    let present = |k: &str| internal.get(k).filter(|x| !x.is_null());
    v.first_pv = if v.pv > 0 {
        present("first_pv").map(php_int)
    } else {
        None
    };
    v.entry = match g("entry_page") {
        SqlValue::Null => None,
        other => Some(sql_str(&other)),
    };
    v.exit = match g("exit_page") {
        SqlValue::Null => None,
        other => Some(sql_str(&other)),
    };
    v.pages = pages.values().map(php_strval).collect();
    v.c_pv = present("c_pv").map(php_int).unwrap_or(v.pv);
    v.c_bounce = present("c_bounce")
        .map(php_int)
        .unwrap_or((v.pv == 1) as i64);
    v.c_duration = present("c_duration")
        .map(php_int)
        .unwrap_or_else(|| v.duration());
    v.dirty = false;
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn visit_dims_like_php() {
        let a = json!({"channel": "campaign", "utm_source": "news", "utm_medium": "", "country": "(unknown)", "city": null});
        let d = visit_dims(&Attrs::from_map(
            a.as_object().unwrap().clone(),
            &mut Pool::default(),
        ));
        let names: Vec<(&str, &str)> = d.iter().map(|(i, k)| (dims::ALL[*i], k.as_str())).collect();
        assert_eq!(
            names,
            vec![
                ("channel", "campaign"),
                ("utm_source", "news"),
                ("country", "(unknown)"),
                ("source_medium", "news / (not set)")
            ]
        );
    }
}
