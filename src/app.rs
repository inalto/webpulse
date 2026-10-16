//! Process-wide context: configuration, paths, logger. Built once in `main`.
//! R1 adds the lazy `Registry`, `CpanelUsers` and `GeoIp` accessors here.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::log::{Level, Log};
use crate::paths::Paths;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/webpulse/config.json";

/// Global command-line flags shared by every subcommand.
#[derive(Debug, Clone, Default)]
pub struct Global {
    pub config: Option<PathBuf>,
    pub verbose: bool,
    pub quiet: bool,
    pub json: bool,
}

pub struct App {
    pub config: Config,
    pub paths: Paths,
    pub log: Log,
    pub global: Global,
}

impl App {
    /// Boots from `--config`, else `WEBPULSE_CONFIG`, else `/etc/webpulse/config.json`.
    /// Never fails because of an unreadable config (defaults apply; see `Config::load_error`).
    pub fn boot(global: Global) -> App {
        let config_path = global
            .config
            .clone()
            .or_else(|| std::env::var_os("WEBPULSE_CONFIG").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));
        let config = Config::load(Some(&config_path));
        let paths = Paths::from_config(&config, install_root());
        let level = if global.verbose { Level::Debug } else { Level::Info };
        let log_file = paths.log_dir().join("webpulse.log");
        let log = Log::new(Some(log_file), level, global.verbose);
        if let Some(e) = config.load_error() {
            log.debug(format!("config: {e} (using defaults)"));
        }
        App { config, paths, log, global }
    }

    pub fn version() -> &'static str { env!("CARGO_PKG_VERSION") }
    pub fn config_path(&self) -> Option<&Path> { self.config.path() }
    pub fn now(&self) -> i64 { crate::util::time::now_ts() }
    pub fn server_tz(&self) -> chrono_tz::Tz { crate::util::time::tz_or_server(self.config.get_str("timezone").as_deref()) }
    pub fn is_root(&self) -> bool { unsafe { libc::geteuid() == 0 } }
}

/// Installation root: the directory containing `bin/` when running from an install
/// (`/usr/local/webpulse/bin/webpulse` → `/usr/local/webpulse`), else the cwd.
pub fn install_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin) = exe.parent() {
            if bin.file_name().map(|n| n == "bin").unwrap_or(false) {
                if let Some(root) = bin.parent() {
                    return root.to_path_buf();
                }
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
