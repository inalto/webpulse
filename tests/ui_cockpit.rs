//! The Cockpit adapter (`ui/cockpit.js`) in headless Chrome, against a mock
//! `cockpit.js` whose `cockpit.spawn` runs the real binary through a tiny local
//! HTTP bridge (`POST /spawn`): the document of `webpulse page --cockpit` boots
//! (whoami, domains), renders the dashboard of the sites the test user owns,
//! follows the shell's dark style, exports CSV as a Blob (no navigation), opens
//! the report in the same frame and prints it; a missing binary shows the error
//! state. Skipped when no Chrome/Chromium is found (`WEBPULSE_CHROME`, the
//! Playwright cache, `chromium`, `google-chrome`).

mod common;

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use common::*;
use serde_json::{Value, json};

fn chrome() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WEBPULSE_CHROME").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let cache = PathBuf::from(home).join(".cache/ms-playwright");
        if let Ok(rd) = std::fs::read_dir(&cache) {
            let mut dirs: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("chromium-"))
                })
                .collect();
            dirs.sort();
            for d in dirs.iter().rev() {
                let c = d.join("chrome-linux64/chrome");
                if c.is_file() {
                    return Some(c);
                }
            }
        }
    }
    for name in [
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
    ] {
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("command -v {name}")])
            .output()
            .ok()?;
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

const MOCK: &str = r##"(function () {
  'use strict';
  var log = { spawns: [], blobs: [], downloads: [], prints: 0, errors: [], step: 0 };
  window.__mock = log;
  // the shell's dark style, as Cockpit sets it on a page's <html>
  document.documentElement.classList.add('pf-v6-theme-dark');
  window.addEventListener('error', function (e) { log.errors.push(String(e.message)); });
  URL.createObjectURL = function (b) { log.blobs.push({ size: b.size, type: b.type }); return 'blob:mock/' + log.blobs.length; };
  URL.revokeObjectURL = function () {};
  var click = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () {
    if (String(this.href).indexOf('blob:') === 0) { log.downloads.push(this.download); return; }
    return click.call(this);
  };
  window.print = function () { log.prints++; };
  window.cockpit = {
    user: function () { return Promise.resolve({ name: 'mock', full_name: 'Mock User', id: 1000 }); },
    spawn: function (args, opts) {
      log.spawns.push({ args: args, superuser: opts.superuser, err: opts.err, environ: opts.environ });
      var settle;
      var p = new Promise(function (res, rej) { settle = { res: res, rej: rej }; });
      var proc = {
        input: function (data) {
          fetch('/spawn', { method: 'POST', body: JSON.stringify({ args: args, environ: opts.environ || [], input: data == null ? '' : String(data) }) })
            .then(function (r) { return r.text().then(function (t) { return { code: r.status, text: t, exit: r.headers.get('X-Exit') }; }); })
            .then(function (r) {
              if (r.code === 404) { settle.rej({ ex: { problem: 'not-found', message: 'not-found' }, out: '' }); return; }
              if (r.exit !== '0') { settle.rej({ ex: { exit_status: Number(r.exit), message: args[0] + ' exited with code ' + r.exit }, out: r.text }); return; }
              settle.res(r.text);
            });
          return proc;
        },
        then: function (ok, bad) { p.then(ok, function (x) { bad(x.ex, x.out); }); return proc; }
      };
      return proc;
    }
  };
  // the test driver
  setInterval(function () {
    var root = document.getElementById('webpulse-app');
    if (!root) return;
    log.error = root.getAttribute('data-cockpit-error');
    if (log.error) log.errorText = root.textContent;
    if (log.step === 0 && document.querySelector('.wp-domain-name') && document.querySelector('a[download]')) {
      log.step = 1;
      log.domain = document.querySelector('.wp-domain-name').textContent;
      log.theme = root.getAttribute('data-theme');
      log.principal = root.getAttribute('data-principal');
      var f = document.querySelector('.wp-foot-user');
      log.foot = f ? f.textContent : null;
      document.querySelector('a[download]').click();
    } else if (log.step === 1 && log.downloads.length) {
      log.step = 2;
      var pdf = document.querySelector('a.wp-icon-btn[href*="#report"]');
      log.pdfTarget = pdf ? pdf.getAttribute('target') : 'missing';
      log.pdfHref = pdf ? pdf.getAttribute('href') : null;
      if (pdf) pdf.click();
    } else if (log.step === 2 && root.getAttribute('data-report-ready') === '1') {
      log.step = 3;
      log.hash = location.hash;
    }
    var pre = document.getElementById('mock-log');
    if (!pre) { pre = document.createElement('pre'); pre.id = 'mock-log'; pre.hidden = true; document.body.appendChild(pre); }
    pre.textContent = JSON.stringify(log);
  }, 50);
})();
"##;

struct Bridge {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

fn serve(page: String, config: PathBuf, now: i64) -> Bridge {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    std::thread::spawn(move || {
        for mut rq in server.incoming_requests() {
            let url = rq.url().to_string();
            seen.lock().unwrap().push(format!("{} {url}", rq.method()));
            let resp = match url.as_str() {
                "/webpulse/index.html" => tiny_http::Response::from_data(page.clone().into_bytes())
                    .with_header(
                        tiny_http::Header::from_bytes(
                            &b"Content-Type"[..],
                            &b"text/html; charset=utf-8"[..],
                        )
                        .unwrap(),
                    ),
                "/base1/cockpit.js" => tiny_http::Response::from_data(MOCK.as_bytes().to_vec())
                    .with_header(
                        tiny_http::Header::from_bytes(
                            &b"Content-Type"[..],
                            &b"text/javascript"[..],
                        )
                        .unwrap(),
                    ),
                "/spawn" => {
                    let mut body = String::new();
                    let _ = rq.as_reader().read_to_string(&mut body);
                    let req: Value = serde_json::from_str(&body).unwrap();
                    let args: Vec<String> = req["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap().to_string())
                        .collect();
                    if !std::path::Path::new(&args[0]).is_file() {
                        let _ = rq.respond(
                            tiny_http::Response::from_string("not found").with_status_code(404),
                        );
                        continue;
                    }
                    let mut cmd = std::process::Command::new(&args[0]);
                    cmd.args(&args[1..])
                        .env_clear()
                        .env("PATH", "/usr/bin:/bin")
                        .env("WEBPULSE_CONFIG", &config)
                        .env("WEBPULSE_NOW", now.to_string())
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::null());
                    for e in req["environ"].as_array().unwrap() {
                        if let Some((k, v)) = e.as_str().unwrap().split_once('=') {
                            cmd.env(k, v);
                        }
                    }
                    let mut child = cmd.spawn().unwrap();
                    child
                        .stdin
                        .take()
                        .unwrap()
                        .write_all(req["input"].as_str().unwrap().as_bytes())
                        .unwrap();
                    let out = child.wait_with_output().unwrap();
                    tiny_http::Response::from_data(out.stdout).with_header(
                        tiny_http::Header::from_bytes(
                            &b"X-Exit"[..],
                            out.status.code().unwrap_or(-1).to_string().as_bytes(),
                        )
                        .unwrap(),
                    )
                }
                _ => tiny_http::Response::from_data(b"no".to_vec()).with_status_code(404),
            };
            let _ = rq.respond(resp);
        }
    });
    Bridge { port, requests }
}

fn run_chrome(chrome: &PathBuf, port: u16, profile: &std::path::Path) -> Value {
    let out = std::process::Command::new(chrome)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            "--disable-extensions",
            &format!("--user-data-dir={}", profile.display()),
            "--virtual-time-budget=20000",
            "--dump-dom",
            &format!("http://127.0.0.1:{port}/webpulse/index.html"),
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .unwrap();
    let dom = String::from_utf8_lossy(&out.stdout).into_owned();
    let i = dom.find("<pre id=\"mock-log\"").unwrap_or_else(|| {
        panic!(
            "no mock log in the DOM ({} bytes):\n{}",
            dom.len(),
            &dom[dom.len().saturating_sub(600)..]
        )
    });
    let rest = &dom[i..];
    let start = rest.find('>').unwrap() + 1;
    let end = rest.find("</pre>").unwrap();
    let text = rest[start..end]
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[test]
fn cockpit_adapter_in_headless_chrome() {
    let Some(chrome) = chrome() else {
        eprintln!("skipped: no Chrome/Chromium (set WEBPULSE_CHROME)");
        return;
    };
    let uid = unsafe { libc::geteuid() };
    let me = webpulse::util::fs::user_name(uid).unwrap_or_default();
    let owner = webpulse::engine::standalone::owner_name(&me).unwrap_or_else(|| "local".into());
    let mut fr = FakeRoot::create_standalone("ui-cockpit", json!({}));
    let r = fr.root().display().to_string();
    fr.patch_config(json!({"sites": [
        {"domain": "mine.example.com", "owner": owner, "logs": [format!("{r}/var/log/httpd/mine.log")]}
    ]}));
    let mut l = Vec::new();
    for i in 0..20 {
        l.push(line(
            &format!("203.0.113.{}", i + 1),
            &format!("2026-10-10 10:{:02}:00", i),
            &format!("/page-{}", i % 4),
        ));
    }
    fr.write_log("var/log/httpd/mine.log", &l, false);
    let now = ts("2026-10-10 12:00:00");
    fr.set_now(now);
    fr.run("collect").ok("collect");
    let bin_s = bin().display().to_string();
    let page = fr
        .run(&format!("page --cockpit --bin {bin_s}"))
        .ok("page")
        .stdout;
    let bridge = serve(page, fr.config_path(), now);
    let profile = fr.dir().join("chrome-profile");
    let log = run_chrome(&chrome, bridge.port, &profile);

    assert_eq!(log["errors"], json!([]), "{log:#}");
    assert_eq!(log["error"], Value::Null, "{log:#}");
    // every API call went through cockpit.spawn as `<bin> cgi --cockpit --api`, superuser try
    let spawns = log["spawns"].as_array().unwrap();
    assert!(spawns.len() >= 3, "{log:#}");
    for s in spawns {
        assert_eq!(s["args"], json!([bin_s, "cgi", "--cockpit", "--api"]));
        assert_eq!(s["superuser"], "try");
        assert_eq!(s["err"], "message");
    }
    let env0: Vec<&str> = spawns[0]["environ"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        env0.contains(&"QUERY_STRING=a=whoami") && env0.contains(&"REQUEST_METHOD=GET"),
        "{env0:?}"
    );
    // the dashboard of the user's own site, dark like the shell, the Cockpit user in the footer
    assert_eq!(log["domain"], "mine.example.com", "{log:#}");
    assert_eq!(log["theme"], "dark");
    assert_eq!(log["principal"], if uid == 0 { "admin" } else { "user" });
    assert!(log["foot"].as_str().unwrap_or("").starts_with("Mock User"));
    // CSV export: a Blob download, no navigation inside the frame
    assert_eq!(
        log["blobs"][0]["type"], "text/csv; charset=utf-8",
        "{log:#}"
    );
    assert!(log["blobs"][0]["size"].as_u64().unwrap() > 10);
    assert!(log["downloads"][0].as_str().unwrap().ends_with(".csv"));
    let reqs = bridge.requests.lock().unwrap().clone();
    assert!(
        !reqs.iter().any(|r| r.contains("index.html?")),
        "no navigation to the API URL: {reqs:?}"
    );
    // the report opens in the same frame and prints it
    assert_eq!(log["pdfTarget"], Value::Null, "no target=_blank in Cockpit");
    assert_eq!(log["step"], 3, "report rendered: {log:#}");
    assert!(log["hash"].as_str().unwrap().starts_with("#report"));
    assert!(
        log["prints"].as_u64().unwrap() >= 1,
        "window.print() of the frame"
    );
    assert_eq!(
        reqs.iter()
            .filter(|r| r.contains("/webpulse/index.html"))
            .count(),
        1,
        "the report is a hash change, not a new load: {reqs:?}"
    );
}

#[test]
fn cockpit_adapter_reports_a_missing_binary() {
    let Some(chrome) = chrome() else {
        eprintln!("skipped: no Chrome/Chromium (set WEBPULSE_CHROME)");
        return;
    };
    let fr = FakeRoot::create_standalone("ui-cockpit-missing", json!({}));
    let page = fr
        .run("page --cockpit --bin /nonexistent/webpulse/bin/webpulse")
        .ok("page")
        .stdout;
    let bridge = serve(page, fr.config_path(), 0);
    let log = run_chrome(&chrome, bridge.port, &fr.dir().join("chrome-profile"));
    assert_eq!(log["error"], "1", "{log:#}");
    let text = log["errorText"].as_str().unwrap();
    assert!(text.contains("could not start"), "{text}");
    assert!(
        text.contains("/nonexistent/webpulse/bin/webpulse was not found"),
        "{text}"
    );
    assert_eq!(log["errors"], json!([]));
}
