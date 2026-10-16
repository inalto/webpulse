//! The only place that decides which domains a principal may see (port of
//! `Http\DomainResolver`).
//!
//! * user     → `<data_dir>/<user>/*.db` (plus registry rows without data,
//!   when the registry is readable — it is not for cPanel accounts);
//! * reseller → the accounts it owns (and its own account);
//! * admin    → every account directory and every registry row.
//!
//! Domain and user names that do not match a strict pattern are rejected
//! before any filesystem access.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::api::ApiError;
use super::principal::Principal;
use super::registry::Registry;
use crate::paths::{Paths, is_valid_user};
use crate::reports::php::compare_str;

/// A domain visible to a principal.
#[derive(Debug, Clone)]
pub struct DomainRef {
    /// Owner account.
    pub user: String,
    /// Domain name.
    pub domain: String,
    /// main | addon | sub | parked | unknown
    pub kind: String,
    /// Effective registry switch (`None` without a registry row).
    pub enabled: Option<bool>,
    /// A database exists.
    pub has_data: bool,
    /// Database path.
    pub db_path: PathBuf,
}

impl DomainRef {
    /// Size of the database (0 without data).
    pub fn db_bytes(&self) -> u64 {
        if self.has_data {
            std::fs::metadata(&self.db_path)
                .map(|m| m.len())
                .unwrap_or(0)
        } else {
            0
        }
    }
}

/// `DomainResolver::validDomain()`: `^[a-z0-9.-]{1,253}\z`, no `..`, no leading
/// `.` or `-` (looser than `paths::is_valid_domain`, the engine's rule).
pub fn valid_domain(d: &str) -> bool {
    let b = d.as_bytes();
    !b.is_empty()
        && b.len() <= 253
        && !d.contains("..")
        && b[0] != b'.'
        && b[0] != b'-'
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
}

/// `<data_dir>/<user>/<domain>.db` for names already validated with
/// [`valid_user`] and [`valid_domain`] (neither can contain `/` or `..`).
pub fn db_path(paths: &Paths, user: &str, domain: &str) -> PathBuf {
    paths.data_dir().join(user).join(format!("{domain}.db"))
}

/// `DomainResolver::validUser()`.
pub fn valid_user(u: &str) -> bool {
    is_valid_user(u)
}

fn is_symlink(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// PHP `glob($dir . '/' . $pattern-ish)`: entries of `dir` not starting with
/// a dot, accepted by `keep`, sorted byte-wise.
fn glob(dir: &Path, keep: impl Fn(&str, &Path) -> bool) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok().map(|n| (n, e.path())))
            .filter(|(n, p)| !n.starts_with('.') && keep(n, p))
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    out
}

/// `glob($dir . '/*.db')`.
pub fn glob_db(dir: &Path) -> Vec<(String, PathBuf)> {
    glob(dir, |n, _| n.ends_with(".db"))
}

/// `glob($dir . '/*', GLOB_ONLYDIR)` (follows symlinks like glob does).
pub fn glob_dirs(dir: &Path) -> Vec<(String, PathBuf)> {
    glob(dir, |_, p| p.is_dir())
}

fn kind_rank(kind: &str) -> u8 {
    match kind {
        "main" => 0,
        "addon" => 1,
        "sub" => 2,
        "parked" => 3,
        _ => 4,
    }
}

fn row_str(m: &Map<String, Value>, k: &str) -> String {
    m.get(k)
        .and_then(crate::reports::php::scalar_string)
        .unwrap_or_default()
}

fn row_bool(m: &Map<String, Value>, k: &str) -> Option<bool> {
    match m.get(k) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::Number(n)) => Some(n.as_f64().unwrap_or(0.0) != 0.0),
        Some(Value::String(s)) => Some(s != "0"),
        Some(_) => Some(true),
    }
}

/// Registry domain rows of one account, keyed by domain (insertion order).
type RegRows = Vec<(String, Map<String, Value>)>;

/// Domains visible to the principal, sorted by user, kind, domain.
pub fn visible_domains(
    p: &Principal,
    registry: Option<&Registry>,
    paths: &Paths,
) -> Vec<DomainRef> {
    let users = visible_users(p, registry, paths);
    let mut reg_rows: HashMap<String, RegRows> = HashMap::new();
    if let Some(reg) = registry
        && let Ok(rows) = reg.domains(if p.is_user() {
            Some(p.name.as_str())
        } else {
            None
        })
    {
        for row in rows {
            let u = row_str(&row, "user");
            let removed = row
                .get("removed_at")
                .map(|v| !v.is_null() && v != &Value::from(0))
                .unwrap_or(false);
            if users.contains(&u) && !removed {
                let d = row_str(&row, "domain");
                let list = reg_rows.entry(u).or_default();
                match list.iter_mut().find(|(x, _)| *x == d) {
                    Some(e) => e.1 = row,
                    None => list.push((d, row)),
                }
            }
        }
    }
    let mut out = Vec::new();
    for user in &users {
        let mut seen: Vec<String> = Vec::new();
        let dir = paths.data_dir().join(user);
        let files = if dir.is_dir() && !is_symlink(&dir) {
            glob_db(&dir)
        } else {
            Vec::new()
        };
        let rows = reg_rows.get(user.as_str());
        let row_of = |d: &str| {
            rows.and_then(|r| r.iter().find(|(x, _)| x == d))
                .map(|(_, m)| m)
        };
        for (name, file) in files {
            let domain = name[..name.len() - 3].to_string();
            if !valid_domain(&domain) || is_symlink(&file) || !file.is_file() {
                continue;
            }
            let row = row_of(&domain);
            out.push(DomainRef {
                user: user.clone(),
                domain: domain.clone(),
                kind: row
                    .map(|m| row_str(m, "kind"))
                    .unwrap_or_else(|| "unknown".into()),
                enabled: row.and_then(|m| row_bool(m, "enabled")),
                has_data: true,
                db_path: file,
            });
            seen.push(domain);
        }
        for (domain, row) in rows.map(|r| r.as_slice()).unwrap_or(&[]) {
            if seen.contains(domain) || !valid_domain(domain) {
                continue;
            }
            out.push(DomainRef {
                user: user.clone(),
                domain: domain.clone(),
                kind: row_str(row, "kind"),
                enabled: row_bool(row, "enabled"),
                has_data: false,
                db_path: db_path(paths, user, domain),
            });
        }
    }
    out.sort_by(|a, b| {
        compare_str(&a.user, &b.user)
            .then(kind_rank(&a.kind).cmp(&kind_rank(&b.kind)))
            .then_with(|| compare_str(&a.domain, &b.domain))
    });
    out
}

/// Account names the principal may see, in PHP's order.
pub fn visible_users(p: &Principal, registry: Option<&Registry>, paths: &Paths) -> Vec<String> {
    if p.is_user() {
        return if valid_user(&p.name) {
            vec![p.name.clone()]
        } else {
            Vec::new()
        };
    }
    let mut users: Vec<String> = Vec::new();
    let add = |u: String, users: &mut Vec<String>| {
        if valid_user(&u) && !users.contains(&u) {
            users.push(u);
        }
    };
    if p.is_reseller() {
        let owned = match &p.owned_users {
            Some(o) => o.clone(),
            None => registry
                .and_then(|r| r.accounts(Some(&p.name), None).ok())
                .map(|rows| rows.iter().map(|m| row_str(m, "user")).collect())
                .unwrap_or_default(),
        };
        for u in owned.into_iter().chain(std::iter::once(p.name.clone())) {
            add(u, &mut users);
        }
        return users;
    }
    for (name, path) in glob_dirs(paths.data_dir()) {
        if valid_user(&name) && !is_symlink(&path) {
            add(name, &mut users);
        }
    }
    if let Some(reg) = registry
        && let Ok(rows) = reg.accounts(None, None)
    {
        for m in rows {
            add(row_str(&m, "user"), &mut users);
        }
    }
    users.sort_by(|a, b| compare_str(a, b));
    users
}

/// Resolves a `domain` (+ optional `user`) request parameter: 400 when
/// missing, 404 when malformed or not visible.
pub fn resolve(
    p: &Principal,
    registry: Option<&Registry>,
    paths: &Paths,
    domain: Option<&str>,
    user: Option<&str>,
) -> Result<DomainRef, ApiError> {
    let domain = match domain {
        Some(d) if !d.is_empty() => d.to_ascii_lowercase(),
        _ => return Err(ApiError::bad_request("Parameter \"domain\" is required")),
    };
    if !valid_domain(&domain) || domain.contains('/') {
        return Err(ApiError::not_found("Unknown domain"));
    }
    let user = user.filter(|u| !u.is_empty());
    if let Some(u) = user
        && (!valid_user(u) || !p.can_see_user(u))
    {
        return Err(ApiError::not_found("Unknown domain"));
    }
    if p.is_user() {
        let file = crate::reports::domain_data::domain_db_checked(paths, &p.name, &domain)
            .map_err(|e| ApiError::bad_request(&e.to_string()))?;
        if file.is_file() && !is_symlink(&file) {
            return Ok(DomainRef {
                user: p.name.clone(),
                domain,
                kind: "unknown".into(),
                enabled: None,
                has_data: true,
                db_path: file,
            });
        }
    }
    let mut found: Option<DomainRef> = None;
    for r in visible_domains(p, registry, paths) {
        if r.domain != domain || user.is_some_and(|u| r.user != u) {
            continue;
        }
        if found.is_none() || (r.has_data && !found.as_ref().is_some_and(|m| m.has_data)) {
            found = Some(r);
        }
    }
    found.ok_or_else(|| ApiError::not_found("Unknown domain"))
}

/// Comparator used by `admin.accounts` for its domain lists.
pub fn by_kind_then_domain(a: (&str, &str), b: (&str, &str)) -> Ordering {
    kind_rank(a.0)
        .cmp(&kind_rank(b.0))
        .then_with(|| compare_str(a.1, b.1))
}

/// Rank of a domain kind for sorting (main, addon, sub, parked, other).
pub fn rank(kind: &str) -> u8 {
    kind_rank(kind)
}
