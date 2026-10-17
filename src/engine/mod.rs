//! Collector engine (R1): discovery, log sources, parser, classifiers, sessions,
//! counters, stores, registry, rollup. See docs/RUST-PORT.md and docs/METRICS.md.

pub mod classify;
pub mod collector;
pub mod counters;
pub mod discovery;
pub mod logsource;
pub mod parser;
pub mod registry;
pub mod rollup;
pub mod run;
pub mod session;
pub mod standalone;
pub mod store;
pub mod update;
pub mod user_settings;
