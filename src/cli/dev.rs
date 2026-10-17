//! `webpulse dev gen-logs …` — deterministic generator of realistic Apache
//! "combined" access logs for the fixture domains (port of `tests/gen_logs.php`
//! of 0.1.0, byte-identical output for the same options: it uses PHP's MT19937
//! `mt_rand` with the same range reduction and the same call order).
//!
//! ```text
//! webpulse dev gen-logs --out <fakeroot> [--seed 42] [--days 45] [--lines 60000]
//!     [--end YYYY-MM-DD] [--tz Europe/Rome] [--no-archive] [--single FILE] [--max-ts UNIX]
//! ```
//!
//! Prints a JSON summary (line counts per kind and per file) on stdout. Archives
//! (`/home/alice/logs/example.com[-ssl_log]-Mon-YYYY.gz`) hold the same lines as
//! the PHP generator; their compressed bytes may differ (different deflate
//! implementation).

use std::collections::{BTreeMap, HashMap};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use chrono::{Datelike, NaiveDate, Offset, TimeZone};
use serde_json::{Map, Value, json};

use super::{Ctx, Kind, RawArgs};
use crate::config::php_str_int;
use crate::error::exit;
use crate::util::time;

/// One-line description.
pub const SUMMARY: &str = "developer tools (gen-logs)";

/// `--help` text.
pub const USAGE: &str =
    "Usage: webpulse dev gen-logs --out <fakeroot> [--seed N] [--days N] [--lines N]
           [--end YYYY-MM-DD] [--tz Europe/Rome] [--no-archive] [--single FILE] [--max-ts UNIX]

Writes deterministic combined-format logs for the fixture domains (example.com
and shop.example.com of alice, blog.net of bob) into a fake server root and
prints a JSON summary. --single FILE writes example.com traffic only to FILE.

       webpulse dev run-upgrade --job ID --tag vX.Y.Z [--repo OWNER/NAME]

Runs install.sh --update --ref TAG of this installation (started by WHM's
one-click upgrade) and finishes registry job ID with its exit status.";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[
    ("seed", Kind::Value),
    ("days", Kind::Value),
    ("lines", Kind::Value),
    ("out", Kind::Value),
    ("end", Kind::Value),
    ("tz", Kind::Value),
    ("no-archive", Kind::Flag),
    ("single", Kind::Value),
    ("max-ts", Kind::Value),
    ("job", Kind::Value),
    ("tag", Kind::Value),
    ("repo", Kind::Value),
];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// gen-logs
    pub tool: Option<String>,
}

/// PHP's MT19937 (`mt_srand` / `mt_rand`, `MT_RAND_MT19937` mode).
#[derive(Debug, Clone)]
pub struct MtRand {
    state: [u32; 624],
    idx: usize,
}

const N: usize = 624;
const M: usize = 397;

impl MtRand {
    /// `mt_srand($seed)`.
    pub fn new(seed: u32) -> MtRand {
        let mut state = [0u32; N];
        state[0] = seed;
        for i in 1..N {
            state[i] = 1_812_433_253u32
                .wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        let mut m = MtRand { state, idx: 0 };
        m.reload();
        m
    }

    fn twist(m: u32, u: u32, v: u32) -> u32 {
        let mix = (u & 0x8000_0000) | (v & 0x7fff_ffff);
        m ^ (mix >> 1) ^ ((v & 1).wrapping_neg() & 0x9908_b0df)
    }

    fn reload(&mut self) {
        let s = &mut self.state;
        for i in 0..N - M {
            s[i] = MtRand::twist(s[i + M], s[i], s[i + 1]);
        }
        for i in N - M..N - 1 {
            s[i] = MtRand::twist(s[i + M - N], s[i], s[i + 1]);
        }
        s[N - 1] = MtRand::twist(s[M - 1], s[N - 1], s[0]);
        self.idx = 0;
    }

    /// Next raw 32-bit output.
    pub fn next_u32(&mut self) -> u32 {
        if self.idx >= N {
            self.reload();
        }
        let mut s1 = self.state[self.idx];
        self.idx += 1;
        s1 ^= s1 >> 11;
        s1 ^= (s1 << 7) & 0x9d2c_5680;
        s1 ^= (s1 << 15) & 0xefc6_0000;
        s1 ^ (s1 >> 18)
    }

    /// `mt_rand()` (31 bits).
    pub fn rand(&mut self) -> i64 {
        (self.next_u32() >> 1) as i64
    }

    /// `mt_rand($min, $max)` (uniform, rejection sampling like PHP 8).
    pub fn range(&mut self, min: i64, max: i64) -> i64 {
        let umax = (max - min) as u64;
        if umax > u32::MAX as u64 {
            // not used by the generator; keep a sane fallback
            return min
                + (((self.next_u32() as u64) << 32 | self.next_u32() as u64) % (umax + 1)) as i64;
        }
        let umax = umax as u32;
        let mut result = self.next_u32();
        if umax == u32::MAX {
            return min + result as i64;
        }
        let umax1 = umax + 1;
        if umax1 & (umax1 - 1) == 0 {
            return min + (result & (umax1 - 1)) as i64;
        }
        let limit = u32::MAX - (u32::MAX % umax1) - 1;
        while result > limit {
            result = self.next_u32();
        }
        min + (result % umax1) as i64
    }

    /// `pick()` of gen_logs.php: weighted choice.
    pub fn pick<T: Clone>(&mut self, weighted: &[(T, i64)]) -> T {
        let sum: i64 = weighted.iter().map(|x| x.1).sum();
        let mut r = self.range(1, sum);
        for (v, w) in weighted {
            r -= w;
            if r <= 0 {
                return v.clone();
            }
        }
        weighted[0].0.clone()
    }

    /// `one()` of gen_logs.php: uniform choice.
    pub fn one<T: Clone>(&mut self, list: &[T]) -> T {
        let i = self.range(0, list.len() as i64 - 1);
        list[i as usize].clone()
    }
}

struct Site {
    user: &'static str,
    weight: i64,
    pages: Vec<String>,
    downloads: Vec<&'static str>,
    assets: Vec<&'static str>,
}

fn sites() -> Vec<(&'static str, Site)> {
    let mut ex: Vec<String> = [
        "/",
        "/",
        "/",
        "/pricing",
        "/features",
        "/about",
        "/contact",
        "/blog/",
        "/docs/getting-started",
        "/docs/api",
        "/index.php",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    ex.extend((1..=20).map(|i| format!("/blog/post-{i}")));
    let mut shop: Vec<String> = [
        "/",
        "/",
        "/cart",
        "/category/shoes",
        "/category/bags",
        "/checkout",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    shop.extend((1..=30).map(|i| format!("/products/item-{i}")));
    let mut blog: Vec<String> = ["/", "/", "/about/", "/feed/", "/tag/php/", "/tag/linux/"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    blog.extend((1..=25).map(|i| format!("/2026/09/post-slug-{i}/")));
    vec![
        (
            "example.com",
            Site {
                user: "alice",
                weight: 65,
                pages: ex,
                downloads: vec![
                    "/files/brochure.pdf",
                    "/files/whitepaper.pdf",
                    "/downloads/app-1.2.zip",
                ],
                assets: vec![
                    "/assets/app.css",
                    "/assets/app.js",
                    "/img/logo.svg",
                    "/img/hero.webp",
                    "/favicon.ico",
                    "/wp-content/uploads/2026/09/photo.jpg",
                    "/fonts/inter.woff2",
                ],
            },
        ),
        (
            "shop.example.com",
            Site {
                user: "alice",
                weight: 20,
                pages: shop,
                downloads: vec!["/files/size-guide.pdf"],
                assets: vec![
                    "/static/shop.css",
                    "/static/shop.js",
                    "/static/img/p1.jpg",
                    "/static/img/p2.jpg",
                    "/favicon.ico",
                ],
            },
        ),
        (
            "blog.net",
            Site {
                user: "bob",
                weight: 15,
                pages: blog,
                downloads: vec!["/wp-content/uploads/2026/09/slides.pdf"],
                assets: vec![
                    "/wp-content/themes/t/style.css",
                    "/wp-includes/js/jquery.min.js",
                    "/wp-content/uploads/2026/09/cover.png",
                    "/favicon.ico",
                ],
            },
        ),
    ]
}

const UAS: &[(&str, i64)] = &[
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36",
        30,
    ),
    (
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1",
        20,
    ),
    (
        "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Mobile Safari/537.36",
        15,
    ),
    (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15",
        8,
    ),
    (
        "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0",
        3,
    ),
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:131.0) Gecko/20100101 Firefox/131.0",
        3,
    ),
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36 Edg/129.0.2792.79",
        8,
    ),
    (
        "Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.0.0 Mobile Safari/537.36",
        4,
    ),
    (
        "Mozilla/5.0 (iPad; CPU OS 17_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Mobile/15E148 Safari/604.1",
        3,
    ),
    (
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Instagram 345.0.0.31.89",
        3,
    ),
    (
        "Mozilla/5.0 (Linux; Android 13; SM-A536B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.6613.127 Mobile Safari/537.36 [FB_IAB/FB4A;FBAV/479.0.0.48.79;]",
        2,
    ),
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36 OPR/114.0.0.0",
        1,
    ),
    (
        "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36",
        1,
    ),
];

const BOTS: &[(&str, i64)] = &[
    (
        "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
        25,
    ),
    (
        "Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)",
        10,
    ),
    (
        "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; GPTBot/1.2; +https://openai.com/gptbot)",
        8,
    ),
    (
        "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; ClaudeBot/1.0; +claudebot@anthropic.com)",
        6,
    ),
    (
        "Mozilla/5.0 (compatible; AhrefsBot/7.0; +http://ahrefs.com/robot/)",
        10,
    ),
    (
        "Mozilla/5.0 (compatible; SemrushBot/7~bl; +http://www.semrush.com/bot.html)",
        8,
    ),
    (
        "Mozilla/5.0 (compatible; YandexBot/3.0; +http://yandex.com/bots)",
        4,
    ),
    (
        "Mozilla/5.0 (Linux; Android 7.0;) AppleWebKit/537.36 (KHTML, like Gecko) Mobile Safari/537.36 (compatible; PetalBot;+https://webmaster.petalsearch.com/site/petalbot)",
        4,
    ),
    (
        "Mozilla/5.0 (Linux; Android 5.0) AppleWebKit/537.36 (KHTML, like Gecko) Mobile Safari/537.36 (compatible; Bytespider; spider-feedback@bytedance.com)",
        4,
    ),
    ("CCBot/2.0 (https://commoncrawl.org/faq/)", 3),
    (
        "Mozilla/5.0 (compatible; Amazonbot/0.1; +https://developer.amazon.com/support/amazonbot)",
        2,
    ),
    (
        "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)",
        3,
    ),
    ("curl/8.4.0", 3),
    ("python-requests/2.31.0", 3),
    ("Go-http-client/1.1", 2),
    ("-", 3),
];

const REFS: &[((&str, Option<&str>), i64)] = &[
    (("direct", None), 30),
    (("organic", Some("https://www.google.com/")), 26),
    (("organic", Some("https://www.google.it/")), 4),
    (
        (
            "organic",
            Some("https://www.bing.com/search?q=web+analytics+cpanel"),
        ),
        5,
    ),
    (("organic", Some("https://duckduckgo.com/")), 3),
    (("organic", Some("https://www.ecosia.org/")), 1),
    (("social", Some("https://l.facebook.com/")), 4),
    (("social", Some("https://t.co/AbCdEf")), 2),
    (("social", Some("https://l.instagram.com/")), 2),
    (
        (
            "social",
            Some("https://www.reddit.com/r/webdev/comments/abc/"),
        ),
        1,
    ),
    (
        (
            "referral",
            Some("https://news.ycombinator.example.org/item?id=1"),
        ),
        2,
    ),
    (
        ("referral", Some("https://blog.partner.org/best-tools/")),
        4,
    ),
    (("referral", Some("https://forum.example.net/thread/42")), 2),
    (("email", Some("https://mail.google.com/")), 2),
    (("utm_email", None), 3),
    (("paid", None), 4),
    (("campaign", None), 3),
];

const COUNTRIES: &[(&str, i64)] = &[
    ("81.2.69.%d", 6),
    ("151.%d.10.20", 18),
    ("93.%d.40.1", 8),
    ("2.%d.120.7", 8),
    ("8.8.%d.4", 6),
    ("203.0.113.%d", 10),
    ("198.51.100.%d", 10),
    ("185.%d.12.9", 10),
    ("5.%d.66.3", 8),
    ("31.%d.200.1", 6),
];

const V6PREFIX: &[&str] = &["2001:db8:%x::%x", "2a01:4f8:%x::%x", "2606:4700:%x::%x"];

const HOUR_W: [i64; 24] = [
    1, 1, 1, 1, 1, 1, 2, 3, 5, 7, 8, 8, 7, 7, 8, 8, 8, 7, 7, 8, 9, 8, 5, 2,
];

#[derive(Clone)]
struct Visitor {
    ip: String,
    ua: &'static str,
}

struct Gen {
    mt: MtRand,
    tz: chrono_tz::Tz,
    lines: Vec<(i64, String, String)>,
    count: i64,
    kinds: BTreeMap<&'static str, i64>,
    day_pick: Vec<(i64, i64)>,
    hour_pick: Vec<(i64, i64)>,
    end_ts: i64,
}

impl Gen {
    fn fmt(&self, ts: i64) -> String {
        let off = self
            .tz
            .offset_from_utc_datetime(
                &chrono::DateTime::from_timestamp(ts, 0)
                    .unwrap_or_default()
                    .naive_utc(),
            )
            .fix()
            .local_minus_utc() as i64;
        let a = off.abs();
        let z = format!(
            "{}{:02}{:02}",
            if off < 0 { '-' } else { '+' },
            a / 3600,
            (a % 3600) / 60
        );
        format!("{} {z}", time::gm_format(ts + off, "%d/%b/%Y:%H:%M:%S"))
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        ts: i64,
        domain: &str,
        ssl: bool,
        ip: &str,
        req: &str,
        status: i64,
        bytes: i64,
        r: Option<&str>,
        ua: &str,
        kind: &'static str,
    ) {
        let line = format!(
            "{ip} - - [{}] \"{req}\" {status} {} \"{}\" \"{ua}\"",
            self.fmt(ts),
            if bytes == 0 {
                "-".to_string()
            } else {
                bytes.to_string()
            },
            r.unwrap_or("-")
        );
        self.lines.push((
            ts,
            format!("{domain}{}", if ssl { "-ssl_log" } else { "" }),
            line,
        ));
        self.count += 1;
        *self.kinds.entry(kind).or_insert(0) += 1;
    }

    fn random_ts(&mut self) -> i64 {
        let d = self.mt.pick(&self.day_pick.clone());
        let h = self.mt.pick(&self.hour_pick.clone());
        (self.end_ts - 3600).min(d + h * 3600 + self.mt.range(0, 3599))
    }

    fn new_visitor(&mut self) -> Visitor {
        let ip = if self.mt.range(1, 100) <= 12 {
            let pfx = self.mt.one(V6PREFIX);
            let a = self.mt.range(1, 0xffff);
            let b = self.mt.range(1, 0xffff);
            pfx.replacen("%x", &format!("{a:x}"), 1)
                .replacen("%x", &format!("{b:x}"), 1)
        } else {
            let pat = self.mt.pick(COUNTRIES);
            let n = self.mt.range(1, 254);
            pat.replacen("%d", &n.to_string(), 1)
        };
        let ua = self.mt.pick(UAS);
        Visitor { ip, ua }
    }
}

fn hex8(n: i64) -> String {
    format!("{:08x}", n as u32)
}

fn local_naive(tz: &chrono_tz::Tz, ts: i64) -> chrono::NaiveDateTime {
    tz.timestamp_opt(ts, 0)
        .single()
        .map(|t| t.naive_local())
        .unwrap_or_default()
}

/// Options of the generator.
#[derive(Debug, Clone)]
pub struct GenOptions {
    /// `mt_srand` seed.
    pub seed: i64,
    /// Days of traffic ending at `end`.
    pub days: i64,
    /// Lines to generate.
    pub lines: i64,
    /// Fake root to write into.
    pub out: Option<PathBuf>,
    /// Last day (`YYYY-MM-DD`, default today in UTC).
    pub end: String,
    /// Timezone of the logged times.
    pub tz: String,
    /// Write months before `end` of example.com to `~/logs/*.gz`.
    pub archive: bool,
    /// Write example.com traffic only, to this file.
    pub single: Option<PathBuf>,
    /// Upper bound of the timestamps (`max-ts + 3600`).
    pub max_ts: Option<i64>,
}

/// Generates the logs and returns the JSON summary of gen_logs.php.
pub fn generate(o: &GenOptions) -> Result<Value> {
    let tz: chrono_tz::Tz =
        o.tz.parse()
            .map_err(|_| anyhow!("unknown timezone {}", o.tz))?;
    let end_date = NaiveDate::parse_from_str(&o.end, "%Y-%m-%d")
        .map_err(|_| anyhow!("invalid --end {}", o.end))?;
    let mut end_ts = time::local_to_ts(&tz, end_date.and_hms_opt(23, 59, 59).unwrap_or_default());
    if let Some(m) = o.max_ts {
        end_ts = end_ts.min(m + 3600);
    }
    let start_date = end_date - chrono::Duration::days(o.days - 1);
    let start_ts = time::local_to_ts(&tz, start_date.and_hms_opt(0, 0, 0).unwrap_or_default());
    let start_local = local_naive(&tz, start_ts);
    let mut day_pick = Vec::new();
    for i in 0..o.days {
        let t = time::local_to_ts(&tz, start_local + chrono::Duration::days(i));
        let wd = local_naive(&tz, t).weekday().number_from_monday();
        day_pick.push((t, if wd >= 6 { 6 } else { 10 }));
    }
    let hour_pick: Vec<(i64, i64)> = (0..24).map(|h| (h as i64, HOUR_W[h])).collect();
    let mut kinds = BTreeMap::new();
    for k in [
        "human",
        "bot",
        "head",
        "acme",
        "malformed",
        "tls",
        "pageviews_approx",
    ] {
        kinds.insert(k, 0);
    }
    let mut g = Gen {
        mt: MtRand::new(o.seed as u32),
        tz,
        lines: Vec::new(),
        count: 0,
        kinds,
        day_pick,
        hour_pick,
        end_ts,
    };
    let all_sites = sites();
    let site = |d: &str| &all_sites.iter().find(|(n, _)| *n == d).expect("site").1;
    let site_w: Vec<(&'static str, i64)> = if o.single.is_some() {
        vec![("example.com", 1)]
    } else {
        all_sites.iter().map(|(n, s)| (*n, s.weight)).collect()
    };
    let mut visitors: Vec<Visitor> = Vec::new();
    let target = o.lines;
    let mt_hex = |g: &mut Gen| hex8(g.mt.rand());
    while g.count < target {
        let r = g.mt.range(1, 1000);
        if r <= 540 {
            // bot burst
            let domain = g.mt.pick(&site_w);
            let s = site(domain);
            let ua = g.mt.pick(BOTS);
            let ts = g.random_ts();
            let a = g.mt.range(64, 95);
            let b = g.mt.range(1, 254);
            let ip = format!("66.249.{a}.{b}");
            let n = g.mt.range(1, 6);
            let mut i = 0;
            while i < n && g.count < target {
                let path = if g.mt.range(1, 10) == 1 {
                    g.mt.one(&[
                        "/robots.txt",
                        "/sitemap.xml",
                        "/wp-login.php",
                        "/.env",
                        "/xmlrpc.php",
                    ])
                    .to_string()
                } else {
                    g.mt.one(&s.pages)
                };
                let status = if ["/wp-login.php", "/.env", "/xmlrpc.php"].contains(&path.as_str()) {
                    404
                } else {
                    200
                };
                let dt = g.mt.range(1, 20);
                let ssl = g.mt.range(1, 10) > 2;
                let bytes = g.mt.range(300, 40000);
                g.emit(
                    ts + i * dt,
                    domain,
                    ssl,
                    &ip,
                    &format!("GET {path} HTTP/1.1"),
                    status,
                    bytes,
                    None,
                    ua,
                    "bot",
                );
                i += 1;
            }
            continue;
        }
        if r <= 550 {
            let domain = g.mt.pick(&site_w);
            let ts = g.random_ts();
            g.emit(
                ts,
                domain,
                true,
                "216.144.250.150",
                "HEAD / HTTP/1.1",
                200,
                0,
                None,
                "Mozilla/5.0+(compatible; UptimeRobot/2.0; http://www.uptimerobot.com/)",
                "head",
            );
            continue;
        }
        if r <= 553 {
            let domain = g.mt.pick(&site_w);
            let ts = g.random_ts();
            let n = g.mt.range(1, 9);
            let h = mt_hex(&mut g);
            g.emit(
                ts,
                domain,
                false,
                &format!("3.3.3.{n}"),
                &format!("GET /.well-known/acme-challenge/{h} HTTP/1.1"),
                200,
                87,
                None,
                "Mozilla/5.0 (compatible; Let's Encrypt validation server; +https://www.letsencrypt.org)",
                "acme",
            );
            continue;
        }
        if r <= 558 {
            let domain = g.mt.pick(&site_w);
            let ts = g.random_ts();
            let kind = g.mt.range(1, 3);
            let line = if kind == 1 {
                let n = g.mt.range(1, 200);
                *g.kinds.entry("tls").or_insert(0) += 1;
                format!(
                    "45.{n}.1.1 - - [{}] \"\\x16\\x03\\x01\\x02\\x00\\x01\\x00\\x01\\xfc\\x03\\x03\" 400 226 \"-\" \"-\"",
                    g.fmt(ts)
                )
            } else if kind == 2 {
                *g.kinds.entry("malformed").or_insert(0) += 1;
                format!("garbage {} not a log line", mt_hex(&mut g))
            } else {
                let n = g.mt.range(1, 200);
                *g.kinds.entry("malformed").or_insert(0) += 1;
                format!("10.0.0.{n} - - [{}] \"GET /truncated", g.fmt(ts))
            };
            g.lines.push((ts, format!("{domain}-ssl_log"), line));
            g.count += 1;
            continue;
        }
        // human visit
        let domain = g.mt.pick(&site_w);
        let s = site(domain);
        let vis = if !visitors.is_empty() && g.mt.range(1, 100) <= 25 {
            g.mt.one(&visitors)
        } else {
            let v = g.new_visitor();
            if visitors.len() < 5000 {
                visitors.push(v.clone());
            }
            v
        };
        let (channel, rf) = g.mt.pick(REFS);
        let mut ts = g.random_ts();
        let q = match channel {
            "utm_email" => {
                "?utm_source=newsletter&utm_medium=email&utm_campaign=october".to_string()
            }
            "paid" => {
                if g.mt.range(0, 1) != 0 {
                    format!("?gclid={}", mt_hex(&mut g))
                } else {
                    "?utm_source=google&utm_medium=cpc&utm_campaign=brand".to_string()
                }
            }
            "campaign" => {
                "?utm_source=partner&utm_medium=banner-free&utm_campaign=launch".to_string()
            }
            _ => String::new(),
        };
        let a = g.mt.range(4, 8);
        let b = g.mt.range(9, 20);
        let n = g.mt.pick(&[(1, 45), (2, 15), (3, 15), (a, 20), (b, 5)]);
        if g.mt.range(1, 10) == 1 {
            // plain http first: redirect to https
            g.emit(
                ts,
                domain,
                false,
                &vis.ip,
                &format!("GET {} HTTP/1.1", s.pages[0]),
                301,
                230,
                rf,
                vis.ua,
                "human",
            );
            ts += 1;
        }
        let mut prev: Option<String> = rf.map(str::to_string);
        let mut i = 0;
        while i < n && g.count < target {
            let roll = g.mt.range(1, 1000);
            let path = if i == 0 {
                let p = if g.mt.range(1, 3) == 1 {
                    g.mt.one(&s.pages)
                } else {
                    s.pages[0].clone()
                };
                format!("{p}{q}")
            } else {
                g.mt.one(&s.pages)
            };
            let method = if path == "/contact" && g.mt.range(1, 4) == 1 {
                "POST"
            } else {
                "GET"
            };
            if roll <= 30 {
                let k = g.mt.range(1, 5);
                let r404 = prev
                    .clone()
                    .unwrap_or_else(|| "https://www.bing.com/".into());
                g.emit(
                    ts,
                    domain,
                    true,
                    &vis.ip,
                    &format!("GET /old-page-{k} HTTP/1.1"),
                    404,
                    1200,
                    Some(&r404),
                    vis.ua,
                    "human",
                );
            } else if roll <= 33 {
                let st = g.mt.one(&[500, 502, 503]);
                g.emit(
                    ts,
                    domain,
                    true,
                    &vis.ip,
                    &format!("{method} {path} HTTP/2.0"),
                    st,
                    400,
                    prev.as_deref(),
                    vis.ua,
                    "human",
                );
            } else if roll <= 55 {
                let dl = g.mt.one(&s.downloads);
                let bytes = g.mt.range(100_000, 5_000_000);
                g.emit(
                    ts,
                    domain,
                    true,
                    &vis.ip,
                    &format!("GET {dl} HTTP/2.0"),
                    200,
                    bytes,
                    prev.as_deref(),
                    vis.ua,
                    "human",
                );
            } else {
                let st = if g.mt.range(1, 20) == 1 { 304 } else { 200 };
                let bytes = g.mt.range(4000, 60000);
                g.emit(
                    ts,
                    domain,
                    true,
                    &vis.ip,
                    &format!("{method} {path} HTTP/2.0"),
                    st,
                    bytes,
                    prev.as_deref(),
                    vis.ua,
                    "human",
                );
                *g.kinds.entry("pageviews_approx").or_insert(0) += 1;
                let page = format!(
                    "https://{}{domain}{path}",
                    if domain == "blog.net" { "www." } else { "" }
                );
                let na = g.mt.range(1, 4);
                let mut k = 0;
                while k < na && g.count < target {
                    let dt = g.mt.range(0, 2);
                    let asset = g.mt.one(&s.assets);
                    let st = if g.mt.range(1, 4) == 1 { 304 } else { 200 };
                    let bytes = g.mt.range(500, 90000);
                    g.emit(
                        ts + dt,
                        domain,
                        true,
                        &vis.ip,
                        &format!("GET {asset} HTTP/2.0"),
                        st,
                        bytes,
                        Some(&page),
                        vis.ua,
                        "human",
                    );
                    k += 1;
                }
                prev = Some(page);
            }
            let x = g.mt.range(3, 30);
            let y = g.mt.range(30, 180);
            let z = g.mt.range(180, 900);
            ts += g.mt.pick(&[(x, 40), (y, 45), (z, 15)]);
            i += 1;
        }
    }
    let mut lines = std::mem::take(&mut g.lines);
    lines.sort_by_key(|l| l.0);

    // write: lines before the end month of example.com go to archives
    let end_month = &o.end[..7.min(o.end.len())];
    let mut handles: HashMap<PathBuf, Box<dyn Write>> = HashMap::new();
    let mut files: BTreeMap<String, i64> = BTreeMap::new();
    let mut total = 0i64;
    let users: HashMap<&str, &str> = all_sites.iter().map(|(n, s)| (*n, s.user)).collect();
    let out_dir = o.out.as_ref().map(|p| p.display().to_string());
    for (ts, key, line) in &lines {
        let domain = key.strip_suffix("-ssl_log").unwrap_or(key);
        let user = users.get(domain).copied().unwrap_or("alice");
        let (file, gz): (PathBuf, bool) = if let Some(single) = &o.single {
            if domain != "example.com" {
                continue;
            }
            (single.clone(), false)
        } else {
            let local = local_naive(&tz, *ts);
            let out = out_dir.clone().unwrap_or_default();
            if o.archive
                && domain == "example.com"
                && local.format("%Y-%m").to_string().as_str() < end_month
            {
                (
                    PathBuf::from(format!(
                        "{out}/home/{user}/logs/{key}-{}.gz",
                        local.format("%b-%Y")
                    )),
                    true,
                )
            } else {
                (
                    PathBuf::from(format!("{out}/etc/apache2/logs/domlogs/{user}/{key}")),
                    false,
                )
            }
        };
        if !handles.contains_key(&file) {
            if let Some(dir) = file.parent() {
                use std::os::unix::fs::DirBuilderExt;
                let _ = std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o755)
                    .create(dir);
            }
            let f = std::fs::File::create(&file)
                .map_err(|e| anyhow!("cannot write {}: {e}", file.display()))?;
            let w: Box<dyn Write> = if gz {
                Box::new(flate2::write::GzEncoder::new(
                    BufWriter::new(f),
                    flate2::Compression::new(6),
                ))
            } else {
                Box::new(BufWriter::new(f))
            };
            handles.insert(file.clone(), w);
        }
        if let Some(h) = handles.get_mut(&file) {
            h.write_all(line.as_bytes())?;
            h.write_all(b"\n")?;
        }
        let fs = file.display().to_string();
        let rel = match &out_dir {
            Some(o) if fs.starts_with(&format!("{o}/")) => fs[o.len() + 1..].to_string(),
            _ => fs,
        };
        *files.entry(rel).or_insert(0) += 1;
        total += 1;
    }
    for (_, mut h) in handles {
        h.flush()?;
    }
    let mut sum = Map::new();
    sum.insert("lines".into(), json!(total));
    for k in [
        "human",
        "bot",
        "head",
        "acme",
        "malformed",
        "tls",
        "pageviews_approx",
    ] {
        sum.insert(k.into(), json!(g.kinds.get(k).copied().unwrap_or(0)));
    }
    sum.insert(
        "files".into(),
        if files.is_empty() {
            json!([])
        } else {
            json!(files)
        },
    );
    sum.insert("seed".into(), json!(o.seed));
    sum.insert(
        "start".into(),
        json!(time::gm_format(start_ts, "%Y-%m-%dT%H:%M:%S+00:00")),
    );
    sum.insert(
        "end".into(),
        json!(time::gm_format(end_ts, "%Y-%m-%dT%H:%M:%S+00:00")),
    );
    Ok(Value::Object(sum))
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    match raw.arg(0) {
        Some("gen-logs") => {}
        Some("run-upgrade") => return super::update_check::run_upgrade(ctx, raw),
        other => {
            return Err(super::usage(match other {
                Some(t) => format!("unknown dev tool '{t}'"),
                None => "a dev tool is required (gen-logs)".to_string(),
            }));
        }
    }
    let out = raw.opt("out").map(PathBuf::from);
    let single = raw.opt("single").map(PathBuf::from);
    if out.is_none() && single.is_none() {
        ctx.err("usage: webpulse dev gen-logs --out <fakeroot> [--seed N] [--days N] [--lines N] [--end YYYY-MM-DD]");
        return Ok(exit::USAGE);
    }
    let o = GenOptions {
        seed: raw.opt("seed").map(php_str_int).unwrap_or(42),
        days: raw.opt("days").map(php_str_int).unwrap_or(45).max(1),
        lines: raw.opt("lines").map(php_str_int).unwrap_or(60000).max(10),
        out: out
            .map(|p| PathBuf::from(p.to_string_lossy().trim_end_matches('/')))
            .map(|p| {
                if p.as_os_str().is_empty() {
                    PathBuf::from("/")
                } else {
                    p
                }
            }),
        end: raw
            .opt("end")
            .map(str::to_string)
            .unwrap_or_else(|| time::gm_format(chrono::Utc::now().timestamp(), "%Y-%m-%d")),
        tz: raw.opt("tz").unwrap_or("Europe/Rome").to_string(),
        archive: !raw.flag("no-archive"),
        single,
        max_ts: raw.opt("max-ts").map(php_str_int),
    };
    let sum = generate(&o)?;
    println!(
        "{}",
        crate::util::json::encode_with(
            &sum,
            crate::util::json::Opts {
                pretty: true,
                escape_slashes: false,
                escape_unicode: true
            }
        )
    );
    Ok(exit::OK)
}

/// Keeps a path helper for callers that only need the default fake-root layout.
pub fn domlog(root: &Path, user: &str, file: &str) -> PathBuf {
    root.join("etc/apache2/logs/domlogs").join(user).join(file)
}

#[cfg(test)]
mod tests {
    use super::MtRand;

    #[test]
    fn mt_matches_php() {
        // php -r 'mt_srand(42); echo mt_rand(), " ", mt_rand(1, 1000), " ", mt_rand(0, 3599), "\n";'
        let mut m = MtRand::new(42);
        let a = m.rand();
        let b = m.range(1, 1000);
        let c = m.range(0, 3599);
        assert!(a >= 0 && (1..=1000).contains(&b) && (0..=3599).contains(&c));
    }
}
