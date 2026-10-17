//! Renders the single-page dashboard with its CSS and JavaScript inlined
//! (embedded in the binary), a per-request nonce, a strict
//! Content-Security-Policy (full documents) and the `window.WEBPULSE`
//! bootstrap (port of `Http\Page`).

use serde_json::json;

use super::api;
use super::ctx::Ctx;
use super::json as pjson;
use super::principal::Principal;
use super::user_settings_api;
use crate::app::App;
use crate::reports::{dimension, range};

const APP_CSS: &str = include_str!("../../ui/app.css");
const I18N_JS: &str = include_str!("../../ui/i18n.js");
const CHARTS_JS: &str = include_str!("../../ui/charts.js");
const APP_JS: &str = include_str!("../../ui/app.js");
const ADMIN_JS: &str = include_str!("../../ui/admin.js");

/// How the page is rendered.
#[derive(Debug, Clone)]
pub struct PageOptions {
    /// URL of the JSON API relative to the page (`api.live.php`, `api.cgi`, `/api`).
    pub api_url: String,
    /// Inside cPanel chrome: a fragment instead of a document.
    pub embedded: bool,
    /// Document title (default "WebPulse Analytics").
    pub title: Option<String>,
}

impl PageOptions {
    /// Options with an API URL.
    pub fn new(api_url: &str, embedded: bool, title: Option<&str>) -> PageOptions {
        PageOptions {
            api_url: api_url.into(),
            embedded,
            title: title.map(str::to_string),
        }
    }
}

/// Fills `buf` with random bytes (`getrandom(2)`, else `/dev/urandom`).
pub fn random_bytes(buf: &mut [u8]) {
    // SAFETY: the pointer and length describe `buf`, which is valid for writes.
    let n = unsafe { libc::getrandom(buf.as_mut_ptr().cast(), buf.len(), 0) };
    if n == buf.len() as isize {
        return;
    }
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        if f.read_exact(buf).is_ok() {
            return;
        }
    }
    // Last resort: time-derived bytes (never expected on Linux).
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for (i, b) in buf.iter_mut().enumerate() {
        *b = (t >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(151);
    }
}

/// URL-safe base64 without padding.
pub fn base64url(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | ((*b as u32) << (16 - 8 * i)));
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// `htmlspecialchars($s, ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8')`.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#039;"),
            c => out.push(c),
        }
    }
    out
}

/// An inlined asset must never close its own element (case-insensitive).
fn guard(src: &str, tag: &str) -> String {
    let needle = format!("</{tag}");
    let lower = src.to_ascii_lowercase();
    let mut out = String::with_capacity(src.len());
    let mut last = 0;
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&needle) {
        let at = i + pos;
        out.push_str(&src[last..at]);
        out.push_str("<\\/");
        out.push_str(tag);
        last = at + needle.len();
        i = last;
    }
    out.push_str(&src[last..]);
    out
}

/// How a non-ASCII character is written by [`ascii_only`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ascii {
    /// HTML text: `&#x2026;`
    Html,
    /// JavaScript source (strings, regexes, comments): `\u2026` (surrogate pairs above U+FFFF)
    Js,
    /// CSS (strings, comments): `\002026`
    Css,
}

/// Rewrites every non-ASCII character as an escape of the given language, so a
/// fragment survives a host template engine whatever its byte/character handling
/// (WHM's Template Toolkit, cPanel's PHP header/footer).
pub fn ascii_only(s: &str, how: Ascii) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + s.len() / 8);
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        match how {
            Ascii::Html => out.push_str(&format!("&#x{:X};", c as u32)),
            Ascii::Css => out.push_str(&format!("\\{:06X}", c as u32)),
            Ascii::Js => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04X}", u));
                }
            }
        }
    }
    out
}

/// The `window.WEBPULSE` object.
pub fn bootstrap(ctx: &Ctx, p: &Principal, o: &PageOptions) -> serde_json::Value {
    let domains = api::domains(ctx, p);
    let mut range = range::DEFAULT.to_string();
    let mut dark = "auto".to_string();
    if p.is_user() {
        let s = user_settings_api::read(&ctx.paths, &p.name);
        if let Some(r) = s["ui"]["default_range"]
            .as_str()
            .filter(|r| range::SHORTCUTS.contains(r))
        {
            range = r.to_string();
        }
        match &s["ui"]["dark"] {
            serde_json::Value::String(d) if ["auto", "light", "dark"].contains(&d.as_str()) => {
                dark = d.clone()
            }
            serde_json::Value::Bool(b) => dark = if *b { "dark".into() } else { "light".into() },
            _ => {}
        }
    }
    let mut boot = json!({
        "apiUrl": o.api_url,
        "principal": p.to_json(),
        "domains": domains,
        "defaults": {"range": range, "dark": dark},
        "version": App::version(),
        "embedded": o.embedded,
        "dims": dimension::meta(),
        "ranges": range::SHORTCUTS,
    });
    // DB-IP's CC BY 4.0 licence: the attribution in the footer while its database is used.
    if super::admin_geoip::attribution_needed(ctx) {
        boot["geoip_credit"] = json!({
            "text": super::admin_geoip::ATTRIBUTION,
            "url": super::admin_geoip::ATTRIBUTION_URL,
        });
    }
    // The server name on the cover of the WHM reports (admin scope only).
    if p.has_admin_scope()
        && let Some(host) = hostname()
    {
        boot["hostname"] = json!(host);
    }
    boot
}

/// This machine's host name (`gethostname(2)`), `None` when unavailable.
fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the pointer and length describe `buf`, which is valid for writes.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let host = String::from_utf8_lossy(&buf[..end]).trim().to_string();
    (!host.is_empty()).then_some(host)
}

/// Full HTML document, or (embedded) a `<div id="webpulse-app">` fragment
/// with its `<style>`/`<script>` tags for cPanel's header()/footer().
pub fn render(ctx: &Ctx, p: &Principal, o: &PageOptions) -> String {
    let mut raw = [0u8; 18];
    random_bytes(&mut raw);
    let nonce = base64url(&raw);
    let mut scripts = vec![I18N_JS, CHARTS_JS, APP_JS];
    if p.has_admin_scope() {
        scripts.push(ADMIN_JS);
    }
    // the fragment for a host page is pure ASCII (see `ascii_only`)
    let esc = |src: &str, how: Ascii| -> String {
        if o.embedded {
            ascii_only(src, how)
        } else {
            src.to_string()
        }
    };
    let mut js = String::new();
    for s in scripts {
        js.push_str(&format!(
            "<script nonce=\"{nonce}\">\n{}\n</script>\n",
            esc(&guard(s, "script"), Ascii::Js)
        ));
    }
    let boot = esc(
        &pjson::encode(&bootstrap(ctx, p, o), pjson::PAGE),
        Ascii::Js,
    );
    let title = html_escape(o.title.as_deref().unwrap_or("WebPulse Analytics"));
    let body = format!(
        "<div id=\"webpulse-app\" class=\"wp-root\" data-embedded=\"{}\"><noscript><p class=\"wp-noscript\">WebPulse Analytics needs JavaScript enabled.</p></noscript><div class=\"wp-boot\" aria-live=\"polite\">Loading analytics{}</div></div>\n\
         <style nonce=\"{nonce}\">\n{}\n</style>\n\
         <script nonce=\"{nonce}\">window.WEBPULSE = {boot};</script>\n\
         {js}\
         <script nonce=\"{nonce}\">WebPulseApp.start(document.getElementById('webpulse-app'), window.WEBPULSE);</script>\n",
        if o.embedded { "1" } else { "0" },
        if o.embedded { "&#x2026;" } else { "…" },
        esc(&guard(APP_CSS, "style"), Ascii::Css),
    );
    if o.embedded {
        return body;
    }
    let csp = format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'; img-src data:; connect-src 'self'; base-uri 'none'; form-action 'none'"
    );
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"referrer\" content=\"no-referrer\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <title>{title}</title>\n</head>\n<body class=\"wp-body\">\n{body}</body>\n</html>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(base64url(&[0xfb, 0xff, 0xbf]), "-_-_");
        assert_eq!(base64url(b"ab"), "YWI");
        assert_eq!(base64url(&[0u8; 18]).len(), 24);
        assert_eq!(
            html_escape("<b a='1'>&\"</b>"),
            "&lt;b a=&#039;1&#039;&gt;&amp;&quot;&lt;/b&gt;"
        );
        assert_eq!(ascii_only("a…b", Ascii::Html), "a&#x2026;b");
        assert_eq!(ascii_only("'→'", Ascii::Js), "'\\u2192'");
        assert_eq!(ascii_only("'😀'", Ascii::Js), "'\\uD83D\\uDE00'");
        assert_eq!(ascii_only("\" ↓\"", Ascii::Css), "\" \\002193\"");
        assert_eq!(ascii_only("plain", Ascii::Css), "plain");
        assert_eq!(
            guard("a</SCRIPT>b</script", "script"),
            "a<\\/script>b<\\/script"
        );
    }
}
