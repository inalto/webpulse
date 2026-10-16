//! Read side (R2): ranges, dimensions, report queries, exports. See docs/API.md.
//!
//! Every builder returns a `serde_json::Value` whose key order, value types
//! (PHP int vs float) and rounding equal the PHP 0.1.0 implementation
//! (`lib/WebPulse/Reports`), because the dashboard is shared and the parity
//! harness diffs the JSON of both implementations.

use std::collections::HashMap;

pub mod data;
pub mod dimension;
pub mod domain_data;
pub mod export;
pub mod health;
pub mod overview;
pub mod php;
pub mod pretty;
pub mod range;
pub mod realtime;
pub mod report;
pub mod visits_log;

/// A validation error of user input (PHP `InvalidArgumentException`): HTTP 400
/// `bad_request` in the API, a usage error (exit 2) in the CLI.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Invalid(pub String);

/// Shorthand for an `Invalid` error wrapped in `anyhow`.
pub fn invalid(msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Invalid(msg.into()))
}

/// Query parameters as PHP's `parse_str()` produces them: a scalar string, or
/// an array (`a[]=1`), which every reader treats as "not a string".
#[derive(Debug, Clone, Default)]
pub struct Query {
    params: HashMap<String, Option<String>>,
}

impl Query {
    /// Empty query.
    pub fn new() -> Query {
        Query::default()
    }

    /// Builds a query from `(name, value)` string pairs (later pairs win).
    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Query {
        let mut q = Query::new();
        for (k, v) in pairs {
            q.set(k, v);
        }
        q
    }

    /// Sets a scalar parameter.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.params.insert(name.into(), Some(value.into()));
    }

    /// Marks a parameter as an array (`name[]=…` / `name[k]=…`).
    pub fn set_array(&mut self, name: impl Into<String>) {
        self.params.insert(name.into(), None);
    }

    /// The parameter when it is a scalar string (`Request::get`).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.params.get(name).and_then(|v| v.as_deref())
    }

    /// The parameter as PHP's `(string)` cast sees it: arrays become `"Array"`.
    pub fn get_any(&self, name: &str) -> Option<&str> {
        self.params
            .get(name)
            .map(|v| v.as_deref().unwrap_or("Array"))
    }

    /// True when the parameter is present (scalar or array; PHP `isset`).
    pub fn has(&self, name: &str) -> bool {
        self.params.contains_key(name)
    }

    /// PHP `parse_str()` of a query string: `+` and `%XX` decoding, `.`/space
    /// in names mangled to `_`, `name[...]` makes an array, last value wins.
    pub fn parse(qs: &str) -> Query {
        let mut q = Query::new();
        for part in qs.split('&') {
            if part.is_empty() {
                continue;
            }
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            let k = urldecode(k);
            let v = urldecode(v);
            let k = k.trim_start_matches(' ');
            if k.is_empty() {
                continue;
            }
            let (base, is_arr) = match k.find('[') {
                Some(i) if i > 0 && k[i..].contains(']') => (&k[..i], true),
                Some(0) => continue,
                _ => (k, false),
            };
            let mut name: String = base
                .chars()
                .map(|c| if c == '.' || c == ' ' { '_' } else { c })
                .collect();
            if !is_arr {
                // an unmatched '[' is also mangled to '_' by PHP
                name = name.replace('[', "_");
            }
            if is_arr {
                q.set_array(name);
            } else {
                q.set(name, v);
            }
        }
        q
    }
}

/// `urldecode()`: `+` → space, `%XX` → byte; invalid UTF-8 is replaced.
pub fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let h = (b[i + 1] as char).to_digit(16);
                let l = (b[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_str_like_php() {
        let q = Query::parse("a=overview&domain=ex%41mple.com&q=a+b&x[]=1&y.z=2&a=report");
        assert_eq!(q.get("a"), Some("report"));
        assert_eq!(q.get("domain"), Some("exAmple.com"));
        assert_eq!(q.get("q"), Some("a b"));
        assert!(q.has("x") && q.get("x").is_none());
        assert_eq!(q.get("y_z"), Some("2"));
        assert_eq!(urldecode("%zz%4"), "%zz%4");
    }
}
