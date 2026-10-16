//! Page rendering (port of tests/cases/ui_page.php): document vs embedded
//! fragment, CSP nonce, escaped bootstrap, admin.js only for admin scope,
//! Response CGI mode, UI asset checks.

mod http_common;

use http_common::*;
use serde_json::{Value, json};
use webpulse::http::page::{PageOptions, render};
use webpulse::http::{Ctx, Principal, Response};

const SEED: &str = "http-page";

fn page(env: &Env, p: &Principal, o: PageOptions) -> String {
    let a = app(env);
    let ctx = Ctx::new(&a);
    render(&ctx, p, &o)
}

fn nonce_of(html: &str) -> String {
    let i = html.find("script-src 'nonce-").expect("nonce in CSP") + "script-src 'nonce-".len();
    html[i..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

#[test]
fn full_document_with_nonce_csp() {
    let env = shared(SEED);
    let html = page(
        &env,
        &Principal::user("alice"),
        PageOptions::new("api.live.php", false, None),
    );
    assert!(html.starts_with("<!doctype html>\n<html lang=\"en\">"));
    let nonce = nonce_of(&html);
    assert!(nonce.len() >= 20);
    assert!(html.contains(&format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'; img-src data:; connect-src 'self'"
    )));
    let tags: Vec<&str> = html
        .match_indices("<script")
        .chain(html.match_indices("<style"))
        .map(|(i, _)| &html[i..i + html[i..].find('>').unwrap()])
        .collect();
    assert!(tags.len() >= 5, "style + bootstrap + 3 scripts + start");
    for t in tags {
        assert!(
            t.contains(&format!("nonce=\"{nonce}\"")),
            "every tag carries the nonce: {t}"
        );
    }
    assert!(
        !html.contains("WebPulseAdmin = {"),
        "no admin code for users"
    );
    assert!(
        !html.contains("src=\"http") && !html.contains("href=\"http"),
        "no external resources"
    );
    assert!(html.contains("<div id=\"webpulse-app\" class=\"wp-root\" data-embedded=\"0\">"));
    let again = page(
        &env,
        &Principal::user("alice"),
        PageOptions::new("api.live.php", false, None),
    );
    assert_ne!(nonce_of(&again), nonce, "fresh nonce per render");
}

#[test]
fn embedded_fragment_for_cpanel_chrome() {
    let env = shared(SEED);
    let html = page(
        &env,
        &Principal::user("alice"),
        PageOptions::new("api.live.php", true, None),
    );
    assert!(html.starts_with("<div id=\"webpulse-app\" class=\"wp-root\" data-embedded=\"1\">"));
    assert!(!html.contains("<html") && !html.contains("<head") && !html.contains("<body"));
    assert!(html.contains("<style nonce=") && html.contains("WebPulseApp.start("));
}

#[test]
fn admin_pages_include_admin_js() {
    let env = shared(SEED);
    let html = page(
        &env,
        &Principal::admin(),
        PageOptions::new("api.cgi", false, None),
    );
    assert!(html.contains("window.WebPulseAdmin = { render: render };"));
    assert!(html.contains("\"principal\":{\"type\":\"admin\",\"name\":\"root\"}"));
}

#[test]
fn bootstrap_json_is_escaped_for_script() {
    let env = shared(SEED);
    let evil = "x</script><script>alert(1)</script>&<!--";
    let html = page(
        &env,
        &Principal::user("alice"),
        PageOptions::new(evil, false, Some("<b>T</b>")),
    );
    assert!(
        !html.contains("</script><script>alert(1)"),
        "no raw closing tag"
    );
    assert!(html.contains(r#""apiUrl":"x\u003C/script\u003E\u003Cscript\u003Ealert(1)\u003C/script\u003E\u0026\u003C!--""#));
    assert!(html.contains("<title>&lt;b&gt;T&lt;/b&gt;</title>"));
    let start = html.find("window.WEBPULSE = ").unwrap() + "window.WEBPULSE = ".len();
    let end = start + html[start..].find(";</script>").unwrap();
    let boot: Value = serde_json::from_str(&html[start..end]).unwrap();
    assert_eq!(boot["apiUrl"], evil, "round-trips");
    let doms: Vec<&str> = boot["domains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["domain"].as_str().unwrap())
        .collect();
    assert_eq!(doms, vec!["example.com", "shop.example.com"]);
    assert_eq!(boot["dims"].as_object().unwrap().len(), 32);
    assert_eq!(boot["defaults"]["range"], "30d");
    assert_eq!(
        boot["ranges"],
        json!([
            "today",
            "yesterday",
            "7d",
            "30d",
            "90d",
            "12m",
            "month",
            "lastmonth",
            "year",
            "all"
        ])
    );
    assert_eq!(boot["embedded"], false);
}

#[test]
fn response_cgi_status_line_and_envelope() {
    let r = Response::error(404, "not_found", "Unknown domain");
    let lines = r.header_lines(true);
    assert_eq!(lines[0], "Status: 404 Not Found");
    assert!(lines.contains(&"Content-Type: application/json; charset=utf-8".to_string()));
    let out = String::from_utf8(r.cgi_bytes()).unwrap();
    assert!(
        out.starts_with(
            "Status: 404 Not Found\r\nContent-Type: application/json; charset=utf-8\r\n"
        )
    );
    assert!(out.contains(
        "\r\n\r\n{\"ok\":false,\"error\":{\"code\":\"not_found\",\"message\":\"Unknown domain\"}}"
    ));
    let j: Value = serde_json::from_slice(
        &Response::json(json!({"n": 1.5, "z": 0}), std::time::Instant::now()).body,
    )
    .unwrap();
    assert_eq!(j["data"], json!({"n": 1.5, "z": 0}));
    assert!(j["meta"]["generated_at"].is_string());
    let h = Response::new(
        200,
        "text/plain",
        "x",
        vec![("X-Evil".into(), "a\r\nSet-Cookie: b".into())],
    )
    .header_lines(false);
    assert!(
        h.contains(&"X-Evil: aSet-Cookie: b".to_string()),
        "header injection stripped"
    );
}

#[test]
fn ui_assets_size_no_inner_html_no_external_urls() {
    let mut total = 0;
    for f in ["app.css", "charts.js", "i18n.js", "app.js", "admin.js"] {
        let src = std::fs::read_to_string(repo().join("ui").join(f)).unwrap();
        total += src.len();
        for bad in [
            ".innerHTML =",
            ".innerHTML=",
            ".outerHTML",
            "insertAdjacentHTML",
            "document.write",
        ] {
            assert!(!src.contains(bad), "{f} must not inject HTML ({bad})");
        }
        for (i, _) in src.match_indices("http") {
            let rest = &src[i..];
            if rest.starts_with("http://") || rest.starts_with("https://") {
                assert!(
                    rest.starts_with("http://www.w3.org/2000/svg"),
                    "{f} must not reference external URLs"
                );
            }
        }
        if f.ends_with(".js") {
            assert!(src.contains("'use strict';"), "{f} strict mode");
        }
    }
    assert!(total < 200 * 1024, "inline payload under 200 KB");
}
