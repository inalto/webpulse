//! Filesystem layout (see docs/ARCHITECTURE.md "Paths") and strict name validation
//! (`WebPulse\Paths`). User and domain names are validated before they are turned
//! into paths, so no traversal is possible.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::config::Config;

/// Account names: cPanel usernames (`^[a-z0-9_][a-z0-9_-]{0,31}\z`, no `..`).
pub fn is_valid_user(u: &str) -> bool {
    let b = u.as_bytes();
    if b.is_empty() || b.len() > 32 || u.contains("..") {
        return false;
    }
    let first_ok = b[0].is_ascii_lowercase() || b[0].is_ascii_digit() || b[0] == b'_';
    first_ok
        && b[1..]
            .iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

/// Lowercase domain names as they appear in file names:
/// `^(?=.{1,253}\z)[a-z0-9_][a-z0-9_-]{0,62}(\.[a-z0-9_][a-z0-9_-]{0,62})*\z`.
pub fn is_valid_domain(d: &str) -> bool {
    if d.is_empty() || d.len() > 253 {
        return false;
    }
    d.split('.').all(|label| {
        let b = label.as_bytes();
        !b.is_empty()
            && b.len() <= 63
            && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit() || b[0] == b'_')
            && b[1..]
                .iter()
                .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
    })
}

/// `Paths::assertUser`: the name, or the 0.1.0 error `invalid user name: "x"`.
pub fn assert_user(user: &str) -> Result<&str> {
    if is_valid_user(user) {
        Ok(user)
    } else {
        Err(anyhow!(
            "invalid user name: {}",
            crate::util::json::encode_php_default(&serde_json::Value::from(user))
        ))
    }
}

/// `Paths::assertDomain`: the name, or the 0.1.0 error `invalid domain name: "x"`.
pub fn assert_domain(domain: &str) -> Result<&str> {
    if is_valid_domain(domain) {
        Ok(domain)
    } else {
        Err(anyhow!(
            "invalid domain name: {}",
            crate::util::json::encode_php_default(&serde_json::Value::from(domain))
        ))
    }
}

fn rtrim_slash(s: &str) -> String {
    s.trim_end_matches('/').to_string()
}

/// All configured paths, resolved once from the configuration.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
    data_dir: PathBuf,
    registry_db: PathBuf,
    log_dir: PathBuf,
    run_dir: PathBuf,
    domlogs_dirs: Vec<String>,
    archive_dir: String,
    userdomains_file: PathBuf,
    cpanel_users_dir: PathBuf,
    cpanel_userdata_dir: PathBuf,
    home_root: PathBuf,
    cron_file: PathBuf,
    geoip_dir: PathBuf,
    cpanel_marker: PathBuf,
    apache_conf_dirs: Vec<PathBuf>,
    nginx_conf: PathBuf,
}

impl Paths {
    /// Builds the layout from `config.paths.*`; `root` is the installation directory
    /// (directory containing `bin/`, `etc/`), used for nothing security-relevant.
    pub fn from_config(cfg: &Config, root: PathBuf) -> Paths {
        let s = |k: &str| match cfg.get(&format!("paths.{k}")) {
            serde_json::Value::String(v) => v.clone(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        };
        let dir = |k: &str| PathBuf::from(rtrim_slash(&s(k)));
        let geoip_dir = match cfg.get("geoip.country_db").as_str() {
            Some(db) => Path::new(db)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")),
            None => PathBuf::from("."),
        };
        Paths {
            root,
            data_dir: dir("data_dir"),
            registry_db: PathBuf::from(s("registry_db")),
            log_dir: dir("log_dir"),
            run_dir: dir("run_dir"),
            domlogs_dirs: cfg
                .get("paths.domlogs_dirs")
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .filter(|d| !d.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            archive_dir: s("archive_dir"),
            userdomains_file: PathBuf::from(s("userdomains_file")),
            cpanel_users_dir: dir("cpanel_users_dir"),
            cpanel_userdata_dir: dir("cpanel_userdata_dir"),
            home_root: dir("home_root"),
            cron_file: PathBuf::from(s("cron_file")),
            geoip_dir,
            cpanel_marker: PathBuf::from(s("cpanel_marker")),
            apache_conf_dirs: cfg
                .get("paths.apache_conf_dirs")
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .filter(|d| !d.is_empty())
                        .map(|d| PathBuf::from(rtrim_slash(d)))
                        .collect()
                })
                .unwrap_or_default(),
            nginx_conf: PathBuf::from(s("nginx_conf")),
        }
    }

    /// Install directory (contains `bin/`, `etc/`).
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Directory holding one sub-directory per account (`paths.data_dir`).
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    /// Registry database file (`paths.registry_db`).
    pub fn registry_db(&self) -> &Path {
        &self.registry_db
    }
    /// Directory of webpulse.log (`paths.log_dir`).
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }
    /// Directory of lock files and hook flags (`paths.run_dir`).
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }
    /// `<run_dir>/hook` — flag files written by the Stats::RunUser hook (1777).
    pub fn hook_dir(&self) -> PathBuf {
        self.run_dir.join("hook")
    }
    /// Directory holding GeoIP databases (dirname of `geoip.country_db`).
    pub fn geoip_dir(&self) -> &Path {
        &self.geoip_dir
    }
    /// `/etc/userdomains` (`paths.userdomains_file`).
    pub fn userdomains_file(&self) -> &Path {
        &self.userdomains_file
    }
    /// `/var/cpanel/users` (`paths.cpanel_users_dir`).
    pub fn cpanel_users_dir(&self) -> &Path {
        &self.cpanel_users_dir
    }
    /// `/var/cpanel/userdata` (`paths.cpanel_userdata_dir`).
    pub fn cpanel_userdata_dir(&self) -> &Path {
        &self.cpanel_userdata_dir
    }
    /// Parent of the account home directories (`paths.home_root`).
    pub fn home_root(&self) -> &Path {
        &self.home_root
    }
    /// Cron file whose presence `status` reports (`paths.cron_file`).
    pub fn cron_file(&self) -> &Path {
        &self.cron_file
    }

    /// File whose existence means "this is a cPanel server" (`paths.cpanel_marker`,
    /// default `/usr/local/cpanel/cpanel`).
    pub fn cpanel_marker(&self) -> &Path {
        &self.cpanel_marker
    }
    /// Apache configuration roots probed in standalone mode (`paths.apache_conf_dirs`,
    /// default `/etc/httpd` (EL) and `/etc/apache2` (Debian)).
    pub fn apache_conf_dirs(&self) -> &[PathBuf] {
        &self.apache_conf_dirs
    }
    /// Main nginx configuration file probed in standalone mode (`paths.nginx_conf`).
    pub fn nginx_conf(&self) -> &Path {
        &self.nginx_conf
    }

    /// `<data_dir>/<user>`; panics on an invalid name — callers validate first
    /// (see [`Paths::try_user_dir`]).
    pub fn user_dir(&self, user: &str) -> PathBuf {
        assert!(is_valid_user(user), "invalid user name");
        self.data_dir.join(user)
    }
    /// `<data_dir>/<user>`, or the `invalid user name` error.
    pub fn try_user_dir(&self, user: &str) -> Result<PathBuf> {
        Ok(self.data_dir.join(assert_user(user)?))
    }
    /// `<data_dir>/<user>/user` — owned by the account, holds settings.json.
    pub fn user_private_dir(&self, user: &str) -> PathBuf {
        self.user_dir(user).join("user")
    }
    /// `<data_dir>/<user>/user/settings.json`.
    pub fn user_settings_file(&self, user: &str) -> PathBuf {
        self.user_private_dir(user).join("settings.json")
    }
    /// `<data_dir>/<user>/<domain>.db`; panics on invalid names — callers validate
    /// first (see [`Paths::try_domain_db`]).
    pub fn domain_db(&self, user: &str, domain: &str) -> PathBuf {
        assert!(is_valid_domain(domain), "invalid domain name");
        self.user_dir(user).join(format!("{domain}.db"))
    }
    /// `<data_dir>/<user>/<domain>.db`, or the `invalid … name` error.
    pub fn try_domain_db(&self, user: &str, domain: &str) -> Result<PathBuf> {
        let dir = self.try_user_dir(user)?;
        Ok(dir.join(format!("{}.db", assert_domain(domain)?)))
    }
    /// Domlog directories with `{user}` substituted; the second element says whether
    /// the directory is per-user (its template contained `{user}`).
    pub fn domlogs_dirs(&self, user: &str) -> Vec<(PathBuf, bool)> {
        self.domlogs_dirs
            .iter()
            .map(|d| {
                (
                    PathBuf::from(rtrim_slash(&d.replace("{user}", user))),
                    d.contains("{user}"),
                )
            })
            .collect()
    }
    /// `Paths::domlogsDirMap`: distinct directories (first occurrence order) with
    /// `true` when the directory is shared by every account (no `{user}` in any of
    /// the templates that produced it).
    pub fn domlogs_dir_map(&self, user: &str) -> Vec<(PathBuf, bool)> {
        let mut out: Vec<(PathBuf, bool)> = Vec::new();
        for d in &self.domlogs_dirs {
            let dir = PathBuf::from(rtrim_slash(&d.replace("{user}", user)));
            let shared = !d.contains("{user}");
            match out.iter_mut().find(|(p, _)| *p == dir) {
                Some(e) => e.1 = e.1 && shared,
                None => out.push((dir, shared)),
            }
        }
        out
    }
    /// The raw `paths.domlogs_dirs` templates.
    pub fn domlogs_templates(&self) -> &[String] {
        &self.domlogs_dirs
    }
    /// Archive directory for a user and whether it is shared (no `{user}` placeholder).
    pub fn archive_dir(&self, user: &str) -> (PathBuf, bool) {
        (
            PathBuf::from(rtrim_slash(&self.archive_dir.replace("{user}", user))),
            !self.archive_dir.contains("{user}"),
        )
    }
    /// True when the archive directory template has no `{user}` placeholder.
    pub fn archive_dir_shared(&self) -> bool {
        !self.archive_dir.contains("{user}")
    }
    /// `<run_dir>/<name>.lock`.
    pub fn lock_path(&self, name: &str) -> PathBuf {
        self.run_dir.join(format!("{name}.lock"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        for u in ["alice", "a1", "user_name", "r-1", "_a1-b"] {
            assert!(is_valid_user(u), "{u}");
        }
        for u in [
            "",
            "..",
            "a/b",
            "Alice",
            "a.b",
            &"a".repeat(40),
            "a\0",
            "alice\n",
        ] {
            assert!(!is_valid_user(u), "{u:?}");
        }
        for d in [
            "example.com",
            "shop.example.com",
            "xn--bcher-kva.example",
            "a-b.co.uk",
            "localhost",
            "a_b.com",
            "a-.com",
        ] {
            assert!(is_valid_domain(d), "{d}");
        }
        for d in [
            "",
            "..",
            "../x",
            "a/b",
            "Example.com",
            "-a.com",
            "a..b",
            "a.com/",
            "*.example.com",
            ".x",
            "a.com.",
        ] {
            assert!(!is_valid_domain(d), "{d}");
        }
        assert_eq!(
            assert_user("../x").unwrap_err().to_string(),
            "invalid user name: \"..\\/x\""
        );
    }
}
