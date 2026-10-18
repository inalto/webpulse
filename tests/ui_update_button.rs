//! The admin header's "Update to X.Y.Z" button in headless Chrome: it appears only
//! when a newer release can be installed, and clicking it opens the Updates popover
//! at the confirmation step (regression: the click bubbled to the document-level
//! "close popovers" handler, which removed the popover it had just opened).

mod common;
mod http_common;

use std::path::PathBuf;

use http_common::*;
use serde_json::Value;
use webpulse::http::page::{PageOptions, render};
use webpulse::http::{Ctx, Principal};

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

fn nonce_of(html: &str) -> String {
    let i = html.find("script-src 'nonce-").expect("nonce in CSP") + "script-src 'nonce-".len();
    html[i..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Canned API: admin.update reports a newer installable release; every other
/// action fails, which the dashboard renders as card errors (irrelevant here).
const MOCK: &str = r#"
(function () {
  var update = {current: "0.6.3", latest: "9.9.9", tag: "v9.9.9", is_newer: true,
    html_url: "https://example.invalid/r", published_at: "2026-10-18T00:00:00Z", notes: "",
    checked_at: 1792300000, checked_at_iso: "2026-10-18T10:00:00+00:00", error: null,
    check_enabled: true, install_sh: true, can_upgrade: true, upgrade_job: null, stale: false};
  window.fetch = function (url) {
    var a = (String(url).match(/[?&]a=([^&]+)/) || [])[1];
    var body = a === "admin.update" ? {ok: true, data: update}
      : {ok: false, error: {code: "mock", message: "mock"}};
    return Promise.resolve(new Response(JSON.stringify(body),
      {status: 200, headers: {"Content-Type": "application/json"}}));
  };
  function done(result) {
    var pre = document.createElement("pre");
    pre.id = "mock-log";
    pre.textContent = JSON.stringify(result);
    document.body.appendChild(pre);
  }
  setTimeout(function () {
    var cta = document.querySelector(".wp-upd-cta");
    var before = {present: !!cta, hidden: cta ? cta.hidden : null, text: cta ? cta.textContent : null};
    if (!cta || cta.hidden) { done({before: before}); return; }
    cta.click();
    setTimeout(function () {
      var pop = document.querySelector(".wp-pop");
      var bell = document.querySelector(".wp-bell");
      done({before: before, popOpen: !!pop, popText: pop ? pop.textContent : "",
            bellExpanded: bell ? bell.getAttribute("aria-expanded") : null});
    }, 400);
  }, 2500);
})();
"#;

#[test]
fn update_button_opens_the_confirmation_popover() {
    let Some(chrome) = chrome() else {
        eprintln!("skipped: no Chrome/Chromium found");
        return;
    };
    let env = shared("ui-update-button");
    let a = app(&env);
    let ctx = Ctx::new(&a);
    let html = render(
        &ctx,
        &Principal::admin(),
        &PageOptions::new("api", false, None),
    );
    let nonce = nonce_of(&html);
    let page = html.replacen(
        "<head>",
        &format!("<head><script nonce=\"{nonce}\">{MOCK}</script>"),
        1,
    );
    let dir = env.root.join("ui-update-button");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("index.html");
    std::fs::write(&file, page).unwrap();

    let out = std::process::Command::new(&chrome)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            "--disable-extensions",
            "--window-size=1280,900",
            &format!("--user-data-dir={}", dir.join("profile").display()),
            "--virtual-time-budget=8000",
            "--dump-dom",
            &format!("file://{}", file.display()),
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .unwrap();
    let dom = String::from_utf8_lossy(&out.stdout).into_owned();
    let i = dom
        .find("<pre id=\"mock-log\"")
        .unwrap_or_else(|| panic!("no result in the DOM ({} bytes)", dom.len()));
    let rest = &dom[i..];
    let text = &rest[rest.find('>').unwrap() + 1..rest.find("</pre>").unwrap()];
    let text = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&");
    let r: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));

    assert_eq!(r["before"]["present"], true, "{r}");
    assert_eq!(
        r["before"]["hidden"], false,
        "button shown for a newer release: {r}"
    );
    assert_eq!(r["before"]["text"], "Update to 9.9.9", "{r}");
    assert_eq!(
        r["popOpen"], true,
        "popover stays open after the click: {r}"
    );
    assert_eq!(r["bellExpanded"], "true", "{r}");
    assert!(
        r["popText"]
            .as_str()
            .unwrap_or("")
            .contains("Upgrade to 9.9.9"),
        "popover opens at the confirmation step: {r}"
    );
}
