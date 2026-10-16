//! Port of tests/cases/engine_registry.php (in-process).

mod common;

use common::FakeRoot;
use serde_json::{Value, json};
use webpulse::config::Config;
use webpulse::engine::collector::Collector;
use webpulse::engine::discovery::{CpanelUsers, LogFiles};
use webpulse::engine::registry::Registry;
use webpulse::engine::store::DomainStore;
use webpulse::engine::user_settings::{DomainSettings, UserSettings};
use webpulse::error::WpError;
use webpulse::paths::{is_valid_domain, is_valid_user};

fn kinds(m: &std::collections::BTreeMap<String, String>) -> Vec<(&str, &str)> {
    m.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

#[test]
fn app_boot_contract_config_paths_webpulse_config() {
    let fr = FakeRoot::create("rs-reg-app", json!({}));
    let app = fr.app();
    assert_eq!(app.paths.data_dir(), fr.path("var/webpulse/data"));
    assert_eq!(
        app.paths.user_dir("alice"),
        fr.path("var/webpulse/data/alice")
    );
    assert_eq!(
        app.paths.domain_db("alice", "example.com"),
        fr.path("var/webpulse/data/alice/example.com.db")
    );
    assert_eq!(
        app.paths.user_settings_file("alice"),
        fr.path("var/webpulse/data/alice/user/settings.json")
    );
    assert_eq!(app.paths.registry_db(), fr.path("var/webpulse/webpulse.db"));
    assert_eq!(app.config.get("timezone"), "Europe/Rome");
    assert_eq!(app.config.get("session_timeout_min"), 30);
    assert!(app.config.get_opt("no.such.key").is_none());
    let missing = webpulse::app::App::try_boot(webpulse::app::Global {
        config: Some("/nonexistent/config.json".into()),
        ..Default::default()
    });
    assert!(missing.unwrap_err().to_string().contains("not found"));
    let e = app.open_domain("alice", "example.com", true).unwrap_err();
    assert!(matches!(
        e.downcast_ref::<WpError>(),
        Some(WpError::NotFound(_))
    ));
    assert!(
        app.open_domain("../x", "example.com", true)
            .unwrap_err()
            .to_string()
            .contains("invalid user name")
    );
    assert!(
        app.open_domain("alice", "../../etc/passwd", true)
            .unwrap_err()
            .to_string()
            .contains("invalid domain name")
    );
}

#[test]
fn paths_reject_traversal() {
    for u in ["alice", "a1", "user_name", "r-1"] {
        assert!(is_valid_user(u), "{u}");
    }
    for u in ["", "..", "a/b", "Alice", "a.b", &"a".repeat(40), "a\0"] {
        assert!(!is_valid_user(u), "{u:?}");
    }
    for d in [
        "example.com",
        "shop.example.com",
        "xn--bcher-kva.example",
        "a-b.co.uk",
        "localhost",
    ] {
        assert!(is_valid_domain(d), "{d}");
    }
    for d in [
        "",
        "..",
        "../x",
        "a/b",
        "Example.com",
        "-a.com",
        "a..b",
        "a.com/",
        "*.example.com",
    ] {
        assert!(!is_valid_domain(d), "{d}");
    }
}

#[test]
fn config_precedence_defaults_file_registry_overrides() {
    let fr = FakeRoot::create("rs-reg-config", json!({"raw_retention_days": 45}));
    let app = fr.app();
    assert_eq!(app.config.get("raw_retention_days"), 45);
    assert_eq!(app.config.get("max_rows_per_dim"), 1000);
    let reg = app.registry().unwrap();
    reg.set_setting(
        "raw_retention_days",
        &Config::validate("raw_retention_days", &json!(90)).unwrap(),
    )
    .unwrap();
    reg.set_setting(
        "geoip.enabled",
        &Config::validate("geoip.enabled", &json!("true")).unwrap(),
    )
    .unwrap();
    let fresh = fr.app();
    assert_eq!(fresh.config.get("raw_retention_days"), 90);
    assert_eq!(fresh.config.get("geoip.enabled"), true);
    assert_eq!(
        fresh.config.get_str("geoip.country_db").unwrap(),
        fr.path("var/webpulse/geoip/dbip-country-lite.mmdb")
            .display()
            .to_string(),
        "sibling keys kept"
    );
    assert_eq!(fresh.config.effective()["raw_retention_days"], 90);
    let s = fresh.registry().unwrap().settings();
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        json!({"geoip.enabled": true, "raw_retention_days": 90})
    );
    fresh
        .registry()
        .unwrap()
        .delete_setting("raw_retention_days")
        .unwrap();
    assert_eq!(fr.app().config.get("raw_retention_days"), 45);
    assert!(Config::validate("paths.data_dir", &json!("/tmp")).is_err());
    assert!(Config::validate("max_rows_per_dim", &json!(5)).is_err());
    assert!(Config::validate("timezone", &json!("Mars/Base")).is_err());
    assert!(Config::validate("exclude_ips", &json!(["nope"])).is_err());
    assert!(Config::validate("exclude_paths", &json!(["relative"])).is_err());
    assert_eq!(
        Config::validate("exclude_ips", &json!([" 1.2.3.0/24 "])).unwrap(),
        json!(["1.2.3.0/24"])
    );
    assert_eq!(
        Config::validate("timezone", &json!("")).unwrap(),
        Value::Null
    );
    assert_eq!(
        Config::load(Some(std::path::Path::new("/nonexistent.json"))).get("paths.data_dir"),
        "/var/webpulse/data"
    );
    let bad = fr.dir().join("bad.json");
    std::fs::write(&bad, "{oops").unwrap();
    assert!(
        Config::try_load(Some(&bad))
            .unwrap_err()
            .to_string()
            .contains("invalid JSON")
    );
    std::fs::write(
        fr.dir().join("rel.json"),
        json!({"paths": {"data_dir": "data"}}).to_string(),
    )
    .unwrap();
    assert_eq!(
        Config::load(Some(&fr.dir().join("rel.json")))
            .get_str("paths.data_dir")
            .unwrap(),
        format!("{}/data", fr.dir().display())
    );
}

#[test]
fn registry_effective_enabled_matrix() {
    let cases = [
        (None, None, true, true),
        (None, None, false, false),
        (Some(true), None, false, true),
        (Some(false), None, true, false),
        (None, Some(true), false, true),
        (None, Some(false), true, false),
        (Some(true), Some(false), true, false),
        (Some(false), Some(true), true, false),
        (Some(true), Some(true), false, true),
    ];
    for (a, d, def, exp) in cases {
        assert_eq!(Registry::effective(a, d, def), exp, "{a:?} {d:?} {def}");
    }
    let fr = FakeRoot::create("rs-reg-switch", json!({}));
    let app = fr.app();
    let reg = app.registry().unwrap();
    assert!(
        reg.is_enabled("alice", "example.com").unwrap(),
        "unknown -> default"
    );
    reg.set_account("alice", Some(false), None).unwrap();
    assert!(!reg.is_enabled("alice", "example.com").unwrap());
    assert_eq!(
        reg.account("alice").unwrap().unwrap().owner.as_deref(),
        Some("root"),
        "owner filled from cPanel data"
    );
    reg.set_domain("alice", "example.com", Some(true)).unwrap();
    assert!(!reg.is_enabled("alice", "example.com").unwrap());
    reg.set_account("alice", None, Some("Europe/Paris"))
        .unwrap();
    assert!(reg.is_enabled("alice", "example.com").unwrap());
    assert_eq!(
        reg.account("alice").unwrap().unwrap().timezone.as_deref(),
        Some("Europe/Paris")
    );
    reg.set_account("alice", None, None).unwrap();
    assert_eq!(
        reg.account("alice").unwrap().unwrap().timezone.as_deref(),
        Some("Europe/Paris"),
        "null timezone = unchanged"
    );
    reg.set_account("alice", None, Some("")).unwrap();
    assert!(reg.account("alice").unwrap().unwrap().timezone.is_none());
    assert!(
        reg.set_account("alice", Some(true), Some("Nowhere/Land"))
            .is_err()
    );
    assert!(
        reg.set_domain("alice", "bad/../domain", Some(true))
            .is_err()
    );
    let rows: Vec<(String, bool, Option<bool>)> = reg
        .domains(Some("alice"))
        .unwrap()
        .into_iter()
        .map(|r| (r.domain, r.enabled, r.enabled_override))
        .collect();
    assert_eq!(rows, [("example.com".to_string(), true, Some(true))]);
}

#[test]
fn registry_sync_removal_owner_filter_search() {
    let fr = FakeRoot::create("rs-reg-sync", json!({}));
    let app = fr.app();
    Collector::new(&app).sync_registry(None).unwrap();
    let reg = app.registry().unwrap();
    let users = |rows: Vec<webpulse::engine::registry::AccountRow>| {
        rows.into_iter().map(|r| r.user).collect::<Vec<_>>()
    };
    assert_eq!(users(reg.accounts(None, None).unwrap()), ["alice", "bob"]);
    assert_eq!(
        users(reg.accounts(Some("reseller1"), None).unwrap()),
        ["bob"]
    );
    assert_eq!(users(reg.accounts(None, Some("shop")).unwrap()), ["alice"]);
    assert_eq!(reg.accounts(None, Some("ali")).unwrap()[0].domains, 2);
    std::fs::remove_file(fr.path("var/cpanel/users/bob")).unwrap();
    let app2 = fr.app();
    Collector::new(&app2).sync_registry(None).unwrap();
    assert!(
        app2.registry()
            .unwrap()
            .account("bob")
            .unwrap()
            .unwrap()
            .removed_at
            .is_some()
    );
    assert!(
        app2.registry().unwrap().domains(Some("bob")).unwrap()[0]
            .removed_at
            .is_some()
    );
    fr.write_log(
        "etc/apache2/logs/domlogs/alice/old.example.org-ssl_log",
        &["x".into()],
        true,
    );
    fr.write_log(
        "etc/apache2/logs/domlogs/alice/example.com-bytes_log",
        &["1".into()],
        true,
    );
    let app3 = fr.app();
    Collector::new(&app3).sync_registry(Some("alice")).unwrap();
    let k: Vec<(String, String)> = app3
        .registry()
        .unwrap()
        .domains(Some("alice"))
        .unwrap()
        .into_iter()
        .map(|r| (r.domain, r.kind))
        .collect();
    assert_eq!(
        k,
        [
            ("example.com".to_string(), "main".to_string()),
            ("old.example.org".into(), "unknown".into()),
            ("shop.example.com".into(), "sub".into())
        ]
    );
}

#[test]
fn registry_salts_jobs_summaries() {
    let fr = FakeRoot::create("rs-reg-misc", json!({}));
    let app = fr.app();
    let reg = app.registry().unwrap();
    let a = reg.salt("2026-10-10").unwrap();
    assert_eq!(a.len(), 32);
    assert_eq!(a, reg.salt("2026-10-10").unwrap());
    assert_eq!(
        a,
        fr.app().registry().unwrap().salt("2026-10-10").unwrap(),
        "persisted"
    );
    assert_ne!(a, reg.salt("2026-10-11").unwrap());
    assert_eq!(reg.prune_salts("2026-10-11").unwrap(), 1);
    assert_ne!(
        a,
        reg.salt("2026-10-10").unwrap(),
        "pruned salt is regenerated"
    );
    let mut last = 0;
    for _ in 0..505 {
        last = reg.start_job("collect", Some("all"), None).unwrap();
    }
    let n: i64 = reg
        .conn()
        .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 500);
    reg.finish_job(last, "error", 7, 2, 1, Some("bad \u{1} message"))
        .unwrap();
    let j = &reg.jobs(1).unwrap()[0];
    assert_eq!(
        (
            j["status"].as_str(),
            j["lines"].as_i64(),
            j["files"].as_i64(),
            j["domains"].as_i64(),
            j["message"].as_str()
        ),
        (
            Some("error"),
            Some(7),
            Some(2),
            Some(1),
            Some("bad ? message")
        )
    );
    assert_eq!(reg.jobs(50).unwrap().len(), 50);
    assert!(reg.summaries(Some(&[])).unwrap().is_empty());
    // job limits
    reg.conn()
        .execute("UPDATE jobs SET status = 'ok'", [])
        .unwrap();
    let since = 0;
    let id = reg
        .start_job_limited("backfill", "alice", since, 10)
        .unwrap();
    let e = reg
        .start_job_limited("backfill", "alice", since, 10)
        .unwrap_err();
    assert!(
        e.downcast_ref::<webpulse::engine::registry::JobRefused>()
            .is_some(),
        "{e}"
    );
    assert!(e.to_string().contains(&format!("(#{id})")));
}

#[test]
fn discovery_cpanel_users_kinds_owners_homes() {
    let fr = FakeRoot::create("rs-reg-cpusers", json!({}));
    let app = fr.app();
    let cp = CpanelUsers::new(&app.paths);
    let all = cp.all();
    assert_eq!(all.keys().cloned().collect::<Vec<_>>(), ["alice", "bob"]);
    assert_eq!(all["alice"].owner.as_deref(), Some("root"));
    assert_eq!(cp.owner("bob").as_deref(), Some("reseller1"));
    assert!(cp.owner("ghost").is_none());
    assert!(!cp.exists("ghost"));
    assert!(!cp.exists("../etc"));
    assert_eq!(
        kinds(&cp.domains_of("alice")),
        [("example.com", "main"), ("shop.example.com", "sub")]
    );
    assert_eq!(all["alice"].domains, ["example.com", "shop.example.com"]);
    assert_eq!(
        all["alice"].home,
        fr.path("home/alice").display().to_string()
    );
    assert_eq!(kinds(&cp.domains_of("bob")), [("blog.net", "main")]);
    std::fs::remove_file(fr.path("var/cpanel/userdata/alice/main")).unwrap();
    std::fs::write(
        fr.path("var/cpanel/users/carol"),
        "DNS=carol.io\nDNS1=other.net\nDNS2=blog.carol.io\nDNS3=parked.org\nOWNER=reseller1\n",
    )
    .unwrap();
    let mut ud = std::fs::read_to_string(fr.path("etc/userdomains")).unwrap();
    ud.push_str("dev.other.net: carol\n");
    std::fs::write(fr.path("etc/userdomains"), ud).unwrap();
    std::fs::create_dir_all(fr.path("var/cpanel/users/.hidden")).unwrap();
    std::fs::write(fr.path("var/cpanel/users/system"), "DNS=x.com\n").unwrap();
    let cp2 = CpanelUsers::new(&app.paths);
    assert_eq!(
        cp2.all().keys().cloned().collect::<Vec<_>>(),
        ["alice", "bob", "carol"]
    );
    assert_eq!(
        kinds(&cp2.domains_of("carol")),
        [
            ("blog.carol.io", "sub"),
            ("carol.io", "main"),
            ("dev.other.net", "sub"),
            ("other.net", "addon"),
            ("parked.org", "addon")
        ]
    );
    std::fs::create_dir_all(fr.path("var/cpanel/userdata/carol")).unwrap();
    std::fs::write(
        fr.path("var/cpanel/userdata/carol/main"),
        "---\naddon_domains:\n  other.net: other.carol.io\nmain_domain: carol.io\nparked_domains:\n  - parked.org\nsub_domains:\n  - blog.carol.io\n  - other.carol.io\n",
    )
    .unwrap();
    let cp3 = CpanelUsers::new(&app.paths);
    assert_eq!(
        kinds(&cp3.domains_of("carol")),
        [
            ("blog.carol.io", "sub"),
            ("carol.io", "main"),
            ("dev.other.net", "sub"),
            ("other.net", "addon"),
            ("parked.org", "parked")
        ]
    );
}

#[test]
fn discovery_log_files_ignore_patterns_symlinked_dirs_archives() {
    let fr = FakeRoot::create("rs-reg-logfiles", json!({}));
    let app = fr.app();
    let lf = LogFiles::new(&app.paths);
    for f in [
        "example.com",
        "example.com-ssl_log",
        "example.com-bytes_log",
        "example.com-ftp_log",
        "ftpxferlog",
        "example.com.bkup",
        "example.com.offset",
    ] {
        fr.write_log(
            &format!("etc/apache2/logs/domlogs/alice/{f}"),
            &["x".into()],
            true,
        );
    }
    std::fs::create_dir_all(fr.path("usr/local/apache/domlogs")).unwrap();
    std::os::unix::fs::symlink(
        fr.path("etc/apache2/logs/domlogs/alice"),
        fr.path("usr/local/apache/domlogs/alice"),
    )
    .unwrap();
    fr.write_log(
        "var/log/nginx/domains/shop.example.com",
        &["x".into()],
        true,
    );
    let files: Vec<(std::path::PathBuf, bool)> = lf
        .for_domain("alice", "example.com")
        .into_iter()
        .map(|f| (f.path, f.https))
        .collect();
    assert_eq!(
        files,
        [
            (fr.domlog("alice", "example.com"), false),
            (fr.domlog("alice", "example.com-ssl_log"), true)
        ]
    );
    assert_eq!(
        lf.for_domain("alice", "shop.example.com")
            .into_iter()
            .map(|f| f.path)
            .collect::<Vec<_>>(),
        [fr.path("var/log/nginx/domains/shop.example.com")]
    );
    assert_eq!(lf.domains_in_user_dirs("alice"), ["example.com"]);
    let logs = fr.path("home/alice/logs");
    let gz = |s: &str| {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(s.as_bytes()).unwrap();
        e.finish().unwrap()
    };
    for n in [
        "example.com-Sep-2026.gz",
        "example.com-ssl_log-Sep-2026.gz",
        "shop.example.com-Sep-2026.gz",
        "example.com-bytes_log-Sep-2026.gz",
        "unrelated.org-Sep-2026.gz",
    ] {
        std::fs::write(logs.join(n), gz("x\n")).unwrap();
    }
    std::fs::write(fr.dir().join("secret.gz"), gz("secret\n")).unwrap();
    std::os::unix::fs::symlink(
        fr.dir().join("secret.gz"),
        logs.join("example.com-Aug-2026.gz"),
    )
    .unwrap();
    let doms = vec!["example.com".to_string(), "shop.example.com".to_string()];
    let arch = lf.archives("alice", &doms, None);
    assert_eq!(arch.keys().cloned().collect::<Vec<_>>(), doms);
    let mut ex: Vec<(String, bool)> = arch["example.com"]
        .iter()
        .map(|a| {
            (
                a.path.file_name().unwrap().to_string_lossy().into_owned(),
                a.https,
            )
        })
        .collect();
    ex.sort();
    assert_eq!(
        ex,
        [
            ("example.com-Sep-2026.gz".to_string(), false),
            ("example.com-ssl_log-Sep-2026.gz".to_string(), true)
        ]
    );
    assert_eq!(arch["shop.example.com"].len(), 1);
    std::fs::rename(&logs, format!("{}.real", logs.display())).unwrap();
    std::os::unix::fs::symlink(format!("{}.real", logs.display()), &logs).unwrap();
    assert!(lf.archives("alice", &doms[..1], None).is_empty());
}

#[test]
fn user_settings_validation_defaults_symlink_and_size_checks() {
    let v = UserSettings::validate(&json!({"version": 1, "domains": {"Example.COM": {"timezone": "Europe/Rome", "exclude_ips": ["1.2.3.4", "2001:db8::/32"], "exclude_paths": ["/x/"]}}, "ui": {"dark": "on"}})).unwrap();
    assert_eq!(
        v.domains["example.com"].timezone.as_deref(),
        Some("Europe/Rome")
    );
    assert_eq!(v.dark, "dark");
    assert_eq!(v.default_range, "30d");
    assert_eq!(v.for_domain("other.com"), DomainSettings::default());
    let fr = FakeRoot::create("rs-reg-usersettings", json!({}));
    let app = fr.app();
    let (s, err) = UserSettings::read(&app.paths, "alice");
    assert_eq!(s, UserSettings::default());
    assert!(err.is_none());
    DomainStore::prepare_user_dir(&app.paths, "alice").unwrap();
    let file = app.paths.user_settings_file("alice");
    let written = UserSettings::write(
        &app.paths,
        "alice",
        &json!({"domains": {"example.com": {"exclude_paths": ["/admin/"]}}}),
    )
    .unwrap();
    assert_eq!(
        UserSettings::read(&app.paths, "alice").0.domains["example.com"].exclude_paths,
        ["/admin/"]
    );
    assert_eq!(written, UserSettings::read(&app.paths, "alice").0);
    std::fs::write(&file, "{not json").unwrap();
    let (s, err) = UserSettings::read(&app.paths, "alice");
    assert_eq!(s, UserSettings::default());
    assert!(err.unwrap().contains("invalid JSON"));
    std::fs::write(&file, json!({"ui": {"x": "a".repeat(70000)}}).to_string()).unwrap();
    assert!(
        UserSettings::read(&app.paths, "alice")
            .1
            .unwrap()
            .contains("64 KiB")
    );
    std::fs::remove_file(&file).unwrap();
    std::fs::write(
        fr.dir().join("target.json"),
        json!({"domains": {"example.com": {"exclude_paths": ["/leak/"]}}}).to_string(),
    )
    .unwrap();
    std::os::unix::fs::symlink(fr.dir().join("target.json"), &file).unwrap();
    let (s, err) = UserSettings::read(&app.paths, "alice");
    assert_eq!(s, UserSettings::default());
    assert!(err.unwrap().contains("not a regular file"));
    std::fs::remove_file(&file).unwrap();
    std::fs::remove_dir(file.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(fr.dir(), file.parent().unwrap()).unwrap();
    let (s, err) = UserSettings::read(&app.paths, "alice");
    assert_eq!(s, UserSettings::default());
    assert!(err.unwrap().contains("not a directory"));
}

#[test]
fn domain_store_creation_modes_read_only_access_schema_guard() {
    use std::os::unix::fs::PermissionsExt;
    let fr = FakeRoot::create("rs-reg-store", json!({}));
    let app = fr.app();
    let w = app.open_domain("alice", "example.com", false).unwrap();
    assert!(!w.read_only());
    let m = w.meta();
    let g = |k: &str| m.get(k).cloned().flatten().unwrap_or_default();
    assert_eq!(
        (
            g("schema_version"),
            g("user"),
            g("domain"),
            g("next_visit_id")
        ),
        ("1".into(), "alice".into(), "example.com".into(), "1".into())
    );
    assert_eq!(
        std::fs::metadata(w.path()).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        std::fs::metadata(app.paths.user_dir("alice"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    let jm: String = w
        .conn()
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(jm, "delete");
    w.set_meta("timezone", Some("Europe/Rome")).unwrap();
    let r = app.open_domain("alice", "example.com", true).unwrap();
    assert!(r.read_only());
    assert_eq!(r.get_meta("timezone").as_deref(), Some("Europe/Rome"));
    assert!(
        r.conn()
            .execute("INSERT INTO meta VALUES ('x', 'y')", [])
            .is_err()
    );
    assert!(r.set_meta("x", Some("y")).is_err());
    w.set_meta("schema_version", Some("99")).unwrap();
    let e = app.open_domain("alice", "example.com", true).unwrap_err();
    assert!(matches!(
        e.downcast_ref::<WpError>(),
        Some(WpError::SchemaTooNew { .. })
    ));
    let e = app.open_domain("alice", "example.com", false).unwrap_err();
    assert!(matches!(
        e.downcast_ref::<WpError>(),
        Some(WpError::SchemaTooNew { .. })
    ));
    let s = app.open_domain("alice", "shop.example.com", false).unwrap();
    let tables: Vec<String> = s
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    assert_eq!(
        tables,
        [
            "checkpoints",
            "daily",
            "dim_daily",
            "events",
            "hourly",
            "imported_archives",
            "meta",
            "open_visits",
            "visitor_day",
            "visits"
        ]
    );
}
