//! Front-end shims: the cPanel LiveAPI pages (index.live.php, api.live.php, run on
//! cPanel's PHP) and the WHM bash CGIs. All logic lives in the binary; these tests
//! use a stub binary (WEBPULSE_BIN) that records its arguments, environment and
//! stdin and prints a CGI response, and check LiveApiUser, CgiBridge and the exec
//! arguments. The PHP shims run under the `php` CLI with a stub `CPANEL` class
//! (`-d auto_prepend_file`); those tests are skipped when `php` is not installed.

mod pkg_common;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use pkg_common::*;
use serde_json::{Value, json};

const DEFAULT_BIN: &str = "/usr/local/webpulse/bin/webpulse";

fn real_install_exists() -> bool {
    is_executable(Path::new(DEFAULT_BIN))
}

/// A stub WebPulse binary: records args/env/stdin in `call.*` and prints
/// `response` (a complete CGI response) verbatim, then exits `exit`.
struct Stub {
    _tmp: TmpDir,
    bin: PathBuf,
    call: PathBuf,
}

impl Stub {
    fn new(response: &[u8], exit: i32) -> Stub {
        let tmp = TmpDir::new("shimbin");
        let bin = tmp.join("webpulse");
        let resp = tmp.join("response.txt");
        let call = tmp.join("call");
        write(&resp, response);
        let body = r#"#!/bin/bash
log='@CALL@'
echo "$*" > "$log.args"
env | grep -E '^(REQUEST_METHOD|QUERY_STRING|CONTENT_TYPE|CONTENT_LENGTH|HTTP_X_WEBPULSE_REQUEST|REMOTE_ADDR|REMOTE_USER|WEBPULSE_CONFIG|STUB_CPANEL_USER)=' | sort > "$log.env"
cat > "$log.stdin"
cat '@RESP@'
exit @EXIT@
"#
        .replace("@CALL@", &s(&call))
        .replace("@RESP@", &s(&resp))
        .replace("@EXIT@", &exit.to_string());
        script(&bin, &body);
        Stub {
            _tmp: tmp,
            bin,
            call,
        }
    }

    fn path(&self) -> String {
        s(&self.bin)
    }

    fn recorded(&self, ext: &str) -> String {
        read(&PathBuf::from(format!("{}.{ext}", self.call.display())))
    }
}

/// PHP prepend/append files: a stub CPANEL class, and a dump of the headers the
/// shim set.
struct Harness {
    _tmp: TmpDir,
    prepend: PathBuf,
    append: PathBuf,
    headers_out: PathBuf,
}

fn harness() -> Harness {
    let tmp = TmpDir::new("phpharness");
    let prepend = tmp.join("prepend.php");
    write(
        &prepend,
        r#"<?php
// stands in for /usr/local/cpanel/php/cpanel.php
class CPANEL {
    public function uapi($m, $f, $a = []) { return ['cpanelresult' => ['result' => ['data' => ['user' => getenv('STUB_CPANEL_USER') ?: 'alice']]]]; }
    public function header($title) { return "<!--HEADER:$title-->"; }
    public function footer() { return '<!--FOOTER-->'; }
    public function end() { file_put_contents(getenv('STUB_CPANEL_END') ?: '/dev/null', "end\n", FILE_APPEND); }
}
"#,
    );
    let append = tmp.join("append.php");
    write(
        &append,
        r#"<?php
file_put_contents(getenv('STUB_HEADERS_OUT') ?: '/dev/null', json_encode(['status' => http_response_code(), 'headers' => headers_list()]));
"#,
    );
    let headers_out = tmp.join("headers.json");
    Harness {
        _tmp: tmp,
        prepend,
        append,
        headers_out,
    }
}

/// Runs a LiveAPI shim under php-cli with the stub CPANEL class: (rc, output,
/// headers dump).
fn run_live(
    php: &Path,
    shim: &str,
    env: Vec<(String, String)>,
    stdin: &[u8],
) -> (i32, String, Value) {
    let h = harness();
    let (rc, out) = cmd(&[
        &s(php),
        "-d",
        &format!("auto_prepend_file={}", h.prepend.display()),
        "-d",
        &format!("auto_append_file={}", h.append.display()),
        "-d",
        "error_log=/dev/null",
        "-d",
        "display_errors=0",
        &s(&repo().join("cpanel/webpulse").join(shim)),
    ])
    .envs(&env)
    .env("STUB_HEADERS_OUT", &h.headers_out)
    .stdin(stdin)
    .go();
    let dump = serde_json::from_str(&read(&h.headers_out)).unwrap_or(json!({}));
    (rc, out, dump)
}

/// Runs PHP code with LiveApiUser and CgiBridge loaded; the code prints JSON,
/// which is returned.
fn php_unit(php: &Path, code: &str, env: Vec<(String, String)>) -> Value {
    let tmp = TmpDir::new("phpunit");
    let lib = repo().join("cpanel/webpulse/lib");
    let file = tmp.join("unit.php");
    write(
        &file,
        format!(
            "<?php\ndeclare(strict_types=1);\nrequire '{}';\nrequire '{}';\n\
             use WebPulse\\Cpanel\\CgiBridge;\nuse WebPulse\\Cpanel\\LiveApiUser;\n{code}\n",
            lib.join("LiveApiUser.php").display(),
            lib.join("CgiBridge.php").display()
        ),
    );
    let (rc, out) = cmd(&[
        &s(php),
        "-d",
        "display_errors=stderr",
        "-d",
        "error_reporting=-1",
        &s(&file),
    ])
    .envs(&env)
    .go();
    assert_eq!(rc, 0, "php failed:\n{out}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: not JSON:\n{out}"))
}

/// Kills the PHP built-in server when the test ends.
struct Server(std::process::Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Serves cpanel/webpulse with PHP's built-in web server (a real SAPI where
/// header() works) and sends one HTTP/1.0 request: (status, headers with
/// lower-case names, body).
fn http_live(
    php: &Path,
    path: &str,
    env: Vec<(String, String)>,
    method: &str,
    body: &[u8],
    req_headers: &[(&str, &str)],
) -> (u16, std::collections::BTreeMap<String, String>, String) {
    let h = harness();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let _server = Server(
        cmd(&[
            &s(php),
            "-d",
            &format!("auto_prepend_file={}", h.prepend.display()),
            "-d",
            "error_log=/dev/null",
            "-d",
            "display_errors=0",
            "-S",
            &format!("127.0.0.1:{port}"),
            "-t",
            &s(&repo().join("cpanel/webpulse")),
        ])
        .envs(&env)
        .spawn_quiet(),
    );
    let mut sock = None;
    for _ in 0..100 {
        if let Ok(c) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            sock = Some(c);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let mut sock = sock.expect("the PHP built-in server did not start");
    sock.set_read_timeout(Some(std::time::Duration::from_secs(20)))
        .unwrap();
    let mut req =
        format!("{method} /{path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    for (k, v) in req_headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty() {
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    sock.write_all(req.as_bytes()).unwrap();
    sock.write_all(body).unwrap();
    let mut resp = Vec::new();
    sock.read_to_end(&mut resp).unwrap();
    let resp = String::from_utf8_lossy(&resp).into_owned();
    let (head, body) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
        .collect();
    (status, headers, body.to_string())
}

macro_rules! need_php {
    () => {
        match php() {
            Some(p) => p,
            None => skip!("php not installed"),
        }
    };
}

// ---------------------------------------------------------------------------
// LiveApiUser and CgiBridge (the PHP helpers of the shims)
// ---------------------------------------------------------------------------

#[test]
fn live_api_user_from_result_accepts_every_uapi_shape() {
    let php = need_php!();
    let v = php_unit(
        &php,
        r#"
$arr = ['cpanelresult' => ['result' => ['data' => ['user' => 'alice']]]];
$good = [
    LiveApiUser::fromResult($arr),                                                                       // array shape
    LiveApiUser::fromResult(json_decode(json_encode($arr))),                                             // object shape
    LiveApiUser::fromResult((object) ['result' => (object) ['data' => (object) ['user' => 'bob_1']]]),   // objects all the way
    LiveApiUser::fromResult(['result' => ['data' => ['user' => 'alice']]]),                              // without cpanelresult
    LiveApiUser::fromResult(['data' => ['user' => 'alice']]),                                            // bare data
    LiveApiUser::fromResult(['cpanelresult' => ['data' => ['user' => 'alice']]]),                        // data under cpanelresult
    LiveApiUser::fromResult(['cpanelresult' => ['result' => ['data' => [['user' => 'alice']]]]]),        // list of records
    LiveApiUser::fromResult(['cpanelresult' => ['result' => ['data' => 'alice']]]),                      // plain string
];
$bad = [];
foreach ([null, 'alice', 42, [], ['data' => []], ['data' => ['user' => '../x']], ['data' => ['user' => 'ROOT']],
    ['data' => ['user' => ['a']]], ['data' => ['user' => str_repeat('a', 33)]], ['data' => ['user' => "alice\n"]]] as $b) {
    $bad[] = LiveApiUser::fromResult($b);
}
echo json_encode(['good' => $good, 'bad' => $bad]);
"#,
        vec![],
    );
    assert_eq!(
        v["good"],
        json!([
            "alice", "alice", "bob_1", "alice", "alice", "alice", "alice", "alice"
        ]),
        "accepted shapes"
    );
    assert_eq!(v["bad"], json!(vec![Value::Null; 10]), "rejected shapes");
}

#[test]
fn live_api_user_validates_the_cpanel_user_grammar() {
    let php = need_php!();
    let v = php_unit(
        &php,
        r#"
$ok = ['alice', 'a', '_x', '0day', 'a-b_c', str_repeat('a', 32)];
$bad = ['', 'Alice', '-a', 'a b', 'a/b', '../x', 'a.b', "a\n", str_repeat('a', 33), 'é'];
echo json_encode(['ok' => array_map([LiveApiUser::class, 'valid'], $ok), 'bad' => array_map([LiveApiUser::class, 'valid'], $bad)]);
"#,
        vec![],
    );
    assert_eq!(v["ok"], json!(vec![true; 6]), "valid names");
    assert_eq!(v["bad"], json!(vec![false; 10]), "invalid names");
}

#[test]
fn live_api_user_resolve_cascade_uapi_then_remote_user_then_posix() {
    let php = need_php!();
    let v = php_unit(
        &php,
        r#"
$fake = static function (mixed $result, bool $throw = false): object {
    return new class ($result, $throw) {
        public function __construct(private mixed $result, private bool $throw) {}
        public function uapi(string $module, string $function, array $args = []): mixed
        {
            if ($this->throw) { throw new \RuntimeException('uapi down'); }
            if ($module !== 'Variables' || $function !== 'get_user_information' || $args !== ['name' => 'user']) {
                throw new \LogicException('unexpected uapi call');
            }
            return $this->result;
        }
    };
};
$ok = ['cpanelresult' => ['result' => ['data' => ['user' => 'fromuapi']]]];
putenv('REMOTE_USER=fromenv');
$cascade = [
    LiveApiUser::resolve($fake($ok)),          // 1. uapi wins over everything
    LiveApiUser::resolve($fake(['data' => []])), // 2. uapi garbage / exception / missing -> REMOTE_USER
    LiveApiUser::resolve($fake(null, true)),
    LiveApiUser::resolve(null),
    LiveApiUser::resolve(new \stdClass()),
];
// 3. invalid / absent REMOTE_USER -> posix euid name
$fallback = [];
foreach ([null, '', '../etc', 'Bad User'] as $env) {
    $env === null ? putenv('REMOTE_USER') : putenv('REMOTE_USER=' . $env);
    try { $fallback[] = LiveApiUser::resolve(null); } catch (\RuntimeException) { $fallback[] = null; }
}
echo json_encode(['cascade' => $cascade, 'fallback' => $fallback]);
"#,
        vec![],
    );
    assert_eq!(
        v["cascade"],
        json!(["fromuapi", "fromenv", "fromenv", "fromenv", "fromenv"]),
        "uapi first, then REMOTE_USER"
    );
    // the posix name of the effective user, when it is a valid cPanel user name
    let (_, me) = cmd(&["id", "-un"]).go();
    let me = me.trim();
    let valid = regex::Regex::new(r"\A[a-z0-9_][a-z0-9_-]{0,31}\z").unwrap();
    let expected = if valid.is_match(me) {
        json!(me)
    } else {
        Value::Null
    };
    assert_eq!(
        v["fallback"],
        json!([
            expected.clone(),
            expected.clone(),
            expected.clone(),
            expected
        ]),
        "posix fallback"
    );
}

#[test]
fn live_api_user_resolve_throws_when_nothing_valid_is_found() {
    let php = need_php!();
    // a subprocess without posix and without REMOTE_USER
    let code = r#"require $argv[1]; try { WebPulse\Cpanel\LiveApiUser::resolve(null); echo "no exception"; } catch (RuntimeException $e) { echo "threw: ", $e->getMessage(); }"#;
    let (rc, out) = cmd(&[
        &s(&php),
        "-d",
        "disable_functions=posix_getpwuid,posix_geteuid",
        "-r",
        code,
        "--",
        &s(&repo().join("cpanel/webpulse/lib/LiveApiUser.php")),
    ])
    .env("REMOTE_USER", "")
    .go();
    assert_eq!(rc, 0, "{out}");
    assert_contains(
        &out,
        "threw: Unable to determine the cPanel account name",
        "resolve",
    );
}

#[test]
fn cgi_bridge_parse_head_keeps_plain_headers_and_drops_anything_suspicious() {
    let php = need_php!();
    let v = php_unit(
        &php,
        r#"
echo json_encode([
    CgiBridge::parseHead("Status: 404 Not Found\r\nContent-Type: application/json\r\nX-A: b\r\nbad line\r\nEvil\x01: x\r\nSet-Cookie: a=b\r\n"),
    CgiBridge::parseHead("Content-Type: text/html\n"),
    CgiBridge::parseHead('Status: 503'),
]);
"#,
        vec![],
    );
    assert_eq!(
        v[0],
        json!([
            404,
            [
                "Content-Type: application/json",
                "X-A: b",
                "Set-Cookie: a=b"
            ]
        ])
    );
    assert_eq!(
        v[1],
        json!([200, ["Content-Type: text/html"]]),
        "default status"
    );
    assert_eq!(v[2][0], json!(503));
}

#[test]
fn cgi_bridge_resolves_webpulse_bin_then_the_default_path_then_bin_next_to_the_shim() {
    let php = need_php!();
    let dir = TmpDir::new("binres");
    let env_bin = script(&dir.join("env-bin"), "#!/bin/sh\n");
    let near = script(&dir.join("shim/bin/webpulse"), "#!/bin/sh\n");
    write(&dir.join("not-executable"), "x");
    let v = php_unit(
        &php,
        r#"
$dir = getenv('T_DIR');
putenv("WEBPULSE_BIN=$dir/env-bin");
$a = CgiBridge::binary("$dir/shim");
putenv("WEBPULSE_BIN=$dir/not-executable");
$b = CgiBridge::binary("$dir/shim");
putenv('WEBPULSE_BIN');
$c = CgiBridge::binary("$dir/shim");
$d = CgiBridge::binary("$dir/nowhere");
echo json_encode([$a, $b, $c, $d, CgiBridge::DEFAULT_BIN]);
"#,
        kv!["T_DIR" => s(&dir.path)],
    );
    assert_eq!(v[4], json!(DEFAULT_BIN), "default path");
    assert_eq!(v[0], json!(s(&env_bin)), "env wins");
    assert!(
        v[1] == json!(s(&near)) || v[1] == json!(DEFAULT_BIN),
        "unusable env value ignored: {}",
        v[1]
    );
    if !real_install_exists() {
        assert_eq!(v[2], json!(s(&near)), "next to the shim");
        assert_eq!(v[3], Value::Null, "nothing found");
    }
}

#[test]
fn cgi_bridge_forwards_only_the_cgi_variables_and_webpulse_config() {
    let php = need_php!();
    let env = php_unit(
        &php,
        "echo json_encode(CgiBridge::environment('alice'));",
        kv![
            "REQUEST_METHOD" => "POST",
            "QUERY_STRING" => "a=x",
            "CONTENT_TYPE" => "application/json",
            "CONTENT_LENGTH" => "3",
            "HTTP_X_WEBPULSE_REQUEST" => "1",
            "REMOTE_ADDR" => "203.0.113.9",
            "WEBPULSE_CONFIG" => "/etc/other.json",
            "SECRET_TOKEN" => "nope",
        ],
    );
    assert_eq!(env["REQUEST_METHOD"], json!("POST"));
    assert_eq!(
        env["REMOTE_USER"],
        json!("alice"),
        "the resolved account, not a request value"
    );
    assert_eq!(env["WEBPULSE_CONFIG"], json!("/etc/other.json"));
    assert_eq!(env["REMOTE_ADDR"], json!("203.0.113.9"));
    assert!(
        env.get("SECRET_TOKEN").is_none(),
        "unrelated variables are not forwarded"
    );
    assert!(env.get("PATH").is_some(), "PATH is set");
}

// ---------------------------------------------------------------------------
// api.live.php / index.live.php
// ---------------------------------------------------------------------------

#[test]
fn api_live_php_passes_status_headers_and_body_through_and_pipes_the_request_body() {
    let php = need_php!();
    let stub = Stub::new(
        b"Status: 201 Created\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\nX-Test: yes\r\n\r\n{\"ok\":true,\"data\":{\"hello\":\"world\"}}",
        0,
    );
    // a real HTTP request to PHP's built-in server: CGI variables come from $_SERVER,
    // the body from php://input
    let (status, headers, body) = http_live(
        &php,
        "api.live.php?a=save_user_settings&user=bob",
        kv![
            "WEBPULSE_BIN" => stub.path(),
            "STUB_CPANEL_USER" => "alice",
            "WEBPULSE_CONFIG" => "/some/config.json",
            "REMOTE_USER" => "mallory",
        ],
        "POST",
        br#"{"theme":"dark"}"#,
        &[
            ("Content-Type", "application/json"),
            ("X-WebPulse-Request", "1"),
        ],
    );
    assert_eq!(status, 201, "status line becomes the HTTP status");
    assert_eq!(
        body, r#"{"ok":true,"data":{"hello":"world"}}"#,
        "body passthrough, nothing else printed"
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json; charset=utf-8"),
        "Content-Type"
    );
    assert_eq!(
        headers.get("cache-control").map(String::as_str),
        Some("no-store"),
        "Cache-Control"
    );
    assert_eq!(
        headers.get("x-test").map(String::as_str),
        Some("yes"),
        "custom header"
    );
    assert!(
        !headers.contains_key("status"),
        "the Status line is not echoed as a header"
    );

    assert_eq!(
        stub.recorded("args"),
        "cgi --principal user:alice --api\n",
        "principal from the cPanel session, never from the request"
    );
    let seen = stub.recorded("env");
    for line in [
        "REQUEST_METHOD=POST",
        "QUERY_STRING=a=save_user_settings&user=bob",
        "CONTENT_TYPE=application/json",
        "HTTP_X_WEBPULSE_REQUEST=1",
        "REMOTE_ADDR=127.0.0.1",
        "REMOTE_USER=alice",
        "WEBPULSE_CONFIG=/some/config.json",
        "CONTENT_LENGTH=16",
    ] {
        assert_contains(&seen, &format!("{line}\n"), "forwarded variable");
    }
    assert_eq!(
        stub.recorded("stdin"),
        r#"{"theme":"dark"}"#,
        "request body piped to the binary"
    );
}

#[test]
fn api_live_php_cli_falls_back_to_stdin_for_the_body_and_forwards_the_environment() {
    let php = need_php!();
    let stub = Stub::new(b"Content-Type: application/json\r\n\r\n{\"ok\":true}", 0);
    let (rc, out, _) = run_live(
        &php,
        "api.live.php",
        kv![
            "WEBPULSE_BIN" => stub.path(),
            "REQUEST_METHOD" => "POST",
            "QUERY_STRING" => "a=x",
            "CONTENT_TYPE" => "application/json",
            "CONTENT_LENGTH" => "16",
            "HTTP_X_WEBPULSE_REQUEST" => "1",
            "REMOTE_ADDR" => "203.0.113.9",
            "REMOTE_USER" => "mallory",
            "STUB_CPANEL_USER" => "alice",
        ],
        br#"{"theme":"dark"}"#,
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, r#"{"ok":true}"#);
    assert_eq!(stub.recorded("stdin"), r#"{"theme":"dark"}"#);
    assert_contains(&stub.recorded("env"), "REMOTE_ADDR=203.0.113.9\n", "env");
    assert_contains(&stub.recorded("env"), "REMOTE_USER=alice\n", "env");
}

#[test]
fn api_live_php_get_sends_no_body_and_a_4xx_from_the_binary_reaches_the_client() {
    let php = need_php!();
    let stub = Stub::new(
        b"Status: 403 Forbidden\r\nContent-Type: application/json\r\n\r\n{\"ok\":false}",
        0,
    );
    let (rc, out, dump) = run_live(
        &php,
        "api.live.php",
        kv!["WEBPULSE_BIN" => stub.path(), "REQUEST_METHOD" => "GET", "QUERY_STRING" => "a=domains"],
        b"",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, r#"{"ok":false}"#);
    assert_eq!(dump["status"], json!(403));
    assert_eq!(stub.recorded("stdin"), "", "no body for GET");
    assert_contains(&stub.recorded("env"), "QUERY_STRING=a=domains", "env");
}

#[test]
fn api_live_php_streams_a_large_body_unchanged() {
    let php = need_php!();
    let big = "0123456789abcdef".repeat(40000); // 640 KB
    let stub = Stub::new(format!("Content-Type: text/csv\r\n\r\n{big}").as_bytes(), 0);
    let (rc, out, dump) = run_live(
        &php,
        "api.live.php",
        kv!["WEBPULSE_BIN" => stub.path(), "REQUEST_METHOD" => "GET", "QUERY_STRING" => "a=export"],
        b"",
    );
    assert_eq!(rc, 0);
    assert_eq!(out.len(), big.len(), "length");
    assert!(out == big, "content");
    assert_eq!(dump["status"], json!(200), "default status");
}

#[test]
fn api_live_php_answers_a_generic_json_500_when_the_binary_is_missing_broken_or_silent() {
    let php = need_php!();
    let secret = TmpDir::new("secret");
    let config = s(&secret.join("missing-config.json"));
    let crash = Stub::new(b"", 1);
    let garbage = Stub::new(b"this is not a cgi response", 0);
    let cases = [
        ("missing binary", "/nonexistent/dir/webpulse".to_string()),
        ("crash without output", crash.path()),
        ("garbage without header terminator", garbage.path()),
    ];
    for (label, bin) in cases {
        if label == "missing binary" && real_install_exists() {
            continue;
        }
        let (rc, out, dump) = run_live(
            &php,
            "api.live.php",
            kv![
                "WEBPULSE_BIN" => bin,
                "WEBPULSE_CONFIG" => config,
                "REQUEST_METHOD" => "GET",
                "QUERY_STRING" => "a=domains",
            ],
            b"",
        );
        assert_eq!(rc, 0, "{label}: {out}");
        let v: Value = serde_json::from_str(&out)
            .unwrap_or_else(|_| panic!("{label}: JSON body expected, got: {out}"));
        assert_eq!(v["ok"], json!(false), "{label}: ok=false");
        assert_eq!(v["error"]["code"], json!("internal_error"), "{label}");
        assert_eq!(dump["status"], json!(500), "{label}: HTTP 500");
        assert_not_contains(&out, "missing-config", &format!("{label}: no path leak"));
        assert_not_contains(&out, &bin, &format!("{label}: no path leak"));
        assert_not_contains(&out, &s(&repo()), &format!("{label}: no path leak"));
    }
}

#[test]
fn index_live_php_wraps_the_embedded_dashboard_in_the_cpanel_chrome() {
    let php = need_php!();
    let stub = Stub::new(
        b"Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\r\n<div id=\"webpulse-app\" data-api=\"api.live.php\">dashboard</div>",
        0,
    );
    let end = TmpDir::new("end");
    let end_log = end.join("end.log");
    let (rc, out, dump) = run_live(
        &php,
        "index.live.php",
        kv![
            "WEBPULSE_BIN" => stub.path(),
            "REMOTE_USER" => "mallory",
            "STUB_CPANEL_USER" => "alice",
            "STUB_CPANEL_END" => s(&end_log),
        ],
        b"",
    );
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        out,
        r#"<!--HEADER:WebPulse Analytics--><div id="webpulse-app" data-api="api.live.php">dashboard</div><!--FOOTER-->"#,
        "chrome + fragment, no CGI header text in the page"
    );
    assert_eq!(
        stub.recorded("args"),
        "cgi --principal user:alice --page --embedded --api-url api.live.php\n"
    );
    assert_eq!(read(&end_log), "end\n", "end() called once");
    let headers: Vec<String> = dump["headers"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|h| h.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert_not_contains(
        &headers.join("\n"),
        "Content-Type: text/html",
        "headers of the binary are not applied after the chrome started",
    );
}

#[test]
fn index_live_php_shows_a_friendly_message_without_paths_when_the_binary_fails() {
    let php = need_php!();
    let crash = Stub::new(b"", 1);
    for bin in ["/nonexistent/dir/webpulse".to_string(), crash.path()] {
        if bin.starts_with("/nonexistent") && real_install_exists() {
            continue;
        }
        let (rc, out, _) = run_live(
            &php,
            "index.live.php",
            kv!["WEBPULSE_BIN" => bin, "WEBPULSE_CONFIG" => "/nonexistent/config.json"],
            b"",
        );
        assert_eq!(rc, 0, "{out}");
        assert!(
            out.starts_with("<!--HEADER:WebPulse Analytics-->"),
            "chrome header: {out}"
        );
        assert_contains(&out, "not available right now", "message");
        assert!(out.ends_with("<!--FOOTER-->"), "chrome footer: {out}");
        assert_not_contains(&out, "/nonexistent", "no path leak");
        assert_not_contains(&out, "Stack trace", "no stack trace");
    }
}

#[test]
fn the_shims_find_a_binary_next_to_them() {
    let php = need_php!();
    if real_install_exists() {
        skip!("a real installation exists on this machine");
    }
    let dir = TmpDir::new("nextto");
    cp_a(&repo().join("cpanel/webpulse/."), &dir.path);
    let stub = Stub::new(b"Content-Type: application/json\r\n\r\n{\"ok\":true}", 0);
    // the stub logs to absolute paths, so a copy records into the same files
    script(&dir.join("bin/webpulse"), &read(&stub.bin));
    let prepend = dir.join("prepend.php");
    write(
        &prepend,
        "<?php class CPANEL { public function uapi($m,$f,$a=[]) { return ['data'=>['user'=>'alice']]; } public function end() {} }\n",
    );
    let (rc, out) = cmd(&[
        &s(&php),
        "-d",
        &format!("auto_prepend_file={}", prepend.display()),
        &s(&dir.join("api.live.php")),
    ])
    .env("REQUEST_METHOD", "GET")
    .env("QUERY_STRING", "a=domains")
    .go();
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out, r#"{"ok":true}"#);
    assert_eq!(stub.recorded("args"), "cgi --principal user:alice --api\n");
}

// ---------------------------------------------------------------------------
// WHM CGIs: index.cgi (Perl, WHM master template) and api.cgi (bash)
// ---------------------------------------------------------------------------

/// The interpreter of a WHM CGI here: index.cgi is Perl (cPanel's perl on a server,
/// the system perl in tests), api.cgi is bash.
fn interp(cgi: &str) -> &'static str {
    if cgi == "index.cgi" { "perl" } else { "bash" }
}

/// Stand-ins for cPanel's Perl modules: `Whostmgr::ACLS::init_acls()` and
/// `Cpanel::Template::process_template()` record their calls in `$STUB_OUT.*`; the
/// template prints the body between chrome markers.
struct PerlStubs {
    tmp: TmpDir,
    out: PathBuf,
}

impl PerlStubs {
    fn new() -> PerlStubs {
        let tmp = TmpDir::new("perlstubs");
        let out = tmp.join("rec");
        write(
            &tmp.join("lib/Whostmgr/ACLS.pm"),
            r#"package Whostmgr::ACLS;
use strict;
sub init_acls { open(my $f, '>>', "$ENV{STUB_OUT}.acls") or die; print $f "init\n"; close $f; return 1 }
sub hasroot { return 1 }
1;
"#,
        );
        write(
            &tmp.join("lib/Cpanel/Template.pm"),
            r#"package Cpanel::Template;
use strict;
sub process_template {
    my ($service, $args) = @_;
    open(my $f, '>', "$ENV{STUB_OUT}.tmpl") or die;
    binmode $f;
    print $f "service=$service\n";
    print $f "template_file=$args->{template_file}\n";
    print $f "print=$args->{print}\n";
    print $f "body=$args->{data}{body}";
    close $f;
    die "simulated template failure\n" if $ENV{STUB_TEMPLATE_DIE};
    print "<!--WHM-CHROME-->" . $args->{data}{body} . "<!--/WHM-CHROME-->" if $args->{print};
    return 1;
}
1;
"#,
        );
        PerlStubs { tmp, out }
    }
    fn lib(&self) -> String {
        format!("-I{}", s(&self.tmp.join("lib")))
    }
    fn recorded(&self, ext: &str) -> Option<String> {
        std::fs::read_to_string(format!("{}.{ext}", self.out.display())).ok()
    }
}

const FRAGMENT: &[u8] = b"Status: 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\r\n<div id=\"webpulse-app\" class=\"wp-root\" data-embedded=\"1\"></div>\n<script nonce=\"n\">x</script>\n";

#[test]
fn whm_index_cgi_renders_the_fragment_inside_the_whm_master_template() {
    if !have("perl") {
        skip!("perl not installed");
    }
    let stub = Stub::new(FRAGMENT, 0);
    let perl = PerlStubs::new();
    let (rc, out) = cmd(&["perl", &perl.lib(), &s(&repo().join("whm/cgi/index.cgi"))])
        .envs(&kv![
            "WEBPULSE_BIN" => stub.path(),
            "STUB_OUT" => s(&perl.out),
            "REMOTE_USER" => "reseller1",
            "REQUEST_METHOD" => "GET",
            "QUERY_STRING" => "",
            "REMOTE_ADDR" => "198.51.100.7",
            "WEBPULSE_CONFIG" => "/some/config.json",
        ])
        .stdin("this must not reach the binary")
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        stub.recorded("args"),
        "cgi --whm --page --embedded --api-url api.cgi\n",
        "embedded fragment requested"
    );
    let body = "<div id=\"webpulse-app\" class=\"wp-root\" data-embedded=\"1\"></div>\n<script nonce=\"n\">x</script>\n";
    assert_eq!(
        out,
        format!("Content-type: text/html\r\n\r\n<!--WHM-CHROME-->{body}<!--/WHM-CHROME-->"),
        "WHM's header line, then the master template around the fragment (no binary headers)"
    );
    assert_eq!(
        perl.recorded("tmpl").unwrap(),
        format!("service=whostmgr\ntemplate_file=webpulse/index.tmpl\nprint=1\nbody={body}"),
        "process_template('whostmgr', {{ template_file => 'webpulse/index.tmpl', print => 1, data => {{ body }} }})"
    );
    assert_eq!(
        perl.recorded("acls").as_deref(),
        Some("init\n"),
        "init_acls() called"
    );
    let seen = stub.recorded("env");
    for line in [
        "REMOTE_USER=reseller1",
        "REQUEST_METHOD=GET",
        "REMOTE_ADDR=198.51.100.7",
        "WEBPULSE_CONFIG=/some/config.json",
    ] {
        assert_contains(
            &seen,
            &format!("{line}\n"),
            &format!("index.cgi forwards {line}"),
        );
    }
    assert_eq!(stub.recorded("stdin"), "", "stdin of the binary is empty");
}

#[test]
fn whm_index_cgi_passes_errors_of_the_binary_through_untouched() {
    if !have("perl") {
        skip!("perl not installed");
    }
    let perl = PerlStubs::new();
    for response in [
        &b"Status: 403 Forbidden\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\r\n<!doctype html><h2>Access denied</h2>"[..],
        &b"Status: 500 Internal Server Error\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<h2>broken</h2>"[..],
    ] {
        let stub = Stub::new(response, 0);
        let (rc, out) = cmd(&["perl", &perl.lib(), &s(&repo().join("whm/cgi/index.cgi"))])
            .envs(&kv![
                "WEBPULSE_BIN" => stub.path(),
                "STUB_OUT" => s(&perl.out),
                "REMOTE_USER" => "nobody-here",
            ])
            .go();
        assert_eq!(rc, 0, "{out}");
        assert_eq!(out.as_bytes(), response, "error page unchanged");
        assert!(perl.recorded("tmpl").is_none(), "no template for an error");
    }
}

#[test]
fn whm_index_cgi_falls_back_to_the_standalone_page_without_cpanel_template() {
    if !have("perl") {
        skip!("perl not installed");
    }
    let page = b"Status: 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<!doctype html><html></html>";
    let stub = Stub::new(page, 0);
    let (rc, out) = cmd(&["perl", &s(&repo().join("whm/cgi/index.cgi"))])
        .envs(&kv!["WEBPULSE_BIN" => stub.path(), "REMOTE_USER" => "root"])
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_eq!(out.as_bytes(), page, "the full document of the binary");
    assert_eq!(
        stub.recorded("args"),
        "cgi --whm --page --api-url api.cgi\n",
        "second run without --embedded"
    );
    // a template that dies still leaves the dashboard on the page
    let stub = Stub::new(FRAGMENT, 0);
    let perl = PerlStubs::new();
    let (_, out) = cmd(&["perl", &perl.lib(), &s(&repo().join("whm/cgi/index.cgi"))])
        .envs(&kv![
            "WEBPULSE_BIN" => stub.path(),
            "STUB_OUT" => s(&perl.out),
            "STUB_TEMPLATE_DIE" => "1",
            "REMOTE_USER" => "root",
        ])
        .go();
    assert!(
        out.starts_with("Content-type: text/html\r\n\r\n<div id=\"webpulse-app\""),
        "fragment printed after a template failure: {out}"
    );
}

#[test]
fn whm_api_cgi_execs_the_binary_with_the_documented_arguments() {
    let response: &[u8] =
        b"Status: 405 Method Not Allowed\r\nContent-Type: application/json\r\n\r\n{\"ok\":false}";
    let stub = Stub::new(response, 0);
    let (rc, out) = cmd(&["bash", &s(&repo().join("whm/cgi/api.cgi"))])
        .envs(&kv![
            "WEBPULSE_BIN" => stub.path(),
            "REMOTE_USER" => "reseller1",
            "REQUEST_METHOD" => "POST",
            "QUERY_STRING" => "a=admin.run",
            "CONTENT_TYPE" => "application/json",
            "CONTENT_LENGTH" => "2",
            "HTTP_X_WEBPULSE_REQUEST" => "1",
            "REMOTE_ADDR" => "198.51.100.7",
            "WEBPULSE_CONFIG" => "/some/config.json",
        ])
        .stdin("{}")
        .go();
    assert_eq!(rc, 0, "{out}");
    assert_eq!(
        stub.recorded("args"),
        "cgi --whm --api\n",
        "api.cgi arguments"
    );
    assert_eq!(
        out.as_bytes(),
        response,
        "stdout is exactly what the binary printed"
    );
    let seen = stub.recorded("env");
    for line in [
        "REMOTE_USER=reseller1",
        "REQUEST_METHOD=POST",
        "QUERY_STRING=a=admin.run",
        "HTTP_X_WEBPULSE_REQUEST=1",
        "REMOTE_ADDR=198.51.100.7",
        "WEBPULSE_CONFIG=/some/config.json",
    ] {
        assert_contains(
            &seen,
            &format!("{line}\n"),
            &format!("api.cgi forwards {line}"),
        );
    }
    assert_eq!(
        stub.recorded("stdin"),
        "{}",
        "the request body reaches the binary"
    );
}

#[test]
fn whm_cgis_print_a_generic_error_when_the_binary_is_missing() {
    if real_install_exists() {
        skip!("a real installation exists on this machine");
    }
    let dir = TmpDir::new("whmmissing");
    for cgi in ["index.cgi", "api.cgi"] {
        script(&dir.join(cgi), &read(&repo().join("whm/cgi").join(cgi)));
        let (rc, out) = cmd(&[interp(cgi), &s(&dir.join(cgi))])
            .env("WEBPULSE_BIN", dir.join("nonexistent/webpulse"))
            .env("REMOTE_USER", "root")
            .go();
        assert_eq!(rc, 0, "{out}");
        assert!(
            out.starts_with("Status: 500 Internal Server Error\r\n"),
            "{cgi} status: {out}"
        );
        assert_not_contains(&out, &s(&dir.path), &format!("{cgi}: no path leak"));
        assert_not_contains(&out, "nonexistent", &format!("{cgi}: no path leak"));
    }
    let (_, out) = cmd(&["bash", &s(&dir.join("api.cgi"))])
        .env("REMOTE_USER", "root")
        .go();
    let json_part = &out[out.find('{').expect("a JSON body")..];
    let v: Value = serde_json::from_str(json_part).expect("api.cgi error is JSON");
    assert_eq!(v["ok"], json!(false), "api.cgi error is JSON");
}

#[test]
fn whm_cgis_also_find_bin_webpulse_next_to_the_script() {
    if real_install_exists() {
        skip!("a real installation exists on this machine");
    }
    let dir = TmpDir::new("whmnear");
    let stub = Stub::new(b"Content-Type: text/plain\r\n\r\nnear", 0);
    script(&dir.join("bin/webpulse"), &read(&stub.bin));
    for cgi in ["index.cgi", "api.cgi"] {
        let path = script(&dir.join(cgi), &read(&repo().join("whm/cgi").join(cgi)));
        let (rc, out) = cmd(&[interp(cgi), &s(&path)])
            .env("REMOTE_USER", "root")
            .go();
        assert_eq!(rc, 0, "{out}");
        assert_contains(&out, "near", &format!("{cgi}: output of the binary"));
    }
    assert_eq!(stub.recorded("args"), "cgi --whm --api\n");
}

#[test]
fn whm_cgis_ignore_a_webpulse_bin_that_is_group_or_other_writable() {
    // WEBPULSE_BIN must be owned by the user running the CGI (root under cpsrvd) and not
    // group/other-writable, file and directory alike; otherwise the default path is used
    if real_install_exists() {
        skip!("a real installation exists on this machine");
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = TmpDir::new("whmtrust");
    for cgi in ["index.cgi", "api.cgi"] {
        script(&dir.join(cgi), &read(&repo().join("whm/cgi").join(cgi)));
    }
    let set_mode =
        |p: &Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    let stub = Stub::new(b"Status: 200 OK\r\nContent-Type: text/plain\r\n\r\nstub", 0);
    let stub_dir = stub.bin.parent().unwrap().to_path_buf();
    for cgi in ["index.cgi", "api.cgi"] {
        let run = || {
            cmd(&[interp(cgi), &s(&dir.join(cgi))])
                .env("WEBPULSE_BIN", stub.path())
                .env("REMOTE_USER", "root")
                .env("REQUEST_METHOD", "GET")
                .go()
        };
        // trusted: used
        let (_, out) = run();
        assert_contains(
            &out,
            "stub",
            &format!("{cgi}: a trusted WEBPULSE_BIN is used"),
        );
        // group-writable binary: ignored (no default install here -> generic 500)
        set_mode(&stub.bin, 0o775);
        let (rc, out) = run();
        assert_eq!(rc, 0, "{out}");
        assert!(
            out.starts_with("Status: 500 "),
            "{cgi}: group-writable binary ignored: {out}"
        );
        set_mode(&stub.bin, 0o757);
        let (_, out) = run();
        assert!(
            out.starts_with("Status: 500 "),
            "{cgi}: other-writable binary ignored: {out}"
        );
        set_mode(&stub.bin, 0o755);
        // world-writable directory: ignored (the binary could be swapped)
        set_mode(&stub_dir, 0o777);
        let (_, out) = run();
        assert!(
            out.starts_with("Status: 500 "),
            "{cgi}: writable directory ignored: {out}"
        );
        set_mode(&stub_dir, 0o755);
        // a symlink to an untrusted binary is judged by its target
        let link = dir.join(&format!("link-{cgi}"));
        std::os::unix::fs::symlink(&stub.bin, &link).unwrap();
        set_mode(&stub.bin, 0o777);
        let (_, out) = cmd(&[interp(cgi), &s(&dir.join(cgi))])
            .env("WEBPULSE_BIN", &link)
            .env("REMOTE_USER", "root")
            .go();
        assert!(
            out.starts_with("Status: 500 "),
            "{cgi}: symlink target checked: {out}"
        );
        set_mode(&stub.bin, 0o755);
    }
}
