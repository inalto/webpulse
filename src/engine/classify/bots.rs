//! Bot detection from the User-Agent (docs/METRICS.md "bot" class),
//! `WebPulse\Classify\Bots`. An empty UA (or `-`) is a bot too.

use foldhash::HashMap;
use std::sync::OnceLock;

use regex::bytes::Regex;

/// `[case-insensitive substring of the User-Agent, family name]`; the first match
/// wins (`Classify/data/bots.php` of 0.1.0).
pub const TABLE: &[(&str, &str)] = &[
    // AI crawlers and assistants
    ("gptbot", "GPTBot"),
    ("chatgpt-user", "ChatGPT-User"),
    ("oai-searchbot", "OAI-SearchBot"),
    ("claudebot", "ClaudeBot"),
    ("claude-user", "Claude-User"),
    ("claude-searchbot", "Claude-SearchBot"),
    ("claude-web", "ClaudeBot"),
    ("anthropic-ai", "ClaudeBot"),
    ("perplexitybot", "PerplexityBot"),
    ("perplexity-user", "Perplexity-User"),
    ("ccbot", "CCBot"),
    ("bytespider", "Bytespider"),
    ("amazonbot", "Amazonbot"),
    ("applebot-extended", "Applebot"),
    ("meta-externalagent", "Meta-ExternalAgent"),
    ("meta-externalfetcher", "Meta-ExternalAgent"),
    ("facebookbot", "Meta-ExternalAgent"),
    ("google-extended", "Google-Extended"),
    ("cohere-ai", "cohere-ai"),
    ("diffbot", "Diffbot"),
    ("youbot", "YouBot"),
    ("imagesiftbot", "ImagesiftBot"),
    ("timpibot", "Timpibot"),
    ("omgili", "Omgilibot"),
    ("ai2bot", "AI2Bot"),
    ("duckassistbot", "DuckAssistBot"),
    ("mistralai-user", "MistralAI-User"),
    // search engines
    ("googlebot", "Googlebot"),
    ("adsbot-google", "Googlebot"),
    ("mediapartners-google", "Googlebot"),
    ("google-inspectiontool", "Googlebot"),
    ("googleother", "GoogleOther"),
    ("storebot-google", "Googlebot"),
    ("google-read-aloud", "Googlebot"),
    ("feedfetcher-google", "Googlebot"),
    ("google page speed", "PageSpeed Insights"),
    ("chrome-lighthouse", "Lighthouse"),
    ("lighthouse", "Lighthouse"),
    ("bingbot", "bingbot"),
    ("bingpreview", "bingbot"),
    ("adidxbot", "bingbot"),
    ("msnbot", "bingbot"),
    ("yandex", "YandexBot"),
    ("baiduspider", "Baiduspider"),
    ("duckduckbot", "DuckDuckBot"),
    ("duckduckgo-favicons-bot", "DuckDuckBot"),
    ("applebot", "Applebot"),
    ("petalbot", "PetalBot"),
    ("sogou", "Sogou"),
    ("exabot", "Exabot"),
    ("qwantify", "Qwantbot"),
    ("qwantbot", "Qwantbot"),
    ("mojeekbot", "MojeekBot"),
    ("seznambot", "SeznamBot"),
    ("yeti/", "Yeti"),
    ("coccocbot", "coccocbot"),
    ("bravebot", "Bravebot"),
    // SEO tools
    ("ahrefsbot", "AhrefsBot"),
    ("ahrefssiteaudit", "AhrefsBot"),
    ("semrushbot", "SemrushBot"),
    ("siteauditbot", "SemrushBot"),
    ("mj12bot", "MJ12bot"),
    ("dotbot", "DotBot"),
    ("blexbot", "BLEXBot"),
    ("dataforseobot", "DataForSeoBot"),
    ("serpstatbot", "serpstatbot"),
    ("barkrowler", "Barkrowler"),
    ("screaming frog", "Screaming Frog"),
    ("seokicks", "SEOkicks"),
    ("linkdexbot", "Linkdex"),
    ("megaindex", "MegaIndex"),
    ("rogerbot", "Moz"),
    ("zoominfobot", "ZoominfoBot"),
    // social previews
    ("facebookexternalhit", "facebookexternalhit"),
    ("facebot", "facebookexternalhit"),
    ("twitterbot", "Twitterbot"),
    ("linkedinbot", "LinkedInBot"),
    ("slackbot", "Slackbot"),
    ("discordbot", "Discordbot"),
    ("telegrambot", "TelegramBot"),
    ("whatsapp/", "WhatsApp"),
    ("pinterestbot", "Pinterestbot"),
    ("redditbot", "redditbot"),
    ("embedly", "Embedly"),
    ("skypeuripreview", "Skype"),
    ("mastodon/", "Mastodon"),
    ("bluesky", "Bluesky"),
    ("iframely", "Iframely"),
    ("vkshare", "VKShare"),
    ("snap url preview", "Snapchat"),
    // monitoring & performance
    ("uptimerobot", "UptimeRobot"),
    ("pingdom", "Pingdom"),
    ("statuscake", "StatusCake"),
    ("gtmetrix", "GTmetrix"),
    ("site24x7", "Site24x7"),
    ("betteruptime", "Better Uptime"),
    ("uptime-kuma", "Uptime Kuma"),
    ("uptimekuma", "Uptime Kuma"),
    ("freshping", "Freshping"),
    ("newrelicpinger", "New Relic"),
    ("datadog", "Datadog"),
    ("jetpack", "Jetpack"),
    ("headlesschrome", "HeadlessChrome"),
    ("phantomjs", "PhantomJS"),
    ("ptst/", "WebPageTest"),
    // security scanners
    ("censysinspect", "Censys"),
    ("expanse", "Expanse"),
    ("zgrab", "zgrab"),
    ("masscan", "masscan"),
    ("nmap", "Nmap"),
    ("nuclei", "Nuclei"),
    ("l9explore", "LeakIX"),
    ("l9tcpid", "LeakIX"),
    ("internetmeasurement", "InternetMeasurement"),
    ("paloaltonetworks", "Palo Alto Networks"),
    ("wpscan", "WPScan"),
    ("nikto", "Nikto"),
    ("sqlmap", "sqlmap"),
    // libraries & tools
    ("curl/", "curl"),
    ("wget", "Wget"),
    ("python-requests", "python-requests"),
    ("python-urllib", "python-urllib"),
    ("python-httpx", "python-httpx"),
    ("aiohttp", "aiohttp"),
    ("go-http-client", "Go-http-client"),
    ("okhttp", "okhttp"),
    ("java/", "Java"),
    ("apache-httpclient", "Apache-HttpClient"),
    ("libwww-perl", "libwww-perl"),
    ("lwp::simple", "libwww-perl"),
    ("axios/", "axios"),
    ("node-fetch", "node-fetch"),
    ("undici", "node-fetch"),
    ("scrapy", "Scrapy"),
    ("guzzlehttp", "Guzzle"),
    ("php/", "PHP"),
    ("wordpress/", "WordPress"),
    ("httrack", "HTTrack"),
    ("postmanruntime", "Postman"),
    ("insomnia", "Insomnia"),
    ("feedly", "Feedly"),
    ("feedburner", "FeedBurner"),
    ("w3c_validator", "W3C Validator"),
    ("validator.nu", "W3C Validator"),
    ("archive.org_bot", "Internet Archive"),
    ("ia_archiver", "Internet Archive"),
    ("heritrix", "Internet Archive"),
];

struct Res {
    generic: Regex,
    false_positives: Regex,
    script: Regex,
}

fn res() -> &'static Res {
    static R: OnceLock<Res> = OnceLock::new();
    R.get_or_init(|| Res {
        // generic fragments meaning "automated client" (after the named table)
        generic: Regex::new(r"(?-u)(?i)bot\b|bot[/_;:.)\- ]|crawl|spider|slurp|scrap|fetcher|scanner|preview|monitor|checker|http[-_ ]?client|headless").expect("regex"),
        // real browsers / devices whose UA contains a generic fragment
        false_positives: Regex::new(r"(?-u)(?i)cubot|bot-?tle|robotics").expect("regex"),
        // no browser-like token at all: scripts and libraries
        script: Regex::new(r"(?-u)^[a-z][\w.-]*/[\d.]+\s*(\(|$)").expect("regex"),
    })
}

/// Bot family of a User-Agent, or `None` for humans (cached).
#[derive(Debug, Default)]
pub struct Bots {
    cache: HashMap<Vec<u8>, Option<&'static str>>,
}

impl Bots {
    /// A new detector.
    pub fn new() -> Bots {
        Bots::default()
    }

    /// Bot family name when `ua` is a bot; `Empty user agent` for an empty UA,
    /// `Other bot` for generic matches.
    pub fn matches(&mut self, ua: &[u8]) -> Option<&'static str> {
        if let Some(v) = self.cache.get(ua) {
            return *v;
        }
        if self.cache.len() > 20000 {
            self.cache.clear();
        }
        let v = detect(ua);
        self.cache.insert(ua.to_vec(), v);
        v
    }
}

/// Uncached detection.
pub fn detect(ua: &[u8]) -> Option<&'static str> {
    let t = crate::util::strs::php_trim(ua);
    if t.is_empty() || t == b"-" {
        return Some("Empty user agent");
    }
    let l = t.to_ascii_lowercase();
    for (needle, name) in TABLE {
        if memchr::memmem::find(&l, needle.as_bytes()).is_some() {
            return Some(name);
        }
    }
    let r = res();
    if r.generic.is_match(&l) && !r.false_positives.is_match(&l) {
        return Some("Other bot");
    }
    if memchr::memmem::find(&l, b"mozilla/").is_none()
        && memchr::memmem::find(&l, b"opera").is_none()
        && r.script.is_match(&l)
    {
        return Some("Other bot");
    }
    None
}
