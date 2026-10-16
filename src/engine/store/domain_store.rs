//! One SQLite database per domain: `<data_dir>/<user>/<domain>.db`
//! (`WebPulse\Store\DomainStore`).
//!
//! Writer (root): `journal_mode=DELETE` so the account can read with read permission
//! only; file 0640 root:<user>, directory 0750 root:<user>. Reader (dashboard):
//! opened with `SQLITE_OPEN_READ_ONLY` and `query_only`.

use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use indexmap::IndexMap;
use rusqlite::{Connection, OpenFlags, params};

use super::{migrate, schema};
use crate::error::WpError;
use crate::paths::{Paths, assert_domain, assert_user};
use crate::util::fs;

/// Converts any SQLite value to PHP's string cast (`null` stays `None`).
pub fn value_to_string(v: rusqlite::types::ValueRef<'_>) -> Option<String> {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(crate::util::json::php_float(f)),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
    }
}

/// An open domain database.
#[derive(Debug)]
pub struct DomainStore {
    conn: Connection,
    path: PathBuf,
    user: String,
    domain: String,
    read_only: bool,
    meta: RefCell<Option<IndexMap<String, Option<String>>>>,
}

impl DomainStore {
    /// Opens (and for writers creates/migrates) a domain database.
    ///
    /// Errors: [`WpError::NotFound`] for a read-only open of a missing database,
    /// [`WpError::SchemaTooNew`] for a database newer than this binary, `invalid …
    /// name` for bad names, I/O and SQLite errors otherwise.
    pub fn open(paths: &Paths, user: &str, domain: &str, read_only: bool) -> Result<DomainStore> {
        if read_only {
            DomainStore::open_read_only(paths, user, domain)
        } else {
            DomainStore::open_write(paths, user, domain)
        }
    }

    /// Read-only open (dashboard / reports): never creates anything.
    pub fn open_read_only(paths: &Paths, user: &str, domain: &str) -> Result<DomainStore> {
        let file = paths.try_domain_db(user, domain)?;
        if fs::is_link(&file) {
            return Err(anyhow!("refusing symlinked database {}", file.display()));
        }
        if !file.is_file() {
            return Err(WpError::NotFound(format!("no data for {domain}")).into());
        }
        let conn = Connection::open_with_flags(
            &file,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_millis(5000))?;
        conn.execute_batch("PRAGMA query_only = ON")?;
        let v = migrate::version(&conn);
        if v > schema::VERSION {
            return Err(WpError::SchemaTooNew {
                found: v,
                supported: schema::VERSION,
            }
            .into());
        }
        Ok(DomainStore {
            conn,
            path: file,
            user: user.to_string(),
            domain: domain.to_string(),
            read_only: true,
            meta: RefCell::new(None),
        })
    }

    /// Writer open: creates the user directory and the database when missing and
    /// applies the migrations.
    pub fn open_write(paths: &Paths, user: &str, domain: &str) -> Result<DomainStore> {
        let file = paths.try_domain_db(user, domain)?;
        if fs::is_link(&file) {
            return Err(anyhow!("refusing symlinked database {}", file.display()));
        }
        DomainStore::prepare_user_dir(paths, user)?;
        let fresh = !file.is_file();
        let conn = Connection::open(&file)?;
        conn.busy_timeout(Duration::from_millis(5000))?;
        conn.set_prepared_statement_cache_capacity(64);
        let _: String = conn.query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
        migrate::migrate(&conn, &schema::domain_migrations())?;
        if fresh {
            let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640));
            fs::own(&file, Some("root"), Some(user));
        }
        let store = DomainStore {
            conn,
            path: file,
            user: user.to_string(),
            domain: domain.to_string(),
            read_only: false,
            meta: RefCell::new(None),
        };
        if fresh || store.get_meta("domain").is_none() {
            let next = store
                .get_meta("next_visit_id")
                .unwrap_or_else(|| "1".into());
            let total = store.get_meta("total_lines").unwrap_or_else(|| "0".into());
            store.set_meta_many(&[
                ("user", Some(user.to_string())),
                ("domain", Some(domain.to_string())),
                ("created_at", Some(crate::util::time::now_ts().to_string())),
                ("next_visit_id", Some(next)),
                ("total_lines", Some(total)),
            ])?;
        }
        Ok(store)
    }

    /// Creates `<data_dir>/<user>` (0750 root:<user>) and its `user/` sub-directory
    /// (0700 <user>:<user>) when missing. Ownership is only applied as root; a
    /// symlinked `user/` is never followed.
    pub fn prepare_user_dir(paths: &Paths, user: &str) -> Result<()> {
        assert_user(user)?;
        let data = paths.data_dir();
        if !data.is_dir() {
            fs::ensure_dir(data, 0o711, Some("root"), Some("root"))?;
        }
        let dir = paths.user_dir(user);
        if !dir.is_dir() || fs::is_root() {
            fs::ensure_dir(&dir, 0o750, Some("root"), Some(user))?;
        }
        let private = paths.user_private_dir(user);
        if fs::is_link(&private) {
            return Ok(());
        }
        if !private.is_dir() {
            fs::ensure_dir(&private, 0o700, Some(user), Some(user))?;
        }
        Ok(())
    }

    /// The connection (read-only when opened read-only).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
    /// True for a read-only store.
    pub fn read_only(&self) -> bool {
        self.read_only
    }
    /// Database file.
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Account name.
    pub fn user(&self) -> &str {
        &self.user
    }
    /// Domain name.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The meta table (cached until the next write).
    pub fn meta(&self) -> IndexMap<String, Option<String>> {
        self.load_meta();
        self.meta.borrow().clone().unwrap_or_default()
    }

    fn load_meta(&self) {
        if self.meta.borrow().is_some() {
            return;
        }
        let mut m = IndexMap::new();
        if let Ok(mut st) = self.conn.prepare("SELECT key, value FROM meta")
            && let Ok(mut rows) = st.query([])
        {
            while let Ok(Some(r)) = rows.next() {
                let k = r
                    .get_ref(0)
                    .ok()
                    .and_then(value_to_string)
                    .unwrap_or_default();
                let v = r.get_ref(1).ok().and_then(value_to_string);
                m.insert(k, v);
            }
        }
        *self.meta.borrow_mut() = Some(m);
    }

    /// One meta value (`None` when absent or NULL).
    pub fn get_meta(&self, key: &str) -> Option<String> {
        self.load_meta();
        self.meta
            .borrow()
            .as_ref()
            .and_then(|m| m.get(key).cloned().flatten())
    }

    /// Sets one meta key (writer only; joins the current transaction if any).
    pub fn set_meta(&self, key: &str, value: Option<&str>) -> Result<()> {
        self.set_meta_many(&[(key, value.map(str::to_string))])
    }

    /// Sets several meta keys, in order.
    pub fn set_meta_many(&self, values: &[(&str, Option<String>)]) -> Result<()> {
        if self.read_only {
            return Err(anyhow!("read-only store"));
        }
        let mut st = self.conn.prepare_cached("INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")?;
        for (k, v) in values {
            st.execute(params![k, v])?;
        }
        *self.meta.borrow_mut() = None;
        Ok(())
    }

    /// Drops the cached meta (after external writes).
    pub fn reload_meta(&self) {
        *self.meta.borrow_mut() = None;
    }

    /// Validates names without opening anything (`invalid user/domain name`).
    pub fn check_names(user: &str, domain: &str) -> Result<()> {
        assert_user(user)?;
        assert_domain(domain)?;
        Ok(())
    }
}
