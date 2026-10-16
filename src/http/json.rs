//! JSON encoder reproducing PHP's `json_encode()` output byte for byte for the
//! flag combinations WebPulse uses (see `lib/WebPulse/Http/Response.php`,
//! `Page.php`, `Util/Json.php` of 0.1.0).
//!
//! Differences from `serde_json::to_string` that matter for parity:
//! * PHP arrays: an empty map and a map keyed `"0".."n-1"` encode as a list;
//! * floats use PHP's `serialize_precision = -1` format (`1.0e-5`), with an
//!   optional `.0` suffix (`JSON_PRESERVE_ZERO_FRACTION`);
//! * U+2028/U+2029 are always escaped; `<`, `>`, `&` optionally (`JSON_HEX_*`);
//!   non-ASCII is either raw (`JSON_UNESCAPED_UNICODE`) or `\uXXXX`;
//! * pretty printing uses four spaces and `"key": value`.

use serde_json::{Map, Value};

use crate::reports::php::float_repr;

/// Encoder options (always `JSON_UNESCAPED_SLASHES`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Flags {
    /// `JSON_UNESCAPED_UNICODE`
    pub unescaped_unicode: bool,
    /// `JSON_HEX_TAG | JSON_HEX_AMP`
    pub hex_tag_amp: bool,
    /// `JSON_PRESERVE_ZERO_FRACTION`
    pub zero_fraction: bool,
    /// `JSON_PRETTY_PRINT`
    pub pretty: bool,
    /// Escape `/` as `\/` (PHP's default, i.e. without `JSON_UNESCAPED_SLASHES`).
    pub escape_slashes: bool,
}

/// Flags of the API envelope (`Response::encode`).
pub const API: Flags = Flags {
    unescaped_unicode: true,
    hex_tag_amp: false,
    zero_fraction: true,
    pretty: false,
    escape_slashes: false,
};
/// Flags of `Util\Json::encode` (registry settings, CLI `--json` without pretty).
pub const PLAIN: Flags = Flags {
    unescaped_unicode: true,
    hex_tag_amp: false,
    zero_fraction: false,
    pretty: false,
    escape_slashes: false,
};
/// Flags of the CLI `--json` output (`Util\Json::encode($v, true)`).
pub const CLI: Flags = Flags {
    unescaped_unicode: true,
    hex_tag_amp: false,
    zero_fraction: false,
    pretty: true,
    escape_slashes: false,
};
/// Flags of the page bootstrap (`Page::JSON_FLAGS`).
pub const PAGE: Flags = Flags {
    unescaped_unicode: false,
    hex_tag_amp: true,
    zero_fraction: false,
    pretty: false,
    escape_slashes: false,
};
/// `json_encode($v)` with PHP's default flags (used in error messages).
pub const DEFAULT: Flags = Flags {
    unescaped_unicode: false,
    hex_tag_amp: false,
    zero_fraction: false,
    pretty: false,
    escape_slashes: true,
};
/// Flags of the user settings file written by the cPanel UI.
pub const SETTINGS_FILE: Flags = Flags {
    unescaped_unicode: false,
    hex_tag_amp: false,
    zero_fraction: false,
    pretty: true,
    escape_slashes: false,
};

/// `json_encode($s)` of a string with PHP's default flags.
pub fn quote(s: &str) -> String {
    encode(&Value::String(s.to_string()), DEFAULT)
}

/// Encodes `v` like PHP's `json_encode($v, flags)`.
pub fn encode(v: &Value, f: Flags) -> String {
    let mut out = String::with_capacity(256);
    write_value(&mut out, v, f, 0);
    out
}

fn is_list(m: &Map<String, Value>) -> bool {
    m.keys().enumerate().all(|(i, k)| *k == i.to_string())
}

fn indent(out: &mut String, f: Flags, depth: usize) {
    if f.pretty {
        out.push('\n');
        for _ in 0..depth {
            out.push_str("    ");
        }
    }
}

fn write_value(out: &mut String, v: &Value, f: Flags, depth: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                let mut s = float_repr(n.as_f64().unwrap_or(0.0));
                if f.zero_fraction
                    && !s.contains('.')
                    && !s.contains('e')
                    && !s.contains('N')
                    && !s.contains('I')
                {
                    s.push_str(".0");
                }
                out.push_str(&s);
            }
        }
        Value::String(s) => write_str(out, s, f),
        Value::Array(a) => write_seq(out, a.iter(), a.len(), f, depth),
        Value::Object(m) => {
            if m.is_empty() || is_list(m) {
                write_seq(out, m.values(), m.len(), f, depth);
                return;
            }
            out.push('{');
            for (i, (k, val)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                indent(out, f, depth + 1);
                write_str(out, k, f);
                out.push(':');
                if f.pretty {
                    out.push(' ');
                }
                write_value(out, val, f, depth + 1);
            }
            indent(out, f, depth);
            out.push('}');
        }
    }
}

fn write_seq<'a>(
    out: &mut String,
    it: impl Iterator<Item = &'a Value>,
    len: usize,
    f: Flags,
    depth: usize,
) {
    out.push('[');
    if len == 0 {
        out.push(']');
        return;
    }
    for (i, val) in it.enumerate() {
        if i > 0 {
            out.push(',');
        }
        indent(out, f, depth + 1);
        write_value(out, val, f, depth + 1);
    }
    indent(out, f, depth);
    out.push(']');
}

fn write_str(out: &mut String, s: &str, f: Flags) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' if f.escape_slashes => out.push_str("\\/"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' if f.hex_tag_amp => out.push_str("\\u003C"),
            '>' if f.hex_tag_amp => out.push_str("\\u003E"),
            '&' if f.hex_tag_amp => out.push_str("\\u0026"),
            c if (c as u32) < 0x20
                || c == '\u{2028}'
                || c == '\u{2029}'
                || (!f.unescaped_unicode && (c as u32) > 0x7f) =>
            {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str("\\u");
                    for shift in [12, 8, 4, 0] {
                        out.push(HEX[((*unit >> shift) & 0xf) as usize] as char);
                    }
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn php_compatible() {
        let v = json!(["a<b>&c/é\u{2028}\u{7f}\u{1}😀", 1.0, 0.5, [], {}, {"0": 1, "1": 2}, {"a": null}]);
        assert_eq!(
            encode(&v, PAGE),
            "[\"a\\u003Cb\\u003E\\u0026c/\\u00e9\\u2028\u{7f}\\u0001\\ud83d\\ude00\",1,0.5,[],[],[1,2],{\"a\":null}]"
        );
        assert_eq!(
            encode(&json!(["é\t", 2.0, -0.0]), API),
            "[\"é\\t\",2.0,-0.0]"
        );
        assert_eq!(
            encode(&json!({"a": [], "b": {"c": 1}}), CLI),
            "{\n    \"a\": [],\n    \"b\": {\n        \"c\": 1\n    }\n}"
        );
    }
}
