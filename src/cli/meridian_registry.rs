//! `webpulse meridian-registry add|remove` — maintains the WebPulse hub entry in the
//! Meridian theme's hub registry (`_assets/_registry.js`), the file that drives the
//! theme's side navigation. cPanel regenerates that file on theme updates, so the
//! `System::upcp` post hook re-runs `add` after every update.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Ctx, Kind, RawArgs, usage};
use crate::error::exit;

/// One-line description.
pub const SUMMARY: &str = "add or remove the WebPulse hub in the Meridian side-menu registry";

/// `--help` text.
pub const USAGE: &str =
    "Usage: webpulse meridian-registry add --registry FILE --manifest FILE [--entry FILE]
       webpulse meridian-registry remove --registry FILE [--slug SLUG]

Meridian builds its side navigation from the hub manifests listed in
/usr/local/cpanel/base/frontend/meridian/_assets/_registry.js, a file that
cPanel regenerates on theme updates. `add` inserts (or replaces) the entry for
the manifest's slug, setting entryMtime from --entry; `remove` deletes the
entry (default slug: webpulse). Both are idempotent, keep every other entry
untouched and write the file atomically.";

/// Options.
pub const SPEC: &[(&str, Kind)] = &[
    ("registry", Kind::Value),
    ("manifest", Kind::Value),
    ("entry", Kind::Value),
    ("slug", Kind::Value),
];

/// Arguments (clap description).
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// add | remove
    pub action: Option<String>,
    /// Path of Meridian's _registry.js
    #[arg(long)]
    pub registry: Option<String>,
    /// Hub manifest.json to insert (add)
    #[arg(long)]
    pub manifest: Option<String>,
    /// remoteEntry.js whose mtime becomes entryMtime (add)
    #[arg(long)]
    pub entry: Option<String>,
    /// Slug to remove (remove, default webpulse)
    #[arg(long)]
    pub slug: Option<String>,
}

/// Default hub slug.
pub const DEFAULT_SLUG: &str = "webpulse";

const MARKER: &str = "window.__MERIDIAN_REGISTRY__";
const NOTE: &str = "// The \"webpulse\" hub entry is maintained by `webpulse meridian-registry` (re-applied after cPanel updates).\n";

/// Entry point from the dispatcher.
pub fn main(ctx: &Ctx, raw: &RawArgs) -> Result<i32> {
    let action = raw
        .arg(0)
        .ok_or_else(|| usage("'add' or 'remove' is required"))?;
    let registry = raw
        .opt("registry")
        .ok_or_else(|| usage("--registry FILE is required"))?;
    match action {
        "add" => {
            let manifest = raw
                .opt("manifest")
                .ok_or_else(|| usage("--manifest FILE is required for add"))?;
            let (slug, n) = add(
                Path::new(registry),
                Path::new(manifest),
                raw.opt("entry").map(Path::new),
            )?;
            ctx.result(
                &json!({"action": "add", "slug": slug, "registry": registry, "entries": n}),
                || format!("hub '{slug}' registered in {registry} ({n} entries)"),
            );
        }
        "remove" => {
            let slug = raw.opt("slug").unwrap_or(DEFAULT_SLUG);
            let (removed, n) = remove(Path::new(registry), slug)?;
            ctx.result(
                &json!({"action": "remove", "slug": slug, "registry": registry, "removed": removed, "entries": n}),
                || {
                    if removed {
                        format!("hub '{slug}' removed from {registry} ({n} entries left)")
                    } else {
                        format!("hub '{slug}' was not registered in {registry}")
                    }
                },
            );
        }
        other => return Err(usage(format!("unknown action '{other}' (add|remove)"))),
    }
    Ok(exit::OK)
}

/// The registry file split into: text before the JSON array, the entries, text after.
#[derive(Debug, Clone, PartialEq)]
pub struct Registry {
    pub prefix: String,
    pub entries: Vec<Value>,
    pub suffix: String,
}

impl Registry {
    /// Parses the `window.__MERIDIAN_REGISTRY__ = [...]` file.
    pub fn parse(text: &str) -> Result<Registry> {
        let marker = text
            .find(MARKER)
            .with_context(|| format!("{MARKER} not found: not a Meridian registry file"))?;
        let rel = text[marker..]
            .find('[')
            .context("no JSON array after the registry marker")?;
        let start = marker + rel;
        let rest = &text[start..];
        let mut de = serde_json::Deserializer::from_str(rest);
        let value = Value::deserialize(&mut de).context("registry array is not valid JSON")?;
        let consumed = {
            // serde_json has no public offset on Deserializer; re-scan with a stream.
            let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
            stream.next();
            stream.byte_offset()
        };
        let entries = match value {
            Value::Array(a) => a,
            _ => bail!("registry value is not an array"),
        };
        Ok(Registry {
            prefix: text[..start].to_string(),
            entries,
            suffix: text[start + consumed..].to_string(),
        })
    }

    /// Serialises back, adding the WebPulse note once.
    pub fn render(&self) -> String {
        let mut prefix = self.prefix.clone();
        if !prefix.contains(NOTE)
            && let Some(pos) = prefix.find(MARKER)
        {
            prefix.insert_str(pos, NOTE);
        }
        let body = serde_json::to_string_pretty(&Value::Array(self.entries.clone()))
            .unwrap_or_else(|_| "[]".to_string());
        let suffix = if self.suffix.trim().is_empty() {
            ";\n".to_string()
        } else {
            self.suffix.clone()
        };
        format!("{prefix}{body}{suffix}")
    }

    /// Removes every entry with this slug; returns how many were removed.
    pub fn remove_slug(&mut self, slug: &str) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|e| e.get("slug").and_then(Value::as_str) != Some(slug));
        before - self.entries.len()
    }
}

fn load(path: &Path) -> Result<Registry> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Registry::parse(&text)
}

fn save(path: &Path, reg: &Registry) -> Result<()> {
    let text = reg.render();
    let mode = std::fs::metadata(path).ok().map(|m| {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o777
    });
    crate::util::fs::write_atomic(path, text.as_bytes(), mode.unwrap_or(0o644))
        .with_context(|| format!("writing {}", path.display()))
}

/// Inserts (replacing an existing entry with the same slug) the manifest into the
/// registry; returns the slug and the resulting number of entries.
pub fn add(registry: &Path, manifest: &Path, entry: Option<&Path>) -> Result<(String, usize)> {
    let text = std::fs::read_to_string(manifest)
        .with_context(|| format!("reading {}", manifest.display()))?;
    let mut m: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", manifest.display()))?;
    let slug = m
        .get("slug")
        .and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(str::to_string)
        .context("manifest has no valid \"slug\"")?;
    for key in ["id", "displayName", "version", "entrypoints"] {
        if m.get(key).is_none() {
            bail!("manifest lacks required key \"{key}\"");
        }
    }
    if let Some(e) = entry {
        let mtime = std::fs::metadata(e)
            .and_then(|md| md.modified())
            .with_context(|| format!("stat {}", e.display()))?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some(obj) = m.as_object_mut() {
            obj.insert("entryMtime".to_string(), json!(mtime));
        }
    }
    let mut reg = load(registry)?;
    reg.remove_slug(&slug);
    reg.entries.push(m);
    save(registry, &reg)?;
    Ok((slug, reg.entries.len()))
}

/// Removes the slug's entry; returns whether something was removed and the count left.
pub fn remove(registry: &Path, slug: &str) -> Result<(bool, usize)> {
    let mut reg = load(registry)?;
    let removed = reg.remove_slug(slug);
    if removed > 0 {
        save(registry, &reg)?;
    }
    Ok((removed > 0, reg.entries.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "// _registry.js -- auto-generated by rebuild-meridian-registry, do not edit\nwindow.__MERIDIAN_REGISTRY__ = [\n   {\n      \"slug\" : \"dashboard\",\n      \"id\" : \"com.cpanel.meridian-dashboard\",\n      \"nav\" : { \"path\" : \"/\" }\n   },\n   {\n      \"slug\" : \"email\",\n      \"id\" : \"com.cpanel.meridian-email\"\n   }\n];\n";

    #[test]
    fn parses_and_renders_round_trip() {
        let reg = Registry::parse(SAMPLE).unwrap();
        assert_eq!(reg.entries.len(), 2);
        assert!(reg.prefix.ends_with("window.__MERIDIAN_REGISTRY__ = "));
        assert_eq!(reg.suffix, ";\n");
        let again = Registry::parse(&reg.render()).unwrap();
        assert_eq!(again.entries, reg.entries);
        assert!(again.prefix.contains("maintained by"));
    }

    #[test]
    fn add_replaces_and_remove_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let registry = dir.path().join("_registry.js");
        std::fs::write(&registry, SAMPLE).unwrap();
        let manifest = dir.path().join("manifest.json");
        std::fs::write(
            &manifest,
            r#"{"slug":"webpulse","id":"net.inalto.webpulse","displayName":"WebPulse","version":"0.2.3","entrypoints":{"mf":"./remoteEntry.js"},"nav":{"path":"/webpulse"}}"#,
        )
        .unwrap();
        let entry = dir.path().join("remoteEntry.js");
        std::fs::write(
            &entry,
            "export function init(){}\nexport function get(){}\n",
        )
        .unwrap();

        let (slug, n) = add(&registry, &manifest, Some(&entry)).unwrap();
        assert_eq!((slug.as_str(), n), ("webpulse", 3));
        let (_, n) = add(&registry, &manifest, Some(&entry)).unwrap();
        assert_eq!(n, 3, "idempotent");
        let reg = Registry::parse(&std::fs::read_to_string(&registry).unwrap()).unwrap();
        let ours = reg
            .entries
            .iter()
            .find(|e| e["slug"] == "webpulse")
            .unwrap();
        assert!(ours["entryMtime"].as_u64().unwrap() > 1_600_000_000);
        assert_eq!(
            reg.entries[0]["slug"], "dashboard",
            "other entries untouched"
        );

        let (removed, n) = remove(&registry, "webpulse").unwrap();
        assert!(removed);
        assert_eq!(n, 2);
        let (removed, _) = remove(&registry, "webpulse").unwrap();
        assert!(!removed);
        assert!(Registry::parse(SAMPLE.replace(MARKER, "window.other").as_str()).is_err());
    }
}
