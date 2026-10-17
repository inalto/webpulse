//! Standalone mode (docs/STANDALONE-COCKPIT.md, Phase A): the Apache and nginx
//! readers on realistic EL / Debian / nginx layouts (tests/fixtures/standalone),
//! precedence and de-duplication, `webpulse sites`, collection end to end on a
//! fake standalone server (two sites, logrotate with and without dateext, zero
//! loss across a rotation), plain and gzip archives for backfill (never twice,
//! even when logrotate renames them) and the `mode` switch.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use serde_json::{Value, json};
use webpulse::engine::standalone::{self, FormatKind, Found, Site, Source, apache, nginx};

fn fixture(rel: &str) -> PathBuf {
    repo().join("tests/fixtures/standalone").join(rel)
}

fn sites<'a>(f: &'a Found, domain: &str) -> Vec<&'a Site> {
    f.sites.iter().filter(|s| s.domain == domain).collect()
}

fn one<'a>(f: &'a Found, domain: &str) -> &'a Site {
    let v = sites(f, domain);
    assert_eq!(v.len(), 1, "{domain}: {:#?}", f.sites);
    v[0]
}

fn paths(s: &Site) -> Vec<String> {
    s.logs
        .iter()
        .map(|l| l.path.display().to_string())
        .collect()
}

fn me() -> (u32, String) {
    let uid = unsafe { libc::geteuid() };
    (
        uid,
        webpulse::util::fs::user_name(uid).unwrap_or_else(|| "local".into()),
    )
}

#[test]
fn apache_el_layout() {
    let dir = fixture("el/etc/httpd");
    let f = apache::parse(&dir, "host.test").expect("EL layout recognised");
    let d = dir.display().to_string();
    // files read: main, modules, conf.d, and the absolute include mapped onto the copy
    for rel in [
        "conf/httpd.conf",
        "conf.modules.d/00-base.conf",
        "conf.d/ssl.conf",
        "sites/blog.conf",
    ] {
        assert!(
            f.files.contains(&dir.join(rel)),
            "{rel} read: {:?}",
            f.files
        );
    }
    // the main server: CustomLog "logs/access_log" combined, relative to the server root
    let main = sites(&f, "host.test");
    assert_eq!(main.len(), 2, "main server + _default_:443 vhost");
    assert_eq!(paths(main[0]), [format!("{d}/logs/access_log")]);
    assert_eq!(main[0].logs[0].kind, FormatKind::Combined);
    assert!(!main[0].logs[0].https);
    // ssl.conf: TransferLog = Apache's default format (common), ssl_request_log is not a hit log
    let ssl = main[1];
    assert_eq!(
        paths(ssl),
        [
            format!("{d}/logs/ssl_access_log"),
            format!("{d}/logs/ssl_request_log")
        ]
    );
    assert!(ssl.logs.iter().all(|l| l.https));
    assert_eq!(ssl.logs[0].kind, FormatKind::Common);
    assert_eq!(ssl.logs[1].kind, FormatKind::Incompatible);
    assert!(
        ssl.warnings
            .iter()
            .any(|w| w.contains("not combined-compatible"))
    );
    // example.com: http + https vhosts, suexec owner, alias
    let ex = sites(&f, "example.com");
    assert_eq!(ex.len(), 2);
    assert!(
        ex.iter()
            .all(|s| s.owner == "alice" && s.source == Source::Apache)
    );
    assert_eq!(ex[0].aliases, ["www.example.com"]);
    assert_eq!(paths(ex[0]), ["/var/log/httpd/example.com-access.log"]);
    assert!(!ex[0].logs[0].https);
    assert_eq!(paths(ex[1]), ["/var/log/httpd/example.com-ssl-access.log"]);
    assert!(ex[1].logs[0].https);
    // ServerName with case and port, AssignUserID (mpm-itk), a nickname with %I %O, wildcard alias dropped
    let shop = one(&f, "shop.example.net");
    assert_eq!(shop.owner, "bob");
    assert!(shop.aliases.is_empty());
    assert_eq!(
        paths(shop),
        [format!("{d}/logs/shop.example.net-access_log")]
    );
    assert_eq!(shop.logs[0].format, "combinedio");
    assert_eq!(shop.logs[0].kind, FormatKind::Combined);
    // no CustomLog: logs to the main server's file
    let nolog = one(&f, "nolog.example.org");
    assert!(nolog.logs.is_empty());
    assert!(
        nolog
            .warnings
            .iter()
            .any(|w| w.contains("no CustomLog of its own")),
        "{:?}",
        nolog.warnings
    );
    let piped = one(&f, "piped.example.org");
    assert!(piped.logs.is_empty());
    assert!(piped.warnings.iter().any(|w| w.contains("piped log")));
    // Define + ${VAR}, a leading %v, an incompatible nickname
    let vh = one(&f, "vhost.example.org");
    assert_eq!(
        paths(vh),
        [
            "/var/log/httpd/vhost.example.org.log",
            "/var/log/httpd/vhost.example.org-short.log"
        ]
    );
    assert_eq!(vh.logs[0].kind, FormatKind::Combined);
    assert_eq!(vh.logs[1].kind, FormatKind::Incompatible);
    assert_eq!(
        one(&f, "short.example.org").logs[0].kind,
        FormatKind::Incompatible
    );
    // the absolute include below /etc/httpd, and a format string continued on the next line
    let blog = one(&f, "blog.example.com");
    assert_eq!(blog.logs[0].kind, FormatKind::Combined);
    assert_eq!(blog.owner, standalone::LOCAL);
    // macro bodies are templates
    assert!(f.sites.iter().all(|s| !s.domain.contains('$')));
    assert!(
        f.sites
            .iter()
            .all(|s| !paths(s).iter().any(|p| p.contains('$')))
    );
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
}

#[test]
fn apache_debian_layout() {
    let dir = fixture("debian/etc/apache2");
    let f = apache::parse(&dir, "host.test").expect("Debian layout recognised");
    for rel in [
        "apache2.conf",
        "ports.conf",
        "conf-enabled/other-vhosts-access-log.conf",
        "sites-enabled/000-default.conf",
        "sites-enabled/example.org.conf",
    ] {
        assert!(f.files.contains(&dir.join(rel)), "{rel} read");
    }
    assert!(
        !f.files
            .iter()
            .any(|p| p.to_string_lossy().contains("sites-available")),
        "only enabled sites"
    );
    assert!(sites(&f, "disabled.example.org").is_empty());
    // ${APACHE_LOG_DIR} from envvars ($SUFFIX dropped)
    let main = sites(&f, "host.test");
    assert_eq!(main.len(), 2, "global other_vhosts log + 000-default");
    assert_eq!(paths(main[0]), ["/var/log/apache2/other_vhosts_access.log"]);
    assert_eq!(main[0].logs[0].format, "vhost_combined");
    assert_eq!(main[0].logs[0].kind, FormatKind::Combined, "%v:%p prefix");
    assert!(
        main[0]
            .warnings
            .iter()
            .any(|w| w.contains("shared by the virtual hosts"))
    );
    assert_eq!(paths(main[1]), ["/var/log/apache2/access.log"]);
    assert_eq!(
        main[1].logs[0].kind,
        FormatKind::Combined,
        "%O instead of %b"
    );
    let ex = sites(&f, "example.org");
    assert_eq!(
        ex.len(),
        2,
        "port 80 and the 443 vhost inside <IfModule mod_ssl.c>"
    );
    assert_eq!(paths(ex[0]), ["/var/log/apache2/example.org-access.log"]);
    assert_eq!(
        paths(ex[1]),
        ["/var/log/apache2/example.org-ssl-access.log"]
    );
    assert!(ex[1].logs[0].https && !ex[0].logs[0].https);
    assert_eq!(ex[0].aliases, ["www.example.org"]);
}

#[test]
fn nginx_layout() {
    let conf = fixture("nginx/etc/nginx/nginx.conf");
    let f = nginx::parse(&conf, "host.test").expect("nginx.conf");
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    // http-level access_log (EL's "main": combined + x-forwarded-for) under the default host
    let main = sites(&f, "host.test");
    assert_eq!(main.len(), 2, "http block + the catch-all server");
    assert_eq!(paths(main[0]), ["/var/log/nginx/access.log"]);
    assert_eq!(main[0].logs[0].format, "main");
    assert_eq!(main[0].logs[0].kind, FormatKind::Combined);
    assert!(main[1].logs.is_empty() && main[1].warnings.is_empty());
    let blog = one(&f, "blog.example.com");
    assert_eq!(blog.aliases, ["www.blog.example.com"]);
    assert_eq!(paths(blog), ["/var/log/nginx/blog.example.com.access.log"]);
    assert!(blog.logs[0].https, "listen 443 ssl");
    assert_eq!(blog.logs[0].kind, FormatKind::Combined, "default format");
    assert_eq!(
        one(&f, "api.example.com").logs[0].kind,
        FormatKind::Incompatible
    );
    let quiet = one(&f, "quiet.example.com");
    assert!(quiet.logs.is_empty() && quiet.warnings.iter().any(|w| w == "access_log off"));
    let dyn_ = one(&f, "dyn.example.com");
    assert!(dyn_.logs.is_empty());
    assert!(dyn_.warnings.iter().any(|w| w.contains("not supported")));
    let shop = one(&f, "shop.example.com");
    assert!(shop.logs.is_empty());
    assert!(
        shop.warnings
            .iter()
            .any(|w| w.contains("no access_log of its own"))
    );
    // sites-enabled/* without an extension, options after the format
    let legacy = one(&f, "legacy.example.com");
    assert_eq!(paths(legacy), ["/var/log/nginx/legacy.access.log"]);
    assert_eq!(legacy.logs[0].kind, FormatKind::Combined);
}

#[test]
fn missing_configurations_are_not_an_error() {
    let d = repo().join("tests/tmp/standalone-none");
    rm(&d);
    std::fs::create_dir_all(&d).unwrap();
    assert!(apache::parse(&d, "h").is_none());
    assert!(nginx::parse(&d.join("nginx.conf"), "h").is_none());
}

/// Writes an Apache configuration into a standalone fake root.
fn write_httpd(fr: &FakeRoot, vhosts: &str) {
    let r = fr.root().display().to_string();
    let conf = "ServerRoot \"/etc/httpd\"\n\
         LogFormat \"%h %l %u %t \\\"%r\\\" %>s %b \\\"%{Referer}i\\\" \\\"%{User-Agent}i\\\"\" combined\n\
         CustomLog \"logs/access_log\" combined\n\
         IncludeOptional conf.d/*.conf\n";
    std::fs::create_dir_all(fr.path("etc/httpd/conf")).unwrap();
    std::fs::create_dir_all(fr.path("etc/httpd/conf.d")).unwrap();
    std::fs::write(fr.path("etc/httpd/conf/httpd.conf"), conf).unwrap();
    std::fs::write(
        fr.path("etc/httpd/conf.d/sites.conf"),
        vhosts.replace("@ROOT@", &r),
    )
    .unwrap();
}

#[test]
fn precedence_shared_logs_and_owners() {
    let (uid, _) = me();
    let fr = FakeRoot::create_standalone("standalone-prec", json!({}));
    let r = fr.root().display().to_string();
    std::fs::create_dir_all(fr.path("srv/www")).unwrap();
    write_httpd(
        &fr,
        "<VirtualHost *:80>\n ServerName both.example.com\n CustomLog @ROOT@/var/log/httpd/both-apache.log combined\n</VirtualHost>\n\
         <VirtualHost *:80>\n ServerName owned.example.com\n DocumentRoot @ROOT@/srv/www\n CustomLog @ROOT@/var/log/httpd/owned.log combined\n</VirtualHost>\n\
         <VirtualHost *:80>\n ServerName twin.example.com\n CustomLog @ROOT@/var/log/httpd/owned.log combined\n</VirtualHost>\n\
         <VirtualHost *:80>\n ServerName gone.example.com\n CustomLog @ROOT@/var/log/httpd/gone.log combined\n</VirtualHost>\n",
    );
    for f in ["both-apache.log", "owned.log", "both-config.log"] {
        std::fs::write(fr.path(&format!("var/log/httpd/{f}")), "").unwrap();
    }
    fr.patch_config(json!({"sites": [
        {"domain": "Both.Example.com", "owner": "carol", "logs": [format!("{r}/var/log/httpd/both-config.log")], "aliases": ["www.both.example.com", "both.example.com"]},
        {"domain": "bad domain"},
        {"domain": "noowner.example.com", "owner": "Bad Owner", "logs": ["relative.log"]}
    ]}));
    let app = fr.app();
    let d = standalone::discover(&app.config, &app.paths);
    let get = |dom: &str| d.sites.iter().find(|s| s.domain == dom);
    let ign = |dom: &str| d.ignored.iter().find(|s| s.domain == dom);
    // config wins over apache for the same domain
    let both = get("both.example.com").expect("config site");
    assert_eq!(both.source, Source::Config);
    assert_eq!(both.owner, "carol");
    assert_eq!(both.aliases, ["www.both.example.com"]);
    assert_eq!(paths(both), [format!("{r}/var/log/httpd/both-config.log")]);
    let shadowed = ign("both.example.com").expect("apache definition reported");
    assert!(
        shadowed
            .warnings
            .iter()
            .any(|w| w.contains("also defined by config"))
    );
    // a document root owned by a regular user names the owner
    let owned = get("owned.example.com").expect("owned");
    assert_eq!(
        owned.owner,
        standalone::owner_of_uid(uid).unwrap_or_else(|| standalone::LOCAL.into())
    );
    // a log file is counted for one site only
    let twin = ign("twin.example.com").expect("twin has no log left");
    assert!(
        twin.warnings
            .iter()
            .any(|w| w.contains("is also the log of owned.example.com"))
    );
    // auto-detected site whose log does not exist; the main server's logs/access_log neither
    assert!(ign("gone.example.com").is_some());
    assert!(get(&standalone::machine_hostname()).is_none());
    // invalid config entries
    assert!(
        d.warnings
            .iter()
            .any(|w| w.contains("invalid or missing domain"))
    );
    let no = get("noowner.example.com").expect("kept with warnings");
    assert_eq!(no.owner, "local");
    assert!(no.warnings.iter().any(|w| w.contains("invalid owner")));
    assert!(no.warnings.iter().any(|w| w.contains("must be absolute")));
    assert!(
        no.warnings
            .iter()
            .any(|w| w.contains("no log file configured"))
    );
    // owners → accounts
    let owners = d.owners();
    assert_eq!(owners["carol"], ["both.example.com"]);
    assert!(owners["local"].contains(&"noowner.example.com".to_string()));
}

#[test]
fn archives_of_a_site() {
    let fr = FakeRoot::create_standalone("standalone-arch", json!({}));
    let dir = fr.path("var/log/httpd");
    for f in [
        "a.log",
        "a.log-20261001.gz",
        "a.log-20261008",
        "a.log.1",
        "a.log.2.gz",
        "a.log.old",
        "a.log-ssl",
        "a.log-20261001.bz2",
        "b.log",
        "b.log-1.gz",
    ] {
        std::fs::write(dir.join(f), "").unwrap();
    }
    let log = |p: &Path| standalone::SiteLog {
        path: p.to_path_buf(),
        https: false,
        format: "combined".into(),
        kind: FormatKind::Combined,
        origin: None,
    };
    let mut site = Site {
        source: Source::Config,
        domain: "a.example.com".into(),
        aliases: vec![],
        owner: "local".into(),
        logs: vec![log(&dir.join("a.log"))],
        archives: None,
        warnings: vec![],
        origin: None,
    };
    // a-log-ssl is a live log of another site
    let live = vec![dir.join("a.log"), dir.join("a.log-ssl"), dir.join("b.log")];
    let names = |v: Vec<(PathBuf, bool, PathBuf)>| -> Vec<String> {
        let mut n: Vec<String> = v
            .iter()
            .map(|(p, _, s)| {
                assert_eq!(s, &dir.join("a.log"), "stream");
                p.file_name().unwrap().to_string_lossy().into_owned()
            })
            .collect();
        n.sort();
        n
    };
    assert_eq!(
        names(standalone::site_archives(&site, &live)),
        [
            "a.log-20261001.gz",
            "a.log-20261008",
            "a.log.1",
            "a.log.2.gz"
        ]
    );
    site.archives = Some(format!("{}/a.log-*", dir.display()));
    assert_eq!(
        names(standalone::site_archives(&site, &live)),
        ["a.log-20261001.gz", "a.log-20261008"],
        "configured pattern (live logs, bz2 excluded)"
    );
}

fn me_owner() -> String {
    standalone::owner_name(&me().1).unwrap_or_else(|| standalone::LOCAL.into())
}

fn gz(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn lines(v: &[(&str, &str, &str)]) -> Vec<String> {
    v.iter().map(|(ip, t, p)| line(ip, t, p)).collect()
}

#[test]
fn standalone_collect_end_to_end_with_rotation_and_backfill() {
    let owner = me_owner();
    let mut fr = FakeRoot::create_standalone("standalone-e2e", json!({}));
    let r = fr.root().display().to_string();
    std::fs::create_dir_all(fr.path("srv/apache")).unwrap();
    write_httpd(
        &fr,
        "<VirtualHost *:80>\n ServerName apache.example.com\n DocumentRoot @ROOT@/srv/apache\n CustomLog @ROOT@/var/log/httpd/apache.example.com-access_log combined\n</VirtualHost>\n\
         <VirtualHost *:443>\n ServerName apache.example.com\n SSLEngine on\n CustomLog @ROOT@/var/log/httpd/apache.example.com-ssl_access_log combined\n</VirtualHost>\n",
    );
    fr.patch_config(json!({"sites": [{
        "domain": "config.example.com", "owner": owner,
        "logs": [format!("{r}/var/log/nginx/config.example.com.log")]
    }]}));
    let http = "var/log/httpd/apache.example.com-access_log";
    let https = "var/log/httpd/apache.example.com-ssl_access_log";
    let cfg = "var/log/nginx/config.example.com.log";
    fr.write_log(
        http,
        &lines(&[
            ("203.0.113.1", "2026-10-10 10:00:00", "/a"),
            ("203.0.113.2", "2026-10-10 10:01:00", "/b"),
        ]),
        false,
    );
    fr.write_log(
        https,
        &lines(&[("203.0.113.3", "2026-10-10 10:02:00", "/c")]),
        false,
    );
    fr.write_log(
        cfg,
        &lines(&[
            ("198.51.100.1", "2026-10-10 10:00:00", "/x"),
            ("198.51.100.2", "2026-10-10 10:00:30", "/y"),
        ]),
        false,
    );
    let apache_owner = {
        let app = fr.app();
        let d = standalone::discover(&app.config, &app.paths);
        d.sites
            .iter()
            .find(|s| s.domain == "apache.example.com")
            .expect("apache site")
            .owner
            .clone()
    };
    fr.set_now(ts("2026-10-10 12:00:00"));
    let o = fr.run("collect --json").ok("collect");
    assert_eq!(o.i("lines"), 5, "{}", o.stdout);
    let pv = |fr: &FakeRoot, u: &str, d: &str| Q::t(&fr.db(u, d), "pageviews");
    assert_eq!(pv(&fr, &apache_owner, "apache.example.com"), 3);
    assert_eq!(pv(&fr, &owner, "config.example.com"), 2);
    // registry: accounts are the owners, domains of kind "site"
    let reg = rusqlite::Connection::open(fr.path("var/webpulse/webpulse.db")).unwrap();
    let kinds: Vec<(String, String, String)> = reg
        .prepare("SELECT user, domain, kind FROM domains ORDER BY domain")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(kinds.contains(&(
        apache_owner.clone(),
        "apache.example.com".into(),
        "site".into()
    )));
    assert!(kinds.contains(&(owner.clone(), "config.example.com".into(), "site".into())));

    // logrotate with dateext: lines appended after the last run, then mv + new file
    fr.write_log(
        http,
        &lines(&[("203.0.113.4", "2026-10-10 12:30:00", "/d")]),
        true,
    );
    std::fs::rename(fr.path(http), fr.path(&format!("{http}-20261010"))).unwrap();
    fr.write_log(
        http,
        &lines(&[
            ("203.0.113.5", "2026-10-10 12:40:00", "/e"),
            ("203.0.113.6", "2026-10-10 12:41:00", "/f"),
        ]),
        false,
    );
    // numbered rotation (no dateext) of the config site's log
    fr.write_log(
        cfg,
        &lines(&[("198.51.100.3", "2026-10-10 12:30:00", "/z")]),
        true,
    );
    std::fs::rename(fr.path(cfg), fr.path(&format!("{cfg}.1"))).unwrap();
    fr.write_log(
        cfg,
        &lines(&[("198.51.100.4", "2026-10-10 12:45:00", "/w")]),
        false,
    );
    fr.set_now(ts("2026-10-10 13:00:00"));
    fr.run("collect").ok("collect after rotation");
    assert_eq!(
        pv(&fr, &apache_owner, "apache.example.com"),
        6,
        "the line written before the rotation is not lost"
    );
    assert_eq!(pv(&fr, &owner, "config.example.com"), 4);
    assert!(
        fr.log_text().contains("was rotated: finishing"),
        "{}",
        fr.log_text()
    );
    fr.run("collect").ok("idle collect");
    assert_eq!(pv(&fr, &apache_owner, "apache.example.com"), 6);
    assert_eq!(pv(&fr, &owner, "config.example.com"), 4);

    // backfill: an old compressed rotation; the plain rotated file is newer than live_since
    std::fs::write(
        fr.path(&format!("{http}-20261001.gz")),
        gz(lines(&[
            ("192.0.2.1", "2026-10-01 09:00:00", "/old1"),
            ("192.0.2.2", "2026-10-01 09:05:00", "/old2"),
            ("192.0.2.3", "2026-10-01 09:10:00", "/old3"),
        ])
        .join("\n")
        .as_bytes()),
    )
    .unwrap();
    let o = fr
        .run(&format!("backfill --user {apache_owner} --json"))
        .ok("backfill");
    assert_eq!(o.i("imported"), 3, "{}", o.stdout);
    assert_eq!(pv(&fr, &apache_owner, "apache.example.com"), 9);
    let o = fr
        .run(&format!("backfill --user {apache_owner} --json"))
        .ok("backfill again");
    assert_eq!(o.i("imported"), 0);
    assert_eq!(pv(&fr, &apache_owner, "apache.example.com"), 9);

    // `webpulse sites`
    let o = fr.run("sites --json").ok("sites");
    assert_eq!(o.json["mode"], "standalone");
    let s: Vec<&Value> = o.json["sites"].as_array().unwrap().iter().collect();
    let site = |d: &str| s.iter().find(|x| x["domain"] == d).copied().unwrap();
    let a = site("apache.example.com");
    assert_eq!(a["source"], "apache");
    assert_eq!(a["collected"], true);
    assert_eq!(a["logs"][0]["exists"], true);
    assert_eq!(a["logs"][0]["readable"], true);
    assert_eq!(a["logs"][0]["first_line"], "ok");
    assert_eq!(a["logs"][1]["https"], true);
    assert_eq!(a["archives"]["found"], 2, "-20261010 and -20261001.gz");
    let c = site("config.example.com");
    assert_eq!(c["source"], "config");
    assert_eq!(c["logs"][0]["compatible"], "unknown");
    let o = fr.run("sites").ok("sites text");
    assert!(o.stdout.contains("Mode: standalone"), "{}", o.stdout);
    assert!(
        o.stdout.contains("apache.example.com  [apache, owner"),
        "{}",
        o.stdout
    );
    assert!(o.stdout.contains("first line ok"));
    assert!(o.stdout.contains("site(s) collected"));
}

#[test]
fn backfill_never_imports_a_renamed_rotation_twice() {
    let mut fr = FakeRoot::create_standalone("standalone-renamed", json!({}));
    let r = fr.root().display().to_string();
    fr.patch_config(json!({"sites": [{
        "domain": "rot.example.com", "owner": "rotuser",
        "logs": [format!("{r}/var/log/nginx/rot.log")]
    }]}));
    let log = "var/log/nginx/rot.log";
    fr.write_log(log, &[], false);
    std::fs::write(
        fr.path(&format!("{log}.2.gz")),
        gz(lines(&[("192.0.2.1", "2026-09-01 10:00:00", "/a")])
            .join("\n")
            .as_bytes()),
    )
    .unwrap();
    fr.write_log(
        &format!("{log}.1"),
        &lines(&[
            ("192.0.2.2", "2026-09-05 10:00:00", "/b"),
            ("192.0.2.3", "2026-09-05 10:01:00", "/c"),
        ]),
        false,
    );
    fr.set_now(ts("2026-09-06 12:00:00"));
    let o = fr.run("backfill --user rotuser --json").ok("backfill");
    assert_eq!(o.i("imported"), 3, "plain and gzip archives: {}", o.stdout);
    // logrotate runs: .2.gz -> .3.gz, .1 -> .2.gz (compressed), the live log -> .1
    std::fs::rename(
        fr.path(&format!("{log}.2.gz")),
        fr.path(&format!("{log}.3.gz")),
    )
    .unwrap();
    let plain = std::fs::read(fr.path(&format!("{log}.1"))).unwrap();
    std::fs::write(fr.path(&format!("{log}.2.gz")), gz(&plain)).unwrap();
    std::fs::remove_file(fr.path(&format!("{log}.1"))).unwrap();
    let o = fr
        .run("backfill --user rotuser --json")
        .ok("backfill after renames");
    assert_eq!(o.i("imported"), 0, "{}", o.stdout);
    assert_eq!(Q::t(&fr.db("rotuser", "rot.example.com"), "pageviews"), 3);
}

#[test]
fn mode_auto_and_explicit() {
    // the cPanel fake root: auto = cPanel (its users directory exists)
    let fr = FakeRoot::create("standalone-mode-cp", json!({}));
    assert_eq!(fr.app().mode().as_str(), "cpanel");
    let o = fr.run("sites --json").ok("sites on cPanel");
    assert_eq!(o.json["mode"], "cpanel");
    assert!(
        o.json["sites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["source"] == "cpanel" && s["owner"] == "alice")
    );
    fr.patch_config(
        json!({"mode": "standalone", "paths": {"apache_conf_dirs": [], "nginx_conf": ""}}),
    );
    assert_eq!(fr.app().mode().as_str(), "standalone");
    // standalone root: auto = standalone, unless the marker exists
    let fr = FakeRoot::create_standalone("standalone-mode-auto", json!({"mode": "auto"}));
    assert_eq!(fr.app().mode().as_str(), "standalone");
    std::fs::create_dir_all(fr.path("usr/local/cpanel")).unwrap();
    std::fs::write(fr.path("usr/local/cpanel/cpanel"), "").unwrap();
    assert_eq!(fr.app().mode().as_str(), "cpanel");
    fr.patch_config(json!({"mode": "standalone"}));
    assert_eq!(fr.app().mode().as_str(), "standalone");
    let o = fr.run("config get mode").ok("config get mode");
    assert_eq!(o.stdout.trim(), "standalone");
    let o = fr.run("config set mode cpanel");
    assert_ne!(o.code, 0, "mode is file-only");
}
