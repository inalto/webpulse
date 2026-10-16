//! Ports of tests/cases/sec_files.php, sec_openvisits.php and the engine half of
//! sec_tenant.php: FIFOs swapped into account-writable places, gzip bombs, removal
//! of data directories without following symlinks, log injection, the open-visit
//! cap, crash isolation of the global collect, and tenant ownership of shared logs.

mod common;

use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;

use common::{FakeRoot, Q, line, ts};
use serde_json::json;
use webpulse::engine::collector::Collector;
use webpulse::engine::logsource::LogSource;
use webpulse::engine::parser::LogParser;
use webpulse::engine::user_settings::UserSettings;
use webpulse::util::fs;

fn mkfifo(p: &std::path::Path) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let st = std::process::Command::new("mkfifo")
        .arg(p)
        .status()
        .unwrap();
    assert!(st.success());
}

/// Runs `f` in a thread and fails when it does not finish within `secs`.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(secs))
        .expect("must not block")
}

#[test]
fn m1_a_fifo_swapped_in_after_lstat_is_refused_without_blocking() {
    let fr = FakeRoot::create("rs-sec-fifo", json!({}));
    let dir = fr.path("var/webpulse/data/alice/user");
    std::fs::create_dir_all(&dir).unwrap();
    let settings = dir.join("settings.json");
    std::fs::write(dir.join("regular.json"), "{\"version\":1}").unwrap();
    mkfifo(&settings);
    let archive = fr.path("home/alice/logs/example.com-Oct-2026.gz");
    mkfifo(&archive);
    let live = fr.path("etc/apache2/logs/domlogs/alice/example.com");
    mkfifo(&live);
    let app = fr.app();
    let paths = app.paths.clone();
    let (regular, s2, a2, l2) = (
        dir.join("regular.json"),
        settings.clone(),
        archive.clone(),
        live.clone(),
    );
    let out = within(10, move || {
        let lst = std::fs::symlink_metadata(&regular).unwrap();
        let r1 = fs::read_regular(&s2, 65536, Some(&lst))
            .map(|_| ())
            .unwrap_err()
            .to_string();
        let (s, err) = UserSettings::read(&paths, "alice");
        let mut a = LogSource::new(&a2, false, None, true);
        let r3 = a.open().unwrap_err().to_string();
        let mut l = LogSource::new(&l2, false, None, false);
        let r4 = l.open().unwrap_err().to_string();
        (r1, s == UserSettings::default(), err, r3, r4)
    });
    assert!(out.0.contains("changed while opening"), "{}", out.0);
    assert!(out.1);
    assert!(out.2.unwrap().contains("not a regular file"));
    assert!(out.3.contains("not a regular file"), "archive FIFO");
    assert!(out.4.contains("not a regular file"), "live log FIFO");
    assert_eq!(
        fs::read_regular(&dir.join("regular.json"), 100, None).unwrap(),
        b"{\"version\":1}"
    );
    assert!(
        fs::read_regular(&dir.join("regular.json"), 5, None)
            .unwrap_err()
            .to_string()
            .contains("larger than")
    );
    std::fs::remove_file(&settings).unwrap();
    std::fs::write(&settings, "{\"version\":1,\"ui\":{\"dark\":\"dark\"}}").unwrap();
    let (s, err) = UserSettings::read(&app.paths, "alice");
    assert_eq!(s.dark, "dark");
    assert!(err.is_none());
}

#[test]
fn l1_a_gzip_bomb_is_read_in_bounded_memory_and_counted_as_bad_lines() {
    let mut fr = FakeRoot::create("rs-sec-bomb", json!({}));
    let bomb = fr.path("home/alice/logs/example.com-Oct-2026.gz");
    {
        let f = std::fs::File::create(&bomb).unwrap();
        let mut e = flate2::write::GzEncoder::new(f, flate2::Compression::best());
        let mb = vec![b'a'; 1 << 20];
        for _ in 0..200 {
            e.write_all(&mb).unwrap();
        }
        e.finish().unwrap();
    }
    assert!(
        std::fs::metadata(&bomb).unwrap().len() < 1 << 20,
        "tiny archive"
    );
    let b2 = bomb.clone();
    let (lines, bad) = within(120, move || {
        let mut src = LogSource::new(&b2, false, None, true);
        src.open().unwrap();
        let mut p = LogParser::new();
        let (mut lines, mut bad) = (0, 0);
        let mut buf = Vec::new();
        let mut scratch = Vec::new();
        while src.next_line(&mut buf).unwrap() {
            lines += 1;
            if p.parse(&buf, &mut scratch).is_none() {
                bad += 1;
            }
        }
        (lines, bad)
    });
    assert!(
        lines >= 1 && bad == lines,
        "the bomb is one (cut) bad line: {lines} {bad}"
    );
    fr.set_now(ts("2026-11-02 12:00:00"));
    let time = std::path::Path::new("/usr/bin/time");
    let b = if time.is_file() {
        let o = std::process::Command::new(time)
            .args(["-f", "%M"])
            .arg(common::bin())
            .args(["backfill", "--user", "alice", "--json"])
            .env("WEBPULSE_CONFIG", fr.config_path())
            .env("WEBPULSE_NOW", fr.now.unwrap().to_string())
            .output()
            .unwrap();
        let kb: u64 = String::from_utf8_lossy(&o.stderr)
            .lines()
            .last()
            .unwrap_or("0")
            .trim()
            .parse()
            .unwrap_or(0);
        assert!(kb < 128 * 1024, "peak RSS {kb} KiB");
        serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap()
    } else {
        fr.run("backfill --user alice --json").ok("backfill").json
    };
    assert!(b["bad"].as_i64().unwrap() >= 1);
    assert_eq!(b["imported"], 0);
    // normal archives still decompress (several members)
    let multi = fr.dir().join("multi.gz");
    let lines: Vec<String> = (0..20000)
        .map(|i| {
            format!(
                "line {i} {}",
                ((b'a' + (i % 26) as u8) as char).to_string().repeat(i % 50)
            )
        })
        .collect();
    let gz = |s: &str| {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(s.as_bytes()).unwrap();
        e.finish().unwrap()
    };
    let mut d = gz(&(lines[..7000].join("\n") + "\n"));
    d.extend(gz(&(lines[7000..].join("\n") + "\n")));
    std::fs::write(&multi, d).unwrap();
    let mut src = LogSource::new(&multi, false, None, true);
    src.open().unwrap();
    let mut got = Vec::new();
    while let Some(l) = src.read_line().unwrap() {
        got.push(String::from_utf8(l).unwrap());
    }
    assert_eq!(got, lines);
}

#[test]
fn l4_remove_tree_never_follows_symlinks_and_hands_foreign_directories_to_rm() {
    let fr = FakeRoot::create("rs-sec-rm", json!({}));
    let outside = fr.dir().join("outside");
    std::fs::create_dir_all(outside.join("sub")).unwrap();
    std::fs::write(outside.join("keep.txt"), "keep").unwrap();
    std::fs::write(outside.join("sub/keep.txt"), "keep").unwrap();
    let build = |d: &std::path::Path| {
        std::fs::create_dir_all(d.join("user/nested/deeper")).unwrap();
        std::fs::write(d.join("example.com.db"), "x").unwrap();
        std::fs::write(d.join("user/settings.json"), "{}").unwrap();
        std::os::unix::fs::symlink(&outside, d.join("user/link-dir")).unwrap();
        std::os::unix::fs::symlink(outside.join("keep.txt"), d.join("user/link-file")).unwrap();
        std::os::unix::fs::symlink(outside.join("sub"), d.join("user/nested/deeper/x")).unwrap();
    };
    let a = fr.dir().join("data-a");
    build(&a);
    assert!(fs::remove_tree(&a, None));
    assert!(!a.exists());
    let b = fr.dir().join("data-b");
    build(&b);
    assert!(
        fs::remove_tree(&b, Some(u32::MAX)),
        "foreign owner: rm -rf --one-file-system"
    );
    assert!(!b.exists());
    std::os::unix::fs::symlink(&outside, fr.dir().join("root-link")).unwrap();
    assert!(fs::remove_tree(&fr.dir().join("root-link"), None));
    assert_eq!(
        std::fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "keep"
    );
    assert_eq!(
        std::fs::read_to_string(outside.join("sub/keep.txt")).unwrap(),
        "keep"
    );
    assert!(fs::remove_tree(&fr.dir().join("does-not-exist"), None));
}

#[test]
fn l6_log_messages_cannot_forge_lines() {
    let mut fr = FakeRoot::create("rs-sec-log", json!({}));
    let app = fr.app();
    app.log.warn("cannot read archive /home/alice/logs/x.gz\n2026-01-01T00:00:00+00:00 [error] 1 forged\r\u{1b}[2J");
    let e = UserSettings::validate(
        &json!({"evil\n2026-01-01 [error] forged": 1, "domains": {"example.com": {"k\r\nx": 1}}}),
    )
    .unwrap_err()
    .to_string();
    assert!(
        !e.contains('\n') && !e.contains('\r'),
        "validation message is one line"
    );
    app.log
        .warn(format!("ignoring settings.json of alice: {e}"));
    std::fs::write(
        fr.path("home/alice/logs/example.com-\n[error] forged.gz"),
        "not gzip",
    )
    .unwrap();
    fr.set_now(ts("2026-11-02 12:00:00"));
    fr.run("backfill --user alice");
    let log = fr.log_text();
    let re = regex::Regex::new(
        r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d \[(debug|info|warn|error)\] \d+ ",
    )
    .unwrap();
    for l in log.trim_end_matches('\n').split('\n') {
        assert!(re.is_match(l), "well-formed log line: {l}");
    }
    assert!(!log.contains('\u{1b}'), "no escape sequences");
    assert!(
        log.matches("forged").count() >= 3,
        "the content is kept, neutralised"
    );
    assert_eq!(webpulse::log::sanitize("a\nb"), "a?b");
    assert_eq!(
        webpulse::log::sanitize(&"x".repeat(100000)).len(),
        webpulse::log::MAX_MESSAGE
    );
}

const OV_LOG: &str = "etc/apache2/logs/domlogs/alice/example.com-ssl_log";

fn flood(fr: &FakeRoot, n: usize) {
    let base = ts("2026-10-10 10:00:00");
    let tz: chrono_tz::Tz = "Europe/Rome".parse().unwrap();
    let file = fr.path(OV_LOG);
    let mut f = std::io::BufWriter::new(std::fs::File::create(file).unwrap());
    use chrono::TimeZone;
    for i in 0..n {
        let d = tz
            .timestamp_opt(base + (i as i64 * 1700) / n as i64, 0)
            .unwrap();
        writeln!(
            f,
            "203.0.{}.{} - - [{}] \"GET /p{} HTTP/2.0\" 200 1000 \"-\" \"Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0 u{i}\"",
            (i >> 8) & 255,
            i & 255,
            d.format("%d/%b/%Y:%H:%M:%S %z"),
            i % 500
        )
        .unwrap();
    }
}

fn consistent(fr: &FakeRoot, n: i64, what: &str) -> (i64, i64) {
    let db = fr.db("alice", "example.com");
    let open = Q::count(&db, "open_visits", "1=1");
    let closed = Q::count(&db, "visits", "1=1");
    assert_eq!(
        Q::sum(&db, "SELECT SUM(visits) FROM daily"),
        n,
        "{what}: visits counted"
    );
    assert_eq!(open + closed, n, "{what}: open + closed visits");
    assert_eq!(
        Q::sum(
            &db,
            "SELECT COALESCE(SUM(visits),0) FROM dim_daily WHERE dim = 'visit_duration'"
        ),
        closed,
        "{what}: duration buckets"
    );
    assert_eq!(
        Q::sum(&db, "SELECT SUM(pageviews) FROM daily"),
        n,
        "{what}: pageviews"
    );
    (open, closed)
}

#[test]
fn max_open_visits_60k_distinct_visitors_stay_bounded() {
    let mut fr = FakeRoot::create("rs-sec-openvisits", json!({}));
    let n = 60000;
    flood(&fr, n);
    fr.set_now(ts("2026-10-10 10:29:00"));
    let r = fr.run("collect --json").ok("collect");
    assert_eq!(r.i("lines"), n as i64);
    assert_eq!(r.i("errors"), 0);
    assert!(
        r.i("peak_memory") < 200 * 1048576,
        "peak memory {}",
        r.i("peak_memory")
    );
    let (open, _) = consistent(&fr, n as i64, "first run");
    assert!(open <= 50000, "open visits ({open}) within the default cap");
    assert!(fr.log_text().contains("closed early"));
    fr.patch_config(json!({"max_open_visits": 1000}));
    fr.set_now(ts("2026-10-10 10:35:00"));
    let r = fr.run("collect --json").ok("collect");
    assert!(r.i("peak_memory") < 200 * 1048576);
    let (open, _) = consistent(&fr, n as i64, "second run");
    assert!(open <= 1000, "open visits ({open}) within the lowered cap");
    fr.set_now(ts("2026-10-10 12:00:00"));
    fr.run("collect --json").ok("collect");
    let (open, closed) = consistent(&fr, n as i64, "third run");
    assert_eq!((open, closed), (0, n as i64));
}

#[test]
fn global_collect_a_crash_in_one_account_does_not_stop_the_others() {
    let mut fr = FakeRoot::create("rs-sec-child", json!({}));
    fr.write_log(
        OV_LOG,
        &[line("203.0.113.1", "2026-10-10 10:00:00", "/")],
        true,
    );
    fr.write_log(
        "etc/apache2/logs/domlogs/bob/blog.net",
        &[line("198.51.100.1", "2026-10-10 10:00:00", "/")],
        true,
    );
    let wrapper = fr.dir().join("crash-alice.sh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n  *'--user alice'*) exit 255 ;;\nesac\nexec '{}' \"$@\"\n",
            common::bin().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    fr.set_now(ts("2026-10-10 12:00:00"));
    let r = fr
        .run_env(
            "collect --json",
            &[("WEBPULSE_CHILD_EXE", wrapper.display().to_string())],
        )
        .ok("collect");
    assert_eq!(r.i("errors"), 1, "the crashed account is an error");
    assert_eq!(r.i("users"), 1, "bob was collected");
    assert_eq!(
        fr.scalar("bob", "blog.net", "SELECT SUM(hits) FROM daily"),
        1
    );
    assert!(!fr.has_db("alice", "example.com"), "alice untouched");
    assert!(
        fr.log_text()
            .contains("collect alice failed: child process exited with code 255")
    );
    assert_eq!(fr.run("collect --child").code, 2);
}

#[test]
fn tenant_a_registry_row_alone_never_opens_shared_domlogs_of_another_tenant() {
    let fr = FakeRoot::create("rs-sec-tenant", json!({}));
    let l = line("203.0.113.7", "2026-10-10 10:00:00", "/secret-page");
    fr.write_log(
        "etc/apache2/logs/domlogs/example.com",
        std::slice::from_ref(&l),
        true,
    );
    fr.write_log("var/log/nginx/domains/example.com-ssl_log", &[l], true);
    fr.write_log(
        "etc/apache2/logs/domlogs/bob/blog.net",
        &[line("198.51.100.1", "2026-10-10 10:00:00", "/")],
        true,
    );
    let app = fr.app();
    let mut c = Collector::new(&app);
    c.sync_registry(None).unwrap();
    // the race of the review: the row is inserted after the global sync
    app.registry()
        .unwrap()
        .set_domain("bob", "example.com", Some(true))
        .unwrap();
    let r = c.collect_user("bob", None).unwrap();
    assert!(
        !fr.has_db("bob", "example.com"),
        "no database for the foreign domain"
    );
    assert_eq!(r.domains, 1, "only blog.net collected");
    assert_eq!(r.lines, 1);
    assert!(!c.owns_domain("bob", "example.com"));
    assert!(c.owns_domain("alice", "example.com"));
    assert!(
        fr.log_text()
            .contains("not reading the shared logs of example.com for bob")
    );
    c.collect_user("alice", None).unwrap();
    assert_eq!(
        fr.scalar("alice", "example.com", "SELECT SUM(hits) FROM daily"),
        2,
        "both shared logs read for the owner"
    );
    fr.write_log(
        "etc/apache2/logs/domlogs/bob/extra.example",
        &[line("198.51.100.2", "2026-10-10 10:00:00", "/")],
        true,
    );
    fr.write_log(
        "etc/apache2/logs/domlogs/extra.example",
        &[line("198.51.100.3", "2026-10-10 10:00:00", "/")],
        true,
    );
    c.sync_registry(Some("bob")).unwrap();
    c.collect_user("bob", None).unwrap();
    assert_eq!(
        fr.scalar("bob", "extra.example", "SELECT SUM(hits) FROM daily"),
        1,
        "per-user dir only, not the shared copy"
    );
}

#[test]
fn tenant_the_global_run_resyncs_each_account_before_collecting_it() {
    let mut fr = FakeRoot::create("rs-sec-tenant-global", json!({}));
    fr.write_log(
        "etc/apache2/logs/domlogs/example.com",
        &[line("203.0.113.7", "2026-10-10 10:00:00", "/")],
        true,
    );
    fr.set_now(ts("2026-10-10 12:00:00"));
    fr.app()
        .registry()
        .unwrap()
        .set_domain("bob", "example.com", Some(true))
        .unwrap();
    fr.run("collect --json").ok("collect");
    assert!(!fr.has_db("bob", "example.com"));
    let rows = fr.app().registry().unwrap().domains(Some("bob")).unwrap();
    assert!(
        rows.iter()
            .find(|d| d.domain == "example.com")
            .unwrap()
            .removed_at
            .is_some(),
        "injected row marked removed"
    );
}

#[test]
fn tenant_a_shared_archive_directory_only_yields_the_accounts_own_domains() {
    let fr = FakeRoot::create("rs-sec-tenant-backfill", json!({}));
    fr.patch_config(
        json!({"paths": {"archive_dir": fr.path("var/archives").display().to_string()}}),
    );
    std::fs::create_dir_all(fr.path("var/archives")).unwrap();
    let gz = |s: String| {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(s.as_bytes()).unwrap();
        e.finish().unwrap()
    };
    std::fs::write(
        fr.path("var/archives/example.com-Oct-2026.gz"),
        gz(line("203.0.113.7", "2026-10-01 10:00:00", "/") + "\n"),
    )
    .unwrap();
    std::fs::write(
        fr.path("var/archives/blog.net-Oct-2026.gz"),
        gz(line("198.51.100.1", "2026-10-01 10:00:00", "/") + "\n"),
    )
    .unwrap();
    let app = fr.app();
    let mut c = Collector::new(&app);
    c.sync_registry(None).unwrap();
    app.registry()
        .unwrap()
        .set_domain("bob", "example.com", Some(true))
        .unwrap();
    let r = c.backfill_user("bob", None, None).unwrap();
    assert!(
        r.skipped
            .contains(&"example.com (not a domain of bob)".to_string()),
        "{r:?}"
    );
    assert!(!fr.has_db("bob", "example.com"));
    assert_eq!(
        fr.scalar("bob", "blog.net", "SELECT SUM(hits) FROM daily"),
        1
    );
}
