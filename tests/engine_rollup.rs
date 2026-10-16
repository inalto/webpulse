//! Port of tests/cases/engine_rollup.php.

mod common;

use common::{FakeRoot, L, Q, ts};
use serde_json::json;

#[test]
fn rollup_caps_dimensions_purges_raw_data_closes_stale_visits_refreshes_summary() {
    let mut fr = FakeRoot::create(
        "rs-rollup",
        json!({"max_rows_per_dim": 10, "raw_retention_days": 3}),
    );
    let mut lines = Vec::new();
    for i in 0..30 {
        lines.push(
            L::new(
                &format!("203.0.113.{}", i + 1),
                &format!("2026-10-05 10:00:{i:02}"),
                &format!("/page-{i}"),
            )
            .bytes(100 + i)
            .referrer(&format!("https://site{i}.example.org/"))
            .s(),
        );
    }
    for i in 0..5 {
        lines.push(
            L::new(
                &format!("198.51.100.{}", i + 1),
                "2026-10-05 11:00:00",
                "/page-0",
            )
            .s(),
        );
    }
    lines.push(L::new("192.0.2.1", "2026-10-10 11:50:00", "/today").s());
    fr.write_log(
        "etc/apache2/logs/domlogs/alice/example.com-ssl_log",
        &lines,
        true,
    );
    fr.set_now(ts("2026-10-10 11:55:00"));
    fr.run("collect").ok("collect");
    let db = fr.db("alice", "example.com");
    assert_eq!(Q::dim(&db, "page", Some("2026-10-05")).len(), 30);
    assert_eq!(Q::count(&db, "open_visits", "1=1"), 1);
    let before = Q::totals(&db);
    let pv_before: i64 = Q::col(&db, "page", "pageviews", Some("2026-10-05"))
        .values()
        .sum();

    fr.set_now(ts("2026-10-10 13:00:00"));
    let r = fr.run("rollup --json").ok("rollup");
    assert_eq!(r.i("closed"), 1);
    assert!(r.i("folded") > 0);
    let pages = Q::dim(&db, "page", Some("2026-10-05"));
    assert_eq!(pages.len(), 10);
    assert!(pages.contains_key("(other)"));
    assert_eq!(pages["/page-0"][1], 6, "top row kept");
    assert_eq!(
        pages.values().map(|v| v[1]).sum::<i64>(),
        pv_before,
        "sums preserved"
    );
    assert_eq!(Q::dim(&db, "referrer_host", Some("2026-10-05")).len(), 10);
    assert_eq!(
        Q::sum(
            &db,
            "SELECT SUM(visits) FROM dim_daily WHERE dim = 'referrer_host'"
        ),
        30,
        "referrer visits preserved"
    );
    assert_eq!(
        Q::dim(&db, "page", Some("2026-10-10")).len(),
        1,
        "unfinished day untouched"
    );
    assert_eq!(before, Q::totals(&db));
    assert_eq!(Q::count(&db, "visits", "day < '2026-10-07'"), 0);
    assert_eq!(
        Q::count(
            &db,
            "events",
            &format!("ts < {}", ts("2026-10-07 00:00:00"))
        ),
        0
    );
    assert_eq!(
        Q::count(&db, "visits", "1=1"),
        1,
        "the closed visit of today"
    );
    assert_eq!(Q::count(&db, "open_visits", "1=1"), 0);
    assert_eq!(Q::count(&db, "visitor_day", "day < '2026-10-08'"), 0);
    let last: String = db
        .query_row(
            "SELECT value FROM meta WHERE key = 'last_rollup_at'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(last, ts("2026-10-10 13:00:00").to_string());
    let app = fr.app();
    let sum = app
        .registry()
        .unwrap()
        .summaries(Some(&["alice".to_string()]))
        .unwrap();
    assert_eq!(sum.len(), 1);
    assert_eq!(sum[0]["visits_today"], 1);
    assert_eq!(sum[0]["visits_7d"], 36);
    assert!(sum[0]["db_bytes"].as_i64().unwrap() > 0);
    // idempotent
    assert_eq!(fr.run("rollup --json").ok("rollup").i("folded"), 0);
    assert_eq!(Q::dim(&db, "page", Some("2026-10-05")).len(), 10);
    let n: i64 = app
        .registry()
        .unwrap()
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM salts WHERE day < '2026-10-07'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}
