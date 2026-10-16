//! Applies numbered migrations; the current version lives in
//! `meta.schema_version` (domain databases and the registry), `WebPulse\Store\Migrator`.

use anyhow::Result;
use rusqlite::{Connection, params};

use crate::error::WpError;

/// Reads `meta.schema_version` (0 when the database is empty or has no meta table).
pub fn version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |r| r.get::<_, rusqlite::types::Value>(0),
    )
    .map(|v| match v {
        rusqlite::types::Value::Integer(i) => i,
        rusqlite::types::Value::Text(s) => crate::config::php_str_int(&s),
        rusqlite::types::Value::Real(f) => f as i64,
        _ => 0,
    })
    .unwrap_or(0)
}

/// Brings `conn` up to the newest migration (writer side only); returns the version.
/// Fails with [`WpError::SchemaTooNew`] when the database is newer than the code.
pub fn migrate(conn: &Connection, migrations: &[(i64, &[&str])]) -> Result<i64> {
    let latest = migrations.iter().map(|m| m.0).max().unwrap_or(0);
    let current = version(conn);
    if current > latest {
        return Err(WpError::SchemaTooNew {
            found: current,
            supported: latest,
        }
        .into());
    }
    if current == latest {
        return Ok(current);
    }
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let run = || -> Result<i64> {
        let mut current = version(conn);
        let mut sorted: Vec<&(i64, &[&str])> = migrations.iter().collect();
        sorted.sort_by_key(|m| m.0);
        for (v, statements) in sorted {
            if *v <= current {
                continue;
            }
            for sql in statements.iter() {
                conn.execute_batch(sql)?;
            }
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('schema_version', ?)
                    ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![v.to_string()],
            )?;
            current = *v;
        }
        Ok(current)
    };
    match run() {
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
