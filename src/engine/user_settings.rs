//! Per-account settings file `<data_dir>/<user>/user/settings.json`
//! (`WebPulse\Collect\UserSettings`), written by the cPanel plugin (as the account)
//! and read by the collector (as root) with symlink, size and schema checks.
//!
//! ```json
//! { "version": 1,
//!   "domains": { "<domain>": { "timezone": null, "exclude_ips": [], "exclude_paths": [] } },
//!   "ui": { "default_range": "30d", "dark": "auto" } }
//! ```

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use crate::paths::{Paths, is_valid_domain};
use crate::util::{fs, ip, json as pjson, strs, time};

/// Largest settings file accepted.
pub const MAX_BYTES: u64 = 65536;
/// Most IPs / path prefixes per domain.
pub const MAX_ITEMS: usize = 50;
/// Most domains in one file.
pub const MAX_DOMAINS: usize = 500;
/// Accepted `ui.default_range` values.
pub const RANGES: &[&str] = &[
    "today",
    "yesterday",
    "7d",
    "30d",
    "90d",
    "12m",
    "month",
    "lastmonth",
    "year",
    "all",
];

/// Settings of one domain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainSettings {
    /// Domain timezone (`None` = account / server).
    pub timezone: Option<String>,
    /// IPs / CIDRs whose hits are ignored.
    pub exclude_ips: Vec<String>,
    /// Path prefixes whose hits are ignored.
    pub exclude_paths: Vec<String>,
}

impl DomainSettings {
    /// JSON object (`timezone`, `exclude_ips`, `exclude_paths`).
    pub fn to_json(&self) -> Value {
        json!({"timezone": self.timezone, "exclude_ips": self.exclude_ips, "exclude_paths": self.exclude_paths})
    }
}

/// Validated settings of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSettings {
    /// Always 1.
    pub version: i64,
    /// Lowercase domain → settings (sorted).
    pub domains: BTreeMap<String, DomainSettings>,
    /// Default range of the dashboard.
    pub default_range: String,
    /// `auto`, `light` or `dark`.
    pub dark: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        UserSettings {
            version: 1,
            domains: BTreeMap::new(),
            default_range: "30d".into(),
            dark: "auto".into(),
        }
    }
}

/// PHP `array_is_list` for JSON (arrays, and `{}` which decodes to `[]`).
fn is_list(v: &Value) -> bool {
    matches!(v, Value::Array(_)) || matches!(v, Value::Object(m) if m.is_empty())
}

/// A JSON container as PHP sees it: entries with their keys.
fn entries(v: &Value) -> Vec<(String, &Value)> {
    match v {
        Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(a) => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => Vec::new(),
    }
}

fn is_container(v: &Value) -> bool {
    matches!(v, Value::Object(_) | Value::Array(_))
}

/// A user-supplied key made safe for messages and logs (no control characters, ≤ 64 bytes).
fn printable(k: &str) -> String {
    strs::truncate(&strs::clean(k.as_bytes()), 64, strs::ELLIPSIS)
}

impl UserSettings {
    /// Settings of one domain (defaults when absent).
    pub fn for_domain(&self, domain: &str) -> DomainSettings {
        self.domains
            .get(&domain.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    /// JSON form (key order of 0.1.0).
    pub fn to_json(&self) -> Value {
        let mut d = Map::new();
        for (k, v) in &self.domains {
            d.insert(k.clone(), v.to_json());
        }
        json!({"version": self.version, "domains": if d.is_empty() { json!([]) } else { Value::Object(d) },
            "ui": {"default_range": self.default_range, "dark": self.dark}})
    }

    /// Reads and validates the settings of `user`. Any problem (missing file,
    /// symlink, too large, FIFO, invalid JSON or schema) yields the defaults; the
    /// reason is returned as the second element (`None` for a missing file).
    pub fn read(paths: &Paths, user: &str) -> (UserSettings, Option<String>) {
        if let Err(e) = paths.try_user_dir(user) {
            return (UserSettings::default(), Some(e.to_string()));
        }
        let dir = paths.user_private_dir(user);
        let file = paths.user_settings_file(user);
        let Ok(dst) = std::fs::symlink_metadata(&dir) else {
            return (UserSettings::default(), None);
        };
        if !dst.file_type().is_dir() {
            return (
                UserSettings::default(),
                Some("settings directory is not a directory".into()),
            );
        }
        let Ok(st) = std::fs::symlink_metadata(&file) else {
            return (UserSettings::default(), None);
        };
        if !st.file_type().is_file() {
            return (
                UserSettings::default(),
                Some("settings.json is not a regular file".into()),
            );
        }
        if st.len() > MAX_BYTES {
            return (
                UserSettings::default(),
                Some("settings.json is larger than 64 KiB".into()),
            );
        }
        let raw = match fs::read_regular(&file, MAX_BYTES, Some(&st)) {
            Ok(r) => r,
            Err(e) => {
                let m = e.to_string();
                let why = if m.contains("larger than") {
                    "settings.json is larger than 64 KiB"
                } else if m.contains("cannot open") {
                    "settings.json is not readable"
                } else {
                    "settings.json changed while reading"
                };
                return (UserSettings::default(), Some(why.into()));
            }
        };
        let data: Value = match serde_json::from_slice(&raw) {
            Ok(v) => v,
            Err(e) => return (UserSettings::default(), Some(format!("invalid JSON: {e}"))),
        };
        if !is_container(&data) {
            return (
                UserSettings::default(),
                Some("settings must be a JSON object".into()),
            );
        }
        match UserSettings::validate(&data) {
            Ok(s) => (s, None),
            Err(e) => (UserSettings::default(), Some(e.to_string())),
        }
    }

    /// Validates raw settings and returns them normalised; the error lists every
    /// problem (`"; "` separated), with the messages of 0.1.0.
    pub fn validate(raw: &Value) -> Result<UserSettings> {
        let mut errors: Vec<String> = Vec::new();
        let mut out = UserSettings::default();
        let get = |v: &Value, k: &str| -> Option<Value> {
            match v {
                Value::Object(m) => m.get(k).cloned(),
                Value::Array(a) => k.parse::<usize>().ok().and_then(|i| a.get(i).cloned()),
                _ => None,
            }
        };
        for (k, _) in entries(raw) {
            if !["version", "domains", "ui"].contains(&k.as_str()) {
                errors.push(format!("unknown key '{}'", printable(&k)));
            }
        }
        if let Some(v) = get(raw, "version")
            && !v.is_null()
            && !(v.is_i64() && v.as_i64() == Some(1))
        {
            errors.push("version must be 1".into());
        }
        let mut domains = get(raw, "domains")
            .filter(|v| !v.is_null())
            .unwrap_or(json!([]));
        if !is_container(&domains) || (is_list(&domains) && !entries(&domains).is_empty()) {
            errors.push("domains must be an object".into());
            domains = json!([]);
        }
        if entries(&domains).len() > MAX_DOMAINS {
            errors.push("too many domains".into());
            domains = json!([]);
        }
        for (d, cfg) in entries(&domains) {
            let d = d.to_ascii_lowercase();
            if !is_valid_domain(&d) {
                errors.push(format!(
                    "invalid domain {}",
                    pjson::encode_php_default(&Value::from(d.as_str()))
                ));
                continue;
            }
            if !is_container(cfg) || (is_list(cfg) && !entries(cfg).is_empty()) {
                errors.push(format!("{d}: must be an object"));
                continue;
            }
            let mut dc = DomainSettings::default();
            for (k, _) in entries(cfg) {
                if !["timezone", "exclude_ips", "exclude_paths"].contains(&k.as_str()) {
                    errors.push(format!("{d}: unknown key '{}'", printable(&k)));
                }
            }
            if let Some(tz) = get(cfg, "timezone").filter(|v| !v.is_null())
                && tz.as_str() != Some("")
            {
                match tz.as_str() {
                    Some(t) if time::is_valid_tz_identifier(t) => dc.timezone = Some(t.to_string()),
                    _ => errors.push(format!(
                        "{d}: invalid timezone {}",
                        pjson::encode_php_default(&tz)
                    )),
                }
            }
            for k in ["exclude_ips", "exclude_paths"] {
                let list = get(cfg, k).filter(|v| !v.is_null()).unwrap_or(json!([]));
                if !is_list(&list) {
                    errors.push(format!("{d}: {k} must be a list"));
                    continue;
                }
                let items = entries(&list);
                if items.len() > MAX_ITEMS {
                    errors.push(format!("{d}: {k} accepts at most {MAX_ITEMS} entries"));
                    continue;
                }
                let mut clean: Vec<String> = Vec::new();
                for (_, v) in items {
                    let v2 = match v.as_str() {
                        Some(s) => Value::from(strs::php_trim_str(s)),
                        None => v.clone(),
                    };
                    let ok = match v2.as_str() {
                        Some(s) if k == "exclude_ips" => ip::is_valid_cidr(s),
                        Some(s) => {
                            s.starts_with('/') && s.len() <= 255 && !s.bytes().any(|b| b < 0x20)
                        }
                        None => false,
                    };
                    if !ok {
                        let what = if k == "exclude_ips" {
                            "IP/CIDR "
                        } else {
                            "path prefix "
                        };
                        errors.push(format!(
                            "{d}: invalid {what}{}",
                            pjson::encode_php_default(&v2)
                        ));
                        continue;
                    }
                    let s = v2.as_str().unwrap_or_default().to_string();
                    if !clean.contains(&s) {
                        clean.push(s);
                    }
                }
                if k == "exclude_ips" {
                    dc.exclude_ips = clean;
                } else {
                    dc.exclude_paths = clean;
                }
            }
            out.domains.insert(d, dc);
        }
        let mut ui = get(raw, "ui").filter(|v| !v.is_null()).unwrap_or(json!([]));
        if !is_container(&ui) || (is_list(&ui) && !entries(&ui).is_empty()) {
            errors.push("ui must be an object".into());
            ui = json!([]);
        }
        for (k, _) in entries(&ui) {
            if !["default_range", "dark"].contains(&k.as_str()) {
                errors.push(format!("ui: unknown key '{}'", printable(&k)));
            }
        }
        if let Some(r) = get(&ui, "default_range").filter(|v| !v.is_null()) {
            match r.as_str() {
                Some(s) if RANGES.contains(&s) => out.default_range = s.to_string(),
                _ => errors.push(format!(
                    "ui.default_range must be one of {}",
                    RANGES.join(", ")
                )),
            }
        }
        if let Value::Object(m) = &ui
            && let Some(dark) = m.get("dark")
        {
            let v = match dark {
                Value::Null => Some("auto"),
                Value::String(s) if s == "auto" => Some("auto"),
                Value::String(s) if s == "dark" || s == "on" => Some("dark"),
                Value::String(s) if s == "light" || s == "off" => Some("light"),
                Value::Bool(true) => Some("dark"),
                Value::Bool(false) => Some("light"),
                _ => None,
            };
            match v {
                Some(v) => out.dark = v.to_string(),
                None => errors.push("ui.dark must be auto, light or dark".into()),
            }
        }
        if !errors.is_empty() {
            return Err(anyhow!("{}", errors.join("; ")));
        }
        Ok(out)
    }

    /// Validates and atomically writes the settings of `user` (meant for the cPanel
    /// plugin, which runs as the account); returns the normalised settings.
    pub fn write(paths: &Paths, user: &str, raw: &Value) -> Result<UserSettings> {
        let data = UserSettings::validate(raw)?;
        let text = format!("{}\n", pjson::encode_pretty(&data.to_json()));
        if text.len() as u64 > MAX_BYTES {
            return Err(anyhow!("settings larger than 64 KiB"));
        }
        paths.try_user_dir(user)?;
        let dir = paths.user_private_dir(user);
        if fs::is_link(&dir) || !dir.is_dir() {
            return Err(anyhow!("settings directory missing"));
        }
        let file = paths.user_settings_file(user);
        if fs::is_link(&file) {
            let _ = std::fs::remove_file(&file);
        }
        fs::write_atomic(&file, text.as_bytes(), 0o600)?;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        let v = UserSettings::validate(&json!({"version": 1, "domains": {"Example.COM": {"timezone": "Europe/Rome",
            "exclude_ips": ["1.2.3.4", "2001:db8::/32"], "exclude_paths": ["/x/"]}}, "ui": {"dark": "on"}}))
        .unwrap();
        assert_eq!(
            v.domains["example.com"].timezone.as_deref(),
            Some("Europe/Rome")
        );
        assert_eq!(v.dark, "dark");
        assert_eq!(v.default_range, "30d");
        assert_eq!(v.for_domain("other.com"), DomainSettings::default());
        let e = UserSettings::validate(&json!({
            "version": 2, "extra": 1,
            "domains": {"bad domain": {}, "ok.com": {"timezone": "Mars/X", "exclude_ips": ["999.1.1.1"], "exclude_paths": ["nope"], "zzz": 1}},
            "ui": {"default_range": "forever", "dark": "purple"}
        }))
        .unwrap_err()
        .to_string();
        for m in [
            "version must be 1",
            "unknown key 'extra'",
            "invalid domain",
            "invalid timezone",
            "invalid IP/CIDR",
            "invalid path prefix",
            "unknown key 'zzz'",
            "ui.default_range",
            "ui.dark",
        ] {
            assert!(e.contains(m), "{m} in {e}");
        }
        let many: Vec<&str> = vec!["1.1.1.1"; 51];
        let e = UserSettings::validate(&json!({"domains": {"a.com": {"exclude_ips": many}}}))
            .unwrap_err()
            .to_string();
        assert!(e.contains("at most 50"));
        let e = UserSettings::validate(&json!({"evil\n[error]": 1}))
            .unwrap_err()
            .to_string();
        assert!(!e.contains('\n'));
    }
}
