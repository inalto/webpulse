//! Port of tests/cases/engine_parser.php.

mod common;

use webpulse::engine::parser::{Hit, HitRef, LogParser};
use webpulse::util::strs;

fn fixture() -> Vec<Vec<u8>> {
    let raw = std::fs::read(common::repo().join("tests/fixtures/logs/edge_cases.log")).unwrap();
    let mut v: Vec<Vec<u8>> = raw.split(|&b| b == b'\n').map(|l| l.to_vec()).collect();
    if v.last().map(|l| l.is_empty()).unwrap_or(false) {
        v.pop();
    }
    v
}

struct P {
    line: Vec<u8>,
    scratch: Vec<u8>,
    hit: Option<HitRef>,
}

impl P {
    fn hit(&self) -> Hit<'_> {
        Hit::new(
            &self.line,
            &self.scratch,
            self.hit.as_ref().expect("parsed"),
        )
    }
}

fn parse(line: &[u8]) -> P {
    let mut scratch = Vec::new();
    let hit = LogParser::new().parse(line, &mut scratch);
    P {
        line: line.to_vec(),
        scratch,
        hit,
    }
}

fn gm(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    chrono::NaiveDate::from_ymd_opt(y, mo, d)
        .unwrap()
        .and_hms_opt(h, mi, s)
        .unwrap()
        .and_utc()
        .timestamp()
}

#[test]
fn combined_line_with_utm_query() {
    let p = parse(&fixture()[0]);
    let h = p.hit();
    assert_eq!(h.ip(), b"203.0.113.5");
    assert_eq!(h.ts(), gm(2026, 10, 10, 11, 55, 36));
    assert_eq!(h.offset(), 7200);
    assert_eq!(h.method(), b"GET");
    assert_eq!(h.target(), b"/pricing?utm_source=news&utm_medium=email");
    assert_eq!(h.protocol(), b"HTTP/1.1");
    assert_eq!(h.status(), 200);
    assert_eq!(h.bytes(), 2326);
    assert_eq!(h.referrer(), Some(&b"https://mail.google.com/"[..]));
    assert!(String::from_utf8_lossy(h.ua()).contains("Chrome/129"));
    assert!(h.vhost().is_none());
}

#[test]
fn leading_vhost_with_port_and_ipv6_client() {
    let p = parse(&fixture()[1]);
    let h = p.hit();
    assert_eq!(h.vhost(), Some(&b"example.com:443"[..]));
    assert_eq!(h.ip(), b"2001:db8:85a3::8a2e:370:7334");
    assert_eq!(h.protocol(), b"HTTP/2.0");
    assert!(h.referrer().is_none());
}

#[test]
fn leading_vhost_without_port_dash_size() {
    let p = parse(&fixture()[2]);
    let h = p.hit();
    assert_eq!(h.vhost(), Some(&b"example.com"[..]));
    assert_eq!(h.ip(), b"198.51.100.7");
    assert_eq!(h.status(), 304);
    assert_eq!(h.bytes(), 0);
}

#[test]
fn escaped_quotes_in_request_and_ua() {
    let p = parse(&fixture()[3]);
    let h = p.hit();
    assert_eq!(h.ua(), b"Agent with \"quotes\" inside");
    assert_eq!(h.status(), 404);
    assert!(!h.request_valid());
}

#[test]
fn tls_garbage_request_line_is_a_hit_with_empty_method() {
    let p = parse(&fixture()[4]);
    let h = p.hit();
    assert_eq!(h.method(), b"");
    assert_eq!(h.status(), 400);
    assert_eq!(h.ua(), b"");
}

#[test]
fn dash_request_line_408() {
    let p = parse(&fixture()[5]);
    let h = p.hit();
    assert_eq!(h.method(), b"");
    assert_eq!(h.status(), 408);
    assert_eq!(h.bytes(), 0);
}

#[test]
fn common_log_format_without_referrer_and_ua() {
    let p = parse(&fixture()[6]);
    let h = p.hit();
    assert!(h.referrer().is_none());
    assert_eq!(h.ua(), b"");
}

#[test]
fn extra_trailing_fields_are_ignored() {
    let p = parse(&fixture()[7]);
    let h = p.hit();
    assert_eq!(h.target(), b"/extra");
    assert_eq!(h.ua(), b"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)");
}

#[test]
fn binary_escapes_are_decoded_and_later_cleaned() {
    let p = parse(&fixture()[9]);
    let h = p.hit();
    assert_eq!(h.target(), b"/bin\xff\xfe");
    assert_eq!(strs::clean(h.ua()), "Mozilla/5.0 ?? garbage");
    assert!(std::str::from_utf8(strs::clean(h.referrer().unwrap()).as_bytes()).is_ok());
}

#[test]
fn garbage_and_impossible_dates_are_rejected() {
    let f = fixture();
    assert!(parse(&f[10]).hit.is_none());
    assert!(parse(&f[11]).hit.is_none(), "31 Feb");
    assert!(parse(&f[12]).hit.is_none(), "hour 25");
    assert!(parse(b"").hit.is_none());
    assert!(
        parse(b"1.2.3.4 - - [10/Oct/2026:13:55:36 +0200] \"GET / HTTP/1.1\"")
            .hit
            .is_none()
    );
}

#[test]
fn unknown_methods_and_options_star() {
    let f = fixture();
    assert_eq!(parse(&f[13]).hit().method(), b"PROPFIND");
    let p = parse(&f[14]);
    let h = p.hit();
    assert_eq!(h.method(), b"OPTIONS");
    assert_eq!(h.target(), b"*");
    assert_eq!(h.offset(), -25200);
    assert_eq!(h.ts(), gm(2026, 10, 10, 20, 55, 47));
}

#[test]
fn every_fixture_line_yields_a_hit_or_none() {
    let ok = fixture().iter().filter(|l| parse(l).hit.is_some()).count();
    assert_eq!(ok, 12);
}

#[test]
fn http09_style_request_without_protocol() {
    let p = parse(b"1.2.3.4 - - [10/Oct/2026:13:55:36 +0000] \"GET /old\" 200 5 \"-\" \"x\"");
    let h = p.hit();
    assert_eq!(h.method(), b"GET");
    assert_eq!(h.target(), b"/old");
    assert_eq!(h.protocol(), b"");
}

#[test]
fn very_long_request_line() {
    let path = format!("/{}", "a".repeat(20000));
    let p = parse(format!("1.2.3.4 - - [10/Oct/2026:13:55:36 +0000] \"GET {path} HTTP/1.1\" 414 0 \"-\" \"Mozilla/5.0\"").as_bytes());
    assert_eq!(p.hit().target().len(), 20001);
}

#[test]
fn backtracking_order_matches_the_regex() {
    // with a leading token the vhost alternative is tried first (PCRE order)
    let p = parse(b"1.2.3.4 - \"a b\" [10/Oct/2026:13:55:36 +0000] \"GET / HTTP/1.1\" 200 5");
    let h = p.hit();
    assert_eq!(h.vhost(), Some(&b"1.2.3.4"[..]));
    assert_eq!(h.ip(), b"-");
    // a 4-digit status is not a status
    assert!(
        parse(b"1.2.3.4 - - [10/Oct/2026:13:55:36 +0000] \"GET / HTTP/1.1\" 2000 5")
            .hit
            .is_none()
    );
    // a quoted user without space
    let p = parse(b"1.2.3.4 - \"bob\" [10/Oct/2026:13:55:36 +0000] \"GET / HTTP/1.1\" 200 -");
    assert!(p.hit().vhost().is_none());
}

/// Same results as the PHP regex for every fixture line and a set of mutations.
#[test]
fn matches_the_php_implementation_when_available() {
    if std::process::Command::new("php")
        .arg("-v")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
        || !common::repo().join("lib/autoload.php").is_file()
    {
        eprintln!("php or lib/ not available: skipped");
        return;
    }
    let mut lines = fixture();
    let base = fixture()[0].clone();
    for cut in [10, 30, 60, 90, 120, 150, 200, base.len() - 1] {
        lines.push(base[..cut.min(base.len())].to_vec());
    }
    lines.push(b"a b c d [10/oct/2026:13:55:36 -0130] \"x\" - 12abc \"r\" \"u\" extra".to_vec());
    lines.push(b"h  - - [10/Oct/2026:13:55:36 +0000] \"GET / HTTP/1.1\" 200 1".to_vec());
    lines.push(
        b"h - - [10/Oct/2026:13:55:36 +0000] \"GET / HTTP/1.1\" - - \"\\\"\" \"\\x41\\n\"".to_vec(),
    );
    let dir = common::repo().join("tests/tmp/rs-parser");
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("lines.txt");
    let mut data = Vec::new();
    for l in &lines {
        data.extend(l);
        data.push(b'\n');
    }
    std::fs::write(&input, &data).unwrap();
    let script = r#"require $argv[1] . '/lib/autoload.php'; $p = new WebPulse\Log\LogParser();
        foreach (explode("\n", rtrim(file_get_contents($argv[2]), "\n")) as $l) { $h = $p->parse($l);
        echo $h === null ? 'null' : bin2hex(json_encode([$h->ip, $h->ts, $h->offset, $h->method, $h->target, $h->protocol, $h->status, $h->bytes, $h->referrer, $h->ua, $h->vhost], JSON_INVALID_UTF8_SUBSTITUTE)), "\n"; }"#;
    let out = std::process::Command::new("php")
        .arg("-r")
        .arg(script)
        .arg(common::repo())
        .arg(&input)
        .output()
        .unwrap();
    let php: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(php.len(), lines.len());
    for (l, expected) in lines.iter().zip(php) {
        let p = parse(l);
        let got = match &p.hit {
            None => "null".to_string(),
            Some(_) => {
                let h = p.hit();
                let s = |b: &[u8]| serde_json::Value::from(String::from_utf8_lossy(b).into_owned());
                let v = serde_json::json!([
                    s(h.ip()),
                    h.ts(),
                    h.offset(),
                    s(h.method()),
                    s(h.target()),
                    s(h.protocol()),
                    h.status(),
                    h.bytes(),
                    h.referrer().map(s),
                    s(h.ua()),
                    h.vhost().map(s)
                ]);
                webpulse::util::fs::hex(serde_json::to_string(&v).unwrap().as_bytes())
            }
        };
        if expected == "null" || got == "null" {
            assert_eq!(got, expected, "{}", String::from_utf8_lossy(l));
        } else {
            let e: serde_json::Value = serde_json::from_slice(&hex_decode(&expected)).unwrap();
            let g: serde_json::Value = serde_json::from_slice(&hex_decode(&got)).unwrap();
            assert_eq!(g, e, "{}", String::from_utf8_lossy(l));
        }
    }
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
