<?php

declare(strict_types=1);

/*
 * Front-end shims: the cPanel LiveAPI pages (index.live.php, api.live.php, run on
 * cPanel's PHP) and the WHM bash CGIs. All logic lives in the binary; these tests
 * use a stub binary (WEBPULSE_BIN) that records its arguments, environment and stdin
 * and prints a CGI response, and check LiveApiUser, CgiBridge and the exec arguments.
 */

require_once __DIR__ . '/pkg_support.php';

$repo = PkgSupport::repo();
require_once "$repo/cpanel/webpulse/lib/LiveApiUser.php";
require_once "$repo/cpanel/webpulse/lib/CgiBridge.php";

use WebPulse\Cpanel\CgiBridge;
use WebPulse\Cpanel\LiveApiUser;

/** Run $fn with REMOTE_USER set/unset, restoring it afterwards. */
$withRemoteUser = static function (?string $value, callable $fn): mixed {
    $old = getenv('REMOTE_USER');
    $value === null ? putenv('REMOTE_USER') : putenv('REMOTE_USER=' . $value);
    try {
        return $fn();
    } finally {
        $old === false ? putenv('REMOTE_USER') : putenv('REMOTE_USER=' . $old);
    }
};

$fakeCpanel = static function (mixed $result, bool $throw = false): object {
    return new class ($result, $throw) {
        public function __construct(private mixed $result, private bool $throw)
        {
        }

        public function uapi(string $module, string $function, array $args = []): mixed
        {
            if ($this->throw) {
                throw new \RuntimeException('uapi down');
            }
            if ($module !== 'Variables' || $function !== 'get_user_information' || $args !== ['name' => 'user']) {
                throw new \LogicException('unexpected uapi call');
            }
            return $this->result;
        }
    };
};

/**
 * A stub WebPulse binary: records args/env/stdin in $log.* and prints $response
 * (a complete CGI response) verbatim, then exits $exit.
 */
$stubBinary = static function (string $response, int $exit = 0): array {
    $dir = PkgSupport::tmpDir('shimbin');
    $bin = "$dir/webpulse";
    $respFile = "$dir/response.txt";
    file_put_contents($respFile, $response);
    PkgSupport::script($bin, <<<SH
#!/bin/bash
log='$dir/call'
echo "\$*" > "\$log.args"
env | grep -E '^(REQUEST_METHOD|QUERY_STRING|CONTENT_TYPE|CONTENT_LENGTH|HTTP_X_WEBPULSE_REQUEST|REMOTE_ADDR|REMOTE_USER|WEBPULSE_CONFIG|STUB_CPANEL_USER)=' | sort > "\$log.env"
cat > "\$log.stdin"
cat '$respFile'
exit $exit
SH);
    return [$bin, "$dir/call"];
};

/** PHP prepend/append files: a stub CPANEL class, and a dump of the headers the shim set. */
$phpHarness = static function (): array {
    $dir = PkgSupport::tmpDir('phpharness');
    file_put_contents("$dir/prepend.php", <<<'PHP'
<?php
// stands in for /usr/local/cpanel/php/cpanel.php
class CPANEL {
    public function uapi($m, $f, $a = []) { return ['cpanelresult' => ['result' => ['data' => ['user' => getenv('STUB_CPANEL_USER') ?: 'alice']]]]; }
    public function header($title) { return "<!--HEADER:$title-->"; }
    public function footer() { return '<!--FOOTER-->'; }
    public function end() { file_put_contents(getenv('STUB_CPANEL_END') ?: '/dev/null', "end\n", FILE_APPEND); }
}
PHP);
    file_put_contents("$dir/append.php", <<<'PHP'
<?php
file_put_contents(getenv('STUB_HEADERS_OUT') ?: '/dev/null', json_encode(['status' => http_response_code(), 'headers' => headers_list()]));
PHP);
    return ["$dir/prepend.php", "$dir/append.php", "$dir/headers.json"];
};

/** Run a LiveAPI shim under php-cli with the stub CPANEL class. @return array{0:int,1:string,2:array} rc, output, headers dump */
$runLive = static function (string $shim, array $env, ?string $stdin = null) use ($repo, $phpHarness): array {
    [$prepend, $append, $headersOut] = $phpHarness();
    [$rc, $out] = PkgSupport::run([PHP_BINARY, '-d', "auto_prepend_file=$prepend", '-d', "auto_append_file=$append", '-d', 'error_log=/dev/null',
        '-d', 'display_errors=0', "$repo/cpanel/webpulse/$shim"], $env + ['STUB_HEADERS_OUT' => $headersOut], $stdin);
    $dump = is_file($headersOut) ? json_decode((string) file_get_contents($headersOut), true) : [];
    return [$rc, $out, is_array($dump) ? $dump : []];
};

/**
 * Serve cpanel/webpulse with PHP's built-in web server (a real SAPI where header() works)
 * and send one request. @return array{0:int,1:array<string,string>,2:string} status, headers (lower-case names), body
 */
$httpLive = static function (string $path, array $env, string $method = 'GET', ?string $body = null, array $reqHeaders = []) use ($repo, $phpHarness): array {
    [$prepend] = $phpHarness();
    $port = 0;
    $sock = @stream_socket_server('tcp://127.0.0.1:0', $errno, $errstr);
    if ($sock === false) {
        PkgSupport::skip('cannot allocate a local port');
    }
    $port = (int) substr((string) stream_socket_get_name($sock, false), strrpos((string) stream_socket_get_name($sock, false), ':') + 1);
    fclose($sock);
    $envFull = array_merge(['PATH' => (string) getenv('PATH'), 'HOME' => (string) getenv('HOME')], $env);
    $proc = proc_open([PHP_BINARY, '-d', "auto_prepend_file=$prepend", '-d', 'error_log=/dev/null', '-d', 'display_errors=0',
        '-S', "127.0.0.1:$port", '-t', "$repo/cpanel/webpulse"], [0 => ['file', '/dev/null', 'r'], 1 => ['file', '/dev/null', 'w'], 2 => ['file', '/dev/null', 'w']], $pipes, null, $envFull);
    if (!is_resource($proc)) {
        PkgSupport::skip('cannot start the PHP built-in server');
    }
    try {
        for ($i = 0; $i < 50; $i++) {
            $c = @fsockopen('127.0.0.1', $port, $e1, $e2, 0.2);
            if ($c !== false) {
                fclose($c);
                break;
            }
            usleep(100000);
        }
        $headerLines = '';
        foreach ($reqHeaders as $k => $v) {
            $headerLines .= "$k: $v\r\n";
        }
        $ctx = stream_context_create(['http' => ['method' => $method, 'header' => $headerLines, 'content' => $body ?? '', 'ignore_errors' => true, 'timeout' => 20]]);
        $resp = @file_get_contents("http://127.0.0.1:$port/$path", false, $ctx);
        $status = 0;
        $headers = [];
        foreach ($http_response_header ?? [] as $i => $line) {
            if ($i === 0 && preg_match('#^HTTP/\S+ (\d{3})#', $line, $m) === 1) {
                $status = (int) $m[1];
            } elseif (str_contains($line, ':')) {
                [$k, $v] = explode(':', $line, 2);
                $headers[strtolower(trim($k))] = trim($v);
            }
        }
        return [$status, $headers, (string) $resp];
    } finally {
        proc_terminate($proc);
        proc_close($proc);
    }
};


$recorded = static fn (string $prefix, string $ext): string => (string) @file_get_contents("$prefix.$ext");

return [
    'pkg_frontends: LiveApiUser::fromResult accepts every uapi() shape' => static function (): void {
        $arr = ['cpanelresult' => ['result' => ['data' => ['user' => 'alice']]]];
        PkgSupport::assertSame('alice', LiveApiUser::fromResult($arr), 'array shape');
        PkgSupport::assertSame('alice', LiveApiUser::fromResult(json_decode(json_encode($arr))), 'object shape');
        PkgSupport::assertSame('bob_1', LiveApiUser::fromResult((object) ['result' => (object) ['data' => (object) ['user' => 'bob_1']]]), 'objects all the way');
        PkgSupport::assertSame('alice', LiveApiUser::fromResult(['result' => ['data' => ['user' => 'alice']]]), 'without cpanelresult');
        PkgSupport::assertSame('alice', LiveApiUser::fromResult(['data' => ['user' => 'alice']]), 'bare data');
        PkgSupport::assertSame('alice', LiveApiUser::fromResult(['cpanelresult' => ['data' => ['user' => 'alice']]]), 'data directly under cpanelresult');
        PkgSupport::assertSame('alice', LiveApiUser::fromResult(['cpanelresult' => ['result' => ['data' => [['user' => 'alice']]]]]), 'list of records');
        PkgSupport::assertSame('alice', LiveApiUser::fromResult(['cpanelresult' => ['result' => ['data' => 'alice']]]), 'plain string');
        foreach ([null, 'alice', 42, [], ['data' => []], ['data' => ['user' => '../x']], ['data' => ['user' => 'ROOT']],
            ['data' => ['user' => ['a']]], ['data' => ['user' => str_repeat('a', 33)]], ["data" => ["user" => "alice\n"]]] as $bad) {
            PkgSupport::assertSame(null, LiveApiUser::fromResult($bad), 'rejects ' . json_encode($bad));
        }
    },

    'pkg_frontends: LiveApiUser validates the cPanel user grammar' => static function (): void {
        foreach (['alice', 'a', '_x', '0day', 'a-b_c', str_repeat('a', 32)] as $ok) {
            PkgSupport::assertTrue(LiveApiUser::valid($ok), "$ok is valid");
        }
        foreach (['', 'Alice', '-a', 'a b', 'a/b', '../x', 'a.b', "a\n", str_repeat('a', 33), 'é'] as $bad) {
            PkgSupport::assertTrue(!LiveApiUser::valid($bad), json_encode($bad) . ' is invalid');
        }
    },

    'pkg_frontends: LiveApiUser::resolve cascade uapi -> REMOTE_USER -> posix' => static function () use ($withRemoteUser, $fakeCpanel): void {
        $ok = ['cpanelresult' => ['result' => ['data' => ['user' => 'fromuapi']]]];
        // 1. uapi wins over everything
        $withRemoteUser('fromenv', function () use ($fakeCpanel, $ok): void {
            PkgSupport::assertSame('fromuapi', LiveApiUser::resolve($fakeCpanel($ok)), 'uapi first');
            // 2. uapi garbage / exception / missing -> REMOTE_USER
            PkgSupport::assertSame('fromenv', LiveApiUser::resolve($fakeCpanel(['data' => []])), 'empty uapi data');
            PkgSupport::assertSame('fromenv', LiveApiUser::resolve($fakeCpanel(null, true)), 'uapi throws');
            PkgSupport::assertSame('fromenv', LiveApiUser::resolve(null), 'no cpanel object');
            PkgSupport::assertSame('fromenv', LiveApiUser::resolve(new \stdClass()), 'object without uapi');
        });
        // 3. invalid / absent REMOTE_USER -> posix euid name
        $pw = function_exists('posix_getpwuid') ? posix_getpwuid(posix_geteuid()) : false;
        $expected = is_array($pw) && LiveApiUser::valid($pw['name']) ? $pw['name'] : null;
        foreach ([null, '', '../etc', 'Bad User'] as $env) {
            $r = $withRemoteUser($env, static function () use ($expected): ?string {
                try {
                    return LiveApiUser::resolve(null);
                } catch (\RuntimeException) {
                    return null;
                }
            });
            PkgSupport::assertSame($expected, $r, 'fallback for REMOTE_USER=' . json_encode($env));
        }
    },

    'pkg_frontends: LiveApiUser::resolve throws when nothing valid is found' => static function () use ($repo): void {
        // a subprocess without posix and without REMOTE_USER
        $code = 'require $argv[1]; try { WebPulse\Cpanel\LiveApiUser::resolve(null); echo "no exception"; } catch (RuntimeException $e) { echo "threw: ", $e->getMessage(); }';
        [$rc, $out] = PkgSupport::run([PHP_BINARY, '-d', 'disable_functions=posix_getpwuid,posix_geteuid', '-r', $code, '--', "$repo/cpanel/webpulse/lib/LiveApiUser.php"], ['REMOTE_USER' => '']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('threw: Unable to determine the cPanel account name', $out);
    },

    'pkg_frontends: CgiBridge::parseHead keeps plain headers and drops anything suspicious' => static function (): void {
        [$status, $lines] = CgiBridge::parseHead("Status: 404 Not Found\r\nContent-Type: application/json\r\nX-A: b\r\nbad line\r\nEvil\x01: x\r\nSet-Cookie: a=b\r\n");
        PkgSupport::assertSame(404, $status);
        PkgSupport::assertSame(['Content-Type: application/json', 'X-A: b', 'Set-Cookie: a=b'], $lines);
        [$status, $lines] = CgiBridge::parseHead("Content-Type: text/html\n");
        PkgSupport::assertSame(200, $status, 'default status');
        PkgSupport::assertSame(['Content-Type: text/html'], $lines);
        [$status] = CgiBridge::parseHead('Status: 503');
        PkgSupport::assertSame(503, $status);
    },

    'pkg_frontends: CgiBridge resolves WEBPULSE_BIN, then the default path, then bin/ next to the shim' => static function (): void {
        $dir = PkgSupport::tmpDir('binres');
        $env = PkgSupport::script("$dir/env-bin", "#!/bin/sh\n");
        mkdir("$dir/shim/bin", 0777, true);
        $near = PkgSupport::script("$dir/shim/bin/webpulse", "#!/bin/sh\n");
        $old = getenv('WEBPULSE_BIN');
        try {
            putenv("WEBPULSE_BIN=$env");
            PkgSupport::assertSame($env, CgiBridge::binary("$dir/shim"), 'env wins');
            putenv("WEBPULSE_BIN=$dir/not-executable");
            file_put_contents("$dir/not-executable", 'x');
            $r = CgiBridge::binary("$dir/shim");
            PkgSupport::assertTrue($r === $near || $r === CgiBridge::DEFAULT_BIN, 'unusable env value ignored');
            putenv('WEBPULSE_BIN');
            if (!is_executable(CgiBridge::DEFAULT_BIN)) {
                PkgSupport::assertSame($near, CgiBridge::binary("$dir/shim"), 'next to the shim');
                PkgSupport::assertSame(null, CgiBridge::binary("$dir/nowhere"), 'nothing found');
            }
        } finally {
            $old === false ? putenv('WEBPULSE_BIN') : putenv("WEBPULSE_BIN=$old");
        }
    },

    'pkg_frontends: CgiBridge forwards only the CGI variables and WEBPULSE_CONFIG' => static function (): void {
        $set = ['REQUEST_METHOD' => 'POST', 'QUERY_STRING' => 'a=x', 'CONTENT_TYPE' => 'application/json', 'CONTENT_LENGTH' => '3',
            'HTTP_X_WEBPULSE_REQUEST' => '1', 'REMOTE_ADDR' => '203.0.113.9', 'WEBPULSE_CONFIG' => '/etc/other.json', 'SECRET_TOKEN' => 'nope'];
        $old = [];
        foreach ($set as $k => $v) {
            $old[$k] = getenv($k);
            putenv("$k=$v");
        }
        try {
            $env = CgiBridge::environment('alice');
        } finally {
            foreach ($old as $k => $v) {
                $v === false ? putenv($k) : putenv("$k=$v");
            }
        }
        PkgSupport::assertSame('POST', $env['REQUEST_METHOD']);
        PkgSupport::assertSame('alice', $env['REMOTE_USER'], 'the resolved account, not a request value');
        PkgSupport::assertSame('/etc/other.json', $env['WEBPULSE_CONFIG']);
        PkgSupport::assertSame('203.0.113.9', $env['REMOTE_ADDR']);
        PkgSupport::assertTrue(!isset($env['SECRET_TOKEN']), 'unrelated variables are not forwarded');
        PkgSupport::assertTrue(isset($env['PATH']), 'PATH is set');
    },

    'pkg_frontends: api.live.php passes status, headers and body through and pipes the request body' => static function () use ($stubBinary, $httpLive, $recorded): void {
        [$bin, $call] = $stubBinary("Status: 201 Created\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\nX-Test: yes\r\n\r\n{\"ok\":true,\"data\":{\"hello\":\"world\"}}");
        // a real HTTP request to PHP's built-in server: CGI variables come from $_SERVER, the body from php://input
        [$status, $headers, $body] = $httpLive('api.live.php?a=save_user_settings&user=bob', [
            'WEBPULSE_BIN' => $bin, 'STUB_CPANEL_USER' => 'alice', 'WEBPULSE_CONFIG' => '/some/config.json', 'REMOTE_USER' => 'mallory',
        ], 'POST', '{"theme":"dark"}', ['Content-Type' => 'application/json', 'X-WebPulse-Request' => '1']);
        PkgSupport::assertSame(201, $status, 'status line becomes the HTTP status');
        PkgSupport::assertSame('{"ok":true,"data":{"hello":"world"}}', $body, 'body passthrough, nothing else printed');
        PkgSupport::assertSame('application/json; charset=utf-8', $headers['content-type'] ?? null, 'Content-Type');
        PkgSupport::assertSame('no-store', $headers['cache-control'] ?? null, 'Cache-Control');
        PkgSupport::assertSame('yes', $headers['x-test'] ?? null, 'custom header');
        PkgSupport::assertTrue(!isset($headers['status']), 'the Status line is not echoed as a header');

        PkgSupport::assertSame("cgi --principal user:alice --api\n", $recorded($call, 'args'), 'principal from the cPanel session, never from the request');
        $seen = $recorded($call, 'env');
        foreach (['REQUEST_METHOD=POST', 'QUERY_STRING=a=save_user_settings&user=bob', 'CONTENT_TYPE=application/json',
            'HTTP_X_WEBPULSE_REQUEST=1', 'REMOTE_ADDR=127.0.0.1', 'REMOTE_USER=alice', 'WEBPULSE_CONFIG=/some/config.json', 'CONTENT_LENGTH=16'] as $line) {
            PkgSupport::assertContains($line . "\n", $seen, 'forwarded variable');
        }
        PkgSupport::assertSame('{"theme":"dark"}', $recorded($call, 'stdin'), 'request body piped to the binary');
    },

    'pkg_frontends: api.live.php (CLI) falls back to STDIN for the body and forwards the environment' => static function () use ($stubBinary, $runLive, $recorded): void {
        [$bin, $call] = $stubBinary("Content-Type: application/json\r\n\r\n{\"ok\":true}");
        $env = ['WEBPULSE_BIN' => $bin, 'REQUEST_METHOD' => 'POST', 'QUERY_STRING' => 'a=x', 'CONTENT_TYPE' => 'application/json', 'CONTENT_LENGTH' => '16',
            'HTTP_X_WEBPULSE_REQUEST' => '1', 'REMOTE_ADDR' => '203.0.113.9', 'REMOTE_USER' => 'mallory', 'STUB_CPANEL_USER' => 'alice'];
        [$rc, $out] = $runLive('api.live.php', $env, '{"theme":"dark"}');
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertSame('{"ok":true}', $out);
        PkgSupport::assertSame('{"theme":"dark"}', $recorded($call, 'stdin'));
        PkgSupport::assertContains("REMOTE_ADDR=203.0.113.9\n", $recorded($call, 'env'));
        PkgSupport::assertContains("REMOTE_USER=alice\n", $recorded($call, 'env'));
    },

    'pkg_frontends: api.live.php GET sends no body and a 4xx from the binary reaches the client' => static function () use ($stubBinary, $runLive, $recorded): void {
        [$bin, $call] = $stubBinary("Status: 403 Forbidden\r\nContent-Type: application/json\r\n\r\n{\"ok\":false}");
        [$rc, $out, $dump] = $runLive('api.live.php', ['WEBPULSE_BIN' => $bin, 'REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=domains']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertSame('{"ok":false}', $out);
        PkgSupport::assertSame(403, $dump['status'] ?? null);
        PkgSupport::assertSame('', $recorded($call, 'stdin'), 'no body for GET');
        PkgSupport::assertContains('QUERY_STRING=a=domains', $recorded($call, 'env'));
    },

    'pkg_frontends: api.live.php streams a large body unchanged' => static function () use ($stubBinary, $runLive): void {
        $big = str_repeat('0123456789abcdef', 40000); // 640 KB
        [$bin] = $stubBinary("Content-Type: text/csv\r\n\r\n" . $big);
        [$rc, $out, $dump] = $runLive('api.live.php', ['WEBPULSE_BIN' => $bin, 'REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=export']);
        PkgSupport::assertSame(0, $rc);
        PkgSupport::assertSame(strlen($big), strlen($out), 'length');
        PkgSupport::assertTrue($out === $big, 'content');
        PkgSupport::assertSame(200, $dump['status'] ?? null, 'default status');
    },

    'pkg_frontends: api.live.php answers a generic JSON 500 when the binary is missing, broken or silent' => static function () use ($stubBinary, $runLive, $repo): void {
        $secret = PkgSupport::tmpDir('secret') . '/missing-config.json';
        $cases = [
            'missing binary' => ['/nonexistent/dir/webpulse'],
            'crash without output' => [$stubBinary('', 1)[0]],
            'garbage without header terminator' => [$stubBinary('this is not a cgi response', 0)[0]],
        ];
        foreach ($cases as $label => [$bin]) {
            [$rc, $out, $dump] = $runLive('api.live.php', ['WEBPULSE_BIN' => $bin, 'WEBPULSE_CONFIG' => $secret, 'REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=domains']);
            if ($label === 'missing binary' && is_executable('/usr/local/webpulse/bin/webpulse')) {
                continue;
            }
            PkgSupport::assertSame(0, $rc, "$label: $out");
            $json = json_decode($out, true);
            PkgSupport::assertTrue(is_array($json), "$label: JSON body expected, got: $out");
            PkgSupport::assertSame(false, $json['ok'], "$label: ok=false");
            PkgSupport::assertSame('internal_error', $json['error']['code'], $label);
            PkgSupport::assertSame(500, $dump['status'] ?? null, "$label: HTTP 500");
            PkgSupport::assertNotContains('missing-config', $out, "$label: no path leak");
            PkgSupport::assertNotContains($bin, $out, "$label: no path leak");
            PkgSupport::assertNotContains($repo, $out, "$label: no path leak");
        }
    },

    'pkg_frontends: index.live.php wraps the embedded dashboard in the cPanel chrome' => static function () use ($stubBinary, $runLive, $recorded): void {
        [$bin, $call] = $stubBinary("Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\r\n<div id=\"webpulse-app\" data-api=\"api.live.php\">dashboard</div>");
        $endLog = PkgSupport::tmpDir('end') . '/end.log';
        [$rc, $out, $dump] = $runLive('index.live.php', ['WEBPULSE_BIN' => $bin, 'REMOTE_USER' => 'mallory', 'STUB_CPANEL_USER' => 'alice', 'STUB_CPANEL_END' => $endLog]);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertSame('<!--HEADER:WebPulse Analytics--><div id="webpulse-app" data-api="api.live.php">dashboard</div><!--FOOTER-->', $out, 'chrome + fragment, no CGI header text in the page');
        PkgSupport::assertSame("cgi --principal user:alice --page --embedded --api-url api.live.php\n", $recorded($call, 'args'));
        PkgSupport::assertSame("end\n", (string) @file_get_contents($endLog), 'end() called once');
        PkgSupport::assertNotContains('Content-Type: text/html', implode("\n", $dump['headers'] ?? []), 'headers of the binary are not applied after the chrome started');
    },

    'pkg_frontends: index.live.php shows a friendly message, without paths, when the binary fails' => static function () use ($stubBinary, $runLive): void {
        foreach (['/nonexistent/dir/webpulse', $stubBinary('', 1)[0]] as $bin) {
            if ($bin === '/nonexistent/dir/webpulse' && is_executable('/usr/local/webpulse/bin/webpulse')) {
                continue;
            }
            [$rc, $out] = $runLive('index.live.php', ['WEBPULSE_BIN' => $bin, 'WEBPULSE_CONFIG' => '/nonexistent/config.json']);
            PkgSupport::assertSame(0, $rc, $out);
            PkgSupport::assertTrue(str_starts_with($out, '<!--HEADER:WebPulse Analytics-->'), 'chrome header');
            PkgSupport::assertContains('not available right now', $out);
            PkgSupport::assertTrue(str_ends_with($out, '<!--FOOTER-->'), 'chrome footer');
            PkgSupport::assertNotContains('/nonexistent', $out, 'no path leak');
            PkgSupport::assertNotContains('Stack trace', $out);
        }
    },

    'pkg_frontends: the shims find a binary next to them (bin/webpulse)' => static function () use ($repo, $stubBinary, $recorded): void {
        if (is_executable('/usr/local/webpulse/bin/webpulse')) {
            PkgSupport::skip('a real installation exists on this machine');
        }
        $dir = PkgSupport::tmpDir('nextto');
        PkgSupport::run(['cp', '-a', "$repo/cpanel/webpulse/.", $dir]);
        [$stub, $call] = $stubBinary("Content-Type: application/json\r\n\r\n{\"ok\":true}");
        mkdir("$dir/bin");
        copy($stub, "$dir/bin/webpulse");
        chmod("$dir/bin/webpulse", 0755);
        // the stub logs next to itself: point it at the original location
        file_put_contents("$dir/bin/webpulse", str_replace("log='" . dirname($stub) . "/call'", "log='$call'", (string) file_get_contents($stub)));
        $prepend = PkgSupport::tmpDir('prep') . '/prepend.php';
        file_put_contents($prepend, "<?php class CPANEL { public function uapi(\$m,\$f,\$a=[]) { return ['data'=>['user'=>'alice']]; } public function end() {} }\n");
        [$rc, $out] = PkgSupport::run([PHP_BINARY, '-d', "auto_prepend_file=$prepend", "$dir/api.live.php"], ['REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=domains']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertSame('{"ok":true}', $out);
        PkgSupport::assertSame("cgi --principal user:alice --api\n", $recorded($call, 'args'));
    },

    'pkg_frontends: WHM index.cgi and api.cgi exec the binary with the documented arguments' => static function () use ($repo, $stubBinary, $recorded): void {
        $cases = [
            'index.cgi' => ["cgi --whm --page --api-url api.cgi\n", "Content-Type: text/html; charset=utf-8\r\n\r\n<!doctype html><html></html>"],
            'api.cgi' => ["cgi --whm --api\n", "Status: 405 Method Not Allowed\r\nContent-Type: application/json\r\n\r\n{\"ok\":false}"],
        ];
        foreach ($cases as $cgi => [$expectedArgs, $response]) {
            [$bin, $call] = $stubBinary($response);
            [$rc, $out] = PkgSupport::run(['bash', "$repo/whm/cgi/$cgi"], [
                'WEBPULSE_BIN' => $bin, 'REMOTE_USER' => 'reseller1', 'REQUEST_METHOD' => 'POST', 'QUERY_STRING' => 'a=admin.run',
                'CONTENT_TYPE' => 'application/json', 'CONTENT_LENGTH' => '2', 'HTTP_X_WEBPULSE_REQUEST' => '1', 'REMOTE_ADDR' => '198.51.100.7',
                'WEBPULSE_CONFIG' => '/some/config.json',
            ], '{}');
            PkgSupport::assertSame(0, $rc, "$cgi: $out");
            PkgSupport::assertSame($expectedArgs, $recorded($call, 'args'), "$cgi arguments");
            PkgSupport::assertSame($response, $out, "$cgi stdout is exactly what the binary printed");
            $seen = $recorded($call, 'env');
            foreach (['REMOTE_USER=reseller1', 'REQUEST_METHOD=POST', 'QUERY_STRING=a=admin.run', 'HTTP_X_WEBPULSE_REQUEST=1', 'REMOTE_ADDR=198.51.100.7', 'WEBPULSE_CONFIG=/some/config.json'] as $line) {
                PkgSupport::assertContains($line . "\n", $seen, "$cgi forwards $line");
            }
            PkgSupport::assertSame('{}', $recorded($call, 'stdin'), "$cgi: the request body reaches the binary");
        }
    },

    'pkg_frontends: WHM CGIs print a generic error when the binary is missing' => static function () use ($repo): void {
        if (is_executable('/usr/local/webpulse/bin/webpulse')) {
            PkgSupport::skip('a real installation exists on this machine');
        }
        $dir = PkgSupport::tmpDir('whmmissing');
        foreach (['index.cgi', 'api.cgi'] as $cgi) {
            copy("$repo/whm/cgi/$cgi", "$dir/$cgi");
            chmod("$dir/$cgi", 0755);
            [$rc, $out] = PkgSupport::run(['bash', "$dir/$cgi"], ['WEBPULSE_BIN' => "$dir/nonexistent/webpulse", 'REMOTE_USER' => 'root']);
            PkgSupport::assertSame(0, $rc, $out);
            PkgSupport::assertTrue(str_starts_with($out, "Status: 500 Internal Server Error\r\n"), "$cgi status");
            PkgSupport::assertNotContains($dir, $out, "$cgi: no path leak");
            PkgSupport::assertNotContains('nonexistent', $out, "$cgi: no path leak");
        }
        [, $out] = PkgSupport::run(['bash', "$dir/api.cgi"], ['REMOTE_USER' => 'root']);
        PkgSupport::assertSame(false, json_decode(substr($out, (int) strpos($out, '{')), true)['ok'] ?? null, 'api.cgi error is JSON');
    },

    'pkg_frontends: WHM CGIs also find bin/webpulse next to the script' => static function () use ($repo, $stubBinary, $recorded): void {
        if (is_executable('/usr/local/webpulse/bin/webpulse')) {
            PkgSupport::skip('a real installation exists on this machine');
        }
        $dir = PkgSupport::tmpDir('whmnear');
        mkdir("$dir/bin");
        [$stub, $call] = $stubBinary("Content-Type: text/plain\r\n\r\nnear");
        file_put_contents("$dir/bin/webpulse", str_replace("log='" . dirname($stub) . "/call'", "log='$call'", (string) file_get_contents($stub)));
        chmod("$dir/bin/webpulse", 0755);
        copy("$repo/whm/cgi/index.cgi", "$dir/index.cgi");
        chmod("$dir/index.cgi", 0755);
        [$rc, $out] = PkgSupport::run(["$dir/index.cgi"], ['REMOTE_USER' => 'root']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('near', $out);
        PkgSupport::assertSame("cgi --whm --page --api-url api.cgi\n", $recorded($call, 'args'));
    },
];
