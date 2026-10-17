//! HTTP layer (R2): request/response, principal, API dispatch, page rendering,
//! dev server, CGI mode. See docs/API.md and docs/RUST-PORT.md.
//!
//! The JSON produced here must equal the PHP 0.1.0 implementation
//! (`lib/WebPulse/Http`) for the same databases: the dashboard (`ui/*.js`) is
//! shared and the parity harness (`tests/parity/api.sh`) diffs both.

pub mod admin_api;
pub mod admin_geoip;
pub mod api;
pub mod cpanel;
pub mod ctx;
pub mod dev_server;
pub mod job_runner;
pub mod json;
pub mod page;
pub mod principal;
pub mod registry;
pub mod request;
pub mod resolver;
pub mod response;
pub mod user_settings_api;

pub use api::ApiError;
pub use ctx::Ctx;
pub use principal::Principal;
pub use request::Request;
pub use response::Response;
