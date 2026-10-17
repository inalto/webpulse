//! `webpulse serve [--listen ADDR:PORT | --port 8099] [--token-file PATH] [--allow-host HOST]
//! (--user U | --admin | --reseller R) [--allow-root]`: the dashboard over plain
//! HTTP (see `http::dev_server`). Development by default; with `--token-file` and
//! a reverse proxy in front (TLS) it is the standalone servers' web view.

use crate::app::App;
use crate::error::{Result, WpError};
use crate::paths::is_valid_user;

/// Options of `webpulse serve`.
#[derive(Debug, clap::Args)]
#[command(
    after_help = "The dashboard is served at / and the API at /api, acting as the given principal. \
The server answers only requests carrying the access token (http://ADDR:PORT/?token=…; the page then keeps it \
in a cookie): a random one printed at start, or the persistent one of --token-file. Stop it with Ctrl-C."
)]
pub struct Args {
    /// TCP port on 127.0.0.1 (same as --listen 127.0.0.1:PORT)
    #[arg(long, conflicts_with = "listen")]
    pub port: Option<String>,
    /// Address and port to listen on (default 127.0.0.1:8099)
    #[arg(long, value_name = "ADDR:PORT")]
    pub listen: Option<String>,
    /// Persistent access token (created with mode 0600 when missing)
    #[arg(long, value_name = "PATH")]
    pub token_file: Option<std::path::PathBuf>,
    /// Also accept this Host header (name or name:port of a reverse proxy; repeatable)
    #[arg(long, value_name = "HOST")]
    pub allow_host: Vec<String>,
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

fn port_of(s: &str, what: &str) -> Result<u16> {
    match s.parse::<u32>() {
        Ok(p) if s.bytes().all(|c| c.is_ascii_digit()) && (1..=65535).contains(&p) => Ok(p as u16),
        _ => Err(WpError::Usage(format!("{what} must be a number between 1 and 65535")).into()),
    }
}

/// `--listen ADDR:PORT` (`[v6]:PORT` for IPv6) or `--port PORT` → socket address.
pub fn listen_addr(listen: Option<&str>, port: Option<&str>) -> Result<std::net::SocketAddr> {
    if let Some(p) = port {
        let port = port_of(p, "--port")?;
        return Ok(std::net::SocketAddr::from(([127, 0, 0, 1], port)));
    }
    let l = listen.unwrap_or("127.0.0.1:8099");
    let bad = || {
        WpError::Usage(format!(
            "--listen must be ADDR:PORT (e.g. 127.0.0.1:8099), got '{l}'"
        ))
    };
    let (host, p) = l.rsplit_once(':').ok_or_else(bad)?;
    let port = port_of(p, "the --listen port")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let ip: std::net::IpAddr = match host {
        "localhost" => std::net::IpAddr::from([127, 0, 0, 1]),
        h => h.parse().map_err(|_| bad())?,
    };
    Ok(std::net::SocketAddr::new(ip, port))
}

/// Runs the server.
pub fn run(app: &App, a: &Args) -> Result<i32> {
    let addr = listen_addr(a.listen.as_deref(), a.port.as_deref())?;
    for h in &a.allow_host {
        if h.is_empty()
            || h.len() > 255
            || !h
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:[]_".contains(&b))
        {
            return Err(WpError::Usage(format!("invalid --allow-host '{h}'")).into());
        }
    }
    let spec = principal_spec(a.user.as_deref(), a.admin, a.reseller.as_deref())?;
    if app.is_root() && !a.allow_root {
        eprintln!("webpulse serve: refusing to run as root (use --allow-root to override)");
        return Ok(crate::error::exit::ERROR);
    }
    crate::http::dev_server::principal_from_spec(app, &spec)?;
    let token = match &a.token_file {
        Some(f) => crate::http::dev_server::Token::from_file(f)?,
        None => crate::http::dev_server::Token::random(),
    };
    crate::http::dev_server::run(app, addr, &spec, &token, &a.allow_host)
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
        assert_eq!(
            listen_addr(None, None).unwrap().to_string(),
            "127.0.0.1:8099"
        );
        assert_eq!(
            listen_addr(Some("0.0.0.0:9000"), None).unwrap().to_string(),
            "0.0.0.0:9000"
        );
        assert_eq!(
            listen_addr(Some("[::1]:8099"), None).unwrap().to_string(),
            "[::1]:8099"
        );
        assert_eq!(
            listen_addr(None, Some("8100")).unwrap().to_string(),
            "127.0.0.1:8100"
        );
        assert!(listen_addr(Some("8099"), None).is_err());
        assert!(listen_addr(Some("host.example:8099"), None).is_err());
        assert!(listen_addr(Some("127.0.0.1:0"), None).is_err());
    }
}
