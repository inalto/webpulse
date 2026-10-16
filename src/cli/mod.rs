//! Command-line definition. Each subcommand's `Args` struct and `run` function live in
//! the subcommand's own module; this file only wires them together.

use clap::{Parser, Subcommand};

use crate::app::{App, Global};
use crate::error::{exit, Result, WpError};

pub mod accounts;
pub mod backfill;
pub mod cgi;
pub mod collect;
pub mod config_cmd;
pub mod dev;
pub mod domains;
pub mod geoip_update;
pub mod hooked;
pub mod purge;
pub mod report;
pub mod rollup;
pub mod serve;
pub mod setup;
pub mod status;

#[derive(Debug, Parser)]
#[command(name = "webpulse", version, about = "WebPulse — log-based web analytics for cPanel & WHM",
          after_help = "Run \"webpulse <command> --help\" for the options of a command.")]
pub struct Cli {
    /// Configuration file (default: $WEBPULSE_CONFIG or /etc/webpulse/config.json)
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<std::path::PathBuf>,
    /// Log debug messages and mirror the log to stderr
    #[arg(long, global = true)]
    pub verbose: bool,
    /// Suppress informational output on stdout
    #[arg(long, global = true)]
    pub quiet: bool,
    /// Machine-readable JSON output
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Tail the access logs and update the statistics (cron runs this)
    Collect(collect::Args),
    /// Import archived logs (/home/<user>/logs/*.gz), never twice
    Backfill(backfill::Args),
    /// Nightly compaction: close visits, cap dimensions, retention, vacuum
    Rollup(rollup::Args),
    /// Collection health: paths, accounts, lag, last jobs, GeoIP
    Status(status::Args),
    /// List accounts or switch collection per account
    Accounts(accounts::Args),
    /// List domains or switch collection per domain
    Domains(domains::Args),
    /// Show the effective configuration or change a run-time setting
    #[command(name = "config")]
    ConfigCmd(config_cmd::Args),
    /// Create config, directories and registry with the right modes (idempotent)
    Setup(setup::Args),
    /// Download the free DB-IP country database (when geoip.enabled)
    GeoipUpdate(geoip_update::Args),
    /// Delete the statistics of an account or domain (irreversible)
    Purge(purge::Args),
    /// Drain the accounts flagged by the Stats::RunUser hook (= collect --hooked)
    Hooked(hooked::Args),
    /// Developer tools (gen-logs, …)
    #[command(hide = true)]
    Dev(dev::Args),
    /// Print the overview or one report of a domain (table or --json)
    Report(report::Args),
    /// Local dev dashboard on http://127.0.0.1:<port> (no cPanel needed)
    Serve(serve::Args),
    /// CGI entry point used by the cPanel and WHM shims
    Cgi(cgi::Args),
}

/// Parses `argv`, boots the app and runs the command; returns the process exit code.
pub fn main(argv: Vec<std::ffi::OsString>) -> i32 {
    let cli = match Cli::try_parse_from(argv) {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() { exit::USAGE } else { exit::OK };
        }
    };
    let global = Global { config: cli.config.clone(), verbose: cli.verbose, quiet: cli.quiet, json: cli.json };
    let app = App::boot(global);
    match dispatch(&app, &cli.command) {
        Ok(code) => code,
        Err(err) => {
            if let Some(WpError::Usage(m)) = err.downcast_ref::<WpError>() {
                eprintln!("webpulse: {m}");
                return exit::USAGE;
            }
            app.log.error(format!("{err:#}"));
            if app.global.json {
                println!("{}", serde_json::json!({"ok": false, "error": format!("{err:#}")}));
            } else {
                eprintln!("webpulse: {err:#}");
            }
            exit::ERROR
        }
    }
}

fn dispatch(app: &App, cmd: &Command) -> Result<i32> {
    match cmd {
        Command::Collect(a) => collect::run(app, a),
        Command::Backfill(a) => backfill::run(app, a),
        Command::Rollup(a) => rollup::run(app, a),
        Command::Status(a) => status::run(app, a),
        Command::Accounts(a) => accounts::run(app, a),
        Command::Domains(a) => domains::run(app, a),
        Command::ConfigCmd(a) => config_cmd::run(app, a),
        Command::Setup(a) => setup::run(app, a),
        Command::GeoipUpdate(a) => geoip_update::run(app, a),
        Command::Purge(a) => purge::run(app, a),
        Command::Hooked(a) => hooked::run(app, a),
        Command::Dev(a) => dev::run(app, a),
        Command::Report(a) => report::run(app, a),
        Command::Serve(a) => serve::run(app, a),
        Command::Cgi(a) => cgi::run(app, a),
    }
}
