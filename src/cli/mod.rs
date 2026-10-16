//! Command line (`WebPulse\Cli\Cli`): the 0.1.0 dispatcher, argument parser, output
//! helpers and job bookkeeping, reproduced exactly (texts, `--json` shapes, exit
//! codes 0 ok / 1 error / 2 usage).
//!
//! Global flags are accepted anywhere on the line: `--config <file>`,
//! `--verbose|-v`, `--quiet|-q`, `--json`, `--help|-h`, `--version`. The R1 commands
//! parse their options with the 0.1.0 rules (`--name value`, `--name=value`,
//! `--flag`, optional values consuming only a numeric next token, `--` ends
//! options); `report`, `serve` and `cgi` (R2) are parsed with clap from their
//! `Args` structs.

use std::cell::OnceCell;
use std::ffi::OsString;

use clap::{FromArgMatches, Parser, Subcommand};
use indexmap::IndexMap;
use serde_json::Value;

use crate::app::{App, Global};
use crate::error::{Result, WpError, exit};
use crate::util::json as pjson;

pub mod accounts;
pub mod backfill;
pub mod cgi;
pub mod collect;
pub mod config_cmd;
pub mod dev;
pub mod domains;
pub mod geoip_update;
pub mod hooked;
pub mod meridian_registry;
pub mod purge;
pub mod report;
pub mod rollup;
pub mod serve;
pub mod setup;
pub mod status;

/// clap description of the command set (documentation; the dispatcher is
/// [`main`], which follows the 0.1.0 parsing rules).
#[derive(Debug, Parser)]
#[command(
    name = "webpulse",
    version,
    about = "WebPulse — log-based web analytics for cPanel & WHM"
)]
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
    /// The command.
    #[command(subcommand)]
    pub command: Command,
}

/// Every subcommand (see [`Cli`]).
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
    /// Add or remove the WebPulse hub in the Meridian side-menu registry
    #[command(name = "meridian-registry")]
    MeridianRegistry(meridian_registry::Args),
}

// ------------------------------------------------------------------ arguments

/// Kind of a command option (`Args::VALUE`, `FLAG`, `OPTIONAL` of 0.1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Requires a value (`--name value` or `--name=value`).
    Value,
    /// Boolean flag.
    Flag,
    /// Optional value: consumes the next token only when it is all digits.
    Optional,
}

/// Value of a parsed option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptVal {
    /// Given with a value.
    Str(String),
    /// Flag, or optional option without value.
    True,
}

/// Parsed arguments of one sub-command (`WebPulse\Cli\Args`).
#[derive(Debug, Clone, Default)]
pub struct RawArgs {
    positional: Vec<String>,
    options: IndexMap<String, OptVal>,
}

impl RawArgs {
    /// Parses `argv` (without the command name) against `spec`; the error is the
    /// usage message of 0.1.0 (`unknown option --x`…).
    pub fn parse(argv: &[String], spec: &[(&str, Kind)]) -> std::result::Result<RawArgs, String> {
        let mut out = RawArgs::default();
        let mut i = 0;
        while i < argv.len() {
            let a = &argv[i];
            if a == "--" {
                out.positional.extend(argv[i + 1..].iter().cloned());
                break;
            }
            if !a.starts_with("--") || a == "-" {
                out.positional.push(a.clone());
                i += 1;
                continue;
            }
            let body = &a[2..];
            let (name, value) = match body.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (body.to_string(), None),
            };
            let kind = spec
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, k)| *k)
                .ok_or_else(|| format!("unknown option --{name}"))?;
            match kind {
                Kind::Flag => {
                    if value.is_some() {
                        return Err(format!("option --{name} takes no value"));
                    }
                    out.options.insert(name, OptVal::True);
                }
                Kind::Value => {
                    let v = match value {
                        Some(v) => v,
                        None => {
                            if i + 1 >= argv.len() {
                                return Err(format!("option --{name} requires a value"));
                            }
                            i += 1;
                            argv[i].clone()
                        }
                    };
                    out.options.insert(name, OptVal::Str(v));
                }
                Kind::Optional => {
                    let mut v = value;
                    if v.is_none()
                        && i + 1 < argv.len()
                        && !argv[i + 1].is_empty()
                        && argv[i + 1].bytes().all(|b| b.is_ascii_digit())
                    {
                        i += 1;
                        v = Some(argv[i].clone());
                    }
                    out.options
                        .insert(name, v.map(OptVal::Str).unwrap_or(OptVal::True));
                }
            }
            i += 1;
        }
        Ok(out)
    }

    /// Positional arguments.
    pub fn positional(&self) -> &[String] {
        &self.positional
    }
    /// Positional argument `i`.
    pub fn arg(&self, i: usize) -> Option<&str> {
        self.positional.get(i).map(String::as_str)
    }
    /// True when the option was given (with or without value).
    pub fn has(&self, name: &str) -> bool {
        self.options.contains_key(name)
    }
    /// True for a given flag.
    pub fn flag(&self, name: &str) -> bool {
        self.options.contains_key(name)
    }
    /// String value of an option (`None` when absent or given without value).
    pub fn opt(&self, name: &str) -> Option<&str> {
        match self.options.get(name) {
            Some(OptVal::Str(s)) => Some(s),
            _ => None,
        }
    }
    /// Raw option value.
    pub fn get(&self, name: &str) -> Option<&OptVal> {
        self.options.get(name)
    }
}

/// A usage error (exit code 2, usage text appended).
pub fn usage(msg: impl Into<String>) -> anyhow::Error {
    WpError::Usage(msg.into()).into()
}

// --------------------------------------------------------------------- context

/// Per-invocation context of the R1 commands: global flags, lazily booted app,
/// output helpers.
pub struct Ctx {
    /// Global flags.
    pub global: Global,
    /// Command name.
    pub name: String,
    app: OnceCell<App>,
}

impl std::fmt::Debug for Ctx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx").field("name", &self.name).finish()
    }
}

impl Ctx {
    /// A context for `name` with the global flags.
    pub fn new(global: Global, name: &str) -> Ctx {
        Ctx {
            global,
            name: name.to_string(),
            app: OnceCell::new(),
        }
    }

    /// The application (booted on first use with `--config` / `$WEBPULSE_CONFIG`).
    pub fn app(&self) -> Result<&App> {
        if let Some(a) = self.app.get() {
            return Ok(a);
        }
        let app = App::try_boot(self.global.clone())?;
        app.log.set_verbose(self.global.verbose);
        app.log
            .set_stderr_errors(!self.global.quiet && !self.global.verbose);
        warn_php_bin(&app);
        let _ = self.app.set(app);
        self.app
            .get()
            .ok_or_else(|| anyhow::anyhow!("application unavailable"))
    }

    /// The app when it was booted already.
    pub fn booted(&self) -> Option<&App> {
        self.app.get()
    }

    /// True with `--json`.
    pub fn json(&self) -> bool {
        self.global.json
    }
    /// True with `--quiet`.
    pub fn quiet(&self) -> bool {
        self.global.quiet
    }
    /// True with `--verbose`.
    pub fn verbose(&self) -> bool {
        self.global.verbose
    }

    /// Informational output (suppressed by `--quiet`).
    pub fn out(&self, msg: &str) {
        out(&self.global, msg)
    }
    /// Notice (suppressed by `--quiet` and `--json`).
    pub fn info(&self, msg: &str) {
        if !self.global.quiet && !self.global.json {
            println!("{msg}");
        }
    }
    /// Error output (always, stderr).
    pub fn err(&self, msg: &str) {
        eprintln!("{msg}");
    }
    /// Prints a result: JSON with `--json` (even with `--quiet`), else the human
    /// rendering (suppressed by `--quiet`).
    pub fn result(&self, data: &Value, human: impl FnOnce() -> String) {
        result(&self.global, data, human)
    }

    /// `<run_dir>/<name>.lock`
    pub fn lock_path(&self, name: &str) -> Result<std::path::PathBuf> {
        Ok(self.app()?.paths.lock_path(name))
    }

    /// Takes the per-user lock (waiting up to `wait` seconds); `None` when busy.
    pub fn user_lock(&self, user: &str, wait: f64) -> Result<Option<crate::util::lock::Lock>> {
        crate::util::lock::Lock::acquire(&self.lock_path(&format!("user-{user}"))?, wait)
    }
}

/// `config.json` `php_bin` is ignored since the Rust port: warned about once.
fn warn_php_bin(app: &App) {
    let set = matches!(app.config.get("php_bin"), Value::String(s) if !s.is_empty());
    if !set {
        return;
    }
    let marker = app.paths.run_dir().join(".php_bin_warned");
    if marker.exists() || !app.paths.run_dir().is_dir() {
        return;
    }
    app.log
        .warn("config.json: php_bin is ignored since 0.2.0 (webpulse is a native binary)");
    let _ = std::fs::write(&marker, b"");
}

/// Informational output (suppressed by `--quiet`).
pub fn out(g: &Global, msg: &str) {
    if !g.quiet {
        println!("{msg}");
    }
}

/// Prints a result: pretty JSON with `--json`, else the human rendering unless `--quiet`.
pub fn result(g: &Global, data: &Value, human: impl FnOnce() -> String) {
    if g.json {
        println!("{}", pjson::encode_pretty(data));
        return;
    }
    if g.quiet {
        return;
    }
    println!("{}", human());
}

/// Renders rows as an aligned text table (`Cli::table`): booleans as yes/no, null
/// as `-`, columns padded, trailing spaces trimmed.
pub fn table(headers: &[&str], rows: &[Vec<Value>]) -> String {
    let mut w: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            r.iter()
                .enumerate()
                .map(|(i, c)| {
                    let s = match c {
                        Value::Bool(b) => if *b { "yes" } else { "no" }.to_string(),
                        Value::Null => "-".to_string(),
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        other => pjson::encode(other),
                    };
                    if i >= w.len() {
                        w.push(0);
                    }
                    w[i] = w[i].max(s.len());
                    s
                })
                .collect()
        })
        .collect();
    let fmt = |line: &[String]| -> String {
        let parts: Vec<String> = line
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{s:<width$}", width = w[i]))
            .collect();
        parts.join("  ").trim_end().to_string()
    };
    let mut out = vec![
        fmt(&headers.iter().map(|h| h.to_string()).collect::<Vec<_>>()),
        fmt(&w.iter().map(|x| "-".repeat(*x)).collect::<Vec<_>>()),
    ];
    for l in &cells {
        out.push(fmt(l));
    }
    out.join("\n")
}

// ------------------------------------------------------------------------ jobs

/// Bookkeeping of `registry.jobs` for long-running commands (`WebPulse\Cli\Job`):
/// `--job` creates a row now and finishes it at the end; `--job <id>` adopts a row
/// created by the WHM UI; without `--job` a row is written at the end only when
/// `record` is set.
#[derive(Debug)]
pub struct Job<'a> {
    ctx: &'a Ctx,
    kind: String,
    scope: Option<String>,
    id: Option<i64>,
    started_at: i64,
}

impl<'a> Job<'a> {
    /// Creates or adopts the job row when `--job` is present.
    pub fn new(ctx: &'a Ctx, kind: &str, scope: Option<&str>, args: &RawArgs) -> Result<Job<'a>> {
        let app = ctx.app()?;
        let mut job = Job {
            ctx,
            kind: kind.to_string(),
            scope: scope.map(str::to_string),
            id: None,
            started_at: app.now(),
        };
        if !args.has("job") {
            return Ok(job);
        }
        let reg = app.registry()?;
        let adopt = args
            .opt("job")
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|v| v.parse::<i64>().ok());
        match adopt {
            Some(id) if reg.job(id)?.is_some() => {
                reg.touch_job(id)?;
                job.id = Some(id);
            }
            _ => job.id = Some(reg.start_job(kind, scope, None)?),
        }
        Ok(job)
    }

    /// Job id (`None` when no row was written yet).
    pub fn id(&self) -> Option<i64> {
        self.id
    }

    /// Finishes the job (`record`: write a row even without `--job`).
    pub fn finish(
        &mut self,
        status: &str,
        lines: i64,
        files: i64,
        domains: i64,
        message: Option<&str>,
        record: bool,
    ) {
        let Ok(app) = self.ctx.app() else {
            return;
        };
        let r = (|| -> Result<()> {
            let reg = app.registry()?;
            let id = match self.id {
                Some(id) => id,
                None => {
                    if !record {
                        return Ok(());
                    }
                    let id =
                        reg.start_job(&self.kind, self.scope.as_deref(), Some(self.started_at))?;
                    self.id = Some(id);
                    id
                }
            };
            reg.finish_job(id, status, lines, files, domains, message)
        })();
        if let Err(e) = r {
            app.log.warn(format!("cannot record job: {e}"));
        }
    }
}

// --------------------------------------------------------------------- commands

type R1Run = fn(&Ctx, &RawArgs) -> Result<i32>;

enum Handler {
    R1(R1Run),
    R2,
}

struct CmdDef {
    name: &'static str,
    summary: &'static str,
    hidden: bool,
    spec: &'static [(&'static str, Kind)],
    handler: Handler,
}

const COMMANDS: &[CmdDef] = &[
    CmdDef {
        name: "accounts",
        summary: accounts::SUMMARY,
        hidden: false,
        spec: accounts::SPEC,
        handler: Handler::R1(accounts::main),
    },
    CmdDef {
        name: "backfill",
        summary: backfill::SUMMARY,
        hidden: false,
        spec: backfill::SPEC,
        handler: Handler::R1(backfill::main),
    },
    CmdDef {
        name: "cgi",
        summary: "CGI entry point used by the cPanel and WHM shims",
        hidden: false,
        spec: &[],
        handler: Handler::R2,
    },
    CmdDef {
        name: "collect",
        summary: collect::SUMMARY,
        hidden: false,
        spec: collect::SPEC,
        handler: Handler::R1(collect::main),
    },
    CmdDef {
        name: "config",
        summary: config_cmd::SUMMARY,
        hidden: false,
        spec: config_cmd::SPEC,
        handler: Handler::R1(config_cmd::main),
    },
    CmdDef {
        name: "dev",
        summary: dev::SUMMARY,
        hidden: true,
        spec: dev::SPEC,
        handler: Handler::R1(dev::main),
    },
    CmdDef {
        name: "domains",
        summary: domains::SUMMARY,
        hidden: false,
        spec: domains::SPEC,
        handler: Handler::R1(domains::main),
    },
    CmdDef {
        name: "geoip-update",
        summary: geoip_update::SUMMARY,
        hidden: false,
        spec: geoip_update::SPEC,
        handler: Handler::R1(geoip_update::main),
    },
    CmdDef {
        name: "hooked",
        summary: hooked::SUMMARY,
        hidden: false,
        spec: hooked::SPEC,
        handler: Handler::R1(hooked::main),
    },
    CmdDef {
        name: "purge",
        summary: purge::SUMMARY,
        hidden: false,
        spec: purge::SPEC,
        handler: Handler::R1(purge::main),
    },
    CmdDef {
        name: "meridian-registry",
        summary: meridian_registry::SUMMARY,
        hidden: false,
        spec: meridian_registry::SPEC,
        handler: Handler::R1(meridian_registry::main),
    },
    CmdDef {
        name: "report",
        summary: "print the overview or one report of a domain (table or --json)",
        hidden: false,
        spec: &[],
        handler: Handler::R2,
    },
    CmdDef {
        name: "rollup",
        summary: rollup::SUMMARY,
        hidden: false,
        spec: rollup::SPEC,
        handler: Handler::R1(rollup::main),
    },
    CmdDef {
        name: "serve",
        summary: "local dev dashboard on http://127.0.0.1:<port> (no cPanel needed)",
        hidden: false,
        spec: &[],
        handler: Handler::R2,
    },
    CmdDef {
        name: "setup",
        summary: setup::SUMMARY,
        hidden: false,
        spec: setup::SPEC,
        handler: Handler::R1(setup::main),
    },
    CmdDef {
        name: "status",
        summary: status::SUMMARY,
        hidden: false,
        spec: status::SPEC,
        handler: Handler::R1(status::main),
    },
];

/// Usage text of a command (`<command> --help`).
pub fn usage_of(name: &str) -> String {
    match name {
        "accounts" => accounts::USAGE.to_string(),
        "backfill" => backfill::USAGE.to_string(),
        "collect" => collect::USAGE.to_string(),
        "config" => config_cmd::usage(),
        "dev" => dev::USAGE.to_string(),
        "domains" => domains::USAGE.to_string(),
        "geoip-update" => geoip_update::USAGE.to_string(),
        "hooked" => hooked::USAGE.to_string(),
        "purge" => purge::USAGE.to_string(),
        "meridian-registry" => meridian_registry::USAGE.to_string(),
        "rollup" => rollup::USAGE.to_string(),
        "setup" => setup::USAGE.to_string(),
        "status" => status::USAGE.to_string(),
        "report" => REPORT_USAGE.to_string(),
        "serve" => SERVE_USAGE.to_string(),
        "cgi" => CGI_USAGE.to_string(),
        _ => String::new(),
    }
}

const REPORT_USAGE: &str = "Usage: webpulse report --user USER --domain DOMAIN [--range 30d | --from YYYY-MM-DD --to YYYY-MM-DD]
                       [--name DIMENSION] [--limit 10] [--sort -visits] [--compare] [--json]

Without --name prints the KPIs of the period (with the change versus the
previous period). With --name prints the ranked table of a dimension:
  page entry_page exit_page referrer_host referrer_url channel search_engine
  keyword campaign utm_source utm_medium source_medium country city browser
  browser_version os os_version device status method protocol http_version
  file_type static_file download not_found not_found_referrer server_error bot
  visit_duration pages_per_visit
  Ranges: today yesterday 7d 30d 90d 12m month lastmonth year all
";

const SERVE_USAGE: &str = "Usage: webpulse serve [--port 8099] (--user USER | --admin | --reseller NAME) [--config FILE] [--allow-root]

Serves the dashboard on 127.0.0.1:PORT (no cPanel needed). The dashboard
is served at / and the API at /api, acting as the given principal:
  --user USER       a cPanel account (sees its own domains)
  --admin           the WHM root administrator (default)
  --reseller NAME   a reseller (sees the accounts it owns)
  --allow-root      run even as root (refused by default)
The server only listens on 127.0.0.1 and answers only requests carrying
the random token of the printed URL (http://127.0.0.1:PORT/?token=…;
the page then keeps it in a cookie). Stop it with Ctrl-C.";

const CGI_USAGE: &str = "Usage: webpulse cgi (--principal user:NAME | --whm) (--api | --page) [--embedded] [--api-url URL]

CGI entry point of the cPanel and WHM shims: reads the CGI environment and
the request body, prints `Status:`, the headers, a blank line and the body.";

fn find(name: &str) -> Option<&'static CmdDef> {
    let b = name.as_bytes();
    let valid = !b.is_empty()
        && b.len() <= 41
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-');
    if !valid {
        return None;
    }
    COMMANDS.iter().find(|c| c.name == name)
}

fn help() -> String {
    let mut lines = vec![
        format!(
            "WebPulse {} — log-based web analytics for cPanel & WHM",
            App::version()
        ),
        String::new(),
        "Usage: webpulse <command> [options]".into(),
        String::new(),
        "Commands:".into(),
    ];
    for c in COMMANDS.iter().filter(|c| !c.hidden) {
        lines.push(format!("  {:<14} {}", c.name, c.summary));
    }
    lines.push(String::new());
    lines.push(
        "Global options: --config <file>  --verbose  --quiet  --json  --help  --version".into(),
    );
    lines.push("Run \"webpulse <command> --help\" for the options of a command.".into());
    lines.join("\n")
}

fn run_r2(name: &str, global: &Global, rest: &[String]) -> std::result::Result<i32, (i32, String)> {
    fn parse<A: clap::Args + FromArgMatches>(
        name: &'static str,
        rest: &[String],
    ) -> std::result::Result<A, String> {
        let cmd = A::augment_args(
            clap::Command::new(name)
                .no_binary_name(true)
                .disable_help_flag(true),
        );
        let m = cmd
            .try_get_matches_from(rest.iter())
            .map_err(|e| e.kind().to_string() + ": " + &first_line(&e.to_string()))?;
        A::from_arg_matches(&m).map_err(|e| first_line(&e.to_string()))
    }
    fn first_line(s: &str) -> String {
        s.lines()
            .next()
            .unwrap_or("")
            .trim_start_matches("error: ")
            .to_string()
    }
    let boot = || {
        App::try_boot(global.clone()).inspect(|a| {
            a.log.set_stderr_errors(!global.quiet && !global.verbose);
        })
    };
    let res = match name {
        "report" => {
            let a: report::Args = parse("report", rest).map_err(|m| (exit::USAGE, m))?;
            boot().and_then(|app| report::run(&app, &a))
        }
        "serve" => {
            let a: serve::Args = parse("serve", rest).map_err(|m| (exit::USAGE, m))?;
            boot().and_then(|app| serve::run(&app, &a))
        }
        _ => {
            let a: cgi::Args = parse("cgi", rest).map_err(|m| (exit::USAGE, m))?;
            boot().and_then(|app| cgi::run(&app, &a))
        }
    };
    match res {
        Ok(c) => Ok(c),
        Err(e) => match e.downcast_ref::<WpError>() {
            Some(WpError::Usage(m)) => Err((exit::USAGE, m.clone())),
            _ => Err((exit::ERROR, format!("{e:#}"))),
        },
    }
}

/// Parses `argv`, runs the command and returns the process exit code.
pub fn main(argv: Vec<OsString>) -> i32 {
    let args: Vec<String> = argv
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let mut global = Global::default();
    let mut rest: Vec<String> = Vec::new();
    let mut want_help = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            rest.extend(args[i..].iter().cloned());
            break;
        }
        match a.as_str() {
            "--config" => {
                if i + 1 >= args.len() {
                    eprintln!("option --config requires a value");
                    return exit::USAGE;
                }
                i += 1;
                global.config = Some(args[i].clone().into());
            }
            s if s.starts_with("--config=") => global.config = Some(s[9..].to_string().into()),
            "--verbose" | "-v" => global.verbose = true,
            "--quiet" | "-q" => global.quiet = true,
            "--json" => global.json = true,
            "--help" | "-h" => want_help = true,
            "--version" => {
                rest = vec!["version".into()];
                break;
            }
            _ => rest.push(a.clone()),
        }
        i += 1;
    }
    let name = rest.first().cloned();
    if name.is_none() || name.as_deref() == Some("help") {
        if let Some(t) = rest.get(1)
            && find(t).is_some()
        {
            out(&global, &usage_of(t));
            return exit::OK;
        }
        out(&global, &help());
        return if name.is_none() && !want_help {
            exit::USAGE
        } else {
            exit::OK
        };
    }
    let name = name.unwrap_or_default();
    if name == "version" {
        out(&global, &format!("webpulse {}", App::version()));
        return exit::OK;
    }
    let Some(def) = find(&name) else {
        eprintln!("unknown command '{name}'\n\n{}", help());
        return exit::USAGE;
    };
    if want_help {
        out(&global, &usage_of(&name));
        return exit::OK;
    }
    match &def.handler {
        Handler::R2 => match run_r2(def.name, &global, &rest[1..]) {
            Ok(c) => c,
            Err((code, msg)) => {
                if code == exit::USAGE {
                    eprintln!("{msg}\n\n{}", usage_of(&name));
                } else {
                    eprintln!("error: {}", crate::log::sanitize(&msg));
                }
                code
            }
        },
        Handler::R1(run) => {
            let raw = match RawArgs::parse(&rest[1..], def.spec) {
                Ok(r) => r,
                Err(m) => {
                    eprintln!("{m}\n\n{}", usage_of(&name));
                    return exit::USAGE;
                }
            };
            let ctx = Ctx::new(global.clone(), &name);
            match run(&ctx, &raw) {
                Ok(c) => c,
                Err(e) => {
                    if let Some(WpError::Usage(m)) = e.downcast_ref::<WpError>() {
                        eprintln!("{m}\n\n{}", usage_of(&name));
                        return exit::USAGE;
                    }
                    eprintln!("error: {}", crate::log::sanitize(&e.to_string()));
                    if let Some(app) = ctx.booted() {
                        app.log.error(format!("{name} failed: {e:#}"));
                    }
                    if global.verbose {
                        eprintln!("{e:?}");
                    }
                    exit::ERROR
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_like_php() {
        let spec = &[
            ("user", Kind::Value),
            ("job", Kind::Optional),
            ("yes", Kind::Flag),
        ];
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let a = RawArgs::parse(&s(&["--user", "alice", "--job", "12", "x"]), spec).unwrap();
        assert_eq!(a.opt("user"), Some("alice"));
        assert_eq!(a.opt("job"), Some("12"));
        assert_eq!(a.positional(), &["x".to_string()]);
        let a = RawArgs::parse(&s(&["--job", "x", "--yes"]), spec).unwrap();
        assert_eq!(a.get("job"), Some(&OptVal::True));
        assert!(a.flag("yes"));
        assert_eq!(
            RawArgs::parse(&s(&["--bogus"]), spec).unwrap_err(),
            "unknown option --bogus"
        );
        assert_eq!(
            RawArgs::parse(&s(&["--yes=1"]), spec).unwrap_err(),
            "option --yes takes no value"
        );
        assert_eq!(
            RawArgs::parse(&s(&["--user"]), spec).unwrap_err(),
            "option --user requires a value"
        );
        let a = RawArgs::parse(&s(&["--user=bob", "--", "--yes"]), spec).unwrap();
        assert_eq!(a.opt("user"), Some("bob"));
        assert_eq!(a.positional(), &["--yes".to_string()]);
    }

    #[test]
    fn tables() {
        let t = table(
            &["A", "BB"],
            &[
                vec![Value::Bool(true), Value::Null],
                vec![Value::from("long"), Value::from(3)],
            ],
        );
        assert_eq!(t, "A     BB\n----  --\nyes   -\nlong  3");
    }
}
