//! Process-wide context (`WebPulse\App`): configuration, paths, logger and lazily
//! opened services (registry, cPanel discovery, GeoIP). Built once in `main`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Result, anyhow};

use crate::config::Config;
use crate::engine::classify::geoip::GeoIp;
use crate::engine::discovery::CpanelUsers;
use crate::engine::registry::Registry;
use crate::engine::store::DomainStore;
use crate::log::{Level, Log};
use crate::paths::Paths;

/// Default configuration file.
pub const DEFAULT_CONFIG_PATH: &str = "/etc/webpulse/config.json";

/// Global command-line flags shared by every subcommand.
#[derive(Debug, Clone, Default)]
pub struct Global {
    /// `--config FILE`
    pub config: Option<PathBuf>,
    /// `--verbose` / `-v`
    pub verbose: bool,
    /// `--quiet` / `-q`
    pub quiet: bool,
    /// `--json`
    pub json: bool,
}

/// The application context.
pub struct App {
    /// Effective configuration (defaults < file < registry overrides read at boot).
    pub config: Config,
    /// Filesystem layout.
    pub paths: Paths,
    /// Logger.
    pub log: Log,
    /// Global flags.
    pub global: Global,
    config_path: PathBuf,
    registry: OnceLock<Registry>,
    cpanel_users: OnceLock<CpanelUsers>,
    geoip: OnceLock<GeoIp>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("config_path", &self.config_path)
            .finish()
    }
}

/// Configuration file in use: `--config`, else `$WEBPULSE_CONFIG`, else the default;
/// the flag says whether it was chosen explicitly.
pub fn config_path_for(global: &Global) -> (PathBuf, bool) {
    if let Some(p) = &global.config {
        return (p.clone(), true);
    }
    match std::env::var_os("WEBPULSE_CONFIG") {
        Some(v) if !v.is_empty() => (PathBuf::from(v), true),
        _ => (PathBuf::from(DEFAULT_CONFIG_PATH), false),
    }
}

/// True when `path` is the production configuration file `/etc/webpulse/config.json`,
/// compared after resolving symlinks and `..` on both sides (a path that cannot be
/// resolved is compared as given).
pub fn is_production_config(path: &Path) -> bool {
    let prod = Path::new(DEFAULT_CONFIG_PATH);
    path == prod || resolve_path(path) == resolve_path(prod)
}

/// Resolves symlinks and `.`/`..` like `realpath -m`: a dangling symlink is followed
/// through its target (up to 40 hops), then the longest existing prefix is
/// canonicalized and the missing remainder is appended lexically.
fn resolve_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut p = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    for _ in 0..40 {
        if let Ok(c) = std::fs::canonicalize(&p) {
            return c;
        }
        match std::fs::read_link(&p) {
            Ok(target) => p = p.parent().map(|d| d.join(&target)).unwrap_or(target),
            Err(_) => break,
        }
    }
    let comps: Vec<Component<'_>> = p.components().collect();
    for i in (1..=comps.len()).rev() {
        let prefix: PathBuf = comps[..i].iter().collect();
        if let Ok(mut r) = std::fs::canonicalize(&prefix) {
            for c in &comps[i..] {
                match c {
                    Component::ParentDir => {
                        r.pop();
                    }
                    Component::CurDir => {}
                    other => r.push(other.as_os_str()),
                }
            }
            return r;
        }
    }
    p
}

impl App {
    /// Boots from `--config`, else `WEBPULSE_CONFIG`, else `/etc/webpulse/config.json`.
    /// Never fails because of the config file (defaults apply; see `Config::load_error`).
    pub fn boot(global: Global) -> App {
        let (path, _) = config_path_for(&global);
        let config = Config::load(Some(&path));
        App::from_config(global, path, config)
    }

    /// Boots like [`App::boot`] but fails like 0.1.0 when an explicitly given file
    /// (`--config` / `$WEBPULSE_CONFIG`) is missing or the file is not valid JSON.
    pub fn try_boot(global: Global) -> Result<App> {
        let (path, explicit) = config_path_for(&global);
        let config = Config::try_load(Some(&path))?;
        if explicit && config.load_error() == Some("missing") {
            return Err(anyhow!("config file not found: {}", path.display()));
        }
        Ok(App::from_config(global, path, config))
    }

    fn from_config(global: Global, path: PathBuf, mut config: Config) -> App {
        let paths = Paths::from_config(&config, install_root());
        // registry overrides (silently skipped when the registry is not readable,
        // e.g. in the cPanel plugin running as the account)
        if let Ok(o) = Registry::read_settings(paths.registry_db())
            && !o.is_empty()
        {
            config.apply_overrides(o);
        }
        let log = Log::new(
            Some(paths.log_dir().join("webpulse.log")),
            if global.verbose {
                Level::Debug
            } else {
                Level::Info
            },
            global.verbose,
        );
        log.set_verbose(global.verbose);
        if let Some(e) = config.load_error() {
            log.debug(format!("config: {e} (using defaults)"));
        }
        // WEBPULSE_NOW (test clock) is ignored for root on the production configuration
        let root = crate::util::fs::is_root();
        let allow_now = crate::util::time::fake_clock_policy(root, is_production_config(&path));
        crate::util::time::set_fake_clock_allowed(allow_now);
        if root && allow_now && std::env::var_os("WEBPULSE_NOW").is_some_and(|v| !v.is_empty()) {
            log.warn(format!(
                "WEBPULSE_NOW is set: running as root with a frozen clock (config {})",
                path.display()
            ));
        }
        App {
            config,
            paths,
            log,
            global,
            config_path: path,
            registry: OnceLock::new(),
            cpanel_users: OnceLock::new(),
            geoip: OnceLock::new(),
        }
    }

    /// Version of this binary.
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// Path of the configuration file in use (it may not exist).
    pub fn config_path(&self) -> Option<&Path> {
        Some(&self.config_path)
    }

    /// Current unix time (`WEBPULSE_NOW` pins it).
    pub fn now(&self) -> i64 {
        crate::util::time::now_ts()
    }

    /// Server timezone (config `timezone`, else the system zone).
    pub fn server_tz(&self) -> chrono_tz::Tz {
        crate::util::time::tz_or_server(
            self.config
                .get_str("timezone")
                .as_deref()
                .filter(|s| !s.is_empty()),
        )
    }

    /// Name of the server timezone (`App::serverTimezone`).
    pub fn server_timezone(&self) -> String {
        match self.config.get_str("timezone") {
            Some(t) if !t.is_empty() => t,
            _ => crate::util::time::server_tz().name().to_string(),
        }
    }

    /// True when running as root.
    pub fn is_root(&self) -> bool {
        crate::util::fs::is_root()
    }

    /// The registry, opened (created, migrated) on first use.
    pub fn registry(&self) -> Result<&Registry> {
        if let Some(r) = self.registry.get() {
            return Ok(r);
        }
        let users_dir = self.paths.cpanel_users_dir().to_path_buf();
        let lookup: crate::engine::registry::OwnerLookup = Box::new(move |user: &str| {
            if !crate::paths::is_valid_user(user) {
                return None;
            }
            let raw = std::fs::read(users_dir.join(user)).ok()?;
            let mut owner = None;
            for line in raw.split(|&b| b == b'\n') {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                if line.is_empty() || line[0] == b'#' {
                    continue;
                }
                if let Some(p) = line.iter().position(|&b| b == b'=')
                    && crate::util::strs::php_trim(&line[..p]) == b"OWNER"
                {
                    owner = Some(
                        String::from_utf8_lossy(crate::util::strs::php_trim(&line[p + 1..]))
                            .into_owned(),
                    );
                }
            }
            owner.filter(|o| !o.is_empty())
        });
        let reg = Registry::open(
            self.paths.registry_db(),
            &self.config,
            Some(App::version()),
            Some(lookup),
        )?;
        let _ = self.registry.set(reg);
        self.registry
            .get()
            .ok_or_else(|| anyhow!("registry unavailable"))
    }

    /// cPanel account discovery (lazy).
    pub fn cpanel_users(&self) -> &CpanelUsers {
        self.cpanel_users
            .get_or_init(|| CpanelUsers::new(&self.paths))
    }

    /// GeoIP lookups; an instance without databases when `geoip.enabled` is false.
    pub fn geoip(&self) -> &GeoIp {
        self.geoip.get_or_init(|| {
            if !crate::config::php_bool(self.config.get("geoip.enabled")) {
                return GeoIp::none();
            }
            let country = self.config.get_str("geoip.country_db").map(PathBuf::from);
            let city = self.config.get_str("geoip.city_db").map(PathBuf::from);
            GeoIp::new(country.as_deref(), city.as_deref(), Some(&self.log))
        })
    }

    /// Opens a domain database (read-only: [`crate::error::WpError::NotFound`] when missing).
    pub fn open_domain(&self, user: &str, domain: &str, read_only: bool) -> Result<DomainStore> {
        DomainStore::open(&self.paths, user, domain, read_only)
    }
}

/// Installation root: the directory containing `bin/` when running from an install
/// (`/usr/local/webpulse/bin/webpulse` → `/usr/local/webpulse`), else the cwd.
pub fn install_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
        && bin.file_name().map(|n| n == "bin").unwrap_or(false)
        && let Some(root) = bin.parent()
    {
        return root.to_path_buf();
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
