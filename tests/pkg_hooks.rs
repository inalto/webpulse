//! Standardized Hook scripts: `--describe`, the flag file of Stats::RunUser and
//! the collect run of Stats::RunAll.

mod pkg_common;

use pkg_common::*;
use serde_json::{Value, json};

fn hook(name: &str) -> String {
    s(&repo().join("hooks").join(name))
}

#[test]
fn describe_prints_the_registration_json() {
    for (file, event) in [
        ("stats_runall_pre.sh", "RunAll"),
        ("stats_runuser_pre.sh", "RunUser"),
    ] {
        let path = hook(file);
        let (rc, out) = cmd(&[path.as_str(), "--describe"]).go();
        assert_eq!(rc, 0, "{file} --describe exit\n{out}");
        let v: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{file}: {e}: {out}"));
        let list = v.as_array().expect("a list");
        assert_eq!(list.len(), 1, "one hook described");
        assert_eq!(
            list[0],
            json!({"category": "Stats", "event": event, "stage": "pre", "hook": path, "exectype": "script"}),
            "{file} describe"
        );
        let keys: Vec<&str> = list[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["category", "event", "stage", "hook", "exectype"],
            "key order"
        );
    }
}

#[test]
fn describe_keeps_the_logical_path_of_a_symlinked_install_dir() {
    let tmp = TmpDir::new("hooklink");
    std::os::unix::fs::symlink(repo(), tmp.join("webpulse")).unwrap();
    let path = s(&tmp.join("webpulse/hooks/stats_runall_pre.sh"));
    let (_, out) = cmd(&[path.as_str(), "--describe"]).go();
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v[0]["hook"], json!(path), "hook path through the symlink");
}

#[test]
fn runuser_touches_the_flag_for_a_valid_user() {
    let dir = TmpDir::new("hookflag");
    let (rc, out) = cmd(&[&hook("stats_runuser_pre.sh")])
        .env("WEBPULSE_HOOK_DIR", &dir.path)
        .stdin(r#"{"context":{"event":"RunUser"},"data":{"user":"alice","domain":"example.com"}}"#)
        .go();
    assert_eq!(rc, 0, "exit status");
    assert_eq!(out, "", "silent");
    assert_exists(&dir.join("alice"));
    assert_eq!(list_tree(&dir.path), ["alice"], "only the flag was created");
}

#[test]
fn runuser_needs_neither_php_nor_the_binary_only_plain_shell_tools() {
    let src = read(&repo().join("hooks/stats_runuser_pre.sh"));
    assert_not_contains(&src.to_lowercase(), "php", "no PHP in the hook");
    assert_not_contains(&src, "bin/webpulse", "independent of the binary");
    let bin = TmpDir::new("hooktools");
    for tool in ["head", "tr", "touch", "dirname", "basename", "cat"] {
        let path = which(tool).unwrap_or_else(|| panic!("{tool} not found"));
        std::os::unix::fs::symlink(path, bin.join(tool)).unwrap();
    }
    let dir = TmpDir::new("hookflag2");
    let (rc, out) = cmd(&["bash", &hook("stats_runuser_pre.sh")])
        .env("PATH", &bin.path)
        .env("WEBPULSE_HOOK_DIR", &dir.path)
        .stdin(r#"{"data":{"user":"alice"}}"#)
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_exists(&dir.join("alice"));
}

#[test]
fn runuser_reads_data_user_tolerates_whitespace_and_pretty_printing() {
    let dir = TmpDir::new("hookjson");
    let payloads = [
        (
            "alice",
            r#"{"context":{"user":"root","event":"RunUser"},"data":{"user":"alice","domain":"example.com"}}"#,
        ),
        (
            "bob_1",
            "{\n  \"context\": {\"stage\": \"pre\"},\n  \"data\" : {\n    \"args\": {\"x\": 1},\n    \"user\" :  \"bob_1\"\n  }\n}\n",
        ),
        ("carol-2", r#"{"data":{"user":"carol-2"},"context":{}}"#),
    ];
    for (expected, payload) in payloads {
        let (rc, out) = cmd(&[&hook("stats_runuser_pre.sh")])
            .env("WEBPULSE_HOOK_DIR", &dir.path)
            .stdin(payload)
            .go();
        assert_eq!(rc, 0, "{out}");
        assert_exists(&dir.join(expected));
    }
    assert_eq!(
        list_tree(&dir.path),
        ["alice", "bob_1", "carol-2"],
        "exactly the data.user flags, never context.user"
    );
}

#[test]
fn runuser_ignores_invalid_users_and_bad_payloads_and_exits_0() {
    let dir = TmpDir::new("hookbad");
    let flags = dir.join("flags");
    std::fs::create_dir(&flags).unwrap();
    let long = format!(r#"{{"data":{{"user":"{}"}}}}"#, "a".repeat(33));
    let payloads = [
        r#"{"data":{"user":"../x"}}"#,
        r#"{"data":{"user":"a/b"}}"#,
        r#"{"data":{"user":""}}"#,
        r#"{"data":{"user":"Root"}}"#,
        r#"{"data":{"user":"-x"}}"#,
        &long,
        r#"{"data":{"user":["alice"]}}"#,
        r#"{"data":{"user":"alice\n"}}"#,
        r#"{"data":{}}"#,
        r#"{"data":"alice"}"#,
        "not json",
        "",
        "[]",
    ];
    for payload in payloads {
        let (rc, out) = cmd(&[&hook("stats_runuser_pre.sh")])
            .env("WEBPULSE_HOOK_DIR", &flags)
            .stdin(payload)
            .go();
        assert_eq!(rc, 0, "exit status for {payload}\n{out}");
    }
    assert!(
        list_tree(&flags).is_empty(),
        "no flag created: {:?}",
        list_tree(&flags)
    );
    assert_missing(&dir.join("x"));
    // a missing flag directory is not an error either
    let (rc, _) = cmd(&[&hook("stats_runuser_pre.sh")])
        .env("WEBPULSE_HOOK_DIR", flags.join("nope"))
        .stdin(r#"{"data":{"user":"alice"}}"#)
        .go();
    assert_eq!(rc, 0, "missing dir");
}

#[test]
fn runuser_does_not_follow_a_planted_symlink() {
    let dir = TmpDir::new("hooklnk");
    let target = dir.join("victim");
    write(&target, "x");
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    std::fs::File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_modified(old)
        .unwrap();
    std::fs::create_dir(dir.join("flags")).unwrap();
    std::os::unix::fs::symlink(&target, dir.join("flags/alice")).unwrap();
    cmd(&[&hook("stats_runuser_pre.sh")])
        .env("WEBPULSE_HOOK_DIR", dir.join("flags"))
        .stdin(r#"{"data":{"user":"alice"}}"#)
        .go();
    let mtime = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(mtime, old, "symlink target untouched");
}

#[test]
fn runall_runs_collect_quiet_and_always_exits_0() {
    let home = TmpDir::new("hookall");
    let log = home.join("calls.log");
    script(
        &home.join("bin/webpulse"),
        &format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit 3\n", log.display()),
    );
    let (rc, out) = cmd(&[&hook("stats_runall_pre.sh")])
        .env("WEBPULSE_HOME", &home.path)
        .stdin(r#"{"context":{},"data":{}}"#)
        .go();
    assert_eq!(rc, 0, "exit status even when collect fails");
    assert_eq!(out, "", "silent");
    assert_eq!(read(&log), "collect --quiet\n", "collect invocation");
    // missing engine is not an error
    let (rc, _) = cmd(&[&hook("stats_runall_pre.sh")])
        .env("WEBPULSE_HOME", home.join("none"))
        .go();
    assert_eq!(rc, 0, "no engine");
}

#[test]
fn runall_uses_a_20_minute_timeout() {
    assert_contains(
        &read(&repo().join("hooks/stats_runall_pre.sh")),
        "timeout 1200",
        "RunAll hook",
    );
}

#[test]
fn runall_ignores_an_untrusted_webpulse_home() {
    // WEBPULSE_HOME is honoured only when the directory, bin/ and bin/webpulse belong
    // to the user running the hook (root) and are not group/other-writable
    use std::os::unix::fs::PermissionsExt;
    let set_mode = |p: &std::path::Path, m: u32| {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap()
    };
    let home = TmpDir::new("hookuntrusted");
    let log = home.join("calls.log");
    script(
        &home.join("bin/webpulse"),
        &format!("#!/bin/sh\necho \"$*\" >> '{}'\n", log.display()),
    );
    set_mode(&home.path, 0o755);
    set_mode(&home.join("bin"), 0o755);
    let run = || {
        cmd(&[&hook("stats_runall_pre.sh")])
            .env("WEBPULSE_HOME", &home.path)
            .stdin(r#"{"context":{},"data":{}}"#)
            .go()
    };
    for (what, path, bad) in [
        ("group-writable binary", home.join("bin/webpulse"), 0o775),
        ("other-writable bin/", home.join("bin"), 0o757),
        ("world-writable home", home.path.clone(), 0o777),
    ] {
        let good = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        set_mode(&path, bad);
        let (rc, _) = run();
        assert_eq!(rc, 0, "{what}: always exits 0");
        assert!(!log.exists(), "{what}: the binary was not run");
        set_mode(&path, good);
    }
    let (rc, _) = run();
    assert_eq!(rc, 0);
    assert_eq!(read(&log), "collect --quiet\n", "a trusted home is used");
}

#[test]
fn runuser_checks_for_a_symlink_right_before_touch() {
    let src = read(&repo().join("hooks/stats_runuser_pre.sh"));
    assert_contains(&src, "[ -L \"$flag\" ] && exit 0", "RunUser hook");
    assert_contains(
        &src,
        "fs.protected_symlinks",
        "RunUser hook documents the mitigation",
    );
    let i = src.find("[ -L \"$flag\" ] && exit 0").unwrap();
    let j = src.find("touch -- \"$flag\"").unwrap();
    assert!(i < j, "the check precedes touch");
}
