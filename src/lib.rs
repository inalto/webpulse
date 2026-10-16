//! WebPulse — lightweight, log-based web analytics for cPanel & WHM.
//!
//! One static binary (`src/main.rs`): collector, registry, rollup, reports, JSON API,
//! dashboard, CGI. The library target exposes the modules to the integration tests.

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
