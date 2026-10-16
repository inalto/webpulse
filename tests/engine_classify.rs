//! Port of tests/cases/engine_classify.php.
#![allow(clippy::type_complexity)]

use webpulse::engine::classify::bots;
use webpulse::engine::classify::path::{INVALID, PathAnalyzer};
use webpulse::engine::classify::referrer::{Referrer, Utm};
use webpulse::engine::classify::ua::UserAgent;
use webpulse::engine::store::dims;
use webpulse::util::{ip, strs};

#[test]
fn user_agents_browsers_versions_os_devices() {
    let mut ua = UserAgent::new();
    let cases = [
        (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36",
            "Chrome",
            "Chrome 129",
            "Windows",
            "Windows 10/11",
            "desktop",
        ),
        (
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1",
            "Safari",
            "Safari 17",
            "iOS",
            "iOS 17",
            "mobile",
        ),
        (
            "Mozilla/5.0 (iPad; CPU OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1",
            "Safari",
            "Safari 16",
            "iOS",
            "iOS 16",
            "tablet",
        ),
        (
            "Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.0.0 Mobile Safari/537.36",
            "Samsung Internet",
            "Samsung Internet 25",
            "Android",
            "Android 14",
            "mobile",
        ),
        (
            "Mozilla/5.0 (Linux; Android 13; SM-X700) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36",
            "Chrome",
            "Chrome 128",
            "Android",
            "Android 13",
            "tablet",
        ),
        (
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Safari/605.1.15",
            "Safari",
            "Safari 17",
            "macOS",
            "macOS 10.15+",
            "desktop",
        ),
        (
            "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0",
            "Firefox",
            "Firefox 131",
            "Linux",
            "Linux",
            "desktop",
        ),
        (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36 Edg/129.0.2792.79",
            "Edge",
            "Edge 129",
            "Windows",
            "Windows 10/11",
            "desktop",
        ),
        (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36 OPR/114.0.0.0",
            "Opera",
            "Opera 114",
            "Windows",
            "Windows 10/11",
            "desktop",
        ),
        (
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Instagram 345.0.0.31.89",
            "In-app",
            "In-app Instagram",
            "iOS",
            "iOS 17",
            "mobile",
        ),
        (
            "Mozilla/5.0 (Linux; Android 13; SM-A536B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Mobile Safari/537.36 [FB_IAB/FB4A;FBAV/479.0.0.48.79;]",
            "In-app",
            "In-app Facebook",
            "Android",
            "Android 13",
            "mobile",
        ),
        (
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/129.0.6668.69 Mobile/15E148 Safari/604.1",
            "Chrome",
            "Chrome 129",
            "iOS",
            "iOS 17",
            "mobile",
        ),
        (
            "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36",
            "Chrome",
            "Chrome 129",
            "ChromeOS",
            "ChromeOS",
            "desktop",
        ),
        (
            "Mozilla/5.0 (compatible; MSIE 9.0; Windows NT 6.1; Trident/5.0)",
            "IE",
            "IE 9",
            "Windows",
            "Windows 7",
            "desktop",
        ),
        (
            "Mozilla/5.0 (Windows NT 6.1; Trident/7.0; rv:11.0) like Gecko",
            "IE",
            "IE 11",
            "Windows",
            "Windows 7",
            "desktop",
        ),
        (
            "Mozilla/5.0 (Linux; Android 10; CUBOT X30) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Mobile Safari/537.36",
            "Chrome",
            "Chrome 120",
            "Android",
            "Android 10",
            "mobile",
        ),
        (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 YaBrowser/24.7.0.0 Safari/537.36",
            "Yandex Browser",
            "Yandex Browser 24",
            "Windows",
            "Windows 10/11",
            "desktop",
        ),
    ];
    for (s, b, bv, os, osv, dev) in cases {
        let r = ua.parse(s.as_bytes());
        assert_eq!(
            (
                r.browser.as_str(),
                r.browser_version.as_str(),
                r.os.as_str(),
                r.os_version.as_str(),
                r.device.as_str(),
                r.bot
            ),
            (b, bv, os, osv, dev, None),
            "{s}"
        );
    }
}

#[test]
fn bots_named_families_ai_crawlers_scripts_empty_ua() {
    let cases = [
        (
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
            "Googlebot",
        ),
        (
            "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; GPTBot/1.2; +https://openai.com/gptbot)",
            "GPTBot",
        ),
        (
            "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; ClaudeBot/1.0; +claudebot@anthropic.com)",
            "ClaudeBot",
        ),
        (
            "Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)",
            "bingbot",
        ),
        ("CCBot/2.0 (https://commoncrawl.org/faq/)", "CCBot"),
        (
            "Mozilla/5.0 (compatible; AhrefsBot/7.0; +http://ahrefs.com/robot/)",
            "AhrefsBot",
        ),
        (
            "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)",
            "facebookexternalhit",
        ),
        (
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/120.0.0.0 Safari/537.36",
            "HeadlessChrome",
        ),
        (
            "Mozilla/5.0 (Linux; Android 11; moto g power (2022)) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/109.0.0.0 Mobile Safari/537.36 Chrome-Lighthouse",
            "Lighthouse",
        ),
        ("curl/8.4.0", "curl"),
        ("python-requests/2.31.0", "python-requests"),
        ("Go-http-client/1.1", "Go-http-client"),
        ("Wget/1.21.4", "Wget"),
        (
            "Mozilla/5.0+(compatible; UptimeRobot/2.0; http://www.uptimerobot.com/)",
            "UptimeRobot",
        ),
        ("Mozilla/5.0 (compatible; MyCustomCrawler/1.0)", "Other bot"),
        ("SomeTool/1.0", "Other bot"),
        ("", "Empty user agent"),
        ("-", "Empty user agent"),
    ];
    for (ua, name) in cases {
        assert_eq!(bots::detect(ua.as_bytes()), Some(name), "{ua}");
    }
    assert_eq!(bots::detect(b"Mozilla/5.0 (Linux; Android 10; CUBOT X30) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Mobile Safari/537.36"), None);
    assert_eq!(UserAgent::new().parse(b"curl/8.4.0").device, "bot");
}

#[test]
fn referrer_channels_and_search_engines() {
    let mut r = Referrer::new();
    let none = Utm::default();
    let cases: &[(Option<&str>, &str, Option<&str>, Option<&str>, Option<&str>)] = &[
        (None, "direct", None, None, None),
        (Some("-"), "direct", None, None, None),
        (
            Some("https://www.example.com/x"),
            "direct",
            None,
            None,
            None,
        ),
        (Some("https://example.com/"), "direct", None, None, None),
        (
            Some("https://www.google.com/"),
            "organic",
            Some("google.com"),
            Some("google"),
            None,
        ),
        (
            Some("https://www.google.co.uk/search?q=Foo+Bar"),
            "organic",
            Some("google.co.uk"),
            Some("google"),
            Some("foo bar"),
        ),
        (
            Some("https://www.bing.com/search?q=web+stats"),
            "organic",
            Some("bing.com"),
            Some("bing"),
            Some("web stats"),
        ),
        (
            Some("https://duckduckgo.com/"),
            "organic",
            Some("duckduckgo.com"),
            Some("duckduckgo"),
            None,
        ),
        (
            Some("https://yandex.ru/search/?text=%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82"),
            "organic",
            Some("yandex.ru"),
            Some("yandex"),
            Some("привет"),
        ),
        (
            Some("https://www.baidu.com/s?wd=test"),
            "organic",
            Some("baidu.com"),
            Some("baidu"),
            Some("test"),
        ),
        (
            Some("https://search.brave.com/search?q=x"),
            "organic",
            Some("search.brave.com"),
            Some("brave"),
            Some("x"),
        ),
        (
            Some("android-app://com.google.android.googlequicksearchbox/"),
            "organic",
            Some("com.google.android.googlequicksearchbox"),
            Some("google"),
            None,
        ),
        (
            Some("https://l.facebook.com/l.php?u=x"),
            "social",
            Some("l.facebook.com"),
            None,
            None,
        ),
        (Some("https://t.co/abc"), "social", Some("t.co"), None, None),
        (
            Some("https://www.linkedin.com/feed/"),
            "social",
            Some("linkedin.com"),
            None,
            None,
        ),
        (
            Some("https://mail.google.com/mail/u/0/"),
            "email",
            Some("mail.google.com"),
            None,
            None,
        ),
        (
            Some("https://outlook.live.com/"),
            "email",
            Some("outlook.live.com"),
            None,
            None,
        ),
        (
            Some("https://blog.other.org/post?x=1"),
            "referral",
            Some("blog.other.org"),
            None,
            None,
        ),
        (
            Some("https://shop.example.com/"),
            "referral",
            Some("shop.example.com"),
            None,
            None,
        ),
    ];
    for (rf, ch, host, se, kw) in cases {
        let c = r.classify(rf.map(str::as_bytes), "example.com", &none);
        assert_eq!(
            (
                c.channel,
                c.referrer_host.as_deref(),
                c.search_engine,
                c.keyword.as_deref()
            ),
            (*ch, *host, *se, *kw),
            "{rf:?}"
        );
    }
    assert_eq!(
        r.classify(
            Some(b"https://blog.other.org/post?x=1"),
            "example.com",
            &none
        )
        .referrer_url
        .as_deref(),
        Some("blog.other.org/post")
    );
    assert!(
        r.classify(Some(b"https://www.google.com/"), "example.com", &none)
            .referrer_url
            .is_none()
    );
    assert!(
        r.external_host(Some(b"https://www.example.com/a"), "example.com")
            .is_none()
    );
    assert_eq!(
        r.any_host(Some(b"https://www.example.com/a")).as_deref(),
        Some("example.com")
    );
}

#[test]
fn referrer_utm_and_click_ids_take_precedence() {
    let mut r = Referrer::new();
    let u = |s: Option<&'static str>,
             m: Option<&'static str>,
             c: Option<&'static str>,
             click: bool| Utm {
        source: s,
        medium: m,
        campaign: c,
        click_id: click,
    };
    assert_eq!(
        r.classify(
            Some(b"https://www.google.com/"),
            "example.com",
            &u(Some("google"), Some("cpc"), None, false)
        )
        .channel,
        "paid"
    );
    assert_eq!(
        r.classify(
            Some(b"https://www.google.com/"),
            "example.com",
            &u(None, None, None, true)
        )
        .channel,
        "paid"
    );
    assert_eq!(
        r.classify(
            None,
            "example.com",
            &u(Some("news"), Some("email"), Some("oct"), false)
        )
        .channel,
        "email"
    );
    assert_eq!(
        r.classify(
            Some(b"https://www.google.com/"),
            "example.com",
            &u(Some("partner"), Some("banner-free"), Some("launch"), false)
        )
        .channel,
        "campaign"
    );
    assert_eq!(
        r.classify(
            None,
            "example.com",
            &u(Some("fb"), Some("social"), None, false)
        )
        .channel,
        "social"
    );
    assert_eq!(
        r.classify(
            Some(b"https://www.google.com/"),
            "example.com",
            &u(Some("google"), Some("organic"), None, false)
        )
        .channel,
        "organic"
    );
}

#[test]
fn path_normalisation() {
    let p = PathAnalyzer::new(vec![
        "/wp-content/uploads/".into(),
        "/wp-includes/".into(),
        "/wp-json/".into(),
    ]);
    let a = p.compute(b"/a//b/index.php?utm_source=fb&utm_medium=cpc&utm_campaign=x&x=1");
    assert_eq!(
        (
            a.path.as_str(),
            a.utm_source.as_deref(),
            a.utm_medium.as_deref(),
            a.utm_campaign.as_deref()
        ),
        ("/a/b/", Some("fb"), Some("cpc"), Some("x"))
    );
    assert_eq!(p.compute(b"/caf%C3%A9/").path, "/caf\u{e9}/");
    assert_eq!(p.compute(b"/%ff%fe.pdf").path, "/??.pdf");
    assert_eq!(p.compute(b"/index.html").path, "/");
    assert_eq!(p.compute(b"/blog/INDEX.HTM").path, "/blog/");
    assert_eq!(p.compute(b"/x#frag").path, "/x");
    let abs = p.compute(b"http://example.com/foo?gclid=1");
    assert_eq!(
        (abs.path.as_str(), abs.scheme.as_deref(), abs.click_id),
        ("/foo", Some("http"), true)
    );
    assert_eq!(p.compute(b"").path, INVALID);
    assert_eq!(
        p.compute(b"/a%3Fb").path,
        "/a?b",
        "encoded ? is part of the path"
    );
    let long = p.compute(format!("/{}", "\u{e9}".repeat(400)).as_bytes());
    assert!(long.path.len() <= 512);
    assert!(long.key.len() <= 255 && long.key.ends_with('\u{2026}'));
    assert_eq!(p.compute(b"/a%00").path, "/a?", "NUL replaced");
}

#[test]
fn path_file_types_static_and_downloads() {
    let p = PathAnalyzer::new(vec![
        "/wp-content/uploads/".into(),
        "/wp-includes/".into(),
        "/wp-json/".into(),
    ]);
    let t = |x: &str| {
        let i = p.compute(x.as_bytes());
        (i.file_type, i.is_static, i.is_download)
    };
    assert_eq!(t("/"), ("html".into(), false, false));
    assert_eq!(t("/page.php"), ("html".into(), false, false));
    assert_eq!(t("/about.htm"), ("html".into(), false, false));
    assert_eq!(t("/x.JPG"), ("jpg".into(), true, false));
    assert_eq!(t("/a.css?v=3"), ("css".into(), true, false));
    assert_eq!(t("/wp-content/uploads/a.pdf"), ("pdf".into(), true, true));
    assert_eq!(t("/doc.tar.gz"), ("gz".into(), false, true));
    assert_eq!(t("/wp-json/wp/v2/posts"), ("html".into(), true, false));
    assert_eq!(t("/a.verylongextension"), ("html".into(), false, false));
    assert_eq!(t("/f.woff2"), ("woff2".into(), true, false));
    assert_eq!(t("/app-1.2.zip"), ("zip".into(), false, true));
}

#[test]
fn dims_list_and_buckets() {
    assert_eq!(dims::ALL.len(), 32);
    for d in dims::VISIT_LEVEL {
        assert!(dims::ALL.contains(d));
    }
    let d: Vec<&str> = [0, 10, 11, 60, 61, 300, 600, 1800, 1801]
        .iter()
        .map(|&s| dims::duration_bucket(s))
        .collect();
    assert_eq!(
        d,
        [
            "0-10s", "0-10s", "11-30s", "31-60s", "1-2m", "2-5m", "5-10m", "10-30m", "30m+"
        ]
    );
    let p: Vec<&str> = [1, 2, 5, 6, 20, 21]
        .iter()
        .map(|&n| dims::pages_bucket(n))
        .collect();
    assert_eq!(p, ["1", "2", "5", "6-10", "11-20", "21+"]);
}

#[test]
fn utf8_cleaning_and_truncation() {
    assert_eq!(strs::clean(b"a\xffb"), "a?b");
    assert_eq!(strs::scrub(b"a\xffb"), "a?b");
    assert_eq!(strs::scrub(b"\xc3\xa9\xc3"), "\u{e9}?");
    assert_eq!(strs::clean(b"a\x01b"), "a?b");
    assert_eq!(strs::truncate("ab\u{e9}", 3, ""), "ab");
    assert_eq!(strs::truncate("abcdef", 4, "\u{2026}"), "a\u{2026}");
    assert!(strs::key("\u{20ac}".repeat(200).as_bytes(), 255).len() <= 255);
    assert_eq!(strs::studly("geoip-update"), "GeoipUpdate");
}

#[test]
fn ip_cidr_matching_v4_v6_mapped() {
    let c = ip::compile(&[
        "203.0.113.0/24",
        "2001:db8::/32",
        "198.51.100.7",
        "bogus",
        "10.0.0.0/33",
    ]);
    assert_eq!(c.len(), 3);
    assert!(ip::matches("203.0.113.99", &c));
    assert!(!ip::matches("203.0.114.1", &c));
    assert!(ip::matches("2001:db8:1::5", &c));
    assert!(!ip::matches("2001:db9::1", &c));
    assert!(ip::matches("::ffff:198.51.100.7", &c));
    assert!(!ip::matches("not-an-ip", &c));
    assert!(ip::is_valid_cidr("::/0"));
    assert!(!ip::is_valid_cidr("1.2.3.4/"));
}
