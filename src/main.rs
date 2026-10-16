//! WebPulse — lightweight, log-based web analytics for cPanel & WHM.
//! One static binary: collector, registry, rollup, reports, JSON API, dashboard, CGI.

pub mod app;
pub mod cli;
pub mod config;
pub mod engine;
pub mod error;
pub mod http;
pub mod log;
pub mod paths;
pub mod reports;
pub mod util;

fn main() {
    let code = cli::main(std::env::args_os().collect());
    std::process::exit(code);
}
