//! The Cockpit entry points: `webpulse cgi --cockpit` (principal from the
//! effective uid, `REMOTE_USER` ignored, the edge rule for mutations, the
//! `whoami` action) and `webpulse page --cockpit` (the module document).

mod common;

use common::*;
use serde_json::{Value, json};

fn euid() -> u32 {
    // SAFETY: no preconditions.
    unsafe { libc::geteuid() }
}

/// Login name of the current uid.
fn me() -> String {
    webpulse::util::fs::user_name(euid()).unwrap_or_default()
}

struct Reply {
    code: i32,
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.body))
    }
    fn header(&self, n: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.as_str())
    }
}

fn cgi(
    fr: &FakeRoot,
    args: &str,
    method: &str,
    qs: &str,
    body: &str,
    extra: &[(&str, String)],
) -> Reply {
    let mut env: Vec<(&str, String)> = vec![
        ("REQUEST_METHOD", method.into()),
        ("QUERY_STRING", qs.into()),
        ("CONTENT_LENGTH", body.len().to_string()),
    ];
    env.extend(extra.iter().cloned());
    let mut cmd = std::process::Command::new(bin());
    cmd.args(split(args))
        .env("WEBPULSE_CONFIG", fr.config_path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in &env {
        cmd.env(k, v);
    }
    if let Some(n) = fr.now {
        cmd.env("WEBPULSE_NOW", n.to_string());
    }
    let mut child = cmd.spawn().unwrap();
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(body.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or(("", &text));
    let mut status = 200;
    let mut headers = Vec::new();
    for l in head.split("\r\n") {
        if let Some((k, v)) = l.split_once(": ") {
            if k == "Status" {
                status = v.split(' ').next().unwrap_or("0").parse().unwrap_or(0);
            } else {
                headers.push((k.to_string(), v.to_string()));
            }
        }
    }
    Reply {
        code: out.status.code().unwrap_or(-1),
        status,
        headers,
        body: body.to_string(),
    }
}

/// A standalone root with a site owned by the current user and one owned by `local`.
fn root(name: &str) -> FakeRoot {
    let owner = webpulse::engine::standalone::owner_name(&me()).unwrap_or_else(|| "local".into());
    let mut fr = FakeRoot::create_standalone(name, json!({}));
    let r = fr.root().display().to_string();
    fr.patch_config(json!({"sites": [
        {"domain": "mine.example.com", "owner": owner, "logs": [format!("{r}/var/log/httpd/mine.log")]},
        {"domain": "other.example.com", "logs": [format!("{r}/var/log/httpd/other.log")]}
    ]}));
    fr.write_log(
        "var/log/httpd/mine.log",
        &[line("203.0.113.1", "2026-10-10 10:00:00", "/")],
        false,
    );
    fr.write_log(
        "var/log/httpd/other.log",
        &[line("203.0.113.2", "2026-10-10 10:00:00", "/")],
        false,
    );
    fr.set_now(ts("2026-10-10 12:00:00"));
    fr.run("collect").ok("collect");
    fr
}

#[test]
fn principal_comes_from_the_effective_uid() {
    let fr = root("cockpit-principal");
    // REMOTE_USER is ignored
    let r = cgi(
        &fr,
        "cgi --cockpit --api",
        "GET",
        "a=whoami",
        "",
        &[("REMOTE_USER", "root".into())],
    );
    assert_eq!(r.code, 0);
    assert_eq!(r.status, 200, "{}", r.body);
    let who = r.json()["data"].clone();
    assert_eq!(who["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(who["mode"], "standalone");
    let domains = cgi(&fr, "cgi --cockpit --api", "GET", "a=domains", "", &[]).json();
    let names: Vec<String> = domains["data"]["domains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["domain"].as_str().unwrap().to_string())
        .collect();
    if euid() == 0 {
        assert_eq!(who["principal"], json!({"type": "admin", "name": "root"}));
        assert!(who["hostname"].is_string());
        assert!(names.contains(&"other.example.com".to_string()));
        return;
    }
    let name = me();
    assert_eq!(who["principal"], json!({"type": "user", "name": name}));
    assert!(
        who.get("hostname").is_none(),
        "host name only for the admin scope"
    );
    // the sites whose owner is this user, nothing else
    assert_eq!(names, ["mine.example.com"]);
    let ov = cgi(
        &fr,
        "cgi --cockpit --api",
        "GET",
        "a=overview&domain=other.example.com&range=7d",
        "",
        &[],
    );
    assert_eq!(ov.status, 404, "{}", ov.body);
    let ov = cgi(
        &fr,
        "cgi --cockpit --api",
        "GET",
        "a=overview&domain=mine.example.com&range=7d",
        "",
        &[],
    );
    assert_eq!(ov.status, 200, "{}", ov.body);
    // no admin scope
    let adm = cgi(
        &fr,
        "cgi --cockpit --api",
        "GET",
        "a=admin.overview",
        "",
        &[],
    );
    assert_eq!(adm.status, 403);
    // CSV export works over the same transport
    let csv = cgi(
        &fr,
        "cgi --cockpit --api",
        "GET",
        "a=export&domain=mine.example.com&name=page&range=7d&format=csv",
        "",
        &[],
    );
    assert_eq!(csv.status, 200);
    assert!(
        csv.header("Content-Type")
            .unwrap_or("")
            .starts_with("text/csv")
    );
    assert!(
        csv.header("Content-Disposition")
            .unwrap_or("")
            .contains(".csv")
    );
}

#[test]
fn mutations_need_post_and_the_request_header() {
    let fr = root("cockpit-mutations");
    let body = r#"{"settings":{"ui":{"default_range":"7d"}}}"#;
    let r = cgi(
        &fr,
        "cgi --cockpit --api",
        "GET",
        "a=save_user_settings",
        "",
        &[],
    );
    assert_eq!(r.status, 405);
    let r = cgi(
        &fr,
        "cgi --cockpit --api",
        "POST",
        "a=save_user_settings",
        body,
        &[],
    );
    assert_eq!(r.status, 405, "POST without X-WebPulse-Request");
    if euid() != 0 {
        let r = cgi(
            &fr,
            "cgi --cockpit --api",
            "POST",
            "a=save_user_settings",
            body,
            &[
                ("HTTP_X_WEBPULSE_REQUEST", "1".into()),
                ("CONTENT_TYPE", "application/json".into()),
            ],
        );
        assert_eq!(r.status, 200, "{}", r.body);
    }
}

#[test]
fn usage_and_page_mode() {
    let fr = root("cockpit-usage");
    for args in [
        "cgi --cockpit --whm --api",
        "cgi --cockpit --principal user:alice --api",
        "cgi --cockpit",
        "page",
        "page --cockpit --bin relative/webpulse",
    ] {
        let o = fr.run(args);
        assert_eq!(o.code, 2, "{args}: {}{}", o.stdout, o.stderr);
    }
    let p = cgi(&fr, "cgi --cockpit --page", "GET", "", "", &[]);
    assert_eq!(p.status, 200);
    assert!(
        p.body
            .contains("<script src=\"../base1/cockpit.js\"></script>")
    );
    assert!(
        p.body
            .contains("\"bin\":\"\\/usr\\/local\\/webpulse\\/bin\\/webpulse\"")
            || p.body
                .contains("\"bin\":\"/usr/local/webpulse/bin/webpulse\"")
    );
}

#[test]
fn page_cockpit_document() {
    let fr = root("cockpit-page");
    let o = fr
        .run("page --cockpit --bin /opt/webpulse/bin/webpulse")
        .ok("page --cockpit");
    let html = o.stdout;
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<script src=\"../base1/cockpit.js\"></script>"));
    // no CSP meta, no nonce: Cockpit applies the manifest's policy
    assert!(!html.contains("Content-Security-Policy"));
    assert!(!html.contains("nonce"));
    let i = html.find("window.WEBPULSE = ").unwrap() + "window.WEBPULSE = ".len();
    let end = html[i..].find(";</script>").unwrap();
    let boot: Value = serde_json::from_str(&html[i..i + end]).unwrap();
    assert_eq!(boot["transport"], "cockpit");
    assert_eq!(boot["bin"], "/opt/webpulse/bin/webpulse");
    assert_eq!(boot["principal"], Value::Null, "resolved at run time");
    assert_eq!(boot["domains"], json!([]));
    assert_eq!(boot["version"], env!("CARGO_PKG_VERSION"));
    // every asset, the admin view and the adapter, started by the adapter
    for marker in [
        "WebPulseApp",
        "WebPulseAdmin",
        "WebPulseCockpit",
        "cockpit.spawn",
        "'--cockpit', '--api'",
        "superuser: 'try'",
        "WebPulseCockpit.start(document.getElementById('webpulse-app'), window.WEBPULSE)",
    ] {
        assert!(html.contains(marker), "{marker}");
    }
    assert_eq!(
        html.matches("</script>").count(),
        8,
        "cockpit.js + boot + 5 assets + start"
    );
    // the shipped manifest
    let m: Value = serde_json::from_str(
        &std::fs::read_to_string(repo().join("cockpit/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(m["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(m["requires"]["cockpit"], "266");
    assert_eq!(m["menu"]["index"]["label"], "Web analytics");
    assert_eq!(m["menu"]["index"]["order"], 65);
    let csp = m["content-security-policy"].as_str().unwrap();
    assert!(csp.contains("script-src 'self' 'unsafe-inline'"));
    assert!(csp.contains("style-src 'self' 'unsafe-inline'"));
}
