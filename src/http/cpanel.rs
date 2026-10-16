//! The parts of cPanel account discovery the HTTP layer needs: account owners
//! (`/var/cpanel/users/<u>` `OWNER=`) and the domains cPanel lists for an
//! account. Thin wrappers over `engine::discovery::CpanelUsers` (port of
//! `Discovery\CpanelUsers`), so the API and the collector agree on ownership.

use std::collections::BTreeSet;

use crate::engine::discovery::CpanelUsers;
use crate::paths::Paths;

pub use crate::reports::domain_data::is_strict_domain;

/// Reseller owning the account (`OWNER=`), or `None`.
pub fn owner(paths: &Paths, user: &str) -> Option<String> {
    CpanelUsers::new(paths).owner(user)
}

/// Every account with its owner, sorted by name (`CpanelUsers::all()`).
pub fn all_owners(paths: &Paths) -> Vec<(String, Option<String>)> {
    CpanelUsers::new(paths)
        .all()
        .into_iter()
        .map(|(u, i)| (u, i.owner))
        .collect()
}

/// Accounts whose owner is `reseller`, sorted (`Whm\Edge::principal`).
pub fn owned_by(paths: &Paths, reseller: &str) -> Vec<String> {
    all_owners(paths)
        .into_iter()
        .filter(|(_, o)| o.as_deref() == Some(reseller))
        .map(|(u, _)| u)
        .collect()
}

/// Domains cPanel lists for an account (`CpanelUsers::domainsOf()` keys).
pub fn domains_of(paths: &Paths, user: &str) -> BTreeSet<String> {
    CpanelUsers::new(paths)
        .domains_of(user)
        .into_keys()
        .collect()
}
