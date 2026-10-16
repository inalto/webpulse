//! Tolerant parser for Apache/Nginx "combined" lines (`WebPulse\Log\LogParser`):
//!
//! ```text
//! [%v[:%p] ]%h %l %u %t "%r" %>s %b "%{Referer}i" "%{User-Agent}i"[ extra…]
//! ```
//!
//! Hand-written equivalent of the 0.1.0 regex
//! `^(?:(\S+) )?(\S+) \S+ (?:"…"|\S+) \[(\d\d/[A-Za-z]{3}/\d{4}:\d\d:\d\d:\d\d) ([+-]\d{4})\] "(…)" (\d{1,3}|-) (\d+|-)(?: "(…)"(?: "(…)")?)?`
//! including its backtracking order (with a vhost first, then without; a quoted
//! user first, then a bare token), so it accepts exactly the same lines and
//! captures the same fields. Fields are returned as spans into the line (or into a
//! scratch buffer when Apache escapes had to be decoded): no allocation per line.

use foldhash::HashMap;

/// A byte range in the line (`scratch == false`) or in the scratch buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    start: u32,
    end: u32,
    scratch: bool,
}

impl Span {
    fn line(start: usize, end: usize) -> Span {
        Span {
            start: start as u32,
            end: end as u32,
            scratch: false,
        }
    }
    fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// A parsed line: numeric fields plus spans of the string fields.
#[derive(Debug, Clone, Copy, Default)]
pub struct HitRef {
    ip: Span,
    method: Span,
    target: Span,
    protocol: Span,
    referrer: Option<Span>,
    ua: Span,
    vhost: Option<Span>,
    /// Unix timestamp of the request.
    pub ts: i64,
    /// UTC offset of the logged time, seconds.
    pub offset: i32,
    /// HTTP status (0 when logged as `-`).
    pub status: i64,
    /// Response size (`-` = 0).
    pub bytes: i64,
}

/// A parsed line with access to its string fields (`WebPulse\Log\Hit`).
#[derive(Debug, Clone, Copy)]
pub struct Hit<'a> {
    line: &'a [u8],
    scratch: &'a [u8],
    r: &'a HitRef,
}

impl<'a> Hit<'a> {
    /// Binds a [`HitRef`] to the buffers it was parsed from.
    pub fn new(line: &'a [u8], scratch: &'a [u8], r: &'a HitRef) -> Hit<'a> {
        Hit { line, scratch, r }
    }
    fn get(&self, s: Span) -> &'a [u8] {
        let buf = if s.scratch { self.scratch } else { self.line };
        &buf[s.start as usize..s.end as usize]
    }
    /// Client address (`%h`).
    pub fn ip(&self) -> &'a [u8] {
        self.get(self.r.ip)
    }
    /// Upper-case method, empty when the request line is malformed.
    pub fn method(&self) -> &'a [u8] {
        self.get(self.r.method)
    }
    /// Raw request target, empty when malformed.
    pub fn target(&self) -> &'a [u8] {
        self.get(self.r.target)
    }
    /// `HTTP/1.1`, `HTTP/2.0`… (empty when absent).
    pub fn protocol(&self) -> &'a [u8] {
        self.get(self.r.protocol)
    }
    /// Referer header (`None` when `-` or empty).
    pub fn referrer(&self) -> Option<&'a [u8]> {
        self.r.referrer.map(|s| self.get(s))
    }
    /// User-Agent (empty when `-`).
    pub fn ua(&self) -> &'a [u8] {
        self.get(self.r.ua)
    }
    /// Leading `%v` field when present.
    pub fn vhost(&self) -> Option<&'a [u8]> {
        self.r.vhost.map(|s| self.get(s))
    }
    /// Unix timestamp.
    pub fn ts(&self) -> i64 {
        self.r.ts
    }
    /// UTC offset of the logged time (seconds).
    pub fn offset(&self) -> i32 {
        self.r.offset
    }
    /// HTTP status.
    pub fn status(&self) -> i64 {
        self.r.status
    }
    /// Response size.
    pub fn bytes(&self) -> i64 {
        self.r.bytes
    }
    /// False when the request line could not be split into method/target/protocol.
    pub fn request_valid(&self) -> bool {
        !self.r.method.is_empty()
    }
}

/// PCRE `\s` (non-UTF mode): space, \t, \n, \v, \f, \r.
#[inline]
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `\S+` from `i`: end index, `None` when empty.
#[inline]
fn token(s: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    while j < s.len() && !is_ws(s[j]) {
        j += 1;
    }
    (j > i).then_some(j)
}

/// `"[^"\\]*(?:\\.[^"\\]*)*"` at `i`: index after the closing quote.
#[inline]
fn quoted(s: &[u8], i: usize) -> Option<usize> {
    if s.get(i) != Some(&b'"') {
        return None;
    }
    let mut j = i + 1;
    loop {
        j += memchr::memchr2(b'"', b'\\', s.get(j..)?)?;
        if s[j] == b'"' {
            return Some(j + 1);
        }
        if j + 1 >= s.len() || s[j + 1] == b'\n' {
            return None;
        }
        j += 2;
    }
}

#[inline]
fn at(s: &[u8], i: usize, c: u8) -> bool {
    s.get(i) == Some(&c)
}

#[inline]
fn digit(s: &[u8], i: usize) -> bool {
    s.get(i).map(|b| b.is_ascii_digit()).unwrap_or(false)
}

struct Raw {
    vhost: Option<(usize, usize)>,
    ip: (usize, usize),
    time: usize,
    tz: usize,
    req: (usize, usize),
    status: (usize, usize),
    bytes: (usize, usize),
    referrer: Option<(usize, usize)>,
    ua: Option<(usize, usize)>,
}

fn try_match(s: &[u8], with_vhost: bool, quoted_user: bool) -> Option<Raw> {
    let mut pos = 0;
    let mut vhost = None;
    if with_vhost {
        let e = token(s, pos)?;
        if !at(s, e, b' ') {
            return None;
        }
        vhost = Some((pos, e));
        pos = e + 1;
    }
    let e = token(s, pos)?;
    if !at(s, e, b' ') {
        return None;
    }
    let ip = (pos, e);
    pos = e + 1;
    let e = token(s, pos)?;
    if !at(s, e, b' ') {
        return None;
    }
    pos = e + 1;
    let e = if quoted_user {
        quoted(s, pos)?
    } else {
        token(s, pos)?
    };
    if !at(s, e, b' ') {
        return None;
    }
    pos = e + 1;
    // \[(\d\d/[A-Za-z]{3}/\d{4}:\d\d:\d\d:\d\d) ([+-]\d{4})\] "
    if !at(s, pos, b'[') || s.len() < pos + 31 {
        return None;
    }
    let t = pos + 1;
    let tb = &s[t..t + 20];
    let ok = tb[0].is_ascii_digit()
        && tb[1].is_ascii_digit()
        && tb[2] == b'/'
        && tb[3..6].iter().all(u8::is_ascii_alphabetic)
        && tb[6] == b'/'
        && tb[7..11].iter().all(u8::is_ascii_digit)
        && tb[11] == b':'
        && tb[12..14].iter().all(u8::is_ascii_digit)
        && tb[14] == b':'
        && tb[15..17].iter().all(u8::is_ascii_digit)
        && tb[17] == b':'
        && tb[18..20].iter().all(u8::is_ascii_digit);
    if !ok {
        return None;
    }
    let z = t + 21;
    if s[t + 20] != b' '
        || !(s[z] == b'+' || s[z] == b'-')
        || !s[z + 1..z + 5].iter().all(u8::is_ascii_digit)
        || s[z + 5] != b']'
        || s[z + 6] != b' '
    {
        return None;
    }
    pos = z + 7;
    let qe = quoted(s, pos)?;
    let req = (pos + 1, qe - 1);
    pos = qe;
    if !at(s, pos, b' ') {
        return None;
    }
    pos += 1;
    // (\d{1,3}|-) then a space
    let status;
    let mut k = 0;
    while k < 3 && digit(s, pos + k) {
        k += 1;
    }
    if k > 0 {
        if !at(s, pos + k, b' ') {
            return None;
        }
        status = (pos, pos + k);
        pos += k + 1;
    } else if at(s, pos, b'-') && at(s, pos + 1, b' ') {
        status = (pos, pos + 1);
        pos += 2;
    } else {
        return None;
    }
    // (\d+|-)
    let bytes;
    let mut k = 0;
    while digit(s, pos + k) {
        k += 1;
    }
    if k > 0 {
        bytes = (pos, pos + k);
        pos += k;
    } else if at(s, pos, b'-') {
        bytes = (pos, pos + 1);
        pos += 1;
    } else {
        return None;
    }
    // (?: "(ref)"(?: "(ua)")?)?
    let mut referrer = None;
    let mut ua = None;
    if at(s, pos, b' ')
        && let Some(e) = quoted(s, pos + 1)
    {
        referrer = Some((pos + 2, e - 1));
        pos = e;
        if at(s, pos, b' ')
            && let Some(e2) = quoted(s, pos + 1)
        {
            ua = Some((pos + 2, e2 - 1));
        }
    }
    Some(Raw {
        vhost,
        ip,
        time: t,
        tz: z,
        req,
        status,
        bytes,
        referrer,
        ua,
    })
}

fn digits_to_i64(b: &[u8]) -> i64 {
    let mut n: i64 = 0;
    for &c in b {
        n = n.saturating_mul(10).saturating_add((c - b'0') as i64);
    }
    n
}

/// `+0200` → 7200
fn offset_seconds(z: &[u8]) -> i32 {
    let hh = digits_to_i64(&z[1..3]) as i32;
    let mm = digits_to_i64(&z[3..5]) as i32;
    let s = hh * 3600 + mm * 60;
    if z[0] == b'-' { -s } else { s }
}

fn days_in_month(m: u32, y: i64) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// Unix day number of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Apache's escaping undone (`\"`, `\\`, `\xhh`, `\n`, `\t`), appended to `out`.
pub fn unescape_into(s: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c == b'\\' && i + 1 < s.len() {
            let n = s[i + 1];
            if n == b'x'
                && i + 3 < s.len()
                && s[i + 2].is_ascii_hexdigit()
                && s[i + 3].is_ascii_hexdigit()
            {
                let h = (s[i + 2] as char).to_digit(16).unwrap_or(0) as u8;
                let l = (s[i + 3] as char).to_digit(16).unwrap_or(0) as u8;
                out.push(h * 16 + l);
                i += 4;
                continue;
            }
            out.push(match n {
                b'n' => b'\n',
                b't' => b'\t',
                other => other,
            });
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
}

/// Apache's escaping undone (owned result).
pub fn unescape(s: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len());
    unescape_into(s, &mut v);
    v
}

fn is_method(m: &[u8]) -> bool {
    !m.is_empty()
        && m.len() <= 20
        && m[0].is_ascii_uppercase()
        && m.iter()
            .all(|&c| c.is_ascii_uppercase() || c == b'_' || c == b'-')
}

/// HTTP/1.0, HTTP/1.1, HTTP/2, HTTP/2.0, HTTP/3 (any case).
fn is_protocol(p: &[u8]) -> bool {
    let l = p.len();
    (l == 6 || l == 8)
        && p[..5].eq_ignore_ascii_case(b"HTTP/")
        && p[5].is_ascii_digit()
        && (l == 6 || (p[6] == b'.' && p[7].is_ascii_digit()))
}

/// The parser (holds small date caches).
#[derive(Debug, Default)]
pub struct LogParser {
    day_cache: HashMap<[u8; 11], Option<i64>>,
    last: Option<([u8; 20], [u8; 5], Option<i64>)>,
}

impl LogParser {
    /// A new parser.
    pub fn new() -> LogParser {
        LogParser::default()
    }

    fn timestamp(&mut self, t: &[u8], z: &[u8]) -> Option<i64> {
        if let Some((lt, lz, ts)) = &self.last
            && lt[..] == *t
            && lz[..] == *z
        {
            return *ts;
        }
        let ts = self.compute_timestamp(t, z);
        let mut lt = [0u8; 20];
        lt.copy_from_slice(t);
        let mut lz = [0u8; 5];
        lz.copy_from_slice(z);
        self.last = Some((lt, lz, ts));
        ts
    }

    fn compute_timestamp(&mut self, t: &[u8], z: &[u8]) -> Option<i64> {
        let mut dkey = [0u8; 11];
        dkey.copy_from_slice(&t[..11]);
        let base = match self.day_cache.get(&dkey) {
            Some(b) => *b,
            None => {
                if self.day_cache.len() > 4096 {
                    self.day_cache.clear();
                }
                let mon = match &t[3..6].to_ascii_lowercase()[..] {
                    b"jan" => 1,
                    b"feb" => 2,
                    b"mar" => 3,
                    b"apr" => 4,
                    b"may" => 5,
                    b"jun" => 6,
                    b"jul" => 7,
                    b"aug" => 8,
                    b"sep" => 9,
                    b"oct" => 10,
                    b"nov" => 11,
                    b"dec" => 12,
                    _ => 0,
                };
                let d = digits_to_i64(&t[0..2]) as u32;
                let y = digits_to_i64(&t[7..11]);
                let b =
                    if mon != 0 && (1..=32767).contains(&y) && d >= 1 && d <= days_in_month(mon, y)
                    {
                        Some(days_from_civil(y, mon, d) * 86400)
                    } else {
                        None
                    };
                self.day_cache.insert(dkey, b);
                b
            }
        }?;
        let h = digits_to_i64(&t[12..14]);
        let i = digits_to_i64(&t[15..17]);
        let s = digits_to_i64(&t[18..20]);
        if h > 23 || i > 59 || s > 60 {
            return None;
        }
        Some(base + h * 3600 + i * 60 + s - offset_seconds(z) as i64)
    }

    /// Parses one line; `None` when it is not a combined-format line. Decoded
    /// escapes are written to `scratch` (cleared first).
    pub fn parse(&mut self, line: &[u8], scratch: &mut Vec<u8>) -> Option<HitRef> {
        scratch.clear();
        let raw = try_match(line, true, true)
            .or_else(|| try_match(line, true, false))
            .or_else(|| try_match(line, false, true))
            .or_else(|| try_match(line, false, false))?;
        let ts = self.timestamp(&line[raw.time..raw.time + 20], &line[raw.tz..raw.tz + 5])?;
        let mut h = HitRef {
            ip: Span::line(raw.ip.0, raw.ip.1),
            vhost: raw.vhost.map(|v| Span::line(v.0, v.1)),
            ts,
            offset: offset_seconds(&line[raw.tz..raw.tz + 5]),
            status: if line[raw.status.0] == b'-' {
                0
            } else {
                digits_to_i64(&line[raw.status.0..raw.status.1])
            },
            bytes: if line[raw.bytes.0] == b'-' {
                0
            } else {
                digits_to_i64(&line[raw.bytes.0..raw.bytes.1])
            },
            ..HitRef::default()
        };
        // request line, possibly unescaped into scratch
        let (req_buf_is_scratch, req_start, req_end) = {
            let r = &line[raw.req.0..raw.req.1];
            if r.contains(&b'\\') {
                let s0 = scratch.len();
                unescape_into(r, scratch);
                (true, s0, scratch.len())
            } else {
                (false, raw.req.0, raw.req.1)
            }
        };
        {
            let buf: &[u8] = if req_buf_is_scratch { scratch } else { line };
            let req = &buf[req_start..req_end];
            // explode(' ', $req, 4)
            let mut parts: [(usize, usize); 4] = [(0, 0); 4];
            let mut n = 0;
            let mut st = 0;
            for (i, &c) in req.iter().enumerate() {
                if c == b' ' && n < 3 {
                    parts[n] = (st, i);
                    n += 1;
                    st = i + 1;
                }
            }
            parts[n] = (st, req.len());
            n += 1;
            let part = |k: usize| &req[parts[k].0..parts[k].1];
            if (n == 3 || n == 2)
                && !part(1).is_empty()
                && is_method(part(0))
                && (n == 2 || is_protocol(part(2)))
            {
                let mk = |k: usize| Span {
                    start: (req_start + parts[k].0) as u32,
                    end: (req_start + parts[k].1) as u32,
                    scratch: req_buf_is_scratch,
                };
                h.method = mk(0);
                h.target = mk(1);
                if n == 3 {
                    let p = part(2);
                    if p.iter().any(u8::is_ascii_lowercase) {
                        let up = p.to_ascii_uppercase();
                        let s0 = scratch.len();
                        scratch.extend_from_slice(&up);
                        h.protocol = Span {
                            start: s0 as u32,
                            end: scratch.len() as u32,
                            scratch: true,
                        };
                    } else {
                        h.protocol = mk(2);
                    }
                }
            }
        }
        // referrer and user agent
        let field = |span: Option<(usize, usize)>, scratch: &mut Vec<u8>| -> Span {
            match span {
                None => Span::default(),
                Some((a, b)) => {
                    let v = &line[a..b];
                    if !v.is_empty() && v.contains(&b'\\') {
                        let s0 = scratch.len();
                        unescape_into(v, scratch);
                        Span {
                            start: s0 as u32,
                            end: scratch.len() as u32,
                            scratch: true,
                        }
                    } else {
                        Span::line(a, b)
                    }
                }
            }
        };
        let rs = field(raw.referrer, scratch);
        let us = field(raw.ua, scratch);
        let sc: &[u8] = scratch;
        let get = |s: Span| -> &[u8] {
            let b: &[u8] = if s.scratch { sc } else { line };
            &b[s.start as usize..s.end as usize]
        };
        let rv = get(rs);
        h.referrer = if rv.is_empty() || rv == b"-" {
            None
        } else {
            Some(rs)
        };
        h.ua = if get(us) == b"-" { Span::default() } else { us };
        Some(h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: &str) -> Option<(HitRef, Vec<u8>)> {
        let mut s = Vec::new();
        LogParser::new()
            .parse(line.as_bytes(), &mut s)
            .map(|h| (h, s))
    }

    #[test]
    fn combined_lines() {
        let l = r#"203.0.113.5 - - [10/Oct/2026:13:55:36 +0200] "GET /pricing?utm_source=news HTTP/1.1" 200 2326 "https://mail.google.com/" "Mozilla/5.0 Chrome/129""#;
        let (h, s) = p(l).unwrap();
        let hit = Hit::new(l.as_bytes(), &s, &h);
        assert_eq!(hit.ip(), b"203.0.113.5");
        assert_eq!(hit.ts(), 1_791_633_336);
        assert_eq!(hit.offset(), 7200);
        assert_eq!(hit.method(), b"GET");
        assert_eq!(hit.target(), b"/pricing?utm_source=news");
        assert_eq!(hit.protocol(), b"HTTP/1.1");
        assert_eq!(hit.referrer(), Some(&b"https://mail.google.com/"[..]));
        assert!(hit.vhost().is_none());
        let l2 = r#"example.com:443 2001:db8::1 - - [10/Oct/2026:13:55:37 +0200] "GET / http/2.0" 200 5120 "-" "-""#;
        let (h, s) = p(l2).unwrap();
        let hit = Hit::new(l2.as_bytes(), &s, &h);
        assert_eq!(hit.vhost(), Some(&b"example.com:443"[..]));
        assert_eq!(hit.protocol(), b"HTTP/2.0");
        assert!(hit.referrer().is_none());
        assert_eq!(hit.ua(), b"");
        assert!(
            p("1.2.3.4 - - [31/Feb/2026:13:55:45 +0200] \"GET / HTTP/1.1\" 200 100 \"-\" \"x\"")
                .is_none()
        );
        assert!(p("1.2.3.4 - - [10/Oct/2026:13:55:36 +0200] \"GET / HTTP/1.1\"").is_none());
        let l3 = r#"1.2.3.4 - - [10/Oct/2026:13:55:36 +0000] "GET /say \"hi\" HTTP/1.1" 404 196 "-" "Agent with \"quotes\" inside""#;
        let (h, s) = p(l3).unwrap();
        let hit = Hit::new(l3.as_bytes(), &s, &h);
        assert!(!hit.request_valid());
        assert_eq!(hit.ua(), br#"Agent with "quotes" inside"#);
    }
}
