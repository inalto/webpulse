//! Nightly maintenance of the domain databases (docs/ARCHITECTURE.md step 7),
//! `WebPulse\Rollup\Rollup`: close stale visits, cap high-cardinality dimensions per
//! day, purge raw tables, prune visitor_day / salts, refresh the summary, optimise
//! and vacuum. The caller (CLI) holds the per-user lock.

use anyhow::Result;
use rusqlite::params;

use super::collector::Collector;
use super::store::DomainStore;
use super::store::dims::{OTHER, VISIT_LEVEL};
use crate::app::App;
use crate::paths::{is_valid_domain, is_valid_user};
use crate::util::{fs, time};

/// Dims never folded (fixed small key sets).
const NO_CAP: &[&str] = &["visit_duration", "pages_per_visit", "channel", "device"];

/// Statistics of one rollup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RollupStats {
    /// Visits closed.
    pub closed: i64,
    /// dim rows folded into `(other)`.
    pub folded: i64,
    /// Visits purged.
    pub purged_visits: i64,
    /// Events purged.
    pub purged_events: i64,
    /// visitor_day rows pruned.
    pub pruned_visitor_day: i64,
    /// 1 when the database was vacuumed.
    pub vacuum: i64,
}

/// The rollup.
pub struct Rollup<'a> {
    app: &'a App,
    collector: Collector<'a>,
}

impl std::fmt::Debug for Rollup<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rollup").finish()
    }
}

impl<'a> Rollup<'a> {
    /// A rollup with its own collector (to close stale visits).
    pub fn new(app: &'a App) -> Rollup<'a> {
        Rollup {
            app,
            collector: Collector::new(app),
        }
    }

    /// Users that have a data directory.
    pub fn users(&self) -> Vec<String> {
        let dir = self.app.paths.data_dir();
        fs::scandir(dir)
            .into_iter()
            .filter(|u| !u.starts_with('.') && is_valid_user(u) && dir.join(u).is_dir())
            .collect()
    }

    /// Domains of a user that have a database (sorted).
    pub fn domains_with_data(&self, user: &str) -> Vec<String> {
        let Ok(dir) = self.app.paths.try_user_dir(user) else {
            return Vec::new();
        };
        let mut out: Vec<String> = fs::scandir(&dir)
            .into_iter()
            .filter(|n| !n.starts_with('.'))
            .filter_map(|n| n.strip_suffix(".db").map(str::to_string))
            .filter(|d| is_valid_domain(d) && !fs::is_link(&dir.join(format!("{d}.db"))))
            .collect();
        out.sort();
        out
    }

    /// Prunes registry salts older than 2 days (plus the server day).
    pub fn prune_salts(&self) -> Result<usize> {
        let today = time::DayClock::new(&self.app.server_timezone())
            .day(self.app.now())
            .to_string();
        self.app
            .registry()?
            .prune_salts(&time::add_days(&today, -3))
    }

    /// Rolls up one domain database.
    pub fn rollup_domain(&mut self, user: &str, domain: &str) -> Result<RollupStats> {
        let app = self.app;
        let store = DomainStore::open_write(&app.paths, user, domain)?;
        let conn = store.conn();
        let tz = store
            .get_meta("timezone")
            .unwrap_or_else(|| app.server_timezone());
        let now = app.now();
        let today = time::DayClock::new(&tz).day(now).to_string();
        let mut stats = RollupStats {
            closed: self.collector.close_stale_visits(&store, &tz)? as i64,
            ..RollupStats::default()
        };

        // cap dimensions of finished days
        let max = app.config.int("max_rows_per_dim", 1000).max(10);
        let finished = time::add_days(&today, -1);
        let groups: Vec<(String, String)> = {
            let mut st = conn.prepare("SELECT day, dim, COUNT(*) AS n FROM dim_daily WHERE day < ? GROUP BY day, dim HAVING COUNT(*) > ?")?;
            let rows = st.query_map(params![finished, max], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !groups.is_empty() {
            tx(conn, || {
                for (day, dim) in &groups {
                    if NO_CAP.contains(&dim.as_str()) {
                        continue;
                    }
                    stats.folded += fold(conn, day, dim, max)?;
                }
                Ok(())
            })?;
        }

        // raw retention
        let days = app.config.int("raw_retention_days", 30).max(1);
        let cut_day = time::add_days(&today, -days);
        let cut_ts = time::local_midnight(&cut_day, &time::tz_or_server(Some(&tz)));
        tx(conn, || {
            stats.purged_visits =
                conn.execute("DELETE FROM visits WHERE day < ?", params![cut_day])? as i64;
            stats.purged_events =
                conn.execute("DELETE FROM events WHERE ts < ?", params![cut_ts])? as i64;
            stats.pruned_visitor_day = conn.execute(
                "DELETE FROM visitor_day WHERE day < ?",
                params![time::add_days(&today, -2)],
            )? as i64;
            store.set_meta_many(&[("last_rollup_at", Some(now.to_string()))])?;
            Ok(())
        })?;

        conn.execute_batch("PRAGMA optimize")?;
        let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        let free: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
        if pages > 0 && free as f64 / pages as f64 > 0.25 {
            conn.execute_batch("VACUUM")?;
            stats.vacuum = 1;
        }
        if let Ok(reg) = app.registry()
            && let Err(e) = reg.refresh_summary(&store, &tz, Some(now))
        {
            app.log
                .warn(format!("summary refresh failed for {user}/{domain}: {e}"));
        }
        Ok(stats)
    }
}

fn tx(conn: &rusqlite::Connection, f: impl FnOnce() -> Result<()>) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match f() {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Keeps the top `max - 1` keys of one (day, dim) and folds the rest into `(other)`.
fn fold(conn: &rusqlite::Connection, day: &str, dim: &str, max: i64) -> Result<i64> {
    let order = if dim == "page" {
        "pageviews DESC, visits DESC, hits DESC"
    } else if VISIT_LEVEL.contains(&dim) || dim == "entry_page" || dim == "exit_page" {
        "visits DESC, pageviews DESC, hits DESC"
    } else {
        "hits DESC, bytes DESC"
    };
    let tail: Vec<String> = {
        let mut st = conn.prepare(&format!("SELECT key FROM dim_daily WHERE day = ? AND dim = ? AND key <> ? ORDER BY {order}, key LIMIT -1 OFFSET ?"))?;
        let rows = st.query_map(params![day, dim, OTHER, max - 1], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if tail.is_empty() {
        return Ok(0);
    }
    let mut sum = [0i64; 6];
    {
        let mut sel = conn.prepare_cached("SELECT visits, pageviews, hits, bytes, bounces, duration FROM dim_daily WHERE day = ? AND dim = ? AND key = ?")?;
        let mut del =
            conn.prepare_cached("DELETE FROM dim_daily WHERE day = ? AND dim = ? AND key = ?")?;
        for k in &tail {
            let row: Option<[i64; 6]> = sel
                .query_row(params![day, dim, k], |r| {
                    Ok([
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ])
                })
                .ok();
            if let Some(r) = row {
                for i in 0..6 {
                    sum[i] += r[i];
                }
            }
            del.execute(params![day, dim, k])?;
        }
    }
    conn.execute(
        "INSERT INTO dim_daily (day, dim, key, visits, pageviews, hits, bytes, bounces, duration) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(day, dim, key) DO UPDATE SET visits = visits + excluded.visits, pageviews = pageviews + excluded.pageviews,
            hits = hits + excluded.hits, bytes = bytes + excluded.bytes, bounces = bounces + excluded.bounces, duration = duration + excluded.duration",
        params![day, dim, OTHER, sum[0], sum[1], sum[2], sum[3], sum[4], sum[5]],
    )?;
    Ok(tail.len() as i64)
}
