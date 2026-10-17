//! `webpulse serve`: the dashboard over HTTP on `127.0.0.1:PORT` or any
//! `--listen ADDR:PORT` (port of `Dev\Router` + `Cli\Command\ServeCommand`, on
//! tiny_http instead of `php -S`). TLS is left to a reverse proxy.
//!
//! Binding to 127.0.0.1 is not authentication (other local users, and pages
//! open in the browser through DNS rebinding, can reach it), so every request
//! must carry an expected Host (`127.0.0.1:PORT`, `localhost:PORT`, `[::1]:PORT`,
//! the `--listen` address, the `--allow-host` names) and the token: as `?token=`
//! on the first page load (answered with a `SameSite=Strict`, `HttpOnly` cookie
//! and a redirect to `/`), then as that cookie or the `X-WebPulse-Dev-Token`
//! header. The token is random per run, or persistent with `--token-file`
//! (created 0600 when missing; a file readable by others is refused).

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

/// The access token of a server run.
#[derive(Debug, Clone)]
pub struct Token {
    /// The secret.
    pub value: String,
    /// The file it was read from (`--token-file`).
    pub file: Option<std::path::PathBuf>,
}

impl Token {
    /// 32 random hex digits, printed at start.
    pub fn random() -> Token {
        let mut raw = [0u8; 16];
        page::random_bytes(&mut raw);
        Token {
            value: raw.iter().map(|b| format!("{b:02x}")).collect(),
            file: None,
        }
    }

    /// The token of `--token-file`: read when the file exists (it must be a
    /// regular file not readable by group or others, with at least 16 token
    /// characters), else a new random token written with mode 0600.
    pub fn from_file(path: &std::path::Path) -> Result<Token> {
        use std::io::Write;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let p = path.display().to_string();
        match std::fs::symlink_metadata(path) {
            Ok(m) => {
                if !m.file_type().is_file() {
                    return Err(anyhow::anyhow!("--token-file {p} is not a regular file"));
                }
                if m.mode() & 0o077 != 0 {
                    return Err(anyhow::anyhow!(
                        "--token-file {p} is readable by other users (chmod 600 {p})"
                    ));
                }
                let raw = std::fs::read_to_string(path)
                    .map_err(|e| anyhow::anyhow!("cannot read --token-file {p}: {e}"))?;
                let value = raw.trim().to_string();
                if value.len() < 16
                    || value.len() > 512
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_graphic() && b != b';' && b != b',')
                {
                    return Err(anyhow::anyhow!(
                        "--token-file {p} must hold a token of 16 to 512 printable characters"
                    ));
                }
                Ok(Token {
                    value,
                    file: Some(path.to_path_buf()),
                })
            }
            Err(_) => {
                let t = Token::random();
                if let Some(dir) = path.parent()
                    && !dir.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", dir.display()))?;
                }
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(path)
                    .map_err(|e| anyhow::anyhow!("cannot create --token-file {p}: {e}"))?;
                f.write_all(format!("{}\n", t.value).as_bytes())?;
                Ok(Token {
                    value: t.value,
                    file: Some(path.to_path_buf()),
                })
            }
        }
    }
}

/// Host header values accepted for a server listening on `addr` (lower-case).
pub fn allowed_hosts(addr: &std::net::SocketAddr, extra: &[String]) -> Vec<String> {
    let port = addr.port();
    let mut v = vec![
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ];
    if !addr.ip().is_unspecified() {
        v.push(addr.to_string().to_ascii_lowercase());
    }
    for h in extra {
        let h = h.to_ascii_lowercase();
        if !v.contains(&h) {
            v.push(h);
        }
    }
    v
}

/// Access check of one request: `(200 | 403, reason, answer with cookie + redirect)`.
/// `cookie` / `query_token` are `None` when absent or not a scalar. `hosts` are
/// the accepted Host values ([`allowed_hosts`]; an entry without a port matches
/// that name with any port, for reverse proxies).
pub fn gate(
    path: &str,
    host: Option<&str>,
    query_token: Option<&str>,
    cookie: Option<&str>,
    header: Option<&str>,
    token: &str,
    hosts: &[String],
) -> (u16, &'static str, bool) {
    if token.len() < 16 {
        return (
            403,
            "no access token configured; start the server with \"webpulse serve\"",
            false,
        );
    }
    let host = host.unwrap_or("").to_ascii_lowercase();
    let name_only = match host.rsplit_once(':') {
        Some((n, p)) if !n.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => n,
        _ => host.as_str(),
    };
    let host_ok = !host.is_empty()
        && hosts
            .iter()
            .any(|h| *h == host || (!h.contains(':') && !h.is_empty() && h.as_str() == name_only));
    if !host_ok {
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

/// Handles one request of the server.
pub fn handle(
    app: &App,
    spec: &str,
    token: &str,
    hosts: &[String],
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
        hosts,
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

/// Serves until killed. Prints the URL on stderr: with the token when it is
/// random, without it when it comes from a file (the journal of a service is
/// readable by more people than the token file).
pub fn run(
    app: &App,
    addr: std::net::SocketAddr,
    spec: &str,
    token: &Token,
    allow_hosts: &[String],
) -> Result<i32> {
    let server = tiny_http::Server::http(addr)
        .map_err(|e| anyhow::anyhow!("cannot listen on {addr}: {e}"))?;
    let cfg = app
        .config_path()
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::canonicalize(p).ok());
    let shown = if addr.ip().is_unspecified() {
        format!("127.0.0.1:{}", addr.port())
    } else {
        addr.to_string()
    };
    let cfg_s = cfg
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "default".into());
    match &token.file {
        None => eprintln!(
            "WebPulse dev server: http://{shown}/?token={}  (principal {spec}, config {cfg_s})",
            token.value
        ),
        Some(f) => eprintln!(
            "WebPulse server: http://{shown}/  (open it once with ?token=<the token in {}>; principal {spec}, config {cfg_s})",
            f.display()
        ),
    }
    let hosts = allowed_hosts(&addr, allow_hosts);
    let token = token.value.clone();
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
        let resp = handle(app, spec, &token, &hosts, &req, &uri);
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
        let local: std::net::SocketAddr = "127.0.0.1:8099".parse().unwrap();
        let hosts = allowed_hosts(&local, &[]);
        let proxied = allowed_hosts(&local, &["stats.example.com".to_string()]);
        assert_eq!(
            gate(
                "/",
                Some("stats.example.com"),
                Some(&tok),
                None,
                None,
                &tok,
                &proxied
            )
            .0,
            200
        );
        assert_eq!(
            gate(
                "/",
                Some("stats.example.com:443"),
                Some(&tok),
                None,
                None,
                &tok,
                &proxied
            )
            .0,
            200
        );
        assert_eq!(
            gate(
                "/",
                Some("stats.example.com"),
                Some(&tok),
                None,
                None,
                &tok,
                &hosts
            )
            .0,
            403
        );
        assert_eq!(
            gate(
                "/",
                Some("[::1]:8099"),
                Some(&tok),
                None,
                None,
                &tok,
                &hosts
            )
            .0,
            200
        );
        let any: std::net::SocketAddr = "0.0.0.0:8099".parse().unwrap();
        assert_eq!(allowed_hosts(&any, &[]).len(), 3);
        let lan: std::net::SocketAddr = "192.0.2.10:8099".parse().unwrap();
        assert!(allowed_hosts(&lan, &[]).contains(&"192.0.2.10:8099".to_string()));
        let h = Some("127.0.0.1:8099");
        assert_eq!(gate("/", h, Some(&tok), None, None, "", &hosts).0, 403);
        assert_eq!(
            gate("/", h, Some(&tok), None, None, &tok, &hosts),
            (200, "", true)
        );
        assert_eq!(gate("/api", h, Some(&tok), None, None, &tok, &hosts).0, 403);
        assert_eq!(gate("/api", h, None, Some(&tok), None, &tok, &hosts).0, 200);
        assert_eq!(gate("/api", h, None, None, Some(&tok), &tok, &hosts).0, 200);
        assert_eq!(gate("/api", h, None, Some("x"), None, &tok, &hosts).0, 403);
        assert_eq!(
            gate(
                "/",
                Some("attacker.example:8099"),
                Some(&tok),
                None,
                None,
                &tok,
                &hosts
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
                &hosts
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
                &hosts
            )
            .0,
            200
        );
        assert_eq!(gate("/", None, None, Some(&tok), None, &tok, &hosts).0, 403);
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
