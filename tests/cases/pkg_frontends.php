<?php

declare(strict_types=1);

/*
 * WP3: PHP front-ends (cPanel live.php, WHM CGI): LiveApiUser cascade, WHM edge
 * checks, bootstrap resolution, and smoke runs against the real engine/UI
 * classes when those packages are present.
 */

require_once __DIR__ . '/pkg_support.php';

$repo = PkgSupport::repo();
require_once "$repo/cpanel/webpulse/lib/LiveApiUser.php";
require_once "$repo/whm/lib/Edge.php";

use WebPulse\Cpanel\LiveApiUser;
use WebPulse\Whm\Edge;

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

/** Does the engine/UI package exist well enough for a smoke run? */
$haveStack = static function () use ($repo): bool {
    foreach (['lib/autoload.php', 'lib/WebPulse/App.php', 'lib/WebPulse/Http/Api.php', 'lib/WebPulse/Http/Page.php',
        'lib/WebPulse/Http/PageOptions.php', 'lib/WebPulse/Discovery/CpanelUsers.php', 'ui/app.css', 'ui/app.js'] as $f) {
        if (!is_file("$repo/$f")) {
            return false;
        }
    }
    return true;
};

/** A config + fake server root for the smoke runs. @return array{0:string,1:string} config path, root */
$smokeRoot = static function (): array {
    $root = PkgSupport::tmpDir('smoke');
    foreach (['var/cpanel/users', 'var/webpulse/data', 'var/webpulse/log', 'var/webpulse/run', 'etc'] as $d) {
        mkdir("$root/$d", 0777, true);
    }
    file_put_contents("$root/var/cpanel/users/alice", "DNS=example.com\nOWNER=reseller1\n");
    file_put_contents("$root/var/cpanel/users/bob", "DNS=blog.net\nOWNER=root\n");
    file_put_contents("$root/etc/userdomains", "example.com: alice\nblog.net: bob\n");
    $cfg = [
        'version' => 1,
        'paths' => [
            'data_dir' => "$root/var/webpulse/data", 'registry_db' => "$root/var/webpulse/webpulse.db",
            'log_dir' => "$root/var/webpulse/log", 'run_dir' => "$root/var/webpulse/run",
            'domlogs_dirs' => ["$root/domlogs/{user}"], 'archive_dir' => "$root/home/{user}/logs",
            'userdomains_file' => "$root/etc/userdomains", 'cpanel_users_dir' => "$root/var/cpanel/users",
            'cpanel_userdata_dir' => "$root/var/cpanel/userdata", 'home_root' => "$root/home",
            'cron_file' => "$root/etc/cron.d-webpulse", 'hooks_file' => "$root/var/cpanel/hooks.yaml",
        ],
    ];
    file_put_contents("$root/config.json", json_encode($cfg));
    return ["$root/config.json", $root];
};

$cpanelStub = static function (): string {
    $f = PkgSupport::tmpDir('stub') . '/prepend.php';
    file_put_contents($f, <<<'PHP'
<?php
// stands in for /usr/local/cpanel/php/cpanel.php
class CPANEL {
    public function uapi($m, $f, $a = []) { return ['cpanelresult' => ['result' => ['data' => ['user' => getenv('STUB_CPANEL_USER') ?: 'alice']]]]; }
    public function header($title) { return "<!--HEADER:$title-->"; }
    public function footer() { return '<!--FOOTER-->'; }
    public function end() { file_put_contents(getenv('STUB_CPANEL_END') ?: '/dev/null', "end\n", FILE_APPEND); }
}
PHP);
    return $f;
};

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

    'pkg_frontends: WHM Edge::principal maps REMOTE_USER to a principal' => static function () use ($repo): void {
        require_once "$repo/lib/autoload.php";
        if (!class_exists(\WebPulse\Http\Principal::class)) {
            fwrite(STDERR, "SKIP Principal not available\n");
            return;
        }
        $users = new class () {
            public function all(): array
            {
                return [
                    'zed' => ['owner' => 'res1', 'domains' => [], 'home' => '/home/zed'],
                    'alice' => ['owner' => 'res1', 'domains' => [], 'home' => '/home/alice'],
                    'bob' => ['owner' => 'root', 'domains' => [], 'home' => '/home/bob'],
                    'res1' => ['owner' => 'root', 'domains' => [], 'home' => '/home/res1'],
                    'odd' => ['owner' => null],
                ];
            }
        };
        $root = Edge::principal('root', $users);
        PkgSupport::assertSame('admin', $root->type, 'root is admin');
        PkgSupport::assertSame('root', $root->name);
        $res = Edge::principal('res1', $users);
        PkgSupport::assertSame('reseller', $res->type, 'reseller');
        PkgSupport::assertSame(['alice', 'zed'], $res->ownedUsers, 'owned accounts only, sorted');
        PkgSupport::assertTrue($res->canSeeUser('alice') && !$res->canSeeUser('bob'), 'scope');
        foreach ([null, '', '  ', 'nobody', 'bob', 'Root', '../x', 'res1;id', "res1\n", 'ROOT'] as $who) {
            PkgSupport::assertSame(null, Edge::principal($who, $users), 'forbidden: ' . json_encode($who));
        }
    },

    'pkg_frontends: WHM Edge mutation guard' => static function (): void {
        foreach (['admin.set_account', 'admin.set_domain', 'admin.save_settings', 'admin.run', 'admin.purge', 'save_user_settings'] as $a) {
            PkgSupport::assertTrue(Edge::isMutating($a), "$a is mutating");
            PkgSupport::assertTrue(!Edge::mutationAllowed($a, 'GET', null), "$a via GET denied");
            PkgSupport::assertTrue(!Edge::mutationAllowed($a, 'POST', null), "$a without header denied");
            PkgSupport::assertTrue(!Edge::mutationAllowed($a, 'POST', '0'), "$a with wrong header denied");
            PkgSupport::assertTrue(Edge::mutationAllowed($a, 'POST', '1'), "$a POST + header allowed");
        }
        foreach (['domains', 'overview', 'report', 'admin.overview', 'admin.accounts', 'admin.jobs', 'admin.settings', 'export', 'visits'] as $a) {
            PkgSupport::assertTrue(!Edge::isMutating($a), "$a is a read");
            PkgSupport::assertTrue(Edge::mutationAllowed($a, 'GET', null), "$a GET allowed");
            PkgSupport::assertTrue(Edge::mutationAllowed($a, 'HEAD', null), "$a HEAD allowed");
        }
        PkgSupport::assertTrue(!Edge::mutationAllowed('overview', 'DELETE', '1'), 'unsafe methods other than POST denied');
        PkgSupport::assertTrue(!Edge::mutationAllowed('overview', 'PUT', null), 'PUT denied');
    },

    'pkg_frontends: WHM Edge::request builds the Request from the CGI environment' => static function () use ($repo): void {
        require_once "$repo/lib/autoload.php";
        if (!class_exists(\WebPulse\Http\Request::class)) {
            fwrite(STDERR, "SKIP Request not available\n");
            return;
        }
        $r = Edge::request([
            'REQUEST_METHOD' => 'POST', 'QUERY_STRING' => 'a=admin.set_account&x[]=1', 'CONTENT_TYPE' => 'application/json',
            'HTTP_X_WEBPULSE_REQUEST' => '1', 'REMOTE_ADDR' => '203.0.113.9', 'CONTENT_LENGTH' => '10',
        ], '{"user":"a"}');
        PkgSupport::assertSame('POST', $r->method);
        PkgSupport::assertSame('admin.set_account', $r->get('a'));
        PkgSupport::assertSame('1', $r->header('X-WebPulse-Request'));
        PkgSupport::assertSame('{"user":"a"}', $r->body);
        PkgSupport::assertSame('203.0.113.9', $r->remoteAddr);
        $g = Edge::request(['REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=domains'], 'ignored');
        PkgSupport::assertSame(null, $g->body, 'GET has no body');
    },

    'pkg_frontends: bootstrap locates the autoloader (WEBPULSE_HOME, then repo layout)' => static function () use ($repo): void {
        // copy the bootstrap into a mini repository layout, minus the final "require" part
        $tmp = PkgSupport::tmpDir('boot');
        mkdir("$tmp/cpanel/webpulse/lib", 0777, true);
        mkdir("$tmp/home/lib", 0777, true);
        mkdir("$tmp/lib", 0777, true);
        $src = (string) file_get_contents("$repo/cpanel/webpulse/lib/bootstrap.php");
        $cut = strpos($src, '$webpulseAutoload = webpulse_find_autoload();');
        PkgSupport::assertTrue($cut !== false, 'bootstrap structure');
        file_put_contents("$tmp/cpanel/webpulse/lib/bootstrap.php", substr($src, 0, $cut));
        $lookup = 'require $argv[1]; echo WEBPULSE_HOME, "|", webpulse_find_autoload();';
        $boot = "$tmp/cpanel/webpulse/lib/bootstrap.php";

        // nothing exists yet (unless the machine has a real installation)
        [, $out] = PkgSupport::run([PHP_BINARY, '-r', $lookup, '--', $boot], ['WEBPULSE_HOME' => "$tmp/nowhere"]);
        if (!is_file('/usr/local/webpulse/lib/autoload.php')) {
            PkgSupport::assertSame("$tmp/nowhere|", $out, 'nothing found');
        }
        file_put_contents("$tmp/lib/autoload.php", "<?php\n");
        [, $out] = PkgSupport::run([PHP_BINARY, '-r', $lookup, '--', $boot], ['WEBPULSE_HOME' => "$tmp/nowhere"]);
        PkgSupport::assertTrue($out === "$tmp/nowhere|$tmp/lib/autoload.php" || $out === "$tmp/nowhere|/usr/local/webpulse/lib/autoload.php", "repo layout fallback: $out");
        file_put_contents("$tmp/cpanel/webpulse/lib/autoload.php", "<?php\n");
        [, $out] = PkgSupport::run([PHP_BINARY, '-r', $lookup, '--', $boot], ['WEBPULSE_HOME' => "$tmp/nowhere"]);
        PkgSupport::assertTrue($out === "$tmp/nowhere|$tmp/cpanel/webpulse/lib/autoload.php" || $out === "$tmp/nowhere|/usr/local/webpulse/lib/autoload.php", "theme copy fallback: $out");
        file_put_contents("$tmp/home/lib/autoload.php", "<?php\n");
        [, $out] = PkgSupport::run([PHP_BINARY, '-r', $lookup, '--', $boot], ['WEBPULSE_HOME' => "$tmp/home/"]);
        PkgSupport::assertSame("$tmp/home|$tmp/home/lib/autoload.php", $out, 'WEBPULSE_HOME wins and is normalised');

        // the real bootstrap without any autoloader throws a RuntimeException (index.live.php catches it)
        [, $out] = PkgSupport::run([PHP_BINARY, '-r', 'try { require $argv[1]; echo "loaded"; } catch (RuntimeException $e) { echo "threw"; }', '--', "$repo/cpanel/webpulse/lib/bootstrap.php"], ['WEBPULSE_HOME' => "$tmp/nowhere"]);
        PkgSupport::assertTrue($out === 'loaded' || $out === 'threw', $out);
    },

    'pkg_frontends: api.live.php returns a JSON 500 without leaking paths when the engine cannot boot' => static function () use ($repo, $cpanelStub): void {
        $prepend = $cpanelStub();
        $secret = PkgSupport::tmpDir('secret') . '/missing-config.json';
        [$rc, $out] = PkgSupport::run([PHP_BINARY, '-d', "auto_prepend_file=$prepend", '-d', 'error_log=/dev/null', "$repo/cpanel/webpulse/api.live.php"], [
            'WEBPULSE_HOME' => is_file("$repo/lib/autoload.php") ? $repo : '/nonexistent',
            'WEBPULSE_CONFIG' => $secret, 'REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=domains', 'REMOTE_USER' => 'alice',
        ]);
        PkgSupport::assertSame(0, $rc, $out);
        $json = json_decode($out, true);
        PkgSupport::assertTrue(is_array($json), "JSON body expected, got: $out");
        PkgSupport::assertSame(false, $json['ok'], 'ok=false');
        PkgSupport::assertSame('internal_error', $json['error']['code']);
        PkgSupport::assertNotContains('missing-config', $out, 'no path leak');
        PkgSupport::assertNotContains($repo, $out, 'no path leak');
    },

    'pkg_frontends: api.live.php dispatches with the account from the cPanel session' => static function () use ($repo, $haveStack, $smokeRoot, $cpanelStub): void {
        if (!$haveStack()) {
            fwrite(STDERR, "SKIP engine/UI packages not complete yet\n");
            return;
        }
        [$cfg] = $smokeRoot();
        $prepend = $cpanelStub();
        $env = ['WEBPULSE_HOME' => $repo, 'WEBPULSE_CONFIG' => $cfg, 'REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=domains',
            'REMOTE_USER' => 'mallory', 'STUB_CPANEL_USER' => 'alice'];
        [$rc, $out] = PkgSupport::run([PHP_BINARY, '-d', "auto_prepend_file=$prepend", "$repo/cpanel/webpulse/api.live.php"], $env);
        PkgSupport::assertSame(0, $rc, $out);
        $json = json_decode(substr($out, (int) strpos($out, '{')), true);
        PkgSupport::assertTrue(is_array($json), "JSON expected, got: $out");
        PkgSupport::assertSame(true, $json['ok'] ?? null, "ok expected: $out");
        // a user can never ask for another account via parameters
        $env['QUERY_STRING'] = 'a=domains&user=bob';
        [$rc, $out] = PkgSupport::run([PHP_BINARY, '-d', "auto_prepend_file=$prepend", "$repo/cpanel/webpulse/api.live.php"], $env);
        $json = json_decode(substr($out, (int) strpos($out, '{')), true);
        foreach ($json['data']['domains'] ?? [] as $d) {
            PkgSupport::assertSame('alice', $d['user'], 'only the session account is visible');
        }
    },

    'pkg_frontends: index.live.php wraps the dashboard in the cPanel chrome' => static function () use ($repo, $haveStack, $smokeRoot, $cpanelStub): void {
        if (!$haveStack()) {
            fwrite(STDERR, "SKIP engine/UI packages not complete yet\n");
            return;
        }
        [$cfg] = $smokeRoot();
        $prepend = $cpanelStub();
        $endLog = PkgSupport::tmpDir('end') . '/end.log';
        [$rc, $out] = PkgSupport::run([PHP_BINARY, '-d', "auto_prepend_file=$prepend", '-d', 'error_log=/dev/null', "$repo/cpanel/webpulse/index.live.php"],
            ['WEBPULSE_HOME' => $repo, 'WEBPULSE_CONFIG' => $cfg, 'REMOTE_USER' => 'alice', 'STUB_CPANEL_END' => $endLog]);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertTrue(str_starts_with($out, '<!--HEADER:WebPulse Analytics-->'), 'starts with cPanel header');
        PkgSupport::assertContains('webpulse-app', $out, 'dashboard fragment');
        PkgSupport::assertContains('api.live.php', $out, 'api url in the bootstrap');
        PkgSupport::assertTrue(str_ends_with($out, '<!--FOOTER-->'), 'ends with the footer');
        PkgSupport::assertSame("end\n", (string) @file_get_contents($endLog), 'end() called once');

        // a broken installation shows a friendly message, not a stack trace
        [$rc, $out] = PkgSupport::run([PHP_BINARY, '-d', "auto_prepend_file=$prepend", '-d', 'error_log=/dev/null', "$repo/cpanel/webpulse/index.live.php"],
            ['WEBPULSE_HOME' => $repo, 'WEBPULSE_CONFIG' => '/nonexistent/config.json', 'REMOTE_USER' => 'alice']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('not available right now', $out);
        PkgSupport::assertNotContains('/nonexistent', $out, 'no path leak');
        PkgSupport::assertNotContains('Stack trace', $out);
    },

    'pkg_frontends: api.cgi enforces identity and the mutation guard at the edge' => static function () use ($repo, $haveStack, $smokeRoot): void {
        if (!$haveStack()) {
            fwrite(STDERR, "SKIP engine/UI packages not complete yet\n");
            return;
        }
        [$cfg] = $smokeRoot();
        $base = ['WEBPULSE_HOME' => $repo, 'WEBPULSE_CONFIG' => $cfg];
        $run = static function (array $env, ?string $stdin = null) use ($repo, $base): array {
            [$rc, $out] = PkgSupport::run([PHP_BINARY, "$repo/whm/cgi/api.cgi"], $env + $base, $stdin);
            [$head, $body] = array_pad(explode("\r\n\r\n", $out, 2), 2, '');
            return [$rc, $head, $body];
        };

        [, $head, $body] = $run(['REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=admin.overview']);
        PkgSupport::assertContains('Status: 403', $head, 'no REMOTE_USER');
        PkgSupport::assertSame(false, json_decode($body, true)['ok'] ?? null);

        [, $head] = $run(['REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=admin.overview', 'REMOTE_USER' => 'alice']);
        PkgSupport::assertContains('Status: 403', $head, 'a plain account is not a reseller');

        foreach (['GET' => null, 'POST' => null] as $method => $_) {
            [, $head, $body] = $run(['REQUEST_METHOD' => $method, 'QUERY_STRING' => 'a=admin.set_account', 'REMOTE_USER' => 'root', 'CONTENT_LENGTH' => '2'], '{}');
            PkgSupport::assertContains('Status: 405', $head, "$method without X-WebPulse-Request");
        }

        [, $head, $body] = $run(['REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=admin.overview', 'REMOTE_USER' => 'root']);
        PkgSupport::assertContains('Content-Type: application/json', $head, 'root reaches the API');
        PkgSupport::assertTrue(is_array(json_decode($body, true)), "JSON body expected: $body");

        [, $head] = $run(['REQUEST_METHOD' => 'GET', 'QUERY_STRING' => 'a=admin.overview', 'REMOTE_USER' => 'reseller1']);
        PkgSupport::assertNotContains('Status: 403', $head, 'reseller1 owns alice');
    },

    'pkg_frontends: index.cgi serves the dashboard to root and refuses strangers' => static function () use ($repo, $haveStack, $smokeRoot): void {
        if (!$haveStack()) {
            fwrite(STDERR, "SKIP engine/UI packages not complete yet\n");
            return;
        }
        [$cfg] = $smokeRoot();
        $env = ['WEBPULSE_HOME' => $repo, 'WEBPULSE_CONFIG' => $cfg];
        [$rc, $out] = PkgSupport::run([PHP_BINARY, "$repo/whm/cgi/index.cgi"], $env + ['REMOTE_USER' => 'root']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertTrue(str_starts_with($out, "Content-Type: text/html; charset=utf-8\r\n"), 'CGI header first');
        PkgSupport::assertContains("\r\n\r\n<!doctype html", $out, 'full document');
        PkgSupport::assertContains('api.cgi', $out, 'api url');
        PkgSupport::assertNotContains('#WHMADDON', $out, 'marker comments are not printed');

        [, $out] = PkgSupport::run([PHP_BINARY, "$repo/whm/cgi/index.cgi"], $env);
        PkgSupport::assertContains('Status: 403', $out, 'anonymous');
        PkgSupport::assertNotContains('<script', $out, 'no dashboard for strangers');
    },
];
