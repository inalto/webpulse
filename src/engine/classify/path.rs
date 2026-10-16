//! Request-target normaliser (docs/METRICS.md "Path normalisation"),
//! `WebPulse\Classify\Path`: extracts UTM parameters and ad click ids, strips the
//! query, decodes once, collapses slashes and trailing index files, truncates to
//! 512 bytes, forces UTF-8; derives file_type, is_static and is_download.

use foldhash::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use regex::Regex;

use super::referrer::CLICK_IDS;
use crate::util::strs;

/// Longest stored path (bytes).
pub const MAX_PATH: usize = 512;
/// Path of an empty (malformed) request target.
pub const INVALID: &str = "(invalid request)";

/// Download extensions.
pub const DOWNLOAD: &[&str] = &[
    "pdf", "zip", "rar", "7z", "gz", "tgz", "bz2", "xz", "dmg", "exe", "msi", "apk", "deb", "rpm",
    "iso", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "csv", "mp3", "mp4", "m4a",
    "mov", "avi", "mkv", "wav", "flac", "epub", "mobi",
];
/// Static asset extensions.
pub const STATIC: &[&str] = &[
    "css",
    "js",
    "mjs",
    "map",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "avif",
    "svg",
    "ico",
    "bmp",
    "tif",
    "tiff",
    "woff",
    "woff2",
    "ttf",
    "otf",
    "eot",
    "json",
    "xml",
    "txt",
    "webmanifest",
];
/// Extensions reported as file_type `html`.
pub const PAGE: &[&str] = &[
    "html", "htm", "php", "php5", "phtml", "shtml", "asp", "aspx", "jsp", "cgi", "pl", "cfm",
];

/// Result of the analysis of one request target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathInfo {
    /// Clean path (≤ 512 bytes, valid UTF-8).
    pub path: String,
    /// Dimension key (≤ 255 bytes with `…`).
    pub key: String,
    /// Lowercase extension or `html`.
    pub file_type: String,
    /// Static asset.
    pub is_static: bool,
    /// Download.
    pub is_download: bool,
    /// `utm_source` (clean, ≤ 128 bytes).
    pub utm_source: Option<String>,
    /// `utm_medium`.
    pub utm_medium: Option<String>,
    /// `utm_campaign`.
    pub utm_campaign: Option<String>,
    /// An ad click id (`gclid`, `fbclid`…) is present.
    pub click_id: bool,
    /// `http`/`https` of an absolute-form target.
    pub scheme: Option<String>,
}

/// The analyser (cached per distinct target).
#[derive(Debug)]
pub struct PathAnalyzer {
    static_prefixes: Vec<String>,
    cache: HashMap<Vec<u8>, Rc<PathInfo>>,
}

fn index_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i-u)/index\.(?:php|html?)$").expect("regex"))
}

impl PathAnalyzer {
    /// `static_prefixes`: path prefixes treated as static assets.
    pub fn new(static_prefixes: Vec<String>) -> PathAnalyzer {
        PathAnalyzer {
            static_prefixes,
            cache: HashMap::default(),
        }
    }

    /// Analyses a raw request target.
    pub fn analyze(&mut self, target: &[u8]) -> Rc<PathInfo> {
        if let Some(v) = self.cache.get(target) {
            return v.clone();
        }
        if self.cache.len() > 50000 {
            self.cache.clear();
        }
        let v = Rc::new(self.compute(target));
        self.cache.insert(target.to_vec(), v.clone());
        v
    }

    /// Uncached analysis.
    pub fn compute(&self, target: &[u8]) -> PathInfo {
        let mut r = PathInfo {
            path: INVALID.into(),
            key: INVALID.into(),
            file_type: "html".into(),
            is_static: false,
            is_download: false,
            utm_source: None,
            utm_medium: None,
            utm_campaign: None,
            click_id: false,
            scheme: None,
        };
        if target.is_empty() {
            return r;
        }
        if target == b"*" {
            r.path = "*".into();
            r.key = "*".into();
            return r;
        }
        let mut target: &[u8] = target;
        // absolute-form request target (proxies): http://host/path
        let lower8: Vec<u8> = target.iter().take(8).map(u8::to_ascii_lowercase).collect();
        let scheme_len = if lower8.starts_with(b"https://") {
            Some(5)
        } else if lower8.starts_with(b"http://") {
            Some(4)
        } else {
            None
        };
        if let Some(sl) = scheme_len {
            let after = &target[sl + 3..];
            let host_end = after
                .iter()
                .position(|b| matches!(b, b'/' | b'?' | b'#'))
                .unwrap_or(after.len());
            let mut rest = &after[host_end..];
            // `(.*)$`: no newline in the rest, except one at the very end
            let ok = match rest.iter().position(|&b| b == b'\n') {
                None => true,
                Some(p) if p == rest.len() - 1 => {
                    rest = &rest[..p];
                    true
                }
                Some(_) => false,
            };
            if ok {
                r.scheme = Some(String::from_utf8_lossy(&lower8[..sl]).into_owned());
                target = if rest.is_empty() { b"/" } else { rest };
            }
        }
        if let Some(h) = target.iter().position(|&b| b == b'#') {
            target = &target[..h];
        }
        let qpos = target.iter().position(|&b| b == b'?');
        let mut path: Vec<u8> = match qpos {
            None => target.to_vec(),
            Some(q) => target[..q].to_vec(),
        };
        if let Some(q) = qpos {
            let query = &target[q + 1..];
            let has = |n: &[u8]| memchr::memmem::find(query, n).is_some();
            if !query.is_empty()
                && (has(b"utm_") || has(b"clid") || has(b"braid") || has(b"li_fat_id"))
            {
                let vars = strs::parse_str(query);
                for (k, slot) in [
                    ("utm_source", &mut r.utm_source),
                    ("utm_medium", &mut r.utm_medium),
                    ("utm_campaign", &mut r.utm_campaign),
                ] {
                    if let Some(v) = vars.get_str(k) {
                        let t = strs::php_trim(v);
                        if !t.is_empty() {
                            *slot = Some(strs::key(t, 128));
                        }
                    }
                }
                r.click_id = CLICK_IDS.iter().any(|c| vars.isset(c));
            }
        }
        if path.contains(&b'%') {
            path = strs::rawurldecode(&path).into_owned();
        }
        if path.first() != Some(&b'/') {
            path.insert(0, b'/');
        }
        if memchr::memmem::find(&path, b"//").is_some() {
            let mut out = Vec::with_capacity(path.len());
            for &b in &path {
                if b == b'/' && out.last() == Some(&b'/') {
                    continue;
                }
                out.push(b);
            }
            path = out;
        }
        let clean = strs::truncate(&strs::clean(&path), MAX_PATH, "");
        let p = index_re().replace(&clean, "/").into_owned();
        r.key = strs::truncate(&p, 255, strs::ELLIPSIS);
        let base = match p.rfind('/') {
            Some(i) => &p[i + 1..],
            None => &p[..],
        };
        let ext = match base.rfind('.') {
            Some(i) => base[i + 1..].to_ascii_lowercase(),
            None => String::new(),
        };
        if !ext.is_empty() && ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
            r.file_type = if PAGE.contains(&ext.as_str()) {
                "html".into()
            } else {
                ext.clone()
            };
            r.is_download = DOWNLOAD.contains(&ext.as_str());
            r.is_static = STATIC.contains(&ext.as_str());
        }
        if !r.is_static {
            r.is_static = self
                .static_prefixes
                .iter()
                .any(|pre| !pre.is_empty() && p.starts_with(pre.as_str()));
        }
        r.path = p;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation() {
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
        assert_eq!(p.compute(b"/a%3Fb").path, "/a?b");
        let long = p.compute(
            "/\u{e9}"
                .to_string()
                .replace('\u{e9}', &"\u{e9}".repeat(400))
                .as_bytes(),
        );
        assert!(
            long.path.len() <= 512 && long.key.len() <= 255 && long.key.ends_with(strs::ELLIPSIS)
        );
        assert_eq!(p.compute(b"/a%00").path, "/a?");
        let t = |x: &[u8]| {
            let i = p.compute(x);
            (i.file_type, i.is_static, i.is_download)
        };
        assert_eq!(t(b"/"), ("html".into(), false, false));
        assert_eq!(t(b"/x.JPG"), ("jpg".into(), true, false));
        assert_eq!(t(b"/a.css?v=3"), ("css".into(), true, false));
        assert_eq!(t(b"/wp-content/uploads/a.pdf"), ("pdf".into(), true, true));
        assert_eq!(t(b"/doc.tar.gz"), ("gz".into(), false, true));
        assert_eq!(t(b"/wp-json/wp/v2/posts"), ("html".into(), true, false));
        assert_eq!(t(b"/a.verylongextension"), ("html".into(), false, false));
    }
}
