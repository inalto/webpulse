//! Every API action against the seeded fixture (port of tests/cases/ui_api.php):
//! shapes from docs/API.md, totals, validation errors, CSV export, mutations,
//! user settings, admin actions, no-data and schema states.

mod common;
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

// ------------------------------------------------------------------ updates

/// A tiny HTTP server answering every request with `body` (the fake GitHub
/// `releases/latest`); returns its URL and the number of requests served.
fn fake_github(body: &str) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/repos/inalto/webpulse/releases/latest",
        listener.local_addr().unwrap().port()
    );
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (h, body) = (hits.clone(), body.to_string());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let req = String::from_utf8_lossy(&buf).to_ascii_lowercase();
            assert!(req.contains("user-agent: webpulse/"), "User-Agent: {req}");
            h.fetch_add(1, Ordering::SeqCst);
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        }
    });
    (url, hits)
}

fn with_update_url(env: &Env, url: &str) {
    let mut cfg: Value =
        serde_json::from_str(&std::fs::read_to_string(&env.config).unwrap()).unwrap();
    cfg["update"] = json!({"url": url});
    std::fs::write(&env.config, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
}

fn registry_db(env: &Env) -> rusqlite::Connection {
    rusqlite::Connection::open(env.root.join("var/webpulse/webpulse.db")).unwrap()
}

/// Moves the stored `checked_at` back by `secs`.
fn age_update_status(env: &Env, secs: i64) {
    let db = registry_db(env);
    let raw: String = db
        .query_row(
            "SELECT value FROM settings WHERE key = 'update_status'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut v: Value = serde_json::from_str(&raw).unwrap();
    v["checked_at"] = json!(v["checked_at"].as_i64().unwrap() - secs);
    db.execute(
        "UPDATE settings SET value = ? WHERE key = 'update_status'",
        [v.to_string()],
    )
    .unwrap();
}

const RELEASE: &str = r#"{"tag_name":"v9.9.9","html_url":"https://github.com/inalto/webpulse/releases/tag/v9.9.9","published_at":"2026-10-17T08:00:00Z","body":"Added: everything\r\n","assets":[]}"#;

#[test]
fn update_check_cli_cache_admin_update_and_throttle() {
    use std::sync::atomic::Ordering;
    let env = fresh(SEED, "http-api-update");
    let (url, hits) = fake_github(RELEASE);
    with_update_url(&env, &url);
    let cfg = [("WEBPULSE_CONFIG", env.config.display().to_string())];

    let r = run(&["update-check", "--force", "--json"], &cfg, b"");
    assert_eq!(r.code, 0, "{}{}", r.out(), r.stderr);
    let v: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["latest"], json!("9.9.9"));
    assert_eq!(v["tag"], json!("v9.9.9"));
    assert_eq!(v["current"], json!(env!("CARGO_PKG_VERSION")));
    assert_eq!(v["is_newer"], json!(true));
    assert_eq!(v["notes"], json!("Added: everything"));
    assert_eq!(v["checked_at"], json!(NOW));
    assert_eq!(v["error"], Value::Null);
    assert_eq!(v["checked"], json!(true));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    // cached for 20 h
    let r = run(&["update-check", "--json"], &cfg, b"");
    let v: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["checked"], json!(false));
    assert_eq!(hits.load(Ordering::SeqCst), 1, "no request while cached");
    // the nightly rollup checks once the status is stale
    age_update_status(&env, 21 * 3600);
    let r = run(&["rollup", "--quiet"], &cfg, b"");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(hits.load(Ordering::SeqCst), 2, "rollup ran the check");
    // update.check = false: no nightly check, update-check needs --force
    let r = run(&["config", "set", "update.check", "false"], &cfg, b"");
    assert_eq!(r.code, 0, "{}", r.stderr);
    age_update_status(&env, 21 * 3600);
    run(&["rollup", "--quiet"], &cfg, b"");
    let r = run(&["update-check", "--json"], &cfg, b"");
    assert_eq!(r.code, 0);
    assert_eq!(hits.load(Ordering::SeqCst), 2, "disabled");
    let off = ok(
        &api(&env, "admin", &[("a", "admin.update")], None),
        "update",
    );
    assert_eq!(off["check_enabled"], json!(false));
    run(&["config", "unset", "update.check"], &cfg, b"");
    let r = run(&["update-check", "--force", "--quiet"], &cfg, b"");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(hits.load(Ordering::SeqCst), 3, "--force always asks");

    // admin.update: root, reseller (read only), cPanel user (403)
    let u = ok(
        &api(&env, "admin", &[("a", "admin.update")], None),
        "update",
    );
    keys_present(
        &[
            "current",
            "latest",
            "tag",
            "is_newer",
            "html_url",
            "published_at",
            "notes",
            "checked_at",
            "error",
            "checked_at_iso",
            "check_enabled",
            "install_sh",
            "can_upgrade",
            "upgrade_job",
        ],
        &u,
        "admin.update",
    );
    assert_eq!(u["latest"], json!("9.9.9"));
    assert_eq!(u["is_newer"], json!(true));
    assert_eq!(u["check_enabled"], json!(true));
    assert_eq!(
        u["install_sh"],
        json!(false),
        "the test harness is not an installation"
    );
    assert_eq!(u["can_upgrade"], json!(false));
    let rs = ok(
        &api(&env, "reseller:reseller1", &[("a", "admin.update")], None),
        "reseller",
    );
    assert_eq!(rs["latest"], json!("9.9.9"));
    assert_eq!(rs["can_upgrade"], json!(false));
    assert_eq!(rs["upgrade_job"], Value::Null);
    err(
        &api(&env, "user:alice", &[("a", "admin.update")], None),
        403,
        "forbidden",
        "user",
    );

    // admin.update_check: POST, root only, throttled to once an hour
    err(
        &api(
            &env,
            "reseller:reseller1",
            &[("a", "admin.update_check")],
            Some(json!({})),
        ),
        403,
        "forbidden",
        "reseller check",
    );
    err(
        &api_with(
            &env,
            "admin",
            &[("a", "admin.update_check")],
            None,
            "GET",
            None,
        ),
        405,
        "method_not_allowed",
        "GET check",
    );
    let n = hits.load(Ordering::SeqCst);
    let c = ok(
        &api(
            &env,
            "admin",
            &[("a", "admin.update_check")],
            Some(json!({})),
        ),
        "check",
    );
    assert_eq!(c["throttled"], json!(true), "checked less than an hour ago");
    assert_eq!(hits.load(Ordering::SeqCst), n);
    age_update_status(&env, 3601);
    let c = ok(
        &api(
            &env,
            "admin",
            &[("a", "admin.update_check")],
            Some(json!({})),
        ),
        "check",
    );
    assert_eq!(c["throttled"], json!(false));
    assert_eq!(c["checked_at"], json!(NOW));
    assert_eq!(hits.load(Ordering::SeqCst), n + 1);
    // the upgrade needs an installation directory with install.sh
    err(
        &api(
            &env,
            "admin",
            &[("a", "admin.run")],
            Some(json!({"type": "upgrade"})),
        ),
        409,
        "conflict",
        "no installer",
    );
    err(
        &api(
            &env,
            "reseller:reseller1",
            &[("a", "admin.run")],
            Some(json!({"type": "upgrade"})),
        ),
        403,
        "forbidden",
        "reseller upgrade",
    );
}

/// Runs `<install>/bin/webpulse cgi --whm --api` as root (testing mode) with a JSON POST.
fn whm_post(binary: &std::path::Path, env: &Env, action: &str, body: &str) -> Cgi {
    use std::io::Write;
    let mut c = std::process::Command::new(binary);
    c.args(["cgi", "--whm", "--api"])
        .env("WEBPULSE_CONFIG", &env.config)
        .env("WEBPULSE_CGI_TESTING", "1")
        .env("WEBPULSE_NOW", NOW.to_string())
        .env("REQUEST_METHOD", "POST")
        .env("QUERY_STRING", format!("a={action}"))
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("HTTP_X_WEBPULSE_REQUEST", "1")
        .env("REMOTE_USER", "root")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    parse_cgi(&child.wait_with_output().unwrap().stdout)
}

fn wait_job(env: &Env, id: i64) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let jobs = ok(&api(env, "admin", &[("a", "admin.jobs")], None), "jobs");
        let j = jobs["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["id"] == id)
            .cloned()
            .expect("job row");
        if j["status"] != "running" || std::time::Instant::now() > deadline {
            return j;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[test]
fn admin_run_upgrade_runs_install_sh_and_finishes_the_job() {
    use std::os::unix::fs::PermissionsExt;
    let env = fresh(SEED, "http-api-upgrade");
    let (url, _) = fake_github(RELEASE);
    with_update_url(&env, &url);
    let r = run(
        &["update-check", "--force", "--quiet"],
        &[("WEBPULSE_CONFIG", env.config.display().to_string())],
        b"",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    // a fake installation: <inst>/bin/webpulse (this build) + a stub install.sh
    let inst = env.root.join("inst");
    std::fs::create_dir_all(inst.join("bin")).unwrap();
    for d in [&inst, &inst.join("bin")] {
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let binary = inst.join("bin/webpulse");
    std::fs::copy(bin(), &binary).unwrap();
    let stub = |code: i32| {
        let f = inst.join("install.sh");
        std::fs::write(
            &f,
            format!(
                "#!/bin/bash\necho \"$*\" > \"$(dirname \"$0\")/args\"\nenv | sort > \"$(dirname \"$0\")/env\"\necho \"stub installer $*\"\nexit {code}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
    };

    stub(0);
    let c = whm_post(&binary, &env, "admin.run", r#"{"type":"upgrade"}"#);
    assert_eq!(c.status, 200, "{}", c.text());
    let d = c.json()["data"].clone();
    assert_eq!(d["tag"], json!("v9.9.9"));
    let id = d["job_id"].as_i64().unwrap();
    let j = wait_job(&env, id);
    assert_eq!(j["type"], json!("upgrade"));
    assert_eq!(j["scope"], json!("v9.9.9"));
    assert_eq!(j["status"], json!("ok"), "{j}");
    assert_eq!(j["message"], json!("Upgraded to v9.9.9"));
    assert_eq!(read_file(&inst.join("args")), "--update --ref v9.9.9 -y\n");
    let seen_env = read_file(&inst.join("env"));
    assert!(
        !seen_env.contains("REMOTE_USER") && !seen_env.contains("QUERY_STRING"),
        "clean environment: {seen_env}"
    );
    let log = read_file(&env.root.join("var/webpulse/log/upgrade.log"));
    assert!(
        log.contains("stub installer --update --ref v9.9.9 -y"),
        "{log}"
    );
    let u = ok(
        &api(&env, "admin", &[("a", "admin.update")], None),
        "update",
    );
    assert_eq!(u["upgrade_job"]["id"], json!(id));
    assert_eq!(u["upgrade_job"]["status"], json!("ok"));
    // a successful upgrade discards the cached release status (it predates the new
    // version): nothing newer is known until the next check
    assert_eq!(u["is_newer"], json!(false), "{u}");
    assert_eq!(u["checked_at"], json!(null), "{u}");

    // a failing installer: job error with the exit status (re-seed the status first)
    let r = run(
        &["update-check", "--force", "--quiet"],
        &[("WEBPULSE_CONFIG", env.config.display().to_string())],
        b"",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    stub(1);
    let c = whm_post(&binary, &env, "admin.run", r#"{"type":"upgrade"}"#);
    assert_eq!(c.status, 200, "{}", c.text());
    let id = c.json()["data"]["job_id"].as_i64().unwrap();
    let j = wait_job(&env, id);
    assert_eq!(j["status"], json!("error"), "{j}");
    assert!(
        j["message"]
            .as_str()
            .unwrap()
            .contains("exited with status 1"),
        "{j}"
    );

    // an upgrade already running: 409
    registry_db(&env)
        .execute(
            "INSERT INTO jobs (type, scope, started_at, status) VALUES ('upgrade', 'v9.9.9', ?, 'running')",
            [NOW],
        )
        .unwrap();
    let c = whm_post(&binary, &env, "admin.run", r#"{"type":"upgrade"}"#);
    assert_eq!(c.status, 409, "{}", c.text());
    assert_eq!(c.json()["error"]["code"], json!("conflict"));

    // a tag that is not newer: 400
    registry_db(&env)
        .execute(
            "UPDATE jobs SET status = 'error' WHERE status = 'running'",
            [],
        )
        .unwrap();
    let db = registry_db(&env);
    let raw: String = db
        .query_row(
            "SELECT value FROM settings WHERE key = 'update_status'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut st: Value = serde_json::from_str(&raw).unwrap();
    st["latest"] = json!("0.0.1");
    st["tag"] = json!("v0.0.1");
    db.execute(
        "UPDATE settings SET value = ? WHERE key = 'update_status'",
        [st.to_string()],
    )
    .unwrap();
    let c = whm_post(&binary, &env, "admin.run", r#"{"type":"upgrade"}"#);
    assert_eq!(c.status, 400, "{}", c.text());
}

fn read_file(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

// ------------------------------------------------------------------- GeoIP

/// A tiny valid MaxMind DB (203.0.113.0/24 = IT) of the given `database_type`.
fn tiny_mmdb(db_type: &str) -> Vec<u8> {
    use common::{MmdbWriter, map, s};
    let mut w = MmdbWriter::new(6, 24, &[], 0);
    w.insert(
        "203.0.113.0/24",
        &map(&[("country", map(&[("iso_code", s("IT"))]))]),
    );
    w.build(&[("database_type", s(db_type))])
}

fn gzip(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// Serves `body` with `status` for every request; returns `http://127.0.0.1:<port>`.
fn fake_download(status: u16, body: Vec<u8>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let _ = write!(
                s,
                "HTTP/1.1 {status} X\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(&body);
        }
    });
    base
}

fn patch_config(env: &Env, f: impl FnOnce(&mut Value)) {
    let mut cfg: Value =
        serde_json::from_str(&std::fs::read_to_string(&env.config).unwrap()).unwrap();
    f(&mut cfg);
    std::fs::write(&env.config, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
}

/// `webpulse cgi --whm --api` as root with a JSON POST (the spawned job runs this build).
fn whm_run(env: &Env, body: &str) -> Cgi {
    let r = run(
        &["cgi", "--whm", "--api"],
        &[
            ("WEBPULSE_CONFIG", env.config.display().to_string()),
            ("WEBPULSE_CGI_TESTING", "1".into()),
            ("REQUEST_METHOD", "POST".into()),
            ("QUERY_STRING", "a=admin.run".into()),
            ("CONTENT_LENGTH", body.len().to_string()),
            ("HTTP_X_WEBPULSE_REQUEST", "1".into()),
            ("REMOTE_USER", "root".into()),
        ],
        body.as_bytes(),
    );
    parse_cgi(&r.stdout)
}

/// Share of visits with a known country over the seeded domains (last 30 days).
fn seeded_known_share(env: &Env) -> (i64, i64) {
    let (mut t, mut u) = (0i64, 0i64);
    for (user, d) in [
        ("alice", "example.com"),
        ("alice", "shop.example.com"),
        ("bob", "blog.net"),
    ] {
        let db =
            rusqlite::Connection::open(env.root.join(format!("var/webpulse/data/{user}/{d}.db")))
                .unwrap();
        let (a, b): (i64, i64) = db
            .query_row(
                "SELECT COALESCE(SUM(visits),0), COALESCE(SUM(CASE WHEN key = '(unknown)' THEN visits ELSE 0 END),0) FROM dim_daily WHERE dim = 'country' AND day >= '2026-09-17'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        t += a;
        u += b;
    }
    (t, u)
}

#[test]
fn admin_geoip_status_without_and_with_a_database() {
    let env = fresh(SEED, "http-api-geoip");
    let g = ok(
        &api(&env, "admin", &[("a", "admin.geoip")], None),
        "admin.geoip",
    );
    keys_present(
        &[
            "enabled",
            "dir",
            "country",
            "city",
            "url",
            "url_preview",
            "url_default",
            "url_overridden",
            "cron",
            "last_job",
            "known_country_30d",
            "attribution",
            "provider",
            "can_edit",
        ],
        &g,
        "admin.geoip",
    );
    let dir = env.root.join("var/webpulse/geoip");
    assert_eq!(g["enabled"], json!(false));
    assert_eq!(g["dir"], json!(dir.display().to_string()));
    assert_eq!(
        g["country"],
        json!({"path": dir.join("dbip-country-lite.mmdb").display().to_string(), "present": false,
               "bytes": null, "mtime": null, "metadata": null, "error": null})
    );
    assert_eq!(g["city"], Value::Null);
    assert_eq!(
        g["url"],
        json!("https://download.db-ip.com/free/dbip-country-lite-{YYYY}-{MM}.mmdb.gz")
    );
    assert_eq!(
        g["url_preview"],
        json!("https://download.db-ip.com/free/dbip-country-lite-2026-10.mmdb.gz")
    );
    assert_eq!(g["url_overridden"], json!(false));
    assert_eq!(g["cron"]["present"], json!(false));
    assert_eq!(g["cron"]["scheduled"], json!(false));
    assert_eq!(g["last_job"], Value::Null);
    assert_eq!(g["attribution"], Value::Null);
    assert_eq!(
        g["provider"]["url"],
        json!("https://db-ip.com/db/download/ip-to-country-lite")
    );
    assert_eq!(g["provider"]["license"], json!("CC BY 4.0"));
    assert_eq!(g["can_edit"], json!(true));
    let (t, u) = seeded_known_share(&env);
    assert!(t > 0 && u > 0, "seeded countries include (unknown)");
    let k = &g["known_country_30d"];
    assert_eq!(k["visits"], json!(t));
    assert_eq!(k["known"], json!(t - u));
    assert_eq!(k["domains"], json!(3));
    assert_eq!(k["partial"], json!(false));
    let share = k["share"].as_f64().unwrap();
    assert!((share - (t - u) as f64 / t as f64).abs() < 0.0001, "{k}");

    // a DB-IP database, the cron line and GeoIP on
    std::fs::write(
        dir.join("dbip-country-lite.mmdb"),
        tiny_mmdb("DBIP-Country-Lite"),
    )
    .unwrap();
    std::fs::create_dir_all(env.root.join("etc/cron.d")).unwrap();
    std::fs::write(
        env.root.join("etc/cron.d/webpulse"),
        "# WebPulse\n*/5 * * * * root /usr/local/webpulse/bin/webpulse collect --quiet\n30 4 1 * * root /usr/local/webpulse/bin/webpulse geoip-update --quiet\n",
    )
    .unwrap();
    ok(
        &api(
            &env,
            "admin",
            &[("a", "admin.save_settings")],
            Some(json!({"settings": {"geoip.enabled": true}})),
        ),
        "enable",
    );
    let g = ok(
        &api(&env, "admin", &[("a", "admin.geoip")], None),
        "admin.geoip",
    );
    assert_eq!(g["enabled"], json!(true));
    let c = &g["country"];
    assert_eq!(c["present"], json!(true));
    assert!(c["bytes"].as_u64().unwrap() > 0);
    assert!(c["mtime"].is_string());
    let m = &c["metadata"];
    assert_eq!(m["database_type"], json!("DBIP-Country-Lite"));
    assert_eq!(m["build_epoch"], json!(1_790_000_000u64));
    assert_eq!(m["build_date"], json!("2026-09-21"));
    assert_eq!(m["ip_version"], json!(6));
    assert_eq!(m["languages"], json!(["en"]));
    assert_eq!(m["dbip"], json!(true));
    assert!(m["node_count"].as_u64().unwrap() > 0);
    assert_eq!(g["attribution"], json!("IP Geolocation by DB-IP"));
    assert_eq!(g["cron"]["scheduled"], json!(true));
    assert_eq!(
        g["cron"]["line"],
        json!("30 4 1 * * root /usr/local/webpulse/bin/webpulse geoip-update --quiet")
    );

    // a city database that is not a DB-IP one, and a corrupt country file
    std::fs::write(dir.join("city.mmdb"), tiny_mmdb("WebPulse-City")).unwrap();
    ok(
        &api(
            &env,
            "admin",
            &[("a", "admin.save_settings")],
            Some(
                json!({"settings": {"geoip.city_db": dir.join("city.mmdb").display().to_string()}}),
            ),
        ),
        "city_db",
    );
    std::fs::write(dir.join("dbip-country-lite.mmdb"), b"not a database").unwrap();
    let g = ok(
        &api(&env, "admin", &[("a", "admin.geoip")], None),
        "admin.geoip",
    );
    assert_eq!(g["country"]["metadata"], Value::Null);
    assert_eq!(g["country"]["error"], json!("not a valid MaxMind DB file"));
    assert_eq!(
        g["city"]["metadata"]["database_type"],
        json!("WebPulse-City")
    );
    assert_eq!(g["city"]["metadata"]["dbip"], json!(false));
    assert_eq!(g["attribution"], Value::Null);

    // resellers see the status (their domains only) but cannot change anything
    let r = ok(
        &api(&env, "reseller:reseller1", &[("a", "admin.geoip")], None),
        "reseller admin.geoip",
    );
    assert_eq!(r["can_edit"], json!(false));
    assert!(r["known_country_30d"]["domains"].as_i64().unwrap() < 3);
    err(
        &api(&env, "user:alice", &[("a", "admin.geoip")], None),
        403,
        "forbidden",
        "cPanel users",
    );
}

#[test]
fn admin_save_settings_validates_geoip_url_and_city_db() {
    let env = fresh(SEED, "http-api-geoip-settings");
    let dir = env.root.join("var/webpulse/geoip");
    let save = |v: Value| {
        api(
            &env,
            "admin",
            &[("a", "admin.save_settings")],
            Some(json!({ "settings": v })),
        )
    };
    let geo = || ok(&api(&env, "admin", &[("a", "admin.geoip")], None), "geoip");
    // admin.settings keeps the 0.1.0 shape
    let st = ok(
        &api(&env, "admin", &[("a", "admin.settings")], None),
        "settings",
    );
    assert_eq!(st["settings"].as_object().unwrap().len(), 9);

    let good = "https://mirror.example.net/geo/dbip-country-lite-{YYYY}-{MM}.mmdb.gz";
    let r = ok(&save(json!({"geoip.url": good})), "good url");
    assert_eq!(r["settings"].as_object().unwrap().len(), 9, "same shape");
    let g = geo();
    assert_eq!(g["url"], json!(good));
    assert_eq!(g["url_overridden"], json!(true));
    assert_eq!(
        g["url_preview"],
        json!("https://mirror.example.net/geo/dbip-country-lite-2026-10.mmdb.gz")
    );
    ok(
        &save(json!({"geoip.url": "https://x.example/plain.mmdb"})),
        "plain .mmdb",
    );
    for bad in [
        json!("http://x.example/a.mmdb.gz"),
        json!("ftp://x.example/a.mmdb"),
        json!("https://x.example/a.zip"),
        json!("https://x.example/a-{YY}.mmdb"),
        json!("https://{YYYY}.example/a.mmdb"),
        json!("https:///a.mmdb"),
        json!("https://x.example/a b.mmdb"),
        json!("file:///etc/passwd"),
        json!(42),
    ] {
        let r = save(json!({ "geoip.url": bad }));
        assert_eq!(r.status, 400, "geoip.url {bad}: {}", r.body);
        assert!(r.body.contains("geoip.url"), "{}", r.body);
    }
    ok(&save(json!({"geoip.url": ""})), "reset url");
    let g = geo();
    assert_eq!(
        g["url"],
        json!("https://download.db-ip.com/free/dbip-country-lite-{YYYY}-{MM}.mmdb.gz")
    );

    // city database: an existing .mmdb file in the GeoIP directory
    let city = dir.join("dbip-city-lite.mmdb");
    std::fs::write(&city, tiny_mmdb("DBIP-City-Lite")).unwrap();
    std::fs::write(dir.join("junk.mmdb"), b"junk").unwrap();
    std::fs::write(dir.join("city.dat"), tiny_mmdb("x")).unwrap();
    let outside = env.root.join("city.mmdb");
    std::fs::write(&outside, tiny_mmdb("x")).unwrap();
    ok(
        &save(json!({"geoip.city_db": city.display().to_string()})),
        "good city_db",
    );
    assert_eq!(geo()["city"]["path"], json!(city.display().to_string()));
    for bad in [
        "dbip-city-lite.mmdb".to_string(),
        outside.display().to_string(),
        dir.join("missing.mmdb").display().to_string(),
        dir.join("city.dat").display().to_string(),
        dir.join("junk.mmdb").display().to_string(),
        format!("{}/../geoip/dbip-city-lite.mmdb", dir.display()),
        dir.display().to_string(),
    ] {
        let r = save(json!({ "geoip.city_db": bad }));
        assert_eq!(r.status, 400, "geoip.city_db {bad}: {}", r.body);
        assert!(r.body.contains("geoip.city_db"), "{}", r.body);
    }
    assert_eq!(
        geo()["city"]["path"],
        json!(city.display().to_string()),
        "unchanged"
    );
    ok(&save(json!({"geoip.city_db": ""})), "clear city_db");
    assert_eq!(geo()["city"], Value::Null);
    // resellers cannot save
    let r = api(
        &env,
        "reseller:reseller1",
        &[("a", "admin.save_settings")],
        Some(json!({"settings": {"geoip.url": good}})),
    );
    assert_eq!(r.status, 403);
    // the CLI validates the same way
    let cfg = ("WEBPULSE_CONFIG", env.config.display().to_string());
    let r = run(
        &["config", "set", "geoip.url", "http://x.example/a.mmdb"],
        std::slice::from_ref(&cfg),
        b"",
    );
    assert_eq!(r.code, 2, "{}", r.stderr);
    let r = run(
        &[
            "config",
            "set",
            "geoip.city_db",
            &city.display().to_string(),
        ],
        &[cfg],
        b"",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
}

#[test]
fn admin_run_geoip_update_downloads_and_finishes_the_job() {
    let env = fresh(SEED, "http-api-geoip-run");
    let base = fake_download(200, gzip(&tiny_mmdb("DBIP-Country-Lite")));
    patch_config(&env, |c| {
        c["geoip"]["url"] = json!(format!("{base}/dbip-country-lite-{{YYYY}}-{{MM}}.mmdb.gz"));
    });
    // GeoIP is off: the download still runs (download only)
    let c = whm_run(&env, r#"{"type":"geoip-update"}"#);
    assert_eq!(c.status, 200, "{}", c.text());
    let d = c.json()["data"].clone();
    assert_eq!(d["enabled"], json!(false));
    let id = d["job_id"].as_i64().unwrap();
    let j = wait_job(&env, id);
    assert_eq!(j["type"], json!("geoip-update"));
    assert_eq!(j["scope"], json!("all"));
    assert_eq!(j["status"], json!("ok"), "{j}");
    assert!(
        j["message"]
            .as_str()
            .unwrap()
            .starts_with("installed DBIP-Country-Lite built 2026-09-21"),
        "{j}"
    );
    let g = ok(
        &api(&env, "admin", &[("a", "admin.geoip")], None),
        "admin.geoip",
    );
    assert_eq!(
        g["country"]["metadata"]["database_type"],
        json!("DBIP-Country-Lite")
    );
    assert_eq!(g["last_job"]["id"], json!(id));
    assert_eq!(g["last_job"]["status"], json!("ok"));
    assert_eq!(g["attribution"], Value::Null, "GeoIP is still off");

    // a failing download: job error with the HTTP status, the database is kept
    let bad = fake_download(404, b"nope".to_vec());
    patch_config(&env, |c| {
        c["geoip"]["url"] = json!(format!("{bad}/dbip-country-lite-{{YYYY}}-{{MM}}.mmdb.gz"));
    });
    let c = whm_run(&env, r#"{"type":"geoip-update"}"#);
    assert_eq!(c.status, 200, "{}", c.text());
    let id = c.json()["data"]["job_id"].as_i64().unwrap();
    let j = wait_job(&env, id);
    assert_eq!(j["status"], json!("error"), "{j}");
    assert!(j["message"].as_str().unwrap().contains("HTTP 404"), "{j}");
    let g = ok(
        &api(&env, "admin", &[("a", "admin.geoip")], None),
        "admin.geoip",
    );
    assert_eq!(
        g["country"]["present"],
        json!(true),
        "previous database kept"
    );
    assert_eq!(g["last_job"]["status"], json!("error"));

    // a download already running: 409; resellers: 403
    registry_db(&env)
        .execute(
            "INSERT INTO jobs (type, scope, started_at, status) VALUES ('geoip-update', 'all', ?, 'running')",
            [NOW],
        )
        .unwrap();
    let c = whm_run(&env, r#"{"type":"geoip-update"}"#);
    assert_eq!(c.status, 409, "{}", c.text());
    err(
        &api(
            &env,
            "reseller:reseller1",
            &[("a", "admin.run")],
            Some(json!({"type": "geoip-update"})),
        ),
        403,
        "forbidden",
        "reseller geoip-update",
    );
}
