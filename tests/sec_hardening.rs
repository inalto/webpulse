//! Regression tests of the security-review hardening: test hooks ignored on root
//! code paths, archive opening relative to a directory descriptor, private registry
//! files from the first byte, input clamps of the log parser and the counters,
//! directories that are only re-moded when created here, and the release workflows.

mod common;

use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use common::{FakeRoot, L, line, repo, ts};
use serde_json::json;
use webpulse::engine::collector::Collector;
use webpulse::engine::counters::Counters;
use webpulse::engine::discovery::LogFiles;
use webpulse::engine::logsource::{LogSource, UntrustedArchive, open_archive};
use webpulse::engine::parser::{LogParser, MAX_BYTES};
use webpulse::engine::user_settings::UserSettings;
use webpulse::util::fs;

fn tmp(name: &str) -> PathBuf {
    let d = repo().join("tests/tmp/sec-hardening").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn gz(s: &str) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(s.as_bytes()).unwrap();
    e.finish().unwrap()
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
}

fn my_uid() -> u32 {
    fs::euid()
}

// ---------------------------------------------------------------- finding 2: test hooks

#[test]
fn child_exe_override_is_ignored_as_root() {
    use webpulse::cli::collect::child_exe_from;
    let v = || Some(std::ffi::OsString::from("/tmp/evil"));
    assert_eq!(
        child_exe_from(true, v()),
        None,
        "root ignores WEBPULSE_CHILD_EXE"
    );
    assert_eq!(
        child_exe_from(false, v()),
        Some(PathBuf::from("/tmp/evil")),
        "unprivileged runs (tests) honour it"
    );
    assert_eq!(
        child_exe_from(false, Some("".into())),
        None,
        "empty = unset"
    );
}

#[test]
fn fake_clock_is_ignored_for_root_on_the_production_config() {
    use webpulse::util::time::fake_clock_policy;
    assert!(
        !fake_clock_policy(true, true),
        "root + /etc/webpulse/config.json"
    );
    assert!(
        fake_clock_policy(true, false),
        "root + another config (warned)"
    );
    assert!(fake_clock_policy(false, true), "non-root");
    assert!(fake_clock_policy(false, false), "non-root");
}

#[test]
fn production_config_is_recognised_through_symlinks_and_dot_dot() {
    use webpulse::app::is_production_config;
    let d = tmp("prodcfg");
    let link = d.join("config.json");
    std::os::unix::fs::symlink("/etc/webpulse/config.json", &link).unwrap();
    let chain = d.join("chain.json");
    std::os::unix::fs::symlink(&link, &chain).unwrap();
    assert!(is_production_config(Path::new("/etc/webpulse/config.json")));
    assert!(is_production_config(Path::new(
        "/etc/webpulse/../webpulse/./config.json"
    )));
    assert!(
        is_production_config(&link),
        "symlink to the production file"
    );
    assert!(is_production_config(&chain), "symlink chain");
    let other = d.join("other.json");
    std::fs::write(&other, "{}").unwrap();
    assert!(!is_production_config(&other));
}

#[test]
fn cgi_testing_needs_non_root_and_a_non_production_config() {
    use webpulse::cli::cgi::testing_allowed;
    let d = tmp("cgitesting");
    let cfg = d.join("config.json");
    std::fs::write(&cfg, "{}").unwrap();
    let link = d.join("prod.json");
    std::os::unix::fs::symlink("/etc/webpulse/config.json", &link).unwrap();
    assert!(testing_allowed(Some("1"), false, Some(&cfg)));
    assert!(
        !testing_allowed(Some("1"), true, Some(&cfg)),
        "never as root"
    );
    assert!(!testing_allowed(Some("0"), false, Some(&cfg)));
    assert!(!testing_allowed(None, false, Some(&cfg)));
    assert!(
        !testing_allowed(Some("1"), false, Some(&link)),
        "a symlink to the production config is the production config"
    );
    assert!(!testing_allowed(Some("1"), false, None));
}

// ---------------------------------------------------------------- finding 3: archives

#[test]
fn archive_in_a_symlinked_directory_is_refused_at_open_time() {
    let d = tmp("archdir");
    let real = d.join("logs.real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("example.com-Oct-2026.gz"), gz("x\n")).unwrap();
    std::os::unix::fs::symlink(&real, d.join("logs")).unwrap();
    let mut s = LogSource::new(&d.join("logs/example.com-Oct-2026.gz"), false, None, true);
    let e = s.open().unwrap_err();
    assert!(e.downcast_ref::<UntrustedArchive>().is_some(), "{e}");
    // the same file through the real directory opens
    let mut s = LogSource::new(&real.join("example.com-Oct-2026.gz"), false, None, true);
    s.open().unwrap();
}

#[test]
fn archive_ownership_is_rechecked_on_the_opened_descriptor() {
    let d = tmp("archown");
    let f = d.join("example.com-Oct-2026.gz");
    std::fs::write(&f, gz("x\n")).unwrap();
    let me = my_uid();
    let other = me.wrapping_add(4242);
    // root semantics simulated with `root = true`
    open_archive(&f, Some(me), true).expect("own archive in own directory");
    let e = open_archive(&f, Some(other), true).unwrap_err();
    let u = e.downcast_ref::<UntrustedArchive>().expect("untrusted");
    assert!(u.0.contains("belongs to uid"), "{u}");
    // unknown account uid: any owner (as the listing does)
    open_archive(&f, None, true).unwrap();
    // unprivileged: no ownership checks
    open_archive(&f, Some(other), false).unwrap();
    // a FIFO or a symlink is never opened
    std::os::unix::fs::symlink(&f, d.join("link.gz")).unwrap();
    let e = open_archive(&d.join("link.gz"), Some(me), true).unwrap_err();
    assert!(e.downcast_ref::<UntrustedArchive>().is_some(), "{e}");
}

#[test]
fn backfill_skips_an_archive_directory_swapped_for_a_symlink_after_listing() {
    let fr = FakeRoot::create("rs-sec-archive-swap", json!({}));
    let logs = fr.path("home/alice/logs");
    std::fs::write(
        logs.join("example.com-Oct-2026.gz"),
        gz(&(line("203.0.113.7", "2026-10-01 10:00:00", "/") + "\n")),
    )
    .unwrap();
    let app = fr.app();
    let mut c = Collector::new(&app);
    c.sync_registry(None).unwrap();
    let lf = LogFiles::new(&app.paths);
    let listed = lf.archives("alice", &["example.com".to_string()], None);
    let list = listed.get("example.com").expect("archive listed").clone();
    // the account swaps ~/logs for a symlink to somewhere else between listing and open
    let elsewhere = fr.path("home/alice/elsewhere");
    std::fs::rename(&logs, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &logs).unwrap();
    let (settings, _) = UserSettings::read(&app.paths, "alice");
    let (r, why) = c
        .backfill_domain("alice", "example.com", &list, &settings, false)
        .unwrap();
    assert_eq!(why, None);
    assert_eq!(r.files, 0, "nothing opened: {r:?}");
    assert_eq!(r.errors, 0, "skipped with a warning, not an error");
    assert!(
        fr.log_text().contains("skipping archive"),
        "{}",
        fr.log_text()
    );
}

// ---------------------------------------------------------------- finding 4: registry files

#[test]
fn precreate_private_creates_0600_and_never_follows_or_touches_existing_files() {
    let d = tmp("precreate");
    let f = d.join("reg.db");
    assert!(fs::precreate_private(&f).unwrap());
    assert_eq!(mode(&f), 0o600);
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(
        !fs::precreate_private(&f).unwrap(),
        "existing file left alone"
    );
    assert_eq!(mode(&f), 0o640);
    let target = d.join("target");
    std::os::unix::fs::symlink(&target, d.join("link.db")).unwrap();
    assert!(!fs::precreate_private(&d.join("link.db")).unwrap());
    assert!(!target.exists(), "a dangling symlink is not followed");
}

#[test]
fn both_registries_are_created_0600_in_a_0711_directory() {
    let d = tmp("registry");
    let fr = FakeRoot::create("rs-sec-registry", json!({}));
    let app = fr.app();
    let p = d.join("engine/sub/webpulse.db");
    webpulse::engine::registry::Registry::open(&p, &app.config, Some("t"), None).unwrap();
    assert_eq!(mode(&p), 0o600, "engine registry");
    assert_eq!(
        mode(p.parent().unwrap()),
        0o711,
        "engine registry directory"
    );
    let p = d.join("http/sub/webpulse.db");
    webpulse::http::registry::Registry::open(&p, true, "t", 1_791_000_000).unwrap();
    assert_eq!(mode(&p), 0o600, "web registry");
    assert_eq!(mode(p.parent().unwrap()), 0o711, "web registry directory");
}

// ---------------------------------------------------------------- finding 5: clamps

fn parse(p: &mut LogParser, l: &str) -> Option<(i64, i64, i64)> {
    let mut s = Vec::new();
    p.parse(l.as_bytes(), &mut s)
        .map(|h| (h.ts, h.status, h.bytes))
}

#[test]
fn parser_rejects_absurd_sizes_statuses_and_dates() {
    let mut p = LogParser::new();
    let at = "2026-10-10 10:00:00";
    let ok = |b: i64| L::new("203.0.113.1", at, "/").bytes(b).s();
    assert_eq!(parse(&mut p, &ok(MAX_BYTES)).unwrap().2, MAX_BYTES);
    assert!(parse(&mut p, &ok(MAX_BYTES + 1)).is_none(), "> 1 TiB");
    let huge = ok(1).replace(" 200 1 ", " 200 99999999999999999999999 ");
    assert!(parse(&mut p, &huge).is_none(), "saturating i64 size");
    for (st, accepted) in [
        (100, true),
        (999, true),
        (200, true),
        (99, false),
        (0, false),
    ] {
        let l = ok(1).replace(" 200 1 ", &format!(" {st:03} 1 "));
        assert_eq!(parse(&mut p, &l).is_some(), accepted, "status {st:03}");
    }
    let dash = ok(1).replace(" 200 1 ", " - 1 ");
    assert_eq!(parse(&mut p, &dash).unwrap().1, 0, "'-' is still status 0");

    // the time window only applies once set (by the collector)
    let now = ts("2026-10-10 12:00:00");
    let old = L::new("203.0.113.1", "2010-01-01 00:00:00", "/").s();
    let future = L::new("203.0.113.1", "2026-10-13 12:00:00", "/").s();
    let soon = L::new("203.0.113.1", "2026-10-11 12:00:00", "/").s();
    assert!(parse(&mut p, &old).is_some());
    p.set_window(now);
    assert!(parse(&mut p, &old).is_none(), "more than 15 years ago");
    assert!(parse(&mut p, &future).is_none(), "more than 2 days ahead");
    assert!(parse(&mut p, &soon).is_some(), "one day ahead (clock skew)");
    assert!(parse(&mut p, &ok(1)).is_some());
    assert_eq!(p.take_out_of_window(), 2);
    assert_eq!(p.take_out_of_window(), 0);
}

#[test]
fn collect_counts_out_of_window_lines_as_bad_and_logs_them() {
    let mut fr = FakeRoot::create("rs-sec-window", json!({}));
    fr.write_log(
        "etc/apache2/logs/domlogs/alice/example.com",
        &[
            line("203.0.113.1", "2009-10-10 10:00:00", "/ancient"),
            line("203.0.113.1", "2026-10-10 10:00:00", "/"),
            line("203.0.113.1", "2030-10-10 10:00:00", "/future"),
        ],
        true,
    );
    fr.set_now(ts("2026-10-10 12:00:00"));
    let r = fr.run("collect --json").ok("collect");
    assert_eq!(r.i("bad"), 2, "{:?}", r);
    assert_eq!(
        fr.scalar("alice", "example.com", "SELECT SUM(hits) FROM daily"),
        1
    );
    assert!(
        fr.log_text()
            .contains("2 line(s) with a timestamp outside the accepted window"),
        "{}",
        fr.log_text()
    );
}

#[test]
fn counters_saturate_instead_of_wrapping() {
    use webpulse::engine::counters::{d, h};
    let fr = FakeRoot::create("rs-sec-counters", json!({}));
    let app = fr.app();
    let store =
        webpulse::engine::store::DomainStore::open_write(&app.paths, "alice", "example.com")
            .unwrap();
    let mut c = Counters::new();
    let day = "2026-10-10";
    c.day(day, d::BYTES, i64::MAX);
    c.day(day, d::BYTES, i64::MAX);
    c.hour(day, 3, h::BYTES, i64::MAX);
    c.hour(day, 3, h::BYTES, 5);
    c.dim(day, 0, "k", 0, i64::MAX);
    c.dim(day, 0, "k", 0, 1);
    c.dim_hit(day, 0, "h", i64::MAX);
    c.dim_hit(day, 0, "h", i64::MAX);
    c.flush(store.conn()).unwrap();
    let q = |sql: &str| -> i64 { store.conn().query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(q("SELECT bytes FROM daily"), i64::MAX);
    assert_eq!(q("SELECT bytes FROM hourly WHERE hour = 3"), i64::MAX);
    assert_eq!(q("SELECT visits FROM dim_daily WHERE key = 'k'"), i64::MAX);
    assert_eq!(q("SELECT bytes FROM dim_daily WHERE key = 'h'"), i64::MAX);
    assert_eq!(q("SELECT hits FROM dim_daily WHERE key = 'h'"), 2);
}

#[test]
fn an_overflowing_sum_is_treated_as_zero_instead_of_failing() {
    use webpulse::reports::domain_data::query_rows;
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE TABLE daily (day TEXT, bytes INTEGER);
         INSERT INTO daily VALUES ('a', 9223372036854775000), ('b', 9223372036854775000);",
    )
    .unwrap();
    let rows =
        query_rows(&c, "SELECT COALESCE(SUM(bytes),0) AS b FROM daily", &[]).expect("no error");
    assert!(rows.is_empty(), "overflow = no rows (zeros)");
    let rows = query_rows(&c, "SELECT COUNT(*) AS n FROM daily", &[]).unwrap();
    assert_eq!(rows[0].i("n"), 2, "normal queries unaffected");
}

// ---------------------------------------------------------------- finding 6: directories

#[test]
fn ensure_dir_only_modes_directories_it_creates_and_refuses_system_directories() {
    let d = tmp("ensure");
    let existing = d.join("existing");
    std::fs::create_dir(&existing).unwrap();
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!fs::ensure_dir(&existing, 0o700, Some("root"), Some("root")).unwrap());
    assert_eq!(mode(&existing), 0o755, "pre-existing directory untouched");
    let new = d.join("new/sub");
    assert!(fs::ensure_dir(&new, 0o711, None, None).unwrap());
    assert_eq!(mode(&new), 0o711, "created with the exact mode");
    for p in [
        "/",
        "/etc",
        "/etc/",
        "/var",
        "/var/log",
        "/usr",
        "/home",
        "/root",
        "/tmp",
        "/tmp/",
        "/usr/../etc",
        "//var//log/",
    ] {
        let e = fs::ensure_dir(Path::new(p), 0o700, None, None).unwrap_err();
        assert!(e.to_string().contains("system directory"), "{p}: {e}");
    }
    // a symlink resolving to a system directory is refused too
    let link = d.join("to-tmp");
    std::os::unix::fs::symlink("/tmp", &link).unwrap();
    assert!(fs::is_protected_dir(&link));
    assert!(!fs::is_protected_dir(Path::new("/tmp/webpulse")));
    assert!(!fs::is_protected_dir(Path::new("/var/webpulse")));
    assert_eq!(std::fs::metadata("/tmp").unwrap().mode() & 0o7777, 0o1777);
}

#[test]
fn setup_refuses_a_configured_system_directory_and_leaves_existing_ones_alone() {
    let fr = FakeRoot::create("rs-sec-setup", json!({}));
    let data = fr.path("var/webpulse/data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o750)).unwrap();
    fr.run("setup --no-sync").ok("setup");
    assert_eq!(mode(&data), 0o750, "existing data dir keeps its mode");
    assert_eq!(mode(&fr.path("var/webpulse/run")), 0o711, "created run dir");
    fr.patch_config(json!({"paths": {"log_dir": "/var/log"}}));
    let r = fr.run("setup --no-sync");
    assert_ne!(r.code, 0, "setup with log_dir=/var/log must fail");
    assert!(
        r.stdout.contains("system directory") || r.stderr.contains("system directory"),
        "{} {}",
        r.stdout,
        r.stderr
    );
}

// ---------------------------------------------------------------- finding 1: workflows

#[test]
fn workflows_pin_actions_drop_credentials_and_scope_the_write_token() {
    let read = |p: &str| std::fs::read_to_string(repo().join(p)).unwrap();
    let re = regex::Regex::new(r"^\s*-?\s*uses:\s*([^@\s]+)@(\S+)(.*)$").unwrap();
    for wf in [".github/workflows/ci.yml", ".github/workflows/release.yml"] {
        let s = read(wf);
        let mut checkouts = 0;
        for l in s.lines() {
            if let Some(c) = re.captures(l) {
                let (action, rev, rest) = (&c[1], &c[2], &c[3]);
                assert!(
                    rev.len() == 40 && rev.chars().all(|ch| ch.is_ascii_hexdigit()),
                    "{wf}: {action} must be pinned to a commit SHA, not {rev}"
                );
                assert!(
                    rest.trim_start().starts_with('#'),
                    "{wf}: {action} has a # tag comment"
                );
                if action == "actions/checkout" {
                    checkouts += 1;
                }
            }
        }
        assert!(checkouts > 0);
        assert_eq!(
            s.lines()
                .filter(|l| l.trim() == "persist-credentials: false")
                .count(),
            checkouts,
            "{wf}: every checkout drops the token"
        );
    }
    let rel = read(".github/workflows/release.yml");
    assert!(!rel.contains("rust-cache"), "no build cache in releases");
    let top = rel.split("\njobs:").next().unwrap();
    assert!(
        top.contains("permissions:\n  contents: read"),
        "read-only by default"
    );
    assert_eq!(
        rel.lines()
            .filter(|l| l.trim() == "contents: write")
            .count(),
        1,
        "one write job"
    );
    let publish = rel.split("\n  publish:").nth(1).expect("publish job");
    for needle in [
        "contents: write",
        "id-token: write",
        "attestations: write",
        "actions/attest-build-provenance@",
        "subject-path: dist/*.tar.gz",
    ] {
        assert!(publish.contains(needle), "publish job: {needle}");
    }
    assert!(
        !publish.contains("actions/checkout"),
        "publish runs no repository code"
    );
    let install = read("docs/INSTALL.md");
    assert!(
        install.contains("gh attestation verify"),
        "INSTALL.md documents provenance"
    );
}
