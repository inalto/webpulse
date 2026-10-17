//! `webpulse serve` over real HTTP (port of the dev-router cases of
//! tests/cases/ui_engine.php and sec_web.php): token on first load → cookie,
//! header alternative, Host check, page and API, 404s.

mod http_common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use http_common::*;
use serde_json::Value;

struct Server {
    child: Child,
    port: u16,
    token: String,
    _stderr: BufReader<std::process::ChildStderr>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(env: &Env, principal: &[&str]) -> Server {
    for attempt in 0..20u16 {
        let port = 18000 + (std::process::id() as u16 % 1500) + attempt * 37;
        let mut args = vec![
            "serve".to_string(),
            "--port".into(),
            port.to_string(),
            "--allow-root".into(),
        ];
        args.extend(principal.iter().map(|s| s.to_string()));
        let mut child = Command::new(bin())
            .args(&args)
            .env("WEBPULSE_CONFIG", &env.config)
            .env("WEBPULSE_NOW", NOW.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        let mut rd = BufReader::new(child.stderr.take().unwrap());
        let _ = rd.read_line(&mut line);
        if let Some(i) = line.find("?token=") {
            let token: String = line[i + 7..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            assert!(
                line.contains(&format!("http://127.0.0.1:{port}/")),
                "{line}"
            );
            return Server {
                child,
                port,
                token,
                _stderr: rd,
            };
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    panic!("dev server did not start");
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, n: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.as_str())
    }
}

fn get(port: u16, path: &str, headers: &[String]) -> Reply {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("GET {path} HTTP/1.1\r\nConnection: close\r\n");
    if !headers
        .iter()
        .any(|h| h.to_ascii_lowercase().starts_with("host:"))
    {
        req.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for h in headers {
        req.push_str(h);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap_or("")
        .split(' ')
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| {
            l.split_once(": ")
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();
    Reply {
        status,
        headers,
        body: body.to_string(),
    }
}

#[test]
fn dev_server_serves_page_and_api() {
    let env = shared("http-dev");
    let s = start(&env, &["--user", "alice"]);
    let p = s.port;
    let cookie = format!("Cookie: webpulse_dev={}", s.token);
    assert_eq!(get(p, "/", &[]).status, 403, "no token");
    assert_eq!(get(p, "/api?a=domains", &[]).status, 403);
    assert_eq!(
        get(p, &format!("/?token={}", "0".repeat(32)), &[]).status,
        403,
        "wrong token"
    );
    let r = get(p, &format!("/?token={}", s.token), &[]);
    assert_eq!(r.status, 303);
    assert_eq!(r.header("Location"), Some("/"));
    let r2 = get(p, &format!("/?token={}&noprint=1", s.token), &[]);
    assert_eq!(
        r2.header("Location"),
        Some("/?noprint=1"),
        "the report's noprint switch survives the token redirect"
    );
    let set = r.header("Set-Cookie").unwrap_or("");
    assert!(
        set.contains(&format!("webpulse_dev={}", s.token))
            && set.contains("SameSite=Strict")
            && set.contains("HttpOnly")
    );
    assert_eq!(
        get(p, &format!("/api?a=domains&token={}", s.token), &[]).status,
        403,
        "query token only on the page"
    );
    let r = get(p, "/", std::slice::from_ref(&cookie));
    assert_eq!(r.status, 200);
    assert!(r.body.contains("window.WEBPULSE") && r.body.contains("WebPulse Analytics (dev)"));
    assert!(r.body.contains("\"apiUrl\":\"/api\""));
    assert_eq!(
        get(
            p,
            "/api?a=domains",
            &[format!("Cookie: webpulse_dev=x{}", s.token)]
        )
        .status,
        403,
        "wrong cookie"
    );
    let r = get(
        p,
        "/api?a=report&name=page&domain=example.com&limit=2",
        std::slice::from_ref(&cookie),
    );
    assert_eq!(r.status, 200);
    let j: Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(j["data"]["name"], "page");
    assert_eq!(
        r.header("Content-Type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        get(
            p,
            "/api?a=domains",
            &[format!("X-WebPulse-Dev-Token: {}", s.token)]
        )
        .status,
        200,
        "header instead of cookie"
    );
    assert_eq!(
        get(
            p,
            "/api?a=domains",
            &[cookie.clone(), format!("Host: evil.example:{p}")]
        )
        .status,
        403,
        "DNS rebinding"
    );
    assert_eq!(
        get(
            p,
            "/api?a=domains",
            &[cookie.clone(), format!("Host: localhost:{p}")]
        )
        .status,
        200
    );
    assert_eq!(
        get(
            p,
            "/api?a=overview&domain=blog.net",
            std::slice::from_ref(&cookie)
        )
        .status,
        404
    );
    assert_eq!(
        get(p, "/../config.json", std::slice::from_ref(&cookie)).status,
        404
    );
    assert_eq!(
        get(p, "/favicon.ico", std::slice::from_ref(&cookie)).status,
        204
    );
    let r = get(p, "/api?a=export&name=page&domain=example.com", &[cookie]);
    assert_eq!(
        r.header("Content-Disposition"),
        Some("attachment; filename=\"example.com-page-2026-09-17_2026-10-16.csv\"")
    );
}

#[test]
fn dev_server_admin_and_reseller() {
    let env = shared("http-dev");
    let s = start(&env, &["--reseller", "reseller1"]);
    let r = get(
        s.port,
        "/api?a=admin.accounts",
        &[format!("X-WebPulse-Dev-Token: {}", s.token)],
    );
    let j: Value = serde_json::from_str(&r.body).unwrap();
    let users: Vec<&str> = j["data"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["user"].as_str().unwrap())
        .collect();
    assert_eq!(users, vec!["bob", "carol"]);
    drop(s);
    let s = start(&env, &["--admin"]);
    let r = get(s.port, "/", &[format!("X-WebPulse-Dev-Token: {}", s.token)]);
    assert!(
        r.body
            .contains("window.WebPulseAdmin = { render: render, report: report, ")
    );
}

#[test]
fn serve_options() {
    let env = shared("http-dev");
    let cfg = ("WEBPULSE_CONFIG", env.config.display().to_string());
    for args in [
        vec!["serve", "--user", "alice", "--admin"],
        vec!["serve", "--user", "../x"],
        vec!["serve", "--port", "0"],
        vec!["serve", "--port", "http"],
    ] {
        let r = run(&args, std::slice::from_ref(&cfg), b"");
        assert_eq!(r.code, 2, "{args:?}: {}", r.stderr);
    }
    let r = run(&["serve", "--help"], &[cfg], b"");
    assert!((r.out() + &r.stderr).contains("--allow-root"));
}
