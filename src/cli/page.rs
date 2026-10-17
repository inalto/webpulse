//! `webpulse page --cockpit [--bin PATH]`: prints the `index.html` of the Cockpit
//! module (`/usr/share/cockpit/webpulse/index.html`, written by
//! `install.sh --cockpit`). See `http::page::render_cockpit`.

use std::io::Write;

use crate::app::App;
use crate::error::{Result, WpError};
use crate::http::page;

/// `--help` text.
pub const USAGE: &str = "Usage: webpulse page --cockpit [--bin PATH]

Prints the dashboard document of the Cockpit module on stdout (the installer
writes it to /usr/share/cockpit/webpulse/index.html). The page runs
`PATH cgi --cockpit --api` through cockpit.spawn for every API call
(default PATH: /usr/local/webpulse/bin/webpulse).";

/// Options of `webpulse page`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The Cockpit module document
    #[arg(long)]
    pub cockpit: bool,
    /// Binary the page runs (default /usr/local/webpulse/bin/webpulse)
    #[arg(long, value_name = "PATH")]
    pub bin: Option<String>,
}

/// Prints the document.
pub fn run(_app: &App, a: &Args) -> Result<i32> {
    if !a.cockpit {
        return Err(WpError::Usage("only --cockpit is supported".into()).into());
    }
    let bin = a.bin.as_deref().unwrap_or(page::COCKPIT_BIN);
    if !bin.starts_with('/') || bin.chars().any(|c| c.is_control() || c == '"' || c == '<') {
        return Err(WpError::Usage(format!("--bin must be an absolute path, got '{bin}'")).into());
    }
    let html = page::render_cockpit(bin);
    let mut out = std::io::stdout().lock();
    out.write_all(html.as_bytes())?;
    out.flush()?;
    Ok(0)
}
