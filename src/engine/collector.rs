//! The collector pipeline (docs/ARCHITECTURE.md), `WebPulse\Collect\Collector`:
//! discovery → LogSource → LogParser → classifiers → Sessionizer → DomainStore.
//!
//! All the live logs of a domain (`<domain>` and `<domain>-ssl_log`, possibly in
//! several directories) are merged by timestamp so that a visit spanning http and
//! https is sessionized correctly. Counters and checkpoints are written in one
//! transaction per domain (every [`FLUSH_LINES`] lines and at the end). A bad line
//! or an unreadable file never aborts the run. The caller (CLI) holds the per-user
//! lock.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::types::Value as SqlValue;
use serde_json::json;

use super::discovery::{Archive, LogFile, LogFiles};
use super::logsource::{Checkpoint, LogSource};
use super::parser::{Hit, HitRef, LogParser};
use super::run::{ArchiveRow, CheckpointRow, Classifiers, DomainRun, class};
use super::store::DomainStore;
use super::user_settings::{DomainSettings, UserSettings};
use crate::app::App;
use crate::log::Level;
use crate::paths::{assert_user, is_valid_user};
use crate::util::time;

/// Intermediate flush every N lines (bounds memory).
pub const FLUSH_LINES: u64 = 100_000;
/// Sweep stale visits every N lines.
const SWEEP_LINES: u64 = 10_000;
/// Tolerance for out-of-order lines when sweeping visits mid-run.
const SWEEP_SLACK: i64 = 300;

/// Totals of [`Collector::collect_user`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserTotals {
    /// Domains collected (idle ones included).
    pub domains: i64,
    /// Log files read.
    pub files: i64,
    /// Lines read.
    pub lines: i64,
    /// Unparseable lines.
    pub bad: i64,
    /// Ignored lines.
    pub ignored: i64,
    /// Errors (unreadable files, failed domains).
    pub errors: i64,
    /// Skipped domains with the reason.
    pub skipped: Vec<String>,
}

/// Statistics of one domain run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainStats {
    /// Files read.
    pub files: i64,
    /// Lines read.
    pub lines: i64,
    /// Unparseable lines.
    pub bad: i64,
    /// Ignored lines.
    pub ignored: i64,
    /// Errors.
    pub errors: i64,
    /// Checkpoints discarded (`file:reason`).
    pub resets: Vec<String>,
    /// Nothing to do (no write at all).
    pub idle: bool,
    /// Visits closed early because of `max_open_visits`.
    pub visits_closed_early: i64,
}

/// Totals of [`Collector::backfill_user`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackfillTotals {
    /// Domains imported.
    pub domains: i64,
    /// Archives read.
    pub files: i64,
    /// Lines read.
    pub lines: i64,
    /// Lines imported (not already covered).
    pub imported: i64,
    /// Unparseable lines.
    pub bad: i64,
    /// Ignored lines.
    pub ignored: i64,
    /// Archives already imported.
    pub skipped_files: i64,
    /// Errors.
    pub errors: i64,
    /// Skipped domains with the reason.
    pub skipped: Vec<String>,
}

/// Per-source statistics since the last flush.
#[derive(Debug, Clone, Default)]
struct SrcStat {
    lines: i64,
    bad: i64,
    ignored: i64,
    offset: u64,
    /// Inode `offset` belongs to (a rotated live log is finished before the new file).
    inode: u64,
    last_ts: Option<i64>,
    max_ts: Option<i64>,
}

#[derive(Default)]
struct Head {
    line: Vec<u8>,
    scratch: Vec<u8>,
    hit: HitRef,
    offset: u64,
    inode: u64,
    valid: bool,
}

#[derive(Default)]
struct RunCounts {
    lines: i64,
    bad: i64,
    ignored: i64,
    errors: i64,
}

enum Skip<'s> {
    None,
    UpTo(i64),
    Backfill { limit: i64, resume: &'s [i64] },
}

impl Skip<'_> {
    fn skip(&self, ts: i64, i: usize) -> bool {
        match self {
            Skip::None => false,
            Skip::UpTo(u) => ts <= *u,
            Skip::Backfill { limit, resume } => ts >= *limit || ts <= resume[i],
        }
    }
}

/// The collector of one process.
pub struct Collector<'a> {
    app: &'a App,
    cls: Classifiers,
    parser: LogParser,
    log_files: LogFiles,
}

impl std::fmt::Debug for Collector<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collector").finish()
    }
}

fn sql_i64(v: &SqlValue) -> Option<i64> {
    match v {
        SqlValue::Integer(i) => Some(*i),
        SqlValue::Real(f) => Some(*f as i64),
        SqlValue::Text(s) => Some(crate::config::php_str_int(s)),
        _ => None,
    }
}

fn sql_string(v: &SqlValue) -> Option<String> {
    match v {
        SqlValue::Null => None,
        SqlValue::Integer(i) => Some(i.to_string()),
        SqlValue::Real(f) => Some(crate::util::json::php_float(*f)),
        SqlValue::Text(s) => Some(s.clone()),
        SqlValue::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
    }
}

fn basename(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl<'a> Collector<'a> {
    /// Builds the parser and classifiers once for the whole run.
    pub fn new(app: &'a App) -> Collector<'a> {
        Collector {
            app,
            cls: Classifiers::new(&app.config),
            parser: LogParser::new(),
            log_files: LogFiles::new(&app.paths),
        }
    }

    /// Log discovery helper.
    pub fn log_files(&self) -> &LogFiles {
        &self.log_files
    }

    /// Discovery → registry for one user (or all users); returns the users seen.
    pub fn sync_registry(&self, only_user: Option<&str>) -> Result<Vec<String>> {
        let cp = self.app.provider();
        let reg = self.app.registry()?;
        let users = match only_user {
            Some(u) => {
                let info = if is_valid_user(u) && u != "root" && u != "system" {
                    cp.get(u)
                } else {
                    None
                };
                let mut m = BTreeMap::new();
                if let Some(i) = info {
                    m.insert(u.to_string(), i);
                }
                m
            }
            None => cp.all(),
        };
        for (user, info) in &users {
            let mut domains: Vec<(String, String)> = info
                .kinds
                .iter()
                .map(|(d, k)| (d.clone(), k.clone()))
                .collect();
            for d in cp.extra_domains(user) {
                if !domains.iter().any(|(x, _)| *x == d) {
                    domains.push((d, "unknown".into()));
                }
            }
            reg.sync_user(user, info.owner.as_deref(), &domains)?;
        }
        let names: Vec<String> = users.keys().cloned().collect();
        if only_user.is_none() {
            reg.mark_removed_except(&names)?;
        }
        Ok(names)
    }

    /// Collects every enabled domain of a user (or only `only_domain`).
    pub fn collect_user(&mut self, user: &str, only_domain: Option<&str>) -> Result<UserTotals> {
        assert_user(user)?;
        let mut tot = UserTotals::default();
        let reg = self.app.registry()?;
        let (settings, err) = UserSettings::read(&self.app.paths, user);
        if let Some(e) = err {
            self.app
                .log
                .warn(format!("ignoring settings.json of {user}: {e}"));
        }
        let mut domains: Vec<String> = reg
            .domains(Some(user))?
            .into_iter()
            .filter(|r| r.removed_at.is_none())
            .map(|r| r.domain)
            .collect();
        if let Some(od) = only_domain {
            domains.retain(|d| d == od);
            if domains.is_empty() {
                // never collect a domain of another account into this one
                tot.skipped.push(format!("{od} (not a domain of {user})"));
                return Ok(tot);
            }
        }
        for domain in domains {
            if !reg.is_enabled(user, &domain)? {
                tot.skipped.push(format!("{domain} (disabled)"));
                continue;
            }
            let r = match self.collect_domain(user, &domain, Some(&settings)) {
                Ok(r) => r,
                Err(e) => {
                    tot.errors += 1;
                    self.app
                        .log
                        .error(format!("collect {user}/{domain} failed: {e:#}"));
                    continue;
                }
            };
            let Some(r) = r else {
                continue;
            };
            tot.domains += 1;
            tot.files += r.files;
            tot.lines += r.lines;
            tot.bad += r.bad;
            tot.ignored += r.ignored;
            tot.errors += r.errors;
        }
        Ok(tot)
    }

    /// True when discovery lists `domain` for `user` (cPanel: `/var/cpanel/users/<user>`
    /// or `/etc/userdomains`; standalone: a site of that owner); registry rows alone
    /// do not count.
    pub fn owns_domain(&self, user: &str, domain: &str) -> bool {
        self.app.provider().domains_of(user).contains_key(domain)
    }

    /// Live logs of a domain that may be read for `user`: per-account directories
    /// always, shared directories only when the domain belongs to the account.
    fn live_files(&self, user: &str, domain: &str) -> Vec<LogFile> {
        let files = self.app.provider().live_logs(user, domain);
        let shared: Vec<String> = files
            .iter()
            .filter(|f| f.shared)
            .map(|f| f.path.display().to_string())
            .collect();
        if !shared.is_empty() && !self.owns_domain(user, domain) {
            self.app.log.log_ctx(
                Level::Warn,
                &format!("not reading the shared logs of {domain} for {user}: cPanel does not list it for this account"),
                &json!({"files": shared}),
            );
            return files.into_iter().filter(|f| !f.shared).collect();
        }
        files
    }

    fn checkpoints(store: &DomainStore) -> Result<BTreeMap<String, Checkpoint>> {
        let mut out = BTreeMap::new();
        let mut st = store
            .conn()
            .prepare("SELECT path, inode, offset, size, first_line, last_ts FROM checkpoints")?;
        let rows = st.query_map([], |r| {
            let g = |i: usize| r.get::<_, SqlValue>(i);
            Ok((g(0)?, g(1)?, g(2)?, g(3)?, g(4)?, g(5)?))
        })?;
        for row in rows {
            let (p, ino, off, size, fl, lt) = row?;
            let Some(path) = sql_string(&p) else { continue };
            out.insert(
                path,
                Checkpoint {
                    inode: sql_i64(&ino).unwrap_or(0),
                    offset: sql_i64(&off).unwrap_or(0),
                    size: sql_i64(&size).unwrap_or(0),
                    first_line: sql_string(&fl),
                    last_ts: sql_i64(&lt),
                },
            );
        }
        Ok(out)
    }

    fn new_run<'r>(
        &'r mut self,
        store: &'r DomainStore,
        domain: &str,
        tz: &str,
        ds: &DomainSettings,
        persistent: bool,
        watermark: Option<i64>,
    ) -> Result<DomainRun<'r>> {
        let app = self.app;
        let reg = app.registry()?;
        let salt = Box::new(move |day: &str| reg.salt(day));
        DomainRun::new(
            store,
            domain,
            tz,
            ds,
            &app.config,
            &mut self.cls,
            app.geoip(),
            salt,
            persistent,
            watermark,
        )
    }

    /// Tails the live logs of one domain; `None` when the domain has neither logs
    /// nor a database.
    pub fn collect_domain(
        &mut self,
        user: &str,
        domain: &str,
        settings: Option<&UserSettings>,
    ) -> Result<Option<DomainStats>> {
        let app = self.app;
        let files = self.live_files(user, domain);
        let db_file = app.paths.try_domain_db(user, domain)?;
        if files.is_empty() && !db_file.is_file() {
            return Ok(None);
        }
        let owned;
        let settings = match settings {
            Some(s) => s,
            None => {
                owned = UserSettings::read(&app.paths, user).0;
                &owned
            }
        };
        let ds = settings.for_domain(domain);
        let store = DomainStore::open_write(&app.paths, user, domain)?;
        let tz = self.resolve_timezone(&store, user, &ds)?;
        let now = app.now();
        let mut stats = DomainStats::default();
        let cps = Collector::checkpoints(&store)?;
        if self.is_idle(&store, &files, &cps, &tz, now)? {
            stats.files = files.len() as i64;
            stats.idle = true;
            return Ok(Some(stats));
        }
        // newest line already processed: stored visits stale at that point can be closed while loading
        let mut watermark: Option<i64> = None;
        for cp in cps.values() {
            if let Some(lt) = cp.last_ts {
                watermark = Some(watermark.unwrap_or(0).max(lt - SWEEP_SLACK));
            }
        }
        let mut sources: Vec<LogSource> = Vec::new();
        for f in &files {
            let key = f.path.display().to_string();
            let mut src = LogSource::new(&f.path, f.https, cps.get(&key).cloned(), false);
            if let Err(e) = src.open() {
                stats.errors += 1;
                app.log
                    .error(format!("cannot read {}: {e}", f.path.display()));
                continue;
            }
            if let Some(reason) = src.reset_reason() {
                stats.resets.push(format!("{}:{reason}", basename(&f.path)));
                app.log.info(format!(
                    "log {} restarted from 0 ({reason})",
                    f.path.display()
                ));
            }
            if let Some((old, off)) = src.rotated_from() {
                stats.resets.push(format!("{}:rotated", basename(&f.path)));
                app.log.info(format!(
                    "log {} was rotated: finishing {} from offset {off}, then the new file from 0",
                    f.path.display(),
                    old.display()
                ));
            }
            sources.push(src);
        }
        stats.files = sources.len() as i64;
        let backfill_until = store
            .get_meta("backfill_until")
            .map(|s| crate::config::php_str_int(&s))
            .unwrap_or(0);
        let skip = if backfill_until > 0 && store.get_meta("live_since").is_none() {
            Skip::UpTo(backfill_until)
        } else {
            Skip::None
        };
        let live_since_missing = store.get_meta("live_since").is_none();

        let mut parser = std::mem::take(&mut self.parser);
        let mut counts = RunCounts::default();
        let (evicted, max_open) = {
            let mut run = self.new_run(&store, domain, &tz, &ds, true, watermark)?;
            let mut flush =
                |run: &mut DomainRun<'_>, sources: &[LogSource], st: &[SrcStat]| -> Result<()> {
                    run.flush(&checkpoint_rows(sources, st, now), &[], Vec::new())
                };
            let state = run_merged(
                app,
                &mut parser,
                &mut run,
                &mut sources,
                &mut counts,
                &skip,
                &mut flush,
            )?;
            run.close_stale(now);
            let mut meta: Vec<(&str, Option<String>)> = vec![
                ("last_collect_at", Some(now.to_string())),
                ("timezone", Some(tz.clone())),
            ];
            if live_since_missing && let Some(m) = run.min_ts() {
                meta.push(("live_since", Some(m.to_string())));
            }
            run.flush(&checkpoint_rows(&sources, &state, now), &[], meta)?;
            (run.sessions.evicted_count(), run.sessions.max_open())
        };
        self.parser = parser;
        for s in &mut sources {
            s.close();
        }
        stats.lines = counts.lines;
        stats.bad = counts.bad;
        stats.ignored = counts.ignored;
        stats.errors += counts.errors;
        if let Ok(reg) = app.registry()
            && let Err(e) = reg.refresh_summary(&store, &tz, Some(now))
        {
            app.log
                .warn(format!("summary refresh failed for {user}/{domain}: {e}"));
        }
        if evicted > 0 {
            stats.visits_closed_early = evicted as i64;
            app.log.warn(format!("{user}/{domain}: more than {max_open} visits open at once (max_open_visits); {evicted} closed early"));
        }
        if stats.lines > 0 || stats.errors > 0 {
            app.log.log_ctx(
                Level::Info,
                &format!("collected {user}/{domain}"),
                &json!({"files": stats.files, "lines": stats.lines, "bad": stats.bad, "ignored": stats.ignored}),
            );
        }
        Ok(Some(stats))
    }

    /// Closes the visits of a domain whose last hit is older than the session
    /// timeout (rollup); returns the number closed.
    pub fn close_stale_visits(&mut self, store: &DomainStore, tz: &str) -> Result<usize> {
        let now = self.app.now();
        let domain = store.domain().to_string();
        let mut run = self.new_run(
            store,
            &domain,
            tz,
            &DomainSettings::default(),
            true,
            Some(now),
        )?;
        run.close_stale(now);
        let n = run.sessions.closed_count();
        if n > 0 {
            run.flush(&[], &[], Vec::new())?;
        }
        Ok(n)
    }

    /// Imports the compressed archives of a user (never twice; lines already covered
    /// by live collection are skipped).
    pub fn backfill_user(
        &mut self,
        user: &str,
        only_domain: Option<&str>,
        archive_dir: Option<&Path>,
    ) -> Result<BackfillTotals> {
        assert_user(user)?;
        let reg = self.app.registry()?;
        let mut tot = BackfillTotals::default();
        let mut domains: Vec<String> = Vec::new();
        for r in reg.domains(Some(user))? {
            if !domains.contains(&r.domain) {
                domains.push(r.domain);
            }
        }
        for d in self.app.provider().domains_of(user).into_keys() {
            if !domains.contains(&d) {
                domains.push(d);
            }
        }
        let archives = self.app.provider().archives(user, &domains, archive_dir);
        let (settings, _) = UserSettings::read(&self.app.paths, user);
        for (domain, list) in archives {
            if only_domain.map(|o| o != domain).unwrap_or(false) {
                continue;
            }
            if !reg.is_enabled(user, &domain)? {
                tot.skipped.push(format!("{domain} (disabled)"));
                continue;
            }
            let trusted = archive_dir.is_some() || self.app.provider().archives_trusted();
            let r = match self.backfill_domain(user, &domain, &list, &settings, trusted) {
                Ok(r) => r,
                Err(e) => {
                    tot.errors += 1;
                    self.app
                        .log
                        .error(format!("backfill {user}/{domain} failed: {e:#}"));
                    continue;
                }
            };
            if let Some(why) = r.1 {
                tot.skipped.push(format!("{domain} ({why})"));
                continue;
            }
            let r = r.0;
            tot.domains += 1;
            tot.files += r.files;
            tot.lines += r.lines;
            tot.imported += r.imported;
            tot.bad += r.bad;
            tot.ignored += r.ignored;
            tot.skipped_files += r.skipped_files;
            tot.errors += r.errors;
        }
        Ok(tot)
    }

    /// Imports the archives of one domain; the second element is the reason when
    /// nothing was read (`trusted_dir`: the archives come from `--archive-dir`).
    pub fn backfill_domain(
        &mut self,
        user: &str,
        domain: &str,
        archives: &[Archive],
        settings: &UserSettings,
        trusted_dir: bool,
    ) -> Result<(BackfillTotals, Option<String>)> {
        let app = self.app;
        let mut stats = BackfillTotals::default();
        if !trusted_dir && app.paths.archive_dir_shared() && !self.owns_domain(user, domain) {
            app.log.warn(format!("not importing the archives of {domain} for {user}: cPanel does not list it for this account"));
            return Ok((stats, Some(format!("not a domain of {user}"))));
        }
        let ds = settings.for_domain(domain);
        let store = DomainStore::open_write(&app.paths, user, domain)?;
        let tz = self.resolve_timezone(&store, user, &ds)?;
        let now = app.now();
        let mut done: BTreeMap<String, (i64, i64, Option<i64>)> = BTreeMap::new();
        {
            let mut st = store
                .conn()
                .prepare("SELECT path, size, mtime, last_ts FROM imported_archives")?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, SqlValue>(0)?,
                    r.get::<_, SqlValue>(1)?,
                    r.get::<_, SqlValue>(2)?,
                    r.get::<_, SqlValue>(3)?,
                ))
            })?;
            for row in rows {
                let (p, s, m, l) = row?;
                if let Some(p) = sql_string(&p) {
                    done.insert(
                        p,
                        (
                            sql_i64(&s).unwrap_or(0),
                            sql_i64(&m).unwrap_or(0),
                            sql_i64(&l),
                        ),
                    );
                }
            }
        }
        let mut sources: Vec<LogSource> = Vec::new();
        let mut resume: Vec<i64> = Vec::new();
        // standalone archives come from root-managed configuration: no per-account owner
        let account_uid = if app.provider().archives_trusted() {
            None
        } else {
            crate::util::fs::user_id(user)
        };
        for a in archives {
            let key = a.path.display().to_string();
            let prev = done.get(&key);
            if let Some(p) = prev
                && p.0 == a.size
                && p.1 == a.mtime
            {
                stats.skipped_files += 1;
                continue;
            }
            let mut src = LogSource::archive(&a.path, a.https).with_account_uid(account_uid);
            if let Err(e) = src.open() {
                if let Some(u) = e.downcast_ref::<crate::engine::logsource::UntrustedArchive>() {
                    app.log.warn(format!("skipping archive: {u}"));
                    continue;
                }
                stats.errors += 1;
                app.log
                    .error(format!("cannot read archive {}: {e}", a.path.display()));
                continue;
            }
            let mut from = prev.map(|p| p.2.unwrap_or(0)).unwrap_or(0);
            if let Some(stream) = &a.stream {
                // logrotate renames (access_log.1 -> access_log.2.gz, -20261017 ->
                // -20261017.gz): what any file of the stream imported is not imported again
                from = from.max(stream_watermark(&done, stream));
            }
            resume.push(from);
            sources.push(src);
        }
        stats.files = sources.len() as i64;
        if sources.is_empty() {
            return Ok((stats, None));
        }
        let live_since = store.get_meta("live_since");
        let limit = live_since
            .as_deref()
            .map(crate::config::php_str_int)
            .unwrap_or(i64::MAX);
        let skip = Skip::Backfill {
            limit,
            resume: &resume,
        };
        let mut parser = std::mem::take(&mut self.parser);
        let mut counts = RunCounts::default();
        let imported = {
            let mut run = self.new_run(&store, domain, &tz, &ds, false, None)?;
            let mut flush =
                |run: &mut DomainRun<'_>, sources: &[LogSource], st: &[SrcStat]| -> Result<()> {
                    run.flush(&[], &archive_rows(sources, st, false, now), Vec::new())
                };
            let state = run_merged(
                app,
                &mut parser,
                &mut run,
                &mut sources,
                &mut counts,
                &skip,
                &mut flush,
            )?;
            run.close_all();
            let mut meta: Vec<(&str, Option<String>)> = Vec::new();
            if live_since.is_none() && run.min_ts().is_some() {
                let prev_until = store
                    .get_meta("backfill_until")
                    .map(|s| crate::config::php_str_int(&s))
                    .unwrap_or(0);
                let max_ts = state.iter().filter_map(|s| s.max_ts).fold(0, i64::max);
                meta.push(("backfill_until", Some(prev_until.max(max_ts).to_string())));
            }
            meta.push(("timezone", Some(tz.clone())));
            run.flush(&[], &archive_rows(&sources, &state, true, now), meta)?;
            run.lines()
        };
        self.parser = parser;
        for s in &mut sources {
            s.close();
        }
        stats.lines = counts.lines;
        stats.bad = counts.bad;
        stats.ignored = counts.ignored;
        stats.errors += counts.errors;
        stats.imported = imported;
        if let Ok(reg) = app.registry()
            && let Err(e) = reg.refresh_summary(&store, &tz, Some(now))
        {
            app.log
                .warn(format!("summary refresh failed for {user}/{domain}: {e}"));
        }
        app.log.log_ctx(
            Level::Info,
            &format!("backfilled {user}/{domain}"),
            &json!({"files": stats.files, "lines": stats.lines, "imported": stats.imported}),
        );
        Ok((stats, None))
    }

    /// True when nothing would change: every log at its checkpoint, no open visit
    /// due to close, the timezone unchanged and the database touched less than an
    /// hour ago. Idle domains are not written at all.
    fn is_idle(
        &self,
        store: &DomainStore,
        files: &[LogFile],
        cps: &BTreeMap<String, Checkpoint>,
        tz: &str,
        now: i64,
    ) -> Result<bool> {
        if files.is_empty() && cps.is_empty() {
            return Ok(false);
        }
        for f in files {
            let Some(cp) = cps.get(&f.path.display().to_string()) else {
                return Ok(false);
            };
            use std::os::unix::fs::MetadataExt;
            let Ok(st) = std::fs::metadata(&f.path) else {
                return Ok(false);
            };
            if cp.inode != st.ino() as i64 || cp.offset != st.size() as i64 {
                return Ok(false);
            }
        }
        let meta = store.meta();
        let last = meta
            .get("last_collect_at")
            .cloned()
            .flatten()
            .map(|s| crate::config::php_str_int(&s))
            .unwrap_or(0);
        if meta.get("timezone").cloned().flatten().as_deref() != Some(tz) || now - last >= 3600 {
            return Ok(false);
        }
        let timeout = (self.app.config.int("session_timeout_min", 30) * 60).max(60);
        let stale: i64 = store.conn().query_row(
            "SELECT COUNT(*) FROM open_visits WHERE last_ts <= ?",
            [now - timeout],
            |r| r.get(0),
        )?;
        if stale != 0 {
            return Ok(false);
        }
        // more stored visits than the cap: load to close the excess
        let cap = self
            .app
            .config
            .int("max_open_visits", super::session::DEFAULT_MAX_OPEN as i64);
        let total: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM open_visits", [], |r| r.get(0))?;
        Ok(cap <= 0 || total <= cap)
    }

    /// Domain timezone: user settings > registry account > config > server. Records
    /// a change in `meta.tz_changed_at`.
    pub fn resolve_timezone(
        &self,
        store: &DomainStore,
        user: &str,
        ds: &DomainSettings,
    ) -> Result<String> {
        let mut tz = ds.timezone.clone();
        if tz.is_none() {
            tz = self.app.registry()?.account(user)?.and_then(|a| a.timezone);
        }
        let tz = tz.unwrap_or_else(|| self.app.server_timezone());
        if let Some(old) = store.get_meta("timezone")
            && old != tz
        {
            store.set_meta_many(&[
                ("timezone", Some(tz.clone())),
                ("tz_changed_at", Some(self.app.now().to_string())),
            ])?;
            self.app.log.info(format!(
                "timezone of {} changed from {old} to {tz}",
                store.domain()
            ));
        }
        Ok(tz)
    }
}

/// Newest line imported from any archive of a live log's rotation stream
/// (`<log>-…`, `<log>.N`, `.gz` or not).
fn stream_watermark(done: &BTreeMap<String, (i64, i64, Option<i64>)>, stream: &Path) -> i64 {
    let s = stream.display().to_string();
    let Some(name) = stream.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return 0;
    };
    let dir = stream
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    done.iter()
        .filter(|(p, _)| {
            p.as_str() != s
                && p.strip_prefix(&format!("{dir}/{name}"))
                    .is_some_and(|rest| super::standalone::is_rotation_suffix(rest, true))
        })
        .filter_map(|(_, v)| v.2)
        .max()
        .unwrap_or(0)
}

fn checkpoint_rows(sources: &[LogSource], st: &[SrcStat], now: i64) -> Vec<CheckpointRow> {
    sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let x = st.get(i).cloned().unwrap_or_default();
            let off = x.offset as i64;
            let inode = if x.inode == 0 { s.inode() } else { x.inode };
            let (size, first_line) = s.checkpoint_of(inode);
            CheckpointRow {
                path: s.path().display().to_string(),
                inode: inode as i64,
                offset: off,
                size: (size as i64).max(off),
                first_line,
                last_ts: x.last_ts,
                lines: x.lines,
                bad_lines: x.bad,
                ignored_lines: x.ignored,
                updated_at: now,
            }
        })
        .collect()
}

fn archive_rows(
    sources: &[LogSource],
    st: &[SrcStat],
    complete: bool,
    now: i64,
) -> Vec<ArchiveRow> {
    sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let x = st.get(i).cloned().unwrap_or_default();
            ArchiveRow {
                path: s.path().display().to_string(),
                size: if complete { s.size() as i64 } else { -1 },
                mtime: s.mtime(),
                lines: x.lines,
                imported_at: now,
                last_ts: x.last_ts,
            }
        })
        .collect()
}

type FlushFn<'f> = dyn FnMut(&mut DomainRun<'_>, &[LogSource], &[SrcStat]) -> Result<()> + 'f;

/// Merges the sources by timestamp and feeds the run; returns the per-source
/// statistics since the last flush.
fn run_merged(
    app: &App,
    parser: &mut LogParser,
    run: &mut DomainRun<'_>,
    sources: &mut [LogSource],
    counts: &mut RunCounts,
    skip: &Skip<'_>,
    flush: &mut FlushFn<'_>,
) -> Result<Vec<SrcStat>> {
    let n = sources.len();
    // lines dated more than 15 years ago or more than 2 days ahead are bad lines
    parser.set_window(app.now());
    parser.take_out_of_window();
    let mut heads: Vec<Head> = (0..n).map(|_| Head::default()).collect();
    let mut src: Vec<SrcStat> = sources
        .iter()
        .map(|s| SrcStat {
            offset: s.offset(),
            inode: s.current_inode(),
            ..SrcStat::default()
        })
        .collect();

    let fill = |i: usize,
                heads: &mut [Head],
                src: &mut [SrcStat],
                sources: &mut [LogSource],
                counts: &mut RunCounts,
                run: &mut DomainRun<'_>,
                parser: &mut LogParser| {
        let head = &mut heads[i];
        loop {
            match sources[i].next_line(&mut head.line) {
                Ok(true) => {
                    src[i].lines += 1;
                    counts.lines += 1;
                    run.pending_lines += 1;
                    let parsed = if head.line.is_empty() {
                        None
                    } else {
                        parser.parse(&head.line, &mut head.scratch)
                    };
                    match parsed {
                        None => {
                            src[i].bad += 1;
                            counts.bad += 1;
                        }
                        Some(h) => {
                            head.hit = h;
                            head.offset = sources[i].offset();
                            head.inode = sources[i].current_inode();
                            head.valid = true;
                            return;
                        }
                    }
                }
                Ok(false) => break,
                Err(e) => {
                    counts.errors += 1;
                    app.log.error(format!(
                        "read error on {}: {e}",
                        sources[i].path().display()
                    ));
                    break;
                }
            }
        }
        head.valid = false;
        src[i].offset = sources[i].offset();
        src[i].inode = sources[i].current_inode();
    };
    for i in 0..n {
        fill(i, &mut heads, &mut src, sources, counts, run, parser);
    }
    let mut count: u64 = 0;
    let mut watermark: i64 = 0;
    loop {
        let mut best: Option<usize> = None;
        let mut best_ts = i64::MAX;
        for (i, h) in heads.iter().enumerate() {
            if h.valid && h.hit.ts < best_ts {
                best = Some(i);
                best_ts = h.hit.ts;
            }
        }
        let Some(b) = best else {
            break;
        };
        let ts = heads[b].hit.ts;
        if !skip.skip(ts, b) {
            let head = &heads[b];
            let hit = Hit::new(&head.line, &head.scratch, &head.hit);
            let c = run.process(&hit, sources[b].https())?;
            if c == class::IGNORED {
                src[b].ignored += 1;
                counts.ignored += 1;
            }
        }
        src[b].offset = heads[b].offset;
        src[b].inode = heads[b].inode;
        src[b].last_ts = Some(ts);
        if ts > src[b].max_ts.unwrap_or(0) {
            src[b].max_ts = Some(ts);
        }
        if ts > watermark {
            watermark = ts;
        }
        fill(b, &mut heads, &mut src, sources, counts, run, parser);
        count += 1;
        if count.is_multiple_of(SWEEP_LINES) {
            run.close_stale(watermark - SWEEP_SLACK);
            let day = run.clock().day(watermark);
            run.sessions.forget_days_before(&time::prev_day(&day));
        }
        if count.is_multiple_of(FLUSH_LINES) || run.counters.raw_rows() > 200_000 {
            flush(run, sources, &src)?;
            for s in src.iter_mut() {
                s.lines = 0;
                s.bad = 0;
                s.ignored = 0;
            }
        }
    }
    let out_of_window = parser.take_out_of_window();
    if out_of_window > 0 {
        let files: Vec<String> = sources
            .iter()
            .map(|s| s.path().display().to_string())
            .collect();
        app.log.warn(format!(
            "{out_of_window} line(s) with a timestamp outside the accepted window (15 years ago to 2 days ahead) counted as bad: {}",
            files.join(", ")
        ));
    }
    Ok(src)
}
