//! Apache httpd configuration reader (standalone discovery).
//!
//! Layouts: EL (`<dir>/conf/httpd.conf` including `conf.modules.d/*.conf` and
//! `conf.d/*.conf`) and Debian/Ubuntu (`<dir>/apache2.conf` including
//! `ports.conf`, `conf-enabled/*.conf`, `sites-enabled/*.conf`, with variables
//! from `<dir>/envvars`). The configuration directory is the server root:
//! relative paths (`logs/access_log`, `Include conf.d/*.conf`) resolve against
//! it, and absolute include paths below the layout's standard root
//! (`/etc/httpd`, `/etc/apache2`) are mapped onto it, so a copy of a real
//! configuration can be read from anywhere (tests).
//!
//! Understood: `Include`/`IncludeOptional` (wildcards, directories), `Define`
//! and `${VAR}`, `<VirtualHost>`, `ServerName`, `ServerAlias`, `DocumentRoot`,
//! `SuexecUserGroup`, `AssignUserID`, `SSLEngine`, `LogFormat` (nicknames and
//! the default format), `CustomLog`, `TransferLog`. Every other directive and
//! the conditions of `<IfModule>`/`<IfDefine>`/`<If>` are ignored (their
//! content is read); `<Macro>` bodies are skipped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{
    FormatKind, Found, LOCAL, Site, SiteLog, Source, normalize_name, owner_name, owner_of_path,
};

/// Include nesting limit.
const MAX_DEPTH: usize = 16;
/// Largest configuration file read.
const MAX_FILE: u64 = 4 * 1024 * 1024;
/// Apache's built-in default format (`TransferLog` without a `LogFormat`).
pub const DEFAULT_FORMAT: &str = "%h %l %u %t \"%r\" %>s %b";

#[derive(Debug, Clone, Default)]
struct Server {
    name: Option<String>,
    raw_name: Option<String>,
    aliases: Vec<String>,
    logs: Vec<SiteLog>,
    docroot: Option<PathBuf>,
    suexec: Option<String>,
    https: bool,
    origin: String,
    notes: Vec<String>,
}

struct Parser {
    root: PathBuf,
    canonical: &'static str,
    vars: HashMap<String, String>,
    formats: HashMap<String, String>,
    default_format: String,
    main: Server,
    vhosts: Vec<Server>,
    cur: Option<Server>,
    macro_depth: usize,
    chain: Vec<PathBuf>,
    warnings: Vec<String>,
    files: Vec<PathBuf>,
}

/// Reads the Apache configuration rooted at `dir`; `None` when `dir` holds no
/// main configuration file. `hostname` names the default virtual host.
pub fn parse(dir: &Path, hostname: &str) -> Option<Found> {
    let (main, canonical) = if dir.join("conf/httpd.conf").is_file() {
        (dir.join("conf/httpd.conf"), "/etc/httpd")
    } else if dir.join("apache2.conf").is_file() {
        (dir.join("apache2.conf"), "/etc/apache2")
    } else if dir.join("httpd.conf").is_file() {
        (dir.join("httpd.conf"), "/etc/httpd")
    } else {
        return None;
    };
    let mut p = Parser {
        root: dir.to_path_buf(),
        canonical,
        vars: HashMap::new(),
        formats: HashMap::new(),
        default_format: DEFAULT_FORMAT.into(),
        main: Server {
            origin: main.display().to_string(),
            ..Server::default()
        },
        vhosts: Vec::new(),
        cur: None,
        macro_depth: 0,
        chain: Vec::new(),
        warnings: Vec::new(),
        files: Vec::new(),
    };
    p.load_envvars();
    p.parse_file(&main, true);
    if let Some(v) = p.cur.take() {
        p.warnings
            .push(format!("{}: <VirtualHost> is not closed", v.origin));
        p.vhosts.push(v);
    }
    Some(p.finish(hostname))
}

/// Splits a configuration line into arguments (`"…"` / `'…'` quoting with
/// backslash escapes of the quote, like `ap_getword_conf`).
pub fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let mut tok = String::new();
        if b[i] == '"' || b[i] == '\'' {
            let q = b[i];
            i += 1;
            while i < b.len() && b[i] != q {
                if b[i] == '\\' && i + 1 < b.len() && (b[i + 1] == q || b[i + 1] == '\\') {
                    i += 1;
                }
                tok.push(b[i]);
                i += 1;
            }
            i += 1;
        } else {
            while i < b.len() && !b[i].is_whitespace() {
                tok.push(b[i]);
                i += 1;
            }
        }
        out.push(tok);
    }
    out
}

/// Classifies an Apache `LogFormat` string.
pub fn format_kind(fmt: &str) -> FormatKind {
    let t: Vec<&str> = fmt.split_whitespace().collect();
    let ip = |s: &str| matches!(s, "%h" | "%a" | "%{c}a");
    let mut i = 0;
    if t.len() > 1 && !ip(t[0]) && ip(t[1]) && t[0].starts_with('%') {
        // leading vhost: %v, %v:%p, %{Host}i …
        i = 1;
    }
    let at = |k: usize| t.get(i + k).copied().unwrap_or("");
    let ok = ip(at(0))
        && at(1) == "%l"
        && at(2) == "%u"
        && at(3) == "%t"
        && at(4) == "\"%r\""
        && matches!(at(5), "%>s" | "%s" | "%<s")
        && matches!(at(6), "%b" | "%B" | "%O");
    if !ok {
        return FormatKind::Incompatible;
    }
    if at(7).eq_ignore_ascii_case("\"%{Referer}i\"")
        && at(8).eq_ignore_ascii_case("\"%{User-Agent}i\"")
    {
        FormatKind::Combined
    } else {
        FormatKind::Common
    }
}

impl Parser {
    fn load_envvars(&mut self) {
        if self.canonical == "/etc/apache2" {
            for (k, v) in [
                ("APACHE_LOG_DIR", "/var/log/apache2"),
                ("APACHE_RUN_USER", "www-data"),
                ("APACHE_RUN_GROUP", "www-data"),
                ("APACHE_RUN_DIR", "/var/run/apache2"),
                ("APACHE_LOCK_DIR", "/var/lock/apache2"),
            ] {
                self.vars.insert(k.into(), v.into());
            }
        }
        let Ok(raw) = std::fs::read_to_string(self.root.join("envvars")) else {
            return;
        };
        for line in raw.lines() {
            let l = line.trim();
            let l = l.strip_prefix("export ").unwrap_or(l).trim();
            let Some((k, v)) = l.split_once('=') else {
                continue;
            };
            if k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                continue;
            }
            let v = v.trim().trim_matches(['"', '\'']);
            // Debian appends $SUFFIX (multiple instances); the default instance has none
            let v = v.replace("${SUFFIX}", "").replace("$SUFFIX", "");
            let v = self.expand(&v);
            self.vars.insert(k.to_string(), v);
        }
    }

    /// `${VAR}` (and `$VAR` in envvars) expansion; unknown variables stay.
    fn expand(&self, s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(p) = rest.find("${") {
            out.push_str(&rest[..p]);
            match rest[p + 2..].find('}') {
                Some(e) => {
                    let name = &rest[p + 2..p + 2 + e];
                    match self.vars.get(name) {
                        Some(v) => out.push_str(v),
                        None => out.push_str(&rest[p..p + 3 + e]),
                    }
                    rest = &rest[p + 3 + e..];
                }
                None => {
                    out.push_str(&rest[p..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Absolute path of a configuration reference.
    fn resolve(&self, p: &str) -> PathBuf {
        let path = Path::new(p);
        if path.is_absolute() {
            if self.root != Path::new(self.canonical)
                && let Ok(rest) = path.strip_prefix(self.canonical)
            {
                return self.root.join(rest);
            }
            return path.to_path_buf();
        }
        self.root.join(path)
    }

    fn server(&mut self) -> &mut Server {
        match self.cur.as_mut() {
            Some(v) => v,
            None => &mut self.main,
        }
    }

    fn parse_file(&mut self, file: &Path, required: bool) {
        if self.chain.len() >= MAX_DEPTH {
            self.warnings.push(format!(
                "{}: includes nested too deeply, not read",
                file.display()
            ));
            return;
        }
        let key = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        if self.chain.contains(&key) {
            self.warnings
                .push(format!("{}: recursive Include ignored", file.display()));
            return;
        }
        let raw = match std::fs::File::open(file).and_then(|f| {
            use std::io::Read;
            let mut v = Vec::new();
            f.take(MAX_FILE).read_to_end(&mut v).map(|_| v)
        }) {
            Ok(r) => r,
            Err(e) => {
                if required || e.kind() != std::io::ErrorKind::NotFound {
                    self.warnings
                        .push(format!("cannot read {}: {e}", file.display()));
                }
                return;
            }
        };
        self.files.push(file.to_path_buf());
        self.chain.push(key);
        let text = String::from_utf8_lossy(&raw);
        let mut logical = String::new();
        let mut start = 0usize;
        for (n, line) in text.lines().enumerate() {
            if logical.is_empty() {
                start = n + 1;
            }
            if let Some(l) = line.strip_suffix('\\') {
                logical.push_str(l);
                logical.push(' ');
                continue;
            }
            logical.push_str(line);
            let l = std::mem::take(&mut logical);
            self.line(l.trim(), &format!("{}:{start}", file.display()));
        }
        if !logical.is_empty() {
            let l = std::mem::take(&mut logical);
            self.line(l.trim(), &format!("{}:{start}", file.display()));
        }
        self.chain.pop();
    }

    fn line(&mut self, l: &str, at: &str) {
        if l.is_empty() || l.starts_with('#') {
            return;
        }
        if let Some(close) = l.strip_prefix("</") {
            let name = close.trim_end_matches('>').trim();
            if self.macro_depth > 0 {
                if name.eq_ignore_ascii_case("macro") {
                    self.macro_depth -= 1;
                }
                return;
            }
            if name.eq_ignore_ascii_case("virtualhost") {
                match self.cur.take() {
                    Some(v) => self.vhosts.push(v),
                    None => self
                        .warnings
                        .push(format!("{at}: </VirtualHost> without <VirtualHost>")),
                }
            }
            return;
        }
        if let Some(open) = l.strip_prefix('<') {
            let body = open.trim_end().trim_end_matches('>');
            let mut parts = body.splitn(2, char::is_whitespace);
            let name = parts.next().unwrap_or("");
            let args = parts.next().unwrap_or("");
            if name.eq_ignore_ascii_case("macro") || self.macro_depth > 0 {
                if name.eq_ignore_ascii_case("macro") {
                    self.macro_depth += 1;
                }
                return;
            }
            if name.eq_ignore_ascii_case("virtualhost") {
                if let Some(v) = self.cur.take() {
                    self.warnings
                        .push(format!("{}: <VirtualHost> is not closed", v.origin));
                    self.vhosts.push(v);
                }
                let args = self.expand(args);
                let https = tokenize(&args)
                    .iter()
                    .any(|a| a.rsplit(':').next() == Some("443"));
                self.cur = Some(Server {
                    https,
                    origin: at.to_string(),
                    ..Server::default()
                });
            }
            return;
        }
        if self.macro_depth > 0 {
            return;
        }
        let toks = tokenize(l);
        let Some((name, args)) = toks.split_first() else {
            return;
        };
        let args: Vec<String> = args.iter().map(|a| self.expand(a)).collect();
        match name.to_ascii_lowercase().as_str() {
            "define" => {
                if let Some(k) = args.first() {
                    self.vars
                        .insert(k.clone(), args.get(1).cloned().unwrap_or_default());
                }
            }
            "include" | "includeoptional" => {
                let optional = name.eq_ignore_ascii_case("includeoptional");
                for a in &args {
                    self.include(a, optional, at);
                }
            }
            "logformat" => match args.len() {
                0 => {}
                1 => self.default_format = args[0].clone(),
                _ => {
                    self.formats.insert(args[1].clone(), args[0].clone());
                }
            },
            "customlog" => {
                if let Some(path) = args.first() {
                    let fmt = args.get(1).cloned();
                    self.add_log(path, fmt, at);
                }
            }
            "transferlog" => {
                if let Some(path) = args.first() {
                    let fmt = Some(self.default_format.clone());
                    self.add_log(path, fmt, at);
                }
            }
            "servername" => {
                if let Some(n) = args.first() {
                    let s = self.server();
                    s.raw_name = Some(n.clone());
                    s.name = normalize_name(n);
                }
            }
            "serveralias" => {
                let names: Vec<String> = args.iter().filter_map(|a| normalize_name(a)).collect();
                let s = self.server();
                for n in names {
                    if !s.aliases.contains(&n) {
                        s.aliases.push(n);
                    }
                }
            }
            "documentroot" => {
                if let Some(d) = args.first()
                    && !d.contains("${")
                {
                    let p = PathBuf::from(d);
                    self.server().docroot = Some(p);
                }
            }
            "suexecusergroup" | "assignuserid" => {
                if let Some(u) = args.first() {
                    let u = u.clone();
                    self.server().suexec = Some(u);
                }
            }
            "sslengine" if args.first().is_some_and(|a| a.eq_ignore_ascii_case("on")) => {
                self.server().https = true;
            }
            _ => {}
        }
    }

    fn include(&mut self, arg: &str, optional: bool, at: &str) {
        let path = self.resolve(arg);
        let magic = crate::util::glob::has_magic(&path.to_string_lossy());
        let mut files: Vec<PathBuf> = if magic {
            crate::util::glob::expand(&path, Path::new("/"))
        } else {
            vec![path.clone()]
        };
        if !magic && !path.exists() {
            if !optional {
                self.warnings
                    .push(format!("{at}: Include {} does not exist", path.display()));
            }
            return;
        }
        // a directory includes every file below it, in alphabetical order
        let mut expanded = Vec::new();
        for f in files.drain(..) {
            if f.is_dir() {
                collect_files(&f, &mut expanded, 0);
            } else {
                expanded.push(f);
            }
        }
        for f in expanded {
            self.parse_file(&f, !optional);
        }
    }

    fn add_log(&mut self, raw: &str, fmt: Option<String>, at: &str) {
        if raw.starts_with('|') {
            let note =
                format!("{at}: piped log {raw:?} is not supported (log to a file to collect it)");
            self.server().notes.push(note);
            return;
        }
        if raw.starts_with("syslog:") {
            let note = format!("{at}: syslog logging {raw:?} is not supported");
            self.server().notes.push(note);
            return;
        }
        if raw.contains("${") {
            let note = format!("{at}: unresolved variable in log path {raw:?}");
            self.server().notes.push(note);
            return;
        }
        let path = {
            let p = Path::new(raw);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                self.root.join(p)
            }
        };
        let fmt = fmt.unwrap_or_else(|| self.default_format.clone());
        let (format, kind) = if fmt.contains('%') {
            (fmt.clone(), format_kind(&fmt))
        } else {
            match self.formats.get(&fmt) {
                Some(s) => (fmt.clone(), format_kind(s)),
                None => (fmt.clone(), FormatKind::Incompatible),
            }
        };
        let s = self.server();
        if s.logs.iter().any(|l| l.path == path) {
            return;
        }
        s.logs.push(SiteLog {
            path,
            https: s.https,
            format,
            kind,
            origin: Some(at.to_string()),
        });
    }

    fn owner(s: &Server) -> String {
        if let Some(o) = s.suexec.as_deref().and_then(owner_name) {
            return o;
        }
        if let Some(o) = s.docroot.as_deref().and_then(owner_of_path) {
            return o;
        }
        LOCAL.to_string()
    }

    fn finish(mut self, hostname: &str) -> Found {
        let mut f = Found {
            warnings: std::mem::take(&mut self.warnings),
            files: std::mem::take(&mut self.files),
            ..Found::default()
        };
        let main_name = self
            .main
            .name
            .clone()
            .unwrap_or_else(|| hostname.to_string());
        let main_logs: Vec<String> = self
            .main
            .logs
            .iter()
            .map(|l| l.path.display().to_string())
            .collect();
        if !self.main.logs.is_empty() || !self.main.notes.is_empty() {
            let mut site = Site::new(Source::Apache, &main_name, &Parser::owner(&self.main));
            site.aliases = self.main.aliases.clone();
            site.logs = self.main.logs.clone();
            site.origin = Some(format!("{} (main server)", self.main.origin));
            site.warnings = self.main.notes.clone();
            for l in &site.logs {
                if l.format.starts_with("vhost_") || l.format.starts_with("%v") {
                    site.warnings.push(format!(
                        "{} is shared by the virtual hosts without a log of their own: all of it is counted under {main_name}",
                        l.path.display()
                    ));
                }
            }
            push_kind_notes(&mut site);
            f.sites.push(site);
        }
        for v in self.vhosts {
            let name = match (&v.name, &v.raw_name) {
                (Some(n), _) => n.clone(),
                (None, Some(raw)) => {
                    f.warnings.push(format!(
                        "{}: ServerName {raw:?} is not a domain name, virtual host skipped",
                        v.origin
                    ));
                    continue;
                }
                (None, None) => main_name.clone(),
            };
            let mut site = Site::new(Source::Apache, &name, &Parser::owner(&v));
            site.aliases = v.aliases.clone();
            site.logs = v.logs.clone();
            site.origin = Some(v.origin.clone());
            site.warnings = v.notes.clone();
            if v.logs.is_empty() && !main_logs.is_empty() {
                site.warnings.push(format!(
                    "no CustomLog of its own: its requests go to {} (counted under {main_name})",
                    main_logs.join(", ")
                ));
            }
            push_kind_notes(&mut site);
            f.sites.push(site);
        }
        f
    }
}

/// Warnings for logs whose format cannot be read or lacks fields.
pub(super) fn push_kind_notes(site: &mut Site) {
    for l in &site.logs {
        match l.kind {
            FormatKind::Incompatible => site.warnings.push(format!(
                "{}: format {:?} is not combined-compatible, the log is not collected",
                l.path.display(),
                l.format
            )),
            FormatKind::Common => site.warnings.push(format!(
                "{}: format {:?} has no referrer and user agent (use combined)",
                l.path.display(),
                l.format
            )),
            _ => {}
        }
    }
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 4 {
        return;
    }
    for name in crate::util::fs::scandir(dir) {
        if name.starts_with('.') {
            continue;
        }
        let p = dir.join(&name);
        if p.is_dir() {
            collect_files(&p, out, depth + 1);
        } else {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_formats() {
        assert_eq!(
            tokenize(r#"LogFormat "%h %l %u %t \"%r\" %>s %b" common"#),
            vec!["LogFormat", "%h %l %u %t \"%r\" %>s %b", "common"]
        );
        assert_eq!(
            tokenize("CustomLog 'logs/a b' combined env=!x"),
            vec!["CustomLog", "logs/a b", "combined", "env=!x"]
        );
        let combined = "%h %l %u %t \"%r\" %>s %b \"%{Referer}i\" \"%{User-Agent}i\"";
        assert_eq!(format_kind(combined), FormatKind::Combined);
        assert_eq!(
            format_kind(&format!("{combined} %I %O")),
            FormatKind::Combined
        );
        assert_eq!(
            format_kind("%v:%p %h %l %u %t \"%r\" %>s %O \"%{Referer}i\" \"%{User-Agent}i\""),
            FormatKind::Combined
        );
        assert_eq!(format_kind(DEFAULT_FORMAT), FormatKind::Common);
        assert_eq!(
            format_kind("%t %h %{SSL_PROTOCOL}x %{SSL_CIPHER}x \"%r\" %b"),
            FormatKind::Incompatible
        );
        assert_eq!(format_kind("%{Referer}i -> %U"), FormatKind::Incompatible);
    }
}
