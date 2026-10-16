//! The registry database (`/var/webpulse/webpulse.db`, root 0600), `WebPulse\Registry\Registry`:
//! account and domain switches, settings overrides, jobs, daily salts and the
//! per-domain summary used by the WHM overview.
//!
//! The connection sits behind a mutex so that `App` (which owns the registry) stays
//! `Sync`; every method locks it for its own duration.

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Result, anyhow};
use indexmap::IndexMap;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};
use serde_json::{Map, Value, json};

use super::store::{DomainStore, migrate, schema};
use crate::config::{Config, php_bool};
use crate::paths::{assert_domain, assert_user};
use crate::util::{fs, json as pjson, strs, time};

/// A job refused by [`Registry::start_job_limited`] (HTTP 409 in the API); the
/// message is user-facing.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct JobRefused(pub String);

/// Looks up the reseller owning an account (`OWNER=` of `/var/cpanel/users/<u>`).
pub type OwnerLookup = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// One `accounts` row with the effective switch (`AccountRow` of 0.1.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    /// Account name.
    pub user: String,
    /// Reseller (`OWNER=`).
    pub owner: Option<String>,
    /// Effective switch (override, else `default_enabled`).
    pub enabled: bool,
    /// Explicit override.
    pub enabled_override: Option<bool>,
    /// Account timezone (`None` = server).
    pub timezone: Option<String>,
    /// First time discovery saw the account.
    pub first_seen: i64,
    /// Last time discovery saw the account.
    pub last_seen: i64,
    /// When the account disappeared from cPanel.
    pub removed_at: Option<i64>,
    /// Number of non-removed domains.
    pub domains: i64,
}

impl AccountRow {
    /// JSON object with the key order of 0.1.0.
    pub fn to_json(&self) -> Value {
        json!({
            "user": self.user, "owner": self.owner, "enabled": self.enabled, "enabled_override": self.enabled_override,
            "timezone": self.timezone, "first_seen": self.first_seen, "last_seen": self.last_seen,
            "removed_at": self.removed_at, "domains": self.domains,
        })
    }
}

/// One `domains` row with the effective switches (`DomainRow` of 0.1.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainRow {
    /// Account name.
    pub user: String,
    /// Domain name.
    pub domain: String,
    /// main | addon | parked | sub | unknown
    pub kind: String,
    /// Effective switch.
    pub enabled: bool,
    /// Domain override.
    pub enabled_override: Option<bool>,
    /// Effective account switch.
    pub account_enabled: bool,
    /// First time discovery saw the domain.
    pub first_seen: i64,
    /// Last time discovery saw the domain.
    pub last_seen: i64,
    /// When the domain disappeared from cPanel.
    pub removed_at: Option<i64>,
}

impl DomainRow {
    /// JSON object with the key order of 0.1.0.
    pub fn to_json(&self) -> Value {
        json!({
            "user": self.user, "domain": self.domain, "kind": self.kind, "enabled": self.enabled,
            "enabled_override": self.enabled_override, "account_enabled": self.account_enabled,
            "first_seen": self.first_seen, "last_seen": self.last_seen, "removed_at": self.removed_at,
        })
    }
}

/// The registry.
pub struct Registry {
    conn: Mutex<Connection>,
    path: PathBuf,
    salt_cache: Mutex<HashMap<String, Vec<u8>>>,
    base_default_enabled: bool,
    default_override: Mutex<Option<bool>>,
    owner_lookup: Option<OwnerLookup>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("path", &self.path)
            .finish()
    }
}

fn tri(v: &SqlValue) -> Option<bool> {
    match v {
        SqlValue::Null => None,
        SqlValue::Integer(i) => Some(*i == 1),
        SqlValue::Real(f) => Some(*f as i64 == 1),
        SqlValue::Text(s) => Some(crate::config::php_str_int(s) == 1),
        SqlValue::Blob(_) => Some(false),
    }
}

fn opt_i64(v: &SqlValue) -> Option<i64> {
    match v {
        SqlValue::Null => None,
        SqlValue::Integer(i) => Some(*i),
        SqlValue::Real(f) => Some(*f as i64),
        SqlValue::Text(s) => Some(crate::config::php_str_int(s)),
        SqlValue::Blob(_) => Some(0),
    }
}

fn opt_str(v: &SqlValue) -> Option<String> {
    match v {
        SqlValue::Null => None,
        SqlValue::Integer(i) => Some(i.to_string()),
        SqlValue::Real(f) => Some(pjson::php_float(*f)),
        SqlValue::Text(s) => Some(s.clone()),
        SqlValue::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
    }
}

/// A SQLite value as PHP's PDO returns it (native int/float/string/null).
pub fn sql_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(t) => Value::from(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::from(String::from_utf8_lossy(b).into_owned()),
    }
}

fn row_map(
    r: &rusqlite::Row<'_>,
    names: &[String],
) -> rusqlite::Result<IndexMap<String, SqlValue>> {
    let mut m = IndexMap::new();
    for (i, n) in names.iter().enumerate() {
        m.insert(n.clone(), r.get::<_, SqlValue>(i)?);
    }
    Ok(m)
}

fn now() -> i64 {
    time::now_ts()
}

impl Registry {
    /// Opens (creating and migrating when needed) the registry at `path`.
    /// `version` is written to `meta.version` when the file is writable.
    pub fn open(
        path: &Path,
        config: &Config,
        version: Option<&str>,
        owner_lookup: Option<OwnerLookup>,
    ) -> Result<Registry> {
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if !dir.is_dir() {
            fs::ensure_dir(dir, 0o711, None, None)?;
        }
        let fresh = !path.is_file();
        if !fresh && !fs::is_readable(path) {
            return Err(anyhow!("registry {} is not readable", path.display()));
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_millis(5000))?;
        conn.set_prepared_statement_cache_capacity(32);
        if fs::is_writable(path) || fresh {
            let _: String = conn.query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0))?;
            conn.execute_batch("PRAGMA synchronous = NORMAL")?;
            migrate::migrate(&conn, &schema::registry_migrations())?;
            if fresh {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                conn.execute(
                    "INSERT OR IGNORE INTO meta (key, value) VALUES (?, ?)",
                    params!["installed_at", now().to_string()],
                )?;
            }
            if let Some(v) = version {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('version', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![v],
                )?;
            }
        }
        let base_default_enabled = config
            .file_value("default_enabled")
            .map(php_bool)
            .unwrap_or(true);
        let reg = Registry {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
            salt_cache: Mutex::new(HashMap::new()),
            base_default_enabled,
            default_override: Mutex::new(None),
            owner_lookup,
        };
        let ov = reg.settings().get("default_enabled").map(php_bool);
        *reg.default_override
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = ov;
        Ok(reg)
    }

    /// Reads the settings overrides of the registry at `path` without creating,
    /// migrating or writing anything (used at boot; fails when unreadable).
    pub fn read_settings(path: &Path) -> Result<BTreeMap<String, Value>> {
        if !path.is_file() {
            return Ok(BTreeMap::new());
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_millis(5000))?;
        Registry::settings_of(&conn)
    }

    fn settings_of(conn: &Connection) -> Result<BTreeMap<String, Value>> {
        let mut out = BTreeMap::new();
        let mut st = conn.prepare("SELECT key, value FROM settings ORDER BY key")?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, SqlValue>(0)?, r.get::<_, SqlValue>(1)?))
        })?;
        for row in rows {
            let (k, v) = row?;
            let (Some(k), Some(v)) = (opt_str(&k), opt_str(&v)) else {
                continue;
            };
            if let Ok(val) = serde_json::from_str::<Value>(&v) {
                out.insert(k, val);
            }
        }
        Ok(out)
    }

    /// The connection, locked for the caller.
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Effective `default_enabled` (registry override, else the file value).
    pub fn default_enabled(&self) -> bool {
        self.default_override
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unwrap_or(self.base_default_enabled)
    }

    /// Effective switch: an account explicitly disabled turns off all of its domains;
    /// otherwise a domain override wins, else the account setting, else the default.
    pub fn effective(account: Option<bool>, domain: Option<bool>, default: bool) -> bool {
        if account == Some(false) {
            return false;
        }
        domain.or(account).unwrap_or(default)
    }

    fn with_tx<T>(conn: &Connection, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        match f(conn) {
            Ok(v) => {
                conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // ------------------------------------------------------------------ accounts

    fn account_row(&self, r: &IndexMap<String, SqlValue>, def: bool) -> AccountRow {
        let ov = tri(&r["enabled"]);
        AccountRow {
            user: opt_str(&r["user"]).unwrap_or_default(),
            owner: opt_str(&r["owner"]),
            enabled: ov.unwrap_or(def),
            enabled_override: ov,
            timezone: opt_str(&r["timezone"]),
            first_seen: opt_i64(&r["first_seen"]).unwrap_or(0),
            last_seen: opt_i64(&r["last_seen"]).unwrap_or(0),
            removed_at: opt_i64(&r["removed_at"]),
            domains: r.get("domains").and_then(opt_i64).unwrap_or(0),
        }
    }

    fn query_maps(
        conn: &Connection,
        sql: &str,
        args: &[SqlValue],
    ) -> Result<Vec<IndexMap<String, SqlValue>>> {
        let mut st = conn.prepare_cached(sql)?;
        let names: Vec<String> = st.column_names().into_iter().map(str::to_string).collect();
        let rows = st.query_map(params_from_iter(args.iter()), |r| row_map(r, &names))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Accounts with the effective switch; `owner` restricts to a reseller's accounts,
    /// `q` filters on a substring of the user name or one of its domains.
    pub fn accounts(&self, owner: Option<&str>, q: Option<&str>) -> Result<Vec<AccountRow>> {
        let mut sql = String::from(
            "SELECT a.*, (SELECT COUNT(*) FROM domains d WHERE d.user = a.user AND d.removed_at IS NULL) AS domains
                FROM accounts a WHERE 1 = 1",
        );
        let mut args: Vec<SqlValue> = Vec::new();
        if let Some(o) = owner {
            sql.push_str(" AND a.owner = ?");
            args.push(SqlValue::Text(o.to_string()));
        }
        if let Some(q) = q.filter(|q| !q.is_empty()) {
            sql.push_str(" AND (a.user LIKE ? ESCAPE '\\' OR EXISTS (SELECT 1 FROM domains d WHERE d.user = a.user AND d.domain LIKE ? ESCAPE '\\'))");
            let mut like = String::from("%");
            for c in q.to_ascii_lowercase().chars() {
                if c == '%' || c == '_' || c == '\\' {
                    like.push('\\');
                }
                like.push(c);
            }
            like.push('%');
            args.push(SqlValue::Text(like.clone()));
            args.push(SqlValue::Text(like));
        }
        sql.push_str(" ORDER BY a.user");
        let def = self.default_enabled();
        let conn = self.conn();
        Ok(Registry::query_maps(&conn, &sql, &args)?
            .iter()
            .map(|r| self.account_row(r, def))
            .collect())
    }

    /// One account, or `None`.
    pub fn account(&self, user: &str) -> Result<Option<AccountRow>> {
        let def = self.default_enabled();
        let conn = self.conn();
        let rows = Registry::query_maps(
            &conn,
            "SELECT a.*, (SELECT COUNT(*) FROM domains d WHERE d.user = a.user AND d.removed_at IS NULL) AS domains FROM accounts a WHERE a.user = ?",
            &[SqlValue::Text(user.to_string())],
        )?;
        Ok(rows.first().map(|r| self.account_row(r, def)))
    }

    /// Sets the account switch (`Some` = override, `None` = follow `default_enabled`)
    /// and, when `timezone` is `Some`, its timezone (`""` = server timezone).
    pub fn set_account(
        &self,
        user: &str,
        enabled: Option<bool>,
        timezone: Option<&str>,
    ) -> Result<()> {
        assert_user(user)?;
        if let Some(tz) = timezone
            && !tz.is_empty()
            && !time::is_valid_tz_identifier(tz)
        {
            return Err(anyhow!("invalid timezone '{tz}'"));
        }
        self.ensure_account(user)?;
        let conn = self.conn();
        conn.execute(
            "UPDATE accounts SET enabled = ? WHERE user = ?",
            params![enabled.map(|b| b as i64), user],
        )?;
        if let Some(tz) = timezone {
            conn.execute(
                "UPDATE accounts SET timezone = ? WHERE user = ?",
                params![if tz.is_empty() { None } else { Some(tz) }, user],
            )?;
        }
        Ok(())
    }

    fn ensure_account(&self, user: &str) -> Result<()> {
        let owner = self.owner_lookup.as_ref().and_then(|f| f(user));
        let n = now();
        self.conn().execute(
            "INSERT OR IGNORE INTO accounts (user, owner, enabled, timezone, first_seen, last_seen) VALUES (?, ?, NULL, NULL, ?, ?)",
            params![user, owner, n, n],
        )?;
        Ok(())
    }

    /// True when collection is enabled for the account as a whole.
    pub fn account_enabled(&self, user: &str) -> Result<bool> {
        let v: Option<SqlValue> = self
            .conn()
            .query_row(
                "SELECT enabled FROM accounts WHERE user = ?",
                params![user],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.as_ref()
            .and_then(tri)
            .unwrap_or_else(|| self.default_enabled()))
    }

    // ------------------------------------------------------------------- domains

    /// Domains (optionally of one user) with the effective switches.
    pub fn domains(&self, user: Option<&str>) -> Result<Vec<DomainRow>> {
        let mut sql = String::from(
            "SELECT d.*, a.enabled AS account_enabled FROM domains d LEFT JOIN accounts a ON a.user = d.user",
        );
        let mut args = Vec::new();
        if let Some(u) = user {
            sql.push_str(" WHERE d.user = ?");
            args.push(SqlValue::Text(u.to_string()));
        }
        sql.push_str(" ORDER BY d.user, d.domain");
        let def = self.default_enabled();
        let conn = self.conn();
        let rows = Registry::query_maps(&conn, &sql, &args)?;
        Ok(rows
            .iter()
            .map(|r| {
                let acc = tri(&r["account_enabled"]);
                let dom = tri(&r["enabled"]);
                DomainRow {
                    user: opt_str(&r["user"]).unwrap_or_default(),
                    domain: opt_str(&r["domain"]).unwrap_or_default(),
                    kind: opt_str(&r["kind"]).unwrap_or_else(|| "unknown".into()),
                    enabled: Registry::effective(acc, dom, def),
                    enabled_override: dom,
                    account_enabled: acc.unwrap_or(def),
                    first_seen: opt_i64(&r["first_seen"]).unwrap_or(0),
                    last_seen: opt_i64(&r["last_seen"]).unwrap_or(0),
                    removed_at: opt_i64(&r["removed_at"]),
                }
            })
            .collect())
    }

    /// Sets the domain switch (`None` = follow the account).
    pub fn set_domain(&self, user: &str, domain: &str, enabled: Option<bool>) -> Result<()> {
        assert_user(user)?;
        assert_domain(domain)?;
        self.ensure_account(user)?;
        let n = now();
        self.conn().execute(
            "INSERT INTO domains (user, domain, enabled, kind, first_seen, last_seen) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(user, domain) DO UPDATE SET enabled = excluded.enabled",
            params![user, domain, enabled.map(|b| b as i64), "unknown", n, n],
        )?;
        Ok(())
    }

    /// Effective collection switch of one domain.
    pub fn is_enabled(&self, user: &str, domain: &str) -> Result<bool> {
        let conn = self.conn();
        let a: Option<SqlValue> = conn
            .query_row(
                "SELECT a.enabled FROM accounts a WHERE a.user = ?",
                params![user],
                |r| r.get(0),
            )
            .optional()?;
        let d: Option<SqlValue> = conn
            .query_row(
                "SELECT enabled FROM domains WHERE user = ? AND domain = ?",
                params![user, domain],
                |r| r.get(0),
            )
            .optional()?;
        drop(conn);
        Ok(Registry::effective(
            a.as_ref().and_then(tri),
            d.as_ref().and_then(tri),
            self.default_enabled(),
        ))
    }

    /// Records what discovery saw for one account (owner, domains with kinds, in
    /// order) and marks domains no longer present as removed.
    pub fn sync_user(
        &self,
        user: &str,
        owner: Option<&str>,
        domains: &[(String, String)],
    ) -> Result<()> {
        let n = now();
        let conn = self.conn();
        Registry::with_tx(&conn, |c| {
            c.execute(
                "INSERT INTO accounts (user, owner, enabled, timezone, first_seen, last_seen) VALUES (?, ?, NULL, NULL, ?, ?)
                ON CONFLICT(user) DO UPDATE SET owner = excluded.owner, last_seen = excluded.last_seen, removed_at = NULL",
                params![user, owner, n, n],
            )?;
            {
                let mut st = c.prepare_cached(
                    "INSERT INTO domains (user, domain, enabled, kind, first_seen, last_seen) VALUES (?, ?, NULL, ?, ?, ?)
                ON CONFLICT(user, domain) DO UPDATE SET kind = excluded.kind, last_seen = excluded.last_seen, removed_at = NULL",
                )?;
                for (d, kind) in domains {
                    st.execute(params![user, d, kind, n, n])?;
                }
            }
            let current: Vec<String> = {
                let mut st = c.prepare_cached(
                    "SELECT domain FROM domains WHERE user = ? AND removed_at IS NULL",
                )?;
                let rows = st.query_map(params![user], |r| r.get::<_, SqlValue>(0))?;
                rows.filter_map(|r| r.ok())
                    .filter_map(|v| opt_str(&v))
                    .collect()
            };
            let mut rm = c.prepare_cached("UPDATE domains SET removed_at = ? WHERE user = ? AND domain = ? AND removed_at IS NULL")?;
            for d in current {
                if !domains.iter().any(|(k, _)| *k == d) {
                    rm.execute(params![n, user, d])?;
                }
            }
            Ok(())
        })
    }

    /// Marks every account not in `present` (and its domains) as removed.
    pub fn mark_removed_except(&self, present: &[String]) -> Result<usize> {
        let n = now();
        let conn = self.conn();
        let users: Vec<String> = {
            let mut st = conn.prepare("SELECT user FROM accounts WHERE removed_at IS NULL")?;
            let rows = st.query_map([], |r| r.get::<_, SqlValue>(0))?;
            rows.filter_map(|r| r.ok())
                .filter_map(|v| opt_str(&v))
                .collect()
        };
        let mut count = 0;
        for u in users {
            if !present.contains(&u) {
                conn.execute(
                    "UPDATE accounts SET removed_at = ? WHERE user = ?",
                    params![n, u],
                )?;
                conn.execute(
                    "UPDATE domains SET removed_at = ? WHERE user = ? AND removed_at IS NULL",
                    params![n, u],
                )?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Deletes the registry rows of an account (after a purge of a removed account).
    pub fn forget_account(&self, user: &str) -> Result<()> {
        let conn = self.conn();
        for sql in [
            "DELETE FROM domain_summary WHERE user = ?",
            "DELETE FROM domains WHERE user = ?",
            "DELETE FROM accounts WHERE user = ?",
        ] {
            conn.execute(sql, params![user])?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ summary

    /// `domain_summary` rows (optionally restricted to some users), integers cast.
    pub fn summaries(&self, users: Option<&[String]>) -> Result<Vec<Map<String, Value>>> {
        let mut sql = String::from("SELECT * FROM domain_summary");
        let mut args = Vec::new();
        if let Some(us) = users {
            if us.is_empty() {
                return Ok(Vec::new());
            }
            sql.push_str(&format!(
                " WHERE user IN ({})",
                vec!["?"; us.len()].join(",")
            ));
            args.extend(us.iter().map(|u| SqlValue::Text(u.clone())));
        }
        sql.push_str(" ORDER BY user, domain");
        let conn = self.conn();
        let rows = Registry::query_maps(&conn, &sql, &args)?;
        Ok(rows
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|(k, v)| {
                        let j = if k == "user" || k == "domain" {
                            opt_str(&v).map(Value::from).unwrap_or(Value::Null)
                        } else {
                            opt_i64(&v).map(Value::from).unwrap_or(Value::Null)
                        };
                        (k, j)
                    })
                    .collect()
            })
            .collect())
    }

    /// Recomputes the `domain_summary` row of a (writable or read-only) store.
    pub fn refresh_summary(
        &self,
        store: &DomainStore,
        timezone: &str,
        now_ts: Option<i64>,
    ) -> Result<()> {
        let now_ts = now_ts.unwrap_or_else(now);
        let mut clock = time::DayClock::new(timezone);
        let today = clock.day(now_ts).to_string();
        let d7 = time::add_days(&today, -6);
        let d30 = time::add_days(&today, -29);
        let c = store.conn();
        let sum = |from: &str| -> Result<[i64; 4]> {
            let mut st = c.prepare_cached(
                "SELECT COALESCE(SUM(visits),0) v, COALESCE(SUM(pageviews),0) p, COALESCE(SUM(bytes),0) b, COALESCE(SUM(bot_hits),0) bh FROM daily WHERE day >= ? AND day <= ?",
            )?;
            Ok(st.query_row(params![from, today], |r| {
                let g = |i: usize| -> rusqlite::Result<i64> {
                    Ok(opt_i64(&r.get::<_, SqlValue>(i)?).unwrap_or(0))
                };
                Ok([g(0)?, g(1)?, g(2)?, g(3)?])
            })?)
        };
        let t = sum(&today)?;
        let w = sum(&d7)?;
        let m = sum(&d30)?;
        let last_line: Option<i64> = c
            .query_row("SELECT MAX(last_ts) FROM checkpoints", [], |r| {
                r.get::<_, SqlValue>(0)
            })
            .ok()
            .as_ref()
            .and_then(opt_i64);
        let meta = store.meta();
        let db_bytes = std::fs::metadata(store.path())
            .map(|m| m.len() as i64)
            .unwrap_or(0);
        let last_collect = meta
            .get("last_collect_at")
            .cloned()
            .flatten()
            .map(|s| crate::config::php_str_int(&s));
        let lines_total = meta
            .get("total_lines")
            .cloned()
            .flatten()
            .map(|s| crate::config::php_str_int(&s))
            .unwrap_or(0);
        self.conn().execute(
            "INSERT INTO domain_summary (user,domain,db_bytes,last_collect_at,last_line_ts,lines_total,visits_today,pageviews_today,visits_7d,pageviews_7d,bytes_7d,visits_30d,pageviews_30d,bytes_30d,bot_hits_30d) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                ON CONFLICT(user, domain) DO UPDATE SET db_bytes = excluded.db_bytes, last_collect_at = excluded.last_collect_at, last_line_ts = excluded.last_line_ts, lines_total = excluded.lines_total, visits_today = excluded.visits_today, pageviews_today = excluded.pageviews_today, visits_7d = excluded.visits_7d, pageviews_7d = excluded.pageviews_7d, bytes_7d = excluded.bytes_7d, visits_30d = excluded.visits_30d, pageviews_30d = excluded.pageviews_30d, bytes_30d = excluded.bytes_30d, bot_hits_30d = excluded.bot_hits_30d",
            params![
                store.user(), store.domain(), db_bytes, last_collect, last_line, lines_total, t[0], t[1], w[0], w[1], w[2], m[0], m[1], m[2], m[3]
            ],
        )?;
        Ok(())
    }

    /// Removes the summary rows of a user (or of one domain).
    pub fn delete_summary(&self, user: &str, domain: Option<&str>) -> Result<()> {
        let conn = self.conn();
        match domain {
            None => conn.execute("DELETE FROM domain_summary WHERE user = ?", params![user])?,
            Some(d) => conn.execute(
                "DELETE FROM domain_summary WHERE user = ? AND domain = ?",
                params![user, d],
            )?,
        };
        Ok(())
    }

    // --------------------------------------------------------------------- jobs

    fn job_json(r: &IndexMap<String, SqlValue>, cast: bool) -> Value {
        let mut m = Map::new();
        for (k, v) in r {
            let j = if cast
                && [
                    "id",
                    "started_at",
                    "finished_at",
                    "lines",
                    "files",
                    "domains",
                ]
                .contains(&k.as_str())
            {
                opt_i64(v).map(Value::from).unwrap_or(Value::Null)
            } else {
                match v {
                    SqlValue::Null => Value::Null,
                    SqlValue::Integer(i) => Value::from(*i),
                    SqlValue::Real(f) => serde_json::Number::from_f64(*f)
                        .map(Value::Number)
                        .unwrap_or(Value::Null),
                    other => opt_str(other).map(Value::from).unwrap_or(Value::Null),
                }
            };
            m.insert(k.clone(), j);
        }
        Value::Object(m)
    }

    /// Last jobs, newest first (`limit` clamped to 1..500).
    pub fn jobs(&self, limit: i64) -> Result<Vec<Value>> {
        let conn = self.conn();
        let rows = Registry::query_maps(
            &conn,
            "SELECT * FROM jobs ORDER BY id DESC LIMIT ?",
            &[SqlValue::Integer(limit.clamp(1, 500))],
        )?;
        Ok(rows.iter().map(|r| Registry::job_json(r, true)).collect())
    }

    /// One job row, or `None`.
    pub fn job(&self, id: i64) -> Result<Option<Value>> {
        let conn = self.conn();
        let rows = Registry::query_maps(
            &conn,
            "SELECT * FROM jobs WHERE id = ?",
            &[SqlValue::Integer(id)],
        )?;
        Ok(rows.first().map(|r| Registry::job_json(r, false)))
    }

    fn start_job_on(
        conn: &Connection,
        kind: &str,
        scope: Option<&str>,
        started_at: Option<i64>,
    ) -> Result<i64> {
        conn.execute(
            "INSERT INTO jobs (type, scope, started_at, status) VALUES (?, ?, ?, 'running')",
            params![kind, scope, started_at.unwrap_or_else(now)],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute("DELETE FROM jobs WHERE id <= (SELECT id FROM jobs ORDER BY id DESC LIMIT 1 OFFSET 500)", [])?;
        Ok(id)
    }

    /// Inserts a job in state `running` and returns its id (keeps the last 500).
    pub fn start_job(
        &self,
        kind: &str,
        scope: Option<&str>,
        started_at: Option<i64>,
    ) -> Result<i64> {
        Registry::start_job_on(&self.conn(), kind, scope, started_at)
    }

    fn running_on(conn: &Connection, since: i64) -> Result<Vec<Value>> {
        let rows = Registry::query_maps(
            conn,
            "SELECT id, type, scope, started_at FROM jobs WHERE status = 'running' AND started_at >= ? ORDER BY id",
            &[SqlValue::Integer(since)],
        )?;
        Ok(rows
            .iter()
            .map(|r| {
                json!({
                    "id": opt_i64(&r["id"]).unwrap_or(0),
                    "type": opt_str(&r["type"]).unwrap_or_default(),
                    "scope": opt_str(&r["scope"]),
                    "started_at": opt_i64(&r["started_at"]).unwrap_or(0),
                })
            })
            .collect())
    }

    /// Jobs in state `running` started at or after `since` (older ones are presumed dead).
    pub fn running_jobs(&self, since: i64) -> Result<Vec<Value>> {
        Registry::running_on(&self.conn(), since)
    }

    /// Atomically starts a job unless the same type and scope is already running or
    /// `max` jobs are running (only jobs started since `since` count). Refusals are
    /// [`JobRefused`] errors with the user-facing message.
    pub fn start_job_limited(
        &self,
        kind: &str,
        scope: &str,
        since: i64,
        max: usize,
    ) -> Result<i64> {
        let conn = self.conn();
        Registry::with_tx(&conn, |c| {
            let running = Registry::running_on(c, since)?;
            for j in &running {
                if j["type"] == kind && j["scope"] == scope {
                    return Err(JobRefused(format!(
                        "A {kind} job for this scope is already running (#{})",
                        j["id"]
                    ))
                    .into());
                }
            }
            if running.len() >= max {
                return Err(JobRefused(
                    "Too many jobs are running; try again when one has finished".into(),
                )
                .into());
            }
            Registry::start_job_on(c, kind, Some(scope), None)
        })
    }

    /// Marks a job running again (the CLI adopting a job created by the UI).
    pub fn touch_job(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE jobs SET status = 'running', started_at = ? WHERE id = ?",
            params![now(), id],
        )?;
        Ok(())
    }

    /// Finishes a job (`status` ok | error | skipped).
    pub fn finish_job(
        &self,
        id: i64,
        status: &str,
        lines: i64,
        files: i64,
        domains: i64,
        message: Option<&str>,
    ) -> Result<()> {
        let msg = message.map(|m| strs::truncate(&strs::clean(m.as_bytes()), 1000, ""));
        self.conn().execute(
            "UPDATE jobs SET finished_at = ?, status = ?, lines = ?, files = ?, domains = ?, message = ? WHERE id = ?",
            params![now(), status, lines, files, domains, msg, id],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------- salts

    /// Random 32-byte salt of a local day (created on first use).
    pub fn salt(&self, day: &str) -> Result<Vec<u8>> {
        if let Some(s) = self
            .salt_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(day)
        {
            return Ok(s.clone());
        }
        let conn = self.conn();
        let get = |c: &Connection| -> Result<Option<Vec<u8>>> {
            let v: Option<SqlValue> = c
                .query_row("SELECT salt FROM salts WHERE day = ?", params![day], |r| {
                    r.get(0)
                })
                .optional()?;
            Ok(v.map(|v| match v {
                SqlValue::Blob(b) => b,
                SqlValue::Text(t) => t.into_bytes(),
                SqlValue::Integer(i) => i.to_string().into_bytes(),
                SqlValue::Real(f) => pjson::php_float(f).into_bytes(),
                SqlValue::Null => Vec::new(),
            }))
        };
        let s = match get(&conn)? {
            Some(s) => s,
            None => {
                let mut rnd = [0u8; 32];
                rand::fill(&mut rnd);
                conn.execute(
                    "INSERT OR IGNORE INTO salts (day, salt) VALUES (?, ?)",
                    params![day, rnd.to_vec()],
                )?;
                get(&conn)?.unwrap_or_else(|| rnd.to_vec())
            }
        };
        drop(conn);
        self.salt_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(day.to_string(), s.clone());
        Ok(s)
    }

    /// Deletes salts of days before `before_day`; returns the number removed.
    pub fn prune_salts(&self, before_day: &str) -> Result<usize> {
        let n = self
            .conn()
            .execute("DELETE FROM salts WHERE day < ?", params![before_day])?;
        self.salt_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        Ok(n)
    }

    // ----------------------------------------------------------------- settings

    /// Settings overrides: dotted key → decoded JSON value (corrupt values ignored).
    pub fn settings(&self) -> BTreeMap<String, Value> {
        Registry::settings_of(&self.conn()).unwrap_or_default()
    }

    /// Stores a setting override (validate with `Config::validate` first).
    pub fn set_setting(&self, key: &str, value: &Value) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, pjson::encode(value)],
        )?;
        if key == "default_enabled" {
            *self
                .default_override
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(php_bool(value));
        }
        Ok(())
    }

    /// Removes a setting override.
    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM settings WHERE key = ?", params![key])?;
        if key == "default_enabled" {
            *self
                .default_override
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
        }
        Ok(())
    }

    /// The registry `meta` table.
    pub fn meta(&self) -> Result<IndexMap<String, Option<String>>> {
        let conn = self.conn();
        let mut st = conn.prepare("SELECT key, value FROM meta")?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, SqlValue>(0)?, r.get::<_, SqlValue>(1)?))
        })?;
        let mut out = IndexMap::new();
        for row in rows {
            let (k, v) = row?;
            out.insert(opt_str(&k).unwrap_or_default(), opt_str(&v));
        }
        Ok(out)
    }
}
