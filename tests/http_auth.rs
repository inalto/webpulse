//! Authorization matrix (port of tests/cases/ui_auth.php, the API parts of
//! sec_web.php and sec_tenant.php): who may see which domain (404
//! otherwise), the reseller scope, admin-only actions, hostile names,
//! tenant checks on admin.set_domain/run/purge and admin.run concurrency.

mod http_common;

use http_common::*;
use serde_json::{Value, json};
use webpulse::http::principal::Principal;
use webpulse::http::resolver::{valid_domain, valid_user};

const SEED: &str = "http-auth";

fn status(env: &Env, who: &str, q: &[(&str, &str)], body: Option<Value>) -> u16 {
    api(env, who, q, body).status
}

#[test]
fn domain_visibility_matrix() {
    let env = shared(SEED);
    let matrix: [(&str, &str, Option<&str>, u16); 16] = [
        ("user:alice", "example.com", None, 200),
        ("user:alice", "shop.example.com", None, 200),
        ("user:alice", "blog.net", None, 404),
        ("user:alice", "blog.net", Some("bob"), 404),
        ("user:bob", "blog.net", None, 200),
        ("user:bob", "example.com", None, 404),
        ("user:bob", "example.com", Some("alice"), 404),
        ("user:mallory", "example.com", None, 404),
        ("reseller:reseller1", "blog.net", None, 200),
        ("reseller:reseller1", "example.com", None, 404),
        ("reseller:reseller1", "example.com", Some("alice"), 404),
        ("reseller:nobody", "blog.net", None, 404),
        ("admin", "example.com", None, 200),
        ("admin", "blog.net", Some("bob"), 200),
        ("admin", "blog.net", Some("alice"), 404),
        ("admin", "nosuch.example", None, 404),
    ];
    for a in ["overview", "realtime", "export"] {
        for (p, d, u, exp) in matrix {
            let mut q = vec![("a", a), ("domain", d), ("name", "page")];
            if let Some(u) = u {
                q.push(("user", u));
            }
            assert_eq!(
                status(&env, p, &q, None),
                exp,
                "{a} as {p} on {d} (user={u:?})"
            );
        }
    }
}

#[test]
fn domains_list_per_principal() {
    let env = shared(SEED);
    let list = |p: &str| -> Vec<String> {
        api(&env, p, &[("a", "domains")], None).json["data"]["domains"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                format!(
                    "{}/{}",
                    d["user"].as_str().unwrap(),
                    d["domain"].as_str().unwrap()
                )
            })
            .collect()
    };
    assert_eq!(
        list("user:alice"),
        vec!["alice/example.com", "alice/shop.example.com"]
    );
    assert_eq!(list("user:bob"), vec!["bob/blog.net"]);
    assert_eq!(
        list("user:carol"),
        vec!["carol/carol.org"],
        "registered domain without data"
    );
    assert!(
        list("user:mallory").is_empty(),
        "unknown account sees nothing"
    );
    assert_eq!(
        list("reseller:reseller1"),
        vec!["bob/blog.net", "carol/carol.org"]
    );
    assert_eq!(
        list("admin"),
        vec![
            "alice/example.com",
            "alice/shop.example.com",
            "bob/blog.net",
            "carol/carol.org",
            "dave/dave.io"
        ]
    );
    let all = api(&env, "admin", &[("a", "domains")], None).json;
    let carol = all["data"]["domains"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["user"] == "carol")
        .unwrap()
        .clone();
    assert_eq!(carol["has_data"], false);
    // admin rows come from the registry summary: no per-database details
    let alice = &all["data"]["domains"][0];
    assert_eq!(
        (alice["kind"].clone(), alice["first_day"].clone()),
        (json!("main"), Value::Null)
    );
}

#[test]
fn hostile_domain_and_user_parameters() {
    let env = shared(SEED);
    let long = "a".repeat(300);
    for bad in [
        "../bob/blog.net",
        "blog.net/../../x",
        "..",
        ".",
        "/etc/passwd",
        "EXAMPLE.COM/..",
        "example.com\0",
        "exa mple.com",
        &long,
        "-rf",
        ".hidden",
        "example.com.db",
        "%2e%2e",
    ] {
        assert_eq!(
            status(
                &env,
                "user:alice",
                &[("a", "overview"), ("domain", bad)],
                None
            ),
            404,
            "domain {bad:?}"
        );
    }
    assert_eq!(
        status(
            &env,
            "user:alice",
            &[("a", "overview"), ("domain", "EXAMPLE.COM")],
            None
        ),
        200,
        "case-insensitive"
    );
    for u in ["../alice", "alice/..", "root;", ""] {
        let s = status(
            &env,
            "admin",
            &[("a", "overview"), ("domain", "example.com"), ("user", u)],
            None,
        );
        assert!(s == 200 || s == 404, "{u}: {s}");
    }
    assert_eq!(
        status(
            &env,
            "admin",
            &[
                ("a", "overview"),
                ("domain", "example.com"),
                ("user", "../alice")
            ],
            None
        ),
        404
    );
    assert!(!valid_domain("a..b") && !valid_domain("a/b") && valid_domain("xn--bcher-kva.example"));
    assert!(
        !valid_domain("example.com\n")
            && !valid_user("alice\n")
            && !valid_user("al.ice")
            && valid_user("alice_2")
    );
    assert!(!webpulse::paths::is_valid_user("alice\n"));
    assert!(
        !webpulse::reports::range::is_date("2026-10-16\n")
            && webpulse::reports::range::is_date("2026-10-16")
    );
    let r = api(
        &env,
        "user:alice",
        &[("a", "overview"), ("domain", "blog.net")],
        None,
    );
    assert_eq!(
        r.json["error"]["code"], "not_found",
        "same answer as a missing domain"
    );
    assert_eq!(
        status(
            &env,
            "admin",
            &[("a", "overview"), ("domain", "example.com\n")],
            None
        ),
        404
    );
}

#[test]
fn admin_scope_users_and_resellers() {
    let env = fresh(SEED, "http-auth-admin");
    for a in [
        "admin.overview",
        "admin.accounts",
        "admin.settings",
        "admin.jobs",
    ] {
        assert_eq!(
            status(&env, "user:alice", &[("a", a)], None),
            403,
            "{a} as user"
        );
        assert_eq!(
            status(&env, "reseller:reseller1", &[("a", a)], None),
            if a == "admin.settings" { 403 } else { 200 },
            "{a} as reseller"
        );
        assert_eq!(
            status(&env, "admin", &[("a", a)], None),
            200,
            "{a} as admin"
        );
    }
    let acc = api(&env, "reseller:reseller1", &[("a", "admin.accounts")], None).json;
    let users: Vec<&str> = acc["data"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["user"].as_str().unwrap())
        .collect();
    assert_eq!(
        users,
        vec!["bob", "carol"],
        "reseller only sees owned accounts"
    );
    let top = api(&env, "reseller:reseller1", &[("a", "admin.overview")], None).json;
    let doms: Vec<&str> = top["data"]["top_domains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["domain"].as_str().unwrap())
        .collect();
    assert_eq!(doms, vec!["blog.net"]);
    assert_eq!(
        status(
            &env,
            "reseller:reseller1",
            &[("a", "admin.set_account")],
            Some(json!({"user": "alice", "enabled": false}))
        ),
        404,
        "foreign account"
    );
    assert_eq!(
        status(
            &env,
            "reseller:reseller1",
            &[("a", "admin.set_account")],
            Some(json!({"user": "bob", "enabled": false}))
        ),
        200,
        "own account"
    );
    assert_eq!(
        status(
            &env,
            "reseller:reseller1",
            &[("a", "admin.set_domain")],
            Some(json!({"user": "alice", "domain": "example.com", "enabled": false}))
        ),
        404
    );
    assert_eq!(
        status(
            &env,
            "reseller:reseller1",
            &[("a", "admin.save_settings")],
            Some(json!({"settings": {"store_ip": true}}))
        ),
        403
    );
    assert_eq!(
        status(
            &env,
            "reseller:reseller1",
            &[("a", "admin.purge")],
            Some(json!({"user": "bob", "confirm": "DELETE"}))
        ),
        403
    );
    assert_eq!(
        status(
            &env,
            "reseller:reseller1",
            &[("a", "admin.run")],
            Some(json!({"type": "rollup"}))
        ),
        403,
        "rollup is root only"
    );
    assert_eq!(
        status(
            &env,
            "reseller:reseller1",
            &[("a", "admin.run")],
            Some(json!({"type": "collect"}))
        ),
        400,
        "pick an account"
    );
    assert_eq!(
        status(
            &env,
            "reseller:reseller1",
            &[("a", "admin.run")],
            Some(json!({"type": "collect", "user": "alice"}))
        ),
        404
    );
    assert_eq!(
        status(
            &env,
            "user:alice",
            &[("a", "admin.set_account")],
            Some(json!({"user": "alice", "enabled": true}))
        ),
        403,
        "user cannot toggle itself"
    );
    assert_eq!(
        api(
            &env,
            "user:alice",
            &[("a", "admin.purge")],
            Some(json!({"user": "alice", "confirm": "DELETE"}))
        )
        .json["error"]["code"],
        "forbidden"
    );
    let jobs = api(&env, "reseller:reseller1", &[("a", "admin.jobs")], None).json;
    for j in jobs["data"]["jobs"].as_array().unwrap() {
        let scope = j["scope"].as_str().unwrap_or("");
        assert!(
            ["all", "bob", "carol", "reseller1"].contains(&scope.split('/').next().unwrap()),
            "reseller jobs scope {scope}"
        );
    }
    assert_eq!(
        status(&env, "reseller:reseller1", &[("a", "admin.settings")], None),
        403,
        "resellers cannot read global settings"
    );
}

#[test]
fn principal_rules() {
    let u = Principal::user("alice");
    assert!(u.can_see_user("alice") && !u.can_see_user("bob") && !u.is_admin());
    let r = Principal::reseller("res", Some(vec!["bob".into()]));
    assert!(r.can_see_user("bob") && r.can_see_user("res") && !r.can_see_user("alice"));
    assert!(Principal::admin().can_see_user("anyone"));
}

#[test]
fn tenant_checks_on_set_domain_run_purge() {
    let env = fresh(SEED, "http-auth-tenant");
    let st = |who: &str, a: &str, body: Value| status(&env, who, &[("a", a)], Some(body));
    assert_eq!(
        st(
            "reseller:reseller1",
            "admin.set_domain",
            json!({"user": "bob", "domain": "example.com", "enabled": true})
        ),
        404,
        "foreign domain"
    );
    assert_eq!(
        st(
            "reseller:reseller1",
            "admin.set_domain",
            json!({"user": "bob", "domain": "victim.example", "enabled": true})
        ),
        404,
        "unknown"
    );
    assert_eq!(
        st(
            "admin",
            "admin.set_domain",
            json!({"user": "bob", "domain": "example.com", "enabled": true})
        ),
        404,
        "root cannot mis-attach"
    );
    let a = app(&env);
    let ctx = webpulse::http::Ctx::new(&a);
    let rows = ctx.registry().unwrap().domains(Some("bob")).unwrap();
    assert_eq!(
        rows.iter()
            .map(|d| d["domain"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["blog.net"],
        "no row created"
    );
    assert_eq!(
        st(
            "reseller:reseller1",
            "admin.set_domain",
            json!({"user": "bob", "domain": "blog.net", "enabled": false})
        ),
        200
    );
    assert_eq!(
        st(
            "admin",
            "admin.set_domain",
            json!({"user": "alice", "domain": "SHOP.example.com", "enabled": null})
        ),
        200,
        "case-insensitive"
    );
    assert_eq!(
        st(
            "reseller:reseller1",
            "admin.run",
            json!({"type": "collect", "user": "bob", "domain": "example.com"})
        ),
        404
    );
    assert_eq!(
        st(
            "admin",
            "admin.purge",
            json!({"user": "bob", "domain": "example.com", "confirm": "DELETE"})
        ),
        404
    );
    assert!(
        env.root
            .join("var/webpulse/data/alice/example.com.db")
            .exists()
    );
}

#[test]
fn admin_run_refuses_duplicates_and_too_many_jobs() {
    let env = fresh(SEED, "http-auth-run");
    let a = app(&env);
    let ctx = webpulse::http::Ctx::new(&a);
    let reg = ctx.registry().unwrap();
    let now = env.now;
    let start = |t: &str, scope: &str, at: i64| -> i64 {
        reg.conn()
            .execute(
                "INSERT INTO jobs (type, scope, started_at, status) VALUES (?, ?, ?, 'running')",
                rusqlite::params![t, scope, at],
            )
            .unwrap();
        reg.conn().last_insert_rowid()
    };
    let dup = start("collect", "bob", now - 60);
    let r = api(
        &env,
        "reseller:reseller1",
        &[("a", "admin.run")],
        Some(json!({"type": "collect", "user": "bob"})),
    );
    assert_eq!(r.status, 409, "{}", r.body);
    assert_eq!(r.json["error"]["code"], "conflict");
    assert!(
        r.json["error"]["message"]
            .as_str()
            .unwrap()
            .contains(&format!("#{dup}"))
    );
    start("backfill", "alice", now - 60);
    start("collect", "alice", now - 60);
    assert_eq!(
        status(
            &env,
            "admin",
            &[("a", "admin.run")],
            Some(json!({"type": "rollup"}))
        ),
        409,
        "three jobs running"
    );
    // jobs "running" for more than 6 hours are presumed dead
    reg.conn()
        .execute(
            "UPDATE jobs SET started_at = ? WHERE status = 'running'",
            [now - 7 * 3600],
        )
        .unwrap();
    reg.start_job_limited("rollup", "all", now - 3600, 3, now)
        .expect("stale jobs are ignored");
    let e = reg
        .start_job_limited("rollup", "all", now - 3600, 3, now)
        .unwrap_err();
    assert!(
        e.downcast_ref::<webpulse::http::registry::Conflict>()
            .is_some(),
        "already running"
    );
}
