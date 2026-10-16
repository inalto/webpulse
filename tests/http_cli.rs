//! `webpulse report` (port of the CLI cases of tests/cases/ui_engine.php) and
//! the dimension list shared with the engine.

mod http_common;

use http_common::*;
use serde_json::Value;

const SEED: &str = "http-cli";

fn report(env: &Env, args: &[&str]) -> Run {
    let mut a = vec!["report"];
    a.extend_from_slice(args);
    run(
        &a,
        &[("WEBPULSE_CONFIG", env.config.display().to_string())],
        b"",
    )
}

#[test]
fn report_overview_table() {
    let env = shared(SEED);
    let r = report(
        &env,
        &[
            "--user",
            "alice",
            "--domain",
            "example.com",
            "--range",
            "7d",
        ],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let out = r.out();
    assert!(
        out.contains(
            "example.com  2026-10-10 → 2026-10-16 (7 days, previous 2026-10-03 → 2026-10-09)"
        ),
        "{out}"
    );
    assert!(out.contains("bounce_rate") && out.contains("metric") && out.contains("previous"));
    let r = report(
        &env,
        &["--user", "alice", "--domain", "EXAMPLE.COM", "--quiet"],
    );
    assert_eq!(
        (r.code, r.out()),
        (0, String::new()),
        "--quiet prints nothing"
    );
}

#[test]
fn report_dimension_json_and_table() {
    let env = shared(SEED);
    let r = report(
        &env,
        &[
            "--user",
            "alice",
            "--domain",
            "example.com",
            "--name",
            "page",
            "--limit",
            "3",
            "--json",
        ],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let j: Value = serde_json::from_str(&r.out()).unwrap();
    assert_eq!(j["name"], "page");
    assert!(j["rows"].as_array().unwrap().len() <= 3);
    assert!(
        r.out().starts_with("{\n    \"name\": \"page\",\n"),
        "PHP pretty print"
    );
    let t = report(
        &env,
        &[
            "--user",
            "alice",
            "--domain",
            "example.com",
            "--name",
            "country",
            "--limit",
            "2",
        ],
    );
    let out = t.out();
    assert!(
        out.starts_with("example.com  Countries  2026-09-17 → 2026-10-16\n\ncountry"),
        "{out}"
    );
    assert!(out.contains("Italy") && out.contains("more rows (use --limit)"));
}

#[test]
fn report_errors() {
    let env = shared(SEED);
    assert_eq!(
        report(
            &env,
            &[
                "--user",
                "alice",
                "--domain",
                "example.com",
                "--name",
                "nope"
            ]
        )
        .code,
        2,
        "bad dimension"
    );
    assert_eq!(
        report(
            &env,
            &[
                "--user",
                "alice",
                "--domain",
                "example.com",
                "--range",
                "fortnight"
            ]
        )
        .code,
        2,
        "bad range"
    );
    assert_eq!(report(&env, &["--user", "alice"]).code, 2, "missing domain");
    assert_eq!(
        report(&env, &["--user", "../x", "--domain", "example.com"]).code,
        2
    );
    assert_eq!(
        report(&env, &["--user", "alice", "--domain", "nosuch.example"]).code,
        1,
        "no database"
    );
}

/// `reports::dimension::ALL` must equal the engine's `engine::store::dims::ALL`
/// (read from its source so this test does not depend on the engine's wiring).
#[test]
fn dimension_names_agree_with_the_engine() {
    let path = repo().join("src/engine/store/dims.rs");
    let Ok(src) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: {} not present", path.display());
        return;
    };
    let start = src.find("pub const ALL").expect("ALL in dims.rs");
    let body = &src[start..start + src[start..].find("];").unwrap()];
    let names: Vec<&str> = body.split('"').skip(1).step_by(2).collect();
    assert_eq!(names, webpulse::reports::dimension::ALL.to_vec());
    for (c, v) in [
        (
            "DURATION_BUCKETS",
            webpulse::reports::dimension::DURATION_BUCKETS.to_vec(),
        ),
        (
            "PAGES_BUCKETS",
            webpulse::reports::dimension::PAGES_BUCKETS.to_vec(),
        ),
    ] {
        let s = src.find(&format!("pub const {c}")).unwrap();
        let b = &src[s..s + src[s..].find("];").unwrap()];
        assert_eq!(
            b.split('"').skip(1).step_by(2).collect::<Vec<_>>(),
            v,
            "{c}"
        );
    }
    assert!(
        src.contains("pub const NF_REF_SEP: &str = \" \\u{2190} \";")
            || src.contains("pub const NF_REF_SEP: &str = \" ← \";")
    );
}
