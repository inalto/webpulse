//! `webpulse config get [KEY] | set KEY VALUE | unset KEY` (`ConfigCommand`).

use anyhow::Result;
use serde_json::{Map, Value};

use super::{Ctx, Kind, RawArgs};
use crate::config::{Config, EDITABLE};
use crate::error::exit;
use crate::util::json as pjson;

/// One-line description.
pub const SUMMARY: &str = "show the effective configuration or change a run-time setting";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[];

/// `--help` text.
pub fn usage_text() -> String {
    format!(
        "Usage: webpulse config get [KEY]\n       webpulse config set KEY VALUE     (VALUE is JSON, or a plain string)\n       webpulse config unset KEY\n\nRun-time settings are stored in the registry and override config.json:\n  {}\nOther keys (paths.*) are edited in config.json.",
        EDITABLE.join(", ")
    )
}

/// `--help` text (alias of [`usage_text`]).
pub fn usage() -> String {
    usage_text()
}

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// get | set | unset
    pub action: Option<String>,
    /// Dotted key
    pub key: Option<String>,
    /// JSON value or plain string
    pub value: Option<String>,
}

fn reloaded(ctx: &Ctx) -> Result<Config> {
    let app = ctx.app()?;
    let mut c = app.config.clone();
    c.apply_overrides(app.registry()?.settings());
    Ok(c)
}

fn one(key: &str, v: &Value) -> Value {
    let mut m = Map::new();
    m.insert(key.to_string(), v.clone());
    Value::Object(m)
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let action = raw.arg(0).unwrap_or("get");
    let app = ctx.app()?;
    let key = raw.arg(1);
    match action {
        "get" => {
            let Some(key) = key else {
                let data = app.config.effective().clone();
                ctx.result(&data, || pjson::encode_pretty(&data));
                return Ok(exit::OK);
            };
            let Some(v) = app.config.get_opt(key).cloned() else {
                ctx.err(&format!("unknown key '{key}'"));
                return Ok(exit::ERROR);
            };
            ctx.result(&v, || match &v {
                Value::String(s) => s.clone(),
                other => pjson::encode_pretty(other),
            });
            Ok(exit::OK)
        }
        "set" => {
            let (Some(key), Some(rawv)) = (key, raw.arg(2)) else {
                return Err(super::usage("KEY and VALUE are required"));
            };
            let value: Value = serde_json::from_str(rawv).unwrap_or_else(|_| Value::from(rawv));
            let value = match app.config.validate_here(key, &value) {
                Ok(v) => v,
                Err(e) => {
                    ctx.err(&e.to_string());
                    return Ok(exit::USAGE);
                }
            };
            app.registry()?.set_setting(key, &value)?;
            let nv = reloaded(ctx)?.get(key).clone();
            ctx.result(&one(key, &nv), || format!("{key} = {}", pjson::encode(&nv)));
            Ok(exit::OK)
        }
        "unset" => {
            let Some(key) = key else {
                return Err(super::usage("KEY is required"));
            };
            app.registry()?.delete_setting(key)?;
            let nv = reloaded(ctx)?.get(key).clone();
            ctx.result(&one(key, &nv), || {
                format!(
                    "{key} = {} (from config.json / default)",
                    pjson::encode(&nv)
                )
            });
            Ok(exit::OK)
        }
        other => Err(usage_err(other)),
    }
}

fn usage_err(action: &str) -> anyhow::Error {
    super::usage(format!("unknown action '{action}'"))
}

/// Keeps the scaffold name available.
pub fn run(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    main(ctx, raw)
}
