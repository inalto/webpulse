//! `webpulse cgi (--principal user:NAME | --whm) (--api | --page) [--embedded] [--api-url URL]`
//!
//! CGI entry point used by the cPanel (PHP LiveAPI) and WHM (bash) shims
//! (docs/RUST-PORT.md). Reads the CGI environment (`REQUEST_METHOD`,
//! `QUERY_STRING`, `CONTENT_TYPE`, `CONTENT_LENGTH`, `HTTP_*` incl.
//! `HTTP_X_WEBPULSE_REQUEST`, `REMOTE_ADDR`, `REMOTE_USER`) and the body from
//! stdin, prints `Status: NNN`, headers, a blank line and the body.
//!
//! * `--principal user:NAME`: the binary runs as that account; it refuses to
//!   run when the effective uid is neither 0 nor NAME's uid.
//! * `--whm`: principal from `REMOTE_USER` (`root` → admin, otherwise a
//!   reseller owning at least one account, else 403); refuses euid ≠ 0.
//!
//! Development/test escape hatch: with `WEBPULSE_CGI_TESTING=1`, a
//! non-default configuration (`--config` / `WEBPULSE_CONFIG`; compared with
//! `/etc/webpulse/config.json` after resolving symlinks on both sides) and a
//! non-root euid, the uid checks are skipped (the OS permissions on the data remain
//! the real boundary; the production shims use the default configuration).

use std::collections::HashMap;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::app::App;
use crate::error::{Result, WpError};
use crate::http::page::{self, PageOptions, html_escape};
use crate::http::{Ctx, Principal, Request, Response, api, cpanel};
use crate::paths::is_valid_user;

/// Options of `webpulse cgi`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// cPanel principal (`user:NAME`), for the LiveAPI shims
    #[arg(long, value_name = "user:NAME", conflicts_with = "whm")]
    pub principal: Option<String>,
    /// WHM principal from REMOTE_USER (root or a reseller)
    #[arg(long)]
    pub whm: bool,
    /// Dispatch the JSON API
    #[arg(long, conflicts_with = "page")]
    pub api: bool,
    /// Render the dashboard page
    #[arg(long)]
    pub page: bool,
    /// Render an HTML fragment for the cPanel chrome (with --page)
    #[arg(long)]
    pub embedded: bool,
    /// API URL relative to the page (default api.live.php / api.cgi)
    #[arg(long, value_name = "URL")]
    pub api_url: Option<String>,
}

/// Action names that change state (`Whm\Edge::MUTATING`).
pub fn is_mutating(action: &str) -> bool {
    let a = action.strip_prefix("admin.").unwrap_or(action);
    a.starts_with("set_")
        || a.starts_with("save_")
        || a == "run"
        || a == "purge"
        || a == "update_check"
        || a.starts_with("delete")
        || a.starts_with("reset")
        || a.starts_with("enable")
        || a.starts_with("disable")
}

/// `Whm\Edge::mutationAllowed()`: anything but a plain GET/HEAD read needs
/// POST and `X-WebPulse-Request: 1`.
pub fn mutation_allowed(action: &str, method: &str, header: Option<&str>) -> bool {
    let m = method.to_ascii_uppercase();
    if !(m == "GET" || m == "HEAD") || is_mutating(action) {
        return m == "POST" && header == Some("1");
    }
    true
}

/// `Whm\Edge::principal()`: `root` → admin, a reseller owning accounts →
/// reseller, anything else → `None` (forbidden).
pub fn whm_principal(app: &App, remote_user: Option<&str>) -> Option<Principal> {
    let who = remote_user.unwrap_or("");
    if who.is_empty() {
        return None;
    }
    if who == "root" {
        return Some(Principal::admin());
    }
    if !is_valid_user(who) {
        return None;
    }
    let owned = cpanel::owned_by(&app.paths, who);
    if owned.is_empty() {
        return None;
    }
    Some(Principal::reseller(who, Some(owned)))
}

fn euid() -> u32 {
    // SAFETY: geteuid() has no preconditions.
    unsafe { libc::geteuid() }
}

/// uid of a system account, `None` when unknown.
fn uid_of(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 16384];
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: all pointers are valid for the call; getpwnam_r is re-entrant.
    let rc =
        unsafe { libc::getpwnam_r(c.as_ptr(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut out) };
    if rc != 0 || out.is_null() {
        return None;
    }
    Some(pwd.pw_uid)
}

fn testing(app: &App) -> bool {
    testing_allowed(
        std::env::var("WEBPULSE_CGI_TESTING").ok().as_deref(),
        euid() == 0,
        app.config_path(),
    )
}

/// Policy of the `WEBPULSE_CGI_TESTING` escape hatch: the flag is `1`, the process
/// is not root and the configuration is not the production file (compared after
/// resolving symlinks, see [`crate::app::is_production_config`]).
pub fn testing_allowed(flag: Option<&str>, root: bool, config: Option<&std::path::Path>) -> bool {
    flag == Some("1") && !root && config.is_some_and(|p| !crate::app::is_production_config(p))
}

fn denied_page(status: u16, title: &str, message: &str) -> Response {
    let t = html_escape(title);
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{t}</title></head><body><h2>{t}</h2><p>{}</p></body></html>",
        html_escape(message)
    );
    Response::new(
        status,
        "text/html; charset=utf-8",
        body.into_bytes(),
        vec![("Cache-Control".into(), "no-store".into())],
    )
}

fn emit(resp: &Response) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&resp.cgi_bytes());
    let _ = out.flush();
}

/// Runs the CGI request.
pub fn run(app: &App, a: &Args) -> Result<i32> {
    if a.principal.is_none() == !a.whm {
        return Err(WpError::Usage(
            "exactly one of --principal user:NAME or --whm is required".into(),
        )
        .into());
    }
    if a.api == a.page {
        return Err(WpError::Usage("exactly one of --api or --page is required".into()).into());
    }
    let env: HashMap<String, String> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.to_string_lossy().into_owned())))
        .collect();
    let refuse = |why: &str| -> Result<i32> {
        app.log.warn(format!("cgi: {why}"));
        let resp = if a.api {
            Response::error(403, "forbidden", "Access denied.")
        } else {
            denied_page(
                403,
                "Access denied",
                "WebPulse Analytics is not available for this account.",
            )
        };
        emit(&resp);
        Ok(crate::error::exit::ERROR)
    };

    let principal = if let Some(spec) = &a.principal {
        let name = match spec.strip_prefix("user:") {
            Some(n) if is_valid_user(n) => n.to_string(),
            _ => {
                return Err(WpError::Usage(format!(
                    "--principal must be user:NAME with a valid account name, got '{spec}'"
                ))
                .into());
            }
        };
        let me = euid();
        if me != 0 && uid_of(&name) != Some(me) && !testing(app) {
            return refuse(&format!("refusing --principal user:{name} for euid {me}"));
        }
        Some(Principal::user(&name))
    } else {
        if euid() != 0 && !testing(app) {
            return refuse("refusing --whm: not running as root");
        }
        whm_principal(app, env.get("REMOTE_USER").map(String::as_str))
    };

    let resp = catch_unwind(AssertUnwindSafe(|| -> Response {
        let ctx = Ctx::new(app);
        if a.api {
            let Some(p) = &principal else {
                return Response::error(403, "forbidden", "Access denied.");
            };
            if a.whm {
                let action = crate::reports::Query::parse(
                    env.get("QUERY_STRING").map(String::as_str).unwrap_or(""),
                )
                .get("a")
                .unwrap_or("")
                .to_string();
                let method = env
                    .get("REQUEST_METHOD")
                    .map(String::as_str)
                    .filter(|m| !m.is_empty())
                    .unwrap_or("GET");
                if !mutation_allowed(
                    &action,
                    method,
                    env.get("HTTP_X_WEBPULSE_REQUEST").map(String::as_str),
                ) {
                    return Response::error(
                        405,
                        "method_not_allowed",
                        "This action requires POST and the X-WebPulse-Request: 1 header.",
                    );
                }
            }
            let req = Request::from_cgi(&env, &mut std::io::stdin().lock());
            return api::dispatch(&ctx, &req, p);
        }
        let Some(p) = &principal else {
            return denied_page(
                403,
                "Access denied",
                "WebPulse Analytics is available to the server administrator and to resellers that own accounts.",
            );
        };
        let default_url = if a.whm { "api.cgi" } else { "api.live.php" };
        let opts = PageOptions::new(
            a.api_url.as_deref().unwrap_or(default_url),
            a.embedded,
            None,
        );
        let html = page::render(&ctx, p, &opts);
        Response::new(
            200,
            "text/html; charset=utf-8",
            html.into_bytes(),
            vec![("Cache-Control".into(), "no-store".into())],
        )
    }));
    let resp = match resp {
        Ok(r) => r,
        Err(_) => {
            app.log.error("cgi: internal error (panic)");
            if a.api {
                Response::error(500, "internal_error", "Internal error")
            } else {
                denied_page(
                    500,
                    "WebPulse Analytics is not available",
                    "The dashboard could not be loaded. See the error log for details.",
                )
            }
        }
    };
    emit(&resp);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_rules() {
        assert!(
            is_mutating("admin.set_account")
                && is_mutating("save_user_settings")
                && is_mutating("admin.run")
                && is_mutating("admin.update_check")
        );
        assert!(
            !is_mutating("admin.jobs")
                && !is_mutating("overview")
                && !is_mutating("admin.running")
                && !is_mutating("admin.update")
        );
        assert!(mutation_allowed("overview", "GET", None));
        assert!(!mutation_allowed("admin.run", "GET", None));
        assert!(!mutation_allowed("admin.run", "POST", None));
        assert!(mutation_allowed("admin.run", "post", Some("1")));
        assert!(!mutation_allowed("overview", "PUT", Some("1")));
    }
}
