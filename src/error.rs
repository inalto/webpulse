//! Error types shared by the whole binary.
//!
//! Most code uses `anyhow::Result`; the variants below are the few errors that
//! callers need to recognise (exit codes, HTTP status mapping).

use thiserror::Error;

pub type Result<T> = anyhow::Result<T>;

#[derive(Debug, Error)]
pub enum WpError {
    /// Wrong command-line usage → exit code 2.
    #[error("{0}")]
    Usage(String),
    /// A domain database does not exist (→ HTTP 404 for the API).
    #[error("domain database not found: {0}")]
    NotFound(String),
    /// A read-only open met a schema newer than this binary knows.
    #[error("database schema version {found} is newer than supported {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
    /// Permission denied by the OS (→ HTTP 404 for the API: never leak existence).
    #[error("permission denied: {0}")]
    Denied(String),
}

/// Exit codes used by every subcommand.
pub mod exit {
    pub const OK: i32 = 0;
    pub const ERROR: i32 = 1;
    pub const USAGE: i32 = 2;
}
