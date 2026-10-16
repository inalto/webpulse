//! `webpulse domains list [USER] | enable|disable|reset USER DOMAIN` (`DomainsCommand`).

use anyhow::Result;
use serde_json::{Value, json};

use super::{Ctx, Kind, RawArgs, table, usage};
use crate::error::exit;
use crate::paths::{is_valid_domain, is_valid_user};
use crate::util::time;

/// One-line description.
pub const SUMMARY: &str = "list domains or switch collection per domain";

/// `--help` text.
pub const USAGE: &str = "Usage: webpulse domains list [USER]
       webpulse domains enable|disable|reset USER DOMAIN

A domain override wins over the account setting, except when the account
is explicitly disabled (which turns off all of its domains).";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// list | enable | disable | reset
    pub action: Option<String>,
    /// Account
    pub user: Option<String>,
    /// Domain
    pub domain: Option<String>,
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let action = raw.arg(0).unwrap_or("list");
    let app = ctx.app()?;
    let reg = app.registry()?;
    if action == "list" {
        let user = raw.arg(1);
        if let Some(u) = user
            && !is_valid_user(u)
        {
            return Err(usage(format!("invalid user name '{u}'")));
        }
        let rows = reg.domains(user)?;
        let has: Vec<bool> = rows
            .iter()
            .map(|r| {
                app.paths
                    .try_domain_db(&r.user, &r.domain)
                    .map(|p| p.is_file())
                    .unwrap_or(false)
            })
            .collect();
        let list: Vec<Value> = rows
            .iter()
            .zip(&has)
            .map(|(r, h)| {
                let mut j = r.to_json();
                j["has_data"] = json!(h);
                j
            })
            .collect();
        let tz = time::server_tz();
        ctx.result(&json!({"domains": list}), || {
            if rows.is_empty() {
                return "no domains".into();
            }
            let t: Vec<Vec<Value>> = rows
                .iter()
                .zip(&has)
                .map(|(r, h)| {
                    vec![
                        json!(r.user),
                        json!(r.domain),
                        json!(r.kind),
                        json!(r.enabled),
                        json!(match r.enabled_override {
                            None => "account",
                            Some(true) => "on",
                            Some(false) => "off",
                        }),
                        json!(h),
                        json!(
                            r.removed_at
                                .map(|t| time::format_ts(t, &tz, "%Y-%m-%d"))
                                .unwrap_or_default()
                        ),
                    ]
                })
                .collect();
            table(
                &[
                    "USER", "DOMAIN", "KIND", "ENABLED", "OVERRIDE", "DATA", "REMOVED",
                ],
                &t,
            )
        });
        return Ok(exit::OK);
    }
    if !["enable", "disable", "reset"].contains(&action) {
        return Err(usage(format!("unknown action '{action}'")));
    }
    let user = raw.arg(1);
    let domain = raw.arg(2).map(|d| d.to_ascii_lowercase());
    let (Some(user), Some(domain)) = (
        user.filter(|u| is_valid_user(u)),
        domain.filter(|d| is_valid_domain(d)),
    ) else {
        return Err(usage("USER and DOMAIN are required"));
    };
    let enabled = match action {
        "enable" => Some(true),
        "disable" => Some(false),
        _ => None,
    };
    reg.set_domain(user, &domain, enabled)?;
    let on = reg.is_enabled(user, &domain)?;
    ctx.result(
        &json!({"user": user, "domain": domain, "enabled": on}),
        || {
            format!(
                "{user}/{domain}: collection {}",
                if on { "enabled" } else { "disabled" }
            )
        },
    );
    Ok(exit::OK)
}
