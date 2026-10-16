//! An HTTP request as seen by the API: method, query parameters, raw body,
//! lower-cased headers and the client address (port of `Http\Request` and of
//! the CGI parsing of `whm/lib/Edge.php`).

use std::collections::HashMap;
use std::io::Read;

use serde_json::Value;

use super::api::ApiError;
use crate::reports::Query;

/// Largest request body read (1 MiB).
pub const MAX_BODY: usize = 1_048_576;

/// One API request.
#[derive(Debug, Clone)]
pub struct Request {
    /// Upper-case method.
    pub method: String,
    /// Query parameters.
    pub query: Query,
    /// Raw body (POST).
    pub body: Option<Vec<u8>>,
    /// Lower-case header name (`_` → `-`) → value.
    pub headers: HashMap<String, String>,
    /// Client address.
    pub remote_addr: String,
}

impl Request {
    /// A request from its parts; header names are normalised.
    pub fn new(
        method: &str,
        query: Query,
        body: Option<Vec<u8>>,
        headers: Vec<(String, String)>,
        remote_addr: &str,
    ) -> Request {
        let headers = headers
            .into_iter()
            .map(|(k, v)| (k.replace('_', "-").to_ascii_lowercase(), v))
            .collect();
        Request {
            method: method.to_ascii_uppercase(),
            query,
            body,
            headers,
            remote_addr: remote_addr.into(),
        }
    }

    /// Builds the request from a CGI environment (`REQUEST_METHOD`,
    /// `QUERY_STRING`, `CONTENT_TYPE`, `CONTENT_LENGTH`, `HTTP_*`, `REMOTE_ADDR`)
    /// and the body on `stdin` (at most `CONTENT_LENGTH`, capped at 1 MiB).
    pub fn from_cgi(env: &HashMap<String, String>, stdin: &mut dyn Read) -> Request {
        let method = env
            .get("REQUEST_METHOD")
            .map(|s| s.to_ascii_uppercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "GET".into());
        let query = Query::parse(env.get("QUERY_STRING").map(String::as_str).unwrap_or(""));
        let mut headers = Vec::new();
        for (k, v) in env {
            if let Some(name) = k.strip_prefix("HTTP_") {
                headers.push((name.to_string(), v.clone()));
            } else if k == "CONTENT_TYPE" {
                headers.push(("Content-Type".to_string(), v.clone()));
            }
        }
        let mut body = None;
        if method != "GET" && method != "HEAD" {
            match env.get("CONTENT_LENGTH") {
                Some(l) => {
                    let len = crate::reports::php::intval(l).clamp(0, MAX_BODY as i64) as usize;
                    if len > 0 {
                        let mut buf = Vec::with_capacity(len);
                        let _ = stdin.take(len as u64).read_to_end(&mut buf);
                        body = Some(buf);
                    }
                }
                // No CONTENT_LENGTH: a shim piping the body; read to EOF (capped),
                // never from a terminal.
                None if !stdin_is_tty() => {
                    let mut buf = Vec::new();
                    let _ = stdin.take(MAX_BODY as u64).read_to_end(&mut buf);
                    if !buf.is_empty() {
                        body = Some(buf);
                    }
                }
                None => {}
            }
        }
        Request::new(
            &method,
            query,
            body,
            headers,
            env.get("REMOTE_ADDR").map(String::as_str).unwrap_or(""),
        )
    }

    /// A scalar query parameter (`Request::get`).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.query.get(name)
    }

    /// A header (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// The JSON body: an object or array (`{}` when empty); 400 on invalid
    /// JSON, more than 32 levels, or a scalar.
    pub fn json(&self) -> Result<Value, ApiError> {
        let Some(body) = &self.body else {
            return Ok(Value::Object(Default::default()));
        };
        let text = String::from_utf8_lossy(body);
        if crate::reports::php::trim(&text).is_empty() {
            return Ok(Value::Object(Default::default()));
        }
        if body.len() > MAX_BODY {
            return Err(ApiError::bad_request("Request body too large"));
        }
        let invalid = || ApiError::bad_request("Request body is not valid JSON");
        let v: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
        if depth(&v) > 32 {
            return Err(invalid());
        }
        match v {
            Value::Object(_) | Value::Array(_) => Ok(v),
            _ => Err(ApiError::bad_request("Request body must be a JSON object")),
        }
    }
}

fn stdin_is_tty() -> bool {
    // SAFETY: isatty() has no preconditions.
    unsafe { libc::isatty(0) == 1 }
}

/// Nesting depth as PHP counts it for `json_decode($s, true, $depth)`.
fn depth(v: &Value) -> usize {
    match v {
        Value::Array(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
        Value::Object(m) => 1 + m.values().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// `$b[$key]` of a decoded PHP array (objects by key; lists only have
/// numeric keys).
pub fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m.get(key),
        Value::Array(a) => key
            .parse::<usize>()
            .ok()
            .filter(|i| i.to_string() == key)
            .and_then(|i| a.get(i)),
        _ => None,
    }
}

/// `$b[$key] ?? null` treating JSON null as absent.
pub fn field_nn<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    field(v, key).filter(|x| !x.is_null())
}

/// PHP `is_array()` of a decoded value.
pub fn is_array(v: &Value) -> bool {
    v.is_object() || v.is_array()
}

/// PHP `$x === []` of a decoded value.
pub fn is_empty_array(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// PHP `array_is_list()` of a decoded array.
pub fn is_list(v: &Value) -> bool {
    match v {
        Value::Array(_) => true,
        Value::Object(m) => m.keys().enumerate().all(|(i, k)| *k == i.to_string()),
        _ => false,
    }
}

/// `(key, value)` pairs of a decoded array (lists keyed `0..n-1`).
pub fn entries(v: &Value) -> Vec<(String, &Value)> {
    match v {
        Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(a) => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgi_parsing() {
        let env: HashMap<String, String> = [
            ("REQUEST_METHOD", "post"),
            ("QUERY_STRING", "a=admin.run&x=1"),
            ("CONTENT_LENGTH", "16"),
            ("HTTP_X_WEBPULSE_REQUEST", "1"),
            ("REMOTE_ADDR", "203.0.113.9"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let mut body: &[u8] = b"{\"type\":\"rollup\"}trailing";
        let r = Request::from_cgi(&env, &mut body);
        assert_eq!(r.method, "POST");
        assert_eq!(r.get("a"), Some("admin.run"));
        assert_eq!(r.header("X-WebPulse-Request"), Some("1"));
        assert_eq!(r.body.as_deref(), Some(&b"{\"type\":\"rollup\""[..]));
        assert!(r.json().is_err());
        let ok = Request::new("POST", Query::new(), Some(b"[1,2]".to_vec()), vec![], "");
        assert!(ok.json().unwrap().is_array());
        let scalar = Request::new("POST", Query::new(), Some(b"5".to_vec()), vec![], "");
        assert_eq!(
            scalar.json().unwrap_err().message,
            "Request body must be a JSON object"
        );
        let deep = Request::new(
            "POST",
            Query::new(),
            Some(("[".repeat(40) + &"]".repeat(40)).into_bytes()),
            vec![],
            "",
        );
        assert!(deep.json().is_err());
    }
}
