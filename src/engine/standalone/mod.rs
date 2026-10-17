//! Site discovery on servers without cPanel (docs/STANDALONE-COCKPIT.md, Phase A).
//!
//! The list of `(owner, domain, logs[], archives)` comes from, in order of
//! precedence:
//!
//! 1. the `sites` array of `config.json` (explicit, always collected);
//! 2. the Apache configuration ([`apache`]: EL `/etc/httpd`, Debian `/etc/apache2`);
//! 3. the nginx configuration ([`nginx`]: `/etc/nginx/nginx.conf`).
//!
//! A domain defined by a source of higher precedence hides the same domain
//! found by a later source; vhosts of the same source with the same name (port
//! 80 and 443) are merged. A log file is read for one site only: when several
//! sites write to the same file it stays with the first one (a combined log has
//! no vhost field to split it by). Auto-detected sites without a log file of
//! their own are reported by `webpulse sites` but not collected.
//!
//! The owner of a site is a Linux user (`SuexecUserGroup`, the owner of the
//! document root when it is a regular user) and groups sites into accounts
//! exactly as cPanel accounts do; without one the site belongs to [`LOCAL`].

pub mod apache;
pub mod nginx;

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::Config;
use crate::paths::{Paths, is_valid_domain, is_valid_user};

/// Account of the sites without an owner.
pub const LOCAL: &str = "local";

/// Lowest uid of a regular (human) user: document roots owned by system
/// accounts (`root`, `apache`, `nginx`, `www-data`) do not name an owner.
pub const UID_MIN: u32 = 1000;

/// Where a site was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// The `sites` array of config.json.
    Config,
    /// The Apache configuration.
    Apache,
    /// The nginx configuration.
    Nginx,
}

impl Source {
    /// `config`, `apache` or `nginx`.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Config => "config",
            Source::Apache => "apache",
            Source::Nginx => "nginx",
        }
    }
}

/// How a log format relates to the combined format the parser reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    /// Combined (optionally with a leading vhost and trailing fields).
    Combined,
    /// The common log format: parsed, but without referrer and user agent.
    Common,
    /// Not readable by WebPulse.
    Incompatible,
    /// Not declared (explicit `sites`): checked on the first line only.
    Unknown,
}

impl FormatKind {
    /// `combined`, `common`, `incompatible` or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            FormatKind::Combined => "combined",
            FormatKind::Common => "common",
            FormatKind::Incompatible => "incompatible",
            FormatKind::Unknown => "unknown",
        }
    }
    /// True when lines in this format can be collected.
    pub fn usable(self) -> bool {
        self != FormatKind::Incompatible
    }
}

/// One access log of a site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteLog {
    /// Absolute path.
    pub path: PathBuf,
    /// Written by an https virtual host / server.
    pub https: bool,
    /// The format as configured (nickname or format string).
    pub format: String,
    /// Compatibility of the format.
    pub kind: FormatKind,
    /// `file:line` of the declaration.
    pub origin: Option<String>,
}

/// A site found by discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    /// Where it was found.
    pub source: Source,
    /// Lower-case domain name.
    pub domain: String,
    /// Other names of the same site (`ServerAlias`, extra `server_name`s).
    pub aliases: Vec<String>,
    /// Account (Linux user) the site belongs to.
    pub owner: String,
    /// Access logs, in declaration order.
    pub logs: Vec<SiteLog>,
    /// Archive pattern from the configuration (`None`: derived from the logs).
    pub archives: Option<String>,
    /// Problems worth showing in `webpulse sites`.
    pub warnings: Vec<String>,
    /// `file:line` of the virtual host / server block.
    pub origin: Option<String>,
}

impl Site {
    fn new(source: Source, domain: &str, owner: &str) -> Site {
        Site {
            source,
            domain: domain.to_string(),
            aliases: Vec::new(),
            owner: owner.to_string(),
            logs: Vec::new(),
            archives: None,
            warnings: Vec::new(),
            origin: None,
        }
    }
}

/// What one configuration parser found.
#[derive(Debug, Clone, Default)]
pub struct Found {
    /// Sites in configuration order (not merged yet).
    pub sites: Vec<Site>,
    /// Problems not tied to a site.
    pub warnings: Vec<String>,
    /// Configuration files read.
    pub files: Vec<PathBuf>,
}

/// The result of standalone discovery.
#[derive(Debug, Clone, Default)]
pub struct Discovery {
    /// Sites WebPulse collects.
    pub sites: Vec<Site>,
    /// Sites found but not collected; the reason is the last warning.
    pub ignored: Vec<Site>,
    /// Problems not tied to a site.
    pub warnings: Vec<String>,
    /// Configuration files read (Apache and nginx).
    pub files: Vec<PathBuf>,
}

impl Discovery {
    /// Collected sites of `owner`.
    pub fn sites_of<'a>(&'a self, owner: &'a str) -> impl Iterator<Item = &'a Site> + 'a {
        self.sites.iter().filter(move |s| s.owner == owner)
    }
    /// The collected site `domain` of `owner`.
    pub fn site(&self, owner: &str, domain: &str) -> Option<&Site> {
        self.sites
            .iter()
            .find(|s| s.owner == owner && s.domain == domain)
    }
    /// Every owner with its domains (sorted).
    pub fn owners(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for s in &self.sites {
            out.entry(s.owner.clone())
                .or_default()
                .push(s.domain.clone());
        }
        for v in out.values_mut() {
            v.sort();
            v.dedup();
        }
        out
    }
}

/// This machine's host name, lower-case (`localhost` when it is not a usable
/// domain name): the domain of the default virtual host.
pub fn machine_hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the pointer and length describe `buf`, which is valid for writes.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    let name = if rc == 0 {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end])
            .trim()
            .to_ascii_lowercase()
    } else {
        String::new()
    };
    let name = name.trim_end_matches('.').to_string();
    if is_valid_domain(&name) {
        name
    } else {
        "localhost".into()
    }
}

/// Normalises a server name: scheme and port stripped, lower-case, no trailing
/// dot; `None` for wildcards, regexes and invalid names.
pub fn normalize_name(raw: &str) -> Option<String> {
    let mut s = raw.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
    // nginx catch-all names (`_`, `""`) and regexes (`~…`) name no site
    if s.starts_with('~') || s.bytes().all(|b| b == b'_') {
        return None;
    }
    if let Some(p) = s.find("://") {
        s = s[p + 3..].to_string();
    }
    if let Some(p) = s.find('/') {
        s.truncate(p);
    }
    if !s.starts_with('[')
        && let Some(p) = s.rfind(':')
        && s[p + 1..].bytes().all(|b| b.is_ascii_digit())
    {
        s.truncate(p);
    }
    let s = s.trim_end_matches('.').to_string();
    is_valid_domain(&s).then_some(s)
}

/// The owner named by a uid: a regular user (uid ≥ [`UID_MIN`], not `nobody`)
/// with a valid account name.
pub fn owner_of_uid(uid: u32) -> Option<String> {
    if uid < UID_MIN || uid == 65534 {
        return None;
    }
    crate::util::fs::user_name(uid).filter(|n| is_valid_user(n) && n != "root")
}

/// The owner of a document root (see [`owner_of_uid`]).
pub fn owner_of_path(p: &Path) -> Option<String> {
    std::fs::metadata(p)
        .ok()
        .and_then(|m| owner_of_uid(m.uid()))
}

/// The owner named in a `SuexecUserGroup` / `AssignUserID` / `sites[].owner`.
pub fn owner_name(raw: &str) -> Option<String> {
    let n = raw.trim().trim_start_matches('~');
    (is_valid_user(n) && n != "root" && n != "system").then(|| n.to_string())
}

/// Sites declared in `config.json` (`sites`), with warnings for invalid entries.
pub fn config_sites(config: &Config) -> Found {
    let mut f = Found::default();
    let list = match config.get("sites") {
        Value::Array(a) => a.clone(),
        Value::Null => Vec::new(),
        _ => {
            f.warnings
                .push("config.json: \"sites\" must be a list of objects".into());
            Vec::new()
        }
    };
    for (i, e) in list.iter().enumerate() {
        let at = format!("config.json sites[{i}]");
        let Some(obj) = e.as_object() else {
            f.warnings.push(format!("{at}: not an object"));
            continue;
        };
        let raw_domain = obj.get("domain").and_then(Value::as_str).unwrap_or("");
        let Some(domain) = normalize_name(raw_domain) else {
            f.warnings
                .push(format!("{at}: invalid or missing domain {raw_domain:?}"));
            continue;
        };
        let mut site = Site::new(Source::Config, &domain, LOCAL);
        site.origin = Some(at.clone());
        match obj.get("owner") {
            None | Some(Value::Null) => {}
            Some(Value::String(o)) => match owner_name(o) {
                Some(o) => site.owner = o,
                None => site.warnings.push(format!(
                    "invalid owner {o:?}: the site belongs to \"{LOCAL}\""
                )),
            },
            Some(_) => site.warnings.push(format!(
                "owner must be a string: the site belongs to \"{LOCAL}\""
            )),
        }
        let logs: Vec<String> = match obj.get("logs") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        };
        for l in logs {
            if !l.starts_with('/') {
                site.warnings
                    .push(format!("log {l:?} ignored: the path must be absolute"));
                continue;
            }
            let path = PathBuf::from(&l);
            let https = path
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase().contains("ssl"))
                .unwrap_or(false);
            if site.logs.iter().any(|x| x.path == path) {
                continue;
            }
            site.logs.push(SiteLog {
                path,
                https,
                format: "combined".into(),
                kind: FormatKind::Unknown,
                origin: Some(at.clone()),
            });
        }
        match obj.get("archives") {
            None | Some(Value::Null) => {}
            Some(Value::String(g)) if g.starts_with('/') => site.archives = Some(g.clone()),
            Some(_) => site
                .warnings
                .push("archives must be an absolute path or pattern".into()),
        }
        if let Some(Value::Array(a)) = obj.get("aliases") {
            for v in a {
                if let Some(n) = v.as_str().and_then(normalize_name)
                    && n != domain
                    && !site.aliases.contains(&n)
                {
                    site.aliases.push(n);
                }
            }
        }
        f.sites.push(site);
    }
    f
}

/// Whether a path exists: `Some(true|false)`, `None` when it cannot be checked
/// (permission denied on a parent directory).
pub fn exists(p: &Path) -> Option<bool> {
    match std::fs::metadata(p) {
        Ok(m) => Some(m.is_file()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(e) if e.raw_os_error() == Some(libc::ENOTDIR) => Some(false),
        Err(_) => None,
    }
}

/// Runs the three sources and applies precedence, de-duplication and the
/// "has a log of its own" rule.
pub fn discover(config: &Config, paths: &Paths) -> Discovery {
    let host = machine_hostname();
    let mut d = Discovery::default();
    let mut found: Vec<Found> = vec![config_sites(config)];
    for dir in paths.apache_conf_dirs() {
        if let Some(f) = apache::parse(dir, &host) {
            found.push(f);
        }
    }
    if let Some(f) = nginx::parse(paths.nginx_conf(), &host) {
        found.push(f);
    }
    let mut merged: Vec<Site> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for f in found {
        d.warnings.extend(f.warnings);
        d.files.extend(f.files);
        for site in f.sites {
            match index.get(&site.domain) {
                Some(&i) if merged[i].source == site.source => {
                    let into = &mut merged[i];
                    if into.owner != site.owner {
                        into.warnings.push(format!(
                            "another definition{} names owner {}: kept {}",
                            site.origin
                                .as_deref()
                                .map(|o| format!(" ({o})"))
                                .unwrap_or_default(),
                            site.owner,
                            into.owner
                        ));
                    }
                    for l in site.logs {
                        if !into.logs.iter().any(|x| x.path == l.path) {
                            into.logs.push(l);
                        }
                    }
                    for a in site.aliases {
                        if a != into.domain && !into.aliases.contains(&a) {
                            into.aliases.push(a);
                        }
                    }
                    for w in site.warnings {
                        if !into.warnings.contains(&w) {
                            into.warnings.push(w);
                        }
                    }
                    if into.archives.is_none() {
                        into.archives = site.archives;
                    }
                }
                Some(&i) => {
                    let mut s = site;
                    s.warnings.push(format!(
                        "also defined by {} ({}): that definition is used",
                        merged[i].source.as_str(),
                        merged[i].origin.as_deref().unwrap_or("?")
                    ));
                    d.ignored.push(s);
                }
                None => {
                    index.insert(site.domain.clone(), merged.len());
                    merged.push(site);
                }
            }
        }
    }
    // a log file belongs to one site
    let mut owner_of_log: HashMap<PathBuf, String> = HashMap::new();
    for site in &mut merged {
        let mut kept = Vec::new();
        for l in std::mem::take(&mut site.logs) {
            let key = std::fs::canonicalize(&l.path).unwrap_or_else(|_| l.path.clone());
            match owner_of_log.get(&key) {
                Some(other) => site.warnings.push(format!(
                    "{} is also the log of {other}: it is counted there only",
                    l.path.display()
                )),
                None => {
                    owner_of_log.insert(key, site.domain.clone());
                    kept.push(l);
                }
            }
        }
        site.logs = kept;
    }
    for mut site in merged {
        site.aliases.retain(|a| a != &site.domain);
        if site.source == Source::Config {
            if site.logs.is_empty() {
                site.warnings
                    .push("no log file configured (\"logs\" is empty)".into());
            }
            d.sites.push(site);
            continue;
        }
        let usable: Vec<&SiteLog> = site.logs.iter().filter(|l| l.kind.usable()).collect();
        if usable.is_empty() {
            if site.logs.is_empty() {
                site.warnings
                    .push("not collected: no access log of its own".into());
            } else {
                site.warnings.push(
                    "not collected: none of its access logs is in a combined-compatible format"
                        .into(),
                );
            }
            d.ignored.push(site);
            continue;
        }
        if usable.iter().all(|l| exists(&l.path) == Some(false)) {
            site.warnings
                .push("not collected: none of its access logs exists".into());
            d.ignored.push(site);
            continue;
        }
        d.sites.push(site);
    }
    d.sites
        .sort_by(|a, b| (&a.owner, &a.domain).cmp(&(&b.owner, &b.domain)));
    d
}

/// Archives of one site for backfill: the configured pattern, else the rotated
/// siblings of each live log (`<log>-<digit>…`, `<log>.<N>`, plain or `.gz`).
/// Live logs of any site are never archives. Returns `(path, https, stream)`
/// where `stream` is the live log the archive was rotated from.
pub fn site_archives(site: &Site, live: &[PathBuf]) -> Vec<(PathBuf, bool, PathBuf)> {
    let mut out: Vec<(PathBuf, bool, PathBuf)> = Vec::new();
    let is_live = |p: &Path| live.iter().any(|l| l == p);
    let stream_of = |p: &Path| -> (bool, PathBuf) {
        // the live log whose name is the longest prefix of the archive's name
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        site.logs
            .iter()
            .filter(|l| {
                l.path
                    .file_name()
                    .map(|n| name.starts_with(n.to_string_lossy().as_ref()))
                    .unwrap_or(false)
            })
            .max_by_key(|l| l.path.as_os_str().len())
            .map(|l| (l.https, l.path.clone()))
            .unwrap_or_else(|| {
                (
                    name.contains("ssl"),
                    PathBuf::from(site.archives.clone().unwrap_or_default()),
                )
            })
    };
    if let Some(pattern) = &site.archives {
        for p in crate::util::glob::expand(Path::new(pattern), Path::new("/")) {
            if is_live(&p) || !regular(&p) || !archive_name_ok(&p) {
                continue;
            }
            let (https, stream) = stream_of(&p);
            out.push((p, https, stream));
        }
        return out;
    }
    for l in &site.logs {
        let (Some(dir), Some(name)) = (l.path.parent(), l.path.file_name()) else {
            continue;
        };
        let name = name.to_string_lossy().into_owned();
        for f in crate::util::fs::scandir(dir) {
            let Some(rest) = f.strip_prefix(&name) else {
                continue;
            };
            if !is_rotation_suffix(rest, true) {
                continue;
            }
            let p = dir.join(&f);
            if is_live(&p) || !regular(&p) || out.iter().any(|(x, _, _)| *x == p) {
                continue;
            }
            out.push((p, l.https, l.path.clone()));
        }
    }
    out
}

fn regular(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_file())
        .unwrap_or(false)
}

/// Archives are plain or gzip files (other compressions are skipped).
fn archive_name_ok(p: &Path) -> bool {
    let n = p.to_string_lossy();
    !(n.ends_with(".bz2") || n.ends_with(".xz") || n.ends_with(".zst") || n.ends_with(".zip"))
}

/// True for the part of a rotated file name after the live log's name:
/// `-<digit>…` (logrotate `dateext`) or `.<N>`, optionally followed by `.gz`
/// when `gz` is allowed.
pub fn is_rotation_suffix(rest: &str, gz: bool) -> bool {
    let rest = match rest.strip_suffix(".gz") {
        Some(r) if gz => r,
        Some(_) => return false,
        None => rest,
    };
    if rest.ends_with(".bz2") || rest.ends_with(".xz") || rest.ends_with(".zst") {
        return false;
    }
    if let Some(n) = rest.strip_prefix('.') {
        return !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    }
    if let Some(d) = rest.strip_prefix('-') {
        return d.as_bytes().first().is_some_and(u8::is_ascii_digit)
            && d.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            normalize_name("WWW.Example.COM:443").as_deref(),
            Some("www.example.com")
        );
        assert_eq!(
            normalize_name("https://example.com/").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            normalize_name("example.com.").as_deref(),
            Some("example.com")
        );
        assert_eq!(normalize_name("*.example.com"), None);
        assert_eq!(normalize_name("~^www\\d+"), None);
        assert_eq!(normalize_name("_"), None);
        assert_eq!(owner_name("alice").as_deref(), Some("alice"));
        assert_eq!(owner_name("root"), None);
        assert_eq!(owner_name("Bad User"), None);
    }

    #[test]
    fn rotation_suffixes() {
        assert!(is_rotation_suffix("-20261017", true));
        assert!(is_rotation_suffix("-20261017.gz", true));
        assert!(is_rotation_suffix(".1", true));
        assert!(is_rotation_suffix(".12.gz", true));
        assert!(!is_rotation_suffix(".1.gz", false));
        assert!(!is_rotation_suffix("-ssl_log", true));
        assert!(!is_rotation_suffix(".old", true));
        assert!(!is_rotation_suffix("", true));
        assert!(!is_rotation_suffix("-20261017.bz2", true));
    }
}
