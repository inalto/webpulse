//! Layered configuration: built-in defaults < `/etc/webpulse/config.json` < registry
//! `settings` overrides (applied by `App` once the registry is readable).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, Value};

/// Built-in defaults; `etc/config.json.default` is generated from this at `make dist`
/// and must stay identical (`tests/config_default.rs` checks it).
pub const DEFAULTS_JSON: &str = r#"{
  "version": 1,
  "default_enabled": true,
  "timezone": null,
  "raw_retention_days": 30,
  "max_rows_per_dim": 1000,
  "max_open_visits": 50000,
  "session_timeout_min": 30,
  "store_ip": false,
  "exclude_ips": [],
  "exclude_paths": ["/.well-known/", "/cpanel", "/whm", "/webmail", "/cgi-sys/"],
  "static_prefixes": ["/wp-content/uploads/", "/wp-includes/", "/wp-json/", "/wp-admin/admin-ajax.php", "/wp-cron.php", "/xmlrpc.php"],
  "count_head_requests": false,
  "geoip": {
    "enabled": false,
    "country_db": "/var/webpulse/geoip/dbip-country-lite.mmdb",
    "city_db": null,
    "url": "https://download.db-ip.com/free/dbip-country-lite-{YYYY}-{MM}.mmdb.gz"
  },
  "php_bin": null,
  "paths": {
    "data_dir": "/var/webpulse/data",
    "registry_db": "/var/webpulse/webpulse.db",
    "log_dir": "/var/webpulse/log",
    "run_dir": "/var/webpulse/run",
    "domlogs_dirs": ["/etc/apache2/logs/domlogs/{user}", "/etc/apache2/logs/domlogs", "/usr/local/apache/domlogs/{user}", "/var/log/nginx/domains"],
    "archive_dir": "/home/{user}/logs",
    "userdomains_file": "/etc/userdomains",
    "cpanel_users_dir": "/var/cpanel/users",
    "cpanel_userdata_dir": "/var/cpanel/userdata",
    "home_root": "/home",
    "cron_file": "/etc/cron.d/webpulse",
    "hooks_file": "/var/cpanel/hooks/data/hooks.json"
  }
}"#;

/// Keys the WHM UI / `webpulse config set` may override at run time (stored in the
/// registry `settings` table). Everything else comes from the file only.
pub const EDITABLE: &[&str] = &[
    "default_enabled",
    "timezone",
    "raw_retention_days",
    "max_rows_per_dim",
    "max_open_visits",
    "session_timeout_min",
    "store_ip",
    "exclude_ips",
    "exclude_paths",
    "static_prefixes",
    "count_head_requests",
    "geoip.enabled",
];

#[derive(Debug, Clone)]
pub struct Config {
    merged: Value,
    file_path: Option<PathBuf>,
    load_error: Option<String>,
    overrides: BTreeMap<String, Value>,
}

impl Config {
    /// Defaults merged with the JSON file at `path` (missing or unreadable file → defaults
    /// plus `load_error()`, never a failure: the cPanel user cannot read root's config).
    pub fn load(path: Option<&Path>) -> Config {
        let mut merged: Value = serde_json::from_str(DEFAULTS_JSON).expect("valid defaults");
        let mut load_error = None;
        if let Some(p) = path {
            match std::fs::read(p) {
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(v) if v.is_object() => deep_merge(&mut merged, &v),
                    Ok(_) => load_error = Some(format!("{}: top level is not an object", p.display())),
                    Err(e) => load_error = Some(format!("{}: {e}", p.display())),
                },
                Err(e) => load_error = Some(format!("{}: {e}", p.display())),
            }
        }
        Config { merged, file_path: path.map(Path::to_path_buf), load_error, overrides: BTreeMap::new() }
    }

    pub fn path(&self) -> Option<&Path> { self.file_path.as_deref() }
    pub fn load_error(&self) -> Option<&str> { self.load_error.as_deref() }

    /// Applies registry overrides (`key` → JSON value); unknown or non-editable keys are ignored.
    pub fn apply_overrides(&mut self, overrides: BTreeMap<String, Value>) {
        for (k, v) in overrides {
            if EDITABLE.contains(&k.as_str()) {
                set_dotted(&mut self.merged, &k, v.clone());
                self.overrides.insert(k, v);
            }
        }
    }
    pub fn overrides(&self) -> &BTreeMap<String, Value> { &self.overrides }

    /// Raw value at a dotted key (`geoip.enabled`), `Null` when absent.
    pub fn get(&self, key: &str) -> &Value {
        let mut cur = &self.merged;
        for part in key.split('.') {
            match cur.get(part) {
                Some(v) => cur = v,
                None => return &Value::Null,
            }
        }
        cur
    }
    pub fn get_str(&self, key: &str) -> Option<String> { self.get(key).as_str().map(str::to_string) }
    pub fn get_bool(&self, key: &str, default: bool) -> bool { self.get(key).as_bool().unwrap_or(default) }
    pub fn get_i64(&self, key: &str, default: i64) -> i64 {
        match self.get(key) {
            Value::Number(n) => n.as_i64().unwrap_or(default),
            Value::String(s) => s.parse().unwrap_or(default),
            _ => default,
        }
    }
    pub fn get_array(&self, key: &str) -> Vec<Value> { self.get(key).as_array().cloned().unwrap_or_default() }
    pub fn get_str_list(&self, key: &str) -> Vec<String> {
        self.get_array(key).into_iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
    }
    /// The whole effective configuration (for `webpulse config get` and `admin.settings`).
    pub fn effective(&self) -> &Value { &self.merged }

    /// Validates a run-time override; returns the normalised value.
    pub fn validate(key: &str, value: &Value) -> Result<Value> {
        let bad = |m: &str| anyhow::anyhow!("{key}: {m}");
        let int_in = |lo: i64, hi: i64| -> Result<Value> {
            let n = match value {
                Value::Number(n) => n.as_i64(),
                Value::String(s) => s.trim().parse::<i64>().ok(),
                _ => None,
            }
            .ok_or_else(|| bad("must be an integer"))?;
            if n < lo || n > hi {
                return Err(bad(&format!("must be between {lo} and {hi}")));
            }
            Ok(Value::from(n))
        };
        let boolean = || -> Result<Value> {
            match value {
                Value::Bool(b) => Ok(Value::Bool(*b)),
                Value::Number(n) => Ok(Value::Bool(n.as_i64().unwrap_or(0) != 0)),
                Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                    "1" | "true" | "on" | "yes" => Ok(Value::Bool(true)),
                    "0" | "false" | "off" | "no" | "" => Ok(Value::Bool(false)),
                    _ => Err(bad("must be true or false")),
                },
                _ => Err(bad("must be true or false")),
            }
        };
        let str_list = |max_len: usize| -> Result<Value> {
            let items: Vec<String> = match value {
                Value::Array(a) => a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad("list items must be strings"))).collect::<Result<_>>()?,
                Value::String(s) => s.split(|c| c == ',' || c == '\n').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
                Value::Null => vec![],
                _ => return Err(bad("must be a list")),
            };
            if items.len() > 200 || items.iter().any(|s| s.len() > max_len || s.chars().any(char::is_control)) {
                return Err(bad("too many or too long entries"));
            }
            Ok(Value::from(items))
        };
        match key {
            "default_enabled" | "store_ip" | "count_head_requests" | "geoip.enabled" => boolean(),
            "raw_retention_days" => int_in(0, 3650),
            "max_rows_per_dim" => int_in(50, 100_000),
            "max_open_visits" => int_in(1000, 1_000_000),
            "session_timeout_min" => int_in(1, 240),
            "exclude_ips" => str_list(64),
            "exclude_paths" | "static_prefixes" => str_list(255),
            "timezone" => match value {
                Value::Null => Ok(Value::Null),
                Value::String(s) if s.trim().is_empty() => Ok(Value::Null),
                Value::String(s) => s.parse::<chrono_tz::Tz>().map(|_| Value::from(s.trim())).map_err(|_| bad("unknown timezone")),
                _ => Err(bad("must be a timezone name")),
            },
            _ => Err(bad("is not an editable setting")),
        }
    }

    /// Writes `DEFAULTS_JSON` to `path` (used by `setup` when no config exists).
    pub fn write_defaults(path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(path, format!("{}\n", DEFAULTS_JSON.trim())).with_context(|| format!("writing {}", path.display()))
    }
}

fn deep_merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(bv) if bv.is_object() && v.is_object() => deep_merge(bv, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

fn set_dotted(root: &mut Value, key: &str, value: Value) {
    let parts: Vec<&str> = key.split('.').collect();
    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if let Value::Object(m) = cur {
                m.insert((*part).to_string(), value);
            }
            return;
        }
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let m = cur.as_object_mut().expect("object");
        cur = m.entry((*part).to_string()).or_insert_with(|| Value::Object(Map::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_overrides() {
        let mut c = Config::load(None);
        assert_eq!(c.get_i64("raw_retention_days", 0), 30);
        assert!(c.get_bool("default_enabled", false));
        assert_eq!(c.get_str_list("paths.domlogs_dirs").len(), 4);
        let mut o = BTreeMap::new();
        o.insert("raw_retention_days".to_string(), Value::from(7));
        o.insert("geoip.enabled".to_string(), Value::Bool(true));
        o.insert("paths.data_dir".to_string(), Value::from("/x")); // not editable
        c.apply_overrides(o);
        assert_eq!(c.get_i64("raw_retention_days", 0), 7);
        assert!(c.get_bool("geoip.enabled", false));
        assert_eq!(c.get_str("paths.data_dir").as_deref(), Some("/var/webpulse/data"));
        assert!(Config::validate("timezone", &Value::from("Europe/Rome")).is_ok());
        assert!(Config::validate("timezone", &Value::from("Mars/Olympus")).is_err());
        assert!(Config::validate("max_rows_per_dim", &Value::from(10)).is_err());
    }
}
