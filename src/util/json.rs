//! JSON encoding identical to PHP's `json_encode()` for the flag sets WebPulse
//! 0.1.0 used in the engine and the CLI (`WebPulse\Util\Json`):
//!
//! * `JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE` ([`encode`], [`encode_pretty`]);
//! * PHP's defaults (`\/` and `\uXXXX` escapes, [`encode_php_default`]), used in
//!   validation messages such as `invalid IP/CIDR "x"`.
//!
//! Differences from `serde_json` that matter: four-space pretty printing,
//! `U+2028`/`U+2029` always escaped, floats in PHP's `serialize_precision = -1`
//! format (`1.0e+25`, `3.0e-5`, `1` for `1.0`).

use serde_json::Value;

/// Encoder options.
#[derive(Debug, Clone, Copy)]
pub struct Opts {
    /// `JSON_PRETTY_PRINT` (four spaces, `"key": value`).
    pub pretty: bool,
    /// Escape `/` as `\/` (PHP default; off with `JSON_UNESCAPED_SLASHES`).
    pub escape_slashes: bool,
    /// Escape non-ASCII as `\uXXXX` (PHP default; off with `JSON_UNESCAPED_UNICODE`).
    pub escape_unicode: bool,
}

/// `Util\Json::encode($v)`.
pub const FLAGS: Opts = Opts {
    pretty: false,
    escape_slashes: false,
    escape_unicode: false,
};
/// `Util\Json::encode($v, true)` (CLI `--json` output).
pub const PRETTY: Opts = Opts {
    pretty: true,
    escape_slashes: false,
    escape_unicode: false,
};
/// `json_encode($v)` without flags.
pub const PHP_DEFAULT: Opts = Opts {
    pretty: false,
    escape_slashes: true,
    escape_unicode: true,
};

/// Compact encoding with `Util\Json::FLAGS`.
pub fn encode(v: &Value) -> String {
    encode_with(v, FLAGS)
}

/// Pretty encoding with `Util\Json::FLAGS` (what `--json` prints).
pub fn encode_pretty(v: &Value) -> String {
    encode_with(v, PRETTY)
}

/// `json_encode($v)` with PHP's default flags.
pub fn encode_php_default(v: &Value) -> String {
    encode_with(v, PHP_DEFAULT)
}

/// Encodes `v` with the given options.
pub fn encode_with(v: &Value, o: Opts) -> String {
    let mut out = String::new();
    write_value(&mut out, v, o, 0);
    out
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("    ");
    }
}

fn write_value(out: &mut String, v: &Value, o: Opts, level: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                out.push_str(&php_float(n.as_f64().unwrap_or(0.0)));
            }
        }
        Value::String(s) => write_str(out, s, o),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if o.pretty {
                    out.push('\n');
                    indent(out, level + 1);
                }
                write_value(out, e, o, level + 1);
            }
            if o.pretty {
                out.push('\n');
                indent(out, level);
            }
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, e)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if o.pretty {
                    out.push('\n');
                    indent(out, level + 1);
                }
                write_str(out, k, o);
                out.push(':');
                if o.pretty {
                    out.push(' ');
                }
                write_value(out, e, o, level + 1);
            }
            if o.pretty {
                out.push('\n');
                indent(out, level);
            }
            out.push('}');
        }
    }
}

fn write_str(out: &mut String, s: &str, o: Opts) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' if o.escape_slashes => out.push_str("\\/"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if o.escape_unicode && (c as u32) > 0x7f => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A float as PHP prints it with `serialize_precision = -1` (shortest round-trip
/// digits; scientific notation below 1e-4 and from 1e17 on; no `.0` suffix).
pub fn php_float(f: f64) -> String {
    if f.is_nan() || f.is_infinite() {
        return "0".to_string();
    }
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    let sci = format!("{:e}", f.abs());
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let neg = if f < 0.0 { "-" } else { "" };
    let decpt = exp + 1;
    let nd = digits.len() as i32;
    if !(-3..=17).contains(&decpt) {
        let rest = if digits.len() > 1 { &digits[1..] } else { "0" };
        let sign = if exp < 0 { "-" } else { "+" };
        return format!("{neg}{}.{rest}e{sign}{}", &digits[..1], exp.abs());
    }
    if decpt <= 0 {
        format!("{neg}0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if decpt >= nd {
        format!("{neg}{digits}{}", "0".repeat((decpt - nd) as usize))
    } else {
        format!(
            "{neg}{}.{}",
            &digits[..decpt as usize],
            &digits[decpt as usize..]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn floats_like_php() {
        let cases = [
            (1.0, "1"),
            (0.5, "0.5"),
            (1e25, "1.0e+25"),
            (1.5e-7, "1.5e-7"),
            (0.1 + 0.2, "0.30000000000000004"),
            (123456789012345678.0, "1.2345678901234568e+17"),
            (1e15, "1000000000000000"),
            (1e16, "10000000000000000"),
            (3.0e-5, "3.0e-5"),
            (0.0001, "0.0001"),
            (100.0, "100"),
            (0.012, "0.012"),
            (-2.5, "-2.5"),
        ];
        for (f, s) in cases {
            assert_eq!(php_float(f), s, "{f}");
        }
    }

    #[test]
    fn strings_and_pretty() {
        let v = json!(["a/b", "é", "\u{2028}", "\u{1}\u{1f}\u{7f}"]);
        assert_eq!(
            encode_php_default(&v),
            "[\"a\\/b\",\"\\u00e9\",\"\\u2028\",\"\\u0001\\u001f\u{7f}\"]"
        );
        assert_eq!(
            encode(&json!({"a": "x/y", "b": []})),
            "{\"a\":\"x/y\",\"b\":[]}"
        );
        assert_eq!(
            encode_pretty(&json!([[], {}, {"a": []}])),
            "[\n    [],\n    {},\n    {\n        \"a\": []\n    }\n]"
        );
    }
}
