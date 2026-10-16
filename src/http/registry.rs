//! Access to the registry database (`/var/webpulse/webpulse.db`) for the HTTP
//! layer: the subset of `Registry\Registry` (0.1.0) the API uses, with the
//! same SQL, row shapes and open semantics (created and migrated when
//! writable, read-only otherwise).
//!
//! Local until R1's `engine::registry::Registry` lands; the orchestrator can
//! then replace this module by thin calls into the engine.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value, json};

use super::json as pjson;
use crate::error::{Result, WpError};
use crate::reports::domain_data::{Arg, Row, SqlVal, query_rows, schema_version};

/// Newest registry schema version (`Store\Schema::REGISTRY_VERSION`).
pub const REGISTRY_VERSION: i64 = 1;

const MIGRATION_1: [&str; 7] = [
    "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT)",
    "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS accounts (
        user TEXT PRIMARY KEY, owner TEXT, enabled INTEGER, timezone TEXT,
        first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, removed_at INTEGER)",
    "CREATE TABLE IF NOT EXISTS domains (
        user TEXT NOT NULL, domain TEXT NOT NULL, enabled INTEGER, kind TEXT,
        first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, removed_at INTEGER,
        PRIMARY KEY (user, domain))",
    "CREATE TABLE IF NOT EXISTS domain_summary (
        user TEXT NOT NULL, domain TEXT NOT NULL, db_bytes INTEGER NOT NULL DEFAULT 0,
        last_collect_at INTEGER, last_line_ts INTEGER, lines_total INTEGER NOT NULL DEFAULT 0,
        visits_today INTEGER NOT NULL DEFAULT 0, pageviews_today INTEGER NOT NULL DEFAULT 0,
        visits_7d INTEGER NOT NULL DEFAULT 0, pageviews_7d INTEGER NOT NULL DEFAULT 0,
        bytes_7d INTEGER NOT NULL DEFAULT 0, visits_30d INTEGER NOT NULL DEFAULT 0,
        pageviews_30d INTEGER NOT NULL DEFAULT 0, bytes_30d INTEGER NOT NULL DEFAULT 0,
        bot_hits_30d INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (user, domain))",
    "CREATE TABLE IF NOT EXISTS salts (day TEXT PRIMARY KEY, salt BLOB NOT NULL)",
    "CREATE TABLE IF NOT EXISTS jobs (
        id INTEGER PRIMARY KEY, type TEXT NOT NULL, scope TEXT, started_at INTEGER NOT NULL,
        finished_at INTEGER, status TEXT NOT NULL, lines INTEGER NOT NULL DEFAULT 0,
        files INTEGER NOT NULL DEFAULT 0, domains INTEGER NOT NULL DEFAULT 0, message TEXT)",
];

/// `access(2)` check like PHP's `is_readable()` / `is_writable()`.
pub fn access(path: &Path, mode: libc::c_int) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    unsafe { libc::access(c.as_ptr(), mode) == 0 }
}

/// A job start refused because of a conflict (HTTP 409).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Conflict(pub String);

/// An open registry.
pub struct Registry {
    conn: Connection,
    path: PathBuf,
    default_enabled: bool,
}

fn tri(v: &SqlVal) -> Option<bool> {
    if v.is_null() {
        None
    } else {
        Some(v.int() == 1)
    }
}

fn opt_int(v: &SqlVal) -> Value {
    if v.is_null() {
        Value::Null
    } else {
        json!(v.int())
    }
}

fn raw(v: &SqlVal) -> Value {
    match v {
        SqlVal::Null => Value::Null,
        SqlVal::Int(i) => json!(i),
        SqlVal::Real(f) => crate::reports::php::fjson(*f),
        SqlVal::Text(s) => json!(s),
        SqlVal::Blob(b) => json!(String::from_utf8_lossy(b)),
    }
}

impl Registry {
    /// Opens (creating and migrating when writable) the registry at `path`.
    /// `version` is recorded in `meta.version` when writable.
    pub fn open(path: &Path, default_enabled: bool, version: &str, now: i64) -> Result<Registry> {
        if let Some(dir) = path.parent()
            && !dir.is_dir()
        {
            crate::util::fs::ensure_dir(dir, 0o711, None, None)?;
        }
        let fresh = !path.is_file();
        if !fresh && !access(path, libc::R_OK) {
            anyhow::bail!("registry {} is not readable", path.display());
        }
        let writable = fresh || access(path, libc::W_OK);
        if fresh {
            // 0600 from the first byte (and so is SQLite's -journal, which copies it)
            crate::util::fs::precreate_private(path)
                .map_err(|e| anyhow::anyhow!("cannot create registry {}: {e}", path.display()))?;
        }
        let flags = if writable {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        };
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        if writable {
            conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA synchronous = NORMAL;")?;
            migrate(&conn)?;
            if fresh {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                }
                conn.execute(
                    "INSERT OR IGNORE INTO meta (key, value) VALUES (?, ?)",
                    ["installed_at", &now.to_string()],
                )?;
            }
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('version', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [version],
            )?;
        }
        Ok(Registry {
            conn,
            path: path.to_path_buf(),
            default_enabled,
        })
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The connection (tests, maintenance).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    fn rows(&self, sql: &str, args: &[Arg]) -> Result<Vec<Row>> {
        query_rows(&self.conn, sql, args)
    }

    /// Settings overrides (dotted key → decoded JSON value), corrupt values skipped.
    pub fn settings(&self) -> Result<Vec<(String, Value)>> {
        let mut out = Vec::new();
        for r in self.rows("SELECT key, value FROM settings ORDER BY key", &[])? {
            if let Ok(v) = serde_json::from_str::<Value>(&r.s("value")) {
                out.push((r.s("key"), v));
            }
        }
        Ok(out)
    }

    /// Stores a (validated) setting override.
    pub fn set_setting(&self, key: &str, value: &Value) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, &pjson::encode(value, pjson::PLAIN)],
        )?;
        Ok(())
    }

    fn account_row(&self, r: &Row) -> Map<String, Value> {
        let ov = tri(r.get("enabled"));
        let mut m = Map::new();
        m.insert("user".into(), json!(r.s("user")));
        m.insert("owner".into(), json!(r.os("owner")));
        m.insert("enabled".into(), json!(ov.unwrap_or(self.default_enabled)));
        m.insert("enabled_override".into(), json!(ov));
        m.insert("timezone".into(), json!(r.os("timezone")));
        m.insert("first_seen".into(), json!(r.i("first_seen")));
        m.insert("last_seen".into(), json!(r.i("last_seen")));
        m.insert("removed_at".into(), opt_int(r.get("removed_at")));
        m.insert("domains".into(), json!(r.i("domains")));
        m
    }

    /// AccountRows (`owner` = reseller scope, `q` = substring of the user or one of its domains).
    pub fn accounts(
        &self,
        owner: Option<&str>,
        q: Option<&str>,
    ) -> Result<Vec<Map<String, Value>>> {
        let mut sql = "SELECT a.*, (SELECT COUNT(*) FROM domains d WHERE d.user = a.user AND d.removed_at IS NULL) AS domains
                FROM accounts a WHERE 1 = 1"
            .to_string();
        let mut args: Vec<Arg> = Vec::new();
        if let Some(o) = owner {
            sql.push_str(" AND a.owner = ?");
            args.push(o.into());
        }
        if let Some(q) = q.filter(|q| !q.is_empty()) {
            sql.push_str(
                " AND (a.user LIKE ? ESCAPE '\\' OR EXISTS (SELECT 1 FROM domains d WHERE d.user = a.user AND d.domain LIKE ? ESCAPE '\\'))",
            );
            let like = format!(
                "%{}%",
                crate::reports::report::like_escape(&q.to_ascii_lowercase())
            );
            args.push(Arg::S(like.clone()));
            args.push(Arg::S(like));
        }
        sql.push_str(" ORDER BY a.user");
        Ok(self
            .rows(&sql, &args)?
            .iter()
            .map(|r| self.account_row(r))
            .collect())
    }

    /// One AccountRow.
    pub fn account(&self, user: &str) -> Result<Option<Map<String, Value>>> {
        Ok(self
            .rows(
                "SELECT a.*, (SELECT COUNT(*) FROM domains d WHERE d.user = a.user AND d.removed_at IS NULL) AS domains FROM accounts a WHERE a.user = ?",
                &[user.into()],
            )?
            .first()
            .map(|r| self.account_row(r)))
    }

    fn ensure_account(&self, user: &str, owner: Option<&str>, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO accounts (user, owner, enabled, timezone, first_seen, last_seen) VALUES (?, ?, NULL, NULL, ?, ?)",
            rusqlite::params![user, owner, now, now],
        )?;
        Ok(())
    }

    /// Sets the account switch; `timezone`: `None` = unchanged, `""` = clear.
    pub fn set_account(
        &self,
        user: &str,
        enabled: Option<bool>,
        timezone: Option<&str>,
        owner: Option<&str>,
        now: i64,
    ) -> Result<()> {
        self.ensure_account(user, owner, now)?;
        self.conn.execute(
            "UPDATE accounts SET enabled = ? WHERE user = ?",
            rusqlite::params![enabled.map(i64::from), user],
        )?;
        if let Some(tz) = timezone {
            let v = if tz.is_empty() { None } else { Some(tz) };
            self.conn.execute(
                "UPDATE accounts SET timezone = ? WHERE user = ?",
                rusqlite::params![v, user],
            )?;
        }
        Ok(())
    }

    /// DomainRows (optionally of one user) with effective switches.
    pub fn domains(&self, user: Option<&str>) -> Result<Vec<Map<String, Value>>> {
        let mut sql = "SELECT d.*, a.enabled AS account_enabled FROM domains d LEFT JOIN accounts a ON a.user = d.user".to_string();
        let mut args: Vec<Arg> = Vec::new();
        if let Some(u) = user {
            sql.push_str(" WHERE d.user = ?");
            args.push(u.into());
        }
        sql.push_str(" ORDER BY d.user, d.domain");
        let mut out = Vec::new();
        for r in self.rows(&sql, &args)? {
            let acc = tri(r.get("account_enabled"));
            let dom = tri(r.get("enabled"));
            let mut m = Map::new();
            m.insert("user".into(), json!(r.s("user")));
            m.insert("domain".into(), json!(r.s("domain")));
            m.insert(
                "kind".into(),
                json!(r.os("kind").unwrap_or_else(|| "unknown".into())),
            );
            m.insert(
                "enabled".into(),
                json!(effective(acc, dom, self.default_enabled)),
            );
            m.insert("enabled_override".into(), json!(dom));
            m.insert(
                "account_enabled".into(),
                json!(acc.unwrap_or(self.default_enabled)),
            );
            m.insert("first_seen".into(), json!(r.i("first_seen")));
            m.insert("last_seen".into(), json!(r.i("last_seen")));
            m.insert("removed_at".into(), opt_int(r.get("removed_at")));
            out.push(m);
        }
        Ok(out)
    }

    /// Sets the domain switch (`None` = follow the account).
    pub fn set_domain(
        &self,
        user: &str,
        domain: &str,
        enabled: Option<bool>,
        owner: Option<&str>,
        now: i64,
    ) -> Result<()> {
        self.ensure_account(user, owner, now)?;
        self.conn.execute(
            "INSERT INTO domains (user, domain, enabled, kind, first_seen, last_seen) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(user, domain) DO UPDATE SET enabled = excluded.enabled",
            rusqlite::params![user, domain, enabled.map(i64::from), "unknown", now, now],
        )?;
        Ok(())
    }

    /// `domain_summary` rows (optionally of some users), ints cast.
    pub fn summaries(&self, users: Option<&[String]>) -> Result<Vec<Map<String, Value>>> {
        let mut sql = "SELECT * FROM domain_summary".to_string();
        let mut args: Vec<Arg> = Vec::new();
        if let Some(u) = users {
            if u.is_empty() {
                return Ok(Vec::new());
            }
            sql.push_str(&format!(
                " WHERE user IN ({})",
                vec!["?"; u.len()].join(",")
            ));
            args.extend(u.iter().map(Arg::from));
        }
        sql.push_str(" ORDER BY user, domain");
        let mut out = Vec::new();
        for r in self.rows(&sql, &args)? {
            let mut m = Map::new();
            for (c, v) in r.columns().iter().zip(r.values()) {
                let val = if c != "user" && c != "domain" && !v.is_null() {
                    json!(v.int())
                } else {
                    raw(v)
                };
                m.insert(c.clone(), val);
            }
            out.push(m);
        }
        Ok(out)
    }

    /// Removes summary rows of a user (or one domain).
    pub fn delete_summary(&self, user: &str, domain: Option<&str>) -> Result<()> {
        match domain {
            None => self
                .conn
                .execute("DELETE FROM domain_summary WHERE user = ?", [user])?,
            Some(d) => self.conn.execute(
                "DELETE FROM domain_summary WHERE user = ? AND domain = ?",
                [user, d],
            )?,
        };
        Ok(())
    }

    /// Last jobs, newest first.
    pub fn jobs(&self, limit: i64) -> Result<Vec<Map<String, Value>>> {
        let mut out = Vec::new();
        for r in self.rows(
            "SELECT * FROM jobs ORDER BY id DESC LIMIT ?",
            &[Arg::I(limit.clamp(1, 500))],
        )? {
            let mut m = Map::new();
            for (c, v) in r.columns().iter().zip(r.values()) {
                let val = if [
                    "id",
                    "started_at",
                    "finished_at",
                    "lines",
                    "files",
                    "domains",
                ]
                .contains(&c.as_str())
                {
                    opt_int(v)
                } else {
                    raw(v)
                };
                m.insert(c.clone(), val);
            }
            out.push(m);
        }
        Ok(out)
    }

    /// Status of one job.
    pub fn job_status(&self, id: i64) -> Result<Option<String>> {
        Ok(self
            .rows("SELECT status FROM jobs WHERE id = ?", &[Arg::I(id)])?
            .first()
            .map(|r| r.s("status")))
    }

    /// Atomically starts a job unless the same type and scope is running or
    /// `max` jobs are running (only jobs started since `since` count).
    pub fn start_job_limited(
        &self,
        kind: &str,
        scope: &str,
        since: i64,
        max: usize,
        now: i64,
    ) -> Result<i64> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let res = (|| -> Result<i64> {
            let running = self.rows(
                "SELECT id, type, scope, started_at FROM jobs WHERE status = 'running' AND started_at >= ? ORDER BY id",
                &[Arg::I(since)],
            )?;
            for j in &running {
                if j.s("type") == kind && j.os("scope").as_deref() == Some(scope) {
                    return Err(Conflict(format!(
                        "A {kind} job for this scope is already running (#{})",
                        j.i("id")
                    ))
                    .into());
                }
            }
            if running.len() >= max {
                return Err(Conflict(
                    "Too many jobs are running; try again when one has finished".into(),
                )
                .into());
            }
            self.conn.execute(
                "INSERT INTO jobs (type, scope, started_at, status) VALUES (?, ?, ?, 'running')",
                rusqlite::params![kind, scope, now],
            )?;
            let id = self.conn.last_insert_rowid();
            self.conn.execute_batch("DELETE FROM jobs WHERE id <= (SELECT id FROM jobs ORDER BY id DESC LIMIT 1 OFFSET 500)")?;
            Ok(id)
        })();
        match &res {
            Ok(_) => self.conn.execute_batch("COMMIT")?,
            Err(_) => {
                let _ = self.conn.execute_batch("ROLLBACK");
            }
        }
        res
    }

    /// Finishes a job (`ok` | `error` | `skipped`).
    pub fn finish_job(&self, id: i64, status: &str, message: Option<&str>, now: i64) -> Result<()> {
        let msg = message.map(|m| {
            let clean = crate::log::sanitize(m);
            let mut cut = clean.len().min(1000);
            while !clean.is_char_boundary(cut) {
                cut -= 1;
            }
            clean[..cut].to_string()
        });
        self.conn.execute(
            "UPDATE jobs SET finished_at = ?, status = ?, lines = ?, files = ?, domains = ?, message = ? WHERE id = ?",
            rusqlite::params![now, status, 0, 0, 0, msg, id],
        )?;
        Ok(())
    }
}

/// Effective switch: an account explicitly disabled turns off all of its
/// domains; otherwise a domain override wins, else the account, else default.
pub fn effective(account: Option<bool>, domain: Option<bool>, default: bool) -> bool {
    if account == Some(false) {
        return false;
    }
    domain.or(account).unwrap_or(default)
}

fn migrate(conn: &Connection) -> Result<()> {
    let current = schema_version(conn);
    if current > REGISTRY_VERSION {
        return Err(WpError::SchemaTooNew {
            found: current,
            supported: REGISTRY_VERSION,
        }
        .into());
    }
    if current == REGISTRY_VERSION {
        return Ok(());
    }
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let res = (|| -> Result<()> {
        if schema_version(conn) < 1 {
            for sql in MIGRATION_1 {
                conn.execute_batch(sql)?;
            }
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('schema_version', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                ["1"],
            )?;
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
    Ok(())
}
