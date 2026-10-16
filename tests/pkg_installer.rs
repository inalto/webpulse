//! install.sh end to end inside a temporary prefix with a fake cPanel root
//! (WEBPULSE_PREFIX / WEBPULSE_FAKE_CPANEL / stub cPanel tools that log their
//! invocations) and a stub WebPulse binary in the release layout. Download tests
//! use file:// URLs and a fake curl/uname on PATH. Nothing outside tests/tmp is
//! touched.

mod pkg_common;

use std::path::{Path, PathBuf};

use pkg_common::*;
use serde_json::{Value, json};

fn config(p: &Sandbox) -> Value {
    serde_json::from_str(&read(&p.etc.join("config.json"))).expect("config.json is JSON")
}

fn url(p: &Path) -> String {
    format!("file://{}", p.display())
}

#[test]
fn help_exits_0_and_documents_every_flag() {
    let (rc, out) = cmd(&["bash", &s(&repo().join("install.sh")), "--help"]).go();
    assert_eq!(rc, 0, "exit status\n{out}");
    for flag in [
        "--geoip",
        "--ref",
        "--repo",
        "--build",
        "--binary",
        "--no-cron",
        "--update",
        "--uninstall",
        "--purge",
        "--standalone",
        "--all-themes",
        "--theme",
        "-y",
        "--help",
    ] {
        assert_contains(&out, flag, "help text");
    }
}

#[test]
fn bad_flags_are_rejected_with_exit_2_and_nothing_is_touched() {
    let existing = s(&repo().join("Cargo.toml"));
    let cases: [&[&str]; 9] = [
        &["--bogus"],
        &["--ref"],
        &["--repo", "a/b/c"],
        &["--repo", "nonsense"],
        &["--ref", "../x"],
        &["--purge"],
        &["--binary"],
        &["--binary", "/nonexistent/webpulse"],
        &["--build", "--binary", &existing],
    ];
    for args in cases {
        let noop = TmpDir::new("noop");
        let mut argv = vec!["bash".to_string(), s(&repo().join("install.sh"))];
        argv.extend(args.iter().map(|a| a.to_string()));
        let (rc, out) = cmd_os(&argv).env("WEBPULSE_PREFIX", &noop.path).go();
        assert_eq!(rc, 2, "{}\n{out}", args.join(" "));
        assert!(
            list_tree(&noop.path).is_empty(),
            "{}: prefix touched",
            args.join(" ")
        );
    }
}

#[test]
fn refuses_to_run_as_non_root_without_a_test_prefix() {
    if euid() == 0 {
        skip!("running as root");
    }
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = cmd(&["bash", &s(&p.src.join("install.sh")), "-y"]).go();
    assert_eq!(rc, 1, "exit status\n{out}");
    assert_contains(&out, "must be run as root", "installer");
}

#[test]
fn fails_clearly_when_cpanel_is_missing_and_standalone_is_not_given() {
    let p = Sandbox::new();
    rmrf(&p.cpanel);
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "cPanel & WHM not found", "installer");
    assert_contains(&out, "--standalone", "installer");
    assert_missing(&p.link());
}

#[test]
fn no_php_is_needed_to_install_cpanel_php_is_only_an_informational_check() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    std::fs::remove_file(p.cpanel.join("3rdparty/bin/php")).unwrap();
    // a PATH without any php
    let bin = p.dir("nophp-bin");
    for tool in [
        "bash",
        "sh",
        "env",
        "tar",
        "cp",
        "mv",
        "ln",
        "rm",
        "mkdir",
        "chmod",
        "chown",
        "find",
        "sed",
        "grep",
        "diff",
        "awk",
        "cut",
        "tr",
        "head",
        "date",
        "id",
        "uname",
        "cat",
        "sort",
        "readlink",
        "dirname",
        "basename",
        "hostname",
        "cmp",
        "mktemp",
        "sha256sum",
        "touch",
        "printf",
        "wc",
        "tail",
        "sleep",
        "nohup",
        "gzip",
    ] {
        if let Some(path) = which(tool) {
            let _ = std::os::unix::fs::symlink(path, bin.join(tool));
        }
    }
    let (rc, out) = p.installer(&p.src, &["-y"]).env("PATH", &bin).go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "cPanel's PHP", "informational warning");
    assert_contains(&out, "WebPulse 0.1.0 is installed", "installer");
    assert!(
        std::fs::symlink_metadata(p.link()).is_ok_and(|m| m.file_type().is_symlink()),
        "installed"
    );
}

#[test]
fn fresh_install_registers_everything() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "WebPulse 0.1.0 is installed", "installer");
    assert_contains(&out, ":2087/cgi/webpulse/index.cgi", "installer");

    let link = p.link();
    let rel = p.releases().join("0.1.0");
    assert_eq!(
        readlink(&link),
        rel,
        "install dir is a symlink to the release"
    );

    // the binary is installed as is, root-owned 0755
    assert_eq!(
        read(&p.src.join("bin/webpulse")),
        read(&rel.join("bin/webpulse")),
        "binary copied unchanged"
    );
    assert_eq!(mode(&rel.join("bin/webpulse")), 0o755, "binary mode");
    assert_contains(&read(&p.engine_log), "--version", "binary self-test");
    for gone in ["lib", "ui", "bin/webpulse.php", "Makefile", "scripts"] {
        assert_missing(&rel.join(gone));
    }
    for f in [
        "cpanel/plugin/install.json",
        "whm/webpulse.conf",
        "etc/cron.d/webpulse",
        "etc/config.json.default",
        "installer/lib.sh",
        "install.sh",
        "uninstall.sh",
        "VERSION",
        "LICENSE",
    ] {
        assert_exists(&rel.join(f));
    }
    for f in [
        "hooks/stats_runall_pre.sh",
        "hooks/stats_runuser_pre.sh",
        "whm/cgi/index.cgi",
        "whm/cgi/api.cgi",
    ] {
        assert!(is_executable(&rel.join(f)), "{f} executable");
    }

    // config seeded for the redirected layout, then engine setup
    let cfg = config(&p);
    let paths = &cfg["paths"];
    assert_eq!(
        paths["data_dir"],
        json!(s(&p.var.join("data"))),
        "data_dir follows WEBPULSE_VAR"
    );
    assert_eq!(
        paths["userdomains_file"],
        json!(s(&p.prefix.join("etc/userdomains"))),
        "system paths follow the prefix"
    );
    assert!(
        paths["domlogs_dirs"][0]
            .as_str()
            .unwrap()
            .ends_with("/{user}"),
        "placeholders kept"
    );
    assert_eq!(
        paths["cron_file"],
        json!(s(&p.cron.join("webpulse"))),
        "cron_file follows WEBPULSE_CRON_DIR"
    );
    assert_eq!(
        paths["hooks_file"],
        json!(s(&p.prefix.join("var/cpanel/hooks/data/hooks.json"))),
        "every path follows the prefix"
    );
    assert_eq!(
        cfg["exclude_paths"],
        json!(["/.well-known/", "/cpanel"]),
        "URL paths are not prefixed"
    );
    assert_eq!(
        mode(&p.etc.join("config.json")) & 0o777,
        0o600,
        "config mode"
    );
    assert_contains(
        &read(&p.engine_log),
        &format!("setup [cfg={}]", p.etc.join("config.json").display()),
        "engine setup",
    );
    assert!(
        p.var.join("data").is_dir(),
        "engine setup created the data dir"
    );

    // cPanel plugin
    let theme = p.theme_dir();
    for f in [
        "index.live.php",
        "api.live.php",
        "webpulse.svg",
        "lib/LiveApiUser.php",
        "lib/CgiBridge.php",
        "VERSION",
    ] {
        assert_exists(&theme.join(f));
    }
    for f in [
        "lib/autoload.php",
        "lib/WebPulse",
        "lib/bootstrap.php",
        "ui",
    ] {
        assert_missing(&theme.join(f)); // no PHP library any more
    }
    assert_eq!(
        mode(&theme.join("index.live.php")) & 0o777,
        0o644,
        "shim mode"
    );
    // WHM plugin
    let cgi = p.cpanel.join("whostmgr/docroot/cgi/webpulse");
    assert!(
        is_executable(&cgi.join("index.cgi")) && is_executable(&cgi.join("api.cgi")),
        "CGIs executable"
    );
    assert_eq!(
        std::fs::read(repo().join("whm/addon_plugins/webpulse.png")).unwrap(),
        std::fs::read(p.cpanel.join("whostmgr/docroot/addon_plugins/webpulse.png")).unwrap(),
        "WHM icon"
    );
    assert_eq!(
        read(&repo().join("whm/webpulse.conf")),
        read(&p.apps.join("webpulse.conf")),
        "AppConfig"
    );

    // hooks + flag dir
    let hooks = read(&p.state.join("hooks"));
    let l = link.display();
    assert_contains(
        &hooks,
        &format!("{l}/hooks/stats_runall_pre.sh RunAll"),
        "hooks",
    );
    assert_contains(
        &hooks,
        &format!("{l}/hooks/stats_runuser_pre.sh RunUser"),
        "hooks",
    );
    assert_eq!(mode(&p.var.join("run/hook")), 0o1777, "hook dir mode");

    // cron
    let cron = read(&p.cron.join("webpulse"));
    assert_contains(
        &cron,
        &format!("*/5 * * * * root {l}/bin/webpulse collect --quiet"),
        "cron",
    );
    assert_contains(&cron, "collect --hooked --quiet", "cron");
    assert_not_contains(
        &cron,
        " root /usr/local/webpulse",
        "paths follow the prefix",
    );
    assert_eq!(mode(&p.cron.join("webpulse")) & 0o777, 0o644, "cron mode");

    // cPanel tools were driven exactly as documented
    let log = p.stub_log();
    for expected in [
        format!("install_plugin {l}/cpanel/plugin --theme jupiter"),
        format!("register_appconfig {l}/whm/webpulse.conf"),
        format!(
            "manage_hooks add script {l}/hooks/stats_runall_pre.sh --manual --category Stats --event RunAll --stage pre"
        ),
        format!(
            "manage_hooks add script {l}/hooks/stats_runuser_pre.sh --manual --category Stats --event RunUser --stage pre"
        ),
    ] {
        assert!(log.contains(&expected), "{expected}: {}", log.join("|"));
    }
    assert_exists(&p.state.join("plugin.jupiter"));
}

#[test]
fn update_is_idempotent_upgrades_via_a_new_release_and_prunes_nothing_needed() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 0, "{out}");
    let before = list_tree(&p.releases());

    // same version again: nothing new on disk, hooks not registered twice
    let (rc, out) = p.installer(&p.src, &["--update", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "updating in place", "installer");
    assert_contains(&out, "already registered", "installer");
    assert_eq!(before, list_tree(&p.releases()), "release tree unchanged");
    let log = p.stub_log();
    assert_eq!(
        count(&log, "manage_hooks add"),
        2,
        "hooks added exactly once each"
    );
    assert_eq!(count(&log, "install_plugin"), 2, "install_plugin re-run");

    // new version: new release dir, symlink swapped, old release kept, config untouched
    let cfg_file = p.etc.join("config.json");
    write(&cfg_file, read(&cfg_file) + "\n");
    let cfg_before = read(&cfg_file);
    let src2 = make_source(&p, "0.2.0", Some(&p.base.join("src2")), "stub");
    let (rc, out) = p.installer(&src2, &["--update", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        readlink(&p.link()),
        p.releases().join("0.2.0"),
        "symlink moved"
    );
    assert_exists(&p.releases().join("0.1.0/bin/webpulse"));
    assert_eq!(cfg_before, read(&cfg_file), "config preserved");
    assert_eq!(
        read(&p.theme_dir().join("VERSION")).trim(),
        "0.2.0",
        "plugin files updated"
    );
    assert_eq!(
        count(&p.stub_log(), "manage_hooks add"),
        2,
        "still no duplicate hooks"
    );
    let stages: Vec<String> = list_tree(&p.releases())
        .into_iter()
        .filter(|e| e.starts_with(".stage."))
        .collect();
    assert!(stages.is_empty(), "no staging leftovers: {stages:?}");
}

#[test]
fn same_version_with_changed_content_gets_its_own_release_directory() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 0, "{out}");
    write(&p.src.join("README.md"), "changed\n");
    let (rc, out) = p.installer(&p.src, &["--update", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    let target = readlink(&p.link());
    assert_ne!(target, p.releases().join("0.1.0"), "new release expected");
    assert_contains(
        &read(&target.join("README.md")),
        "changed",
        "new release content",
    );
}

#[test]
fn no_cron_and_standalone() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p.installer(&p.src, &["--no-cron", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_missing(&p.cron.join("webpulse"));
    assert_exists(&p.link());
    assert_contains(&out, "--no-cron", "installer");

    let q = Sandbox::new();
    rmrf(&q.cpanel); // no cPanel at all
    make_source(&q, "0.1.0", None, "stub");
    let (rc, out) = q.installer(&q.src, &["--standalone", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_exists(&q.link());
    assert_missing(&q.cron.join("webpulse"));
    assert!(q.stub_log().is_empty(), "no cPanel tool invoked");
    assert_contains(&out, "Standalone mode", "installer");
    assert!(q.etc.join("config.json").is_file(), "config created");
}

#[test]
fn all_themes_and_theme_install_into_the_extra_themes() {
    let p = Sandbox::new();
    std::fs::create_dir_all(p.cpanel.join("base/frontend/paper_lantern")).unwrap();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p.installer(&p.src, &["--all-themes", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    for t in ["jupiter", "paper_lantern"] {
        assert_exists(
            &p.cpanel
                .join(format!("base/frontend/{t}/webpulse/index.live.php")),
        );
        assert_exists(&p.state.join(format!("plugin.{t}")));
    }
    let (rc, out) = p.installer(&p.src, &["--theme", "nosuchtheme", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "Theme 'nosuchtheme' not found", "installer");
}

#[test]
fn geoip_enables_geoip_through_the_binary_and_runs_geoip_update() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p.installer(&p.src, &["--geoip", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        config(&p)["geoip"]["country_db"],
        json!(s(&p.var.join("geoip/dbip-country-lite.mmdb"))),
        "country_db follows WEBPULSE_VAR"
    );
    let log = read(&p.engine_log);
    let set = format!(
        "config set geoip.enabled true [cfg={}]",
        p.etc.join("config.json").display()
    );
    assert_contains(&log, &set, "engine log");
    assert_contains(&log, "geoip-update", "engine log");
    assert!(
        log.find("config set geoip.enabled true") < log.find("geoip-update"),
        "enabled before the download"
    );
}

#[test]
fn a_failing_cpanel_step_prints_the_command_and_rolls_everything_back() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    write(&p.state.join("fail.register_appconfig"), "");
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "Step failed: register_appconfig", "installer");
    assert_contains(
        &out,
        &format!(
            "Command: {}",
            p.cpanel.join("bin/register_appconfig").display()
        ),
        "installer",
    );
    assert_contains(&out, "simulated failure", "installer");
    assert_contains(&out, "Rolling back", "installer");

    let log = p.stub_log();
    let expected = format!(
        "uninstall_plugin {}/cpanel/plugin --theme jupiter",
        p.link().display()
    );
    assert!(
        log.contains(&expected),
        "plugin unregistered: {}",
        log.join("|")
    );
    assert_missing(&p.link());
    assert_missing(&p.releases().join("0.1.0"));
    assert_missing(&p.theme_dir());
    assert_missing(&p.cpanel.join("whostmgr/docroot/cgi/webpulse"));
    assert_missing(&p.cron.join("webpulse"));
    assert_missing(&p.apps.join("webpulse.conf"));
    assert_missing(&p.state.join("plugin.jupiter"));
    assert_missing(&p.etc.join("config.json"));
}

#[test]
fn a_failing_hook_registration_rolls_back_including_the_first_hook() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    // make the stub refuse the RunUser hook only
    let stub = p.cpanel.join("bin/manage_hooks");
    let body = read(&stub).replace(
        "add)",
        "add)\n    case \"$*\" in *RunUser*) echo 'cannot add RunUser' >&2; exit 1;; esac",
    );
    script(&stub, &body);
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(
        &out,
        "Step failed: register Stats::RunUser hook",
        "installer",
    );
    assert_eq!(
        read(&p.state.join("hooks")).trim(),
        "",
        "RunAll hook removed again"
    );
    assert_missing(&p.link());
    assert_missing(&p.apps.join("webpulse.conf"));
}

#[test]
fn a_failing_update_restores_the_previous_release() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 0, "{out}");

    let src2 = make_source(&p, "0.2.0", Some(&p.base.join("src2")), "stub");
    write(&p.state.join("fail.register_appconfig"), "");
    let (rc, out) = p.installer(&src2, &["--update", "-y"]).go();
    assert_eq!(rc, 1, "{out}");
    assert_eq!(
        readlink(&p.link()),
        p.releases().join("0.1.0"),
        "previous release active again"
    );
    assert_missing(&p.releases().join("0.2.0"));
    assert_eq!(
        read(&p.theme_dir().join("VERSION")).trim(),
        "0.1.0",
        "plugin files restored"
    );
    assert_exists(&p.apps.join("webpulse.conf")); // earlier registration is untouched
    assert_exists(&p.cron.join("webpulse"));
}

#[test]
fn engine_setup_failure_rolls_back_before_anything_is_activated() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p
        .installer(&p.src, &["-y"])
        .env("STUB_SETUP_FAIL", "1")
        .go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "setup exploded", "installer");
    assert_missing(&p.link());
    assert!(
        p.stub_log().is_empty(),
        "no cPanel tool invoked: {:?}",
        p.stub_log()
    );
}

#[test]
fn uninstall_reverses_everything_and_keeps_data_purge_removes_it() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 0, "{out}");
    write(&p.var.join("webpulse.db"), "precious");
    assert!(p.leftovers().len() > 10, "installed files present");
    let link = p.link();

    // uninstall from the installed copy, as documented
    let (rc, out) = cmd(&["bash", &s(&link.join("uninstall.sh")), "-y"])
        .env("WEBPULSE_PREFIX", &p.prefix)
        .env("WEBPULSE_FAKE_CPANEL", "1")
        .env("WEBPULSE_CPANEL_ROOT", &p.cpanel)
        .env("WEBPULSE_ETC", &p.etc)
        .env("WEBPULSE_VAR", &p.var)
        .env("WEBPULSE_CRON_DIR", &p.cron)
        .env("WEBPULSE_APPS_DIR", &p.apps)
        .env("NO_COLOR", "1")
        .stdin("")
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "uninstalled", "uninstaller");
    assert_eq!(
        p.leftovers(),
        Vec::<String>::new(),
        "nothing of WebPulse is left in prefix/cron/apps/cpanel"
    );

    let log = p.stub_log();
    assert!(
        log.contains(&"unregister_appconfig webpulse".to_string()),
        "unregister_appconfig"
    );
    let uninstall = format!(
        "uninstall_plugin {}/cpanel/plugin --theme jupiter",
        link.display()
    );
    assert!(
        log.contains(&uninstall),
        "uninstall_plugin: {}",
        log.join("|")
    );
    assert_eq!(
        count(&log, "manage_hooks delete script "),
        2,
        "both hooks deleted"
    );
    assert_eq!(
        read(&p.state.join("hooks")).trim(),
        "",
        "hook registry empty"
    );
    assert_exists(&p.var.join("webpulse.db"));
    assert_exists(&p.etc.join("config.json"));

    // a second uninstall is a harmless no-op
    let (rc, out) = p.installer(&repo(), &["--uninstall", "-y"]).go();
    assert_eq!(rc, 0, "{out}");

    // reinstall keeps the data, then purge
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        read(&p.var.join("webpulse.db")),
        "precious",
        "data survived reinstall"
    );
    let (rc, out) = p.installer(&p.src, &["--uninstall", "--purge", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_missing(&p.var);
    assert_missing(&p.etc);
    assert_eq!(p.leftovers(), Vec::<String>::new(), "clean after purge");
    let in_prefix: Vec<String> = list_tree(&p.prefix)
        .into_iter()
        .filter(|e| e.contains("webpulse"))
        .collect();
    assert!(in_prefix.is_empty(), "prefix clean: {in_prefix:?}");
}

#[test]
fn curl_pipe_bash_downloads_verifies_and_installs_the_release_tarball() {
    let p = Sandbox::new();
    let src = make_source(&p, "0.3.0", Some(&p.base.join("pristine")), "downloaded");
    let (tarball, _) = release_tarball(&src, &p.base.join("dl"), "x86_64-unknown-linux-musl");

    let (rc, out) = p
        .installer_piped(&["--no-cron"])
        .env("WEBPULSE_DOWNLOAD_URL", url(&tarball))
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "Downloading file://", "installer");
    assert_contains(&out, "Checksum OK", "installer");
    assert_contains(&out, "WebPulse 0.3.0 is installed", "installer");
    let rel = p.releases().join("0.3.0");
    assert_eq!(readlink(&p.link()), rel, "release");
    assert_contains(
        &read(&rel.join("bin/webpulse")),
        "downloaded",
        "the downloaded binary is installed",
    );
    let temp: Vec<String> = list_tree(&p.base)
        .into_iter()
        .filter(|e| e.starts_with("webpulse-install."))
        .collect();
    assert!(temp.is_empty(), "temp download dir removed: {temp:?}");

    // the defaults point at GitHub releases
    let script = read(&repo().join("install.sh"));
    for needle in [
        "releases/download/${tag}/webpulse-${ver}-${WP_TARGET}.tar.gz",
        "api.github.com/repos/${WP_REPO}/releases/latest",
        "WEBPULSE_REPO:-inalto/webpulse",
        "WEBPULSE_REF:-main",
    ] {
        assert_contains(&script, needle, "install.sh");
    }

    // a tarball that is not WebPulse is refused before anything is installed
    let bad = p.dir("badtar");
    write(&bad.join("x/README"), "hi");
    let bad_tar = bad.join("bad.tar.gz");
    let (rc, out) = cmd(&["tar", "-czf", &s(&bad_tar), "-C", &s(&bad), "x"]).go();
    assert_eq!(rc, 0, "{out}");
    write(
        &bad.join("bad.tar.gz.sha256"),
        format!("{}  bad.tar.gz\n", sha256_file(&bad_tar)),
    );
    let q = Sandbox::new();
    let (rc, out) = q
        .installer_piped(&["-y"])
        .env("WEBPULSE_DOWNLOAD_URL", url(&bad_tar))
        .go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "does not look like WebPulse", "installer");
    assert_missing(&q.link());

    // not a tar.gz at all
    let junk = bad.join("junk.tar.gz");
    write(&junk, "not an archive");
    write(
        &bad.join("junk.tar.gz.sha256"),
        format!("{}  junk.tar.gz\n", sha256_file(&junk)),
    );
    let (rc, out) = q
        .installer_piped(&["-y"])
        .env("WEBPULSE_DOWNLOAD_URL", url(&junk))
        .go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "not a valid tar.gz", "installer");
}

#[test]
fn the_checksum_is_mandatory_and_a_mismatch_installs_nothing() {
    let p = Sandbox::new();
    let src = make_source(&p, "0.3.0", Some(&p.base.join("pristine")), "stub");
    let (tarball, sha) = release_tarball(&src, &p.base.join("dl"), "x86_64-unknown-linux-musl");
    let run = || {
        p.installer_piped(&["-y"])
            .env("WEBPULSE_DOWNLOAD_URL", url(&tarball))
    };

    // tampered checksum
    write(&sha, format!("{}  x.tar.gz\n", "0".repeat(64)));
    let (rc, out) = run().go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "Checksum mismatch", "installer");
    assert_contains(&out, "Nothing was installed", "installer");
    assert_missing(&p.link());
    assert_missing(&p.releases());

    // garbage checksum file
    write(&sha, "oops\n");
    let (rc, out) = run().go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "Invalid checksum file", "installer");

    // missing checksum file
    std::fs::remove_file(&sha).unwrap();
    let (rc, out) = run().go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "Cannot verify the download", "installer");
    assert_missing(&p.link());

    // the hash can also be given explicitly
    let (rc, out) = p
        .installer_piped(&["-y", "--no-cron"])
        .env("WEBPULSE_DOWNLOAD_URL", url(&tarball))
        .env(
            "WEBPULSE_DOWNLOAD_SHA256",
            sha256_file(&tarball).to_uppercase(),
        )
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "Checksum OK", "installer");
}

#[test]
fn a_missing_release_asset_without_cargo_fails_with_a_clear_message() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "stub");
    let (rc, out) = p
        .installer_piped(&["-y"])
        .env("WEBPULSE_DOWNLOAD_URL", url(&p.base.join("nope.tar.gz")))
        .go();
    assert_eq!(rc, 1, "{out}");
    for needle in [
        "No prebuilt binary",
        "no release asset",
        "cargo is not installed",
        "--build",
        "--binary",
    ] {
        assert_contains(&out, needle, "installer");
    }
    assert_missing(&p.link());
    assert_missing(&p.releases());
}

#[test]
fn release_resolution_latest_tag_ref_and_architecture_against_github_urls() {
    if !have("sha256sum") {
        skip!("sha256sum not installed");
    }
    let p = Sandbox::new();
    let src = make_source(&p, "0.5.0", Some(&p.base.join("pristine")), "aarch64-build");
    let (tarball, sha) = release_tarball(&src, &p.base.join("dl"), "aarch64-unknown-linux-musl");
    let fake = p.dir("fakebin");
    let curl_log = p.base.join("curl.log");
    script(
        &fake.join("uname"),
        "#!/bin/sh\ncase \"$1\" in -m) echo \"${FAKE_ARCH:-aarch64}\";; -s) echo Linux;; *) exec /usr/bin/env uname \"$@\";; esac\n",
    );
    let tag = "v0.5.0";
    let base = "https://github.com/inalto/webpulse/releases/download";
    let curl = r#"#!/bin/bash
out=''; url=''
while [ $# -gt 0 ]; do
    case "$1" in
        -o) out=$2; shift ;;
        -w|--retry|--connect-timeout) shift ;;
        -*) ;;
        *) url=$1 ;;
    esac
    shift
done
echo "$url" >> '@LOG@'
code=404
case "$url" in
    https://api.github.com/repos/inalto/webpulse/releases/latest)
        printf '{"url":"x","tag_name": "@TAG@","name":"WebPulse"}' > "$out"; code=200 ;;
    @BASE@/@TAG@/webpulse-0.5.0-aarch64-unknown-linux-musl.tar.gz) cp '@TARBALL@' "$out"; code=200 ;;
    @BASE@/@TAG@/webpulse-0.5.0-aarch64-unknown-linux-musl.tar.gz.sha256) cp '@SHA@' "$out"; code=200 ;;
esac
[ "$code" = 200 ] || : > "$out"
printf '%s' "$code"
[ "$code" = 200 ]
"#
    .replace("@LOG@", &s(&curl_log))
    .replace("@TAG@", tag)
    .replace("@BASE@", base)
    .replace("@TARBALL@", &s(&tarball))
    .replace("@SHA@", &s(&sha));
    script(&fake.join("curl"), &curl);
    let path = format!(
        "{}:{}",
        fake.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let urls = || -> Vec<String> { read(&curl_log).lines().map(str::to_string).collect() };

    // no --ref: the latest release, arm64 asset
    let (rc, out) = p
        .installer_piped(&["-y", "--no-cron"])
        .env("PATH", &path)
        .go();
    assert_eq!(rc, 0, "{out}");
    let seen = urls();
    for u in [
        "https://api.github.com/repos/inalto/webpulse/releases/latest".to_string(),
        format!("{base}/{tag}/webpulse-0.5.0-aarch64-unknown-linux-musl.tar.gz"),
        format!("{base}/{tag}/webpulse-0.5.0-aarch64-unknown-linux-musl.tar.gz.sha256"),
    ] {
        assert!(seen.contains(&u), "{u} requested: {}", seen.join("\n"));
    }
    assert_contains(&out, "WebPulse 0.5.0 is installed", "installer");
    assert_contains(
        &read(&p.link().join("bin/webpulse")),
        "aarch64-build",
        "installed binary",
    );

    // --ref 0.5.0 and --ref v0.5.0 name the tag directly (no API call)
    for r in ["0.5.0", "v0.5.0"] {
        std::fs::remove_file(&curl_log).unwrap();
        let q = Sandbox::new();
        let (rc, out) = q
            .installer_piped(&["-y", "--no-cron", "--ref", r])
            .env("PATH", &path)
            .go();
        assert_eq!(rc, 0, "--ref {r}\n{out}");
        assert_not_contains(
            &read(&curl_log),
            "api.github.com",
            &format!("--ref {r} skips the API"),
        );
    }

    // a tag without assets, no cargo: clear failure naming the URL
    let q = Sandbox::new();
    let (rc, out) = q
        .installer_piped(&["-y", "--ref", "v9.9.9"])
        .env("PATH", &path)
        .go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(
        &out,
        &format!(
            "no release asset at {base}/v9.9.9/webpulse-9.9.9-aarch64-unknown-linux-musl.tar.gz"
        ),
        "installer",
    );

    // a branch has no binary
    let (rc, out) = q
        .installer_piped(&["-y", "--ref", "feature/x"])
        .env("PATH", &path)
        .go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "'feature/x' is a branch", "installer");

    // x86_64 picks the other asset name
    std::fs::remove_file(&curl_log).unwrap();
    let (rc, out) = q
        .installer_piped(&["-y", "--ref", "v0.5.0"])
        .env("PATH", &path)
        .env("FAKE_ARCH", "x86_64")
        .go();
    assert_eq!(rc, 1, "no x86_64 asset in the fake release\n{out}");
    assert_contains(
        &read(&curl_log),
        "webpulse-0.5.0-x86_64-unknown-linux-musl.tar.gz",
        "x86_64 asset",
    );

    // unsupported architecture
    let (rc, out) = q
        .installer_piped(&["-y"])
        .env("PATH", &path)
        .env("FAKE_ARCH", "riscv64")
        .go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "Unsupported CPU architecture 'riscv64'", "installer");
}

#[test]
fn binary_installs_the_given_binary_instead_of_the_one_in_the_release() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "bundled");
    let custom = script(
        &p.base.join("my-webpulse"),
        &stub_binary_body("custom-build"),
    );
    let (rc, out) = p.installer(&p.src, &["--binary", &s(&custom), "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "Using the provided binary", "installer");
    let installed = read(&p.link().join("bin/webpulse"));
    assert_contains(&installed, "custom-build", "installed binary");
    assert_not_contains(&installed, "bundled", "installed binary");
    assert_eq!(mode(&p.link().join("bin/webpulse")), 0o755, "mode");

    // WEBPULSE_BINARY is the same thing; a binary that cannot run is rejected before activation
    let q = Sandbox::new();
    make_source(&q, "0.1.0", None, "stub");
    let broken = script(&q.base.join("broken"), "#!/nonexistent/interpreter\n");
    let (rc, out) = q
        .installer(&q.src, &["-y"])
        .env("WEBPULSE_BINARY", &broken)
        .go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "does not run on this machine", "installer");
    assert_missing(&q.link());
    assert_missing(&q.releases().join("0.1.0"));
}

/// A source tree: the release layout without bin/, plus Cargo.toml and src/.
fn source_checkout(p: &Sandbox, version: &str, dir: &Path) -> PathBuf {
    let src = make_source(p, version, Some(dir), "stub");
    std::fs::remove_file(src.join("bin/webpulse")).unwrap();
    std::fs::remove_dir(src.join("bin")).unwrap();
    write(
        &src.join("Cargo.toml"),
        format!("[package]\nname = \"webpulse\"\nversion = \"{version}\"\n"),
    );
    std::fs::create_dir(src.join("src")).unwrap();
    src
}

#[test]
fn a_source_checkout_needs_a_built_binary_binary_or_build() {
    let p = Sandbox::new();
    let src = source_checkout(&p, "0.1.0", &p.src);

    let (rc, out) = p.installer(&src, &["-y"]).go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "source checkout without a built binary", "installer");
    assert_contains(&out, "cargo build --release", "installer");
    assert_missing(&p.link());

    // an existing target/release/webpulse is used
    script(
        &src.join("target/release/webpulse"),
        &stub_binary_body("from-target"),
    );
    let (rc, out) = p.installer(&src, &["-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "Using the existing build", "installer");
    assert_contains(
        &read(&p.link().join("bin/webpulse")),
        "from-target",
        "installed binary",
    );
    let sources: Vec<String> = list_tree(&p.releases().join("0.1.0"))
        .into_iter()
        .filter(|e| e.starts_with("src") || e.starts_with("target") || e == "Cargo.toml")
        .collect();
    assert!(sources.is_empty(), "sources are not installed: {sources:?}");

    // --build in a checkout without cargo fails clearly and installs nothing
    let q = Sandbox::new();
    let (rc, out) = q.installer(&src, &["--build", "-y"]).go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(
        &out,
        "cargo (the Rust toolchain) was not found",
        "installer",
    );
    assert_contains(&out, "--binary", "installer");
    assert_missing(&q.link());
}

/// The source tree as codeload serves it: one top-level `webpulse-main/`.
fn source_tarball(p: &Sandbox, tree: &Path) -> PathBuf {
    let tarball = p.base.join("source.tar.gz");
    let (rc, out) = cmd(&[
        "tar",
        "-czf",
        &s(&tarball),
        "--transform",
        "s,^\\.,webpulse-main,",
        "-C",
        &s(tree),
        ".",
    ])
    .go();
    assert_eq!(rc, 0, "{out}");
    tarball
}

#[test]
fn build_downloads_the_source_tarball_and_runs_cargo_build_release_locked() {
    let p = Sandbox::new();
    let tree = source_checkout(&p, "0.7.0", &p.base.join("srctree"));
    write(&tree.join("Cargo.lock"), "# lock\n");
    let tarball = source_tarball(&p, &tree);

    let cargo_log = p.base.join("cargo.log");
    let cargo = script(
        &p.base.join("fake-cargo"),
        &format!(
            "#!/bin/bash\necho \"$* (cwd=$(basename \"$PWD\"))\" >> '{}'\n\
             mkdir -p target/release\ncat > target/release/webpulse <<'EOS'\n{}EOS\nchmod 755 target/release/webpulse\n",
            cargo_log.display(),
            stub_binary_body("built-from-source")
        ),
    );
    let (rc, out) = p
        .installer_piped(&["--build", "-y", "--no-cron"])
        .env("WEBPULSE_SOURCE_URL", url(&tarball))
        .env("WEBPULSE_CARGO", &cargo)
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "Building WebPulse from source", "installer");
    assert_contains(&out, "WebPulse 0.7.0 is installed", "installer");
    assert_contains(
        &read(&p.link().join("bin/webpulse")),
        "built-from-source",
        "installed binary",
    );
    assert_contains(
        &read(&cargo_log),
        "build --release --locked",
        "cargo invocation",
    );

    // without cargo: clear failure, nothing installed
    let q = Sandbox::new();
    let (rc, out) = q
        .installer_piped(&["--build", "-y"])
        .env("WEBPULSE_SOURCE_URL", url(&tarball))
        .go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(
        &out,
        "cargo (the Rust toolchain) was not found",
        "installer",
    );
    assert_missing(&q.link());

    // a failing build is reported and nothing is installed
    let bad_cargo = script(
        &p.base.join("bad-cargo"),
        "#!/bin/sh\necho 'error: boom' >&2\nexit 101\n",
    );
    let r = Sandbox::new();
    let (rc, out) = r
        .installer_piped(&["--build", "-y"])
        .env("WEBPULSE_SOURCE_URL", url(&tarball))
        .env("WEBPULSE_CARGO", &bad_cargo)
        .go();
    assert_eq!(rc, 1, "{out}");
    assert_contains(&out, "cargo build failed", "installer");
    assert_missing(&r.link());

    // no release asset + cargo present: falls back to the source build
    let t = Sandbox::new();
    let (rc, out) = t
        .installer_piped(&["-y", "--no-cron"])
        .env("WEBPULSE_DOWNLOAD_URL", url(&t.base.join("missing.tar.gz")))
        .env("WEBPULSE_SOURCE_URL", url(&tarball))
        .env("WEBPULSE_CARGO", &cargo)
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "falling back to a build from source", "installer");
    assert_contains(
        &read(&t.link().join("bin/webpulse")),
        "built-from-source",
        "installed binary",
    );
}

#[test]
fn binary_with_curl_pipe_bash_fetches_only_the_support_files_source_tarball() {
    let p = Sandbox::new();
    let tree = make_source(&p, "0.8.0", Some(&p.base.join("srctree")), "stub");
    std::fs::remove_file(tree.join("bin/webpulse")).unwrap();
    std::fs::remove_dir(tree.join("bin")).unwrap();
    let tarball = source_tarball(&p, &tree);
    let custom = script(
        &p.base.join("my-webpulse"),
        &stub_binary_body("offline-binary"),
    );
    let (rc, out) = p
        .installer_piped(&["--binary", &s(&custom), "-y", "--no-cron"])
        .env("WEBPULSE_SOURCE_URL", url(&tarball))
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "WebPulse 0.8.0 is installed", "installer");
    assert_contains(
        &read(&p.link().join("bin/webpulse")),
        "offline-binary",
        "installed binary",
    );
}

#[test]
fn an_update_run_from_the_installed_copy_fetches_the_new_release_instead_of_reinstalling_itself() {
    let p = Sandbox::new();
    make_source(&p, "0.1.0", None, "old");
    let (rc, out) = p.installer(&p.src, &["-y"]).go();
    assert_eq!(rc, 0, "{out}");

    let new = make_source(&p, "0.2.0", Some(&p.base.join("new")), "new");
    let (tarball, _) = release_tarball(&new, &p.base.join("dl"), "x86_64-unknown-linux-musl");
    let (rc, out) = p
        .installer(&p.link(), &["--update", "-y"])
        .env("WEBPULSE_DOWNLOAD_URL", url(&tarball))
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "Downloading file://", "installer");
    assert_eq!(
        readlink(&p.link()),
        p.releases().join("0.2.0"),
        "moved to the new release"
    );
    assert_contains(
        &read(&p.link().join("bin/webpulse")),
        "new",
        "installed binary",
    );
}

#[test]
fn the_real_binary_installs_into_the_sandbox() {
    // the binary under test (target/{debug,release}/webpulse, built by cargo for this test)
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_webpulse"));
    let (rc, out) = cmd(&[&s(&bin), "setup", "--help"]).go();
    assert_eq!(rc, 0, "setup --help\n{out}");
    let p = Sandbox::new();
    let (rc, out) = p
        .installer(&repo(), &["-y", "--binary", &s(&bin)])
        .env("STUB_ENGINE_LOG", "/dev/null")
        .go();
    assert_eq!(rc, 0, "{out}");
    let version = read(&repo().join("VERSION")).trim().to_string();
    assert_contains(
        &out,
        &format!("WebPulse {version} is installed"),
        "installer",
    );
    let link = p.link();
    assert!(p.var.is_dir(), "setup created the data dir");
    assert!(p.etc.join("config.json").is_file(), "config seeded");
    let (rc, out) = cmd(&[&s(&link.join("bin/webpulse")), "--version"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, &version, "--version");
    let (rc, out) = cmd(&[&s(&link.join("bin/webpulse")), "status"])
        .env("WEBPULSE_CONFIG", p.etc.join("config.json"))
        .go();
    assert!(rc == 0 || rc == 1, "status ran (rc={rc})\n{out}");
    let (rc, out) = p.installer(&repo(), &["--uninstall", "--purge", "-y"]).go();
    assert_eq!(rc, 0, "{out}");
    assert_eq!(p.leftovers(), Vec::<String>::new(), "clean after purge");
}
