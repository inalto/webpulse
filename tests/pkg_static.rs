//! Static checks of the packaging: syntax of every shell/PHP/Python file, the
//! plugin descriptors against docs/CPANEL-INTEGRATION.md, icons, cron file,
//! Makefile targets, CI and release workflows, and what is left of PHP.

mod pkg_common;

use pkg_common::*;
use serde_json::{Value, json};

const SHELL_SCRIPTS: &[&str] = &[
    "install.sh",
    "uninstall.sh",
    "installer/lib.sh",
    "hooks/stats_runall_pre.sh",
    "hooks/stats_runuser_pre.sh",
    "scripts/build_dist.sh",
    "scripts/build_release.sh",
    "scripts/devenv.sh",
    "scripts/geoip_fixture.sh",
    "scripts/publish_wiki.sh",
    "whm/cgi/index.cgi",
    "whm/cgi/api.cgi",
];

/// The only PHP of the repository: the cPanel LiveAPI shims (cPanel requires PHP
/// or Perl for `*.live.php`) and the parity driver of the 0.1.0 release.
const PHP_FILES: &[&str] = &[
    "cpanel/webpulse/index.live.php",
    "cpanel/webpulse/api.live.php",
    "cpanel/webpulse/frame.live.php",
    "cpanel/webpulse/lib/LiveApiUser.php",
    "cpanel/webpulse/lib/CgiBridge.php",
];
const PHP_TEST_TOOLING: &[&str] = &["tests/parity/php_api.php"];

#[test]
fn bash_n_on_every_shell_script() {
    let r = repo();
    for f in SHELL_SCRIPTS {
        let (rc, out) = cmd(&["bash", "-n", &s(&r.join(f))]).go();
        assert_eq!(rc, 0, "bash -n {f}\n{out}");
    }
}

#[test]
fn shellcheck_warning_level_when_available() {
    if !have("shellcheck") {
        skip!("shellcheck not installed");
    }
    let r = repo();
    for f in SHELL_SCRIPTS {
        let (rc, out) = cmd(&["shellcheck", "-S", "warning", &s(&r.join(f))]).go();
        assert_eq!(rc, 0, "shellcheck {f}\n{out}");
    }
}

#[test]
fn scripts_are_executable_and_have_a_shebang() {
    let r = repo();
    for f in SHELL_SCRIPTS
        .iter()
        .filter(|f| **f != "installer/lib.sh")
        .chain(["scripts/make_icons.py"].iter())
    {
        assert!(is_executable(&r.join(f)), "{f} must be executable");
        assert!(read(&r.join(f)).starts_with("#!"), "{f} shebang");
    }
}

#[test]
fn php_l_on_the_cpanel_shims_and_their_library() {
    let Some(php) = php() else {
        skip!("php not installed");
    };
    let r = repo();
    for f in PHP_FILES.iter().chain(PHP_TEST_TOOLING) {
        let (rc, out) = cmd(&[&s(&php), "-l", &s(&r.join(f))]).go();
        assert_eq!(rc, 0, "php -l {f}\n{out}");
    }
}

#[test]
fn whm_cgis_are_bash_shims_that_keep_the_whm_markers_and_exec_the_binary() {
    let r = repo();
    for (f, exec) in [
        (
            "index.cgi",
            r#"exec "$bin" cgi --whm --page --api-url api.cgi"#,
        ),
        ("api.cgi", r#"exec "$bin" cgi --whm --api"#),
    ] {
        let src = read(&r.join("whm/cgi").join(f));
        let lines: Vec<&str> = src.split('\n').collect();
        assert_eq!(lines[0], "#!/bin/bash", "{f} shebang");
        assert_eq!(
            lines[1], "#WHMADDON:webpulse:WebPulse Analytics:webpulse.png",
            "{f} WHMADDON"
        );
        assert_eq!(lines[2], "#ACLS:all", "{f} ACLS");
        assert_contains(&src, exec, &format!("{f} exec line"));
        assert_contains(&src, "WEBPULSE_BIN", f);
        assert_contains(&src, "/usr/local/webpulse/bin/webpulse", f);
        assert_not_contains(&src.to_lowercase(), "php", &format!("{f} has no PHP left"));
    }
}

#[test]
fn the_php_implementation_is_gone_and_the_shims_are_thin() {
    let r = repo();
    for gone in [
        "lib",
        "bin",
        "whm/lib",
        "cpanel/webpulse/lib/bootstrap.php",
        "tests/run.php",
        "tests/cases",
        "tests/support",
        "tests/gen_logs.php",
    ] {
        assert_missing(&r.join(gone));
    }
    // every .php file of the repository is a shim or the parity driver
    let mut php: Vec<String> = Vec::new();
    for top in [
        "cpanel",
        "whm",
        "hooks",
        "installer",
        "scripts",
        "etc",
        "src",
        "ui",
        "tests",
        "docs",
        "wiki",
    ] {
        for rel in list_tree(&r.join(top)) {
            let path = format!("{top}/{rel}");
            if path.ends_with(".php") && !path.starts_with("tests/tmp/") {
                php.push(path);
            }
        }
    }
    php.sort();
    let mut allowed: Vec<String> = PHP_FILES
        .iter()
        .chain(PHP_TEST_TOOLING)
        .map(|s| s.to_string())
        .collect();
    allowed.sort();
    assert_eq!(php, allowed, "PHP files of the repository");

    for f in ["index.live.php", "api.live.php", "frame.live.php"] {
        let src = read(&r.join("cpanel/webpulse").join(f));
        assert_contains(&src, "CgiBridge::run", &format!("{f} runs the binary"));
        assert_not_contains(&src, "autoload", &format!("{f} has no autoloader"));
        assert_not_contains(
            &src,
            "WebPulse\\App",
            &format!("{f} does not boot a PHP engine"),
        );
        assert!(src.matches('\n').count() < 70, "{f} stays thin");
    }
    let lib = read(&r.join("installer/lib.sh"));
    for gone in [
        "PHP_BIN",
        "wp_json_patch",
        "wp_detect_php",
        "wp_php_ok",
        r#"cp -a "$rel/lib/.""#,
    ] {
        assert_not_contains(
            &lib,
            gone,
            &format!("installer/lib.sh no longer has {gone}"),
        );
    }
}

#[test]
fn python_scripts_parse() {
    if !have("python3") {
        skip!("python3 not installed");
    }
    let (rc, out) = cmd(&[
        "python3",
        "-I",
        "-c",
        "import ast,sys; ast.parse(open(sys.argv[1]).read())",
        &s(&repo().join("scripts/make_icons.py")),
    ])
    .go();
    assert_eq!(rc, 0, "{out}");
}

#[test]
fn install_json_matches_cpanel_integration_md() {
    let r = repo();
    let v: Value = serde_json::from_str(&read(&r.join("cpanel/plugin/install.json"))).unwrap();
    let list = v.as_array().expect("install.json is a list");
    assert_eq!(list.len(), 1, "install.json is a one-element list");
    let e = &list[0];
    assert_eq!(
        *e,
        json!({"id": "webpulse", "name": "WebPulse Analytics",
            "description": "Visitor statistics generated from your access logs",
            "uri": "webpulse/index.live.php", "icon": "webpulse.svg", "group_id": "metrics",
            "order": 1, "type": "link", "featuremanager": 1, "feature": "webpulse"}),
        "install.json entry"
    );
    // same key order as the documented example
    let keys: Vec<&str> = e.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "id",
            "name",
            "description",
            "uri",
            "icon",
            "group_id",
            "order",
            "type",
            "featuremanager",
            "feature"
        ],
        "install.json key order"
    );
    assert_exists(&r.join("cpanel/plugin").join(e["icon"].as_str().unwrap()));
    assert_exists(&r.join("cpanel").join(e["uri"].as_str().unwrap()));
}

#[test]
fn webpulse_conf_appconfig_matches_cpanel_integration_md() {
    let r = repo();
    let mut conf: Vec<(String, String)> = Vec::new();
    for line in read(&r.join("whm/webpulse.conf")).split('\n') {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("bad AppConfig line: {line}"));
        conf.push((k.to_string(), v.to_string()));
    }
    let expected: Vec<(String, String)> = [
        ("name", "webpulse"),
        ("service", "whostmgr"),
        ("url", "/cgi/webpulse/index.cgi"),
        ("url2", "/cgi/webpulse/api.cgi"),
        ("user", "root"),
        ("acls", "all,list-accts"),
        ("displayname", "WebPulse Analytics"),
        ("entryurl", "webpulse/index.cgi"),
        ("icon", "webpulse.png"),
        ("target", "_self"),
        (
            "searchtext",
            "analytics statistics visitors logs awstats webalizer",
        ),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    assert_eq!(conf, expected, "AppConfig keys");
    assert_exists(&r.join("whm/addon_plugins/webpulse.png"));
    assert_exists(&r.join("whm/cgi/index.cgi"));
}

#[test]
fn png_icon_is_a_48x48_rgba_image_with_transparency() {
    use std::io::Read;
    let d = std::fs::read(repo().join("whm/addon_plugins/webpulse.png")).unwrap();
    assert_eq!(&d[..8], b"\x89PNG\r\n\x1a\n", "PNG magic");
    assert_eq!(&d[12..16], b"IHDR", "first chunk");
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    assert_eq!(be32(&d[16..20]), 48, "width");
    assert_eq!(be32(&d[20..24]), 48, "height");
    assert_eq!(d[24], 8, "bit depth");
    assert_eq!(d[25], 6, "colour type RGBA");
    assert_eq!(&d[d.len() - 8..d.len() - 4], b"IEND", "last chunk");
    // decode: corner pixel fully transparent, centre opaque
    let mut pos = 8;
    let mut idat = Vec::new();
    while pos + 8 <= d.len() {
        let len = be32(&d[pos..]) as usize;
        if &d[pos + 4..pos + 8] == b"IDAT" {
            idat.extend_from_slice(&d[pos + 8..pos + 8 + len]);
        }
        pos += 12 + len;
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&idat[..])
        .read_to_end(&mut raw)
        .unwrap();
    let stride = 1 + 48 * 4;
    assert_eq!(raw.len(), 48 * stride, "decoded size");
    assert_eq!(raw[1 + 3], 0, "top-left alpha");
    let inside = 24 * stride + 1 + 4 * 6;
    assert_eq!(raw[inside + 3], 255, "inside alpha");
}

#[test]
fn meridian_hub_manifest_and_remote_entry_are_valid() {
    let r = repo();
    let m: Value = serde_json::from_str(&read(&r.join("cpanel/meridian/manifest.json"))).unwrap();
    assert_eq!(
        m["slug"], "webpulse",
        "slug = theme dir of the plugin files"
    );
    assert_eq!(m["contractVersion"], "1.0.0");
    assert_eq!(m["entrypoints"]["mf"], "./remoteEntry.js");
    assert_eq!(m["nav"]["path"], "/webpulse");
    assert_eq!(m["nav"]["section"], "monitor");
    assert_eq!(m["platforms"], json!(["cpanel"]));
    assert_eq!(
        m["dependencies"]["required"],
        json!(["webpulse"]),
        "hidden without the feature"
    );
    assert_eq!(
        m["version"],
        read(&r.join("VERSION")).trim(),
        "manifest version = VERSION"
    );
    for key in ["id", "displayName", "icon", "loadPriority", "searchIndex"] {
        assert!(m.get(key).is_some(), "manifest has {key}");
    }
    let js = read(&r.join("cpanel/meridian/remoteEntry.js"));
    assert_contains(&js, "export function init", "federation init");
    assert_contains(&js, "export function get", "federation get");
    assert_contains(&js, "WebPulseHub", "component name ends with Hub");
    assert_contains(&js, "frame.live.php", "embeds the frame page");
    if have("node") {
        let (rc, out) = cmd(&[
            "node",
            "--input-type=module",
            "-e",
            &format!(
                "import('file://{}').then(m => {{ if (typeof m.init !== 'function' || typeof m.get !== 'function') process.exit(2); }})",
                r.join("cpanel/meridian/remoteEntry.js").display()
            ),
        ])
        .go();
        assert_eq!(rc, 0, "remoteEntry.js is a valid ES module\n{out}");
    }
}

#[test]
fn meridian_left_menu_entry_is_valid() {
    let r = repo();
    let yaml = read(&r.join("cpanel/plugin/menu/LeftMenu.yaml"));
    let lines: Vec<&str> = yaml.lines().collect();
    assert_eq!(lines.len(), 3, "one entry with key/order/icon: {yaml}");
    assert_eq!(
        lines[0], "- key: webpulse",
        "key must equal install.json id"
    );
    let order = lines[1].trim().strip_prefix("order: ").expect("order line");
    order.parse::<u32>().expect("order is an integer");
    let icon = lines[2].trim().strip_prefix("icon: ").expect("icon line");
    assert!(
        icon.starts_with("<svg ") && icon.ends_with("</svg>"),
        "inline svg"
    );
    assert_contains(icon, r#"viewBox="0 0 24 24""#, "24x24 viewBox");
    assert_contains(icon, "var(--cp-tertiary-contrast)", "theme colour variable");
    for forbidden in ["<defs", "url(#", "<title", "<style", " id="] {
        assert!(!icon.contains(forbidden), "{forbidden} in menu icon");
    }
    // the installer ships the whole descriptor directory to /var/cpanel/plugins/webpulse
    let json = read(&r.join("cpanel/plugin/install.json"));
    assert_contains(&json, r#""id": "webpulse""#, "install.json id");
}

#[test]
fn svg_icons_are_well_formed_48x48_and_identical() {
    let r = repo();
    let paths = [
        r.join("cpanel/webpulse/webpulse.svg"),
        r.join("cpanel/plugin/webpulse.svg"),
    ];
    let attr = regex::Regex::new(r#"<svg\b[^>]*"#).unwrap();
    for p in &paths {
        let svg = read(p);
        let tag = attr
            .find(&svg)
            .unwrap_or_else(|| panic!("{}: no <svg> element", p.display()))
            .as_str();
        assert_contains(tag, r#"viewBox="0 0 48 48""#, "viewBox");
        // Jupiter flattens all icons into one sprite SVG and drops <defs>: a gradient or
        // any url(#id) reference renders as an invisible square there.
        for forbidden in ["<defs", "url(#", "<title", "<style", " id="] {
            assert!(
                !svg.contains(forbidden),
                "{}: {forbidden} is not sprite-safe",
                p.display()
            );
        }
        assert!(
            svg.trim_end().ends_with("</svg>"),
            "{}: closed",
            p.display()
        );
        if have("python3") {
            let (rc, out) = cmd(&[
                "python3",
                "-I",
                "-c",
                "import sys, xml.dom.minidom as m; m.parse(sys.argv[1])",
                &s(p),
            ])
            .go();
            assert_eq!(rc, 0, "{} is well-formed XML: {out}", p.display());
        }
    }
    assert_eq!(read(&paths[0]), read(&paths[1]), "plugin icon copy");
}

#[test]
fn cron_file_is_valid_cron_d_syntax() {
    let var = regex::Regex::new(r"^([A-Z]+)=(.*)$").unwrap();
    let job = regex::Regex::new(
        r"^(\S+)\s+(\S+)\s+(\S+)\s+(\S+)\s+(\S+)\s+root\s+/usr/local/webpulse/bin/webpulse\s+(.+)$",
    )
    .unwrap();
    let mut vars = std::collections::HashMap::new();
    let mut jobs: Vec<(String, String)> = Vec::new();
    for l in read(&repo().join("etc/cron.d/webpulse")).lines() {
        let l = l.trim_end();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(m) = var.captures(l) {
            vars.insert(m[1].to_string(), m[2].to_string());
            continue;
        }
        let m = job
            .captures(l)
            .unwrap_or_else(|| panic!("bad cron line: {l}"));
        let when: Vec<&str> = (1..=5).map(|i| m.get(i).unwrap().as_str()).collect();
        jobs.push((when.join(" "), m[6].to_string()));
    }
    assert_eq!(
        vars.get("MAILTO").map(String::as_str),
        Some("\"\""),
        "MAILTO"
    );
    assert!(
        vars.get("PATH").is_some_and(|p| p.contains("/usr/bin")),
        "PATH"
    );
    let expected: Vec<(String, String)> = [
        ("*/5 * * * *", "collect --quiet"),
        ("* * * * *", "collect --hooked --quiet"),
        ("10 2 * * *", "rollup --quiet"),
        ("30 4 1 * *", "geoip-update --quiet"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    assert_eq!(jobs, expected, "cron jobs");
}

#[test]
fn makefile_has_the_documented_targets_and_no_php_runner() {
    let r = repo();
    let mk = read(&r.join("Makefile"));
    for target in [
        "build",
        "test",
        "lint",
        "fixtures",
        "serve",
        "serve-admin",
        "dist",
        "release",
        "geoip-fixture",
        "clean",
    ] {
        assert!(
            mk.lines().any(|l| l.starts_with(&format!("{target}:"))),
            "make {target} exists"
        );
    }
    let expanded = mk.replace("$(CARGO)", "cargo");
    assert_contains(&expanded, "cargo build --release", "Makefile");
    assert_contains(&expanded, "cargo test --release", "Makefile");
    assert_contains(&mk, "tests/parity/run.sh", "Makefile");
    assert_contains(&mk, "scripts/build_release.sh", "Makefile");
    assert_not_contains(&mk, "tests/run.php", "no PHP test runner");
    assert_not_contains(&mk, "bin/webpulse serve", "no PHP launcher");
    if have("make") {
        let (rc, out) = cmd(&[
            "make",
            "-n",
            "-C",
            &s(&r),
            "test",
            "lint",
            "dist",
            "release",
            "serve",
            "serve-admin",
            "fixtures",
            "geoip-fixture",
        ])
        .go();
        assert_eq!(rc, 0, "make -n: {out}");
    }
}

#[test]
fn ci_and_release_workflows() {
    let r = repo();
    let ci = read(&r.join(".github/workflows/ci.yml"));
    for needle in [
        "cargo fmt --check",
        "cargo clippy --all-targets -- -D warnings",
        "cargo test --release",
        "shivammathur/setup-php",
        "shellcheck",
        "tests/parity/run.sh",
    ] {
        assert_contains(&ci, needle, "ci.yml");
    }
    assert_not_contains(&ci, "tests/run.php", "ci.yml has no PHP test runner");
    let rel = read(&r.join(".github/workflows/release.yml"));
    for needle in [
        "tags:",
        "'v*'",
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
        "cargo-zigbuild",
        "setup-zig",
        "scripts/build_release.sh",
        "scripts/build_dist.sh",
        ".sha256",
    ] {
        assert_contains(&rel, needle, "release.yml");
    }
    if have("python3") {
        let (rc, out) = cmd(&[
            "python3",
            "-I",
            "-c",
            "import sys\ntry:\n    import yaml\nexcept ImportError:\n    sys.exit(3)\nfor f in sys.argv[1:]:\n    yaml.safe_load(open(f))",
            &s(&r.join(".github/workflows/ci.yml")),
            &s(&r.join(".github/workflows/release.yml")),
        ])
        .go();
        assert!(rc == 0 || rc == 3, "workflow YAML parses\n{out}");
    }
}

#[test]
fn changelog_install_doc_version_and_cargo_metadata() {
    let r = repo();
    let changelog = read(&r.join("CHANGELOG.md"));
    assert_contains(&changelog, "## [0.1.0]", "CHANGELOG");
    assert_contains(&changelog, "## [0.2.0]", "CHANGELOG");
    assert_contains(&changelog, "Rewritten in Rust", "CHANGELOG");
    assert_contains(
        &read(&r.join("docs/INSTALL.md")),
        "Troubleshooting",
        "INSTALL.md",
    );
    let cargo = read(&r.join("Cargo.toml"));
    for key in ["description", "license", "repository", "readme"] {
        assert!(
            cargo
                .lines()
                .any(|l| l.starts_with(key) && l[key.len()..].trim_start().starts_with('=')),
            "Cargo.toml [package] has {key}"
        );
    }
    // one version everywhere
    let version = read(&r.join("VERSION")).trim().to_string();
    assert!(
        cargo.contains(&format!("version = \"{version}\"")),
        "Cargo.toml version is VERSION ({version})"
    );
    assert_eq!(env!("CARGO_PKG_VERSION"), version, "crate version");
    assert_contains(
        &changelog,
        &format!("## [{version}]"),
        "CHANGELOG has the current version",
    );
}
