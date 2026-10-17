//! Update notification: asks GitHub for the latest release of `update.repo`,
//! compares it with this binary's version and keeps the answer in the registry
//! `settings` table under [`SETTING_KEY`] (not a configuration key: it is never
//! applied as an override). Used by `webpulse update-check`, by the nightly
//! `webpulse rollup` and by the WHM `admin.update*` actions; the browser never
//! talks to GitHub.
//!
//! Stored status (`update_status`):
//! `{current, latest, tag, is_newer, html_url, published_at, notes, checked_at, error}`
//! (`checked_at` in unix seconds; on a failed request `error` is set and the
//! previously known release is kept).

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::config::Config;

/// Registry `settings` key holding the last status.
pub const SETTING_KEY: &str = "update_status";
/// Default GitHub repository.
pub const DEFAULT_REPO: &str = "inalto/webpulse";
/// The nightly check skips the network when the last check is newer than this.
pub const CACHE_SECONDS: i64 = 20 * 3600;
/// "Check again" in WHM goes to the network at most this often.
pub const THROTTLE_SECONDS: i64 = 3600;
/// Request timeout.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// Largest response body accepted.
pub const MAX_BODY: u64 = 256 * 1024;
/// Release notes are cut to this many bytes.
pub const MAX_NOTES: usize = 2048;

/// A parsed `x.y.z[-pre]` version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Major, minor, patch.
    pub nums: [u64; 3],
    /// Pre-release suffix (`rc.1`), sorts lower than the release.
    pub pre: Option<String>,
}

/// Parses `v1.2.3`, `1.2.3`, `1.2.3-rc.1` (build metadata after `+` is ignored).
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s
        .strip_prefix('v')
        .or_else(|| s.strip_prefix('V'))
        .unwrap_or(s);
    let s = s.split('+').next().unwrap_or("");
    let (core, pre) = match s.find('-') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut nums = [0u64; 3];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 9 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        nums[i] = p.parse().ok()?;
    }
    let pre = match pre {
        None => None,
        Some(p)
            if !p.is_empty()
                && p.len() <= 40
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') =>
        {
            Some(p.to_string())
        }
        Some(_) => return None,
    };
    Some(Version { nums, pre })
}

fn cmp_pre(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

/// Semantic-version comparison; `None` when either side is not a version.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parse_version(a)?, parse_version(b)?);
    Some(a.nums.cmp(&b.nums).then_with(|| match (&a.pre, &b.pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(p), Some(q)) => cmp_pre(p, q),
    }))
}

/// `latest` is a newer version than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    compare(latest, current) == Some(Ordering::Greater)
}

/// A release tag install.sh may be given: `^v?\d+\.\d+\.\d+(?:[-.][0-9A-Za-z.]+)?$`.
pub fn valid_tag(tag: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^v?[0-9]+\.[0-9]+\.[0-9]+(?:[-.][0-9A-Za-z.]+)?$").expect("tag regex")
    })
    .is_match(tag)
        && tag.len() <= 64
}

/// `owner/name` with GitHub's characters.
pub fn valid_repo(repo: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && !s.starts_with('.')
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    matches!(repo.split_once('/'), Some((o, n)) if ok(o) && ok(n))
}

/// A release as GitHub describes it (the fields WebPulse keeps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// `tag_name` (`v0.4.0`).
    pub tag: String,
    /// Version without the `v`.
    pub version: String,
    /// Release page (https only).
    pub html_url: Option<String>,
    /// `published_at` (ISO 8601).
    pub published_at: Option<String>,
    /// Release notes, plain text, at most [`MAX_NOTES`] bytes.
    pub notes: String,
}

/// Plain text: CRLF → LF, control characters except LF and TAB dropped, trimmed, cut.
pub fn clean_notes(s: &str, max: usize) -> String {
    let s: String = s
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect();
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", s[..cut].trim_end())
}

fn safe_url(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    (s.starts_with("https://") && s.len() <= 500 && !s.bytes().any(|b| b <= 0x20 || b == 0x7f))
        .then(|| s.to_string())
}

fn safe_date(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    let b = s.as_bytes();
    (s.len() >= 10
        && s.len() <= 40
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b':' | b'.' | b'+')))
    .then(|| s.to_string())
}

/// Parses the body of `GET /repos/{repo}/releases/latest`.
pub fn parse_release(body: &[u8]) -> Result<Release> {
    let v: Value =
        serde_json::from_slice(body).map_err(|_| anyhow!("the release data is not JSON"))?;
    let tag = v
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("the release data has no tag_name"))?
        .trim()
        .to_string();
    if !valid_tag(&tag) {
        return Err(anyhow!(
            "release tag '{}' is not a version",
            crate::util::strs::truncate(&tag, 40, "…")
        ));
    }
    Ok(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        html_url: safe_url(v.get("html_url")),
        published_at: safe_date(v.get("published_at")),
        notes: clean_notes(
            v.get("body").and_then(Value::as_str).unwrap_or(""),
            MAX_NOTES,
        ),
        tag,
    })
}

/// Where the latest release is asked for: `update.url` when the override is
/// allowed (not root, or not the production configuration), else the GitHub API
/// for `update.repo`.
pub fn source_url(cfg: &Config, override_allowed: bool) -> Result<String> {
    if let Some(u) = cfg.get_str("update.url").filter(|u| !u.is_empty())
        && override_allowed
    {
        if !(u.starts_with("https://") || u.starts_with("http://")) {
            return Err(anyhow!("update.url must be an http(s) URL"));
        }
        return Ok(u);
    }
    let repo = repo_of(cfg);
    if !valid_repo(&repo) {
        return Err(anyhow!("update.repo '{repo}' is not owner/name"));
    }
    Ok(format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ))
}

/// `update.repo` (default [`DEFAULT_REPO`]).
pub fn repo_of(cfg: &Config) -> String {
    cfg.get_str("update.repo")
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| DEFAULT_REPO.to_string())
}

/// `update.check` (default true).
pub fn check_enabled(cfg: &Config) -> bool {
    match cfg.get("update.check") {
        Value::Null => true,
        v => crate::config::php_bool(v),
    }
}

/// `update.url` may be honoured by this process (the `WEBPULSE_NOW` rule).
pub fn override_allowed(root: bool, production_config: bool) -> bool {
    !root || !production_config
}

/// An `update.url` override is configured and allowed.
pub fn has_override(cfg: &Config, override_allowed: bool) -> bool {
    override_allowed && cfg.get_str("update.url").is_some_and(|u| !u.is_empty())
}

/// Fetches and parses the latest release.
pub fn fetch(url: &str) -> Result<Release> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .https_only(url.starts_with("https://"))
        .max_redirects(2)
        .user_agent(format!("webpulse/{}", crate::app::App::version()))
        .build()
        .into();
    let mut resp = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| anyhow!("cannot reach {}: {e}", host_of(url)))?;
    let status = resp.status().as_u16();
    if status == 404 {
        return Err(anyhow!("no published release found (HTTP 404)"));
    }
    if status != 200 {
        return Err(anyhow!("{} answered HTTP {status}", host_of(url)));
    }
    let body = resp
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .map_err(|e| anyhow!("cannot read the release data: {e}"))?;
    parse_release(&body)
}

fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

fn str_field(v: Option<&Value>, k: &str) -> Value {
    v.and_then(|s| s.get(k))
        .filter(|x| x.is_string())
        .cloned()
        .unwrap_or(Value::Null)
}

/// Status after a successful request.
pub fn status_ok(r: &Release, current: &str, now: i64) -> Value {
    json!({
        "current": current,
        "latest": r.version,
        "tag": r.tag,
        "is_newer": is_newer(&r.version, current),
        "html_url": r.html_url,
        "published_at": r.published_at,
        "notes": r.notes,
        "checked_at": now,
        "error": null,
    })
}

/// Status after a failed request: the previously known release is kept.
pub fn status_err(prev: Option<&Value>, error: &str, current: &str, now: i64) -> Value {
    let latest = str_field(prev, "latest");
    let newer = latest.as_str().is_some_and(|l| is_newer(l, current));
    json!({
        "current": current,
        "latest": latest,
        "tag": str_field(prev, "tag"),
        "is_newer": newer,
        "html_url": str_field(prev, "html_url"),
        "published_at": str_field(prev, "published_at"),
        "notes": str_field(prev, "notes"),
        "checked_at": now,
        "error": crate::util::strs::truncate(error, 300, "…"),
    })
}

/// `checked_at` of a stored status (0 when never checked).
pub fn checked_at(prev: Option<&Value>) -> i64 {
    prev.and_then(|p| p.get("checked_at"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// The stored status is older than `max_age` seconds (or missing).
pub fn is_stale(prev: Option<&Value>, now: i64, max_age: i64) -> bool {
    let at = checked_at(prev);
    at <= 0 || now - at >= max_age || at > now + 300
}

/// Runs a check: the network is used when `force` or the status is older than
/// [`CACHE_SECONDS`]. Returns the new status and whether the network was used.
pub fn check(
    url: Result<String>,
    prev: Option<&Value>,
    current: &str,
    now: i64,
    force: bool,
) -> (Value, bool) {
    if !force && !is_stale(prev, now, CACHE_SECONDS) {
        return (view(prev, current), false);
    }
    let res = url.and_then(|u| fetch(&u));
    let st = match res {
        Ok(r) => status_ok(&r, current, now),
        Err(e) => status_err(prev, &format!("{e:#}"), current, now),
    };
    (st, true)
}

/// A stored status re-evaluated against the running version (after an upgrade
/// the stored `current` is stale); an empty status when nothing is stored.
pub fn view(prev: Option<&Value>, current: &str) -> Value {
    let latest = str_field(prev, "latest");
    let newer = latest.as_str().is_some_and(|l| is_newer(l, current));
    let at = checked_at(prev);
    json!({
        "current": current,
        "latest": latest,
        "tag": str_field(prev, "tag"),
        "is_newer": newer,
        "html_url": str_field(prev, "html_url"),
        "published_at": str_field(prev, "published_at"),
        "notes": str_field(prev, "notes"),
        "checked_at": if at > 0 { json!(at) } else { Value::Null },
        "error": str_field(prev, "error"),
    })
}

fn trusted(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(m) = std::fs::metadata(path) else {
        return false;
    };
    let me = crate::util::fs::euid();
    (m.uid() == 0 || m.uid() == me) && m.mode() & 0o022 == 0
}

/// `<install_root>/install.sh` when this binary runs from an installation
/// (`<root>/bin/webpulse`) and the installer and its directory are owned by root
/// (or by the user running WebPulse) and not group/other-writable.
pub fn installer(install_root: &Path) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    let root = std::fs::canonicalize(bin.parent()?).ok()?;
    if std::fs::canonicalize(install_root).ok()? != root {
        return None;
    }
    let f = root.join("install.sh");
    (f.is_file() && trusted(&f) && trusted(&root)).then_some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver() {
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "1.0.0-rc.2"));
        assert!(is_newer("1.0.0-rc.10", "1.0.0-rc.2"));
        assert!(is_newer("1.0.0-rc.1", "1.0.0-beta"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(!is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(!is_newer("garbage", "0.3.0"));
        assert!(!is_newer("1.2", "0.3.0"));
        assert_eq!(compare("1.2.3+build.5", "1.2.3"), Some(Ordering::Equal));
        assert_eq!(compare("01.2.3", "1.2.3"), Some(Ordering::Equal));
        assert_eq!(parse_version("1.2.3-"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn tags_and_repos() {
        for ok in ["v0.4.0", "0.4.0", "v1.2.3-rc.1", "v1.2.3.post1"] {
            assert!(valid_tag(ok), "{ok}");
        }
        for bad in [
            "main",
            "v1.2",
            "v1.2.3;rm -rf /",
            "-v1.2.3",
            "v1.2.3 ",
            "v1.2.3-",
            "",
        ] {
            assert!(!valid_tag(bad), "{bad}");
        }
        assert!(valid_repo("inalto/webpulse") && valid_repo("a-b/c.d_e"));
        assert!(
            !valid_repo("inalto")
                && !valid_repo("a/b/c")
                && !valid_repo("../x")
                && !valid_repo("a/.x")
        );
    }

    #[test]
    fn release_parsing_and_status() {
        let body = br#"{"tag_name":"v0.4.0","html_url":"https://github.com/inalto/webpulse/releases/tag/v0.4.0",
            "published_at":"2026-10-17T08:00:00Z","body":"Added:\r\n- bell\u0007 icon\n","draft":false}"#;
        let r = parse_release(body).unwrap();
        assert_eq!(r.tag, "v0.4.0");
        assert_eq!(r.version, "0.4.0");
        assert_eq!(r.notes, "Added:\n- bell icon");
        assert_eq!(r.published_at.as_deref(), Some("2026-10-17T08:00:00Z"));
        let st = status_ok(&r, "0.3.0", 100);
        assert_eq!(st["is_newer"], json!(true));
        assert_eq!(st["checked_at"], json!(100));
        assert_eq!(st["error"], Value::Null);
        let keys: Vec<&String> = st.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "current",
                "latest",
                "tag",
                "is_newer",
                "html_url",
                "published_at",
                "notes",
                "checked_at",
                "error"
            ]
        );
        let e = status_err(Some(&st), "timeout", "0.3.0", 200);
        assert_eq!(e["latest"], json!("0.4.0"));
        assert_eq!(e["is_newer"], json!(true));
        assert_eq!(e["error"], json!("timeout"));
        assert_eq!(e["checked_at"], json!(200));
        let none = status_err(None, "x", "0.3.0", 1);
        assert_eq!(none["latest"], Value::Null);
        assert_eq!(none["is_newer"], json!(false));
        // after an upgrade the running version wins
        assert_eq!(view(Some(&st), "0.4.0")["is_newer"], json!(false));
        assert_eq!(view(None, "0.4.0")["checked_at"], Value::Null);

        let bad = parse_release(br#"{"tag_name":"v1; rm -rf /","html_url":"javascript:alert(1)"}"#);
        assert!(bad.is_err());
        let r = parse_release(
            br#"{"tag_name":"v1.0.0","html_url":"javascript:alert(1)","published_at":"<b>"}"#,
        )
        .unwrap();
        assert_eq!(r.html_url, None);
        assert_eq!(r.published_at, None);
        assert!(parse_release(b"not json").is_err());
        assert!(parse_release(br#"{"message":"Not Found"}"#).is_err());
        let long = "é".repeat(3000);
        let n = clean_notes(&long, MAX_NOTES);
        assert!(n.len() <= MAX_NOTES + 3 && n.ends_with('…'));
    }

    #[test]
    fn cache_window() {
        let st = json!({"checked_at": 1000});
        assert!(!is_stale(
            Some(&st),
            1000 + CACHE_SECONDS - 1,
            CACHE_SECONDS
        ));
        assert!(is_stale(Some(&st), 1000 + CACHE_SECONDS, CACHE_SECONDS));
        assert!(is_stale(None, 1000, CACHE_SECONDS));
        assert!(
            is_stale(Some(&st), 0, CACHE_SECONDS),
            "clock went backwards"
        );
        let (v, fetched) = check(Err(anyhow!("unused")), Some(&st), "0.3.0", 1001, false);
        assert!(!fetched);
        assert_eq!(v["checked_at"], json!(1000));
        let (v, fetched) = check(Err(anyhow!("offline")), Some(&st), "0.3.0", 1001, true);
        assert!(fetched);
        assert_eq!(v["error"], json!("offline"));
    }
}
