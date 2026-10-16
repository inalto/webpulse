//! Every API action against the seeded fixture (port of tests/cases/ui_api.php):
//! shapes from docs/API.md, totals, validation errors, CSV export, mutations,
//! user settings, admin actions, no-data and schema states.

mod http_common;

use http_common::*;
use serde_json::{Value, json};
use webpulse::reports::dimension;

const SEED: &str = "http-api";

fn q<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut v = vec![("domain", "example.com")];
    v.extend_from_slice(extra);
    v
}

fn sum(v: &Value) -> i64 {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_i64().unwrap_or(0)).sum())
        .unwrap_or(0)
}

#[test]
fn domains() {
    let env = shared(SEED);
    let d = ok(
        &api(&env, "user:alice", &[("a", "domains")], None),
        "domains",
    );
    let list: Vec<&str> = d["domains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["domain"].as_str().unwrap())
        .collect();
    assert_eq!(list, vec!["example.com", "shop.example.com"]);
    let row = &d["domains"][0];
    keys_present(
        &[
            "domain",
            "user",
            "kind",
            "enabled",
            "has_data",
            "first_day",
            "last_day",
            "last_collect_at",
            "visits_today",
            "pageviews_today",
            "timezone",
            "db_bytes",
        ],
        row,
        "domain row",
    );
    assert_eq!(row["user"], "alice");
    assert_eq!(row["has_data"], true);
    assert_eq!(row["timezone"], "Europe/Rome");
    assert!(row["visits_today"].as_i64().unwrap() > 0);
    assert!(row["db_bytes"].as_i64().unwrap() > 0);
    let lc = row["last_collect_at"].as_str().unwrap();
    assert_eq!(lc.len(), 25, "ISO-8601 with offset: {lc}");
    assert_eq!(&lc[10..11], "T");
}

#[test]
fn overview_shape_and_totals() {
    let env = shared(SEED);
    let d = ok(
        &api(
            &env,
            "user:alice",
            &[
                ("a", "overview"),
                ("domain", "example.com"),
                ("range", "30d"),
                ("compare", "1"),
            ],
            None,
        ),
        "overview",
    );
    keys_present(
        &[
            "range", "kpi", "series", "hours", "weekdays", "heatmap", "meta",
        ],
        &d,
        "overview",
    );
    assert_eq!(
        d["range"],
        json!({"from": "2026-09-17", "to": "2026-10-16", "days": 30, "granularity": "day", "prev": {"from": "2026-08-18", "to": "2026-09-16"}})
    );
    for k in [
        "visits",
        "visitors",
        "pageviews",
        "pages_per_visit",
        "avg_duration",
        "bounce_rate",
        "bytes",
        "bot_share",
        "downloads",
        "errors_4xx",
        "errors_5xx",
    ] {
        keys_present(&["value", "prev", "change"], &d["kpi"][k], k);
        assert!(d["kpi"][k]["value"].is_number(), "kpi.{k} numeric");
    }
    let k = &d["kpi"]["visits"];
    let (v, p) = (k["value"].as_i64().unwrap(), k["prev"].as_i64().unwrap());
    assert_eq!(
        k["change"].as_f64().unwrap(),
        webpulse::reports::php::round((v - p) as f64 / p as f64, 4)
    );
    assert_eq!(d["series"]["labels"].as_array().unwrap().len(), 30);
    for s in [
        "visits",
        "visitors",
        "pageviews",
        "bytes",
        "bot_hits",
        "bounces",
        "duration",
        "errors_4xx",
        "errors_5xx",
    ] {
        assert_eq!(d["series"][s].as_array().unwrap().len(), 30, "series.{s}");
    }
    assert_eq!(sum(&d["series"]["visits"]), v, "series sums to the KPI");
    assert_eq!(d["series"]["prev"]["visits"].as_array().unwrap().len(), 30);
    assert_eq!(d["hours"]["visits"].as_array().unwrap().len(), 24);
    assert_eq!(d["weekdays"]["visits"].as_array().unwrap().len(), 7);
    assert_eq!(d["heatmap"].as_array().unwrap().len(), 7);
    assert_eq!(d["heatmap"][0].as_array().unwrap().len(), 24);
    assert_eq!(
        sum(&d["hours"]["visits"]),
        v,
        "hourly visits = daily visits"
    );
    assert_eq!(
        d["heatmap"]
            .as_array()
            .unwrap()
            .iter()
            .map(sum)
            .sum::<i64>(),
        v
    );
    assert_eq!(sum(&d["weekdays"]["visits"]), v);
    keys_present(
        &[
            "domain",
            "timezone",
            "first_day",
            "last_day",
            "last_collect_at",
            "tz_changed_at",
            "raw_retention_days",
        ],
        &d["meta"],
        "meta",
    );
    assert_eq!(d["meta"]["raw_retention_days"], 30);
    // the PHP fixture: 30 days ending 2026-10-16 of example.com
    let d2 = ok(
        &api(&env, "user:alice", &q(&[("a", "overview")]), None),
        "overview",
    );
    assert!(d2["series"].get("prev").is_none());
    assert_eq!(
        d2["kpi"]["visits"], d["kpi"]["visits"],
        "default range is 30d"
    );
}

#[test]
fn overview_hour_granularity_and_week_bucketing() {
    let env = shared(SEED);
    let d = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "overview"), ("range", "today")]),
            None,
        ),
        "today",
    );
    assert_eq!(d["range"]["granularity"], "hour");
    assert_eq!(d["series"]["labels"].as_array().unwrap().len(), 24);
    assert_eq!(
        sum(&d["series"]["visits"]),
        d["kpi"]["visits"]["value"].as_i64().unwrap()
    );
    assert!(
        d["series"]["bounces"].is_null(),
        "hourly table has no bounces"
    );
    let w = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "overview"), ("range", "12m")]),
            None,
        ),
        "12m",
    );
    assert_eq!(w["range"]["granularity"], "week");
    assert_eq!(
        sum(&w["series"]["pageviews"]),
        w["kpi"]["pageviews"]["value"].as_i64().unwrap()
    );
}

#[test]
fn report_every_dimension() {
    let env = shared(SEED);
    for dim in dimension::ALL {
        let d = ok(
            &api(
                &env,
                "user:alice",
                &q(&[("a", "report"), ("name", dim), ("limit", "5")]),
                None,
            ),
            dim,
        );
        keys_present(&["name", "total", "rows", "has_more"], &d, dim);
        assert_eq!(d["name"], dim);
        keys_present(
            &["visits", "pageviews", "hits", "bytes", "rows"],
            &d["total"],
            dim,
        );
        assert!(d["rows"].as_array().unwrap().len() <= 5);
        for row in d["rows"].as_array().unwrap() {
            keys_present(
                &[
                    "key",
                    "visits",
                    "pageviews",
                    "hits",
                    "bytes",
                    "bounces",
                    "duration",
                    "share",
                ],
                row,
                dim,
            );
            assert!(row["key"].is_string() && row["visits"].is_i64());
        }
    }
}

#[test]
fn report_page_exit_rate_compare_sort_q_paging() {
    let env = shared(SEED);
    let d = ok(
        &api(
            &env,
            "user:alice",
            &q(&[
                ("a", "report"),
                ("name", "page"),
                ("compare", "1"),
                ("limit", "3"),
            ]),
            None,
        ),
        "page",
    );
    assert_eq!(d["rows"][0]["key"], "/");
    assert_eq!(d["has_more"], true);
    assert!(
        d["rows"][0].get("exit_rate").is_some() && d["rows"][0]["prev"].get("pageviews").is_some()
    );
    assert!(d["rows"][0]["pageviews"].as_i64() >= d["rows"][1]["pageviews"].as_i64());
    let share = webpulse::reports::php::round(
        d["rows"][0]["pageviews"].as_f64().unwrap() / d["total"]["pageviews"].as_f64().unwrap(),
        4,
    );
    assert_eq!(d["rows"][0]["share"].as_f64().unwrap(), share);
    let asc = ok(
        &api(
            &env,
            "user:alice",
            &q(&[
                ("a", "report"),
                ("name", "page"),
                ("sort", "pageviews"),
                ("limit", "2"),
            ]),
            None,
        ),
        "asc",
    );
    assert!(asc["rows"][0]["pageviews"].as_i64() <= asc["rows"][1]["pageviews"].as_i64());
    let f = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "page"), ("q", "BLOG")]),
            None,
        ),
        "q",
    );
    let rows = f["rows"].as_array().unwrap();
    assert!(rows.len() >= 2);
    for row in rows {
        assert!(row["key"].as_str().unwrap().contains("blog"));
    }
    assert_eq!(rows.len() as i64, f["total"]["rows"].as_i64().unwrap());
    let like = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "page"), ("q", "%")]),
            None,
        ),
        "like",
    );
    assert_eq!(
        like["rows"].as_array().unwrap().len(),
        0,
        "LIKE wildcards are escaped"
    );
    let p1 = ok(
        &api(
            &env,
            "user:alice",
            &q(&[
                ("a", "report"),
                ("name", "page"),
                ("limit", "2"),
                ("offset", "2"),
            ]),
            None,
        ),
        "p1",
    );
    let all = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "page"), ("limit", "4")]),
            None,
        ),
        "all",
    );
    let keys = |v: &Value| {
        v["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["key"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&all)[2..4].to_vec(), keys(&p1), "offset");
    let big = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "page"), ("limit", "100000")]),
            None,
        ),
        "big",
    );
    assert_eq!(
        big["rows"].as_array().unwrap().len(),
        14,
        "limit is capped, not rejected"
    );
}

#[test]
fn report_labels_and_bucket_order() {
    let env = shared(SEED);
    let c = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "country")]),
            None,
        ),
        "country",
    );
    assert_eq!(
        (c["rows"][0]["key"].clone(), c["rows"][0]["label"].clone()),
        (json!("it"), json!("Italy"))
    );
    assert!(
        c["rows"][0].get("bounce_rate").is_some() && c["rows"][0].get("avg_duration").is_some()
    );
    let s = ok(
        &api(
            &env,
            "user:alice",
            &q(&[
                ("a", "report"),
                ("name", "status"),
                ("sort", "key"),
                ("limit", "50"),
            ]),
            None,
        ),
        "status",
    );
    let row404 = s["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "404")
        .unwrap()
        .clone();
    assert_eq!(row404["label"], "404 Not Found");
    let b = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "visit_duration"), ("limit", "20")]),
            None,
        ),
        "vd",
    );
    let keys: Vec<&str> = b["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, dimension::DURATION_BUCKETS.to_vec(), "bucket order");
    let p = ok(
        &api(
            &env,
            "user:alice",
            &q(&[
                ("a", "report"),
                ("name", "pages_per_visit"),
                ("limit", "20"),
            ]),
            None,
        ),
        "ppv",
    );
    let keys: Vec<&str> = p["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, dimension::PAGES_BUCKETS.to_vec());
    assert_eq!(p["rows"][5]["label"], "6–10 pages");
}

#[test]
fn report_series_top7_plus_other() {
    let env = shared(SEED);
    let d = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "country"), ("series", "1")]),
            None,
        ),
        "series",
    );
    let se = &d["series"];
    assert_eq!(se["labels"].as_array().unwrap().len(), 30);
    assert_eq!(se["keys"].as_array().unwrap().len(), 8, "7 keys + (other)");
    assert_eq!(se["keys"][7], "(other)");
    assert_eq!(se["values"].as_array().unwrap().len(), 8);
    assert_eq!(se["metric"], "visits");
    let mut total = 0;
    for v in se["values"].as_array().unwrap() {
        assert_eq!(v.as_array().unwrap().len(), 30);
        total += sum(v);
    }
    assert_eq!(
        total,
        d["total"]["visits"].as_i64().unwrap(),
        "series covers every visit"
    );
}

#[test]
fn realtime() {
    let env = shared(SEED);
    let d = ok(
        &api(&env, "user:alice", &q(&[("a", "realtime")]), None),
        "realtime",
    );
    keys_present(
        &[
            "active_visits",
            "pageviews_30m",
            "per_minute",
            "recent",
            "last_event_at",
            "last_collect_at",
        ],
        &d,
        "realtime",
    );
    assert_eq!(d["active_visits"], 6);
    assert_eq!(d["per_minute"].as_array().unwrap().len(), 30);
    assert_eq!(sum(&d["per_minute"]), d["pageviews_30m"].as_i64().unwrap());
    let recent = d["recent"].as_array().unwrap();
    assert!(!recent.is_empty());
    keys_present(
        &[
            "ts",
            "path",
            "status",
            "type",
            "country",
            "device",
            "browser",
            "referrer",
            "visit_pageviews",
        ],
        &recent[0],
        "recent",
    );
    assert!(
        recent.iter().all(|e| e["type"] != "bot"),
        "bots are not listed"
    );
}

#[test]
fn visits_log_with_cursor_paging() {
    let env = shared(SEED);
    let d = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "visits_log"), ("range", "7d"), ("limit", "10")]),
            None,
        ),
        "visits_log",
    );
    let visits = d["visits"].as_array().unwrap();
    assert_eq!(visits.len(), 10);
    let v = &visits[0];
    keys_present(
        &[
            "visit_id",
            "start",
            "end",
            "duration",
            "pageviews",
            "bytes",
            "entry_page",
            "exit_page",
            "channel",
            "referrer_host",
            "search_engine",
            "keyword",
            "campaign",
            "country",
            "city",
            "browser",
            "os",
            "device",
            "actions",
        ],
        v,
        "visit",
    );
    assert_eq!(v["live"], true, "open visits come first");
    assert!(v.get("ip").is_none(), "no IP unless stored");
    keys_present(
        &["ts", "type", "path", "status", "bytes"],
        &v["actions"][0],
        "action",
    );
    assert!(d["next_before"].is_i64());
    let nb = d["next_before"].to_string();
    let nbi = d["next_before_id"].to_string();
    let n = ok(
        &api(
            &env,
            "user:alice",
            &q(&[
                ("a", "visits_log"),
                ("range", "7d"),
                ("limit", "10"),
                ("before", &nb),
                ("before_id", &nbi),
            ]),
            None,
        ),
        "page 2",
    );
    let mut ids: Vec<i64> = visits
        .iter()
        .chain(n["visits"].as_array().unwrap())
        .map(|x| x["visit_id"].as_i64().unwrap())
        .collect();
    let count = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count, "pages do not overlap");
    assert!(
        n["visits"][0]["start"].as_str().unwrap() <= visits[9]["start"].as_str().unwrap(),
        "older on the next page"
    );
    let cap = ok(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "visits_log"), ("limit", "1000")]),
            None,
        ),
        "cap",
    );
    assert!(cap["visits"].as_array().unwrap().len() <= 100);
    assert_eq!(cap["retention_days"], 30);
}

#[test]
fn health() {
    let env = shared(SEED);
    let d = ok(
        &api(&env, "user:alice", &q(&[("a", "health")]), None),
        "health",
    );
    keys_present(
        &[
            "status",
            "errors_series",
            "not_found",
            "server_error",
            "file_type",
            "static_file",
            "download",
            "protocol",
            "http_version",
            "method",
            "peak",
            "peak_hour",
        ],
        &d,
        "health",
    );
    keys_present(
        &["labels", "errors_4xx", "errors_5xx", "hits", "bot_hits"],
        &d["errors_series"],
        "errors_series",
    );
    let old = d["not_found"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "/old-pricing")
        .unwrap()
        .clone();
    let refs: Vec<&str> = old["referrers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        refs,
        vec!["bing.com", "oldforum.example.net"],
        "404 referrers"
    );
    keys_present(&["day", "hour", "hits"], &d["peak"], "peak");
    assert_eq!(
        d["static_file"][0]["key"], "/assets/hero-2400.jpg",
        "static files by bandwidth"
    );
}

#[test]
fn export_csv() {
    let env = shared(SEED);
    let r = api(
        &env,
        "user:alice",
        &q(&[
            ("a", "export"),
            ("name", "page"),
            ("format", "csv"),
            ("range", "30d"),
        ]),
        None,
    );
    assert_eq!(r.status, 200);
    assert_eq!(r.response.content_type, "text/csv; charset=utf-8");
    let disp = r
        .response
        .headers
        .iter()
        .find(|(k, _)| k == "Content-Disposition")
        .map(|(_, v)| v.clone());
    assert_eq!(
        disp.as_deref(),
        Some("attachment; filename=\"example.com-page-2026-09-17_2026-10-16.csv\"")
    );
    let lines: Vec<&str> = r.body.split('\n').filter(|l| !l.is_empty()).collect();
    assert_eq!(lines[0], "key,pageviews,visits,hits,bytes,exit_rate,share");
    assert_eq!(lines.len(), 15, "header + every row (no limit)");
    assert!(lines[1].starts_with("/,"));
    let c = api(
        &env,
        "user:alice",
        &q(&[("a", "export"), ("name", "country")]),
        None,
    );
    assert!(c.body.contains("key,label,visits"));
    assert!(c.body.contains("\"United States\""));
    assert_eq!(webpulse::reports::export::safe("=cmd()"), "'=cmd()");
    assert_eq!(webpulse::reports::export::safe("/ok"), "/ok");
    let bad = api(
        &env,
        "user:alice",
        &q(&[("a", "export"), ("name", "page"), ("format", "xlsx")]),
        None,
    );
    assert_eq!(bad.status, 400);
}

#[test]
fn validation_errors() {
    let env = shared(SEED);
    err(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "nope")]),
            None,
        ),
        400,
        "bad_request",
        "bad dim",
    );
    err(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "page; DROP TABLE daily")]),
            None,
        ),
        400,
        "bad_request",
        "sql in dim",
    );
    err(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "page"), ("sort", "key DESC; --")]),
            None,
        ),
        400,
        "bad_request",
        "bad sort",
    );
    err(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "report"), ("name", "page"), ("limit", "ten")]),
            None,
        ),
        400,
        "bad_request",
        "bad limit",
    );
    err(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "overview"), ("range", "forever")]),
            None,
        ),
        400,
        "bad_request",
        "bad range",
    );
    err(
        &api(
            &env,
            "user:alice",
            &q(&[
                ("a", "overview"),
                ("from", "2026-13-01"),
                ("to", "2026-12-01"),
            ]),
            None,
        ),
        400,
        "bad_request",
        "bad date",
    );
    err(
        &api(
            &env,
            "user:alice",
            &q(&[("a", "visits_log"), ("before", "-5")]),
            None,
        ),
        400,
        "bad_request",
        "bad cursor",
    );
    err(
        &api(&env, "user:alice", &[("a", "overview")], None),
        400,
        "bad_request",
        "missing domain",
    );
    err(
        &api(&env, "user:alice", &[("a", "teleport")], None),
        400,
        "bad_request",
        "unknown action",
    );
    err(
        &api(&env, "user:alice", &[], None),
        400,
        "bad_request",
        "no action",
    );
    err(
        &api_with(
            &env,
            "user:alice",
            &q(&[("a", "overview")]),
            None,
            "POST",
            None,
        ),
        405,
        "method_not_allowed",
        "POST on a read",
    );
}

#[test]
fn mutations_need_post_and_header() {
    let env = fresh(SEED, "http-api-mut");
    err(
        &api_with(
            &env,
            "admin",
            &[("a", "admin.set_account")],
            None,
            "GET",
            None,
        ),
        405,
        "method_not_allowed",
        "GET",
    );
    let body = json!({"user": "bob", "enabled": false})
        .to_string()
        .into_bytes();
    err(
        &api_with(
            &env,
            "admin",
            &[("a", "admin.set_account")],
            Some(body),
            "POST",
            Some(vec![]),
        ),
        403,
        "forbidden",
        "no header",
    );
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.set_account")],
            Some(json!({"user": "bob", "enabled": "yes"})),
        ),
        400,
        "bad_request",
        "bad enabled",
    );
    let r = api(
        &env,
        "admin",
        &[("a", "admin.set_account")],
        Some(json!({"user": "bob", "enabled": false})),
    );
    let d = ok(&r, "set_account");
    assert_eq!(d["account"]["enabled"], false);
    assert_eq!(d["account"]["enabled_override"], false);
}

#[test]
fn user_settings_read_validate_atomic_save() {
    let env = fresh(SEED, "http-api-settings");
    let d = ok(
        &api(&env, "user:alice", &[("a", "user_settings")], None),
        "user_settings",
    );
    assert_eq!(d["remote_addr"], "203.0.113.9");
    assert_eq!(d["domains"], json!(["example.com", "shop.example.com"]));
    assert_eq!(
        d["settings"],
        json!({"version": 1, "domains": [], "ui": {"default_range": "30d", "dark": "auto"}})
    );
    let new = json!({"domains": {"example.com": {"timezone": "Europe/Berlin", "exclude_ips": ["203.0.113.0/24"], "exclude_paths": ["/staging/"]}},
        "ui": {"default_range": "7d", "dark": "dark"}});
    let s = ok(
        &api(
            &env,
            "user:alice",
            &[("a", "save_user_settings")],
            Some(json!({"settings": new})),
        ),
        "save",
    );
    assert_eq!(s["saved"], true);
    let file = env.root.join("var/webpulse/data/alice/user/settings.json");
    let disk: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(disk["domains"]["example.com"]["timezone"], "Europe/Berlin");
    let leftovers: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".settings-"))
        .collect();
    assert!(leftovers.is_empty(), "no temp files left");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.starts_with("{\n    \"version\": 1,\n") && text.ends_with("}\n"),
        "PHP pretty print"
    );
    let back = ok(
        &api(&env, "user:alice", &[("a", "user_settings")], None),
        "back",
    );
    assert_eq!(
        back["settings"]["domains"]["example.com"]["exclude_ips"],
        json!(["203.0.113.0/24"])
    );
    err(
        &api(
            &env,
            "user:alice",
            &[("a", "save_user_settings")],
            Some(json!({"domains": {"example.com": {"timezone": "Mars/Olympus"}}})),
        ),
        400,
        "bad_request",
        "bad tz",
    );
    err(
        &api(
            &env,
            "user:alice",
            &[("a", "save_user_settings")],
            Some(json!({"domains": {"example.com": {"exclude_ips": ["not-an-ip"]}}})),
        ),
        400,
        "bad_request",
        "bad ip",
    );
    err(
        &api(
            &env,
            "user:alice",
            &[("a", "save_user_settings")],
            Some(json!({"domains": {"blog.net": []}})),
        ),
        400,
        "bad_request",
        "foreign domain",
    );
    err(
        &api(
            &env,
            "admin",
            &[("a", "save_user_settings")],
            Some(json!({"domains": []})),
        ),
        403,
        "forbidden",
        "admin cannot save",
    );
    err(
        &api(&env, "admin", &[("a", "user_settings")], None),
        403,
        "forbidden",
        "admin cannot read",
    );
    err(
        &api_with(
            &env,
            "user:alice",
            &[("a", "save_user_settings")],
            None,
            "POST",
            None,
        ),
        400,
        "bad_request",
        "empty body",
    );
    // the page bootstrap picks the saved UI defaults up
    let a = app(&env);
    let ctx = webpulse::http::Ctx::new(&a);
    let boot = webpulse::http::page::bootstrap(
        &ctx,
        &principal("user:alice", &env.root),
        &webpulse::http::page::PageOptions::new("api.live.php", true, None),
    );
    assert_eq!(boot["defaults"], json!({"range": "7d", "dark": "dark"}));
}

#[test]
fn admin_actions() {
    let env = fresh(SEED, "http-api-admin");
    let o = ok(
        &api(&env, "admin", &[("a", "admin.overview")], None),
        "admin.overview",
    );
    keys_present(
        &[
            "totals",
            "top_domains",
            "disk",
            "jobs",
            "collector",
            "bot_share_30d",
        ],
        &o,
        "admin.overview",
    );
    keys_present(
        &[
            "last_collect_at",
            "lag_seconds",
            "cron_installed",
            "hook_installed",
            "geoip",
        ],
        &o["collector"],
        "collector",
    );
    let top: Vec<&str> = o["top_domains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["domain"].as_str().unwrap())
        .collect();
    assert_eq!(top, vec!["example.com", "blog.net", "shop.example.com"]);
    // `top` sizes the top-domains list (the server report asks for all of them)
    for (n, want) in [("1", 1), ("0", 1), ("500", 3), ("x", 1)] {
        let o = ok(
            &api(&env, "admin", &[("a", "admin.overview"), ("top", n)], None),
            "admin.overview top",
        );
        assert_eq!(o["top_domains"].as_array().unwrap().len(), want, "top={n}");
    }
    let bs = o["bot_share_30d"].as_f64().unwrap();
    assert!(bs > 0.2 && bs < 0.6, "bot share plausible: {bs}");
    assert_eq!(o["collector"]["lag_seconds"], 200);
    let a = ok(
        &api(&env, "admin", &[("a", "admin.accounts")], None),
        "admin.accounts",
    );
    let users: Vec<&str> = a["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["user"].as_str().unwrap())
        .collect();
    assert_eq!(users, vec!["alice", "bob", "carol", "dave"]);
    keys_present(
        &["user", "owner", "enabled", "enabled_override", "domains"],
        &a["accounts"][0],
        "account",
    );
    keys_present(
        &[
            "domain",
            "kind",
            "enabled",
            "has_data",
            "db_bytes",
            "visits_30d",
            "last_collect_at",
        ],
        &a["accounts"][0]["domains"][0],
        "account domain",
    );
    assert_eq!(a["accounts"][3]["enabled"], false, "dave is disabled");
    let f = ok(
        &api(
            &env,
            "admin",
            &[("a", "admin.accounts"), ("q", "blog")],
            None,
        ),
        "q",
    );
    assert_eq!(
        f["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["user"].clone())
            .collect::<Vec<_>>(),
        vec![json!("bob")]
    );
    let sd = ok(
        &api(
            &env,
            "admin",
            &[("a", "admin.set_domain")],
            Some(json!({"user": "alice", "domain": "shop.example.com", "enabled": false})),
        ),
        "set_domain",
    );
    assert_eq!(sd["domain"]["enabled"], false);
    let s = ok(
        &api(&env, "admin", &[("a", "admin.settings")], None),
        "settings",
    );
    keys_present(
        &[
            "default_enabled",
            "raw_retention_days",
            "max_rows_per_dim",
            "session_timeout_min",
            "store_ip",
            "exclude_ips",
            "exclude_paths",
            "geoip.enabled",
            "timezone",
        ],
        &s["settings"],
        "settings",
    );
    let saved = ok(
        &api(
            &env,
            "admin",
            &[("a", "admin.save_settings")],
            Some(
                json!({"settings": {"raw_retention_days": 60, "exclude_ips": ["10.0.0.0/8"], "geoip": {"enabled": true}}}),
            ),
        ),
        "save_settings",
    );
    assert_eq!(saved["settings"]["raw_retention_days"], 60);
    assert_eq!(saved["settings"]["geoip.enabled"], true);
    assert!(
        saved["overridden"]
            .as_array()
            .unwrap()
            .contains(&json!("raw_retention_days"))
    );
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.save_settings")],
            Some(json!({"settings": {"raw_retention_days": 0}})),
        ),
        400,
        "bad_request",
        "out of range",
    );
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.save_settings")],
            Some(json!({"settings": {"paths.data_dir": "/tmp"}})),
        ),
        400,
        "bad_request",
        "not editable",
    );
    let j = ok(&api(&env, "admin", &[("a", "admin.jobs")], None), "jobs");
    assert_eq!(j["jobs"].as_array().unwrap().len(), 13);
    keys_present(
        &[
            "id",
            "type",
            "scope",
            "started_at",
            "finished_at",
            "status",
            "lines",
            "started_at_iso",
            "finished_at_iso",
        ],
        &j["jobs"][0],
        "job",
    );
}

#[test]
fn admin_run_spawns_a_detached_job() {
    let env = fresh(SEED, "http-api-run");
    // run the spawned job with this test binary's webpulse: current_exe of the
    // test process is the test harness, so call the CLI through the binary.
    let r = run(
        &["cgi", "--whm", "--api"],
        &[
            ("WEBPULSE_CONFIG", env.config.display().to_string()),
            ("WEBPULSE_CGI_TESTING", "1".into()),
            ("REQUEST_METHOD", "POST".into()),
            ("QUERY_STRING", "a=admin.run".into()),
            ("CONTENT_LENGTH", "17".into()),
            ("HTTP_X_WEBPULSE_REQUEST", "1".into()),
            ("REMOTE_USER", "root".into()),
        ],
        br#"{"type":"rollup"}"#,
    );
    let c = parse_cgi(&r.stdout);
    assert_eq!(c.status, 200, "{}", c.text());
    let d = c.json();
    let id = d["data"]["job_id"].as_i64().unwrap();
    assert!(id > 13);
    assert_eq!(
        d["data"].as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["job_id"],
        "only the job id is returned"
    );
    // the detached `webpulse rollup --job N` finishes the job row
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut status = String::from("running");
    while std::time::Instant::now() < deadline {
        let jobs = ok(&api(&env, "admin", &[("a", "admin.jobs")], None), "jobs");
        status = jobs["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["id"] == id)
            .map(|j| j["status"].as_str().unwrap().to_string())
            .unwrap_or("missing".into());
        if status != "running" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(
        ["ok", "skipped"].contains(&status.as_str()),
        "job finished (status {status})"
    );
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.run")],
            Some(json!({"type": "rm -rf"})),
        ),
        400,
        "bad_request",
        "bad type",
    );
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.run")],
            Some(json!({"type": "backfill"})),
        ),
        400,
        "bad_request",
        "backfill needs user",
    );
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.run")],
            Some(json!({"type": "collect", "user": "../etc"})),
        ),
        404,
        "not_found",
        "bad user",
    );
}

#[test]
fn admin_purge_needs_delete_confirmation() {
    let env = fresh(SEED, "http-api-purge");
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.purge")],
            Some(json!({"user": "alice", "domain": "shop.example.com"})),
        ),
        400,
        "bad_request",
        "no confirm",
    );
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.purge")],
            Some(json!({"user": "alice", "domain": "shop.example.com", "confirm": "delete"})),
        ),
        400,
        "bad_request",
        "wrong case",
    );
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.purge")],
            Some(json!({"user": "alice", "domain": "../bob/blog.net", "confirm": "DELETE"})),
        ),
        400,
        "bad_request",
        "traversal",
    );
    let d = ok(
        &api(
            &env,
            "admin",
            &[("a", "admin.purge")],
            Some(json!({"user": "alice", "domain": "shop.example.com", "confirm": "DELETE"})),
        ),
        "purge",
    );
    assert_eq!(d["deleted"], json!(["shop.example.com"]));
    assert!(
        !env.root
            .join("var/webpulse/data/alice/shop.example.com.db")
            .exists()
    );
    assert!(
        env.root
            .join("var/webpulse/data/alice/example.com.db")
            .exists(),
        "other domain kept"
    );
    let d = ok(
        &api(
            &env,
            "admin",
            &[("a", "admin.purge")],
            Some(json!({"user": "bob", "confirm": "DELETE"})),
        ),
        "purge bob",
    );
    assert_eq!(d["deleted"], json!(["blog.net"]));
}

#[test]
fn no_data_and_schema_states() {
    let env = fresh(SEED, "http-api-states");
    err(
        &api(
            &env,
            "admin",
            &[
                ("a", "overview"),
                ("domain", "carol.org"),
                ("user", "carol"),
            ],
            None,
        ),
        404,
        "no_data",
        "registered without data",
    );
    let c = rusqlite::Connection::open(env.root.join("var/webpulse/data/alice/example.com.db"))
        .unwrap();
    c.execute(
        "UPDATE meta SET value = '999' WHERE key = 'schema_version'",
        [],
    )
    .unwrap();
    drop(c);
    err(
        &api(&env, "user:alice", &q(&[("a", "overview")]), None),
        503,
        "schema_too_new",
        "newer schema",
    );
}

#[test]
fn json_numbers_keep_php_types() {
    let env = shared(SEED);
    let r = api(
        &env,
        "user:alice",
        &q(&[("a", "report"), ("name", "page"), ("limit", "3")]),
        None,
    );
    // shares are floats ("0.2"), counts ints; PRESERVE_ZERO_FRACTION keeps "1.0"
    assert!(r.body.contains("\"share\":0."), "{}", &r.body[..300]);
    let o = api(
        &env,
        "user:alice",
        &q(&[("a", "overview"), ("range", "today")]),
        None,
    );
    assert!(o.body.contains("\"bounces\":null"));
}

#[test]
fn admin_accounts_offset_is_clamped_and_cannot_overflow() {
    let env = shared(SEED);
    for off in ["9223372036854775807", "99999999999999999999", "1000001"] {
        let a = ok(
            &api(
                &env,
                "admin",
                &[("a", "admin.accounts"), ("offset", off), ("limit", "500")],
                None,
            ),
            "admin.accounts",
        );
        assert_eq!(a["accounts"], json!([]), "offset {off}");
        assert_eq!(a["has_more"], json!(false), "offset {off}");
    }
}

#[test]
fn reports_survive_counters_that_overflow_an_sql_sum() {
    let env = fresh(SEED, "http-api-sum-overflow");
    let c = rusqlite::Connection::open(env.root.join("var/webpulse/data/alice/example.com.db"))
        .unwrap();
    c.execute(
        "UPDATE daily SET bytes = 9223372036854775000, hits = 9223372036854775000",
        [],
    )
    .unwrap();
    c.execute(
        "UPDATE dim_daily SET bytes = 9223372036854775000, hits = 9223372036854775000",
        [],
    )
    .unwrap();
    drop(c);
    let mut queries = vec![
        q(&[("a", "overview"), ("range", "30d")]),
        q(&[("a", "health")]),
    ];
    for dim in dimension::ALL {
        queries.push(q(&[("a", "report"), ("name", dim), ("range", "30d")]));
    }
    for query in queries {
        let r = api(&env, "user:alice", &query, None);
        assert_eq!(r.status, 200, "{query:?}: {}", r.body);
    }
}
