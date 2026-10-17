//! scripts/build_dist.sh (source tarball, git archive) and scripts/build_release.sh
//! (binary tarball): the archives have the layout install.sh consumes.

mod pkg_common;

use pkg_common::*;
use serde_json::Value;

#[test]
fn build_dist_archives_head_and_the_installer_accepts_the_source_tarball() {
    if !have("git") {
        skip!("git not installed");
    }
    let p = Sandbox::new();
    let gitrepo = p.base.join("gitrepo");
    make_source(&p, "0.4.2", Some(&gitrepo), "stub");
    cp_a(&repo().join("scripts"), &gitrepo.join("scripts"));
    for args in [
        vec!["init", "-q"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "test"],
    ] {
        let mut argv = vec![
            "git",
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ];
        argv.extend(&args);
        let (rc, out) = cmd(&argv).cwd(&gitrepo).go();
        assert_eq!(rc, 0, "{}: {out}", args.join(" "));
    }
    // an uncommitted change must not end up in the archive
    write(&gitrepo.join("VERSION"), "9.9.9\n");

    let (rc, out) = cmd(&[
        "bash",
        &s(&gitrepo.join("scripts/build_dist.sh")),
        &s(&gitrepo.join("dist")),
    ])
    .cwd(&gitrepo)
    .go();
    assert_eq!(rc, 0, "{out}");
    let tarball = gitrepo.join("dist/webpulse-0.4.2.tar.gz");
    assert_exists(&tarball);
    assert_contains(&out, "uncommitted changes", "warning about the dirty tree");
    let (_, listing) = cmd(&["tar", "-tzf", &s(&tarball)]).go();
    for f in [
        "webpulse-0.4.2/installer/lib.sh",
        "webpulse-0.4.2/install.sh",
        "webpulse-0.4.2/VERSION",
    ] {
        assert_contains(&listing, &format!("{f}\n"), "archive content");
    }
    assert_not_contains(&listing, "/.git/", "no .git in the archive");
    let (_, version) = cmd(&["tar", "-xzOf", &s(&tarball), "webpulse-0.4.2/VERSION"]).go();
    assert_eq!(
        version, "0.4.2\n",
        "committed VERSION, not the working copy"
    );

    // the tarball is what codeload serves; with --binary only the support files are taken from it
    let q = Sandbox::new();
    let custom = script(&q.base.join("my-webpulse"), &stub_binary_body("dist-test"));
    let (rc, out) = q
        .installer_piped(&["-y", "--binary", &s(&custom)])
        .env(
            "WEBPULSE_SOURCE_URL",
            format!("file://{}", tarball.display()),
        )
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(&out, "WebPulse 0.4.2 is installed", "installer");
}

#[test]
fn build_release_packs_the_binary_tarball_with_the_installer_layout_and_the_installer_installs_it()
{
    let r = repo();
    let p = Sandbox::new();
    let stub = script(
        &p.base.join("built-webpulse"),
        &stub_binary_body("release-script"),
    );
    let out_dir = p.base.join("dist");
    let build = |out: &std::path::Path| {
        cmd(&[
            "bash",
            &s(&r.join("scripts/build_release.sh")),
            "--binary",
            &s(&stub),
            "--target",
            "x86_64-unknown-linux-musl",
            "--out",
            &s(out),
        ])
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .go()
    };
    let (rc, log) = build(&out_dir);
    assert_eq!(rc, 0, "{log}");
    let version = read(&r.join("VERSION")).trim().to_string();
    let name = format!("webpulse-{version}-x86_64-unknown-linux-musl");
    let tarball = out_dir.join(format!("{name}.tar.gz"));
    let sha_file = out_dir.join(format!("{name}.tar.gz.sha256"));
    assert_exists(&tarball);
    assert_exists(&sha_file);

    // checksum file: "<hash>  <file name>", matches the tarball
    assert_eq!(
        read(&sha_file).trim(),
        format!("{}  {name}.tar.gz", sha256_file(&tarball)),
        "sha256 file"
    );

    let (_, listing) = cmd(&["tar", "-tzf", &s(&tarball)]).go();
    for f in [
        "bin/webpulse",
        "installer/lib.sh",
        "install.sh",
        "uninstall.sh",
        "VERSION",
        "LICENSE",
        "README.md",
        "etc/config.json.default",
        "etc/cron.d/webpulse",
        "cpanel/plugin/install.json",
        "cpanel/webpulse/index.live.php",
        "cpanel/webpulse/api.live.php",
        "cpanel/webpulse/lib/LiveApiUser.php",
        "cpanel/webpulse/lib/CgiBridge.php",
        "whm/cgi/index.cgi",
        "whm/cgi/api.cgi",
        "whm/webpulse.conf",
        "whm/addon_plugins/webpulse.png",
        "whm/templates/webpulse/index.tmpl",
        "hooks/stats_runall_pre.sh",
        "hooks/stats_runuser_pre.sh",
        "cockpit/manifest.json",
        "cockpit/README.md",
    ] {
        assert_contains(&listing, &format!("{name}/{f}\n"), "archive content");
    }
    for absent in [
        "/src/",
        "/target/",
        "/tests/",
        "/lib/WebPulse",
        "/ui/",
        "/.git/",
    ] {
        assert_not_contains(
            &listing,
            &format!("{name}{absent}"),
            "not part of the binary tarball",
        );
    }
    let (_, mode_line) = cmd(&[
        "tar",
        "-tvzf",
        &s(&tarball),
        &format!("{name}/bin/webpulse"),
    ])
    .go();
    assert!(
        mode_line.trim().starts_with("-rwxr-xr-x"),
        "binary mode in the archive: {mode_line}"
    );
    assert_eq!(
        listing.lines().next(),
        Some(format!("{name}/").as_str()),
        "single top-level directory"
    );
    let tops: std::collections::BTreeSet<&str> = listing
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split('/').next().unwrap())
        .collect();
    assert_eq!(tops.len(), 1, "one top-level entry only: {tops:?}");

    // reproducible with the same SOURCE_DATE_EPOCH
    let out2 = p.base.join("dist2");
    let (rc, log) = build(&out2);
    assert_eq!(rc, 0, "{log}");
    assert_eq!(
        sha256_file(&tarball),
        sha256_file(&out2.join(format!("{name}.tar.gz"))),
        "reproducible tarball"
    );

    // the installer consumes exactly this tarball (the stub config default is not
    // needed: the real one is shipped)
    let (rc, res) = p
        .installer_piped(&["-y"])
        .env(
            "WEBPULSE_DOWNLOAD_URL",
            format!("file://{}", tarball.display()),
        )
        .go();
    assert_eq!(rc, 0, "{res}");
    assert_contains(
        &res,
        &format!("WebPulse {version} is installed"),
        "installer",
    );
    assert_contains(
        &read(&p.link().join("bin/webpulse")),
        "release-script",
        "installed binary",
    );
    assert_exists(&p.theme_dir().join("lib/CgiBridge.php"));
    assert_exists(&p.cpanel.join("whostmgr/docroot/cgi/webpulse/index.cgi"));
    let cfg: Value = serde_json::from_str(&read(&p.etc.join("config.json"))).unwrap();
    assert_eq!(
        cfg["paths"]["data_dir"].as_str(),
        Some(s(&p.var.join("data")).as_str()),
        "the shipped default config is seeded for the redirected layout"
    );
}

#[test]
fn build_release_refuses_a_missing_binary_a_bad_target_and_a_tag_that_is_not_version() {
    let r = repo();
    let p = Sandbox::new();
    let build = s(&r.join("scripts/build_release.sh"));
    let stub = s(&script(&p.base.join("b"), "#!/bin/sh\n"));
    let out = s(&p.base.join("o"));
    let (rc, log) = cmd(&[
        "bash",
        &build,
        "--binary",
        &s(&p.base.join("missing")),
        "--out",
        &out,
    ])
    .go();
    assert_eq!(rc, 1, "missing binary\n{log}");
    let (rc, log) = cmd(&[
        "bash", &build, "--binary", &stub, "--target", "../x", "--out", &out,
    ])
    .go();
    assert_eq!(rc, 2, "bad target\n{log}");
    let (rc, log) = cmd(&["bash", &build, "--binary", &stub, "--out", &out])
        .env("WEBPULSE_RELEASE_TAG", "v99.0.0")
        .go();
    assert_eq!(rc, 1, "{log}");
    assert_contains(&log, "does not match VERSION", "tag check");
}
