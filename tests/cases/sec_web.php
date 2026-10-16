<?php

declare(strict_types=1);

use WebPulse\Cli\Command\GeoipUpdateCommand;
use WebPulse\Dev\Router;
use WebPulse\Http\DomainResolver;
use WebPulse\Paths;
use WebPulse\Reports\Range;
use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\SeedDb;
use WebPulse\Tests\T;

/*
 * Security review L2, L5 and the INFO items — dev server access token and
 * Host check, admin.run concurrency, reseller access to global settings,
 * regex anchors, one rule for account names, and the GeoIP download
 * (size cap, no downgrade redirects, streamed to disk).
 */

require_once dirname(__DIR__) . '/support/seed_db.php';

return [
    'L2: dev router gate (token, cookie, header, Host)' => static function (): void {
        $tok = str_repeat('ab', 16);
        $srv = ['HTTP_HOST' => '127.0.0.1:8099', 'SERVER_PORT' => '8099'];
        T::eq(403, Router::gate('/', $srv, ['token' => $tok], [], '', '8099')[0], 'no token configured');
        T::eq([200, '', true], Router::gate('/', $srv, ['token' => $tok], [], $tok, '8099'), 'first load sets the cookie');
        T::eq(403, Router::gate('/api', $srv, ['token' => $tok], [], $tok, '8099')[0], 'query token only on the page');
        T::eq(200, Router::gate('/api', $srv, [], [Router::COOKIE => $tok], $tok, '8099')[0], 'cookie');
        T::eq(200, Router::gate('/api', $srv + ['HTTP_X_WEBPULSE_DEV_TOKEN' => $tok], [], [], $tok, '8099')[0], 'header');
        T::eq(403, Router::gate('/api', $srv, [], [Router::COOKIE => 'x'], $tok, '8099')[0], 'wrong cookie');
        T::eq(403, Router::gate('/api', $srv, [], [Router::COOKIE => [$tok]], $tok, '8099')[0], 'array cookie');
        T::eq(403, Router::gate('/', ['HTTP_HOST' => 'attacker.example:8099'] + $srv, ['token' => $tok], [], $tok, '8099')[0], 'DNS rebinding');
        T::eq(403, Router::gate('/', ['HTTP_HOST' => '127.0.0.1:9999'] + $srv, ['token' => $tok], [], $tok, '8099')[0], 'other port');
        T::eq(200, Router::gate('/', ['HTTP_HOST' => 'LOCALHOST:8099'] + $srv, [], [Router::COOKIE => $tok], $tok, '')[0], 'localhost, port from SERVER_PORT');
        T::eq(403, Router::gate('/', ['SERVER_PORT' => '8099'], [], [Router::COOKIE => $tok], $tok, '8099')[0], 'no Host header');
        // the serve command documents --allow-root (it refuses to start as root without it)
        $fr = FakeRoot::create('sec-web-serve');
        T::contains('--allow-root', $fr->run('serve --help')['stdout']);
    },
    'L5: admin.run refuses duplicates and more than 3 running jobs; returns only the job id' => static function (): void {
        $env = SeedDb::fresh('sec-web-run');
        $reg = \WebPulse\App::boot($env['config'])->registry();
        $now = $env['now'];
        $dup = $reg->startJob('collect', 'bob', $now - 60);
        $r = SeedDb::api($env, 'reseller:reseller1', ['a' => 'admin.run'], ['type' => 'collect', 'user' => 'bob']);
        T::eq(409, $r['status'], $r['body']);
        T::eq('conflict', $r['json']['error']['code']);
        T::contains("#$dup", $r['json']['error']['message']);
        $reg->startJob('backfill', 'alice', $now - 60);
        $reg->startJob('collect', 'alice', $now - 60);
        T::eq(409, SeedDb::api($env, 'admin', ['a' => 'admin.run'], ['type' => 'rollup'])['status'], 'three jobs running');
        // a job "running" for more than 6 hours is presumed dead and does not count
        foreach ($reg->runningJobs(0) as $j) {
            $reg->pdo()->prepare('UPDATE jobs SET started_at = ? WHERE id = ?')->execute([$now - 7 * 3600, $j['id']]);
        }
        T::eq([], $reg->runningJobs($now - 6 * 3600));
        $r = SeedDb::api($env, 'admin', ['a' => 'admin.run'], ['type' => 'rollup']);
        T::eq(200, $r['status'], $r['body']);
        T::eq(['job_id'], array_keys($r['json']['data']), 'no command line echoed');
        // atomic check at the registry level
        $reg->startJob('rollup', 'all', $now);
        T::throws(OverflowException::class, static fn () => $reg->startJobLimited('rollup', 'all', $now - 3600, 3), 'already running');
        $deadline = microtime(true) + 20;
        while (microtime(true) < $deadline && $reg->job($r['json']['data']['job_id'])['status'] === 'running') {
            usleep(200000);
        }
    },
    'INFO: resellers cannot read global settings' => static function (): void {
        $env = SeedDb::shared();
        T::eq(403, SeedDb::api($env, 'reseller:reseller1', ['a' => 'admin.settings'])['status']);
        T::eq(200, SeedDb::api($env, 'admin', ['a' => 'admin.settings'])['status']);
    },
    'INFO: anchors reject a trailing newline; one rule for account names' => static function (): void {
        T::false(Paths::isValidUser("alice\n"));
        T::false(Paths::isValidDomain("example.com\n"));
        T::false(DomainResolver::validDomain("example.com\n"));
        T::false(DomainResolver::validUser("alice\n"));
        T::false(DomainResolver::validUser('al.ice'), 'dots are not valid in cPanel account names (as in Paths)');
        T::true(DomainResolver::validUser('alice_2'));
        T::eq(Paths::USER_RE, DomainResolver::USER_RE);
        T::false(Range::isDate("2026-10-16\n"));
        T::true(Range::isDate('2026-10-16'));
        $env = SeedDb::shared();
        T::eq(404, SeedDb::api($env, 'admin', ['a' => 'overview', 'domain' => "example.com\n"])['status']);
    },
    'INFO: geoip-update caps the download, refuses downgrade redirects, streams to disk' => static function (): void {
        $fr = FakeRoot::create('sec-web-geoip');
        // redirect rules
        T::eq('https://cdn.example/x.gz', GeoipUpdateCommand::redirectTarget('https://a.example/y', 'https://cdn.example/x.gz'));
        T::null(GeoipUpdateCommand::redirectTarget('https://a.example/y', 'http://cdn.example/x.gz'), 'https -> http');
        T::null(GeoipUpdateCommand::redirectTarget('https://a.example/y', 'file:///etc/shadow'));
        T::eq('https://a.example/z.gz', GeoipUpdateCommand::redirectTarget('https://a.example/dir/y', '/z.gz'));
        T::eq('https://a.example/dir/z.gz', GeoipUpdateCommand::redirectTarget('https://a.example/dir/y', 'z.gz'));
        T::eq('https://b.example/z', GeoipUpdateCommand::redirectTarget('https://a.example/y', '//b.example/z'));
        T::null(GeoipUpdateCommand::redirectTarget('http://a.example/y', '/z.gz'), 'relative redirect of an http source');
        // local files: size cap
        $src = $fr->dir() . '/big.bin';
        file_put_contents($src, str_repeat('x', 300000));
        $dest = $fr->dir() . '/dest.bin';
        T::throws(RuntimeException::class, static fn () => GeoipUpdateCommand::fetchTo('file://' . $src, $dest, 100000), 'larger than');
        T::false(is_file($dest), 'partial download removed');
        GeoipUpdateCommand::fetchTo($src, $dest, 400000);
        T::eq(300000, filesize($dest));
        // HTTP: redirect to http refused, Content-Length / streamed size capped
        $port = 20000 + random_int(0, 2000);
        $router = $fr->dir() . '/router.php';
        file_put_contents($router, <<<'PHP'
            <?php
            $p = parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH);
            $port = $_SERVER['SERVER_PORT'];
            if ($p === '/redir-http') { header("Location: http://127.0.0.1:$port/ok", true, 302); return true; }
            if ($p === '/redir-rel') { header('Location: /ok', true, 301); return true; }
            if ($p === '/big') { header('Content-Length: 300000'); echo str_repeat('x', 300000); return true; }
            if ($p === '/stream') { for ($i = 0; $i < 30; $i++) { echo str_repeat('y', 10000); flush(); } return true; }
            if ($p === '/ok') { echo 'small body'; return true; }
            http_response_code(404);
            return true;
            PHP);
        $proc = proc_open([PHP_BINARY, '-S', "127.0.0.1:$port", $router], [0 => ['file', '/dev/null', 'r'], 1 => ['file', '/dev/null', 'w'], 2 => ['file', '/dev/null', 'w']], $pipes);
        try {
            $base = "http://127.0.0.1:$port";
            for ($i = 0; $i < 50 && @file_get_contents("$base/ok") === false; $i++) {
                usleep(100000);
            }
            GeoipUpdateCommand::fetchTo("$base/ok", $dest, 1000);
            T::eq('small body', file_get_contents($dest));
            T::throws(RuntimeException::class, static fn () => GeoipUpdateCommand::fetchTo("$base/redir-http", $dest, 1000));
            T::throws(RuntimeException::class, static fn () => GeoipUpdateCommand::fetchTo("$base/redir-rel", $dest, 1000));
            T::throws(RuntimeException::class, static fn () => GeoipUpdateCommand::fetchTo("$base/big", $dest, 100000));
            T::throws(RuntimeException::class, static fn () => GeoipUpdateCommand::fetchTo("$base/stream", $dest, 100000));
            T::throws(RuntimeException::class, static fn () => GeoipUpdateCommand::fetchTo("$base/missing", $dest, 1000), '404');
            $r = $fr->run("geoip-update --force --url $base/redir-http --json");
            T::eq(1, $r['code']);
            T::eq([], glob($fr->path('var/webpulse/geoip') . '/.*.tmp') ?: [], 'no temporary file left');
        } finally {
            proc_terminate($proc);
            proc_close($proc);
        }
    },
];
