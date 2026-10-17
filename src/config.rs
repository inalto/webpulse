//! Layered configuration (`WebPulse\Config`): built-in defaults < `config.json` <
//! registry `settings` overrides (applied by `App` once the registry is readable).
//!
//! Only the keys in [`EDITABLE`] can be overridden from the registry (WHM settings,
//! `webpulse config set`); `paths.*` always come from the file so that the registry
//! location itself is never circular. Relative paths in the file are resolved
//! against the directory of the file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value};

use crate::util::{ip, json, strs, time};

/// Built-in defaults (`Config::DEFAULTS` of 0.1.0, same key order: `config get`
/// prints them in this order).
pub const DEFAULTS_JSON: &str = r#"{
  "version": 1,
  "mode": "auto",
  "default_enabled": true,
  "timezone": null,
  "raw_retention_days": 30,
  "max_rows_per_dim": 1000,
  "session_timeout_min": 30,
  "store_ip": false,
  "exclude_ips": [],
  "exclude_paths": ["/.well-known/", "/cpanel", "/whm", "/webmail", "/cgi-sys/"],
  "count_head_requests": false,
  "max_open_visits": 50000,
  "static_prefixes": ["/wp-content/uploads/", "/wp-includes/", "/wp-json/", "/wp-admin/admin-ajax.php", "/wp-cron.php", "/xmlrpc.php"],
  "geoip": {
    "enabled": false,
    "country_db": "/var/webpulse/geoip/dbip-country-lite.mmdb",
    "city_db": null,
    "url": "https://download.db-ip.com/free/dbip-country-lite-{YYYY}-{MM}.mmdb.gz"
  },
  "update": {
    "check": true,
    "repo": "inalto/webpulse",
    "url": null
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
    "hooks_file": "/var/cpanel/hooks.yaml",
    "cpanel_marker": "/usr/local/cpanel/cpanel",
    "apache_conf_dirs": ["/etc/httpd", "/etc/apache2"],
    "nginx_conf": "/etc/nginx/nginx.conf"
  },
  "sites": []
}"#;

/// Content `webpulse setup` writes when no configuration file exists and
/// `<install>/etc/config.json.default` cannot be read (same as that file in 0.1.0).
pub const FILE_DEFAULT_JSON: &str = r#"{
  "version": 1,
  "mode": "auto",
  "default_enabled": true,
  "timezone": null,
  "raw_retention_days": 30,
  "max_rows_per_dim": 1000,
  "session_timeout_min": 30,
  "store_ip": false,
  "exclude_ips": [],
  "exclude_paths": ["/.well-known/", "/cpanel", "/whm", "/webmail", "/cgi-sys/"],
  "count_head_requests": false,
  "geoip": { "enabled": false, "country_db": "/var/webpulse/geoip/dbip-country-lite.mmdb", "city_db": null },
  "update": { "check": true, "repo": "inalto/webpulse" },
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
    "home_root": "/home",
    "cpanel_marker": "/usr/local/cpanel/cpanel",
    "apache_conf_dirs": ["/etc/httpd", "/etc/apache2"],
    "nginx_conf": "/etc/nginx/nginx.conf"
  },
  "sites": []
}
"#;

/// Keys the WHM UI / `webpulse config set` may override at run time (stored in the
/// registry `settings` table), in the order `config --help` lists them.
pub const EDITABLE: &[&str] = &[
    "default_enabled",
    "timezone",
    "raw_retention_days",
    "max_rows_per_dim",
    "session_timeout_min",
    "store_ip",
    "exclude_ips",
    "exclude_paths",
    "count_head_requests",
    "geoip.enabled",
    "geoip.url",
    "geoip.city_db",
    "static_prefixes",
    "update.check",
];

/// The effective configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// defaults < file
    base: Value,
    /// base < overrides
    merged: Value,
    file_path: Option<PathBuf>,
    load_error: Option<String>,
    overrides: BTreeMap<String, Value>,
}

fn defaults() -> Value {
    serde_json::from_str(DEFAULTS_JSON).unwrap_or(Value::Null)
}

fn is_assoc(v: &Value) -> bool {
    matches!(v, Value::Object(m) if !m.is_empty())
}

/// `Config::mergeDeep`: objects merge recursively, anything else replaces.
fn merge_deep(base: &mut Value, over: &Value) {
    let (Value::Object(b), Value::Object(o)) = (base, over) else {
        return;
    };
    for (k, v) in o {
        match b.get_mut(k) {
            Some(bv) if is_assoc(v) && is_assoc(bv) => merge_deep(bv, v),
            _ => {
                b.insert(k.clone(), v.clone());
            }
        }
    }
}

fn dig<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    let mut cur = root;
    for part in key.split('.') {
        cur = match cur {
            Value::Object(m) => m.get(part)?,
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn set_dotted(root: &mut Value, key: &str, value: Value) {
    let parts: Vec<&str> = key.split('.').collect();
    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let Some(m) = cur.as_object_mut() else {
            return;
        };
        if i == parts.len() - 1 {
            m.insert((*part).to_string(), value);
            return;
        }
        let e = m
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !e.is_object() && !e.is_array() {
            *e = Value::Object(Map::new());
        }
        cur = e;
    }
}

/// PHP `(int)` cast of a JSON value.
pub fn php_int(v: &Value) -> i64 {
    match v {
        Value::Null => 0,
        Value::Bool(b) => *b as i64,
        Value::Number(n) => n
            .as_i64()
            .unwrap_or_else(|| n.as_f64().map(|f| f as i64).unwrap_or(0)),
        Value::String(s) => php_str_int(s),
        Value::Array(a) => !a.is_empty() as i64,
        Value::Object(m) => !m.is_empty() as i64,
    }
}

/// PHP `(int)` cast of a string: leading whitespace, optional sign, digits
/// (saturating); `"1e3"` style numeric strings follow the float conversion.
pub fn php_str_int(s: &str) -> i64 {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    if let Ok(f) = t
        .trim_end_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c'])
        .parse::<f64>()
        && t.contains(['e', 'E', '.'])
        && f.is_finite()
    {
        return f as i64;
    }
    let b = t.as_bytes();
    let mut i = 0;
    let neg = match b.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };
    let mut n: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        let d = (b[i] - b'0') as i64;
        n = if neg {
            n.saturating_mul(10).saturating_sub(d)
        } else {
            n.saturating_mul(10).saturating_add(d)
        };
        i += 1;
    }
    n
}

/// PHP `(bool)` cast of a JSON value.
pub fn php_bool(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !(s.is_empty() || s == "0"),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

impl Config {
    fn from_file_data(
        file: Option<Value>,
        path: Option<&Path>,
        load_error: Option<String>,
    ) -> Config {
        let mut base = defaults();
        if let Some(mut f) = file {
            if let Some(p) = path {
                let dir = p
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                resolve_relative_paths(&mut f, dir);
            }
            merge_deep(&mut base, &f);
        }
        Config {
            merged: base.clone(),
            base,
            file_path: path.map(Path::to_path_buf),
            load_error,
            overrides: BTreeMap::new(),
        }
    }

    /// Loads a config file. A missing or unreadable file yields the defaults with
    /// `load_error()` = `missing` / `unreadable` (the cPanel plugin runs as the
    /// account and cannot read the root-only file); a file that exists but is not a
    /// JSON object is an error.
    pub fn try_load(path: Option<&Path>) -> Result<Config> {
        let Some(p) = path.filter(|p| !p.as_os_str().is_empty()) else {
            return Ok(Config::from_file_data(None, None, None));
        };
        if !p.exists() {
            return Ok(Config::from_file_data(
                None,
                Some(p),
                Some("missing".into()),
            ));
        }
        let raw = match std::fs::read(p) {
            Ok(r) => r,
            Err(_) => {
                return Ok(Config::from_file_data(
                    None,
                    Some(p),
                    Some("unreadable".into()),
                ));
            }
        };
        let data: Value = serde_json::from_slice(&raw)
            .map_err(|e| anyhow!("invalid JSON in {}: {e}", p.display()))?;
        match data {
            Value::Object(_) => Ok(Config::from_file_data(Some(data), Some(p), None)),
            Value::Array(_) => Ok(Config::from_file_data(None, Some(p), None)),
            _ => Err(anyhow!("{} must contain a JSON object", p.display())),
        }
    }

    /// Like [`Config::try_load`] but never fails: an invalid file yields the defaults
    /// and `load_error()` describes the problem.
    pub fn load(path: Option<&Path>) -> Config {
        match Config::try_load(path) {
            Ok(c) => c,
            Err(e) => Config::from_file_data(None, path, Some(e.to_string())),
        }
    }

    /// Path of the loaded file (`None` when running on defaults).
    pub fn path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    /// `missing` / `unreadable` (or a JSON error) when the file could not be used.
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Applies registry overrides (`key` → JSON value); non-editable keys are ignored.
    /// Replaces previously applied overrides.
    pub fn apply_overrides(&mut self, overrides: BTreeMap<String, Value>) {
        let mut merged = self.base.clone();
        let mut kept = BTreeMap::new();
        for (k, v) in overrides {
            if EDITABLE.contains(&k.as_str()) {
                set_dotted(&mut merged, &k, v.clone());
                kept.insert(k, v);
            }
        }
        self.merged = merged;
        self.overrides = kept;
    }

    /// Registry overrides currently applied (dotted keys).
    pub fn overrides(&self) -> &BTreeMap<String, Value> {
        &self.overrides
    }

    /// Raw value at a dotted key (`geoip.enabled`), `Null` when absent.
    pub fn get(&self, key: &str) -> &Value {
        dig(&self.merged, key).unwrap_or(&Value::Null)
    }

    /// Value at a dotted key, `None` when the key does not exist (PHP `get($k, $default)`).
    pub fn get_opt(&self, key: &str) -> Option<&Value> {
        dig(&self.merged, key)
    }

    /// Value without the registry overrides (defaults < file).
    pub fn file_value(&self, key: &str) -> Option<&Value> {
        dig(&self.base, key)
    }

    /// String value (only when it is a JSON string).
    pub fn get_str(&self, key: &str) -> Option<String> {
        self.get(key).as_str().map(str::to_string)
    }
    /// Boolean value (only when it is a JSON boolean), else `default`.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.get(key).as_bool().unwrap_or(default)
    }
    /// Integer value (numbers and numeric strings), else `default`.
    pub fn get_i64(&self, key: &str, default: i64) -> i64 {
        match self.get(key) {
            Value::Number(n) => n.as_i64().unwrap_or(default),
            Value::String(s) => s.parse().unwrap_or(default),
            _ => default,
        }
    }
    /// PHP `(int) $config->get($key, $default)`: `default` only when the key is missing.
    pub fn int(&self, key: &str, default: i64) -> i64 {
        self.get_opt(key).map(php_int).unwrap_or(default)
    }
    /// PHP `(int) ($config[$key] ?? $default)`: `default` when missing or null.
    pub fn int_or(&self, key: &str, default: i64) -> i64 {
        match self.get_opt(key) {
            None | Some(Value::Null) => default,
            Some(v) => php_int(v),
        }
    }
    /// PHP `(bool) $config->get($key, $default)`.
    pub fn truthy(&self, key: &str, default: bool) -> bool {
        self.get_opt(key).map(php_bool).unwrap_or(default)
    }
    /// Array value (empty when not an array).
    pub fn get_array(&self, key: &str) -> Vec<Value> {
        self.get(key).as_array().cloned().unwrap_or_default()
    }
    /// The string items of an array value.
    pub fn get_str_list(&self, key: &str) -> Vec<String> {
        self.get_array(key)
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    }
    /// The whole effective configuration (for `webpulse config get` and `admin.settings`).
    pub fn effective(&self) -> &Value {
        &self.merged
    }

    /// Directory of the GeoIP databases: the directory of `geoip.country_db`.
    pub fn geoip_dir(&self) -> PathBuf {
        geoip_dir_of(self.get_str("geoip.country_db").as_deref())
    }

    /// [`Config::validate`] with the GeoIP directory of this configuration (an
    /// optional city database must be a file inside it).
    pub fn validate_here(&self, key: &str, value: &Value) -> Result<Value> {
        Config::validate_with(key, value, &self.geoip_dir())
    }

    /// Validates and normalises a value for an editable key (`Config::validate`); the
    /// error message is the user-facing one of 0.1.0. `geoip.city_db` is checked
    /// against the default GeoIP directory (see [`Config::validate_here`]).
    pub fn validate(key: &str, value: &Value) -> Result<Value> {
        Config::validate_with(key, value, &geoip_dir_of(None))
    }

    /// [`Config::validate`] with an explicit GeoIP directory.
    pub fn validate_with(key: &str, value: &Value, geoip_dir: &Path) -> Result<Value> {
        if !EDITABLE.contains(&key) {
            return Err(anyhow!(
                "setting '{key}' cannot be changed at run time (edit config.json)"
            ));
        }
        let int = |min: i64, max: i64| -> Result<Value> {
            let n = match value {
                Value::Number(n) => n.as_i64(),
                Value::String(s) if is_int_string(s) => Some(php_str_int(s)),
                _ => None,
            };
            match n {
                Some(n) if n >= min && n <= max => Ok(Value::from(n)),
                _ => Err(anyhow!("{key} must be an integer between {min} and {max}")),
            }
        };
        let boolean = || -> Result<Value> {
            match value {
                Value::Bool(b) => Ok(Value::Bool(*b)),
                Value::String(s) if ["1", "true", "on", "yes"].contains(&s.as_str()) => {
                    Ok(Value::Bool(true))
                }
                Value::String(s) if ["0", "false", "off", "no"].contains(&s.as_str()) => {
                    Ok(Value::Bool(false))
                }
                Value::Number(n) if n.as_i64() == Some(1) && !n.is_f64() => Ok(Value::Bool(true)),
                Value::Number(n) if n.as_i64() == Some(0) && !n.is_f64() => Ok(Value::Bool(false)),
                _ => Err(anyhow!("{key} must be a boolean")),
            }
        };
        let list = |max: usize, ok: &dyn Fn(&str) -> bool, what: &str| -> Result<Value> {
            let items: &[Value] = match value {
                Value::Array(a) => a,
                Value::Object(m) if m.is_empty() => &[],
                _ => return Err(anyhow!("{key} must be a list")),
            };
            if items.len() > max {
                return Err(anyhow!("{key} accepts at most {max} entries"));
            }
            let mut out: Vec<String> = Vec::new();
            for e in items {
                let s = e.as_str().map(strs::php_trim_str);
                match s {
                    Some(t) if ok(t) => {
                        if !out.iter().any(|x| x == t) {
                            out.push(t.to_string());
                        }
                    }
                    _ => {
                        return Err(anyhow!(
                            "{key}: invalid {what} {}",
                            json::encode_php_default(e)
                        ));
                    }
                }
            }
            Ok(Value::from(out))
        };
        let path_prefix = |p: &str| -> bool {
            p.starts_with('/') && p.len() <= 255 && !p.bytes().any(|b| b < 0x20)
        };
        match key {
            "default_enabled"
            | "store_ip"
            | "count_head_requests"
            | "geoip.enabled"
            | "update.check" => boolean(),
            "raw_retention_days" => int(1, 3650),
            "max_rows_per_dim" => int(10, 100_000),
            "session_timeout_min" => int(5, 240),
            "timezone" => match value {
                Value::Null => Ok(Value::Null),
                Value::String(s) if s.is_empty() || s == "null" => Ok(Value::Null),
                Value::String(s) if time::is_valid_tz_identifier(s) => Ok(Value::from(s.as_str())),
                _ => Err(anyhow!(
                    "timezone must be a valid identifier such as Europe/Rome"
                )),
            },
            "exclude_ips" => list(500, &ip::is_valid_cidr, "IP/CIDR"),
            "exclude_paths" | "static_prefixes" => list(200, &path_prefix, "path prefix"),
            "geoip.url" => validate_geoip_url(value),
            "geoip.city_db" => validate_city_db(value, geoip_dir),
            _ => Err(anyhow!(
                "setting '{key}' cannot be changed at run time (edit config.json)"
            )),
        }
    }

    /// Writes [`FILE_DEFAULT_JSON`] to `path` (directories created as needed).
    pub fn write_defaults(path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(path, FILE_DEFAULT_JSON)
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// Directory of a country database path (default `/var/webpulse/geoip`).
fn geoip_dir_of(country_db: Option<&str>) -> PathBuf {
    let default = "/var/webpulse/geoip/dbip-country-lite.mmdb";
    let db = country_db.filter(|s| s.starts_with('/')).unwrap_or(default);
    Path::new(db)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `geoip.url`: an `https://` URL of a `.mmdb` or `.mmdb.gz` file, optionally with the
/// `{YYYY}` and `{MM}` placeholders; empty or null resets it (built-in DB-IP URL).
pub fn validate_geoip_url(value: &Value) -> Result<Value> {
    let s = match value {
        Value::Null => return Ok(Value::Null),
        Value::String(s) => s.trim(),
        _ => return Err(anyhow!("geoip.url must be a string")),
    };
    if s.is_empty() || s == "null" {
        return Ok(Value::Null);
    }
    let bad = || {
        anyhow!(
            "geoip.url must be an https:// URL ending in .mmdb or .mmdb.gz ({{YYYY}} and {{MM}} are replaced by the year and month)"
        )
    };
    let lower = s.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("https://") else {
        return Err(bad());
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let stripped = lower.replace("{yyyy}", "").replace("{mm}", "");
    if s.len() > 500
        || host.is_empty()
        || host.contains('{')
        || s.bytes().any(|b| b <= 0x20 || b == 0x7f || b == b'\\')
        || stripped.contains(['{', '}'])
        || !(path.ends_with(".mmdb") || path.ends_with(".mmdb.gz"))
    {
        return Err(bad());
    }
    Ok(Value::from(s))
}

/// `geoip.city_db`: an existing `.mmdb` file inside `dir` (the directory of the country
/// database), or empty / null for none.
pub fn validate_city_db(value: &Value, dir: &Path) -> Result<Value> {
    let s = match value {
        Value::Null => return Ok(Value::Null),
        Value::String(s) => s.trim(),
        _ => return Err(anyhow!("geoip.city_db must be a string")),
    };
    if s.is_empty() || s == "null" {
        return Ok(Value::Null);
    }
    let p = Path::new(s);
    let inside = p.parent() == Some(dir)
        || std::fs::canonicalize(dir)
            .ok()
            .zip(p.parent().and_then(|d| std::fs::canonicalize(d).ok()))
            .is_some_and(|(a, b)| a == b);
    if !s.starts_with('/')
        || s.len() > 255
        || s.bytes().any(|b| b < 0x20)
        || s.split('/').any(|c| c == "..")
        || !s.to_ascii_lowercase().ends_with(".mmdb")
        || !inside
    {
        return Err(anyhow!(
            "geoip.city_db must be an .mmdb file in {}",
            dir.display()
        ));
    }
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_file() => {
            crate::engine::classify::geoip::read_metadata(p)
                .map_err(|_| anyhow!("geoip.city_db: {s} is not a MaxMind DB file"))?;
            Ok(Value::from(s))
        }
        _ => Err(anyhow!("geoip.city_db: {s} is not a regular file")),
    }
}

/// `/^-?\d+\z/`
fn is_int_string(s: &str) -> bool {
    let d = s.strip_prefix('-').unwrap_or(s);
    !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())
}

/// `Config::resolveRelativePaths`: relative `paths.*` (and lists of them) and
/// `geoip.country_db` / `geoip.city_db` are made absolute against `base`.
fn resolve_relative_paths(file: &mut Value, base: &Path) {
    let fix = |v: &mut Value| {
        if let Value::String(s) = v
            && !s.is_empty()
            && !s.starts_with('/')
        {
            *s = format!("{}/{}", base.display(), s);
        }
    };
    if let Some(Value::Object(paths)) = file.get_mut("paths") {
        for (_, v) in paths.iter_mut() {
            match v {
                Value::Array(a) => a.iter_mut().for_each(fix),
                Value::Object(m) => m.values_mut().for_each(fix),
                other => fix(other),
            }
        }
    }
    if let Some(Value::Object(g)) = file.get_mut("geoip") {
        for k in ["country_db", "city_db"] {
            if let Some(v) = g.get_mut(k) {
                fix(v);
            }
        }
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
        assert_eq!(
            c.get_str("geoip.country_db").as_deref(),
            Some("/var/webpulse/geoip/dbip-country-lite.mmdb")
        );
        assert_eq!(
            c.get_str("paths.data_dir").as_deref(),
            Some("/var/webpulse/data")
        );
        let keys: Vec<&String> = c.effective().as_object().unwrap().keys().collect();
        assert_eq!(keys[0], "version");
        assert_eq!(keys[1], "mode");
        assert_eq!(keys[12], "static_prefixes");
    }

    #[test]
    fn validation_like_php() {
        assert!(Config::validate("timezone", &Value::from("Europe/Rome")).is_ok());
        assert_eq!(
            Config::validate("timezone", &Value::from("")).unwrap(),
            Value::Null
        );
        assert!(Config::validate("timezone", &Value::from("Mars/Olympus")).is_err());
        assert!(Config::validate("max_rows_per_dim", &Value::from(5)).is_err());
        assert_eq!(
            Config::validate("raw_retention_days", &Value::from("60")).unwrap(),
            Value::from(60)
        );
        assert!(Config::validate("raw_retention_days", &Value::from("banana")).is_err());
        assert!(Config::validate("raw_retention_days", &serde_json::json!(60.0)).is_err());
        assert_eq!(
            Config::validate("geoip.enabled", &Value::from("true")).unwrap(),
            Value::Bool(true)
        );
        assert!(Config::validate("geoip.enabled", &Value::from("")).is_err());
        assert_eq!(
            Config::validate(
                "exclude_ips",
                &serde_json::json!([" 1.2.3.0/24 ", "1.2.3.0/24"])
            )
            .unwrap(),
            serde_json::json!(["1.2.3.0/24"])
        );
        let e = Config::validate("exclude_paths", &serde_json::json!(["relative"])).unwrap_err();
        assert_eq!(
            e.to_string(),
            "exclude_paths: invalid path prefix \"relative\""
        );
        let e = Config::validate("paths.data_dir", &Value::from("/tmp")).unwrap_err();
        assert_eq!(
            e.to_string(),
            "setting 'paths.data_dir' cannot be changed at run time (edit config.json)"
        );
        assert_eq!(php_str_int("12abc"), 12);
        assert_eq!(php_str_int("99999999999999999999999"), i64::MAX);
    }
}
