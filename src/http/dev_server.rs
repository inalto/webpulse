//! `webpulse serve`: a local development server on `127.0.0.1:PORT` (port of
//! `Dev\Router` + `Cli\Command\ServeCommand`, on tiny_http instead of `php -S`).
//!
//! Binding to 127.0.0.1 is not authentication (other local users, and pages
//! open in the browser through DNS rebinding, can reach it), so every request
//! must carry a Host of `127.0.0.1:PORT` or `localhost:PORT` and the random
//! token: as `?token=` on the first page load (answered with a
//! `SameSite=Strict`, `HttpOnly` cookie and a redirect to `/`), then as that
//! cookie or the `X-WebPulse-Dev-Token` header.

use std::io::Read;

use super::api;
use super::cpanel;
use super::ctx::Ctx;
use super::page::{self, PageOptions};
use super::principal::Principal;
use super::request::{MAX_BODY, Request};
use super::response::Response;
use crate::app::App;
use crate::error::Result;
use crate::reports::{Query, urldecode};

/// Cookie holding the token.
pub const COOKIE: &str = "webpulse_dev";
/// Header alternative to the cookie.
pub const HEADER: &str = "X-WebPulse-Dev-Token";

/// Parses `user:<name>`, `admin` or `reseller:<name>`.
pub fn principal_from_spec(app: &App, spec: &str) -> Result<Principal> {
    let (kind, name) = spec.split_once(':').unwrap_or((spec, ""));
    match kind {
        "admin" => Ok(Principal::admin()),
        "reseller" if !name.is_empty() => Ok(Principal::reseller(
            name,
            Some(cpanel::owned_by(&app.paths, name)),
        )),
        "user" if !name.is_empty() => Ok(Principal::user(name)),
        _ => Err(crate::reports::invalid(format!(
            "invalid principal \"{spec}\" (user:<name>, admin, reseller:<name>)"
        ))),
    }
}

/// Constant-time string comparison (`hash_equals`).
pub fn hash_equals(known: &str, user: &str) -> bool {
    let (a, b) = (known.as_bytes(), user.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Access check of one request: `(200 | 403, reason, answer with cookie + redirect)`.
/// `cookie` / `query_token` are `None` when absent or not a scalar.
pub fn gate(
    path: &str,
    host: Option<&str>,
    query_token: Option<&str>,
    cookie: Option<&str>,
    header: Option<&str>,
    token: &str,
    port: &str,
) -> (u16, &'static str, bool) {
    if token.len() < 16 {
        return (
            403,
            "no access token configured; start the server with \"webpulse serve\"",
            false,
        );
    }
    let host = host.unwrap_or("").to_ascii_lowercase();
    if port.is_empty()
        || (host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}"))
    {
        return (403, "unexpected Host header", false);
    }
    let ok = |t: Option<&str>| t.is_some_and(|t| !t.is_empty() && hash_equals(token, t));
    if (path == "/" || path == "/index.html") && ok(query_token) {
        return (200, "", true);
    }
    if ok(cookie) || ok(header) {
        return (200, "", false);
    }
    (
        403,
        "missing or wrong access token; open the URL printed by \"webpulse serve\"",
        false,
    )
}

/// The scalar value of a cookie (`$_COOKIE[$name]`), `None` for arrays.
pub fn cookie_value(header: &str, name: &str) -> Option<String> {
    let mut found = None;
    for part in header.split(';') {
        let part = part.trim_start();
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let k = urldecode(k.trim());
        if k == name {
            found = Some(urldecode(v));
        } else if k.starts_with(&format!("{name}[")) {
            found = None;
        }
    }
    found
}

fn text(status: u16, body: &str) -> Response {
    Response::new(
        status,
        "text/plain; charset=utf-8",
        body.as_bytes().to_vec(),
        vec![("Cache-Control".into(), "no-store".into())],
    )
}

/// Handles one request of the dev server.
pub fn handle(
    app: &App,
    spec: &str,
    token: &str,
    port: &str,
    req: &Request,
    uri: &str,
) -> Response {
    let path = uri.split(['?', '#']).next().unwrap_or("/");
    let path = if path.is_empty() { "/" } else { path };
    let cookie = req.header("Cookie").and_then(|c| cookie_value(c, COOKIE));
    let (status, why, set_cookie) = gate(
        path,
        req.header("Host"),
        req.query.get("token"),
        cookie.as_deref(),
        req.header(HEADER),
        token,
        port,
    );
    if status != 200 {
        return text(status, &format!("webpulse serve: {why}\n"));
    }
    if set_cookie {
        // Drop the token from the address bar; keep the report's `noprint=1` switch.
        let location = if req.query.get("noprint") == Some("1") {
            "/?noprint=1"
        } else {
            "/"
        };
        return Response::new(
            303,
            "text/plain; charset=utf-8",
            Vec::new(),
            vec![
                (
                    "Set-Cookie".into(),
                    format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict"),
                ),
                ("Location".into(), location.into()),
                ("Cache-Control".into(), "no-store".into()),
            ],
        );
    }
    let principal = match principal_from_spec(app, spec) {
        Ok(p) => p,
        Err(e) => return text(500, &format!("webpulse serve: {e}\n")),
    };
    let ctx = Ctx::new(app);
    match path {
        "/" | "/index.html" => {
            let html = page::render(
                &ctx,
                &principal,
                &PageOptions::new("/api", false, Some("WebPulse Analytics (dev)")),
            );
            Response::new(
                200,
                "text/html; charset=utf-8",
                html.into_bytes(),
                vec![("Cache-Control".into(), "no-store".into())],
            )
        }
        "/api" => api::dispatch(&ctx, req, &principal),
        "/favicon.ico" => Response::new(204, "image/x-icon", Vec::new(), vec![]),
        _ => Response::new(
            404,
            "text/plain; charset=utf-8",
            b"Not found\n".to_vec(),
            vec![],
        ),
    }
}

/// Serves until killed. Prints the URL with the token on stderr.
pub fn run(app: &App, port: u16, spec: &str) -> Result<i32> {
    let mut raw = [0u8; 16];
    page::random_bytes(&mut raw);
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let server = tiny_http::Server::http(("127.0.0.1", port))
        .map_err(|e| anyhow::anyhow!("cannot listen on 127.0.0.1:{port}: {e}"))?;
    let cfg = app
        .config_path()
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::canonicalize(p).ok());
    eprintln!(
        "WebPulse dev server: http://127.0.0.1:{port}/?token={token}  (principal {spec}, config {})",
        cfg.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "default".into())
    );
    let port_s = port.to_string();
    for mut rq in server.incoming_requests() {
        let method = rq.method().as_str().to_ascii_uppercase();
        let uri = rq.url().to_string();
        let qs = uri
            .split_once('?')
            .map(|(_, q)| q.split('#').next().unwrap_or(""))
            .unwrap_or("");
        let headers: Vec<(String, String)> = rq
            .headers()
            .iter()
            .map(|h| {
                (
                    h.field.as_str().as_str().to_string(),
                    h.value.as_str().to_string(),
                )
            })
            .collect();
        let mut body = None;
        if method != "GET" && method != "HEAD" {
            let mut buf = Vec::new();
            let _ = rq.as_reader().take(MAX_BODY as u64).read_to_end(&mut buf);
            body = Some(buf);
        }
        let remote = rq
            .remote_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or_default();
        let req = Request::new(&method, Query::parse(qs), body, headers, &remote);
        let resp = handle(app, spec, &token, &port_s, &req, &uri);
        let mut out = tiny_http::Response::from_data(resp.body.clone())
            .with_status_code(tiny_http::StatusCode(resp.status));
        let mut all = vec![("Content-Type".to_string(), resp.content_type.clone())];
        all.extend(resp.headers.iter().cloned());
        for (k, v) in all {
            let v = v.replace(['\r', '\n'], "");
            if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                out.add_header(h);
            }
        }
        let _ = rq.respond(out);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_rules() {
        let tok = "ab".repeat(16);
        let h = Some("127.0.0.1:8099");
        assert_eq!(gate("/", h, Some(&tok), None, None, "", "8099").0, 403);
        assert_eq!(
            gate("/", h, Some(&tok), None, None, &tok, "8099"),
            (200, "", true)
        );
        assert_eq!(gate("/api", h, Some(&tok), None, None, &tok, "8099").0, 403);
        assert_eq!(gate("/api", h, None, Some(&tok), None, &tok, "8099").0, 200);
        assert_eq!(gate("/api", h, None, None, Some(&tok), &tok, "8099").0, 200);
        assert_eq!(gate("/api", h, None, Some("x"), None, &tok, "8099").0, 403);
        assert_eq!(
            gate(
                "/",
                Some("attacker.example:8099"),
                Some(&tok),
                None,
                None,
                &tok,
                "8099"
            )
            .0,
            403
        );
        assert_eq!(
            gate(
                "/",
                Some("127.0.0.1:9999"),
                Some(&tok),
                None,
                None,
                &tok,
                "8099"
            )
            .0,
            403
        );
        assert_eq!(
            gate(
                "/",
                Some("LOCALHOST:8099"),
                None,
                Some(&tok),
                None,
                &tok,
                "8099"
            )
            .0,
            200
        );
        assert_eq!(gate("/", None, None, Some(&tok), None, &tok, "8099").0, 403);
        assert_eq!(
            cookie_value("a=1; webpulse_dev=xyz; b=2", COOKIE).as_deref(),
            Some("xyz")
        );
        assert_eq!(cookie_value("webpulse_dev[]=xyz", COOKIE), None);
        assert!(
            hash_equals("abc", "abc") && !hash_equals("abc", "abd") && !hash_equals("abc", "ab")
        );
    }
}
