//! A complete HTTP response: JSON envelope, CGI output with a `Status:` line,
//! header-injection stripping (port of `Http\Response`).

use std::time::Instant;

use serde_json::{Value, json};

use super::json;
use crate::reports::domain_data::date_c;

/// One HTTP response.
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTP status.
    pub status: u16,
    /// Content-Type.
    pub content_type: String,
    /// Body bytes.
    pub body: Vec<u8>,
    /// Extra headers, in order.
    pub headers: Vec<(String, String)>,
}

/// Reason phrase of the statuses WebPulse emits.
pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        303 => "See Other",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Content Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

fn no_store() -> Vec<(String, String)> {
    vec![
        ("Cache-Control".into(), "no-store".into()),
        ("X-Content-Type-Options".into(), "nosniff".into()),
    ]
}

impl Response {
    /// A response with a body.
    pub fn new(
        status: u16,
        content_type: &str,
        body: impl Into<Vec<u8>>,
        headers: Vec<(String, String)>,
    ) -> Response {
        Response {
            status,
            content_type: content_type.into(),
            body: body.into(),
            headers,
        }
    }

    /// `{"ok":true,"data":…,"meta":{"took_ms":…,"generated_at":…}}`.
    pub fn json(data: Value, started: Instant) -> Response {
        let took = crate::reports::php::round_int(started.elapsed().as_secs_f64() * 1000.0);
        let payload = json!({
            "ok": true,
            "data": data,
            "meta": {"took_ms": took, "generated_at": date_c(crate::util::time::now_ts())},
        });
        Response::new(
            200,
            "application/json; charset=utf-8",
            json::encode(&payload, json::API),
            no_store(),
        )
    }

    /// `{"ok":false,"error":{"code":…,"message":…}}`.
    pub fn error(status: u16, code: &str, message: &str) -> Response {
        let payload = json!({"ok": false, "error": {"code": code, "message": message}});
        Response::new(
            status,
            "application/json; charset=utf-8",
            json::encode(&payload, json::API),
            no_store(),
        )
    }

    /// Header lines (with `Status:` first in CGI mode, Content-Length last);
    /// CR and LF are removed from header values.
    pub fn header_lines(&self, cgi: bool) -> Vec<String> {
        let mut lines = Vec::new();
        if cgi {
            lines.push(format!("Status: {} {}", self.status, reason(self.status)));
        }
        lines.push(format!("Content-Type: {}", strip(&self.content_type)));
        for (k, v) in &self.headers {
            lines.push(format!("{}: {}", strip(k), strip(v)));
        }
        lines.push(format!("Content-Length: {}", self.body.len()));
        lines
    }

    /// The complete CGI output: header lines, blank line, body.
    pub fn cgi_bytes(&self) -> Vec<u8> {
        let mut out = self.header_lines(true).join("\r\n").into_bytes();
        out.extend_from_slice(b"\r\n\r\n");
        out.extend_from_slice(&self.body);
        out
    }

    /// The body as text (lossy).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn strip(s: &str) -> String {
    s.replace(['\r', '\n'], "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgi_and_injection() {
        let r = Response::error(404, "not_found", "Unknown domain");
        let out = String::from_utf8(r.cgi_bytes()).unwrap();
        assert!(out.starts_with(
            "Status: 404 Not Found\r\nContent-Type: application/json; charset=utf-8\r\n"
        ));
        assert!(out.ends_with("\r\n\r\n{\"ok\":false,\"error\":{\"code\":\"not_found\",\"message\":\"Unknown domain\"}}"));
        let h = Response::new(
            200,
            "text/plain",
            "x",
            vec![("X-Evil".into(), "a\r\nSet-Cookie: b".into())],
        )
        .header_lines(false);
        assert!(h.contains(&"X-Evil: aSet-Cookie: b".to_string()));
        let j = Response::json(json!({"n": 1.5, "z": 0}), Instant::now());
        let v: Value = serde_json::from_slice(&j.body).unwrap();
        assert_eq!(v["data"], json!({"n": 1.5, "z": 0}));
        assert!(v["meta"]["generated_at"].is_string());
    }
}
