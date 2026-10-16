//! Shared helpers of the engine integration tests (port of tests/support/*.php):
//! a fake cPanel server root ([`FakeRoot`]), log line builders ([`line`], [`ts`]),
//! database query helpers ([`Q`]) and a minimal MaxMind DB writer ([`MmdbWriter`]).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{NaiveDateTime, TimeZone};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

pub const CHROME: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36";
pub const IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";
pub const GOOGLEBOT: &str =
    "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";

/// Repository root.
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The binary under test.
pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_webpulse"))
}

/// Result of a CLI run.
#[derive(Debug, Clone)]
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub json: Value,
}

impl Out {
    /// Asserts exit code 0 and returns self.
    pub fn ok(self, what: &str) -> Out {
        assert_eq!(
            self.code, 0,
            "{what} failed: {}{}",
            self.stderr, self.stdout
        );
        self
    }
    /// Integer field of the JSON output.
    pub fn i(&self, k: &str) -> i64 {
        self.json[k]
            .as_i64()
            .unwrap_or_else(|| panic!("no integer {k} in {}", self.stdout))
    }
}

/// Runs the binary with `args` and extra environment (WEBPULSE_* cleared first).
pub fn exec(args: &[String], env: &[(&str, String)]) -> Out {
    let mut c = Command::new(bin());
    c.args(args)
        .current_dir(repo())
        .env_remove("WEBPULSE_NOW")
        .env_remove("WEBPULSE_CONFIG")
        .env("TZ", "UTC");
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.stdin(Stdio::null()).output().expect("run webpulse");
    let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
    Out {
        code: o.status.code().unwrap_or(-1),
        json: serde_json::from_str(&stdout).unwrap_or(Value::Null),
        stdout,
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// `'quoted words'` aware split of a command line.
pub fn split(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for c in args.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                has = true;
            }
            ' ' if !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

fn deep_merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(bv) if bv.is_object() && v.is_object() => deep_merge(bv, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let t = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).unwrap();
        }
    }
}

/// Removes a directory below tests/tmp (any file type, symlinks not followed).
pub fn rm(dir: &Path) {
    assert!(
        dir.starts_with(repo().join("tests/tmp")),
        "refusing to delete {}",
        dir.display()
    );
    let Ok(st) = std::fs::symlink_metadata(dir) else {
        return;
    };
    if !st.is_dir() {
        let _ = std::fs::remove_file(dir);
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
    for e in std::fs::read_dir(dir).unwrap() {
        rm(&e.unwrap().path());
    }
    let _ = std::fs::remove_dir(dir);
}

/// A fake cPanel server under tests/tmp/<name>/ (skeleton copied from
/// tests/fixtures/fakeroot) with a config.json pointing into it.
pub struct FakeRoot {
    dir: PathBuf,
    pub now: Option<i64>,
}

impl FakeRoot {
    /// Creates (wiping any previous content) tests/tmp/<name>; `config` is merged
    /// into the default configuration.
    pub fn create(name: &str, config: Value) -> FakeRoot {
        let dir = repo().join("tests/tmp").join(name);
        rm(&dir);
        copy_dir(&repo().join("tests/fixtures/fakeroot"), &dir.join("root"));
        let fr = FakeRoot { dir, now: None };
        for d in [
            "etc/apache2/logs/domlogs/alice",
            "etc/apache2/logs/domlogs/bob",
            "home/alice/logs",
            "home/bob/logs",
            "var/webpulse",
            "var/log/nginx/domains",
            "etc/cron.d",
        ] {
            std::fs::create_dir_all(fr.path(d)).unwrap();
        }
        let mut cfg = fr.default_config();
        deep_merge(&mut cfg, &config);
        fr.write_config(&cfg);
        fr
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn root(&self) -> PathBuf {
        self.dir.join("root")
    }
    pub fn config_path(&self) -> PathBuf {
        self.dir.join("config.json")
    }
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root().join(rel.trim_start_matches('/'))
    }
    pub fn domlog(&self, user: &str, file: &str) -> PathBuf {
        self.path(&format!("etc/apache2/logs/domlogs/{user}/{file}"))
    }

    pub fn default_config(&self) -> Value {
        let r = self.root().display().to_string();
        json!({
            "version": 1, "default_enabled": true, "timezone": "Europe/Rome", "raw_retention_days": 30,
            "max_rows_per_dim": 1000, "session_timeout_min": 30, "store_ip": false, "exclude_ips": [],
            "exclude_paths": ["/.well-known/", "/cpanel", "/whm", "/webmail", "/cgi-sys/"], "count_head_requests": false,
            "geoip": {"enabled": false, "country_db": format!("{r}/var/webpulse/geoip/dbip-country-lite.mmdb"), "city_db": null},
            "php_bin": null,
            "paths": {
                "data_dir": format!("{r}/var/webpulse/data"), "registry_db": format!("{r}/var/webpulse/webpulse.db"),
                "log_dir": format!("{r}/var/webpulse/log"), "run_dir": format!("{r}/var/webpulse/run"),
                "domlogs_dirs": [format!("{r}/etc/apache2/logs/domlogs/{{user}}"), format!("{r}/etc/apache2/logs/domlogs"), format!("{r}/usr/local/apache/domlogs/{{user}}"), format!("{r}/var/log/nginx/domains")],
                "archive_dir": format!("{r}/home/{{user}}/logs"), "userdomains_file": format!("{r}/etc/userdomains"),
                "cpanel_users_dir": format!("{r}/var/cpanel/users"), "cpanel_userdata_dir": format!("{r}/var/cpanel/userdata"),
                "home_root": format!("{r}/home"), "cron_file": format!("{r}/etc/cron.d/webpulse")
            }
        })
    }

    pub fn write_config(&self, cfg: &Value) {
        std::fs::write(
            self.config_path(),
            serde_json::to_string_pretty(cfg).unwrap() + "\n",
        )
        .unwrap();
    }

    /// Merges `patch` into config.json.
    pub fn patch_config(&self, patch: Value) {
        let mut cfg: Value =
            serde_json::from_slice(&std::fs::read(self.config_path()).unwrap()).unwrap();
        deep_merge(&mut cfg, &patch);
        self.write_config(&cfg);
    }

    /// Pins the clock of later runs.
    pub fn set_now(&mut self, ts: i64) {
        self.now = Some(ts);
    }

    /// Appends (or writes) lines to a file inside the fake root.
    pub fn write_log(&self, rel: &str, lines: &[String], append: bool) -> PathBuf {
        let file = self.path(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut data = String::new();
        for l in lines {
            data.push_str(l);
            data.push('\n');
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&file)
            .unwrap();
        f.write_all(data.as_bytes()).unwrap();
        file
    }

    /// Runs `webpulse dev gen-logs` into this root; returns its JSON summary.
    pub fn generate_logs(&self, opts: &[(&str, &str)]) -> Value {
        let mut args: Vec<String> = vec![
            "dev".into(),
            "gen-logs".into(),
            "--out".into(),
            self.root().display().to_string(),
        ];
        let have = |k: &str| opts.iter().any(|(x, _)| *x == k);
        let mut defaults: Vec<(&str, &str)> = Vec::new();
        for (k, v) in [
            ("seed", "42"),
            ("days", "3"),
            ("lines", "3000"),
            ("end", "2026-10-16"),
        ] {
            if !have(k) {
                defaults.push((k, v));
            }
        }
        for (k, v) in opts.iter().chain(defaults.iter()) {
            args.push(format!("--{k}"));
            if !v.is_empty() {
                args.push(v.to_string());
            }
        }
        let o = exec(&args, &[]);
        assert_eq!(o.code, 0, "gen-logs failed: {}", o.stderr);
        o.json
    }

    /// Runs the binary with this root's config (and pinned clock).
    pub fn run(&self, args: &str) -> Out {
        self.run_env(args, &[])
    }

    /// [`FakeRoot::run`] with extra environment.
    pub fn run_env(&self, args: &str, env: &[(&str, String)]) -> Out {
        let mut e: Vec<(&str, String)> =
            vec![("WEBPULSE_CONFIG", self.config_path().display().to_string())];
        if let Some(n) = self.now {
            e.push(("WEBPULSE_NOW", n.to_string()));
        }
        e.extend(env.iter().cloned());
        exec(&split(args), &e)
    }

    /// Boots an App on this root in-process.
    pub fn app(&self) -> webpulse::app::App {
        webpulse::app::App::try_boot(webpulse::app::Global {
            config: Some(self.config_path()),
            ..Default::default()
        })
        .expect("boot")
    }

    /// Read-write connection to a domain database (panics when missing).
    pub fn db(&self, user: &str, domain: &str) -> Connection {
        let f = self.path(&format!("var/webpulse/data/{user}/{domain}.db"));
        assert!(f.is_file(), "no database {}", f.display());
        Connection::open(f).unwrap()
    }

    pub fn has_db(&self, user: &str, domain: &str) -> bool {
        self.path(&format!("var/webpulse/data/{user}/{domain}.db"))
            .is_file()
    }

    /// Single integer of a query on a domain database.
    pub fn scalar(&self, user: &str, domain: &str, sql: &str) -> i64 {
        self.db(user, domain)
            .query_row(sql, [], |r| r.get::<_, Option<i64>>(0))
            .unwrap()
            .unwrap_or(0)
    }

    pub fn log_text(&self) -> String {
        std::fs::read_to_string(self.path("var/webpulse/log/webpulse.log")).unwrap_or_default()
    }
}

/// Unix timestamp of a local time `Y-m-d H:i:s` (Europe/Rome by default).
pub fn ts(local: &str) -> i64 {
    ts_tz(local, "Europe/Rome")
}

pub fn ts_tz(local: &str, tz: &str) -> i64 {
    let tz: chrono_tz::Tz = tz.parse().unwrap();
    let n = NaiveDateTime::parse_from_str(local, "%Y-%m-%d %H:%M:%S").unwrap();
    tz.from_local_datetime(&n).earliest().unwrap().timestamp()
}

/// A combined-format line (`Lines::line` of the PHP tests).
#[derive(Debug, Clone)]
pub struct L {
    pub ip: String,
    pub local: String,
    pub path: String,
    pub status: i64,
    pub bytes: i64,
    pub referrer: Option<String>,
    pub ua: String,
    pub method: String,
    pub tz: String,
    pub proto: String,
}

impl L {
    pub fn new(ip: &str, local: &str, path: &str) -> L {
        L {
            ip: ip.into(),
            local: local.into(),
            path: path.into(),
            status: 200,
            bytes: 1000,
            referrer: None,
            ua: CHROME.into(),
            method: "GET".into(),
            tz: "Europe/Rome".into(),
            proto: "HTTP/2.0".into(),
        }
    }
    pub fn status(mut self, s: i64) -> L {
        self.status = s;
        self
    }
    pub fn bytes(mut self, b: i64) -> L {
        self.bytes = b;
        self
    }
    pub fn referrer(mut self, r: &str) -> L {
        self.referrer = Some(r.into());
        self
    }
    pub fn ua(mut self, u: &str) -> L {
        self.ua = u.into();
        self
    }
    pub fn method(mut self, m: &str) -> L {
        self.method = m.into();
        self
    }
    pub fn proto(mut self, p: &str) -> L {
        self.proto = p.into();
        self
    }
    pub fn s(&self) -> String {
        let tz: chrono_tz::Tz = self.tz.parse().unwrap();
        let n = NaiveDateTime::parse_from_str(&self.local, "%Y-%m-%d %H:%M:%S").unwrap();
        let d = tz.from_local_datetime(&n).earliest().unwrap();
        format!(
            "{} - - [{}] \"{} {} {}\" {} {} \"{}\" \"{}\"",
            self.ip,
            d.format("%d/%b/%Y:%H:%M:%S %z"),
            self.method,
            self.path,
            self.proto,
            self.status,
            if self.bytes == 0 {
                "-".to_string()
            } else {
                self.bytes.to_string()
            },
            self.referrer.as_deref().unwrap_or("-"),
            self.ua
        )
    }
}

/// `Lines::line($ip, $local, $path)` with defaults.
pub fn line(ip: &str, local: &str, path: &str) -> String {
    L::new(ip, local, path).s()
}

/// Query helpers for assertions on a domain database (`Q` of the PHP tests).
pub struct Q;

pub const DAILY: [&str; 12] = [
    "visits",
    "visitors",
    "pageviews",
    "hits",
    "bytes",
    "bounces",
    "duration",
    "downloads",
    "errors_4xx",
    "errors_5xx",
    "bot_hits",
    "bot_bytes",
];

impl Q {
    /// daily row of a day (zeros when missing).
    pub fn daily(db: &Connection, day: &str) -> Map<String, Value> {
        let mut out = Map::new();
        let row: Option<Vec<i64>> = db
            .query_row(
                &format!("SELECT {} FROM daily WHERE day = ?", DAILY.join(", ")),
                [day],
                |r| (0..12).map(|i| r.get::<_, i64>(i)).collect(),
            )
            .ok();
        for (i, c) in DAILY.iter().enumerate() {
            out.insert(
                c.to_string(),
                json!(row.as_ref().map(|r| r[i]).unwrap_or(0)),
            );
        }
        out
    }
    pub fn d(db: &Connection, day: &str, col: &str) -> i64 {
        Q::daily(db, day)[col].as_i64().unwrap()
    }
    /// sums of daily over all days.
    pub fn totals(db: &Connection) -> Map<String, Value> {
        let sql = format!(
            "SELECT {} FROM daily",
            DAILY
                .iter()
                .map(|c| format!("COALESCE(SUM({c}),0)"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let row: Vec<i64> = db
            .query_row(&sql, [], |r| (0..12).map(|i| r.get::<_, i64>(i)).collect())
            .unwrap();
        DAILY
            .iter()
            .zip(row)
            .map(|(c, v)| (c.to_string(), json!(v)))
            .collect()
    }
    pub fn t(db: &Connection, col: &str) -> i64 {
        Q::totals(db)[col].as_i64().unwrap()
    }
    /// Rows of one dimension summed over days, keyed by key, without all-zero rows.
    pub fn dim(
        db: &Connection,
        dim: &str,
        day: Option<&str>,
    ) -> std::collections::BTreeMap<String, [i64; 6]> {
        let sql = format!(
            "SELECT key, SUM(visits), SUM(pageviews), SUM(hits), SUM(bytes), SUM(bounces), SUM(duration) FROM dim_daily WHERE dim = ?1{} GROUP BY key ORDER BY key",
            if day.is_some() { " AND day = ?2" } else { "" }
        );
        let mut st = db.prepare(&sql).unwrap();
        let map = |r: &rusqlite::Row| -> rusqlite::Result<(String, [i64; 6])> {
            Ok((
                r.get(0)?,
                [
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ],
            ))
        };
        let rows: Vec<(String, [i64; 6])> = match day {
            Some(d) => st
                .query_map([dim, d], map)
                .unwrap()
                .map(|x| x.unwrap())
                .collect(),
            None => st
                .query_map([dim], map)
                .unwrap()
                .map(|x| x.unwrap())
                .collect(),
        };
        rows.into_iter()
            .filter(|(_, v)| v.iter().any(|&x| x != 0))
            .collect()
    }
    /// key → one column of a dimension (non-zero values only).
    pub fn col(
        db: &Connection,
        dim: &str,
        col: &str,
        day: Option<&str>,
    ) -> std::collections::BTreeMap<String, i64> {
        let i = [
            "visits",
            "pageviews",
            "hits",
            "bytes",
            "bounces",
            "duration",
        ]
        .iter()
        .position(|c| *c == col)
        .unwrap();
        Q::dim(db, dim, day)
            .into_iter()
            .filter(|(_, v)| v[i] != 0)
            .map(|(k, v)| (k, v[i]))
            .collect()
    }
    pub fn count(db: &Connection, table: &str, cond: &str) -> i64 {
        db.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE {cond}"),
            [],
            |r| r.get(0),
        )
        .unwrap()
    }
    pub fn sum(db: &Connection, sql: &str) -> i64 {
        db.query_row(sql, [], |r| r.get::<_, Option<i64>>(0))
            .unwrap()
            .unwrap_or(0)
    }
    /// Stable dump of the aggregate tables.
    pub fn snapshot(db: &Connection) -> String {
        let mut parts = Vec::new();
        for sql in [
            "SELECT * FROM daily ORDER BY day",
            "SELECT * FROM hourly ORDER BY day, hour",
            "SELECT * FROM dim_daily ORDER BY day, dim, key",
            "SELECT * FROM visits ORDER BY visit_id",
            "SELECT visitor, visit_id, day, start_ts, last_ts, pageviews FROM open_visits ORDER BY visitor",
            "SELECT COUNT(*) FROM events",
        ] {
            let mut st = db.prepare(sql).unwrap();
            let n = st.column_count();
            let rows: Vec<String> = st
                .query_map([], |r| {
                    Ok((0..n)
                        .map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(|x| x.unwrap())
                .collect();
            parts.push(rows.join("\n"));
        }
        parts.join("\n--\n")
    }
}

/// Minimal MaxMind DB writer for tests (port of tests/support/MmdbWriter.php).
pub struct MmdbWriter {
    ip_version: u32,
    record_size: u32,
    nodes: Vec<[Option<Rec>; 2]>,
    data: Vec<u8>,
    shared: Vec<String>,
    string_offsets: std::collections::HashMap<String, usize>,
}

#[derive(Clone, Copy)]
enum Rec {
    Node(usize),
    Data(usize),
}

/// A value to encode.
#[derive(Clone, Debug)]
pub enum M {
    Str(String),
    U16(u64),
    U32(u64),
    U64(u128),
    U128(u128),
    I32(i32),
    Double(f64),
    Float(f32),
    Bytes(Vec<u8>),
    Bool(bool),
    Arr(Vec<M>),
    Map(Vec<(String, M)>),
}

pub fn s(v: &str) -> M {
    M::Str(v.into())
}
pub fn map(e: &[(&str, M)]) -> M {
    M::Map(e.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

impl MmdbWriter {
    pub fn new(ip_version: u32, record_size: u32, shared: &[&str], filler: usize) -> MmdbWriter {
        let mut w = MmdbWriter {
            ip_version,
            record_size,
            nodes: vec![[None, None]],
            data: Vec::new(),
            shared: shared.iter().map(|s| s.to_string()).collect(),
            string_offsets: Default::default(),
        };
        if filler > 0 {
            let b = w.encode(&M::Bytes(vec![0; filler]));
            w.data.extend(b);
        }
        w
    }

    pub fn insert(&mut self, cidr: &str, record: &M) {
        let (addr, bits) = cidr
            .split_once('/')
            .map(|(a, b)| (a, Some(b.parse::<usize>().unwrap())))
            .unwrap_or((cidr, None));
        let ip: std::net::IpAddr = addr.parse().unwrap();
        let mut packed: Vec<u8> = match ip {
            std::net::IpAddr::V4(a) => a.octets().to_vec(),
            std::net::IpAddr::V6(a) => a.octets().to_vec(),
        };
        let mut bits = bits.unwrap_or(packed.len() * 8);
        if packed.len() == 4 && self.ip_version == 6 {
            let mut p = vec![0u8; 12];
            p.extend(&packed);
            packed = p;
            bits += 96;
        }
        let bin = self.encode(record);
        let offset = self.data.len();
        self.data.extend(bin);
        let mut node = 0;
        for i in 0..bits {
            let bit = ((packed[i >> 3] >> (7 - (i & 7))) & 1) as usize;
            if i == bits - 1 {
                self.nodes[node][bit] = Some(Rec::Data(offset));
                break;
            }
            let next = match self.nodes[node][bit] {
                Some(Rec::Node(n)) => n,
                _ => {
                    self.nodes.push([None, None]);
                    let n = self.nodes.len() - 1;
                    self.nodes[node][bit] = Some(Rec::Node(n));
                    n
                }
            };
            node = next;
        }
    }

    pub fn build(&mut self, meta: &[(&str, M)]) -> Vec<u8> {
        let count = self.nodes.len();
        let mut tree = Vec::new();
        for n in &self.nodes {
            let v: Vec<u64> = n
                .iter()
                .map(|x| match x {
                    None => count as u64,
                    Some(Rec::Node(i)) => *i as u64,
                    Some(Rec::Data(o)) => (count + 16 + o) as u64,
                })
                .collect();
            let (l, r) = (v[0], v[1]);
            match self.record_size {
                24 => {
                    tree.extend(&(l as u32).to_be_bytes()[1..]);
                    tree.extend(&(r as u32).to_be_bytes()[1..]);
                }
                28 => {
                    tree.extend(&(l as u32).to_be_bytes()[1..]);
                    tree.push(((((l >> 24) & 0x0f) << 4) | ((r >> 24) & 0x0f)) as u8);
                    tree.extend(&(r as u32).to_be_bytes()[1..]);
                }
                _ => {
                    tree.extend(&(l as u32).to_be_bytes());
                    tree.extend(&(r as u32).to_be_bytes());
                }
            }
        }
        let mut md: Vec<(String, M)> = meta
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let defaults = vec![
            ("binary_format_major_version", M::U16(2)),
            ("binary_format_minor_version", M::U16(0)),
            ("build_epoch", M::U64(1_790_000_000)),
            ("database_type", s("WebPulse-Test")),
            ("description", map(&[("en", s("test database"))])),
            ("ip_version", M::U16(self.ip_version as u64)),
            ("languages", M::Arr(vec![s("en")])),
            ("node_count", M::U32(count as u64)),
            ("record_size", M::U16(self.record_size as u64)),
        ];
        for (k, v) in defaults {
            if !md.iter().any(|(x, _)| x == k) {
                md.push((k.to_string(), v));
            }
        }
        let shared = std::mem::take(&mut self.shared);
        let meta_bin = self.encode(&M::Map(md));
        self.shared = shared;
        let mut out = tree;
        out.extend([0u8; 16]);
        out.extend(&self.data);
        out.extend(b"\xAB\xCD\xEFMaxMind.com");
        out.extend(meta_bin);
        out
    }

    pub fn ctrl(t: u8, size: usize) -> Vec<u8> {
        let (sb, extra): (u8, Vec<u8>) = if size < 29 {
            (size as u8, vec![])
        } else if size < 285 {
            (29, vec![(size - 29) as u8])
        } else if size < 65821 {
            (30, ((size - 285) as u16).to_be_bytes().to_vec())
        } else {
            (31, ((size - 65821) as u32).to_be_bytes()[1..].to_vec())
        };
        let mut v = if t <= 7 {
            vec![(t << 5) | sb]
        } else {
            vec![sb, t - 7]
        };
        v.extend(extra);
        v
    }

    pub fn pointer(p: usize) -> Vec<u8> {
        if p < 2048 {
            vec![0x20 | (p >> 8) as u8, (p & 0xff) as u8]
        } else if p < 526336 {
            let v = p - 2048;
            vec![
                0x20 | (1 << 3) | (v >> 16) as u8,
                ((v >> 8) & 0xff) as u8,
                (v & 0xff) as u8,
            ]
        } else if p < 134744064 {
            let v = p - 526336;
            vec![
                0x20 | (2 << 3) | (v >> 24) as u8,
                ((v >> 16) & 0xff) as u8,
                ((v >> 8) & 0xff) as u8,
                (v & 0xff) as u8,
            ]
        } else {
            let mut v = vec![0x20 | (3 << 3)];
            v.extend((p as u32).to_be_bytes());
            v
        }
    }

    fn uint(v: u128, max: usize) -> Vec<u8> {
        let b = v.to_be_bytes();
        let start = b.iter().position(|&x| x != 0).unwrap_or(16);
        let mut out = b[start..].to_vec();
        if out.len() > max {
            out = out[out.len() - max..].to_vec();
        }
        out
    }

    fn encode(&mut self, v: &M) -> Vec<u8> {
        let with = |t: u8, b: Vec<u8>| {
            let mut o = MmdbWriter::ctrl(t, b.len());
            o.extend(b);
            o
        };
        match v {
            M::U16(x) => with(5, MmdbWriter::uint(*x as u128, 2)),
            M::U32(x) => with(6, MmdbWriter::uint(*x as u128, 4)),
            M::U64(x) => with(9, MmdbWriter::uint(*x, 8)),
            M::U128(x) => with(10, MmdbWriter::uint(*x, 16)),
            M::I32(x) => with(8, x.to_be_bytes().to_vec()),
            M::Double(x) => with(3, x.to_be_bytes().to_vec()),
            M::Float(x) => with(15, x.to_be_bytes().to_vec()),
            M::Bytes(b) => with(4, b.clone()),
            M::Bool(b) => MmdbWriter::ctrl(14, *b as usize),
            M::Str(st) => {
                if self.shared.contains(st) {
                    if let Some(&o) = self.string_offsets.get(st) {
                        return MmdbWriter::pointer(o);
                    }
                    let off = self.data.len();
                    let enc = with(2, st.as_bytes().to_vec());
                    self.data.extend(enc);
                    self.string_offsets.insert(st.clone(), off);
                    return MmdbWriter::pointer(off);
                }
                with(2, st.as_bytes().to_vec())
            }
            M::Arr(a) => {
                let mut o = MmdbWriter::ctrl(11, a.len());
                for e in a {
                    let x = self.encode(e);
                    o.extend(x);
                }
                o
            }
            M::Map(m) => {
                let mut o = MmdbWriter::ctrl(7, m.len());
                for (k, e) in m {
                    let kk = self.encode(&M::Str(k.clone()));
                    o.extend(kk);
                    let x = self.encode(e);
                    o.extend(x);
                }
                o
            }
        }
    }
}
