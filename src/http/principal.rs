//! Who is calling the API: a cPanel account (`user`), the WHM root
//! administrator (`admin`) or a reseller limited to the accounts it owns
//! (port of `Http\Principal`).

use serde_json::{Value, json};

/// Principal type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A cPanel account.
    User,
    /// The WHM root administrator.
    Admin,
    /// A WHM reseller.
    Reseller,
}

impl Kind {
    /// `user`, `admin` or `reseller`.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Admin => "admin",
            Kind::Reseller => "reseller",
        }
    }
}

/// The caller of the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Type.
    pub kind: Kind,
    /// Account / reseller name (`root` for the admin).
    pub name: String,
    /// Accounts owned by a reseller (`None` = look them up in the registry).
    pub owned_users: Option<Vec<String>>,
}

impl Principal {
    /// A cPanel account.
    pub fn user(name: &str) -> Principal {
        Principal {
            kind: Kind::User,
            name: name.into(),
            owned_users: None,
        }
    }
    /// The root administrator.
    pub fn admin() -> Principal {
        Principal {
            kind: Kind::Admin,
            name: "root".into(),
            owned_users: None,
        }
    }
    /// A reseller with the accounts it owns.
    pub fn reseller(name: &str, owned: Option<Vec<String>>) -> Principal {
        Principal {
            kind: Kind::Reseller,
            name: name.into(),
            owned_users: owned,
        }
    }
    /// Admin principal.
    pub fn is_admin(&self) -> bool {
        self.kind == Kind::Admin
    }
    /// Reseller principal.
    pub fn is_reseller(&self) -> bool {
        self.kind == Kind::Reseller
    }
    /// cPanel account principal.
    pub fn is_user(&self) -> bool {
        self.kind == Kind::User
    }
    /// Admin scope = admin or reseller.
    pub fn has_admin_scope(&self) -> bool {
        self.kind != Kind::User
    }
    /// True when the principal may see `user`'s data.
    pub fn can_see_user(&self, user: &str) -> bool {
        match self.kind {
            Kind::Admin => true,
            Kind::User => user == self.name,
            Kind::Reseller => {
                user == self.name
                    || self
                        .owned_users
                        .as_ref()
                        .is_some_and(|o| o.iter().any(|u| u == user))
            }
        }
    }
    /// `{"type": …, "name": …}`.
    pub fn to_json(&self) -> Value {
        json!({"type": self.kind.as_str(), "name": self.name})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes() {
        let u = Principal::user("alice");
        assert!(u.can_see_user("alice") && !u.can_see_user("bob") && !u.is_admin());
        let r = Principal::reseller("res", Some(vec!["bob".into()]));
        assert!(r.can_see_user("bob") && r.can_see_user("res") && !r.can_see_user("alice"));
        assert!(Principal::admin().can_see_user("anyone"));
        assert_eq!(
            Principal::admin().to_json().to_string(),
            r#"{"type":"admin","name":"root"}"#
        );
    }
}
