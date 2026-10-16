//! Read-side handle on one domain database: a read-only SQLite connection,
//! its `meta` table, the effective timezone and a clock (port of
//! `Reports\DomainData` plus the read-only branch of `Store\DomainStore::open`).
//!
//! Until R1's `engine::store::DomainStore::open_read_only` exists, opening is
//! implemented here with the same rules: refuse symlinks, `NotFound` when the
//! file is missing, `SchemaTooNew` above the supported schema version,
//! `SQLITE_OPEN_READONLY` + `PRAGMA query_only` (the cPanel account only has
//! read permission on its databases).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone};
use chrono_tz::Tz;
use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::{Connection, OpenFlags, ToSql};

use super::php;
use crate::error::{Result, WpError};
use crate::paths::{Paths, is_valid_user};

/// Newest domain schema version this binary reads (`Store\Schema::VERSION`).
pub const SCHEMA_VERSION: i64 = 1;

/// A SQL parameter (PDO binds everything as text or int; SQLite's affinity
/// rules make both equivalent for the queries below).
#[derive(Debug, Clone)]
pub enum Arg {
    /// Text parameter.
    S(String),
    /// Integer parameter.
    I(i64),
}

impl From<&str> for Arg {
    fn from(s: &str) -> Arg {
        Arg::S(s.to_string())
    }
}
impl From<String> for Arg {
    fn from(s: String) -> Arg {
        Arg::S(s)
    }
}
impl From<&String> for Arg {
    fn from(s: &String) -> Arg {
        Arg::S(s.clone())
    }
}
impl From<i64> for Arg {
    fn from(i: i64) -> Arg {
        Arg::I(i)
    }
}

impl ToSql for Arg {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        match self {
            Arg::S(s) => s.to_sql(),
            Arg::I(i) => i.to_sql(),
        }
    }
}

/// A fetched SQL value with PHP's casts.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlVal {
    /// NULL
    Null,
    /// INTEGER
    Int(i64),
    /// REAL
    Real(f64),
    /// TEXT (lossy UTF-8)
    Text(String),
    /// BLOB
    Blob(Vec<u8>),
}

impl SqlVal {
    fn from_ref(v: ValueRef<'_>) -> SqlVal {
        match v {
            ValueRef::Null => SqlVal::Null,
            ValueRef::Integer(i) => SqlVal::Int(i),
            ValueRef::Real(f) => SqlVal::Real(f),
            ValueRef::Text(t) => SqlVal::Text(String::from_utf8_lossy(t).into_owned()),
            ValueRef::Blob(b) => SqlVal::Blob(b.to_vec()),
        }
    }

    /// PHP `(int)` cast.
    pub fn int(&self) -> i64 {
        match self {
            SqlVal::Null => 0,
            SqlVal::Int(i) => *i,
            SqlVal::Real(f) => php::float_to_int(*f),
            SqlVal::Text(s) => php::intval(s),
            SqlVal::Blob(b) => php::intval(&String::from_utf8_lossy(b)),
        }
    }

    /// PHP `(string)` cast (`null` → empty string).
    pub fn string(&self) -> String {
        match self {
            SqlVal::Null => String::new(),
            SqlVal::Int(i) => i.to_string(),
            SqlVal::Real(f) => php::float_repr(*f),
            SqlVal::Text(s) => s.clone(),
            SqlVal::Blob(b) => String::from_utf8_lossy(b).into_owned(),
        }
    }

    /// `null` stays `None`, anything else is cast to string.
    pub fn opt_string(&self) -> Option<String> {
        if self.is_null() {
            None
        } else {
            Some(self.string())
        }
    }

    /// True for SQL NULL.
    pub fn is_null(&self) -> bool {
        matches!(self, SqlVal::Null)
    }
}

/// One fetched row (`PDO::FETCH_ASSOC`).
#[derive(Debug, Clone)]
pub struct Row {
    cols: std::sync::Arc<Vec<String>>,
    vals: Vec<SqlVal>,
}

static NULL: SqlVal = SqlVal::Null;

impl Row {
    /// Column value (NULL when absent).
    pub fn get(&self, col: &str) -> &SqlVal {
        self.cols
            .iter()
            .position(|c| c == col)
            .map(|i| &self.vals[i])
            .unwrap_or(&NULL)
    }
    /// True when the column exists in the result.
    pub fn has(&self, col: &str) -> bool {
        self.cols.iter().any(|c| c == col)
    }
    /// `(int) $row[$col]`.
    pub fn i(&self, col: &str) -> i64 {
        self.get(col).int()
    }
    /// `(string) $row[$col]`.
    pub fn s(&self, col: &str) -> String {
        self.get(col).string()
    }
    /// `$row[$col]` as an optional string.
    pub fn os(&self, col: &str) -> Option<String> {
        self.get(col).opt_string()
    }
    /// Column names in result order.
    pub fn columns(&self) -> &[String] {
        &self.cols
    }
    /// Values in result order.
    pub fn values(&self) -> &[SqlVal] {
        &self.vals
    }
}

/// Runs a prepared statement and returns all rows.
pub fn query_rows(conn: &Connection, sql: &str, args: &[Arg]) -> Result<Vec<Row>> {
    let mut st = conn.prepare(sql)?;
    let cols: std::sync::Arc<Vec<String>> =
        std::sync::Arc::new(st.column_names().iter().map(|c| c.to_string()).collect());
    let n = cols.len();
    let mut rows = st.query(rusqlite::params_from_iter(args.iter()))?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let mut vals = Vec::with_capacity(n);
        for i in 0..n {
            vals.push(SqlVal::from_ref(r.get_ref(i)?));
        }
        out.push(Row {
            cols: cols.clone(),
            vals,
        });
    }
    Ok(out)
}

/// An open read-only domain database (the parts of `DomainStore` the read
/// side needs).
pub struct ReadStore {
    /// The read-only connection.
    pub conn: Connection,
    /// `meta` table.
    pub meta: HashMap<String, Option<String>>,
    /// Database file.
    pub path: PathBuf,
}

/// Reads `meta.schema_version` (0 when missing or unreadable).
pub fn schema_version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |r| Ok(SqlVal::from_ref(r.get_ref(0)?).int()),
    )
    .unwrap_or(0)
}

/// Reads the `meta` table (values cast to string, NULL kept).
pub fn read_meta(conn: &Connection) -> Result<HashMap<String, Option<String>>> {
    let mut out = HashMap::new();
    for r in query_rows(conn, "SELECT key, value FROM meta", &[])? {
        out.insert(r.s("key"), r.os("value"));
    }
    Ok(out)
}

/// `Paths::isValidDomain()` of PHP 0.1.0 (stricter than the resolver's
/// rule): dot-separated labels of `[a-z0-9_][a-z0-9_-]{0,62}`, ≤ 253 bytes.
pub fn is_strict_domain(d: &str) -> bool {
    if d.is_empty() || d.len() > 253 {
        return false;
    }
    d.split('.').all(|l| {
        let b = l.as_bytes();
        !b.is_empty()
            && b.len() <= 63
            && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit() || b[0] == b'_')
            && b[1..]
                .iter()
                .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
    })
}

/// `Paths::domainDb()` of PHP: the database path, or an `Invalid` error
/// (`invalid user name: "…"` / `invalid domain name: "…"`) for names the
/// engine would never create.
pub fn domain_db_checked(paths: &Paths, user: &str, domain: &str) -> Result<PathBuf> {
    if !is_valid_user(user) {
        return Err(super::invalid(format!(
            "invalid user name: {}",
            crate::http::json::quote(user)
        )));
    }
    if !is_strict_domain(domain) {
        return Err(super::invalid(format!(
            "invalid domain name: {}",
            crate::http::json::quote(domain)
        )));
    }
    Ok(paths.data_dir().join(user).join(format!("{domain}.db")))
}

/// Opens `<data_dir>/<user>/<domain>.db` read-only.
///
/// Errors: `Invalid` for malformed names, `WpError::NotFound` when missing,
/// `WpError::SchemaTooNew`, other errors for symlinks and I/O failures.
pub fn open_read_only(paths: &Paths, user: &str, domain: &str) -> Result<ReadStore> {
    let file = domain_db_checked(paths, user, domain)?;
    open_file_read_only(&file, domain)
}

/// Opens a domain database file read-only (see [`open_read_only`]).
pub fn open_file_read_only(file: &Path, domain: &str) -> Result<ReadStore> {
    match std::fs::symlink_metadata(file) {
        Ok(m) if m.file_type().is_symlink() => {
            anyhow::bail!("refusing symlinked database {}", file.display())
        }
        Ok(m) if m.is_file() => {}
        _ => return Err(WpError::NotFound(domain.to_string()).into()),
    }
    let conn = Connection::open_with_flags(
        file,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.execute_batch("PRAGMA query_only = ON")?;
    let v = schema_version(&conn);
    if v > SCHEMA_VERSION {
        return Err(WpError::SchemaTooNew {
            found: v,
            supported: SCHEMA_VERSION,
        }
        .into());
    }
    let meta = read_meta(&conn)?;
    Ok(ReadStore {
        conn,
        meta,
        path: file.to_path_buf(),
    })
}

/// Resolves a timezone name like PHP's `new DateTimeZone()` (case-insensitive);
/// `None` when unknown.
pub fn parse_tz(name: &str) -> Option<Tz> {
    if name.is_empty() {
        return None;
    }
    name.parse::<Tz>().ok().or_else(|| {
        chrono_tz::TZ_VARIANTS
            .iter()
            .copied()
            .find(|t| t.name().eq_ignore_ascii_case(name))
    })
}

/// The zone PHP's `date()` uses (`date_default_timezone_get()`): here the
/// server zone (`TZ`, `/etc/localtime`, else UTC).
pub fn php_default_tz() -> Tz {
    crate::util::time::server_tz()
}

/// `date('c', $ts)` in PHP's default zone.
pub fn date_c(ts: i64) -> String {
    iso_in(ts, php_default_tz())
}

/// `date('Y-m-d', $ts)` in PHP's default zone.
pub fn date_ymd(ts: i64) -> String {
    php_default_tz()
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// ISO-8601 with offset (`2026-10-16T10:05:02+02:00`) of a unix timestamp in `tz`.
pub fn iso_in(ts: i64, tz: Tz) -> String {
    tz.timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%Y-%m-%dT%H:%M:%S%:z").to_string())
        .unwrap_or_default()
}

/// Read-side handle on one domain database (`Reports\DomainData`).
pub struct DomainData {
    /// Read-only connection.
    pub conn: Connection,
    /// `meta` table.
    pub meta: HashMap<String, Option<String>>,
    /// Domain name.
    pub domain: String,
    /// Effective timezone (meta.timezone, else the default).
    pub tz: Tz,
    /// `raw_retention_days` of the configuration.
    pub raw_retention_days: i64,
    now: i64,
}

impl DomainData {
    /// Wraps an open store; `default_tz` is used when meta has no valid timezone.
    pub fn new(
        store: ReadStore,
        domain: &str,
        default_tz: &str,
        raw_retention_days: i64,
        now: i64,
    ) -> DomainData {
        let meta_tz = store
            .meta
            .get("timezone")
            .cloned()
            .flatten()
            .filter(|s| !s.is_empty());
        let name = meta_tz.unwrap_or_else(|| default_tz.to_string());
        let tz = parse_tz(&name).unwrap_or_else(php_default_tz);
        DomainData {
            conn: store.conn,
            meta: store.meta,
            domain: domain.to_string(),
            tz,
            raw_retention_days,
            now,
        }
    }

    /// Current unix time (pinned by `WEBPULSE_NOW` in tests).
    pub fn now(&self) -> i64 {
        self.now
    }

    /// Today in the domain's timezone (`YYYY-MM-DD`).
    pub fn today(&self) -> String {
        self.tz
            .timestamp_opt(self.now, 0)
            .single()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    }

    /// A meta value, `None` when missing or empty.
    pub fn meta_string(&self, key: &str) -> Option<String> {
        self.meta
            .get(key)
            .cloned()
            .flatten()
            .filter(|s| !s.is_empty())
    }

    /// First day with data: meta.first_day, else MIN(daily.day).
    pub fn first_day(&self) -> Option<String> {
        self.meta_string("first_day")
            .or_else(|| self.scalar_string("SELECT MIN(day) FROM daily"))
    }

    /// Last day with data: meta.last_day, else MAX(daily.day).
    pub fn last_day(&self) -> Option<String> {
        self.meta_string("last_day")
            .or_else(|| self.scalar_string("SELECT MAX(day) FROM daily"))
    }

    fn scalar_string(&self, sql: &str) -> Option<String> {
        self.conn
            .query_row(sql, [], |r| Ok(SqlVal::from_ref(r.get_ref(0)?)))
            .ok()
            .and_then(|v| match v {
                SqlVal::Text(s) => Some(s),
                _ => None,
            })
    }

    /// ISO-8601 with offset in the domain zone of a unix timestamp.
    pub fn iso_ts(&self, ts: i64) -> String {
        iso_in(ts, self.tz)
    }

    /// ISO-8601 of a meta value (digits = unix timestamp, else a parseable date).
    pub fn iso(&self, v: Option<&str>) -> Option<String> {
        let v = v?;
        if v.is_empty() {
            return None;
        }
        if v.bytes().all(|c| c.is_ascii_digit()) {
            return v.parse::<i64>().ok().map(|ts| self.iso_ts(ts));
        }
        if let Ok(d) = DateTime::parse_from_rfc3339(v) {
            return Some(
                d.with_timezone(&self.tz)
                    .format("%Y-%m-%dT%H:%M:%S%:z")
                    .to_string(),
            );
        }
        if let Ok(n) = chrono::NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S") {
            let local = php_default_tz().from_local_datetime(&n).earliest()?;
            return Some(
                local
                    .with_timezone(&self.tz)
                    .format("%Y-%m-%dT%H:%M:%S%:z")
                    .to_string(),
            );
        }
        None
    }

    /// Runs a prepared statement and returns all rows.
    pub fn rows(&self, sql: &str, args: &[Arg]) -> Result<Vec<Row>> {
        query_rows(&self.conn, sql, args)
    }

    /// First row of a prepared statement, `None` when empty.
    pub fn row(&self, sql: &str, args: &[Arg]) -> Result<Option<Row>> {
        Ok(self.rows(sql, args)?.into_iter().next())
    }

    /// Meta block shared by overview and other reports.
    pub fn meta_block(&self) -> serde_json::Value {
        serde_json::json!({
            "domain": self.domain,
            "timezone": self.tz.name(),
            "first_day": self.first_day(),
            "last_day": self.last_day(),
            "last_collect_at": self.iso(self.meta_string("last_collect_at").as_deref()),
            "tz_changed_at": self.iso(self.meta_string("tz_changed_at").as_deref()),
            "raw_retention_days": self.raw_retention_days,
        })
    }
}
