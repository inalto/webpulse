//! `webpulse accounts list|enable|disable|reset` (`AccountsCommand`).

use anyhow::Result;
use serde_json::{Value, json};

use super::{Ctx, Kind, RawArgs, table, usage};
use crate::engine::collector::Collector;
use crate::error::exit;
use crate::paths::is_valid_user;
use crate::util::time;

/// One-line description.
pub const SUMMARY: &str = "list accounts or switch collection per account";

/// `--help` text.
pub const USAGE: &str = "Usage: webpulse accounts list [--owner RESELLER] [--q TEXT]
       webpulse accounts enable|disable|reset USER [--timezone TZ]

  enable / disable  explicit override for the account
  reset             back to the server default (default_enabled)
  --timezone TZ     also set the account timezone (\"\" = server timezone)";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[
    ("owner", Kind::Value),
    ("q", Kind::Value),
    ("timezone", Kind::Value),
    ("no-sync", Kind::Flag),
];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// list | enable | disable | reset
    pub action: Option<String>,
    /// Account
    pub user: Option<String>,
    /// Only accounts of this reseller
    #[arg(long)]
    pub owner: Option<String>,
    /// Substring of the user name or one of its domains
    #[arg(long)]
    pub q: Option<String>,
    /// Also set the account timezone ("" = server timezone)
    #[arg(long)]
    pub timezone: Option<String>,
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let action = raw.arg(0).unwrap_or("list");
    let app = ctx.app()?;
    let reg = app.registry()?;
    if action == "list" {
        if !raw.flag("no-sync") {
            Collector::new(app).sync_registry(None)?;
        }
        let rows = reg.accounts(raw.opt("owner"), raw.opt("q"))?;
        let data = json!({"accounts": rows.iter().map(|r| r.to_json()).collect::<Vec<_>>()});
        let tz = time::server_tz();
        ctx.result(&data, || {
            if rows.is_empty() {
                return "no accounts".into();
            }
            let t: Vec<Vec<Value>> = rows
                .iter()
                .map(|r| {
                    vec![
                        json!(r.user),
                        json!(r.owner),
                        json!(r.enabled),
                        json!(match r.enabled_override {
                            None => "default",
                            Some(true) => "on",
                            Some(false) => "off",
                        }),
                        json!(r.domains),
                        json!(r.timezone),
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
                    "USER", "OWNER", "ENABLED", "OVERRIDE", "DOMAINS", "TIMEZONE", "REMOVED",
                ],
                &t,
            )
        });
        return Ok(exit::OK);
    }
    if !["enable", "disable", "reset"].contains(&action) {
        return Err(usage(format!("unknown action '{action}'")));
    }
    let user = match raw.arg(1) {
        Some(u) if is_valid_user(u) => u,
        _ => return Err(usage("a valid user name is required")),
    };
    if !app.cpanel_users().exists(user) && reg.account(user)?.is_none() {
        ctx.err(&format!("warning: {user} is not a known cPanel account"));
    }
    let enabled = match action {
        "enable" => Some(true),
        "disable" => Some(false),
        _ => None,
    };
    reg.set_account(user, enabled, raw.opt("timezone"))?;
    let row = reg.account(user)?;
    let data = json!({"account": row.as_ref().map(|r| r.to_json())});
    ctx.result(&data, || {
        let (en, ov) = row
            .as_ref()
            .map(|r| (r.enabled, r.enabled_override))
            .unwrap_or((false, None));
        format!(
            "{user}: collection {} ({})",
            if en { "enabled" } else { "disabled" },
            if ov.is_none() {
                "server default"
            } else {
                "override"
            }
        )
    });
    Ok(exit::OK)
}
