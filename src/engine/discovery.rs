//! cPanel account discovery without any cPanel API (`WebPulse\Discovery\CpanelUsers`)
//! and log-file discovery (`WebPulse\Discovery\LogFiles`).
//!
//! * `/var/cpanel/users/<user>`: `DNS=` (main domain), `DNS<n>=` (other domains),
//!   `OWNER=` (reseller or root);
//! * `/etc/userdomains`: `domain: user` lines (adds subdomains);
//! * `/var/cpanel/userdata/<user>/main` (optional): exact kinds of the addon /
//!   parked / sub domains. Without it kinds are inferred: `DNS=` is main, a domain
//!   ending in `.<another domain of the user>` is sub, other `DNS<n>=` are addon,
//!   anything else unknown.
//!
//! Which accounts, domains and logs exist is asked through a [`Provider`]:
//! [`CpanelProvider`] (the readers above) on cPanel servers, [`StandaloneProvider`]
//! (`engine::standalone`: config `sites`, Apache, nginx) elsewhere; `mode` in
//! config.json chooses ([`resolve_mode`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use indexmap::IndexMap;
use regex::bytes::Regex;

use crate::paths::{Paths, is_valid_domain, is_valid_user};
use crate::util::fs;

/// What discovery knows about one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    /// Reseller owning the account (`OWNER=`), `None` when unknown.
    pub owner: Option<String>,
    /// Domains, sorted.
    pub domains: Vec<String>,
    /// domain → main | addon | parked | sub | unknown (sorted by domain).
    pub kinds: BTreeMap<String, String>,
    /// Main domain (`DNS=`).
    pub main: Option<String>,
    /// Home directory.
    pub home: String,
}

/// Reads `/var/cpanel/users`, `/etc/userdomains` and `/var/cpanel/userdata`.
#[derive(Debug)]
pub struct CpanelUsers {
    paths: Paths,
    all: Mutex<Option<BTreeMap<String, UserInfo>>>,
    userdomains: Mutex<Option<HashMap<String, Vec<String>>>>,
}

fn read_capped(path: &Path, max: u64) -> Option<Vec<u8>> {
    let f = std::fs::File::open(path).ok()?;
    let mut v = Vec::new();
    f.take(max).read_to_end(&mut v).ok()?;
    Some(v)
}

/// `preg_split('/\r?\n/', $raw)`
fn split_lines(raw: &[u8]) -> Vec<&[u8]> {
    raw.split(|&b| b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .collect()
}

fn lower(b: &[u8]) -> String {
    String::from_utf8_lossy(&b.to_ascii_lowercase()).into_owned()
}

impl CpanelUsers {
    /// Discovery over the configured paths.
    pub fn new(paths: &Paths) -> CpanelUsers {
        CpanelUsers {
            paths: paths.clone(),
            all: Mutex::new(None),
            userdomains: Mutex::new(None),
        }
    }

    fn user_file(&self, user: &str) -> PathBuf {
        self.paths.cpanel_users_dir().join(user)
    }

    /// Every account, sorted by name.
    pub fn all(&self) -> BTreeMap<String, UserInfo> {
        if let Some(a) = self.all.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return a.clone();
        }
        let mut out = BTreeMap::new();
        for name in fs::scandir(self.paths.cpanel_users_dir()) {
            if name.starts_with('.') || name == "system" || name == "root" || !is_valid_user(&name)
            {
                continue;
            }
            let file = self.user_file(&name);
            if fs::is_link(&file) || !file.is_file() {
                continue;
            }
            out.insert(name.clone(), self.read(&name));
        }
        *self.all.lock().unwrap_or_else(|e| e.into_inner()) = Some(out.clone());
        out
    }

    /// One account, or `None` when it does not exist (cheaper than `all()`).
    pub fn get(&self, user: &str) -> Option<UserInfo> {
        if let Some(a) = self.all.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return a.get(user).cloned();
        }
        if !self.exists(user) {
            return None;
        }
        let file = self.user_file(user);
        if fs::is_link(&file) || !file.is_file() {
            return None;
        }
        Some(self.read(user))
    }

    /// True when `/var/cpanel/users/<user>` exists.
    pub fn exists(&self, user: &str) -> bool {
        is_valid_user(user) && self.user_file(user).is_file()
    }

    /// Reseller owning the account (`OWNER=`), `None` when unknown.
    pub fn owner(&self, user: &str) -> Option<String> {
        if !self.exists(user) {
            return None;
        }
        self.parse_user_file(user)
            .get("OWNER")
            .filter(|o| !o.is_empty())
            .cloned()
    }

    /// Domains of an account with their kind.
    pub fn domains_of(&self, user: &str) -> BTreeMap<String, String> {
        if let Some(a) = self.all.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            && let Some(i) = a.get(user)
        {
            return i.kinds.clone();
        }
        if self.exists(user) {
            self.read(user).kinds
        } else {
            BTreeMap::new()
        }
    }

    /// Home directory of the account.
    pub fn home(&self, user: &str) -> String {
        if self.paths.home_root() == Path::new("/home")
            && let Some((_, _, dir)) = fs::passwd(user)
            && !dir.is_empty()
        {
            return dir;
        }
        format!("{}/{user}", self.paths.home_root().display())
    }

    fn read(&self, user: &str) -> UserInfo {
        let kv = self.parse_user_file(user);
        let main = kv
            .get("DNS")
            .map(|d| d.to_ascii_lowercase())
            .filter(|d| is_valid_domain(d));
        let dns_re = dns_key_re();
        let mut dns: Vec<String> = Vec::new();
        for (k, v) in &kv {
            let lv = v.to_ascii_lowercase();
            if dns_re.is_match(k.as_bytes()) && is_valid_domain(&lv) {
                dns.push(lv);
            }
        }
        let extra = self.userdomains().get(user).cloned().unwrap_or_default();
        let mut all: Vec<String> = Vec::new();
        for d in main.iter().cloned().chain(dns.iter().cloned()).chain(extra) {
            if !all.contains(&d) {
                all.push(d);
            }
        }
        let kinds = self.kinds(user, main.as_deref(), &dns, &all);
        UserInfo {
            owner: kv.get("OWNER").filter(|o| !o.is_empty()).cloned(),
            domains: kinds.keys().cloned().collect(),
            kinds,
            main,
            home: self.home(user),
        }
    }

    fn kinds(
        &self,
        user: &str,
        main: Option<&str>,
        dns: &[String],
        all: &[String],
    ) -> BTreeMap<String, String> {
        let ud = self.userdata(user);
        let mut kinds = BTreeMap::new();
        for d in all {
            if let Some(k) = ud.as_ref().and_then(|u| u.get(d)) {
                kinds.insert(d.clone(), k.clone());
                continue;
            }
            if Some(d.as_str()) == main {
                kinds.insert(d.clone(), "main".into());
                continue;
            }
            let sub = all.iter().any(|o| o != d && d.ends_with(&format!(".{o}")));
            let kind = if sub {
                "sub"
            } else if dns.contains(d) {
                "addon"
            } else {
                "unknown"
            };
            kinds.insert(d.clone(), kind.to_string());
        }
        kinds
    }

    /// `KEY=value` pairs of `/var/cpanel/users/<user>` (first-occurrence order, last value).
    fn parse_user_file(&self, user: &str) -> IndexMap<String, String> {
        let mut kv = IndexMap::new();
        if !is_valid_user(user) {
            return kv;
        }
        let file = self.user_file(user);
        if fs::is_link(&file) {
            return kv;
        }
        let Some(raw) = read_capped(&file, 1024 * 1024) else {
            return kv;
        };
        for line in split_lines(&raw) {
            if line.is_empty() || line[0] == b'#' || !line.contains(&b'=') {
                continue;
            }
            let p = line.iter().position(|&b| b == b'=').unwrap_or(0);
            let k = String::from_utf8_lossy(crate::util::strs::php_trim(&line[..p])).into_owned();
            let v =
                String::from_utf8_lossy(crate::util::strs::php_trim(&line[p + 1..])).into_owned();
            kv.insert(k, v);
        }
        kv
    }

    /// user → domains from `/etc/userdomains`.
    fn userdomains(&self) -> HashMap<String, Vec<String>> {
        let mut guard = self.userdomains.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(u) = guard.as_ref() {
            return u.clone();
        }
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        if let Ok(raw) = std::fs::read(self.paths.userdomains_file()) {
            for line in split_lines(&raw) {
                let Some(p) = line.iter().position(|&b| b == b':') else {
                    continue;
                };
                let d = lower(crate::util::strs::php_trim(&line[..p]));
                let u = String::from_utf8_lossy(crate::util::strs::php_trim(&line[p + 1..]))
                    .into_owned();
                if d == "*" || u == "nobody" || !is_valid_user(&u) || !is_valid_domain(&d) {
                    continue;
                }
                out.entry(u).or_default().push(d);
            }
        }
        *guard = Some(out.clone());
        out
    }

    /// Kinds from `/var/cpanel/userdata/<user>/main` (YAML subset), `None` when absent.
    fn userdata(&self, user: &str) -> Option<HashMap<String, String>> {
        let file = self.paths.cpanel_userdata_dir().join(user).join("main");
        if !file.is_file() || fs::is_link(&file) {
            return None;
        }
        let raw = read_capped(&file, 4 * 1024 * 1024)?;
        static RES: OnceLock<[Regex; 3]> = OnceLock::new();
        let [sec_re, list_re, map_re] = RES.get_or_init(|| {
            [
                Regex::new(r"(?-u)^([a-z_]+):\s*(.*)$").expect("regex"),
                Regex::new(r#"(?-u)^\s+-\s+['"]?([^'"\s]+)"#).expect("regex"),
                Regex::new(r#"(?-u)^\s+['"]?([^'":\s]+)['"]?\s*:"#).expect("regex"),
            ]
        });
        let mut out: HashMap<String, String> = HashMap::new();
        let mut section: Option<String> = None;
        let trim_q = |b: &[u8]| -> String {
            let s: &[u8] = b;
            let start = s
                .iter()
                .position(|c| !matches!(c, b' ' | b'\'' | b'"'))
                .unwrap_or(s.len());
            let end = s
                .iter()
                .rposition(|c| !matches!(c, b' ' | b'\'' | b'"'))
                .map(|p| p + 1)
                .unwrap_or(start);
            lower(&s[start..end.max(start)])
        };
        for line in split_lines(&raw) {
            if let Some(m) = sec_re.captures(line) {
                let sec = String::from_utf8_lossy(&m[1]).into_owned();
                if sec == "main_domain" {
                    let d = trim_q(&m[2]);
                    if is_valid_domain(&d) {
                        out.insert(d, "main".into());
                    }
                }
                section = Some(sec);
                continue;
            }
            let Some(sec) = &section else {
                continue;
            };
            let item = list_re
                .captures(line)
                .or_else(|| map_re.captures(line))
                .map(|m| lower(&m[1]));
            let Some(item) = item.filter(|i| is_valid_domain(i)) else {
                continue;
            };
            let kind = match sec.as_str() {
                "addon_domains" => "addon",
                "parked_domains" => "parked",
                "sub_domains" => "sub",
                _ => continue,
            };
            out.entry(item).or_insert_with(|| kind.to_string());
        }
        Some(out)
    }
}

fn dns_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?-u)^DNS[0-9]+$").expect("regex"))
}

/// A candidate live log of a domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    /// Path as discovered.
    pub path: PathBuf,
    /// `-ssl_log` file.
    pub https: bool,
    /// Found in a directory shared by every account.
    pub shared: bool,
}

/// A compressed archive of a domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    /// Path of the `.gz` file.
    pub path: PathBuf,
    /// `-ssl_log` archive.
    pub https: bool,
    /// Size in bytes at discovery.
    pub size: i64,
    /// Modification time at discovery.
    pub mtime: i64,
    /// Standalone: the live log this file was rotated from (archives of one
    /// stream are imported in time order, never twice even when logrotate
    /// renames them, see `Collector::backfill_domain`); `None` for cPanel.
    pub stream: Option<PathBuf>,
}

/// File-name patterns that are never access logs (fnmatch syntax, `*` only).
pub const IGNORE: &[&str] = &[
    "*-bytes_log",
    "*-ftp_log",
    "ftpxferlog*",
    "*.bkup*",
    "*.offset*",
    "*.gz",
    "*.bz2",
    "*.zip",
    "*~",
    ".*",
];

/// `fnmatch()` for patterns made of literals and `*`.
fn glob_match(pat: &[u8], s: &[u8]) -> bool {
    match pat.split_first() {
        None => s.is_empty(),
        Some((b'*', rest)) => (0..=s.len()).any(|i| glob_match(rest, &s[i..])),
        Some((c, rest)) => s.first() == Some(c) && glob_match(rest, &s[1..]),
    }
}

/// Finds the access logs of a domain and the compressed archives of an account.
#[derive(Debug, Clone)]
pub struct LogFiles {
    paths: Paths,
}

impl LogFiles {
    /// Log discovery over the configured paths.
    pub fn new(paths: &Paths) -> LogFiles {
        LogFiles {
            paths: paths.clone(),
        }
    }

    /// True when a file name must be skipped.
    pub fn is_ignored(name: &str) -> bool {
        IGNORE
            .iter()
            .any(|p| glob_match(p.as_bytes(), name.as_bytes()))
    }

    /// Candidate live logs of a domain: `<domain>` and `<domain>-ssl_log` in every
    /// `domlogs_dirs` entry; the same physical file reached twice is returned once.
    pub fn for_domain(&self, user: &str, domain: &str) -> Vec<LogFile> {
        if !is_valid_domain(domain) || !is_valid_user(user) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (dir, shared) in self.paths.domlogs_dir_map(user) {
            for (name, https) in [
                (domain.to_string(), false),
                (format!("{domain}-ssl_log"), true),
            ] {
                let file = dir.join(&name);
                if !file.is_file() {
                    continue;
                }
                let id = match std::fs::metadata(&file) {
                    Ok(st) => format!("{}:{}", st.dev(), st.ino()),
                    Err(_) => std::fs::canonicalize(&file)
                        .unwrap_or_else(|_| file.clone())
                        .display()
                        .to_string(),
                };
                if !seen.insert(id) {
                    continue;
                }
                out.push(LogFile {
                    path: file,
                    https,
                    shared,
                });
            }
        }
        out
    }

    /// Domains that have a log file in the per-user domlog directories (templates
    /// with `{user}`) — catches domains missing from `/etc/userdomains`.
    pub fn domains_in_user_dirs(&self, user: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if !is_valid_user(user) {
            return out;
        }
        for tpl in self.paths.domlogs_templates() {
            if !tpl.contains("{user}") {
                continue;
            }
            let dir = PathBuf::from(tpl.replace("{user}", user).trim_end_matches('/'));
            for name in fs::scandir(&dir) {
                if LogFiles::is_ignored(&name) || !dir.join(&name).is_file() {
                    continue;
                }
                let d = name
                    .strip_suffix("-ssl_log")
                    .unwrap_or(&name)
                    .to_ascii_lowercase();
                if is_valid_domain(&d) && d.contains('.') && !out.contains(&d) {
                    out.push(d);
                }
            }
        }
        out.sort();
        out
    }

    /// Compressed archives of the account (`<domain>[-ssl_log]-<Mon>-<YYYY>.gz` or any
    /// `*.gz` starting with `<domain>-`), assigned to the longest matching domain.
    /// Symlinks, non-regular files and (as root) files owned by another account are
    /// skipped: the archive directory is user-writable.
    pub fn archives(
        &self,
        user: &str,
        domains: &[String],
        dir: Option<&Path>,
    ) -> BTreeMap<String, Vec<Archive>> {
        let mut out: BTreeMap<String, Vec<Archive>> = BTreeMap::new();
        let dir = match dir {
            Some(d) => PathBuf::from(d.to_string_lossy().trim_end_matches('/')),
            None => {
                if !is_valid_user(user) {
                    return out;
                }
                self.paths.archive_dir(user).0
            }
        };
        if dir.as_os_str().is_empty() || fs::is_link(&dir) || !dir.is_dir() {
            return out;
        }
        let root = fs::is_root();
        let uid = if root { fs::user_id(user) } else { None };
        let mut doms: Vec<&String> = domains.iter().collect();
        doms.sort_by_key(|d| std::cmp::Reverse(d.len()));
        for name in fs::scandir(&dir) {
            if !name.ends_with(".gz")
                || name.starts_with('.')
                || name.contains("bytes_log")
                || name.contains("ftp_log")
            {
                continue;
            }
            let file = dir.join(&name);
            let Ok(st) = std::fs::symlink_metadata(&file) else {
                continue;
            };
            if !st.file_type().is_file() {
                continue;
            }
            if root && st.uid() != 0 && uid.is_some() && Some(st.uid()) != uid {
                continue;
            }
            if root && st.uid() == 0 && st.nlink() > 1 {
                continue;
            }
            for d in &doms {
                if let Some(rest) = name.strip_prefix(&format!("{d}-")) {
                    out.entry((*d).clone()).or_default().push(Archive {
                        path: file.clone(),
                        https: rest.starts_with("ssl_log"),
                        size: st.size() as i64,
                        mtime: st.mtime(),
                        stream: None,
                    });
                    break;
                }
            }
        }
        for list in out.values_mut() {
            list.sort_by(|a, b| {
                (a.mtime, a.path.as_os_str().as_encoded_bytes())
                    .cmp(&(b.mtime, b.path.as_os_str().as_encoded_bytes()))
            });
        }
        out
    }
}

/// Where accounts and logs come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// cPanel & WHM: `/var/cpanel/users`, `/etc/userdomains`, domlogs.
    Cpanel,
    /// Any other server: config `sites`, Apache and nginx configurations.
    Standalone,
}

impl Mode {
    /// `cpanel` or `standalone`.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Cpanel => "cpanel",
            Mode::Standalone => "standalone",
        }
    }
}

/// The effective mode: `mode` from config.json when it is `cpanel` or
/// `standalone`; `auto` (the default) means cPanel when `paths.cpanel_marker`
/// (`/usr/local/cpanel/cpanel`) exists or `paths.cpanel_users_dir` is a
/// directory, standalone otherwise.
pub fn resolve_mode(config: &crate::config::Config, paths: &Paths) -> Mode {
    match config.get_str("mode").as_deref() {
        Some("cpanel") => Mode::Cpanel,
        Some("standalone") => Mode::Standalone,
        _ => {
            let marker = paths.cpanel_marker();
            if (!marker.as_os_str().is_empty() && marker.exists())
                || paths.cpanel_users_dir().is_dir()
            {
                Mode::Cpanel
            } else {
                Mode::Standalone
            }
        }
    }
}

/// Accounts, domains and log files of the server.
pub trait Provider: Send + Sync + std::fmt::Debug {
    /// The mode this provider implements.
    fn mode(&self) -> Mode;
    /// Every account, sorted by name.
    fn all(&self) -> BTreeMap<String, UserInfo>;
    /// One account.
    fn get(&self, user: &str) -> Option<UserInfo>;
    /// True when the account exists.
    fn exists(&self, user: &str) -> bool {
        self.get(user).is_some()
    }
    /// Domains of an account with their kind.
    fn domains_of(&self, user: &str) -> BTreeMap<String, String>;
    /// Reseller owning the account (cPanel `OWNER=`).
    fn owner(&self, user: &str) -> Option<String>;
    /// Domains with logs but unknown to the account list (cPanel per-user domlogs).
    fn extra_domains(&self, user: &str) -> Vec<String>;
    /// Live logs of a domain (existing files only, each physical file once).
    fn live_logs(&self, user: &str, domain: &str) -> Vec<LogFile>;
    /// Archives for backfill, by domain (`dir`: `backfill --archive-dir`).
    fn archives(
        &self,
        user: &str,
        domains: &[String],
        dir: Option<&Path>,
    ) -> BTreeMap<String, Vec<Archive>>;
    /// True when archives come from root-managed configuration (no per-account
    /// ownership checks when opening them).
    fn archives_trusted(&self) -> bool;
    /// The standalone discovery result (`None` on cPanel).
    fn standalone(&self) -> Option<&crate::engine::standalone::Discovery> {
        None
    }
}

/// The provider for the configuration: [`CpanelProvider`] or [`StandaloneProvider`].
pub fn provider(config: &crate::config::Config, paths: &Paths) -> Box<dyn Provider> {
    match resolve_mode(config, paths) {
        Mode::Cpanel => Box::new(CpanelProvider::new(paths)),
        Mode::Standalone => Box::new(StandaloneProvider::new(config, paths)),
    }
}

/// cPanel discovery: [`CpanelUsers`] + [`LogFiles`] (behaviour of 0.5.0).
#[derive(Debug)]
pub struct CpanelProvider {
    users: CpanelUsers,
    logs: LogFiles,
}

impl CpanelProvider {
    /// Discovery over the configured cPanel paths.
    pub fn new(paths: &Paths) -> CpanelProvider {
        CpanelProvider {
            users: CpanelUsers::new(paths),
            logs: LogFiles::new(paths),
        }
    }
    /// The account reader.
    pub fn users(&self) -> &CpanelUsers {
        &self.users
    }
}

impl Provider for CpanelProvider {
    fn mode(&self) -> Mode {
        Mode::Cpanel
    }
    fn all(&self) -> BTreeMap<String, UserInfo> {
        self.users.all()
    }
    fn get(&self, user: &str) -> Option<UserInfo> {
        self.users.get(user)
    }
    fn exists(&self, user: &str) -> bool {
        self.users.exists(user)
    }
    fn domains_of(&self, user: &str) -> BTreeMap<String, String> {
        self.users.domains_of(user)
    }
    fn owner(&self, user: &str) -> Option<String> {
        self.users.owner(user)
    }
    fn extra_domains(&self, user: &str) -> Vec<String> {
        self.logs.domains_in_user_dirs(user)
    }
    fn live_logs(&self, user: &str, domain: &str) -> Vec<LogFile> {
        self.logs.for_domain(user, domain)
    }
    fn archives(
        &self,
        user: &str,
        domains: &[String],
        dir: Option<&Path>,
    ) -> BTreeMap<String, Vec<Archive>> {
        self.logs.archives(user, domains, dir)
    }
    fn archives_trusted(&self) -> bool {
        false
    }
}

/// Standalone discovery (see `engine::standalone`), computed once.
#[derive(Debug)]
pub struct StandaloneProvider {
    paths: Paths,
    disc: crate::engine::standalone::Discovery,
}

impl StandaloneProvider {
    /// Runs discovery now.
    pub fn new(config: &crate::config::Config, paths: &Paths) -> StandaloneProvider {
        StandaloneProvider {
            paths: paths.clone(),
            disc: crate::engine::standalone::discover(config, paths),
        }
    }

    fn info(&self, user: &str, domains: Vec<String>) -> UserInfo {
        let kinds: BTreeMap<String, String> = domains
            .iter()
            .map(|d| (d.clone(), "site".to_string()))
            .collect();
        UserInfo {
            owner: None,
            domains: kinds.keys().cloned().collect(),
            kinds,
            main: None,
            home: fs::passwd(user).map(|p| p.2).unwrap_or_default(),
        }
    }
}

impl Provider for StandaloneProvider {
    fn mode(&self) -> Mode {
        Mode::Standalone
    }
    fn all(&self) -> BTreeMap<String, UserInfo> {
        self.disc
            .owners()
            .into_iter()
            .filter(|(u, _)| is_valid_user(u))
            .map(|(u, d)| {
                let i = self.info(&u, d);
                (u, i)
            })
            .collect()
    }
    fn get(&self, user: &str) -> Option<UserInfo> {
        let d = self.disc.owners().remove(user)?;
        Some(self.info(user, d))
    }
    fn domains_of(&self, user: &str) -> BTreeMap<String, String> {
        self.get(user).map(|i| i.kinds).unwrap_or_default()
    }
    fn owner(&self, _user: &str) -> Option<String> {
        None
    }
    fn extra_domains(&self, _user: &str) -> Vec<String> {
        Vec::new()
    }
    fn live_logs(&self, user: &str, domain: &str) -> Vec<LogFile> {
        let Some(site) = self.disc.site(user, domain) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut seen: HashSet<(u64, u64)> = HashSet::new();
        for l in site.logs.iter().filter(|l| l.kind.usable()) {
            let Ok(st) = std::fs::metadata(&l.path) else {
                continue;
            };
            if !st.is_file() || !seen.insert((st.dev(), st.ino())) {
                continue;
            }
            out.push(LogFile {
                path: l.path.clone(),
                https: l.https,
                shared: false,
            });
        }
        out
    }
    fn archives(
        &self,
        user: &str,
        domains: &[String],
        dir: Option<&Path>,
    ) -> BTreeMap<String, Vec<Archive>> {
        if dir.is_some() {
            return LogFiles::new(&self.paths).archives(user, domains, dir);
        }
        let live: Vec<PathBuf> = self
            .disc
            .sites
            .iter()
            .chain(self.disc.ignored.iter())
            .flat_map(|s| s.logs.iter().map(|l| l.path.clone()))
            .collect();
        let mut out: BTreeMap<String, Vec<Archive>> = BTreeMap::new();
        for site in self.disc.sites_of(user) {
            if !domains.contains(&site.domain) {
                continue;
            }
            for (path, https, stream) in crate::engine::standalone::site_archives(site, &live) {
                let Ok(st) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                out.entry(site.domain.clone()).or_default().push(Archive {
                    path,
                    https,
                    size: st.size() as i64,
                    mtime: st.mtime(),
                    stream: Some(stream),
                });
            }
        }
        for list in out.values_mut() {
            list.sort_by(|a, b| {
                (a.mtime, a.path.as_os_str().as_encoded_bytes())
                    .cmp(&(b.mtime, b.path.as_os_str().as_encoded_bytes()))
            });
        }
        out
    }
    fn archives_trusted(&self) -> bool {
        true
    }
    fn standalone(&self) -> Option<&crate::engine::standalone::Discovery> {
        Some(&self.disc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ignore_patterns() {
        for n in [
            "example.com-bytes_log",
            "x-ftp_log",
            "ftpxferlog.1",
            "a.bkup2",
            "a.offset",
            "a.gz",
            ".hidden",
            "x~",
        ] {
            assert!(LogFiles::is_ignored(n), "{n}");
        }
        assert!(!LogFiles::is_ignored("example.com-ssl_log"));
        assert!(!LogFiles::is_ignored("example.com"));
    }
}
