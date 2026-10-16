//! Filesystem layout (see docs/ARCHITECTURE.md "Paths") and strict name validation.

use std::path::{Path, PathBuf};

use crate::config::Config;

/// Account names: cPanel usernames (`^[a-z0-9_][a-z0-9_-]{0,31}\z`).
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

/// Domain names as they appear in file names (`^[a-z0-9.-]{1,253}\z`, no `..`, no leading `.`/`-`).
pub fn is_valid_domain(d: &str) -> bool {
    let b = d.as_bytes();
    if b.is_empty() || b.len() > 253 || d.contains("..") || b[0] == b'.' || b[0] == b'-' {
        return false;
    }
    b.iter()
        .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
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
}

impl Paths {
    /// Builds the layout from `config.paths.*`; `root` is the installation directory
    /// (directory containing `bin/`, `ui/`), used for nothing security-relevant.
    pub fn from_config(cfg: &Config, root: PathBuf) -> Paths {
        let p = |k: &str, d: &str| PathBuf::from(cfg.get_str(&format!("paths.{k}")).unwrap_or_else(|| d.to_string()));
        let data_dir = p("data_dir", "/var/webpulse/data");
        Paths {
            root,
            registry_db: p("registry_db", "/var/webpulse/webpulse.db"),
            log_dir: p("log_dir", "/var/webpulse/log"),
            run_dir: p("run_dir", "/var/webpulse/run"),
            domlogs_dirs: cfg
                .get_array("paths.domlogs_dirs")
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            archive_dir: cfg.get_str("paths.archive_dir").unwrap_or_else(|| "/home/{user}/logs".into()),
            userdomains_file: p("userdomains_file", "/etc/userdomains"),
            cpanel_users_dir: p("cpanel_users_dir", "/var/cpanel/users"),
            cpanel_userdata_dir: p("cpanel_userdata_dir", "/var/cpanel/userdata"),
            home_root: p("home_root", "/home"),
            cron_file: p("cron_file", "/etc/cron.d/webpulse"),
            geoip_dir: cfg
                .get_str("geoip.country_db")
                .map(|s| PathBuf::from(s).parent().map(Path::to_path_buf).unwrap_or_else(|| "/var/webpulse/geoip".into()))
                .unwrap_or_else(|| "/var/webpulse/geoip".into()),
            data_dir,
        }
    }

    pub fn root(&self) -> &Path { &self.root }
    pub fn data_dir(&self) -> &Path { &self.data_dir }
    pub fn registry_db(&self) -> &Path { &self.registry_db }
    pub fn log_dir(&self) -> &Path { &self.log_dir }
    pub fn run_dir(&self) -> &Path { &self.run_dir }
    pub fn hook_dir(&self) -> PathBuf { self.run_dir.join("hook") }
    pub fn geoip_dir(&self) -> &Path { &self.geoip_dir }
    pub fn userdomains_file(&self) -> &Path { &self.userdomains_file }
    pub fn cpanel_users_dir(&self) -> &Path { &self.cpanel_users_dir }
    pub fn cpanel_userdata_dir(&self) -> &Path { &self.cpanel_userdata_dir }
    pub fn home_root(&self) -> &Path { &self.home_root }
    pub fn cron_file(&self) -> &Path { &self.cron_file }

    /// `<data_dir>/<user>`; panics on an invalid name — callers validate first.
    pub fn user_dir(&self, user: &str) -> PathBuf {
        assert!(is_valid_user(user), "invalid user name");
        self.data_dir.join(user)
    }
    /// `<data_dir>/<user>/user` — owned by the account, holds settings.json.
    pub fn user_private_dir(&self, user: &str) -> PathBuf { self.user_dir(user).join("user") }
    pub fn user_settings_file(&self, user: &str) -> PathBuf { self.user_private_dir(user).join("settings.json") }
    /// `<data_dir>/<user>/<domain>.db`; panics on invalid names — callers validate first.
    pub fn domain_db(&self, user: &str, domain: &str) -> PathBuf {
        assert!(is_valid_domain(domain), "invalid domain name");
        self.user_dir(user).join(format!("{domain}.db"))
    }
    /// Domlog directories with `{user}` substituted; the second element says whether the
    /// directory is per-user (contained `{user}`) or shared.
    pub fn domlogs_dirs(&self, user: &str) -> Vec<(PathBuf, bool)> {
        self.domlogs_dirs
            .iter()
            .map(|d| (PathBuf::from(d.replace("{user}", user)), d.contains("{user}")))
            .collect()
    }
    /// Archive directory for a user and whether it is shared (no `{user}` placeholder).
    pub fn archive_dir(&self, user: &str) -> (PathBuf, bool) {
        (PathBuf::from(self.archive_dir.replace("{user}", user)), !self.archive_dir.contains("{user}"))
    }
    pub fn lock_path(&self, name: &str) -> PathBuf { self.run_dir.join(format!("{name}.lock")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        assert!(is_valid_user("alice"));
        assert!(is_valid_user("_a1-b"));
        assert!(!is_valid_user("Alice"));
        assert!(!is_valid_user("a.b"));
        assert!(!is_valid_user("alice\n"));
        assert!(!is_valid_user(""));
        assert!(is_valid_domain("shop.example.com"));
        assert!(!is_valid_domain("../x"));
        assert!(!is_valid_domain(".x"));
        assert!(!is_valid_domain("Example.com"));
        assert!(!is_valid_domain("a/b"));
    }
}
