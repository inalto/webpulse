//! Small re-implementations of PHP runtime behaviour that the JSON API depends on.
//!
//! The dashboard (ui/*.js) and the parity harness compare the output of this
//! port with the PHP 0.1.0 implementation byte for byte (after `jq -S`), so the
//! rounding of `round()`, the integer/float distinction of PHP values, the
//! number formatting of `number_format()`, PHP 8's "smart" string comparison
//! used by `usort()`/`ksort()` and the string casts of scalars are reproduced
//! here exactly.

use std::cmp::Ordering;

use serde_json::Value;

/// A PHP number: `int` or `float`. JSON-encoded differently (`1` vs `1.0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    /// PHP `int`.
    I(i64),
    /// PHP `float`.
    F(f64),
}

impl Num {
    /// Value as `f64` (PHP's implicit int → float conversion).
    pub fn f(self) -> f64 {
        match self {
            Num::I(i) => i as f64,
            Num::F(f) => f,
        }
    }

    /// PHP loose comparison with zero (`$x == 0`).
    pub fn is_zero(self) -> bool {
        match self {
            Num::I(i) => i == 0,
            Num::F(f) => f == 0.0,
        }
    }

    /// JSON value preserving the int/float distinction.
    pub fn json(self) -> Value {
        match self {
            Num::I(i) => Value::from(i),
            Num::F(f) => fjson(f),
        }
    }
}

/// A float as a JSON value (non-finite values become `0`, as PHP's
/// `JSON_PARTIAL_OUTPUT_ON_ERROR` would; they never occur in practice).
pub fn fjson(f: f64) -> Value {
    serde_json::Number::from_f64(f)
        .map(Value::Number)
        .unwrap_or_else(|| Value::from(0))
}

/// `pow(10, n)` exactly as PHP's `php_intpow10()` (table for n < 23).
fn intpow10(n: i32) -> f64 {
    const P: [f64; 23] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
        1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
    ];
    if (0..23).contains(&n) {
        P[n as usize]
    } else {
        10f64.powi(n)
    }
}

/// PHP 8.4 `round($value, $places)` (mode `PHP_ROUND_HALF_UP`): the value is
/// rounded as the decimal number it prints as, half away from zero, so
/// `round(0.285, 2) == 0.29` and `round(-2.5) == -3.0`.
pub fn round(value: f64, places: i32) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    let exponent = intpow10(places.abs());
    let scaled = if places > 0 {
        value * exponent
    } else {
        value / exponent
    };
    let (mut tmp, tmp2) = if value >= 0.0 {
        let t = scaled.floor();
        (t, t + 1.0)
    } else {
        let t = scaled.ceil();
        (t, t - 1.0)
    };
    if (if places > 0 {
        tmp2 / exponent
    } else {
        tmp2 * exponent
    }) == value
    {
        tmp = tmp2;
    }
    if tmp.abs() >= 1e16 {
        return value;
    }
    // php_round_helper, PHP_ROUND_HALF_UP
    let half = 0.5f64.copysign(tmp);
    let edge = if places > 0 {
        ((tmp + half) / exponent).abs()
    } else {
        ((tmp + half) * exponent).abs()
    };
    if value.abs() >= edge {
        tmp += 1.0f64.copysign(tmp);
    }
    if places > 0 {
        tmp / exponent
    } else {
        tmp * exponent
    }
}

/// `(int) round($x)` for a non-negative ratio.
pub fn round_int(value: f64) -> i64 {
    float_to_int(round(value, 0))
}

/// PHP's `(int)` cast of a float (truncation; NaN/∞ → 0, out of range saturates).
pub fn float_to_int(f: f64) -> i64 {
    if !f.is_finite() {
        return 0;
    }
    f as i64
}

/// `$a / $b` for two PHP ints (exact division keeps an int).
pub fn div(a: i64, b: i64) -> Num {
    if b != 0 && a % b == 0 {
        Num::I(a / b)
    } else {
        Num::F(a as f64 / b as f64)
    }
}

/// Decimal digits and decimal-point position of the shortest round-trip
/// representation of a finite, non-zero `|f|` (what `zend_dtoa(mode 0)` returns).
fn shortest_digits(f: f64) -> (String, i32) {
    let s = format!("{:e}", f.abs());
    let (mant, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() {
        "0".to_string()
    } else {
        digits.to_string()
    };
    let exp: i32 = exp.parse().unwrap_or(0);
    (digits, exp + 1)
}

/// PHP's float → string conversion with `serialize_precision = -1`
/// (`php_gcvt(value, 17, '.', 'e')`), as used by `json_encode()` and `var_export()`:
/// `0.1218`, `1.0e-5`, `1.0e+20`, `100000000000000` (no trailing `.0`).
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "NAN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "INF".into() } else { "-INF".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    let (digits, decpt) = shortest_digits(f);
    let mut out = String::new();
    if f < 0.0 {
        out.push('-');
    }
    let precision = 17;
    if if decpt < 0 {
        decpt < -3
    } else {
        decpt > precision
    } {
        let e = decpt - 1;
        let mut chars = digits.chars();
        out.push(chars.next().unwrap_or('0'));
        out.push('.');
        let rest: String = chars.collect();
        if rest.is_empty() {
            out.push('0');
        } else {
            out.push_str(&rest);
        }
        out.push('e');
        out.push(if e < 0 { '-' } else { '+' });
        out.push_str(&e.abs().to_string());
    } else if decpt < 0 {
        out.push_str("0.");
        for _ in 0..(-decpt) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        let d = digits.as_bytes();
        for i in 0..decpt as usize {
            out.push(if i < d.len() { d[i] as char } else { '0' });
        }
        if (decpt as usize) < d.len() {
            if decpt == 0 {
                out.push('0');
            }
            out.push('.');
            out.push_str(&digits[decpt as usize..]);
        }
    }
    out
}

/// PHP's `(string)` cast of a scalar JSON value (bool → `"1"`/`""`, null → `""`);
/// arrays/objects yield `None` (not scalar).
pub fn scalar_string(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "1".into() } else { String::new() }),
        Value::Number(n) => Some(if let Some(i) = n.as_i64() {
            i.to_string()
        } else if let Some(u) = n.as_u64() {
            u.to_string()
        } else {
            float_repr(n.as_f64().unwrap_or(0.0))
        }),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// True for a PHP "numeric string" (optional surrounding whitespace, sign,
/// digits with optional fraction and exponent).
pub fn is_numeric(s: &str) -> bool {
    numeric_prefix(s)
        .map(|(n, _)| {
            n == s
                .trim_end_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c'])
                .len()
        })
        .unwrap_or(false)
}

/// Length of the leading numeric part of `s` (after leading whitespace) and
/// whether it is an integer literal.
fn numeric_prefix(s: &str) -> Option<(usize, bool)> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let ds = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - ds;
    let mut is_int = true;
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if int_digits > 0 || j > i + 1 {
            is_int = false;
            i = j;
        }
    }
    if int_digits == 0 && is_int {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let es = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > es {
            is_int = false;
            i = j;
        }
    }
    Some((i, is_int))
}

/// PHP's `(int)` cast of a string: the leading numeric part, 0 when none,
/// saturating on overflow.
pub fn intval(s: &str) -> i64 {
    match numeric_prefix(s) {
        None => 0,
        Some((n, true)) => {
            let t = s[..n].trim_start();
            t.parse::<i64>().unwrap_or(if t.starts_with('-') {
                i64::MIN
            } else {
                i64::MAX
            })
        }
        Some((n, false)) => float_to_int(s[..n].trim_start().parse::<f64>().unwrap_or(0.0)),
    }
}

/// PHP 8 comparison of two strings as done by `<=>`, `sort()` and `ksort()`:
/// numerically when both are numeric strings, else byte-wise.
pub fn compare_str(a: &str, b: &str) -> Ordering {
    if is_numeric(a) && is_numeric(b) {
        let x: f64 = a.trim().parse().unwrap_or(0.0);
        let y: f64 = b.trim().parse().unwrap_or(0.0);
        if let (Ok(i), Ok(j)) = (a.trim().parse::<i64>(), b.trim().parse::<i64>()) {
            return i.cmp(&j);
        }
        return x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    }
    a.as_bytes().cmp(b.as_bytes())
}

/// `ucfirst()` (ASCII only, like PHP 8).
pub fn ucfirst(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

/// `substr($s, 0, $n)` on bytes, made valid UTF-8 the way `json_encode`
/// with `JSON_INVALID_UTF8_SUBSTITUTE` would print it.
pub fn substr(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    String::from_utf8_lossy(&s.as_bytes()[..n]).into_owned()
}

/// `mb_substr($s, 0, $n)` (characters).
pub fn mb_substr(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// PHP `trim()` with the default character list.
pub fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r', '\0', '\x0b'])
}

/// `number_format($v, $decimals)` with `.` and `,` separators.
pub fn number_format(v: f64, decimals: usize) -> String {
    let r = round(v, decimals as i32);
    let s = format!("{:.*}", decimals, r.abs());
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s.clone(), None),
    };
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let negative = r < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0');
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&grouped);
    if let Some(f) = frac {
        out.push('.');
        out.push_str(&f);
    }
    out
}

/// `escapeshellarg()`.
pub fn escapeshellarg(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_matches_php() {
        assert_eq!(round(0.285, 2), 0.29);
        assert_eq!(round(1.955, 2), 1.96);
        assert_eq!(round(-2.5, 0), -3.0);
        assert_eq!(round(2.5, 0), 3.0);
        assert_eq!(round(0.12185, 4), 0.1219);
        assert_eq!(round(1.0 / 3.0, 4), 0.3333);
        assert_eq!(round(5.055, 2), 5.06);
        assert!(round(-0.00001, 4).is_sign_negative());
    }

    #[test]
    fn float_strings() {
        assert_eq!(float_repr(0.1218), "0.1218");
        assert_eq!(float_repr(1e-5), "1.0e-5");
        assert_eq!(float_repr(1.5e-7), "1.5e-7");
        assert_eq!(float_repr(1e20), "1.0e+20");
        assert_eq!(float_repr(1e15), "1000000000000000");
        assert_eq!(float_repr(123456789012345.6), "123456789012345.6");
        assert_eq!(float_repr(0.0001), "0.0001");
        assert_eq!(float_repr(-0.0), "-0");
        assert_eq!(float_repr(2.0), "2");
        assert_eq!(float_repr(1.2345678901234568e17), "1.2345678901234568e+17");
    }

    #[test]
    fn casts_and_compare() {
        assert_eq!(intval("404"), 404);
        assert_eq!(intval(" 12abc"), 12);
        assert_eq!(intval("abc"), 0);
        assert_eq!(intval("1e3"), 1000);
        assert_eq!(intval("99999999999999999999"), i64::MAX);
        assert_eq!(compare_str("10", "9"), Ordering::Greater);
        assert_eq!(compare_str("a10", "a9"), Ordering::Less);
        assert_eq!(number_format(1234567.891, 2), "1,234,567.89");
        assert_eq!(number_format(999.5, 0), "1,000");
        assert_eq!(div(4, 2), Num::I(2));
        assert_eq!(div(1, 2), Num::F(0.5));
    }
}
