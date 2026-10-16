//! Port of tests/cases/engine_sessionizer.php.
#![allow(clippy::type_complexity)]

mod common;

use common::{FakeRoot, GOOGLEBOT, IPHONE, L, Q, line, ts};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const SSL: &str = "etc/apache2/logs/domlogs/alice/example.com-ssl_log";
const PLAIN: &str = "etc/apache2/logs/domlogs/alice/example.com";

fn scenario(
    name: &str,
    lines: &[String],
    now: &str,
    config: Value,
) -> (FakeRoot, Connection, common::Out) {
    let mut fr = FakeRoot::create(&format!("rs-sess-{name}"), config);
    fr.write_log(SSL, lines, true);
    fr.set_now(ts(now));
    let r = fr.run("collect --json").ok("collect");
    let db = fr.db("alice", "example.com");
    (fr, db, r)
}

fn collect(fr: &mut FakeRoot, now: &str) {
    fr.set_now(ts(now));
    fr.run("collect --json").ok("collect");
}

fn m(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn d(db: &Connection, day: &str, cols: &[&str]) -> Vec<i64> {
    let row = Q::daily(db, day);
    cols.iter().map(|c| row[*c].as_i64().unwrap()).collect()
}

#[test]
fn visits_bounces_duration_entry_and_exit_pages() {
    let (_fr, db, _) = scenario(
        "basic",
        &[
            L::new("203.0.113.1", "2026-10-10 10:00:00", "/")
                .referrer("https://www.google.com/")
                .s(),
            L::new("203.0.113.1", "2026-10-10 10:00:01", "/assets/app.css")
                .bytes(500)
                .referrer("https://example.com/")
                .s(),
            L::new("203.0.113.1", "2026-10-10 10:01:00", "/pricing")
                .bytes(2000)
                .referrer("https://example.com/")
                .s(),
            L::new("203.0.113.1", "2026-10-10 10:03:00", "/contact")
                .bytes(3000)
                .referrer("https://example.com/pricing")
                .s(),
            L::new("198.51.100.2", "2026-10-10 10:05:00", "/")
                .ua(IPHONE)
                .s(),
            L::new("66.249.66.1", "2026-10-10 10:06:00", "/")
                .bytes(1500)
                .ua(GOOGLEBOT)
                .s(),
        ],
        "2026-10-10 12:00:00",
        json!({}),
    );
    assert_eq!(
        d(
            &db,
            "2026-10-10",
            &[
                "visits",
                "visitors",
                "pageviews",
                "hits",
                "bytes",
                "bounces",
                "duration",
                "bot_hits",
                "bot_bytes"
            ]
        ),
        [2, 2, 4, 5, 7500, 1, 180, 1, 1500]
    );
    assert_eq!(Q::col(&db, "entry_page", "visits", None), m(&[("/", 2)]));
    assert_eq!(Q::col(&db, "entry_page", "bounces", None), m(&[("/", 1)]));
    assert_eq!(
        Q::col(&db, "exit_page", "visits", None),
        m(&[("/", 1), ("/contact", 1)])
    );
    assert_eq!(
        Q::col(&db, "page", "pageviews", None),
        m(&[("/", 2), ("/contact", 1), ("/pricing", 1)])
    );
    assert_eq!(
        Q::col(&db, "page", "visits", None),
        m(&[("/", 2), ("/contact", 1), ("/pricing", 1)])
    );
    assert_eq!(
        Q::col(&db, "pages_per_visit", "visits", None),
        m(&[("1", 1), ("3", 1)])
    );
    assert_eq!(
        Q::col(&db, "visit_duration", "visits", None),
        m(&[("0-10s", 1), ("2-5m", 1)])
    );
    assert_eq!(
        Q::col(&db, "channel", "visits", None),
        m(&[("direct", 1), ("organic", 1)])
    );
    assert_eq!(Q::col(&db, "channel", "bounces", None), m(&[("direct", 1)]));
    assert_eq!(
        Q::col(&db, "channel", "duration", None),
        m(&[("organic", 180)])
    );
    assert_eq!(
        Q::col(&db, "channel", "pageviews", None),
        m(&[("direct", 1), ("organic", 3)])
    );
    assert_eq!(
        Q::col(&db, "search_engine", "visits", None),
        m(&[("google", 1)])
    );
    assert_eq!(
        Q::col(&db, "device", "visits", None),
        m(&[("desktop", 1), ("mobile", 1)])
    );
    assert_eq!(Q::col(&db, "bot", "hits", None), m(&[("Googlebot", 1)]));
    assert_eq!(
        Q::col(&db, "country", "visits", None),
        m(&[("(unknown)", 2)])
    );
    assert_eq!(Q::count(&db, "visits", "1=1"), 2);
    assert_eq!(Q::count(&db, "open_visits", "1=1"), 0);
    assert_eq!(Q::count(&db, "events", "type = 'pageview'"), 4);
    assert_eq!(Q::count(&db, "events", "type = 'bot'"), 0);
    let v: (String, String, i64, i64, String, String, String, String, String, Option<String>, String, i64) = db
        .query_row(
            "SELECT entry_page, exit_page, pageviews, duration, channel, search_engine, browser, browser_version, os_version, ip, visitor, hits FROM visits ORDER BY start_ts LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?)),
        )
        .unwrap();
    assert_eq!(
        (
            v.0.as_str(),
            v.1.as_str(),
            v.2,
            v.3,
            v.4.as_str(),
            v.5.as_str(),
            v.6.as_str(),
            v.7.as_str(),
            v.8.as_str(),
            v.9.clone()
        ),
        (
            "/",
            "/contact",
            3,
            180,
            "organic",
            "google",
            "Chrome",
            "Chrome 129",
            "Windows 10/11",
            None
        )
    );
    assert!(
        v.10.len() == 16
            && v.10
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    assert_eq!(v.11, 4, "static hit attached to the visit");
    let h: Vec<(i64, i64, i64)> = db
        .prepare(
            "SELECT hour, visits, pageviews FROM hourly WHERE day = '2026-10-10' ORDER BY hour",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    assert_eq!(h, [(10, 2, 4)]);
}

#[test]
fn session_timeout_splits_visits_unique_visitors_per_day() {
    let (_fr, db, _) = scenario(
        "timeout",
        &[
            line("203.0.113.1", "2026-10-10 10:00:00", "/a"),
            line("203.0.113.1", "2026-10-10 10:29:59", "/b"),
            line("203.0.113.1", "2026-10-10 10:59:59", "/c"),
            line("203.0.113.1", "2026-10-10 11:00:00", "/d"),
        ],
        "2026-10-10 13:00:00",
        json!({}),
    );
    assert_eq!(
        d(
            &db,
            "2026-10-10",
            &["visits", "visitors", "bounces", "duration"]
        ),
        [2, 1, 0, 1800]
    );
    assert_eq!(
        Q::col(&db, "entry_page", "visits", None),
        m(&[("/a", 1), ("/c", 1)])
    );
    assert_eq!(
        Q::col(&db, "exit_page", "visits", None),
        m(&[("/b", 1), ("/d", 1)])
    );
}

#[test]
fn visit_crossing_midnight_stays_on_its_first_day() {
    let (_fr, db, _) = scenario(
        "midnight",
        &[
            line("203.0.113.1", "2026-10-10 23:50:00", "/a"),
            line("203.0.113.1", "2026-10-11 00:10:00", "/b"),
            line("203.0.113.1", "2026-10-11 00:20:00", "/c"),
        ],
        "2026-10-11 05:00:00",
        json!({}),
    );
    let cols = ["visits", "visitors", "pageviews", "bounces", "duration"];
    assert_eq!(d(&db, "2026-10-10", &cols), [1, 1, 1, 0, 1800]);
    assert_eq!(d(&db, "2026-10-11", &cols), [0, 0, 2, 0, 0]);
    assert_eq!(
        Q::col(&db, "exit_page", "visits", Some("2026-10-10")),
        m(&[("/c", 1)])
    );
    assert_eq!(
        Q::col(&db, "pages_per_visit", "visits", Some("2026-10-10")),
        m(&[("3", 1)])
    );
    assert_eq!(
        Q::col(&db, "page", "visits", Some("2026-10-11")),
        m(&[("/b", 1), ("/c", 1)])
    );
    assert_eq!(Q::count(&db, "visits", "1=1"), 1);
}

#[test]
fn open_visit_survives_runs() {
    let (mut fr, db, _) = scenario(
        "open",
        &[line("203.0.113.1", "2026-10-10 10:00:00", "/")],
        "2026-10-10 10:05:00",
        json!({}),
    );
    assert_eq!(Q::count(&db, "open_visits", "1=1"), 1);
    assert_eq!(Q::count(&db, "visits", "1=1"), 0);
    assert_eq!(
        d(&db, "2026-10-10", &["visits", "pageviews", "bounces"]),
        [1, 1, 1],
        "open visit counted already"
    );
    assert_eq!(Q::col(&db, "exit_page", "visits", None), m(&[("/", 1)]));
    fr.write_log(
        SSL,
        &[line("203.0.113.1", "2026-10-10 10:10:00", "/b")],
        true,
    );
    collect(&mut fr, "2026-10-10 10:12:00");
    assert_eq!(
        d(
            &db,
            "2026-10-10",
            &["visits", "pageviews", "bounces", "duration"]
        ),
        [1, 2, 0, 600]
    );
    assert_eq!(Q::col(&db, "exit_page", "visits", None), m(&[("/b", 1)]));
    assert_eq!(Q::col(&db, "entry_page", "visits", None), m(&[("/", 1)]));
    assert!(Q::col(&db, "entry_page", "bounces", None).is_empty());
    assert_eq!(Q::count(&db, "open_visits", "1=1"), 1);
    assert_eq!(Q::dim(&db, "channel", None)["direct"][4], 0);
    fr.write_log(
        SSL,
        &[line("203.0.113.1", "2026-10-10 10:20:00", "/b")],
        true,
    );
    collect(&mut fr, "2026-10-10 10:21:00");
    assert_eq!(
        Q::col(&db, "exit_page", "visits", None),
        m(&[("/b", 1)]),
        "reload of the same page keeps the exit row"
    );
    assert_eq!(Q::d(&db, "2026-10-10", "duration"), 1200);
    collect(&mut fr, "2026-10-10 11:00:00");
    assert_eq!(Q::count(&db, "open_visits", "1=1"), 0);
    assert_eq!(Q::count(&db, "visits", "1=1"), 1);
    assert_eq!(
        Q::col(&db, "pages_per_visit", "visits", None),
        m(&[("3", 1)])
    );
    assert_eq!(
        Q::col(&db, "visit_duration", "visits", None),
        m(&[("10-30m", 1)])
    );
    assert_eq!(
        d(
            &db,
            "2026-10-10",
            &["visits", "visitors", "pageviews", "bounces", "duration"]
        ),
        [1, 1, 3, 0, 1200]
    );
    let before = Q::snapshot(&db);
    collect(&mut fr, "2026-10-10 12:00:00");
    assert_eq!(
        before,
        Q::snapshot(&db),
        "re-run without new lines changes nothing"
    );
}

#[test]
fn static_redirect_and_error_hits_do_not_start_visits() {
    let (_fr, db, _) = scenario(
        "nostart",
        &[
            L::new("203.0.113.9", "2026-10-10 10:00:00", "/img/a.png")
                .bytes(100)
                .s(),
            L::new("203.0.113.9", "2026-10-10 10:00:01", "/old")
                .status(301)
                .bytes(50)
                .s(),
            L::new("203.0.113.9", "2026-10-10 10:00:02", "/missing")
                .status(404)
                .bytes(70)
                .referrer("https://www.bing.com/search?q=x")
                .s(),
            L::new("203.0.113.9", "2026-10-10 10:00:03", "/broken")
                .status(500)
                .bytes(10)
                .s(),
            L::new("203.0.113.9", "2026-10-10 10:00:04", "/form")
                .status(204)
                .bytes(0)
                .method("POST")
                .s(),
            L::new("203.0.113.8", "2026-10-10 10:00:05", "/files/doc.pdf")
                .bytes(90000)
                .referrer("https://blog.partner.org/x")
                .s(),
        ],
        "2026-10-10 12:00:00",
        json!({}),
    );
    assert_eq!(
        d(
            &db,
            "2026-10-10",
            &[
                "visits",
                "pageviews",
                "hits",
                "downloads",
                "errors_4xx",
                "errors_5xx",
                "bounces"
            ]
        ),
        [1, 0, 6, 1, 1, 1, 0]
    );
    assert_eq!(
        Q::col(&db, "not_found", "hits", None),
        m(&[("/missing", 1)])
    );
    assert_eq!(
        Q::col(&db, "not_found_referrer", "hits", None),
        m(&[("/missing \u{2190} bing.com", 1)])
    );
    assert_eq!(
        Q::col(&db, "server_error", "hits", None),
        m(&[("/broken", 1)])
    );
    assert_eq!(
        Q::col(&db, "download", "hits", None),
        m(&[("/files/doc.pdf", 1)])
    );
    assert_eq!(
        Q::col(&db, "static_file", "hits", None),
        m(&[("/img/a.png", 1)])
    );
    assert_eq!(
        Q::col(&db, "status", "hits", None),
        m(&[("200", 2), ("204", 1), ("301", 1), ("404", 1), ("500", 1)])
    );
    assert_eq!(
        Q::col(&db, "method", "hits", None),
        m(&[("GET", 5), ("POST", 1)])
    );
    assert_eq!(
        Q::col(&db, "file_type", "hits", None),
        m(&[("html", 4), ("pdf", 1), ("png", 1)])
    );
    assert_eq!(
        Q::col(&db, "channel", "visits", None),
        m(&[("referral", 1)])
    );
    assert_eq!(
        Q::col(&db, "referrer_host", "visits", None),
        m(&[("blog.partner.org", 1)])
    );
    assert_eq!(
        Q::col(&db, "referrer_url", "visits", None),
        m(&[("blog.partner.org/x", 1)])
    );
    assert_eq!(
        Q::col(&db, "visit_duration", "visits", None),
        m(&[("0-10s", 1)])
    );
    assert!(Q::col(&db, "pages_per_visit", "visits", None).is_empty());
    assert_eq!(Q::count(&db, "events", "1=1"), 4);
    assert_eq!(Q::count(&db, "events", "visit_id IS NOT NULL"), 1);
}

#[test]
fn acquisition_is_decided_by_the_first_hit_of_the_visit() {
    let (_fr, db, _) = scenario(
        "attrib",
        &[
            L::new(
                "203.0.113.1",
                "2026-10-10 10:00:00",
                "/?utm_source=news&utm_medium=email&utm_campaign=oct",
            )
            .referrer("https://mail.google.com/")
            .s(),
            L::new("203.0.113.1", "2026-10-10 10:01:00", "/x")
                .referrer("https://l.facebook.com/")
                .s(),
            L::new("203.0.113.2", "2026-10-10 10:00:00", "/?gclid=abc")
                .referrer("https://www.google.com/")
                .s(),
            L::new("203.0.113.3", "2026-10-10 10:00:00", "/")
                .referrer("https://www.google.de/search?q=Web+Pulse")
                .s(),
            L::new("203.0.113.4", "2026-10-10 10:00:00", "/")
                .referrer("https://t.co/xyz")
                .s(),
            L::new("203.0.113.5", "2026-10-10 10:00:00", "/")
                .referrer("https://www.example.com/internal")
                .s(),
        ],
        "2026-10-10 12:00:00",
        json!({}),
    );
    assert_eq!(
        Q::col(&db, "channel", "visits", None),
        m(&[
            ("direct", 1),
            ("email", 1),
            ("organic", 1),
            ("paid", 1),
            ("social", 1)
        ])
    );
    assert_eq!(
        Q::col(&db, "channel", "pageviews", None),
        m(&[
            ("direct", 1),
            ("email", 2),
            ("organic", 1),
            ("paid", 1),
            ("social", 1)
        ])
    );
    assert_eq!(Q::col(&db, "campaign", "visits", None), m(&[("oct", 1)]));
    assert_eq!(Q::col(&db, "utm_source", "visits", None), m(&[("news", 1)]));
    assert_eq!(
        Q::col(&db, "utm_medium", "visits", None),
        m(&[("email", 1)])
    );
    assert_eq!(
        Q::col(&db, "source_medium", "visits", None),
        m(&[("news / email", 1)])
    );
    assert_eq!(
        Q::col(&db, "keyword", "visits", None),
        m(&[("web pulse", 1)])
    );
    assert_eq!(
        Q::col(&db, "search_engine", "visits", None),
        m(&[("google", 2)])
    );
    assert_eq!(
        Q::col(&db, "referrer_host", "visits", None),
        m(&[
            ("google.com", 1),
            ("google.de", 1),
            ("mail.google.com", 1),
            ("t.co", 1)
        ])
    );
    assert_eq!(
        Q::col(&db, "page", "visits", None),
        m(&[("/", 5), ("/x", 1)]),
        "query string stripped"
    );
    let ev: String = db
        .query_row("SELECT referrer FROM events WHERE path = '/x'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(ev, "l.facebook.com");
}

#[test]
fn http_and_https_logs_are_merged_by_time_into_one_visit() {
    let mut fr = FakeRoot::create("rs-sess-merge", json!({}));
    fr.write_log(
        PLAIN,
        &[L::new("203.0.113.1", "2026-10-10 10:00:00", "/")
            .bytes(100)
            .proto("HTTP/1.1")
            .s()],
        true,
    );
    fr.write_log(
        SSL,
        &[
            line("203.0.113.1", "2026-10-10 10:02:00", "/b"),
            line("203.0.113.1", "2026-10-10 10:04:00", "/c"),
        ],
        true,
    );
    fr.write_log(
        PLAIN,
        &[L::new("203.0.113.1", "2026-10-10 10:03:00", "/d")
            .bytes(100)
            .proto("HTTP/1.1")
            .s()],
        true,
    );
    fr.set_now(ts("2026-10-10 12:00:00"));
    fr.run("collect").ok("collect");
    let db = fr.db("alice", "example.com");
    assert_eq!(Q::d(&db, "2026-10-10", "visits"), 1);
    assert_eq!(Q::col(&db, "exit_page", "visits", None), m(&[("/c", 1)]));
    assert_eq!(
        Q::col(&db, "protocol", "hits", None),
        m(&[("http", 2), ("https", 2)])
    );
    assert_eq!(
        Q::col(&db, "http_version", "hits", None),
        m(&[("HTTP/1.1", 2), ("HTTP/2.0", 2)])
    );
    assert_eq!(Q::d(&db, "2026-10-10", "duration"), 240);
}

#[test]
fn ignored_lines_head_excluded_paths_and_ips() {
    let mut fr = FakeRoot::create("rs-sess-ignore", json!({"exclude_ips": ["192.0.2.0/24"]}));
    std::fs::create_dir_all(fr.path("var/webpulse/data/alice/user")).unwrap();
    std::fs::write(
        fr.path("var/webpulse/data/alice/user/settings.json"),
        json!({"version": 1, "domains": {"example.com": {"exclude_ips": ["2001:db8::/32"], "exclude_paths": ["/staging/"], "timezone": null}}}).to_string(),
    )
    .unwrap();
    fr.write_log(
        SSL,
        &[
            L::new("203.0.113.1", "2026-10-10 10:00:00", "/")
                .bytes(0)
                .method("HEAD")
                .s(),
            line(
                "203.0.113.1",
                "2026-10-10 10:00:01",
                "/.well-known/acme-challenge/x",
            ),
            line("192.0.2.55", "2026-10-10 10:00:02", "/"),
            line("2001:db8::7", "2026-10-10 10:00:03", "/"),
            line("203.0.113.1", "2026-10-10 10:00:04", "/staging/page"),
            line("203.0.113.1", "2026-10-10 10:00:05", "/kept"),
            "complete garbage".to_string(),
        ],
        true,
    );
    fr.set_now(ts("2026-10-10 12:00:00"));
    let r = fr.run("collect --json").ok("collect");
    assert_eq!((r.i("lines"), r.i("bad"), r.i("ignored")), (7, 1, 5));
    let db = fr.db("alice", "example.com");
    assert_eq!(Q::col(&db, "page", "pageviews", None), m(&[("/kept", 1)]));
    let cp: (i64, i64, i64) = db
        .query_row(
            "SELECT lines, bad_lines, ignored_lines FROM checkpoints",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(cp, (7, 1, 5));
}

#[test]
fn bots_never_create_visits_empty_ua_is_a_bot() {
    let (_fr, db, _) = scenario(
        "bots",
        &[
            L::new("66.249.66.1", "2026-10-10 10:00:00", "/")
                .bytes(100)
                .ua(GOOGLEBOT)
                .s(),
            L::new("66.249.66.1", "2026-10-10 10:00:01", "/a")
                .status(404)
                .bytes(100)
                .ua(GOOGLEBOT)
                .s(),
            L::new("192.0.2.1", "2026-10-10 10:00:02", "/")
                .bytes(100)
                .ua("-")
                .s(),
            L::new("192.0.2.2", "2026-10-10 10:00:03", "/")
                .bytes(100)
                .ua("curl/8.0")
                .s(),
        ],
        "2026-10-10 12:00:00",
        json!({}),
    );
    assert_eq!(
        d(
            &db,
            "2026-10-10",
            &[
                "visits",
                "pageviews",
                "hits",
                "bot_hits",
                "bot_bytes",
                "errors_4xx"
            ]
        ),
        [0, 0, 0, 4, 400, 0]
    );
    assert_eq!(
        Q::col(&db, "bot", "hits", None),
        m(&[("Empty user agent", 1), ("Googlebot", 2), ("curl", 1)])
    );
    assert_eq!(Q::sum(&db, "SELECT SUM(bot_hits) FROM hourly"), 4);
}

#[test]
fn domain_timezone_from_user_settings_decides_the_local_day() {
    let mut fr = FakeRoot::create("rs-sess-tz", json!({}));
    std::fs::create_dir_all(fr.path("var/webpulse/data/alice/user")).unwrap();
    let settings = fr.path("var/webpulse/data/alice/user/settings.json");
    std::fs::write(
        &settings,
        json!({"version": 1, "domains": {"example.com": {"timezone": "America/New_York"}}})
            .to_string(),
    )
    .unwrap();
    fr.write_log(
        SSL,
        &[line("203.0.113.1", "2026-10-11 03:00:00", "/")],
        true,
    );
    fr.set_now(ts("2026-10-11 12:00:00"));
    fr.run("collect").ok("collect");
    let db = fr.db("alice", "example.com");
    assert_eq!(Q::d(&db, "2026-10-10", "visits"), 1);
    assert_eq!(Q::d(&db, "2026-10-11", "visits"), 0);
    let meta = |k: &str| {
        db.query_row("SELECT value FROM meta WHERE key = ?", [k], |r| {
            r.get::<_, String>(0)
        })
        .unwrap()
    };
    assert_eq!(meta("timezone"), "America/New_York");
    assert_eq!(Q::sum(&db, "SELECT hour FROM hourly"), 21);
    std::fs::write(
        &settings,
        json!({"version": 1, "domains": {"example.com": {"timezone": "Asia/Tokyo"}}}).to_string(),
    )
    .unwrap();
    fr.run("collect").ok("collect");
    assert_eq!(meta("tz_changed_at"), ts("2026-10-11 12:00:00").to_string());
}

#[test]
fn store_ip_keeps_the_address_only_in_visits_invalid_utf8_is_cleaned() {
    let (_fr, db, _) = scenario(
        "storeip",
        &[
            L::new("203.0.113.77", "2026-10-10 10:00:00", "/caf%C3%A9/%FF")
                .bytes(10)
                .referrer("https://ref.example.org/\u{fffd}(")
                .s(),
        ],
        "2026-10-10 12:00:00",
        json!({"store_ip": true}),
    );
    let ip: String = db
        .query_row("SELECT ip FROM visits", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ip, "203.0.113.77");
    assert_eq!(
        Q::col(&db, "page", "pageviews", None)
            .keys()
            .next()
            .unwrap(),
        "/caf\u{e9}/?"
    );
    let keys: Vec<Vec<u8>> = db
        .prepare("SELECT CAST(key AS BLOB) FROM dim_daily")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    for k in keys {
        assert!(std::str::from_utf8(&k).is_ok());
    }
}

#[test]
fn out_of_order_lines_within_the_timeout_stay_in_the_visit() {
    let (_fr, db, _) = scenario(
        "order",
        &[
            line("203.0.113.1", "2026-10-10 10:00:10", "/a"),
            line("203.0.113.1", "2026-10-10 10:00:05", "/b"),
            line("203.0.113.1", "2026-10-10 10:02:00", "/c"),
        ],
        "2026-10-10 12:00:00",
        json!({}),
    );
    assert_eq!(
        d(
            &db,
            "2026-10-10",
            &["visits", "pageviews", "bounces", "duration"]
        ),
        [1, 3, 0, 115]
    );
    assert_eq!(Q::col(&db, "exit_page", "visits", None), m(&[("/c", 1)]));
}
