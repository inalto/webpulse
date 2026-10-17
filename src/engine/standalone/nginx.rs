//! nginx configuration reader (standalone discovery).
//!
//! Reads `nginx.conf` and its `include`s (wildcards, relative to the directory
//! of `nginx.conf`), then the `http` block: `log_format` definitions,
//! `access_log` at `http` level (inherited by servers without their own) and
//! every `server { server_name …; listen …; root …; access_log path [format]; }`.
//! `combined` is built in; a named format is combined-compatible when it
//! starts like combined (EL's `main` adds `"$http_x_forwarded_for"` at the end,
//! which the parser tolerates). `access_log off`, `syslog:` targets and paths
//! with variables are reported, not collected. `location`-level `access_log`s
//! are ignored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::apache::push_kind_notes;
use super::{FormatKind, Found, LOCAL, Site, SiteLog, Source, normalize_name, owner_of_path};

/// Include nesting limit.
const MAX_DEPTH: usize = 16;
/// Largest configuration file read.
const MAX_FILE: u64 = 4 * 1024 * 1024;
/// The standard configuration directory: absolute includes below it are read
/// relative to the directory of the `nginx.conf` given.
const CANONICAL: &str = "/etc/nginx";
/// nginx's default access log when none is configured anywhere.
pub const DEFAULT_LOG: &str = "/var/log/nginx/access.log";
/// The built-in `combined` format.
pub const COMBINED: &str = "$remote_addr - $remote_user [$time_local] \"$request\" $status $body_bytes_sent \"$http_referer\" \"$http_user_agent\"";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    Semi,
    Open,
    Close,
}

/// One directive with its block (`server { … }`).
#[derive(Debug, Clone, Default)]
pub struct Stmt {
    /// Directive name.
    pub name: String,
    /// Arguments (quotes removed).
    pub args: Vec<String>,
    /// Content of the block, `None` for a simple directive.
    pub block: Option<Vec<Stmt>>,
    /// `file:line`.
    pub at: String,
}

fn lex(text: &str) -> Vec<(Tok, usize)> {
    let b: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '#' => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            ';' => {
                out.push((Tok::Semi, line));
                i += 1;
            }
            '{' => {
                out.push((Tok::Open, line));
                i += 1;
            }
            '}' => {
                out.push((Tok::Close, line));
                i += 1;
            }
            '"' | '\'' => {
                let q = c;
                let start_line = line;
                let mut w = String::new();
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == '\\' && i + 1 < b.len() {
                        i += 1;
                        match b[i] {
                            'n' => w.push('\n'),
                            't' => w.push('\t'),
                            other => {
                                if other != q && other != '\\' {
                                    w.push('\\');
                                }
                                w.push(other);
                            }
                        }
                        i += 1;
                        continue;
                    }
                    if b[i] == '\n' {
                        line += 1;
                    }
                    w.push(b[i]);
                    i += 1;
                }
                i += 1;
                out.push((Tok::Word(w), start_line));
            }
            _ => {
                let mut w = String::new();
                while i < b.len() {
                    let d = b[i];
                    if d == '$' && i + 1 < b.len() && b[i + 1] == '{' {
                        while i < b.len() && b[i] != '}' {
                            w.push(b[i]);
                            i += 1;
                        }
                        if i < b.len() {
                            w.push('}');
                            i += 1;
                        }
                        continue;
                    }
                    if d.is_whitespace() || d == ';' || d == '{' || d == '}' {
                        break;
                    }
                    w.push(d);
                    i += 1;
                }
                out.push((Tok::Word(w), line));
            }
        }
    }
    out
}

struct Reader {
    base: PathBuf,
    chain: Vec<PathBuf>,
    warnings: Vec<String>,
    files: Vec<PathBuf>,
}

impl Reader {
    fn read_file(&mut self, file: &Path, required: bool) -> Vec<Stmt> {
        if self.chain.len() >= MAX_DEPTH {
            self.warnings.push(format!(
                "{}: includes nested too deeply, not read",
                file.display()
            ));
            return Vec::new();
        }
        let key = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        if self.chain.contains(&key) {
            self.warnings
                .push(format!("{}: recursive include ignored", file.display()));
            return Vec::new();
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
                return Vec::new();
            }
        };
        self.files.push(file.to_path_buf());
        self.chain.push(key);
        let toks = lex(&String::from_utf8_lossy(&raw));
        let mut pos = 0;
        let stmts = self.block(&toks, &mut pos, file);
        if pos < toks.len() {
            self.warnings.push(format!(
                "{}:{}: unexpected \"}}\"",
                file.display(),
                toks[pos].1
            ));
        }
        self.chain.pop();
        stmts
    }

    fn block(&mut self, toks: &[(Tok, usize)], pos: &mut usize, file: &Path) -> Vec<Stmt> {
        let mut out = Vec::new();
        let mut cur: Vec<String> = Vec::new();
        let mut line = 0;
        while *pos < toks.len() {
            let (t, l) = &toks[*pos];
            *pos += 1;
            match t {
                Tok::Word(w) => {
                    if cur.is_empty() {
                        line = *l;
                    }
                    cur.push(w.clone());
                }
                Tok::Semi => {
                    if cur.is_empty() {
                        continue;
                    }
                    let at = format!("{}:{line}", file.display());
                    let name = cur.remove(0);
                    if name == "include" {
                        for a in std::mem::take(&mut cur) {
                            out.extend(self.include(&a, &at));
                        }
                        continue;
                    }
                    out.push(Stmt {
                        name,
                        args: std::mem::take(&mut cur),
                        block: None,
                        at,
                    });
                }
                Tok::Open => {
                    let at = format!("{}:{line}", file.display());
                    let inner = self.block(toks, pos, file);
                    let (name, args) = if cur.is_empty() {
                        (String::new(), Vec::new())
                    } else {
                        let n = cur.remove(0);
                        (n, std::mem::take(&mut cur))
                    };
                    out.push(Stmt {
                        name,
                        args,
                        block: Some(inner),
                        at,
                    });
                }
                Tok::Close => {
                    return out;
                }
            }
        }
        out
    }

    fn include(&mut self, arg: &str, at: &str) -> Vec<Stmt> {
        let path = Path::new(arg);
        let full = if path.is_absolute() {
            // a copy of a configuration (tests) keeps its absolute includes inside
            match path.strip_prefix(CANONICAL) {
                Ok(rest) if self.base != Path::new(CANONICAL) => self.base.join(rest),
                _ => path.to_path_buf(),
            }
        } else {
            self.base.join(path)
        };
        let files = if crate::util::glob::has_magic(arg) {
            crate::util::glob::expand(&full, Path::new("/"))
        } else {
            if !full.exists() {
                self.warnings
                    .push(format!("{at}: include {} does not exist", full.display()));
                return Vec::new();
            }
            vec![full]
        };
        let mut out = Vec::new();
        for f in files {
            if f.is_file() {
                out.extend(self.read_file(&f, true));
            }
        }
        out
    }
}

/// Parses a configuration file into statements (includes expanded); `None`
/// when the file does not exist.
pub fn read(conf: &Path) -> Option<(Vec<Stmt>, Vec<String>, Vec<PathBuf>)> {
    if !conf.is_file() {
        return None;
    }
    let mut r = Reader {
        base: conf
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/")),
        chain: Vec::new(),
        warnings: Vec::new(),
        files: Vec::new(),
    };
    let stmts = r.read_file(conf, true);
    Some((stmts, r.warnings, r.files))
}

/// Classifies an nginx `log_format` string.
pub fn format_kind(fmt: &str) -> FormatKind {
    let norm: String = fmt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = norm.as_str();
    let cores = [
        "$remote_addr - $remote_user [$time_local] \"$request\" $status $body_bytes_sent",
        "$remote_addr - $remote_user [$time_local] \"$request\" $status $bytes_sent",
    ];
    let starts = |s: &str| cores.iter().find(|c| s.starts_with(*c)).copied();
    if starts(s).is_none() {
        // a leading vhost ($host, $server_name …), like Apache's %v
        if let Some((first, rest)) = s.split_once(' ')
            && first.starts_with('$')
            && !first.contains('"')
            && starts(rest).is_some()
        {
            s = rest;
        } else {
            return FormatKind::Incompatible;
        }
    }
    let Some(core) = starts(s) else {
        return FormatKind::Incompatible;
    };
    if s[core.len()..].starts_with(" \"$http_referer\" \"$http_user_agent\"") {
        FormatKind::Combined
    } else {
        FormatKind::Common
    }
}

#[derive(Debug, Clone)]
struct LogDecl {
    path: String,
    format: String,
    at: String,
}

/// Reads the nginx configuration; `None` when `conf` does not exist.
pub fn parse(conf: &Path, hostname: &str) -> Option<Found> {
    let (stmts, warnings, files) = read(conf)?;
    let mut f = Found {
        warnings,
        files,
        ..Found::default()
    };
    let mut formats: HashMap<String, String> = HashMap::new();
    formats.insert("combined".into(), COMBINED.into());
    let mut http_logs: Vec<LogDecl> = Vec::new();
    let mut http_off = false;
    let mut servers: Vec<&Stmt> = Vec::new();
    let mut any_log = false;
    for http in stmts.iter().filter(|s| s.name == "http") {
        for st in http.block.as_deref().unwrap_or(&[]) {
            match st.name.as_str() {
                "log_format" => {
                    if let Some((name, rest)) = st.args.split_first() {
                        let s: Vec<&str> = rest
                            .iter()
                            .filter(|a| !a.starts_with("escape="))
                            .map(String::as_str)
                            .collect();
                        formats.insert(name.clone(), s.concat());
                    }
                }
                "access_log" => {
                    any_log = true;
                    match decl(st) {
                        Some(d) => http_logs.push(d),
                        None => http_off = true,
                    }
                }
                "server" => servers.push(st),
                _ => {}
            }
        }
    }
    if !any_log {
        http_logs.push(LogDecl {
            path: DEFAULT_LOG.into(),
            format: "combined".into(),
            at: format!("{} (nginx default)", conf.display()),
        });
    }
    let make = |domain: &str,
                aliases: Vec<String>,
                owner: String,
                https: bool,
                decls: Vec<LogDecl>,
                mut notes: Vec<String>,
                origin: String|
     -> Site {
        let mut site = Site::new(Source::Nginx, domain, &owner);
        site.aliases = aliases;
        site.origin = Some(origin);
        for d in decls {
            if d.path.starts_with("syslog:") || d.path.contains('$') {
                notes.push(format!(
                    "{}: access_log {:?} is not supported (a file without variables is)",
                    d.at, d.path
                ));
                continue;
            }
            let path = if d.path.starts_with('/') {
                PathBuf::from(&d.path)
            } else {
                conf.parent().unwrap_or(Path::new("/")).join(&d.path)
            };
            let kind = match formats.get(&d.format) {
                Some(s) => format_kind(s),
                None => {
                    notes.push(format!("{}: unknown log_format {:?}", d.at, d.format));
                    FormatKind::Incompatible
                }
            };
            if site.logs.iter().any(|l| l.path == path) {
                continue;
            }
            site.logs.push(SiteLog {
                path,
                https,
                format: d.format.clone(),
                kind,
                origin: Some(d.at.clone()),
            });
        }
        site.warnings = notes;
        push_kind_notes(&mut site);
        site
    };
    // the http-level logs are shared by every server without its own: they are
    // counted under the default host
    if !http_logs.is_empty() {
        let shared: Vec<String> = http_logs.iter().map(|d| d.path.clone()).collect();
        let site = make(
            hostname,
            Vec::new(),
            LOCAL.to_string(),
            false,
            http_logs.clone(),
            Vec::new(),
            format!("{} (http block)", conf.display()),
        );
        f.sites.push(site);
        if http_off {
            f.warnings.push(format!(
                "{}: access_log off at http level next to access_log {}",
                conf.display(),
                shared.join(", ")
            ));
        }
    }
    let shared_list: Vec<String> = http_logs.iter().map(|d| d.path.clone()).collect();
    for srv in &servers {
        let block = srv.block.as_deref().unwrap_or(&[]);
        let mut names: Vec<String> = Vec::new();
        let mut https = false;
        let mut root: Option<PathBuf> = None;
        let mut logs: Vec<LogDecl> = Vec::new();
        let mut off = false;
        let mut notes: Vec<String> = Vec::new();
        for st in block {
            match st.name.as_str() {
                "server_name" => {
                    for a in &st.args {
                        let a = a.strip_prefix('.').unwrap_or(a);
                        if let Some(n) = normalize_name(a)
                            && !names.contains(&n)
                        {
                            names.push(n);
                        }
                    }
                }
                "listen" => {
                    if st.args.iter().any(|a| a == "ssl" || a == "quic")
                        || st
                            .args
                            .first()
                            .is_some_and(|a| a == "443" || a.ends_with(":443"))
                    {
                        https = true;
                    }
                }
                "ssl" => {
                    if st.args.first().is_some_and(|a| a == "on") {
                        https = true;
                    }
                }
                "root" => {
                    if let Some(r) = st.args.first()
                        && !r.contains('$')
                    {
                        root = Some(PathBuf::from(r));
                    }
                }
                "access_log" => match decl(st) {
                    Some(d) => logs.push(d),
                    None => off = true,
                },
                _ => {}
            }
        }
        let domain = names
            .first()
            .cloned()
            .unwrap_or_else(|| hostname.to_string());
        let owner = root
            .as_deref()
            .and_then(owner_of_path)
            .unwrap_or_else(|| LOCAL.to_string());
        if logs.is_empty() && off {
            notes.push("access_log off".into());
        } else if logs.is_empty() && !names.is_empty() && !shared_list.is_empty() {
            notes.push(format!(
                "no access_log of its own: its requests go to {} (counted under {hostname})",
                shared_list.join(", ")
            ));
        }
        let site = make(
            &domain,
            names.iter().skip(1).cloned().collect(),
            owner,
            https,
            logs,
            notes,
            srv.at.clone(),
        );
        f.sites.push(site);
    }
    Some(f)
}

/// `access_log path [format] [options]`; `None` for `access_log off`.
fn decl(st: &Stmt) -> Option<LogDecl> {
    let path = st.args.first()?;
    if path == "off" {
        return None;
    }
    let format = st
        .args
        .get(1)
        .filter(|a| !a.contains('='))
        .cloned()
        .unwrap_or_else(|| "combined".into());
    Some(LogDecl {
        path: path.clone(),
        format,
        at: st.at.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format_kind(COMBINED), FormatKind::Combined);
        let main = "$remote_addr - $remote_user [$time_local] \"$request\" $status $body_bytes_sent \"$http_referer\" \"$http_user_agent\" \"$http_x_forwarded_for\"";
        assert_eq!(format_kind(main), FormatKind::Combined);
        assert_eq!(
            format_kind(&format!("$host {COMBINED}")),
            FormatKind::Combined
        );
        assert_eq!(
            format_kind(
                "$remote_addr - $remote_user [$time_local] \"$request\" $status $body_bytes_sent"
            ),
            FormatKind::Common
        );
        assert_eq!(
            format_kind("{\"ip\":\"$remote_addr\"}"),
            FormatKind::Incompatible
        );
    }

    #[test]
    fn lexer() {
        let t = lex("log_format main '$a \"x\"' \"$b\";\nserver { listen 80; } # c\n");
        assert_eq!(t[0], (Tok::Word("log_format".into()), 1));
        assert_eq!(t[2].0, Tok::Word("$a \"x\"".into()));
        assert_eq!(t[3].0, Tok::Word("$b".into()));
        assert_eq!(t[4].0, Tok::Semi);
        assert_eq!(t[6], (Tok::Open, 2));
        assert_eq!(*t.last().unwrap(), (Tok::Close, 2));
        let t = lex("access_log /x/${host}.log;");
        assert_eq!(t[1].0, Tok::Word("/x/${host}.log".into()));
    }
}
