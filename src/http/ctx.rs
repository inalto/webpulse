//! Per-request context of the HTTP layer: the effective configuration
//! (file + registry `settings` overrides, as PHP's `App` wires them), paths,
//! logger, a lazily opened registry and the clock.
//!
//! Things the shared `App` does not provide yet are implemented here and
//! listed in the R2 report so R1 can lift them: registry overrides applied to
//! the configuration, the registry handle, read-only domain opening,
//! cPanel owner/domain lookups.

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;

use serde_json::Value;

use super::registry::Registry;
use crate::app::App;
use crate::config::Config;
use crate::error::Result;
use crate::paths::Paths;
use crate::reports::domain_data::{self, DomainData, php_default_tz};

/// Context of one API request / page render.
pub struct Ctx<'a> {
    /// The application (logger, global flags).
    pub app: &'a App,
    /// Paths of the configuration.
    pub paths: Paths,
    config: RefCell<Config>,
    registry: OnceCell<Registry>,
}

impl<'a> Ctx<'a> {
    /// Builds the context; registry overrides are applied when the registry
    /// file exists and can be opened (never for a cPanel account on a real
    /// server: the registry is root 0600).
    pub fn new(app: &'a App) -> Ctx<'a> {
        let ctx = Ctx {
            app,
            paths: app.paths.clone(),
            config: RefCell::new(app.config.clone()),
            registry: OnceCell::new(),
        };
        ctx.reload_overrides();
        ctx
    }

    /// Re-reads the registry `settings` overrides into the configuration.
    pub fn reload_overrides(&self) {
        let mut cfg = self.app.config.clone();
        if self.paths.registry_db().is_file()
            && let Ok(reg) = self.registry()
            && let Ok(s) = reg.settings()
        {
            cfg.apply_overrides(s.into_iter().collect::<BTreeMap<String, Value>>());
        }
        *self.config.borrow_mut() = cfg;
    }

    /// A configuration value (dotted key), `Null` when absent.
    pub fn cfg(&self, key: &str) -> Value {
        self.config.borrow().get(key).clone()
    }

    /// Integer setting with default.
    pub fn cfg_i64(&self, key: &str, default: i64) -> i64 {
        self.config.borrow().get_i64(key, default)
    }

    /// Boolean setting with PHP's `(bool)` cast.
    pub fn cfg_bool(&self, key: &str, default: bool) -> bool {
        match self.cfg(key) {
            Value::Null => default,
            Value::Bool(b) => b,
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            Value::String(s) => !(s.is_empty() || s == "0"),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
        }
    }

    /// Keys overridden in the registry.
    pub fn overridden_keys(&self) -> Vec<String> {
        self.config.borrow().overrides().keys().cloned().collect()
    }

    /// Path of the configuration file in use, if any.
    pub fn config_path(&self) -> Option<std::path::PathBuf> {
        self.app.config_path().map(|p| p.to_path_buf())
    }

    /// The registry, opened (and created/migrated when writable) on first use.
    pub fn registry(&self) -> Result<&Registry> {
        if let Some(r) = self.registry.get() {
            return Ok(r);
        }
        let def = self
            .config
            .try_borrow()
            .map(|c| c.get_bool("default_enabled", true))
            .unwrap_or(true);
        let r = Registry::open(self.paths.registry_db(), def, App::version(), self.now())?;
        Ok(self.registry.get_or_init(|| r))
    }

    /// Current unix time (`WEBPULSE_NOW` aware).
    pub fn now(&self) -> i64 {
        crate::util::time::now_ts()
    }

    /// `App::serverTimezone()`: config timezone, else PHP's default zone.
    pub fn server_timezone(&self) -> String {
        match self.cfg("timezone") {
            Value::String(s) if !s.is_empty() => s,
            _ => php_default_tz().name().to_string(),
        }
    }

    /// Opens a domain database read-only as a `DomainData`.
    pub fn domain_data(&self, user: &str, domain: &str, retention: i64) -> Result<DomainData> {
        let store = domain_data::open_read_only(&self.paths, user, domain)?;
        Ok(DomainData::new(
            store,
            domain,
            &self.server_timezone(),
            retention,
            self.now(),
        ))
    }

    /// Logs an error (never fails).
    pub fn log_error(&self, msg: &str) {
        self.app.log.error(msg);
    }
}
