//! Shared helpers of the packaging tests (`tests/pkg_*.rs`): the installer, the
//! cPanel/WHM shims, the hooks, the release scripts and static checks of the
//! packaging files. Everything runs in throw-away directories below `tests/tmp/`
//! that are removed when the test ends (set `WEBPULSE_KEEP_TMP=1` to keep them):
//!
//! * [`TmpDir`]: a unique directory, removed on drop;
//! * [`Sandbox`]: a fake cPanel root whose tools (`install_plugin`,
//!   `register_appconfig`, `manage_hooks`, ...) are stubs that log their invocations,
//!   plus the directories `install.sh` is redirected to (`WEBPULSE_PREFIX`, ...);
//! * [`make_source`] / [`release_tarball`]: an unpacked release in the layout of
//!   `scripts/build_release.sh` with a stub `bin/webpulse`, and its tarball;
//! * [`cmd`]: runs a program with a clean environment (only `PATH`, `HOME`, `LANG`,
//!   `TMPDIR` are inherited) and returns the exit code and stdout+stderr.
#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Skips the rest of a test (Rust has no skip outcome): prints why and returns.
#[macro_export]
macro_rules! skip {
    ($($arg:tt)*) => {{
        eprintln!("SKIP: {}", format!($($arg)*));
        return;
    }};
}

/// `kv!["K" => v, ...]`: an environment list for [`Run::envs`] (values: anything
/// with `to_string()`).
#[macro_export]
macro_rules! kv {
    ($($k:expr => $v:expr),* $(,)?) => {
        vec![$(($k.to_string(), $v.to_string())),*] as Vec<(String, String)>
    };
}

/// Repository root.
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A path as a `String` (the tests only use UTF-8 paths).
pub fn s(p: &Path) -> String {
    p.to_str().expect("utf-8 path").to_string()
}

/// Absolute path of a program on the test's own `PATH`.
pub fn which(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return is_executable(&p).then_some(p);
    }
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into());
    std::env::split_paths(&path)
        .map(|d| d.join(program))
        .find(|p| is_executable(p))
}

/// Is an external program available?
pub fn have(program: &str) -> bool {
    which(program).is_some()
}

/// The `php` CLI, when installed (only the shim tests and `php -l` need it).
pub fn php() -> Option<PathBuf> {
    which("php")
}

pub fn is_executable(p: &Path) -> bool {
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Permission bits (with setuid/setgid/sticky) of a file, following symlinks.
pub fn mode(p: &Path) -> u32 {
    std::fs::metadata(p)
        .unwrap_or_else(|e| panic!("stat {}: {e}", p.display()))
        .permissions()
        .mode()
        & 0o7777
}

/// File contents as text ("" when missing).
pub fn read(p: &Path) -> String {
    std::fs::read(p)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

pub fn write(p: &Path, content: impl AsRef<[u8]>) {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(p, content).unwrap_or_else(|e| panic!("write {}: {e}", p.display()));
}

pub fn chmod(p: &Path, m: u32) {
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
}

/// Writes an executable (0755) script.
pub fn script(p: &Path, body: &str) -> PathBuf {
    write(p, body);
    chmod(p, 0o755);
    p.to_path_buf()
}

pub fn readlink(p: &Path) -> PathBuf {
    std::fs::read_link(p).unwrap_or_else(|e| panic!("readlink {}: {e}", p.display()))
}

pub fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

#[track_caller]
pub fn assert_exists(p: &Path) {
    assert!(exists(p), "missing: {}", p.display());
}

#[track_caller]
pub fn assert_missing(p: &Path) {
    assert!(!exists(p), "should not exist: {}", p.display());
}

fn clip(s: &str) -> String {
    if s.len() > 4000 {
        let mut end = 4000;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n...[clipped]", &s[..end])
    } else {
        s.to_string()
    }
}

#[track_caller]
pub fn assert_contains(haystack: &str, needle: &str, what: &str) {
    assert!(
        haystack.contains(needle),
        "{what}: '{needle}' not found in:\n{}",
        clip(haystack)
    );
}

#[track_caller]
pub fn assert_not_contains(haystack: &str, needle: &str, what: &str) {
    assert!(
        !haystack.contains(needle),
        "{what}: '{needle}' unexpectedly found in:\n{}",
        clip(haystack)
    );
}

/// Lower-case hex SHA-256 of a file.
pub fn sha256_file(p: &Path) -> String {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Effective uid of the test process.
pub fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

// ---------------------------------------------------------------------------
// Processes
// ---------------------------------------------------------------------------

/// A command to run: argv, extra environment, stdin, working directory.
pub struct Run {
    argv: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    stdin: Vec<u8>,
    cwd: Option<PathBuf>,
}

/// Starts building a command. The program is resolved on the test's own `PATH`
/// (so a test may give the child a restricted `PATH`).
pub fn cmd(argv: &[&str]) -> Run {
    cmd_os(argv)
}

/// [`cmd`] for arguments that are not all `&str`.
pub fn cmd_os<S: AsRef<OsStr>>(argv: &[S]) -> Run {
    Run {
        argv: argv.iter().map(|a| a.as_ref().to_os_string()).collect(),
        env: Vec::new(),
        stdin: Vec::new(),
        cwd: None,
    }
}

impl Run {
    pub fn env(mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) -> Run {
        self.env
            .push((k.as_ref().to_os_string(), v.as_ref().to_os_string()));
        self
    }

    pub fn envs(mut self, kv: &[(String, String)]) -> Run {
        for (k, v) in kv {
            self = self.env(k, v);
        }
        self
    }

    pub fn stdin(mut self, data: impl AsRef<[u8]>) -> Run {
        self.stdin = data.as_ref().to_vec();
        self
    }

    pub fn cwd(mut self, dir: &Path) -> Run {
        self.cwd = Some(dir.to_path_buf());
        self
    }

    fn command(&self) -> Command {
        let prog = self.argv[0].to_str().unwrap_or_default();
        let resolved = which(prog).unwrap_or_else(|| PathBuf::from(&self.argv[0]));
        let mut c = Command::new(resolved);
        c.args(&self.argv[1..]).env_clear();
        for k in ["PATH", "HOME", "LANG", "TMPDIR"] {
            if let Some(v) = std::env::var_os(k) {
                c.env(k, v);
            }
        }
        if std::env::var_os("PATH").is_none() {
            c.env("PATH", "/usr/local/bin:/usr/bin:/bin");
        }
        for (k, v) in &self.env {
            c.env(k, v);
        }
        if let Some(d) = &self.cwd {
            c.current_dir(d);
        }
        c
    }

    /// Starts the program in the background with stdin/stdout/stderr on /dev/null.
    pub fn spawn_quiet(self) -> std::process::Child {
        let mut c = self.command();
        c.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        c.spawn()
            .unwrap_or_else(|e| panic!("cannot start {:?}: {e}", self.argv))
    }

    /// Runs to completion: (exit code, stdout and stderr interleaved). A signal
    /// gives -1.
    pub fn go(self) -> (i32, String) {
        let (code, out) = self.go_bytes();
        (code, String::from_utf8_lossy(&out).into_owned())
    }

    pub fn go_bytes(self) -> (i32, Vec<u8>) {
        let (mut reader, writer) = std::io::pipe().expect("pipe");
        let mut c = self.command();
        c.stdin(Stdio::piped())
            .stdout(writer.try_clone().expect("dup"))
            .stderr(writer);
        let mut child = c
            .spawn()
            .unwrap_or_else(|e| panic!("cannot start {:?}: {e}", self.argv));
        drop(c); // the parent's copies of the pipe's write end
        let mut stdin = child.stdin.take().unwrap();
        let data = self.stdin;
        let feeder = std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
        let mut out = Vec::new();
        reader.read_to_end(&mut out).expect("read output");
        let _ = feeder.join();
        let status = child.wait().expect("wait");
        (status.code().unwrap_or(-1), out)
    }
}

// ---------------------------------------------------------------------------
// Temporary directories
// ---------------------------------------------------------------------------

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A unique directory below `tests/tmp/`, removed (recursively) on drop.
pub struct TmpDir {
    pub path: PathBuf,
}

impl TmpDir {
    pub fn new(label: &str) -> TmpDir {
        let base = repo().join("tests/tmp");
        std::fs::create_dir_all(&base).unwrap();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let path = base.join(format!(
            "pkg-{label}-{}-{}-{nanos:08x}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        TmpDir { path }
    }

    pub fn join(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        if std::env::var_os("WEBPULSE_KEEP_TMP").is_none() {
            rmrf(&self.path);
        }
    }
}

/// `rm -rf` that also enters directories without write/search permission.
pub fn rmrf(p: &Path) {
    let Ok(md) = std::fs::symlink_metadata(p) else {
        return;
    };
    if md.is_dir() {
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                rmrf(&e.path());
            }
        }
        let _ = std::fs::remove_dir(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

/// Relative paths of every file, directory and symlink below `dir` (sorted;
/// symlinked directories are not entered).
pub fn list_tree(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            out.push(s(p.strip_prefix(base).unwrap()));
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(base, &p, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// `cp -a src dst`.
pub fn cp_a(src: &Path, dst: &Path) {
    let (rc, out) = cmd_os(&[
        OsStr::new("cp"),
        OsStr::new("-a"),
        src.as_os_str(),
        dst.as_os_str(),
    ])
    .go();
    assert_eq!(rc, 0, "cp -a {} {}: {out}", src.display(), dst.display());
}

// ---------------------------------------------------------------------------
// Sandbox: fake cPanel root with logging stubs + an unpacked release to install
// ---------------------------------------------------------------------------

/// The directories `install.sh` is redirected to, a fake cPanel root and the
/// logs of the stubs. Removed on drop.
pub struct Sandbox {
    tmp: TmpDir,
    pub base: PathBuf,
    pub prefix: PathBuf,
    pub cpanel: PathBuf,
    pub etc: PathBuf,
    pub var: PathBuf,
    pub cron: PathBuf,
    pub apps: PathBuf,
    /// one line per cPanel tool invocation: "<tool> <args>"
    pub log: PathBuf,
    /// one line per stub binary invocation: "<args> [cfg=$WEBPULSE_CONFIG]"
    pub engine_log: PathBuf,
    pub state: PathBuf,
    pub src: PathBuf,
}

impl Sandbox {
    pub fn new() -> Sandbox {
        let tmp = TmpDir::new("inst");
        let base = tmp.path.clone();
        let p = Sandbox {
            prefix: base.join("prefix"),
            cpanel: base.join("cpanel"),
            etc: base.join("etc-webpulse"),
            var: base.join("var-webpulse"),
            cron: base.join("cron.d"),
            apps: base.join("cpanel-apps"),
            log: base.join("stub.log"),
            engine_log: base.join("engine.log"),
            state: base.join("stub-state"),
            src: base.join("src"),
            base,
            tmp,
        };
        for d in [&p.prefix, &p.cron, &p.apps, &p.state] {
            std::fs::create_dir_all(d).unwrap();
        }
        p.fake_cpanel();
        p
    }

    /// `/usr/local/webpulse` inside the prefix.
    pub fn link(&self) -> PathBuf {
        self.prefix.join("usr/local/webpulse")
    }

    /// `/usr/local/webpulse-releases` inside the prefix.
    pub fn releases(&self) -> PathBuf {
        self.prefix.join("usr/local/webpulse-releases")
    }

    /// The cPanel plugin directory of the Jupiter theme.
    pub fn theme_dir(&self) -> PathBuf {
        self.cpanel.join("base/frontend/jupiter/webpulse")
    }

    fn fake_cpanel(&self) {
        let c = &self.cpanel;
        for d in [
            "scripts",
            "bin",
            "3rdparty/bin",
            "base/frontend/jupiter",
            "whostmgr/docroot/cgi",
            "whostmgr/docroot/addon_plugins",
        ] {
            std::fs::create_dir_all(c.join(d)).unwrap();
        }
        script(&c.join("cpanel"), "#!/bin/sh\n");
        let php = php().unwrap_or_else(|| PathBuf::from("/usr/bin/php"));
        std::os::unix::fs::symlink(php, c.join("3rdparty/bin/php")).unwrap();

        let (log, state, apps) = (s(&self.log), s(&self.state), s(&self.apps));
        let head = format!(
            "#!/bin/bash\nname=$(basename \"$0\")\necho \"$name $*\" >> '{log}'\n\
             if [ -e '{state}/fail.'\"$name\" ]; then echo \"stub $name: simulated failure\" >&2; exit 1; fi\n"
        );
        let stubs: Vec<(&str, String)> = vec![
            (
                "scripts/install_plugin",
                r#"[ -f "$1/install.json" ] || { echo "install.json missing in $1" >&2; exit 2; }
theme=""; while [ $# -gt 0 ]; do [ "$1" = "--theme" ] && theme=$2; shift; done
touch "@STATE@/plugin.$theme"
echo "Plugin installed for $theme""#
                    .to_string(),
            ),
            (
                "scripts/uninstall_plugin",
                r#"theme=""; while [ $# -gt 0 ]; do [ "$1" = "--theme" ] && theme=$2; shift; done
rm -f "@STATE@/plugin.$theme""#
                    .to_string(),
            ),
            ("scripts/restartsrv_cpsrvd", "echo restarted".to_string()),
            (
                "bin/register_appconfig",
                r#"[ -f "$1" ] || { echo "no such conf $1" >&2; exit 2; }
cp "$1" "@APPS@/webpulse.conf"
echo "Registered webpulse. cpsrvd must restart to show the new entry.""#
                    .to_string(),
            ),
            (
                "bin/unregister_appconfig",
                r#"rm -f "@APPS@/$1.conf""#.to_string(),
            ),
            (
                "bin/manage_hooks",
                r#"store="@STATE@/hooks"; touch "$store"
case "$1" in
  list) cat "$store" ;;
  add)
    path=$3; event=""; while [ $# -gt 0 ]; do [ "$1" = "--event" ] && event=$2; shift; done
    [ -x "$path" ] || { echo "hook script not executable: $path" >&2; exit 3; }
    desc=$("$path" --describe) || exit 4
    case "$desc" in *"\"event\":\"$event\""*) ;; *) echo "describe mismatch: $desc" >&2; exit 5 ;; esac
    grep -Fq "$path" "$store" && { echo "hook already exists" >&2; exit 6; }
    echo "$path $event" >> "$store" ;;
  delete)
    path=$3; grep -vF "$path" "$store" > "$store.tmp"; mv "$store.tmp" "$store" ;;
  *) exit 9 ;;
esac"#
                    .to_string(),
            ),
        ];
        for (rel, body) in stubs {
            let body = body.replace("@STATE@", &state).replace("@APPS@", &apps);
            script(&c.join(rel), &format!("{head}{body}\n"));
        }
    }

    /// Environment that redirects `install.sh` into the sandbox.
    pub fn installer_env(&self) -> Vec<(String, String)> {
        vec![
            ("WEBPULSE_PREFIX".into(), s(&self.prefix)),
            ("WEBPULSE_FAKE_CPANEL".into(), "1".into()),
            ("WEBPULSE_CPANEL_ROOT".into(), s(&self.cpanel)),
            ("WEBPULSE_ETC".into(), s(&self.etc)),
            ("WEBPULSE_VAR".into(), s(&self.var)),
            ("WEBPULSE_CRON_DIR".into(), s(&self.cron)),
            ("WEBPULSE_APPS_DIR".into(), s(&self.apps)),
            ("WEBPULSE_SKIP_FIRST_COLLECT".into(), "1".into()),
            // never build for real by accident
            ("WEBPULSE_CARGO".into(), "/nonexistent/cargo".into()),
            ("STUB_ENGINE_LOG".into(), s(&self.engine_log)),
            ("TMPDIR".into(), s(&self.base)),
            ("NO_COLOR".into(), "1".into()),
        ]
    }

    /// `bash <src>/install.sh <args>` inside the sandbox (empty stdin).
    pub fn installer(&self, src: &Path, args: &[&str]) -> Run {
        let mut argv = vec![
            OsString::from("bash"),
            src.join("install.sh").into_os_string(),
        ];
        argv.extend(args.iter().map(OsString::from));
        cmd_os(&argv).envs(&self.installer_env()).stdin("")
    }

    /// The installer the way `curl | bash` runs it (script on stdin, no local source).
    pub fn installer_piped(&self, args: &[&str]) -> Run {
        let mut argv = vec!["bash", "-s", "--"];
        argv.extend_from_slice(args);
        cmd(&argv)
            .envs(&self.installer_env())
            .stdin(std::fs::read(repo().join("install.sh")).unwrap())
    }

    /// Lines of the cPanel stub log.
    pub fn stub_log(&self) -> Vec<String> {
        read(&self.log)
            .lines()
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Files and directories of WebPulse left in prefix/cron/apps/cpanel.
    pub fn leftovers(&self) -> Vec<String> {
        let mut found = Vec::new();
        for root in [&self.prefix, &self.cron, &self.apps, &self.cpanel] {
            for rel in list_tree(root) {
                if rel.contains("webpulse") && !rel.starts_with("3rdparty") {
                    found.push(format!("{}/{rel}", root.display()));
                }
            }
        }
        found
    }

    /// A unique directory inside the sandbox.
    pub fn dir(&self, name: &str) -> PathBuf {
        let d = self.base.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}

/// Number of log lines that start with `prefix`.
pub fn count(log: &[String], prefix: &str) -> usize {
    log.iter().filter(|l| l.starts_with(prefix)).count()
}

/// Body of the stub WebPulse binary: logs its arguments, fakes `setup`.
pub fn stub_binary_body(marker: &str) -> String {
    r#"#!/bin/bash
# stub binary used by the packaging tests (@MARKER@)
if [ -n "$STUB_ENGINE_LOG" ]; then
    echo "$* [cfg=$WEBPULSE_CONFIG]" >> "$STUB_ENGINE_LOG"
fi
case "$1" in
    --version) echo "webpulse 0.0.0-@MARKER@"; exit 0 ;;
    setup)
        if [ -n "$STUB_SETUP_FAIL" ]; then echo "setup exploded" >&2; exit 7; fi
        [ -f "$WEBPULSE_CONFIG" ] || { echo "stub engine: no config" >&2; exit 1; }
        for k in data_dir log_dir run_dir; do
            d=$(sed -n "s/.*\"$k\": *\"\([^\"]*\)\".*/\1/p" "$WEBPULSE_CONFIG" | head -n 1)
            if [ -n "$d" ]; then mkdir -p "$d"; fi
        done
        ;;
esac
exit 0

"#
    .replace("@MARKER@", marker)
}

/// A self-contained unpacked release: the real packaging files plus a stub binary
/// that records its invocations (`bin/webpulse`) and a stub default config (so the
/// test does not depend on the binary). This is the layout
/// `scripts/build_release.sh` produces. `dir` defaults to `p.src`.
pub fn make_source(p: &Sandbox, version: &str, dir: Option<&Path>, marker: &str) -> PathBuf {
    let src = dir.map(Path::to_path_buf).unwrap_or_else(|| p.src.clone());
    std::fs::create_dir_all(&src).unwrap();
    let r = repo();
    for item in [
        "cpanel",
        "whm",
        "hooks",
        "installer",
        "install.sh",
        "uninstall.sh",
    ] {
        cp_a(&r.join(item), &src.join(item));
    }
    // etc/: our cron file + a stub default config
    write(
        &src.join("etc/cron.d/webpulse"),
        std::fs::read(r.join("etc/cron.d/webpulse")).unwrap(),
    );
    let cfg = serde_json::json!({
        "version": 1,
        "exclude_paths": ["/.well-known/", "/cpanel"],
        "geoip": {"enabled": false, "country_db": "/var/webpulse/geoip/dbip-country-lite.mmdb", "city_db": null},
        "php_bin": null,
        "paths": {
            "data_dir": "/var/webpulse/data",
            "registry_db": "/var/webpulse/webpulse.db",
            "log_dir": "/var/webpulse/log",
            "run_dir": "/var/webpulse/run",
            "domlogs_dirs": ["/etc/apache2/logs/domlogs/{user}", "/var/log/nginx/domains"],
            "archive_dir": "/home/{user}/logs",
            "userdomains_file": "/etc/userdomains",
            "cpanel_users_dir": "/var/cpanel/users",
            "home_root": "/home",
            "cron_file": "/etc/cron.d/webpulse",
            "hooks_file": "/var/cpanel/hooks/data/hooks.json"
        }
    });
    write(
        &src.join("etc/config.json.default"),
        serde_json::to_string_pretty(&cfg).unwrap() + "\n",
    );
    script(&src.join("bin/webpulse"), &stub_binary_body(marker));
    for f in ["LICENSE", "README.md"] {
        write(&src.join(f), std::fs::read(r.join(f)).unwrap());
    }
    write(&src.join("VERSION"), format!("{version}\n"));
    src
}

/// Packs an unpacked release like `scripts/build_release.sh` does: one top-level
/// directory `webpulse-<ver>-<target>/`, plus a `.sha256` next to the tarball.
/// Returns (tarball, checksum file).
pub fn release_tarball(src: &Path, out_dir: &Path, target: &str) -> (PathBuf, PathBuf) {
    let ver = read(&src.join("VERSION")).trim().to_string();
    let name = format!("webpulse-{ver}-{target}");
    std::fs::create_dir_all(out_dir).unwrap();
    let tarball = out_dir.join(format!("{name}.tar.gz"));
    let (rc, out) = cmd(&[
        "tar",
        "-czf",
        &s(&tarball),
        "--transform",
        &format!("s,^\\.,{name},"),
        "-C",
        &s(src),
        ".",
    ])
    .go();
    assert_eq!(rc, 0, "tar: {out}");
    let sha = PathBuf::from(format!("{}.sha256", tarball.display()));
    write(&sha, format!("{}  {name}.tar.gz\n", sha256_file(&tarball)));
    (tarball, sha)
}
