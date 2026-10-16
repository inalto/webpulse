//! `webpulse cgi` run as a real CGI program: principals and uid checks, the
//! WHM edge rule, request bodies, page rendering, CSV export, usage errors.

mod http_common;

use http_common::*;
use serde_json::{Value, json};

const SEED: &str = "http-cgi";

fn euid() -> u32 {
    // SAFETY: no preconditions.
    unsafe { libc::geteuid() }
}

/// Login name of the current uid when it is a valid cPanel account name.
fn me() -> Option<String> {
    // SAFETY: getpwuid returns a pointer to static storage or NULL.
    let pw = unsafe { libc::getpwuid(libc::geteuid()) };
    if pw.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }
        .to_string_lossy()
        .into_owned();
    webpulse::paths::is_valid_user(&name).then_some(name)
}

fn testing() -> (&'static str, String) {
    ("WEBPULSE_CGI_TESTING", "1".to_string())
}

fn who(u: &str) -> (&'static str, String) {
    ("REMOTE_USER", u.to_string())
}

#[test]
fn principal_uid_check() {
    let env = shared(SEED);
    let (r, c) = cgi(
        &env,
        &["cgi", "--principal", "user:alice", "--api"],
        "GET",
        "a=domains",
        b"",
        &[],
    );
    if euid() != 0 {
        assert_eq!(r.code, 1, "refused for a foreign account");
        assert_eq!(c.status, 403);
        assert_eq!(c.json()["error"]["code"], "forbidden");
    }
    if let Some(name) = me().filter(|_| euid() != 0) {
        let (r, c) = cgi(
            &env,
            &["cgi", "--principal", &format!("user:{name}"), "--api"],
            "GET",
            "a=domains",
            b"",
            &[],
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(c.status, 200);
        assert_eq!(c.json()["data"]["domains"], json!([]));
    }
    // the testing escape hatch needs a non-default configuration
    if euid() != 0 {
        let r = run(
            &["cgi", "--principal", "user:alice", "--api"],
            &[
                testing(),
                ("REQUEST_METHOD", "GET".into()),
                ("QUERY_STRING", "a=domains".into()),
            ],
            b"",
        );
        let c = parse_cgi(&r.stdout);
        assert_eq!(c.status, 403, "default config: no bypass");
    }
}

#[test]
fn user_api_through_cgi() {
    let env = shared(SEED);
    let (r, c) = cgi(
        &env,
        &["cgi", "--principal", "user:alice", "--api"],
        "GET",
        "a=report&name=page&domain=example.com&limit=2",
        b"",
        &[testing()],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(c.status, 200);
    assert_eq!(
        c.header("Content-Type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(c.header("Cache-Control"), Some("no-store"));
    assert_eq!(
        c.header("Content-Length")
            .map(|l| l.parse::<usize>().unwrap()),
        Some(c.body.len())
    );
    let j = c.json();
    assert_eq!(j["data"]["name"], "page");
    assert_eq!(j["data"]["rows"].as_array().unwrap().len(), 2);
    let (_, c) = cgi(
        &env,
        &["cgi", "--principal", "user:alice", "--api"],
        "GET",
        "a=overview&domain=blog.net",
        b"",
        &[testing()],
    );
    assert_eq!(
        (c.status, c.json()["error"]["code"].clone()),
        (404, json!("not_found"))
    );
    let (_, c) = cgi(
        &env,
        &["cgi", "--principal", "user:alice", "--api"],
        "GET",
        "a=admin.overview",
        b"",
        &[testing()],
    );
    assert_eq!(c.status, 403);
}

#[test]
fn csv_export_headers() {
    let env = shared(SEED);
    let (_, c) = cgi(
        &env,
        &["cgi", "--principal", "user:alice", "--api"],
        "GET",
        "a=export&name=country&domain=example.com",
        b"",
        &[testing()],
    );
    assert_eq!(c.status, 200);
    assert_eq!(c.header("Content-Type"), Some("text/csv; charset=utf-8"));
    assert_eq!(
        c.header("Content-Disposition"),
        Some("attachment; filename=\"example.com-country-2026-09-17_2026-10-16.csv\"")
    );
    assert_eq!(c.header("X-Content-Type-Options"), Some("nosniff"));
    assert!(c.text().starts_with(
        "key,label,visits,pageviews,bounce_rate,avg_duration,bounces,duration,share\nit,Italy,"
    ));
}

#[test]
fn post_body_is_read_from_stdin() {
    let env = fresh(SEED, "http-cgi-post");
    let body = json!({"settings": {"ui": {"default_range": "90d"}}}).to_string();
    let (r, c) = cgi(
        &env,
        &["cgi", "--principal", "user:alice", "--api"],
        "POST",
        "a=save_user_settings",
        body.as_bytes(),
        &[
            testing(),
            ("HTTP_X_WEBPULSE_REQUEST", "1".into()),
            ("CONTENT_TYPE", "application/json".into()),
        ],
    );
    assert_eq!(c.status, 200, "{} {}", c.text(), r.stderr);
    assert_eq!(c.json()["data"]["settings"]["ui"]["default_range"], "90d");
    // without the header: 403 from the API
    let (_, c) = cgi(
        &env,
        &["cgi", "--principal", "user:alice", "--api"],
        "POST",
        "a=save_user_settings",
        body.as_bytes(),
        &[testing()],
    );
    assert_eq!(c.status, 403);
    // without CONTENT_LENGTH the body is read to EOF
    let r = run(
        &["cgi", "--principal", "user:alice", "--api"],
        &[
            testing(),
            ("WEBPULSE_CONFIG", env.config.display().to_string()),
            ("REQUEST_METHOD", "POST".into()),
            ("QUERY_STRING", "a=save_user_settings".into()),
            ("HTTP_X_WEBPULSE_REQUEST", "1".into()),
        ],
        json!({"ui": {"dark": "light"}}).to_string().as_bytes(),
    );
    let c = parse_cgi(&r.stdout);
    assert_eq!(
        (c.status, c.json()["data"]["settings"]["ui"]["dark"].clone()),
        (200, json!("light"))
    );
}

#[test]
fn whm_principal_from_remote_user() {
    let env = shared(SEED);
    let args = ["cgi", "--whm", "--api"];
    if euid() != 0 {
        let (r, c) = cgi(&env, &args, "GET", "a=domains", b"", &[who("root")]);
        assert_eq!(
            (r.code, c.status),
            (1, 403),
            "--whm refuses a non-root euid"
        );
    }
    let doms = |c: &Cgi| -> Vec<String> {
        c.json()["data"]["domains"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                format!(
                    "{}/{}",
                    d["user"].as_str().unwrap(),
                    d["domain"].as_str().unwrap()
                )
            })
            .collect()
    };
    let (_, c) = cgi(
        &env,
        &args,
        "GET",
        "a=domains",
        b"",
        &[testing(), who("root")],
    );
    assert_eq!(c.status, 200);
    assert_eq!(doms(&c).len(), 5);
    let (_, c) = cgi(
        &env,
        &args,
        "GET",
        "a=domains",
        b"",
        &[testing(), who("reseller1")],
    );
    assert_eq!(doms(&c), vec!["bob/blog.net", "carol/carol.org"]);
    let (_, c) = cgi(
        &env,
        &args,
        "GET",
        "a=admin.settings",
        b"",
        &[testing(), who("reseller1")],
    );
    assert_eq!(c.status, 403);
    for nobody in ["bob", "", "../x", "Root"] {
        let (_, c) = cgi(
            &env,
            &args,
            "GET",
            "a=domains",
            b"",
            &[testing(), who(nobody)],
        );
        assert_eq!(
            (c.status, c.json()["error"]["message"].clone()),
            (403, json!("Access denied.")),
            "REMOTE_USER {nobody:?}"
        );
    }
}

#[test]
fn whm_edge_rule_for_mutations() {
    let env = fresh(SEED, "http-cgi-edge");
    let args = ["cgi", "--whm", "--api"];
    let msg = json!("This action requires POST and the X-WebPulse-Request: 1 header.");
    let (_, c) = cgi(
        &env,
        &args,
        "GET",
        "a=admin.set_account",
        b"",
        &[testing(), who("root")],
    );
    assert_eq!(
        (c.status, c.json()["error"]["message"].clone()),
        (405, msg.clone())
    );
    let body = br#"{"user":"bob","enabled":false}"#;
    let (_, c) = cgi(
        &env,
        &args,
        "POST",
        "a=admin.set_account",
        body,
        &[testing(), who("root")],
    );
    assert_eq!(
        (c.status, c.json()["error"]["message"].clone()),
        (405, msg.clone()),
        "POST without the header"
    );
    let (_, c) = cgi(
        &env,
        &args,
        "PUT",
        "a=admin.jobs",
        b"",
        &[
            testing(),
            who("root"),
            ("HTTP_X_WEBPULSE_REQUEST", "1".into()),
        ],
    );
    assert_eq!(c.status, 405, "non-GET reads need POST");
    let (_, c) = cgi(
        &env,
        &args,
        "POST",
        "a=admin.set_account",
        body,
        &[
            testing(),
            who("root"),
            ("HTTP_X_WEBPULSE_REQUEST", "1".into()),
        ],
    );
    assert_eq!(c.status, 200, "{}", c.text());
    assert_eq!(c.json()["data"]["account"]["enabled"], false);
}

#[test]
fn page_rendering() {
    let env = shared(SEED);
    let (r, c) = cgi(
        &env,
        &["cgi", "--whm", "--page"],
        "GET",
        "",
        b"",
        &[testing(), who("root")],
    );
    assert_eq!(r.code, 0);
    assert_eq!(c.status, 200);
    assert_eq!(c.header("Content-Type"), Some("text/html; charset=utf-8"));
    let html = c.text();
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("window.WebPulseAdmin = { render: render, report: report };"));
    assert!(html.contains("\"apiUrl\":\"api.cgi\""));
    let (_, c) = cgi(
        &env,
        &[
            "cgi",
            "--principal",
            "user:alice",
            "--page",
            "--embedded",
            "--api-url",
            "api.live.php?x=1",
        ],
        "GET",
        "",
        b"",
        &[testing()],
    );
    let html = c.text();
    assert!(html.starts_with("<div id=\"webpulse-app\" class=\"wp-root\" data-embedded=\"1\">"));
    assert!(html.contains("\"apiUrl\":\"api.live.php?x=1\""));
    assert!(!html.contains("WebPulseAdmin = {"));
    // `index.live.php#report&…` inside cPanel chrome: report view + print stylesheet
    assert!(html.contains("function renderReport(") && html.contains("@media print {"));
    assert!(html.contains("function printHost("));
    let (_, c) = cgi(
        &env,
        &["cgi", "--whm", "--page"],
        "GET",
        "",
        b"",
        &[testing(), who("bob")],
    );
    assert_eq!(c.status, 403);
    assert!(c.text().contains("<h2>Access denied</h2>"));
}

#[test]
fn usage_errors() {
    let env = shared(SEED);
    for args in [
        vec!["cgi", "--api"],
        vec!["cgi", "--whm"],
        vec!["cgi", "--whm", "--api", "--page"],
        vec!["cgi", "--principal", "alice", "--api"],
        vec!["cgi", "--principal", "user:../x", "--api"],
        vec!["cgi", "--principal", "user:alice", "--whm", "--api"],
    ] {
        let (r, _) = (
            run(
                &args,
                &[("WEBPULSE_CONFIG", env.config.display().to_string())],
                b"",
            ),
            (),
        );
        assert_eq!(r.code, 2, "{args:?}: {}", r.stderr);
        assert!(r.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn json_body_errors() {
    let env = fresh(SEED, "http-cgi-json");
    let h = ("HTTP_X_WEBPULSE_REQUEST", "1".to_string());
    for (body, msg) in [
        (&b"{not json"[..], "Request body is not valid JSON"),
        (&b"42"[..], "Request body must be a JSON object"),
    ] {
        let (_, c) = cgi(
            &env,
            &["cgi", "--whm", "--api"],
            "POST",
            "a=admin.set_account",
            body,
            &[testing(), who("root"), h.clone()],
        );
        assert_eq!(
            (c.status, c.json()["error"]["message"].clone()),
            (400, Value::from(msg))
        );
    }
}
