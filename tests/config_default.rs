//! `etc/config.json.default` (shipped in the release tarball; the installer seeds
//! `/etc/webpulse/config.json` from it) must stay in step with the binary: it is
//! the file `webpulse setup` falls back to ([`FILE_DEFAULT_JSON`]), and every value
//! it sets is the built-in default ([`DEFAULTS_JSON`]).

use serde_json::Value;
use webpulse::config::{DEFAULTS_JSON, FILE_DEFAULT_JSON};

fn shipped() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("etc/config.json.default");
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_shipped_default_config_is_the_setup_fallback() {
    let fallback: Value = serde_json::from_str(FILE_DEFAULT_JSON).unwrap();
    assert_eq!(
        shipped(),
        fallback,
        "etc/config.json.default == config::FILE_DEFAULT_JSON"
    );
}

/// Every leaf of `file` equals the same leaf of `defaults`.
fn check_subset(file: &Value, defaults: &Value, at: &str) {
    match file {
        Value::Object(m) => {
            for (k, v) in m {
                let key = if at.is_empty() {
                    k.clone()
                } else {
                    format!("{at}.{k}")
                };
                let d = defaults
                    .get(k)
                    .unwrap_or_else(|| panic!("{key} is not a built-in config key"));
                check_subset(v, d, &key);
            }
        }
        _ => assert_eq!(
            file, defaults,
            "{at}: shipped value differs from the built-in default"
        ),
    }
}

#[test]
fn the_shipped_default_config_matches_the_built_in_defaults() {
    let defaults: Value = serde_json::from_str(DEFAULTS_JSON).unwrap();
    check_subset(&shipped(), &defaults, "");
}
