//! `webpulse serve [--port 8099] (--user U | --admin | --reseller R) [--allow-root]`:
//! the local dev dashboard on `http://127.0.0.1:<port>` (see `http::dev_server`).

use crate::app::App;
use crate::error::{Result, WpError};
use crate::paths::is_valid_user;

/// Options of `webpulse serve`.
#[derive(Debug, clap::Args)]
#[command(
    after_help = "The dashboard is served at / and the API at /api, acting as the given principal. \
The server only listens on 127.0.0.1 and answers only requests carrying the random token of the printed URL \
(http://127.0.0.1:PORT/?token=…; the page then keeps it in a cookie). Stop it with Ctrl-C."
)]
pub struct Args {
    /// TCP port on 127.0.0.1
    #[arg(long, default_value = "8099")]
    pub port: String,
    /// Act as a cPanel account (sees its own domains)
    #[arg(long)]
    pub user: Option<String>,
    /// Act as the WHM root administrator (default)
    #[arg(long)]
    pub admin: bool,
    /// Act as a reseller (sees the accounts it owns)
    #[arg(long)]
    pub reseller: Option<String>,
    /// Run even as root (refused by default)
    #[arg(long)]
    pub allow_root: bool,
}

/// `user:<name>`, `admin` or `reseller:<name>` from the options.
pub fn principal_spec(user: Option<&str>, admin: bool, reseller: Option<&str>) -> Result<String> {
    let given = user.is_some() as u8 + admin as u8 + reseller.is_some() as u8;
    if given > 1 {
        return Err(WpError::Usage("use only one of --user, --admin, --reseller".into()).into());
    }
    if let Some(u) = user {
        if !is_valid_user(u) {
            return Err(WpError::Usage(format!("invalid user name '{u}'")).into());
        }
        return Ok(format!("user:{u}"));
    }
    if let Some(r) = reseller {
        if !is_valid_user(r) {
            return Err(WpError::Usage(format!("invalid reseller name '{r}'")).into());
        }
        return Ok(format!("reseller:{r}"));
    }
    Ok("admin".into())
}

/// Runs the dev server.
pub fn run(app: &App, a: &Args) -> Result<i32> {
    let port: u16 = match a.port.parse::<u32>() {
        Ok(p) if a.port.bytes().all(|c| c.is_ascii_digit()) && (1..=65535).contains(&p) => p as u16,
        _ => {
            return Err(
                WpError::Usage("--port must be a number between 1 and 65535".into()).into(),
            );
        }
    };
    let spec = principal_spec(a.user.as_deref(), a.admin, a.reseller.as_deref())?;
    if app.is_root() && !a.allow_root {
        eprintln!(
            "webpulse serve is a development tool: refusing to run as root (use --allow-root to override)"
        );
        return Ok(crate::error::exit::ERROR);
    }
    crate::http::dev_server::principal_from_spec(app, &spec)?;
    crate::http::dev_server::run(app, port, &spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs() {
        assert_eq!(
            principal_spec(Some("alice"), false, None).unwrap(),
            "user:alice"
        );
        assert_eq!(principal_spec(None, true, None).unwrap(), "admin");
        assert_eq!(principal_spec(None, false, None).unwrap(), "admin");
        assert_eq!(
            principal_spec(None, false, Some("res1")).unwrap(),
            "reseller:res1"
        );
        assert!(principal_spec(Some("alice"), true, None).is_err());
        assert!(principal_spec(Some("../x"), false, None).is_err());
    }
}
