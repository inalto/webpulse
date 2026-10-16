//! String helpers mirroring `WebPulse\Util\Str` (0.1.0): valid UTF-8 enforcement,
//! byte-safe truncation and the PHP string functions the engine depends on
//! (`rawurldecode`, `urldecode`, `parse_str`, `trim`).
//!
//! Everything that comes out of an access log is attacker controlled; every
//! string stored in a database goes through [`clean`] first.

use std::borrow::Cow;
use std::collections::HashMap;

/// The ellipsis appended by [`key`] when a value is cut.
pub const ELLIPSIS: &str = "\u{2026}";

/// Length of the valid UTF-8 sequence starting at `s[0]`, or `None` when the bytes
/// there are not a valid sequence (same rules as PCRE's `//u` check: no overlongs,
/// no surrogates, nothing above U+10FFFF).
fn utf8_seq_len(s: &[u8]) -> Option<usize> {
    let b0 = *s.first()?;
    let cont = |i: usize, lo: u8, hi: u8| s.get(i).map(|&b| b >= lo && b <= hi).unwrap_or(false);
    match b0 {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf if cont(1, 0x80, 0xbf) => Some(2),
        0xe0 if cont(1, 0xa0, 0xbf) && cont(2, 0x80, 0xbf) => Some(3),
        0xe1..=0xec | 0xee | 0xef if cont(1, 0x80, 0xbf) && cont(2, 0x80, 0xbf) => Some(3),
        0xed if cont(1, 0x80, 0x9f) && cont(2, 0x80, 0xbf) => Some(3),
        0xf0 if cont(1, 0x90, 0xbf) && cont(2, 0x80, 0xbf) && cont(3, 0x80, 0xbf) => Some(4),
        0xf1..=0xf3 if cont(1, 0x80, 0xbf) && cont(2, 0x80, 0xbf) && cont(3, 0x80, 0xbf) => Some(4),
        0xf4 if cont(1, 0x80, 0x8f) && cont(2, 0x80, 0xbf) && cont(3, 0x80, 0xbf) => Some(4),
        _ => None,
    }
}

/// Replaces every byte that does not start a valid UTF-8 sequence with `?`
/// (`Str::scrub`: one `?` per invalid byte).
pub fn scrub(s: &[u8]) -> String {
    if let Ok(v) = std::str::from_utf8(s) {
        return v.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match utf8_seq_len(&s[i..]) {
            Some(n) => {
                // the sequence was validated above
                out.push_str(std::str::from_utf8(&s[i..i + n]).unwrap_or("?"));
                i += n;
            }
            None => {
                out.push('?');
                i += 1;
            }
        }
    }
    out
}

fn is_ctrl(b: u8) -> bool {
    b < 0x20 || b == 0x7f
}

/// `Str::clean`: valid UTF-8 with C0 control characters and DEL replaced by `?`.
pub fn clean(s: &[u8]) -> String {
    let mut out = match std::str::from_utf8(s) {
        Ok(v) => {
            if !v.bytes().any(is_ctrl) {
                return v.to_string();
            }
            v.to_string()
        }
        Err(_) => scrub(s),
    };
    if out.bytes().any(is_ctrl) {
        // replacing ASCII bytes with ASCII keeps the string valid UTF-8
        let bytes: Vec<u8> = out
            .bytes()
            .map(|b| if is_ctrl(b) { b'?' } else { b })
            .collect();
        out = String::from_utf8(bytes).unwrap_or_default();
    }
    out
}

/// [`clean`] for a `&str` (only control characters can change).
pub fn clean_str(s: &str) -> Cow<'_, str> {
    if s.bytes().any(is_ctrl) {
        Cow::Owned(clean(s.as_bytes()))
    } else {
        Cow::Borrowed(s)
    }
}

/// `Str::truncate`: at most `max_bytes` bytes of valid UTF-8 `s` without cutting a
/// character; `ellipsis` (counted in the limit) is appended when cut.
pub fn truncate(s: &str, max_bytes: usize, ellipsis: &str) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut cut = max_bytes.saturating_sub(ellipsis.len());
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + ellipsis.len());
    out.push_str(&s[..cut]);
    out.push_str(ellipsis);
    out
}

/// `Str::key`: clean + truncate to `max` bytes with `…` (the dim_daily key rule).
pub fn key(s: &[u8], max: usize) -> String {
    let c = clean(s);
    if c.len() <= max {
        c
    } else {
        truncate(&c, max, ELLIPSIS)
    }
}

/// [`key`] with the default limit of 255 bytes.
pub fn key255(s: &[u8]) -> String {
    key(s, 255)
}

/// `"geoip-update"` → `"GeoipUpdate"` (`Str::studly`).
pub fn studly(name: &str) -> String {
    name.to_ascii_lowercase()
        .split(['-', '_'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// PHP `trim()` with the default character list (`" \t\n\r\0\x0B"`).
pub fn php_trim(s: &[u8]) -> &[u8] {
    let is_ws = |b: &u8| matches!(*b, b' ' | b'\t' | b'\n' | b'\r' | 0 | 0x0b);
    let start = s.iter().position(|b| !is_ws(b)).unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|b| !is_ws(b))
        .map(|p| p + 1)
        .unwrap_or(start);
    &s[start..end.max(start)]
}

/// [`php_trim`] on a `&str`.
pub fn php_trim_str(s: &str) -> &str {
    s.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\0' | '\x0b'))
}

fn hexval(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn url_decode_impl(s: &[u8], plus: bool) -> Cow<'_, [u8]> {
    if !s.contains(&b'%') && !(plus && s.contains(&b'+')) {
        return Cow::Borrowed(s);
    }
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if b == b'+' && plus {
            out.push(b' ');
        } else if b == b'%' && i + 2 < s.len() {
            match (hexval(s[i + 1]), hexval(s[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 3;
                    continue;
                }
                _ => out.push(b),
            }
        } else {
            out.push(b);
        }
        i += 1;
    }
    Cow::Owned(out)
}

/// PHP `rawurldecode()`: `%XX` → byte (`+` is kept).
pub fn rawurldecode(s: &[u8]) -> Cow<'_, [u8]> {
    url_decode_impl(s, false)
}

/// PHP `urldecode()`: `+` → space and `%XX` → byte.
pub fn urldecode(s: &[u8]) -> Cow<'_, [u8]> {
    url_decode_impl(s, true)
}

/// A value of PHP's `parse_str()` result: a string, or an array (`a[]=1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryValue {
    /// Scalar value (raw bytes, may be invalid UTF-8).
    Str(Vec<u8>),
    /// `name[...]=…` — any array.
    Array,
}

/// Top-level variables of PHP's `parse_str()` (last assignment wins).
#[derive(Debug, Clone, Default)]
pub struct QueryVars {
    vars: HashMap<Vec<u8>, QueryValue>,
}

impl QueryVars {
    /// `isset($q[$name])`.
    pub fn isset(&self, name: &str) -> bool {
        self.vars.contains_key(name.as_bytes())
    }
    /// `$q[$name]` when it is a string.
    pub fn get_str(&self, name: &str) -> Option<&[u8]> {
        match self.vars.get(name.as_bytes()) {
            Some(QueryValue::Str(v)) => Some(v),
            _ => None,
        }
    }
    /// Raw access.
    pub fn get(&self, name: &str) -> Option<&QueryValue> {
        self.vars.get(name.as_bytes())
    }
}

/// PHP `parse_str()` (default `arg_separator.input`, `max_input_vars` = 1000):
/// `&`-separated pairs, URL-decoded; in names leading spaces are dropped, `.` and
/// space become `_`, `name[...]` creates an array; the query and every name end at
/// the first NUL byte (C strings in PHP's implementation).
pub fn parse_str(query: &[u8]) -> QueryVars {
    let query = match query.iter().position(|&b| b == 0) {
        Some(p) => &query[..p],
        None => query,
    };
    let mut out = QueryVars::default();
    let mut count = 0;
    for tok in query.split(|&b| b == b'&') {
        if tok.is_empty() {
            continue;
        }
        count += 1;
        if count > 1000 {
            break;
        }
        let (rawname, rawval) = match tok.iter().position(|&b| b == b'=') {
            Some(p) => (&tok[..p], &tok[p + 1..]),
            None => (tok, &tok[tok.len()..]),
        };
        let name = urldecode(rawname);
        let mut name: &[u8] = &name;
        if let Some(p) = name.iter().position(|&b| b == 0) {
            name = &name[..p];
        }
        while let Some((&b' ', rest)) = name.split_first() {
            name = rest;
        }
        let mut var: Vec<u8> = Vec::with_capacity(name.len());
        let mut is_array = false;
        let mut i = 0;
        while i < name.len() {
            let c = name[i];
            if c == b' ' || c == b'.' {
                var.push(b'_');
            } else if c == b'[' {
                if name[i + 1..].contains(&b']') {
                    is_array = true;
                    break;
                }
                // not an index: the '[' and every later ' ', '.', '[' become '_'
                var.push(b'_');
                for &d in &name[i + 1..] {
                    var.push(if d == b' ' || d == b'.' || d == b'[' {
                        b'_'
                    } else {
                        d
                    });
                }
                break;
            } else {
                var.push(c);
            }
            i += 1;
        }
        if var.is_empty() {
            continue;
        }
        let value = if is_array {
            QueryValue::Array
        } else {
            QueryValue::Str(urldecode(rawval).into_owned())
        };
        out.vars.insert(var, value);
    }
    out
}

/// PHP `empty()` of an optional string (`null`, `""` and `"0"` are empty).
pub fn php_empty(s: Option<&str>) -> bool {
    matches!(s, None | Some("") | Some("0"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning() {
        assert_eq!(clean(b"a\xffb"), "a?b");
        assert_eq!(scrub(b"a\xffb"), "a?b");
        assert_eq!(scrub(b"\xc3\xa9\xc3"), "\u{e9}?");
        assert_eq!(clean(b"a\x01b"), "a?b");
        assert_eq!(clean(b"a\x7fb\tc"), "a?b?c");
        assert_eq!(scrub(b"\xed\xa0\x80"), "???");
        assert_eq!(truncate("ab\u{e9}", 3, ""), "ab");
        assert_eq!(truncate("abcdef", 4, ELLIPSIS), "a\u{2026}");
        let k = key("\u{20ac}".repeat(200).as_bytes(), 255);
        assert!(k.len() <= 255 && k.ends_with(ELLIPSIS));
        assert_eq!(studly("geoip-update"), "GeoipUpdate");
    }

    #[test]
    fn url_decoding() {
        assert_eq!(
            &*rawurldecode(b"/caf%C3%A9/a+b%zz%4"),
            "/caf\u{e9}/a+b%zz%4".as_bytes()
        );
        assert_eq!(&*urldecode(b"a+b%41"), b"a bA");
    }

    #[test]
    fn parse_str_like_php() {
        let q = parse_str(b"utm.source=x&utm_medium=a+b&gclid[]=1&q=%E2%82%AC&a[b=1&&=z& lead=1");
        assert_eq!(q.get_str("utm_source"), Some(&b"x"[..]));
        assert_eq!(q.get_str("utm_medium"), Some(&b"a b"[..]));
        assert!(q.isset("gclid") && q.get_str("gclid").is_none());
        assert_eq!(q.get_str("q"), Some("\u{20ac}".as_bytes()));
        assert_eq!(q.get_str("a_b"), Some(&b"1"[..]));
        assert_eq!(q.get_str("lead"), Some(&b"1"[..]));
        let q = parse_str(b"x=1&x[]=2&y[]=1&y=3");
        assert_eq!(q.get("x"), Some(&QueryValue::Array));
        assert_eq!(q.get_str("y"), Some(&b"3"[..]));
    }
}
