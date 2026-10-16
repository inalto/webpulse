//! `a=user_settings` / `a=save_user_settings`: the cPanel account's own
//! per-domain settings file (`<data_dir>/<user>/user/settings.json`), port of
//! `Http\UserSettingsApi`. Reading and validation are the engine's
//! (`engine::user_settings`, port of `Collect\UserSettings`), so the dashboard
//! and the collector accept exactly the same documents; writing keeps the
//! API's own rules (temporary file in the same directory, 0644, rename).

use std::path::Path;

use serde_json::{Map, Value, json};

use super::api::ApiError;
use super::ctx::Ctx;
use super::json as pjson;
use super::principal::Principal;
use super::request::{Request, entries, field, is_array, is_empty_array};
use super::resolver;
use crate::engine::user_settings::UserSettings;
use crate::paths::{Paths, is_valid_user};
use crate::reports::{invalid, php};

/// Largest settings file.
pub const MAX_BYTES: usize = 65536;

/// True for `addr` or `addr/prefix` with a prefix valid for the family
/// (`Util\Ip::isValidCidr`, shared with the engine).
pub fn is_valid_cidr(cidr: &str) -> bool {
    crate::util::ip::is_valid_cidr(cidr)
}

/// Default settings.
pub fn defaults() -> Value {
    UserSettings::default().to_json()
}

/// Validates raw settings and returns them normalised with defaults; the
/// error lists every problem (`; ` separated). Same rules as the collector
/// (`engine::user_settings`).
pub fn validate(raw: &Value) -> Result<Value, String> {
    UserSettings::validate(raw)
        .map(|s| s.to_json())
        .map_err(|e| e.to_string())
}

/// Reads and validates the settings of `user`; any problem yields the defaults.
pub fn read(paths: &Paths, user: &str) -> Value {
    UserSettings::read(paths, user).0.to_json()
}

fn visible(ctx: &Ctx, p: &Principal) -> Vec<String> {
    resolver::visible_domains(p, None, &ctx.paths)
        .into_iter()
        .map(|d| d.domain)
        .collect()
}

/// `a=user_settings`.
pub fn get(ctx: &Ctx, r: &Request, p: &Principal) -> anyhow::Result<Value> {
    if !p.is_user() {
        return Err(ApiError::forbidden("Settings are managed by each account in cPanel").into());
    }
    Ok(json!({
        "settings": read(&ctx.paths, &p.name),
        "remote_addr": r.remote_addr,
        "domains": visible(ctx, p),
        "server_timezone": ctx.server_timezone(),
    }))
}

/// `a=save_user_settings`.
pub fn save(ctx: &Ctx, r: &Request, p: &Principal) -> anyhow::Result<Value> {
    if !p.is_user() {
        return Err(
            ApiError::forbidden("Settings can only be changed from the cPanel account").into(),
        );
    }
    let body = r.json()?;
    if is_empty_array(&body) {
        return Err(ApiError::bad_request("Request body with the settings is required").into());
    }
    let raw = match field(&body, "settings") {
        Some(s) if is_array(s) => s.clone(),
        _ => body.clone(),
    };
    let visible = visible(ctx, p);
    if let Some(doms) = field(&raw, "domains").filter(|d| is_array(d)) {
        for (dom, _) in entries(doms) {
            if !visible.contains(&dom) {
                return Err(ApiError::bad_request(&format!(
                    "Unknown domain \"{}\"",
                    php::substr(&dom, 60)
                ))
                .into());
            }
        }
    }
    let mut raw_map: Map<String, Value> = entries(&raw)
        .into_iter()
        .map(|(k, v)| (k, v.clone()))
        .collect();
    raw_map.insert("version".into(), json!(1));
    let clean = validate(&Value::Object(raw_map)).map_err(invalid)?;
    let text = pjson::encode(&clean, pjson::SETTINGS_FILE) + "\n";
    if text.len() > MAX_BYTES {
        return Err(ApiError::bad_request("Settings are too large").into());
    }
    if !is_valid_user(&p.name) {
        return Err(invalid(format!(
            "invalid user name: {}",
            pjson::quote(&p.name)
        )));
    }
    write_atomic(&ctx.paths.user_settings_file(&p.name), text.as_bytes())?;
    Ok(json!({"settings": clean, "saved": true}))
}

fn unwritable(msg: &str) -> ApiError {
    ApiError::new(500, "settings_unwritable", msg)
}

/// Writes via a temporary file in the same directory and `rename()`; refuses
/// a symlinked directory or target.
pub fn write_atomic(file: &Path, data: &[u8]) -> Result<(), ApiError> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let dir = file.parent().unwrap_or(Path::new("/"));
    let is_link = |p: &Path| {
        std::fs::symlink_metadata(p)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    };
    if is_link(dir) || is_link(file) {
        return Err(unwritable("The settings location is not safe to write"));
    }
    if !dir.is_dir() || !super::registry::access(dir, libc::W_OK) {
        return Err(unwritable(
            "The settings directory is missing or not writable; ask your administrator to run \"webpulse setup\"",
        ));
    }
    let mut rnd = [0u8; 6];
    crate::http::page::random_bytes(&mut rnd);
    let tmp = dir.join(format!(
        ".settings-{}",
        rnd.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        std::fs::rename(&tmp, file)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(unwritable("Could not save the settings"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr() {
        for ok in [
            "203.0.113.7",
            "203.0.113.0/24",
            "::1",
            "2001:db8::/32",
            " 10.0.0.0/8 ",
            "::ffff:1.2.3.4",
        ] {
            assert!(is_valid_cidr(ok), "{ok}");
        }
        for bad in [
            "not-an-ip",
            "10.0.0.0/33",
            "10.0.0.0/",
            "10.0.0.0/-1",
            "01.2.3.4",
            "::1/129",
            "1.2.3",
        ] {
            assert!(!is_valid_cidr(bad), "{bad}");
        }
    }

    #[test]
    fn validation() {
        let ok = validate(&json!({"domains": {"Example.com": {"timezone": "Europe/Berlin", "exclude_ips": ["203.0.113.0/24", "203.0.113.0/24"]}},
            "ui": {"default_range": "7d", "dark": true}}))
        .unwrap();
        assert_eq!(
            ok,
            json!({"version": 1, "domains": {"example.com": {"timezone": "Europe/Berlin", "exclude_ips": ["203.0.113.0/24"], "exclude_paths": []}},
                "ui": {"default_range": "7d", "dark": "dark"}})
        );
        let e = validate(&json!({"x": 1, "domains": {"example.com": {"timezone": "Mars/Olympus", "exclude_ips": ["nope"]}}})).unwrap_err();
        assert_eq!(
            e,
            "unknown key 'x'; example.com: invalid timezone \"Mars\\/Olympus\"; example.com: invalid IP/CIDR \"nope\""
        );
        assert!(validate(&json!({"domains": [1]})).is_err());
        assert_eq!(
            pjson::encode(&defaults(), pjson::SETTINGS_FILE)
                .lines()
                .nth(2),
            Some("    \"domains\": [],")
        );
    }
}
