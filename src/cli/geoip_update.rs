//! `webpulse geoip-update [--force] [--url URL] [--out FILE]` (`GeoipUpdateCommand`).
//!
//! The download is streamed to a temporary file next to the target (never held in
//! memory), capped at [`MAX_DOWNLOAD`] bytes ([`MAX_DATABASE`] once decompressed);
//! at most one redirect is followed, and only to https.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::{Ctx, Job, Kind, RawArgs};
use crate::error::exit;
use crate::log::Level;
use crate::util::{fs, time};

/// Default download URL template.
pub const DEFAULT_URL: &str =
    "https://download.db-ip.com/free/dbip-country-lite-{YYYY}-{MM}.mmdb.gz";
/// Largest download accepted (bytes).
pub const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;
/// Largest decompressed database accepted (bytes).
pub const MAX_DATABASE: u64 = 512 * 1024 * 1024;

/// Job type in `registry.jobs` (and of `admin.run`).
pub const JOB_TYPE: &str = "geoip-update";

/// One-line description.
pub const SUMMARY: &str = "download the free DB-IP country database (when geoip.enabled)";

/// `--help` text.
pub const USAGE: &str =
    "Usage: webpulse geoip-update [--force] [--url URL] [--out FILE] [--job [ID]]

Downloads dbip-country-lite-YYYY-MM.mmdb.gz for the current month (falls
back to the previous month), validates it and atomically replaces
geoip.country_db. Does nothing unless geoip.enabled, or --force (the monthly
cron job runs it without --force; WHM's \"Download database now\" uses --force,
so the file can be installed before GeoIP is switched on). The run is
recorded in registry.jobs when the registry exists.

  --force     download even when geoip.enabled is false
  --url URL   source (may contain {YYYY} and {MM}; file:// and local paths work);
              default geoip.url
  --out FILE  destination instead of geoip.country_db
  --job [ID]  record the run in registry.jobs (adopt job ID when given)";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[
    ("force", Kind::Flag),
    ("url", Kind::Value),
    ("out", Kind::Value),
    ("job", Kind::Optional),
];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Download even when geoip.enabled is false
    #[arg(long)]
    pub force: bool,
    /// Source URL template
    #[arg(long)]
    pub url: Option<String>,
    /// Destination file
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Record the run in registry.jobs (adopt job ID when given)
    #[arg(long)]
    pub job: Option<Option<String>>,
}

fn has_scheme(url: &str) -> bool {
    let b = url.as_bytes();
    let Some(p) = url.find("://") else {
        return false;
    };
    p > 0
        && b[0].is_ascii_alphabetic()
        && b[..p]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
}

/// Where a redirect may go: an absolute `https://` URL, or a path / relative
/// reference resolved against an `https://` source; anything else is refused.
pub fn redirect_target(from: &str, location: &str) -> Option<String> {
    let location = location.trim();
    if location.is_empty() || location.bytes().any(|b| b <= 0x20 || b == 0x7f) {
        return None;
    }
    let has_scheme_prefix = {
        let b = location.as_bytes();
        match location.find(':') {
            Some(p) if p > 0 => {
                b[0].is_ascii_alphabetic()
                    && b[..p]
                        .iter()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
            }
            _ => false,
        }
    };
    if has_scheme_prefix {
        return location
            .to_ascii_lowercase()
            .starts_with("https://")
            .then(|| location.to_string());
    }
    if !from.to_ascii_lowercase().starts_with("https://") {
        return None;
    }
    if let Some(rest) = location.strip_prefix("//") {
        return Some(format!("https://{rest}"));
    }
    let after = &from[8..];
    let host_end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let origin = &from[..8 + host_end];
    if location.starts_with('/') {
        return Some(format!("{origin}{location}"));
    }
    let path_part = &after[host_end..];
    let path = path_part.split(['?', '#']).next().unwrap_or("");
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    Some(format!("{origin}{dir}{location}"))
}

fn copy_capped(mut input: impl Read, dest: &Path, max: u64, what: &str) -> Result<()> {
    let mut out = File::create(dest).map_err(|_| anyhow!("cannot write {}", dest.display()))?;
    let mut n: u64 = 0;
    let mut buf = vec![0u8; 1 << 16];
    let r = (|| -> Result<()> {
        loop {
            let k = match input.read(&mut buf) {
                Ok(0) => break,
                Ok(k) => k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(anyhow!("{what}: read error")),
            };
            n += k as u64;
            if n > max {
                return Err(anyhow!("{what}: download larger than {max} bytes"));
            }
            out.write_all(&buf[..k])
                .map_err(|_| anyhow!("cannot write {}", dest.display()))?;
        }
        if n == 0 {
            return Err(anyhow!("{what}: empty download"));
        }
        Ok(())
    })();
    if r.is_err() {
        drop(out);
        let _ = std::fs::remove_file(dest);
    }
    r
}

/// Downloads (or copies) `url` into the file `dest`, streaming, at most `max` bytes.
pub fn fetch_to(url: &str, dest: &Path, max: u64) -> Result<()> {
    if url.starts_with("file://") || !has_scheme(url) {
        let path = url.strip_prefix("file://").unwrap_or(url);
        let f = File::open(path).map_err(|_| anyhow!("cannot read {path}"))?;
        return copy_capped(f, dest, max, path);
    }
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(anyhow!("unsupported URL {url}"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent("WebPulse geoip-update")
        .build()
        .into();
    let mut current = url.to_string();
    for hop in 0..=1 {
        let resp = agent
            .get(&current)
            .call()
            .map_err(|e| anyhow!("{url}: {e}"))?;
        let status = resp.status().as_u16();
        if (300..400).contains(&status) {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let next = location
                .as_deref()
                .and_then(|l| redirect_target(&current, l));
            match next {
                Some(n) if hop == 0 => {
                    current = n;
                    continue;
                }
                _ => {
                    return Err(anyhow!(
                        "{url}: refusing redirect to {} (only one redirect, to https)",
                        location.as_deref().unwrap_or("(none)")
                    ));
                }
            }
        }
        if status != 200 {
            return Err(anyhow!("{url}: HTTP {status}"));
        }
        let length = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        if length.map(|l| l > max).unwrap_or(false) {
            return Err(anyhow!("{url}: download larger than {max} bytes"));
        }
        let reader = resp.into_body().into_with_config().limit(max + 1).reader();
        return copy_capped(reader, dest, max, url);
    }
    Err(anyhow!("{url}: too many redirects"))
}

fn gunzip_to(src: &Path, dest: &Path) -> Result<()> {
    let f = File::open(src).map_err(|_| anyhow!("corrupt gzip data"))?;
    let dec = flate2::read::MultiGzDecoder::new(std::io::BufReader::new(f));
    let mut out = File::create(dest).map_err(|_| anyhow!("cannot write {}", dest.display()))?;
    let mut n: u64 = 0;
    let mut buf = vec![0u8; 1 << 16];
    let mut dec = dec;
    let r = (|| -> Result<()> {
        loop {
            let k = match dec.read(&mut buf) {
                Ok(0) => break,
                Ok(k) => k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(anyhow!("corrupt gzip data")),
            };
            n += k as u64;
            if n > MAX_DATABASE {
                return Err(anyhow!(
                    "decompressed database larger than {MAX_DATABASE} bytes"
                ));
            }
            out.write_all(&buf[..k])
                .map_err(|_| anyhow!("cannot write {}", dest.display()))?;
        }
        Ok(())
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    r
}

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let app = ctx.app()?;
    if !raw.flag("force") && !crate::config::php_bool(app.config.get("geoip.enabled")) {
        if raw.has("job") {
            Job::new(ctx, JOB_TYPE, Some("all"), raw)?.finish(
                "skipped",
                0,
                0,
                0,
                Some("GeoIP is disabled"),
                false,
            );
        }
        ctx.info("GeoIP is disabled (geoip.enabled = false); use --force to download anyway");
        return Ok(exit::OK);
    }
    // the run is recorded in registry.jobs when the registry exists (WHM shows the last one)
    let record = app.paths.registry_db().is_file();
    let mut job = Job::new(ctx, JOB_TYPE, Some("all"), raw)?;
    match install(ctx, raw) {
        Ok(info) => {
            let msg = format!(
                "installed {} built {} ({} bytes)",
                info["database_type"].as_str().unwrap_or(""),
                info["build_date"].as_str().unwrap_or(""),
                info["bytes"]
            );
            job.finish("ok", 0, 0, 0, Some(&msg), record);
            ctx.result(&info, || {
                format!(
                    "installed {} ({}, built {})",
                    info["file"].as_str().unwrap_or(""),
                    info["database_type"].as_str().unwrap_or(""),
                    info["build_date"].as_str().unwrap_or("")
                )
            });
            Ok(exit::OK)
        }
        Err(msg) => {
            job.finish("error", 0, 0, 0, Some(&msg), record);
            ctx.err(&msg);
            Ok(exit::ERROR)
        }
    }
}

/// Downloads, validates and installs the database; the error is the message printed.
fn install(ctx: &Ctx, raw: &RawArgs) -> std::result::Result<Value, String> {
    let app = ctx.app().map_err(|e| e.to_string())?;
    let target = raw.opt("out").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(app.config.get_str("geoip.country_db").unwrap_or_default())
    });
    let tpl = raw.opt("url").map(str::to_string).unwrap_or_else(|| {
        match app.config.get_opt("geoip.url") {
            Some(Value::String(s)) => s.clone(),
            Some(other) if !other.is_null() => other.to_string(),
            _ => DEFAULT_URL.to_string(),
        }
    });
    let now = app.now();
    let this_month = time::gm_format(now, "%Y-%m");
    let first = time::parse_day(&format!("{this_month}-01"));
    let prev_month = first
        .and_then(|d| d.pred_opt())
        .map(|d| d.format("%Y-%m").to_string())
        .unwrap_or_else(|| this_month.clone());
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    if let Err(e) = fs::ensure_dir(&dir, 0o755, None, None) {
        return Err(format!("GeoIP update failed: {e}"));
    }
    let base = dir.join(format!(
        ".{}.{}",
        target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    let download = PathBuf::from(format!("{}.download", base.display()));
    let tmp = PathBuf::from(format!("{}.tmp", base.display()));
    let cleanup = || {
        let _ = std::fs::remove_file(&download);
        let _ = std::fs::remove_file(&tmp);
    };
    let mut used: Option<String> = None;
    let mut errors: Vec<String> = Vec::new();
    let mut months = vec![this_month.clone()];
    if prev_month != this_month {
        months.push(prev_month);
    }
    for ym in months {
        let (y, m) = ym.split_once('-').unwrap_or((&ym, ""));
        let url = tpl.replace("{YYYY}", y).replace("{MM}", m);
        match fetch_to(&url, &download, MAX_DOWNLOAD) {
            Ok(()) => {
                used = Some(url);
                break;
            }
            Err(e) => errors.push(e.to_string()),
        }
        if url == tpl {
            break; // no placeholder: no point in retrying
        }
    }
    let Some(used) = used else {
        cleanup();
        return Err(format!("GeoIP download failed: {}", errors.join("; ")));
    };
    let mut head = [0u8; 2];
    let is_gz = File::open(&download)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head == [0x1f, 0x8b];
    if is_gz {
        if let Err(e) = gunzip_to(&download, &tmp) {
            cleanup();
            return Err(format!("GeoIP update failed: {e}"));
        }
        let _ = std::fs::remove_file(&download);
    } else if std::fs::rename(&download, &tmp).is_err() {
        cleanup();
        return Err(format!("cannot write {}", tmp.display()));
    }
    let meta = match crate::engine::classify::geoip::open_reader(&tmp) {
        Ok(r) => r.metadata().clone(),
        Err(e) => {
            cleanup();
            return Err(format!("downloaded file is not a valid MaxMind DB: {e}"));
        }
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
    }
    let bytes = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    if std::fs::rename(&tmp, &target).is_err() {
        cleanup();
        return Err(format!("cannot move the database to {}", target.display()));
    }
    cleanup();
    let info = json!({
        "file": target.display().to_string(),
        "source": used,
        "database_type": meta.database_type,
        "build_epoch": meta.build_epoch,
        "build_date": time::gm_format(meta.build_epoch as i64, "%Y-%m-%d"),
        "bytes": bytes,
    });
    app.log
        .log_ctx(Level::Info, "geoip database updated", &info);
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::redirect_target;
    #[test]
    fn redirects() {
        assert_eq!(
            redirect_target("https://a.example/x/y.gz", "https://b.example/z").as_deref(),
            Some("https://b.example/z")
        );
        assert_eq!(
            redirect_target("https://a.example/x/y.gz", "http://b.example/z"),
            None
        );
        assert_eq!(
            redirect_target("https://a.example/x/y.gz", "/z").as_deref(),
            Some("https://a.example/z")
        );
        assert_eq!(
            redirect_target("https://a.example/x/y.gz", "z.gz").as_deref(),
            Some("https://a.example/x/z.gz")
        );
        assert_eq!(
            redirect_target("https://a.example/x/y.gz", "//c.example/q").as_deref(),
            Some("https://c.example/q")
        );
        assert_eq!(redirect_target("http://a.example/x", "/z"), None);
        assert_eq!(redirect_target("https://a.example/x", "ftp://z"), None);
    }
}
