//! Port of tests/cases/engine_collect.php (CLI integration).

mod common;

use common::{FakeRoot, Q, line, ts};
use serde_json::{Value, json};

const DOMAINS: [(&str, &str); 3] = [
    ("alice", "example.com"),
    ("alice", "shop.example.com"),
    ("bob", "blog.net"),
];

fn sum_all(fr: &FakeRoot, sql: &str) -> i64 {
    DOMAINS
        .iter()
        .filter(|(u, d)| fr.has_db(u, d))
        .map(|(u, d)| fr.scalar(u, d, sql))
        .sum()
}

fn n(v: &Value, k: &str) -> i64 {
    v[k].as_i64().unwrap_or(0)
}

#[test]
fn generated_logs_line_accounting_matches_the_generator() {
    let mut fr = FakeRoot::create("rs-col-gen", json!({}));
    let sum = fr.generate_logs(&[
        ("seed", "3"),
        ("days", "3"),
        ("lines", "6000"),
        ("end", "2026-10-16"),
        ("no-archive", ""),
    ]);
    fr.set_now(ts("2026-10-17 03:00:00"));
    let r = fr.run("collect --json").ok("collect");
    assert_eq!(r.i("lines"), n(&sum, "lines"));
    assert_eq!(r.i("bad"), n(&sum, "malformed"));
    assert_eq!(r.i("ignored"), n(&sum, "head") + n(&sum, "acme"));
    assert_eq!(r.i("domains"), 3);
    assert_eq!(r.i("files"), 6);
    assert_eq!(
        sum_all(&fr, "SELECT SUM(bot_hits) FROM daily"),
        n(&sum, "bot") + n(&sum, "tls")
    );
    assert_eq!(
        sum_all(&fr, "SELECT SUM(hits) FROM daily"),
        n(&sum, "human")
    );
    assert_eq!(
        sum_all(&fr, "SELECT SUM(lines) FROM checkpoints"),
        n(&sum, "lines")
    );
    assert_eq!(
        sum_all(
            &fr,
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'total_lines'"
        ),
        n(&sum, "lines")
    );
    assert_eq!(
        sum_all(&fr, "SELECT COUNT(*) FROM open_visits"),
        0,
        "all visits closed the next day"
    );
    for (u, d) in DOMAINS {
        let db = fr.db(u, d);
        let t = |c: &str| Q::t(&db, c);
        let colsum = |dim: &str, c: &str| Q::col(&db, dim, c, None).values().sum::<i64>();
        let no_pv = Q::count(&db, "visits", "pageviews = 0");
        assert!(t("visits") > 0, "{d} has visits");
        assert_eq!(
            t("visits"),
            Q::count(&db, "visits", "1=1"),
            "{d} visits rows"
        );
        assert_eq!(t("visits"), colsum("channel", "visits"));
        assert_eq!(t("visits"), colsum("device", "visits"));
        assert_eq!(t("visits"), colsum("entry_page", "visits") + no_pv);
        assert_eq!(t("visits") - no_pv, colsum("exit_page", "visits"));
        assert_eq!(t("visits"), colsum("visit_duration", "visits"));
        assert_eq!(t("bounces"), Q::count(&db, "visits", "pageviews = 1"));
        assert_eq!(t("bounces"), colsum("channel", "bounces"));
        assert_eq!(
            t("duration"),
            Q::sum(&db, "SELECT SUM(duration) FROM visits")
        );
        assert_eq!(t("duration"), colsum("channel", "duration"));
        assert_eq!(t("pageviews"), colsum("page", "pageviews"));
        assert_eq!(t("pageviews"), colsum("channel", "pageviews"));
        assert_eq!(
            t("pageviews"),
            Q::sum(&db, "SELECT SUM(pageviews) FROM visits")
        );
        assert_eq!(t("hits"), colsum("status", "hits"));
        assert_eq!(t("hits"), Q::sum(&db, "SELECT SUM(hits) FROM hourly"));
        assert_eq!(
            t("visitors"),
            Q::sum(
                &db,
                "SELECT COUNT(*) FROM (SELECT DISTINCT day, visitor FROM visits)"
            )
        );
        assert!(!Q::col(&db, "bot", "hits", None).is_empty());
        assert_eq!(
            Q::col(&db, "country", "visits", None)
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["(unknown)".to_string()]
        );
    }
}

#[test]
fn rerunning_is_idempotent_appended_lines_are_processed_once() {
    let mut fr = FakeRoot::create("rs-col-rerun", json!({}));
    fr.generate_logs(&[
        ("seed", "5"),
        ("days", "2"),
        ("lines", "3000"),
        ("no-archive", ""),
    ]);
    fr.set_now(ts("2026-10-17 03:00:00"));
    fr.run("collect").ok("collect");
    let db = fr.db("alice", "example.com");
    let snap = Q::snapshot(&db);
    assert_eq!(fr.run("collect --json").ok("collect").i("lines"), 0);
    assert_eq!(snap, Q::snapshot(&db));
    let before = Q::totals(&db);
    fr.write_log(
        "etc/apache2/logs/domlogs/alice/example.com-ssl_log",
        &[
            line("203.0.113.200", "2026-10-17 02:00:00", "/new-page"),
            line("203.0.113.200", "2026-10-17 02:01:00", "/new-page-2"),
        ],
        true,
    );
    assert_eq!(fr.run("collect --json").ok("collect").i("lines"), 2);
    let after = Q::totals(&db);
    assert_eq!(
        n(&json!(before), "pageviews") + 2,
        n(&json!(after), "pageviews")
    );
    assert_eq!(n(&json!(before), "visits") + 1, n(&json!(after), "visits"));
    assert_eq!(fr.run("collect --json").ok("collect").i("lines"), 0);
    assert_eq!(after, Q::totals(&db));
}

#[test]
fn truncation_and_rotation_of_a_live_log_through_the_cli() {
    let mut fr = FakeRoot::create("rs-col-rotate", json!({}));
    let log = "etc/apache2/logs/domlogs/alice/example.com-ssl_log";
    fr.write_log(
        log,
        &[
            line("203.0.113.1", "2026-10-10 10:00:00", "/a"),
            line("203.0.113.2", "2026-10-10 10:00:00", "/b"),
        ],
        true,
    );
    fr.set_now(ts("2026-10-10 12:00:00"));
    fr.run("collect").ok("collect");
    let db = fr.db("alice", "example.com");
    assert_eq!(Q::t(&db, "pageviews"), 2);
    fr.write_log(
        log,
        &[line("203.0.113.3", "2026-10-10 12:30:00", "/c")],
        false,
    );
    fr.run("collect").ok("collect");
    assert_eq!(Q::t(&db, "pageviews"), 3);
    std::fs::rename(fr.path(log), format!("{}.1", fr.path(log).display())).unwrap();
    fr.write_log(
        log,
        &[
            line("203.0.113.4", "2026-10-10 13:00:00", "/d"),
            line("203.0.113.5", "2026-10-10 13:00:00", "/e"),
        ],
        true,
    );
    fr.run("collect").ok("collect");
    assert_eq!(Q::t(&db, "pageviews"), 5);
    fr.run("collect").ok("collect");
    assert_eq!(Q::t(&db, "pageviews"), 5);
}

fn gz(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

#[test]
fn backfill_imports_archives_once_and_never_what_live_collection_covered() {
    let mut fr = FakeRoot::create("rs-col-backfill", json!({}));
    let sum = fr.generate_logs(&[
        ("seed", "9"),
        ("days", "20"),
        ("lines", "8000"),
        ("end", "2026-10-10"),
    ]);
    assert!(sum["files"]["home/alice/logs/example.com-ssl_log-Sep-2026.gz"].is_number());
    fr.set_now(ts("2026-10-11 03:00:00"));
    fr.run("collect").ok("collect");
    let db = fr.db("alice", "example.com");
    let live = Q::totals(&db);
    let min_day = || {
        db.query_row("SELECT MIN(day) FROM daily", [], |r| r.get::<_, String>(0))
            .unwrap()
    };
    assert_eq!(min_day(), "2026-10-01");
    let r = fr.run("backfill --user alice --json").ok("backfill");
    let arch = n(&sum["files"], "home/alice/logs/example.com-Sep-2026.gz")
        + n(
            &sum["files"],
            "home/alice/logs/example.com-ssl_log-Sep-2026.gz",
        );
    assert_eq!(r.i("files"), 2);
    assert_eq!(r.i("lines"), arch);
    assert_eq!(min_day(), "2026-09-21");
    let all = Q::totals(&db);
    assert!(n(&json!(all), "hits") > n(&json!(live), "hits"));
    assert_eq!(Q::count(&db, "imported_archives", "size > 0"), 2);
    let r = fr.run("backfill --user alice --json").ok("backfill");
    assert_eq!(r.i("files"), 0);
    assert_eq!(r.i("skipped_files"), 2);
    assert_eq!(all, Q::totals(&db));
    // an archive of the current month duplicates what the live tail already read: skipped by live_since
    let live_file = fr.domlog("alice", "example.com-ssl_log");
    std::fs::write(
        fr.path("home/alice/logs/example.com-ssl_log-Oct-2026.gz"),
        gz(&std::fs::read(live_file).unwrap()),
    )
    .unwrap();
    let r = fr
        .run("backfill --user alice --domain example.com --json")
        .ok("backfill");
    assert_eq!(r.i("files"), 1);
    assert_eq!(r.i("imported"), 0);
    assert_eq!(all, Q::totals(&db));
    let jobs = fr.app().registry().unwrap().jobs(10).unwrap();
    assert_eq!(jobs[0]["type"], "backfill");
    assert_eq!(jobs[0]["status"], "ok");
}

#[test]
fn backfill_before_live_collection_live_tail_skips_what_the_archives_covered() {
    let mut fr = FakeRoot::create("rs-col-backfill-first", json!({}));
    let log = "etc/apache2/logs/domlogs/alice/example.com-ssl_log";
    let lines = [
        line("203.0.113.1", "2026-10-10 10:00:00", "/a"),
        line("203.0.113.2", "2026-10-10 10:05:00", "/b"),
    ];
    std::fs::write(
        fr.path("home/alice/logs/example.com-ssl_log-Oct-2026.gz"),
        gz(format!("{}\n", lines.join("\n")).as_bytes()),
    )
    .unwrap();
    fr.write_log(log, &lines, true);
    fr.write_log(
        log,
        &[line("203.0.113.3", "2026-10-10 10:10:00", "/c")],
        true,
    );
    fr.set_now(ts("2026-10-10 12:00:00"));
    fr.run("backfill --user alice").ok("backfill");
    let db = fr.db("alice", "example.com");
    assert_eq!(Q::t(&db, "pageviews"), 2);
    fr.run("collect").ok("collect");
    assert_eq!(
        Q::t(&db, "pageviews"),
        3,
        "only the line newer than the archive is added"
    );
}

#[test]
fn grown_archive_resumes_after_its_last_imported_line() {
    let mut fr = FakeRoot::create("rs-col-grow", json!({}));
    let arch = fr.path("home/alice/logs/example.com-ssl_log-Sep-2026.gz");
    std::fs::write(
        &arch,
        gz(format!("{}\n", line("203.0.113.1", "2026-09-10 10:00:00", "/a")).as_bytes()),
    )
    .unwrap();
    fr.set_now(ts("2026-10-10 12:00:00"));
    fr.run("backfill --user alice").ok("backfill");
    let mut d = std::fs::read(&arch).unwrap();
    d.extend(gz(format!(
        "{}\n",
        line("203.0.113.2", "2026-09-11 10:00:00", "/b")
    )
    .as_bytes()));
    std::fs::write(&arch, d).unwrap();
    let t = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&arch)
        .unwrap()
        .set_modified(t)
        .unwrap();
    let r = fr.run("backfill --user alice --json").ok("backfill");
    assert_eq!(r.i("lines"), 2);
    assert_eq!(r.i("imported"), 1);
    assert_eq!(Q::t(&fr.db("alice", "example.com"), "pageviews"), 2);
}

#[test]
fn enable_disable_accounts_and_domains() {
    let mut fr = FakeRoot::create("rs-col-switch", json!({}));
    fr.generate_logs(&[
        ("seed", "1"),
        ("days", "1"),
        ("lines", "800"),
        ("no-archive", ""),
    ]);
    fr.set_now(ts("2026-10-17 03:00:00"));
    fr.run("accounts disable bob").ok("accounts");
    fr.run("domains disable alice shop.example.com")
        .ok("domains");
    assert_eq!(fr.run("collect --json").ok("collect").i("domains"), 1);
    assert!(fr.has_db("alice", "example.com"));
    assert!(!fr.has_db("alice", "shop.example.com"));
    assert!(!fr.has_db("bob", "blog.net"));
    fr.run("domains enable bob blog.net").ok("domains");
    fr.run("collect").ok("collect");
    assert!(
        !fr.has_db("bob", "blog.net"),
        "a domain override cannot re-enable a disabled account"
    );
    fr.run("accounts reset bob").ok("accounts");
    fr.run("domains reset alice shop.example.com").ok("domains");
    assert_eq!(fr.run("collect --json").ok("collect").i("domains"), 3);
    assert!(fr.has_db("bob", "blog.net"));
    fr.run("config set default_enabled false").ok("config");
    fr.run("accounts reset alice").ok("accounts");
    let app = fr.app();
    let reg = app.registry().unwrap();
    assert!(!reg.is_enabled("alice", "example.com").unwrap());
    assert!(
        reg.is_enabled("bob", "blog.net").unwrap(),
        "bob has a domain override"
    );
    let list = fr.run("accounts list --json").ok("accounts").json["accounts"].clone();
    let users: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["user"].as_str().unwrap())
        .collect();
    assert_eq!(users, ["alice", "bob"]);
    assert_eq!(list[1]["owner"], "reseller1");
    let r = fr
        .run("collect --user alice --domain blog.net --json")
        .ok("collect");
    assert_eq!(
        r.json["skipped"],
        json!(["blog.net (not a domain of alice)"])
    );
    assert!(!fr.has_db("alice", "blog.net"));
    let doms = fr.run("domains list alice --json").ok("domains").json["domains"].clone();
    let kinds: Vec<(String, String)> = doms
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["domain"].as_str().unwrap().into(),
                d["kind"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("example.com".to_string(), "main".to_string()),
            ("shop.example.com".into(), "sub".into())
        ]
    );
}

#[test]
fn hooked_collect_drains_flagged_accounts_only_and_removes_the_flags() {
    let mut fr = FakeRoot::create("rs-col-hook", json!({}));
    fr.generate_logs(&[
        ("seed", "2"),
        ("days", "1"),
        ("lines", "600"),
        ("no-archive", ""),
    ]);
    fr.set_now(ts("2026-10-17 03:00:00"));
    fr.run("setup --quiet").ok("setup");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(fr.path("var/webpulse/run/hook"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o1777
    );
    assert_eq!(fr.run("collect --hooked --json").ok("hooked").i("users"), 0);
    for f in ["bob", "ghost", "Bad..Name"] {
        std::fs::write(fr.path(&format!("var/webpulse/run/hook/{f}")), "").unwrap();
    }
    assert_eq!(fr.run("hooked --json").ok("hooked").i("users"), 1);
    assert!(fr.has_db("bob", "blog.net"));
    assert!(!fr.has_db("alice", "example.com"));
    assert!(!fr.path("var/webpulse/run/hook/bob").exists());
    assert!(
        !fr.path("var/webpulse/run/hook/ghost").exists(),
        "unknown account flag removed"
    );
    assert_eq!(
        fr.app().registry().unwrap().jobs(5).unwrap()[0]["type"],
        "hook"
    );
}

#[test]
fn locks_concurrent_collect_exits_0_locked_accounts_are_skipped() {
    let mut fr = FakeRoot::create("rs-col-lock", json!({}));
    fr.generate_logs(&[
        ("seed", "2"),
        ("days", "1"),
        ("lines", "400"),
        ("no-archive", ""),
    ]);
    fr.set_now(ts("2026-10-17 03:00:00"));
    std::fs::create_dir_all(fr.path("var/webpulse/run")).unwrap();
    let lock = webpulse::util::lock::Lock::acquire(&fr.path("var/webpulse/run/collect.lock"), 0.0)
        .unwrap()
        .unwrap();
    let r = fr.run("collect").ok("collect");
    assert!(r.stdout.contains("already running"));
    assert!(!fr.path("var/webpulse/data/alice").is_dir());
    drop(lock);
    let ul = webpulse::util::lock::Lock::acquire(&fr.path("var/webpulse/run/user-alice.lock"), 0.0)
        .unwrap()
        .unwrap();
    let r = fr.run("collect --json").ok("collect");
    assert_eq!(r.json["skipped"], json!(["alice (locked)"]));
    assert!(fr.has_db("bob", "blog.net"));
    drop(ul);
}

#[test]
fn jobs_job_creates_a_row_job_id_adopts_one() {
    let mut fr = FakeRoot::create("rs-col-jobs", json!({}));
    fr.generate_logs(&[
        ("seed", "2"),
        ("days", "1"),
        ("lines", "300"),
        ("no-archive", ""),
    ]);
    fr.set_now(ts("2026-10-17 03:00:00"));
    let r = fr.run("collect --job --json").ok("collect");
    let id = r.i("job_id");
    let app = fr.app();
    let reg = app.registry().unwrap();
    let job = reg.job(id).unwrap().unwrap();
    assert_eq!(
        (
            job["type"].as_str(),
            job["scope"].as_str(),
            job["status"].as_str()
        ),
        (Some("collect"), Some("all"), Some("ok"))
    );
    assert_eq!(job["lines"], 300);
    let pre = reg.start_job("rollup", Some("all"), None).unwrap();
    assert_eq!(reg.job(pre).unwrap().unwrap()["status"], "running");
    fr.run(&format!("rollup --job {pre}")).ok("rollup");
    assert_eq!(reg.job(pre).unwrap().unwrap()["status"], "ok");
    fr.run("collect --user alice --domain example.com --job")
        .ok("collect");
    assert_eq!(reg.jobs(1).unwrap()[0]["scope"], "alice/example.com");
}

#[test]
fn unreadable_log_file_does_not_abort_the_run() {
    if webpulse::util::fs::is_root() {
        return;
    }
    let mut fr = FakeRoot::create("rs-col-unreadable", json!({}));
    fr.generate_logs(&[
        ("seed", "2"),
        ("days", "1"),
        ("lines", "500"),
        ("no-archive", ""),
    ]);
    fr.set_now(ts("2026-10-17 03:00:00"));
    use std::os::unix::fs::PermissionsExt;
    let f = fr.domlog("alice", "example.com-ssl_log");
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o0)).unwrap();
    let r = fr.run("collect --json --quiet");
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
    let r = r.ok("collect");
    assert_eq!(r.i("errors"), 1);
    assert_eq!(r.i("domains"), 3);
    assert!(fr.has_db("bob", "blog.net"));
    assert!(fr.log_text().contains("cannot read"));
}

#[test]
fn setup_is_idempotent_and_creates_the_layout() {
    use std::os::unix::fs::PermissionsExt;
    let fr = FakeRoot::create("rs-col-setup", json!({}));
    let mode = |p: &str| std::fs::metadata(fr.path(p)).unwrap().permissions().mode() & 0o777;
    let r = fr.run("setup --json").ok("setup");
    assert_eq!(r.i("accounts"), 2);
    assert_eq!(mode("var/webpulse/webpulse.db"), 0o600);
    assert_eq!(mode("var/webpulse/data/alice"), 0o750);
    assert_eq!(mode("var/webpulse/data/alice/user"), 0o700);
    assert_eq!(mode("var/webpulse/log"), 0o700);
    assert_eq!(
        fr.run("setup --json").ok("setup").json["actions"],
        json!([])
    );
    // setup with a missing config file creates it from the default
    let cfg = fr.dir().join("new/config.json");
    let _ = common::exec(
        &[
            "setup".into(),
            "--config".into(),
            cfg.display().to_string(),
            "--no-sync".into(),
            "--json".into(),
        ],
        &[("WEBPULSE_CONFIG", "/nonexistent".into())],
    );
    assert!(cfg.is_file());
    assert_eq!(
        std::fs::metadata(&cfg).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let a: Value = serde_json::from_slice(
        &std::fs::read(common::repo().join("etc/config.json.default")).unwrap(),
    )
    .unwrap();
    let b: Value = serde_json::from_slice(&std::fs::read(&cfg).unwrap()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn status_config_usage_errors_and_purge() {
    let mut fr = FakeRoot::create("rs-col-cli", json!({}));
    fr.generate_logs(&[
        ("seed", "2"),
        ("days", "1"),
        ("lines", "300"),
        ("no-archive", ""),
    ]);
    fr.set_now(ts("2026-10-17 03:00:00"));
    fr.run("collect").ok("collect");
    let st = fr.run("status --json").ok("status").json;
    assert_eq!(st["registry"]["ok"], true);
    assert_eq!(st["domains"]["with_data"], 3);
    assert_eq!(st["accounts"]["total"], 2);
    assert_eq!(st["lines_total"], 300);
    assert!(fr.run("status").ok("status").stdout.contains("accounts:"));
    assert_eq!(
        fr.run("config get raw_retention_days --json")
            .ok("config")
            .json,
        30
    );
    fr.run("config set raw_retention_days 60").ok("config");
    assert_eq!(fr.run("config get raw_retention_days --json").json, 60);
    fr.run("config set exclude_ips '[\"10.0.0.0/8\"]'")
        .ok("config");
    assert_eq!(
        fr.run("config get exclude_ips --json").json,
        json!(["10.0.0.0/8"])
    );
    assert_eq!(fr.run("config set raw_retention_days banana").code, 2);
    assert_eq!(fr.run("config set paths.data_dir /tmp").code, 2);
    fr.run("config unset raw_retention_days").ok("config");
    assert_eq!(fr.run("config get raw_retention_days --json").json, 30);
    assert_eq!(fr.run("config get nope.nope").code, 1);
    for (args, code) in [
        ("collect --bogus", 2),
        ("backfill", 2),
        ("collect --domain example.com", 2),
        ("collect --user ../etc", 2),
        ("purge --user alice", 2),
        ("nosuchcommand", 2),
        ("--help", 0),
        ("collect --child", 2),
    ] {
        assert_eq!(fr.run(args).code, code, "{args}");
    }
    assert!(
        fr.run("collect --help")
            .stdout
            .contains("Usage: webpulse collect")
    );
    assert!(fr.run("--version").stdout.starts_with("webpulse "));
    fr.run("purge --user alice --domain shop.example.com --yes")
        .ok("purge");
    assert!(!fr.has_db("alice", "shop.example.com"));
    assert!(fr.has_db("alice", "example.com"));
    assert_eq!(
        fr.app().registry().unwrap().summaries(None).unwrap().len(),
        2
    );
    fr.run("purge --user alice --yes").ok("purge");
    assert!(!fr.has_db("alice", "example.com"));
    assert_eq!(
        fr.app().registry().unwrap().summaries(None).unwrap().len(),
        1
    );
}

#[test]
fn geoip_update_from_a_local_file_then_countries_are_recorded() {
    use common::{MmdbWriter, map, s};
    use std::io::Write;
    let mut fr = FakeRoot::create("rs-col-geoip", json!({}));
    let mut w = MmdbWriter::new(6, 24, &[], 0);
    w.insert(
        "203.0.113.0/24",
        &map(&[("country", map(&[("iso_code", s("IT"))]))]),
    );
    w.insert(
        "2001:db8::/32",
        &map(&[("country", map(&[("iso_code", s("DE"))]))]),
    );
    let src = fr.dir().join("test.mmdb.gz");
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&w.build(&[])).unwrap();
    std::fs::write(&src, e.finish().unwrap()).unwrap();
    assert_eq!(fr.run("geoip-update --json").code, 0);
    let r = fr
        .run(&format!(
            "geoip-update --force --url file://{} --json",
            src.display()
        ))
        .ok("geoip-update");
    assert_eq!(r.json["database_type"], "WebPulse-Test");
    assert!(
        fr.path("var/webpulse/geoip/dbip-country-lite.mmdb")
            .is_file()
    );
    assert_eq!(
        fr.run("geoip-update --force --url file:///nonexistent.gz")
            .code,
        1
    );
    fr.patch_config(json!({"geoip": {"enabled": true}}));
    fr.write_log(
        "etc/apache2/logs/domlogs/alice/example.com-ssl_log",
        &[
            line("203.0.113.5", "2026-10-10 10:00:00", "/"),
            line("2001:db8::5", "2026-10-10 10:00:00", "/"),
            line("192.0.2.1", "2026-10-10 10:00:00", "/"),
        ],
        true,
    );
    fr.set_now(ts("2026-10-10 12:00:00"));
    fr.run("collect").ok("collect");
    let c = Q::col(&fr.db("alice", "example.com"), "country", "visits", None);
    assert_eq!(
        c.into_iter().collect::<Vec<_>>(),
        vec![
            ("(unknown)".to_string(), 1),
            ("de".into(), 1),
            ("it".into(), 1)
        ]
    );
    assert_eq!(
        fr.run("status --json").ok("status").json["geoip"]["present"],
        true
    );
}
