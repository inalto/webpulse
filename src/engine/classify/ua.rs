//! User-Agent classifier (`WebPulse\Classify\UserAgent`): browser, browser_version
//! (`Chrome 129`), os, os_version (`Android 14`), device (desktop | mobile | tablet |
//! bot | other) and the bot family (`None` for humans).

use foldhash::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use regex::bytes::Regex;

use super::bots::Bots;

/// Classification of one User-Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UaInfo {
    /// Browser family.
    pub browser: String,
    /// `Browser Major`.
    pub browser_version: String,
    /// OS family.
    pub os: String,
    /// OS version label.
    pub os_version: String,
    /// desktop | mobile | tablet | bot | other
    pub device: String,
    /// Bot family, `None` for humans.
    pub bot: Option<&'static str>,
}

/// In-app browsers: `[regex, app name]` → browser `In-app`, version `In-app <app>`.
const IN_APP: &[(&str, &str)] = &[
    (r"FBAN|FBAV|FB_IAB|FBIOS", "Facebook"),
    (r"Instagram", "Instagram"),
    (r"WhatsApp", "WhatsApp"),
    (r"\bLine/", "LINE"),
    (r"Snapchat", "Snapchat"),
    (r"(?i)musical_ly|BytedanceWebview|TikTok", "TikTok"),
    (r"LinkedInApp", "LinkedIn"),
    (r"Twitter(?:Android)?", "Twitter"),
    (r"Pinterest", "Pinterest"),
    (r"MicroMessenger", "WeChat"),
    (r"GSA/", "Google App"),
    (r"Telegram", "Telegram"),
];

/// Browsers: `[regex with a version capture, name]`, first match wins.
const BROWSERS: &[(&str, &str)] = &[
    (r"(?:Edg|EdgA|EdgiOS|Edge)/(\d+)", "Edge"),
    (r"(?:OPR|OPiOS|OPT)/(\d+)", "Opera"),
    (r"Opera Mini/(\d+)", "Opera"),
    (r"Opera[/ ](\d+)", "Opera"),
    (r"SamsungBrowser/(\d+)", "Samsung Internet"),
    (r"YaBrowser/(\d+)", "Yandex Browser"),
    (r"UCBrowser/(\d+)", "UC Browser"),
    (r"Vivaldi/(\d+)", "Vivaldi"),
    (r"(?:Firefox|FxiOS)/(\d+)", "Firefox"),
    (r"CriOS/(\d+)", "Chrome"),
    (r"(?:Chrome|Chromium)/(\d+)", "Chrome"),
    (r"Version/(\d+)[\d.]* (?:Mobile/\S+ )?Safari/", "Safari"),
    (r"MSIE (\d+)", "IE"),
    (r"Trident/.*rv:(\d+)", "IE"),
    (r"AppleWebKit/.*(?:iPhone|iPad|iPod|Macintosh)()", "Safari"),
];

/// Windows NT version → label.
const WINDOWS: &[(&str, &str)] = &[
    ("10.0", "Windows 10/11"),
    ("6.3", "Windows 8.1"),
    ("6.2", "Windows 8"),
    ("6.1", "Windows 7"),
    ("6.0", "Windows Vista"),
    ("5.1", "Windows XP"),
    ("5.2", "Windows XP"),
];

struct Res {
    in_app: Vec<(Regex, &'static str)>,
    browsers: Vec<(Regex, &'static str)>,
    win_phone: Regex,
    win_nt: Regex,
    ios_ver: Regex,
    ios: Regex,
    android: Regex,
    mac: Regex,
    linux: Regex,
    tablet: Regex,
    mobile: Regex,
    tv: Regex,
}

fn re(s: &str) -> Regex {
    Regex::new(&format!("(?-u){s}")).expect("valid regex")
}

fn res() -> &'static Res {
    static R: OnceLock<Res> = OnceLock::new();
    R.get_or_init(|| Res {
        in_app: IN_APP.iter().map(|(r, n)| (re(r), *n)).collect(),
        browsers: BROWSERS.iter().map(|(r, n)| (re(r), *n)).collect(),
        win_phone: re(r"Windows Phone(?: OS)? ([\d.]+)?"),
        win_nt: re(r"Windows NT (\d+\.\d)"),
        ios_ver: re(r"(?:iPhone|iPad|iPod)(?:.*?) OS (\d+)[_\d]*"),
        ios: re(r"iPhone|iPad|iPod"),
        android: re(r"Android[ /]?(\d+)?"),
        mac: re(r"Mac OS X (\d+)[_.](\d+)"),
        linux: re(r"Linux|X11|Ubuntu|Fedora|FreeBSD|OpenBSD"),
        tablet: re(r"iPad|Tablet|Kindle|Silk/|PlayBook|Nexus (?:7|9|10)\b|SM-T\d|Tab\b"),
        mobile: re(r"Mobi|iPhone|iPod|Windows Phone|BlackBerry|BB10|Opera Mini|IEMobile|webOS"),
        tv: re(r"(?i)SmartTV|SMART-TV|Tizen|Web0S|PlayStation|Xbox|Nintendo"),
    })
}

fn contains(h: &[u8], n: &[u8]) -> bool {
    memchr::memmem::find(h, n).is_some()
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// The classifier (cached per distinct User-Agent).
#[derive(Debug, Default)]
pub struct UserAgent {
    bots: Bots,
    cache: HashMap<Vec<u8>, Rc<UaInfo>>,
}

impl UserAgent {
    /// A new classifier.
    pub fn new() -> UserAgent {
        UserAgent::default()
    }

    /// Classifies a User-Agent.
    pub fn parse(&mut self, ua: &[u8]) -> Rc<UaInfo> {
        if let Some(v) = self.cache.get(ua) {
            return v.clone();
        }
        if self.cache.len() > 20000 {
            self.cache.clear();
        }
        let info = Rc::new(match self.bots.matches(ua) {
            Some(bot) => UaInfo {
                browser: "Bot".into(),
                browser_version: "Bot".into(),
                os: "Other".into(),
                os_version: "Other".into(),
                device: "bot".into(),
                bot: Some(bot),
            },
            None => {
                let (os, osv) = os(ua);
                let (b, bv) = browser(ua);
                let device = device(ua, &os).to_string();
                UaInfo {
                    browser: b,
                    browser_version: bv,
                    os,
                    os_version: osv,
                    device,
                    bot: None,
                }
            }
        });
        self.cache.insert(ua.to_vec(), info.clone());
        info
    }
}

fn browser(ua: &[u8]) -> (String, String) {
    let r = res();
    for (re, app) in &r.in_app {
        if re.is_match(ua) {
            return ("In-app".into(), format!("In-app {app}"));
        }
    }
    for (re, name) in &r.browsers {
        if let Some(m) = re.captures(ua) {
            let v = m.get(1).map(|x| s(x.as_bytes())).unwrap_or_default();
            if *name == "IE" && !v.is_empty() && crate::config::php_str_int(&v) < 5 {
                continue;
            }
            let bv = if v.is_empty() {
                name.to_string()
            } else {
                format!("{name} {v}")
            };
            return (name.to_string(), bv);
        }
    }
    ("Other".into(), "Other".into())
}

fn os(ua: &[u8]) -> (String, String) {
    let r = res();
    if r.win_phone.is_match(ua) {
        return ("Windows".into(), "Windows Phone".into());
    }
    if let Some(m) = r.win_nt.captures(ua) {
        let v = s(&m[1]);
        let label = WINDOWS
            .iter()
            .find(|(k, _)| *k == v)
            .map(|(_, l)| l.to_string())
            .unwrap_or_else(|| format!("Windows NT {v}"));
        return ("Windows".into(), label);
    }
    if contains(ua, b"Windows") {
        return ("Windows".into(), "Windows".into());
    }
    if let Some(m) = r.ios_ver.captures(ua) {
        return ("iOS".into(), format!("iOS {}", s(&m[1])));
    }
    if r.ios.is_match(ua) {
        return ("iOS".into(), "iOS".into());
    }
    if let Some(m) = r.android.captures(ua) {
        return match m.get(1).filter(|x| !x.as_bytes().is_empty()) {
            Some(v) => ("Android".into(), format!("Android {}", s(v.as_bytes()))),
            None => ("Android".into(), "Android".into()),
        };
    }
    if contains(ua, b"CrOS") {
        return ("ChromeOS".into(), "ChromeOS".into());
    }
    if let Some(m) = r.mac.captures(ua) {
        let major = crate::config::php_str_int(&s(&m[1]));
        if major == 10 {
            // Safari/Chrome/Firefox freeze the reported version at 10.15
            let minor = s(&m[2]);
            return (
                "macOS".into(),
                if crate::config::php_str_int(&minor) >= 15 {
                    "macOS 10.15+".into()
                } else {
                    format!("macOS 10.{minor}")
                },
            );
        }
        return ("macOS".into(), format!("macOS {major}"));
    }
    if contains(ua, b"Macintosh") || contains(ua, b"Mac OS") {
        return ("macOS".into(), "macOS".into());
    }
    if r.linux.is_match(ua) {
        return ("Linux".into(), "Linux".into());
    }
    ("Other".into(), "Other".into())
}

fn device(ua: &[u8], os: &str) -> &'static str {
    let r = res();
    if r.tablet.is_match(ua) {
        return "tablet";
    }
    if os == "Android" && !contains(ua, b"Mobile") {
        return "tablet";
    }
    if r.mobile.is_match(ua) {
        return "mobile";
    }
    if os == "Android" {
        return "mobile";
    }
    if ["Windows", "macOS", "Linux", "ChromeOS"].contains(&os) {
        return if r.tv.is_match(ua) {
            "other"
        } else {
            "desktop"
        };
    }
    "other"
}
