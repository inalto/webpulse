//! Referrer / acquisition classifier (docs/METRICS.md "Acquisition"),
//! `WebPulse\Classify\Referrer`.
//!
//! Channel precedence: paid > email > social/organic (utm_medium) > campaign (UTM) >
//! direct > organic > social > referral.

use foldhash::HashMap;
use std::sync::OnceLock;

use regex::bytes::Regex;

use crate::util::strs;

/// `[host pattern, engine key, query parameters carrying the keyword]`; patterns
/// are anchored (`^(?:…)$`) and matched against the host without `www.`.
pub const SEARCH: &[(&str, &str, &[&str])] = &[
    (
        r"(?:[a-z0-9-]+\.)?google\.(?:[a-z]{2,3}|co\.[a-z]{2}|com\.[a-z]{2})",
        "google",
        &["q"],
    ),
    (r"com\.google\.android\.googlequicksearchbox", "google", &[]),
    (r"com\.google\.android\.gm\.?lite", "google", &[]),
    (r"(?:[a-z]{2}\.)?bing\.com", "bing", &["q"]),
    (r"(?:html\.|lite\.)?duckduckgo\.com", "duckduckgo", &["q"]),
    (
        r"(?:[a-z]{2}\.)?search\.yahoo\.(?:com|co\.jp|[a-z]{2})",
        "yahoo",
        &["p", "q"],
    ),
    (r"search\.yahoo\.co\.jp", "yahoo", &["p"]),
    (
        r"yandex\.(?:ru|com|by|kz|ua|com\.tr|uz)",
        "yandex",
        &["text"],
    ),
    (r"ya\.ru", "yandex", &["text"]),
    (r"(?:m\.)?baidu\.com", "baidu", &["wd", "word"]),
    (r"ecosia\.org", "ecosia", &["q"]),
    (r"(?:[a-z]+\.)?qwant\.com", "qwant", &["q"]),
    (r"startpage\.com", "startpage", &["query", "q"]),
    (r"(?:[a-z]+\.)?startpage\.com", "startpage", &["query", "q"]),
    (r"search\.brave\.com", "brave", &["q"]),
    (r"(?:search\.|m\.search\.)?naver\.com", "naver", &["query"]),
    (r"(?:search\.)?seznam\.cz", "seznam", &["q"]),
    (r"(?:[a-z]+\.)?ask\.com", "ask", &["q"]),
    (r"search\.aol\.(?:com|co\.uk)", "aol", &["q", "query"]),
    (r"(?:[a-z]+\.)?yahoo\.com", "yahoo", &["p"]),
];

/// `[host pattern, network name]`.
pub const SOCIAL: &[(&str, &str)] = &[
    (r"(?:[a-z0-9-]+\.)?facebook\.com", "facebook"),
    (r"fb\.me", "facebook"),
    (r"(?:[a-z0-9-]+\.)?fb\.com", "facebook"),
    (r"(?:l\.)?instagram\.com", "instagram"),
    (r"(?:mobile\.)?twitter\.com", "twitter"),
    (r"x\.com", "twitter"),
    (r"t\.co", "twitter"),
    (r"(?:[a-z]{2}\.)?linkedin\.com", "linkedin"),
    (r"lnkd\.in", "linkedin"),
    (
        r"(?:[a-z]{2}\.)?pinterest\.(?:com|[a-z]{2}|co\.uk|com\.au)",
        "pinterest",
    ),
    (r"pin\.it", "pinterest"),
    (r"(?:old\.|new\.|out\.|m\.)?reddit\.com", "reddit"),
    (r"(?:[a-z]+\.)?tiktok\.com", "tiktok"),
    (r"(?:m\.)?youtube\.com", "youtube"),
    (r"youtu\.be", "youtube"),
    (r"threads\.(?:net|com)", "threads"),
    (r"bsky\.app", "bluesky"),
    (r"(?:[a-z]+\.)?snapchat\.com", "snapchat"),
    (r"(?:web\.|api\.|chat\.)?whatsapp\.com", "whatsapp"),
    (r"wa\.me", "whatsapp"),
    (r"(?:web\.)?telegram\.org", "telegram"),
    (r"t\.me", "telegram"),
    (r"mastodon\.(?:social|online|world)", "mastodon"),
    (
        r"(?:mstdn\.social|fosstodon\.org|hachyderm\.io|infosec\.exchange|mas\.to|techhub\.social|mastodon\.art)",
        "mastodon",
    ),
    (r"(?:m\.)?vk\.com", "vk"),
    (r"ok\.ru", "odnoklassniki"),
    (r"(?:[a-z0-9-]+\.)?tumblr\.com", "tumblr"),
    (r"(?:[a-z]{2}\.)?quora\.com", "quora"),
    (r"news\.ycombinator\.com", "hacker news"),
    (r"(?:[a-z]+\.)?discord(?:app)?\.com", "discord"),
    (r"(?:[a-z]+\.)?medium\.com", "medium"),
    (r"org\.telegram\.messenger", "telegram"),
    (r"com\.facebook\.katana", "facebook"),
    (r"com\.instagram\.android", "instagram"),
    (r"com\.twitter\.android", "twitter"),
    (r"com\.linkedin\.android", "linkedin"),
    (r"com\.reddit\.frontpage", "reddit"),
    (r"com\.whatsapp", "whatsapp"),
    (r"com\.zhiliaoapp\.musically", "tiktok"),
];

/// Webmail hosts (channel `email`).
pub const EMAIL: &[&str] = &[
    r"mail\.google\.com",
    r"com\.google\.android\.gm",
    r"outlook\.live\.com",
    r"outlook\.office(?:365)?\.com",
    r"(?:[a-z0-9-]+\.)?mail\.yahoo\.com",
    r"mail\.aol\.com",
    r"mail\.proton\.me",
    r"mail\.protonmail\.com",
    r"mail\.zoho\.(?:com|eu)",
    r"mail\.yandex\.(?:ru|com)",
    r"e\.mail\.ru",
    r"mail\.libero\.it",
    r"mail\.virgilio\.it",
    r"webmail\.[a-z0-9.-]+",
    r"mail\.tiscali\.it",
    r"posteo\.de",
    r"mail\.gmx\.(?:net|com|de)",
    r"navigator\.gmx\.(?:net|com)",
    r"web\.de",
    r"email\.t-online\.de",
    r"mail\.orange\.fr",
    r"webmail\.orange\.fr",
    r"icloud\.com",
    r"fastmail\.com",
    r"app\.fastmail\.com",
    r"mail\.163\.com",
    r"mail\.qq\.com",
    r"outlook\.com",
];

/// utm_medium values meaning a paid click.
pub const PAID_MEDIUMS: &[&str] = &[
    "cpc",
    "ppc",
    "paid",
    "paidsearch",
    "paid-search",
    "paid_search",
    "display",
    "cpm",
    "banner",
    "cpv",
    "cpa",
    "paidsocial",
    "paid-social",
    "paid_social",
];
/// utm_medium values meaning e-mail.
pub const EMAIL_MEDIUMS: &[&str] = &["email", "e-mail", "e_mail", "newsletter", "mail"];
/// utm_medium values meaning social.
pub const SOCIAL_MEDIUMS: &[&str] = &[
    "social",
    "social-network",
    "social-media",
    "social_media",
    "sm",
    "social network",
    "social media",
];
/// utm_medium values meaning organic.
pub const ORGANIC_MEDIUMS: &[&str] = &["organic"];
/// Query parameters that identify an ad click.
pub const CLICK_IDS: &[&str] = &[
    "gclid",
    "fbclid",
    "msclkid",
    "gbraid",
    "wbraid",
    "dclid",
    "ttclid",
    "twclid",
    "li_fat_id",
    "yclid",
];

/// Kind of a referrer host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    /// A search engine.
    Search,
    /// A social network.
    Social,
    /// A webmail.
    Email,
}

/// What the tables say about a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInfo {
    /// Kind, `None` for an ordinary site.
    pub kind: Option<HostKind>,
    /// Engine key / network name.
    pub name: Option<&'static str>,
    /// Keyword parameters (search engines).
    pub params: &'static [&'static str],
}

/// UTM data of the first hit of a visit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Utm<'a> {
    /// utm_source
    pub source: Option<&'a str>,
    /// utm_medium
    pub medium: Option<&'a str>,
    /// utm_campaign
    pub campaign: Option<&'a str>,
    /// gclid & co.
    pub click_id: bool,
}

/// Acquisition attributes of a visit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Acquisition {
    /// direct | organic | social | paid | email | campaign | referral
    pub channel: &'static str,
    /// External referrer host (`www.` stripped).
    pub referrer_host: Option<String>,
    /// Host + path (referral and social only).
    pub referrer_url: Option<String>,
    /// Search engine key.
    pub search_engine: Option<&'static str>,
    /// Search keyword (lowercase).
    pub keyword: Option<String>,
    /// Social network name.
    pub social: Option<&'static str>,
}

struct Tables {
    split: Regex,
    host_ok: Regex,
    email: regex::Regex,
    search: Vec<regex::Regex>,
    social: Vec<regex::Regex>,
    ws: regex::Regex,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    let anchored = |p: &str| regex::Regex::new(&format!("^(?:{p})$")).expect("regex");
    T.get_or_init(|| Tables {
        split: Regex::new(
            r"(?-u)(?i)^([a-z][a-z0-9+.-]*):(?://)?([^/?#:]*)(?::\d+)?([^?#]*)(?:\?([^#]*))?",
        )
        .expect("regex"),
        host_ok: Regex::new(r"(?-u)^[a-z0-9._-]+$").expect("regex"),
        email: anchored(&EMAIL.join("|")),
        search: SEARCH.iter().map(|(p, _, _)| anchored(p)).collect(),
        social: SOCIAL.iter().map(|(p, _)| anchored(p)).collect(),
        ws: regex::Regex::new(r"\s+").expect("regex"),
    })
}

/// Parsed Referer: host (lowercase, `www.` stripped), path, query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefParts {
    /// Host.
    pub host: String,
    /// Path (raw bytes).
    pub path: Vec<u8>,
    /// Query (raw bytes, without `?`).
    pub query: Vec<u8>,
}

/// Parses a Referer value; `None` when it has no usable host (`android-app://<package>/` works).
pub fn split(r: &[u8]) -> Option<RefParts> {
    let r = strs::php_trim(r);
    if r.is_empty() || r == b"-" {
        return None;
    }
    let t = tables();
    let m = t.split.captures(r)?;
    let raw_host = m.get(2).map(|x| x.as_bytes()).unwrap_or(b"");
    let mut end = raw_host.len();
    while end > 0 && raw_host[end - 1] == b'.' {
        end -= 1;
    }
    let host = raw_host[..end].to_ascii_lowercase();
    if host.is_empty() || !t.host_ok.is_match(&host) {
        return None;
    }
    let host = String::from_utf8_lossy(host.strip_prefix(b"www.").unwrap_or(&host)).into_owned();
    Some(RefParts {
        host,
        path: m.get(3).map(|x| x.as_bytes().to_vec()).unwrap_or_default(),
        query: m.get(4).map(|x| x.as_bytes().to_vec()).unwrap_or_default(),
    })
}

/// True when `host` (www. stripped) is the domain itself or its www. variant.
pub fn is_internal(host: &str, domain: &str) -> bool {
    let d = domain.to_ascii_lowercase();
    let d = d.strip_prefix("www.").unwrap_or(&d);
    host == d
}

/// The classifier (caches host lookups).
#[derive(Debug, Default)]
pub struct Referrer {
    cache: HashMap<String, HostInfo>,
}

impl Referrer {
    /// A new classifier.
    pub fn new() -> Referrer {
        Referrer::default()
    }

    /// External referrer host of a hit, `None` when none or internal.
    pub fn external_host(&self, r: Option<&[u8]>, domain: &str) -> Option<String> {
        let p = split(r?)?;
        if is_internal(&p.host, domain) {
            return None;
        }
        Some(strs::key255(p.host.as_bytes()))
    }

    /// Referrer host including internal ones (for `not_found_referrer`).
    pub fn any_host(&self, r: Option<&[u8]>) -> Option<String> {
        let p = split(r?)?;
        Some(strs::key255(p.host.as_bytes()))
    }

    /// What the tables say about a host.
    pub fn host_info(&mut self, host: &str) -> HostInfo {
        if let Some(i) = self.cache.get(host) {
            return i.clone();
        }
        if self.cache.len() > 20000 {
            self.cache.clear();
        }
        let t = tables();
        let mut info = HostInfo {
            kind: None,
            name: None,
            params: &[],
        };
        if t.email.is_match(host) {
            info.kind = Some(HostKind::Email);
        } else if let Some(i) = t.search.iter().position(|re| re.is_match(host)) {
            info = HostInfo {
                kind: Some(HostKind::Search),
                name: Some(SEARCH[i].1),
                params: SEARCH[i].2,
            };
        } else if let Some(i) = t.social.iter().position(|re| re.is_match(host)) {
            info = HostInfo {
                kind: Some(HostKind::Social),
                name: Some(SOCIAL[i].1),
                params: &[],
            };
        }
        self.cache.insert(host.to_string(), info.clone());
        info
    }

    fn keyword(&self, query: &[u8], params: &[&str]) -> Option<String> {
        if query.is_empty() || params.is_empty() {
            return None;
        }
        let q = strs::parse_str(query);
        for p in params {
            if let Some(v) = q.get_str(p) {
                let c = strs::clean(v);
                let k = tables().ws.replace_all(&c, " ");
                let k = strs::php_trim_str(&k);
                if !k.is_empty() {
                    return Some(strs::key(k.to_lowercase().as_bytes(), 128));
                }
            }
        }
        None
    }

    /// Classifies the first hit of a visit.
    pub fn classify(&mut self, r: Option<&[u8]>, domain: &str, utm: &Utm<'_>) -> Acquisition {
        let mut out = Acquisition {
            channel: "direct",
            ..Acquisition::default()
        };
        let p = r.and_then(split);
        let external = p
            .as_ref()
            .map(|p| !is_internal(&p.host, domain))
            .unwrap_or(false);
        let mut kind = None;
        if external && let Some(p) = &p {
            out.referrer_host = Some(strs::key255(p.host.as_bytes()));
            let info = self.host_info(&p.host);
            kind = info.kind;
            match info.kind {
                Some(HostKind::Search) => {
                    out.search_engine = info.name;
                    out.keyword = self.keyword(&p.query, info.params);
                }
                Some(HostKind::Social) => out.social = info.name,
                _ => {}
            }
            if kind.is_none() || kind == Some(HostKind::Social) {
                let mut url = p.host.as_bytes().to_vec();
                if p.path.is_empty() {
                    url.push(b'/');
                } else {
                    url.extend_from_slice(&p.path);
                }
                out.referrer_url = Some(strs::key255(&url));
            }
        }
        let medium = utm.medium.map(|m| m.to_ascii_lowercase());
        let m = medium.as_deref();
        let has_utm = utm.source.is_some() || utm.medium.is_some() || utm.campaign.is_some();
        out.channel = if m.map(|m| PAID_MEDIUMS.contains(&m)).unwrap_or(false) || utm.click_id {
            "paid"
        } else if m.map(|m| EMAIL_MEDIUMS.contains(&m)).unwrap_or(false)
            || kind == Some(HostKind::Email)
        {
            "email"
        } else if m.map(|m| SOCIAL_MEDIUMS.contains(&m)).unwrap_or(false) {
            "social"
        } else if m.map(|m| ORGANIC_MEDIUMS.contains(&m)).unwrap_or(false) {
            "organic"
        } else if has_utm {
            "campaign"
        } else if !external {
            "direct"
        } else if kind == Some(HostKind::Search) {
            "organic"
        } else if kind == Some(HostKind::Social) {
            "social"
        } else {
            "referral"
        };
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels() {
        let mut r = Referrer::new();
        let none = Utm::default();
        type Case<'a> = (
            Option<&'a str>,
            &'a str,
            Option<&'a str>,
            Option<&'a str>,
            Option<&'a str>,
        );
        let cases: &[Case] = &[
            (None, "direct", None, None, None),
            (Some("-"), "direct", None, None, None),
            (
                Some("https://www.example.com/x"),
                "direct",
                None,
                None,
                None,
            ),
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
                Some("https://yandex.ru/search/?text=%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82"),
                "organic",
                Some("yandex.ru"),
                Some("yandex"),
                Some("привет"),
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
            (
                Some("https://mail.google.com/mail/u/0/"),
                "email",
                Some("mail.google.com"),
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
        let paid = Utm {
            source: Some("google"),
            medium: Some("cpc"),
            ..Utm::default()
        };
        assert_eq!(
            r.classify(Some(b"https://www.google.com/"), "example.com", &paid)
                .channel,
            "paid"
        );
        let camp = Utm {
            source: Some("partner"),
            medium: Some("banner-free"),
            campaign: Some("launch"),
            click_id: false,
        };
        assert_eq!(
            r.classify(Some(b"https://www.google.com/"), "example.com", &camp)
                .channel,
            "campaign"
        );
        assert_eq!(
            r.any_host(Some(b"https://www.example.com/a")).as_deref(),
            Some("example.com")
        );
        assert!(
            r.external_host(Some(b"https://www.example.com/a"), "example.com")
                .is_none()
        );
    }
}
