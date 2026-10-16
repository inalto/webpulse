<?php

declare(strict_types=1);

use WebPulse\Reports\Dimension;
use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\SeedDb;
use WebPulse\Tests\T;

/*
 * WP2 × WP1 integration — the API on databases written by the real engine
 * (`bin/webpulse collect` on generated logs), and the CLI commands of WP2.
 */

require_once dirname(__DIR__) . '/support/seed_db.php';

$engine = static function (): array {
    static $env = null;
    if ($env !== null) {
        return $env;
    }
    $fr = FakeRoot::create('ui-engine');
    $now = strtotime('2026-10-16 18:00:00 Europe/Rome');
    $fr->setNow($now);
    $fr->generateLogs(['seed' => 7, 'days' => 10, 'lines' => 12000, 'end' => '2026-10-16', 'max-ts' => $now]);
    foreach (['setup --quiet', 'collect --quiet'] as $cmd) {
        $r = $fr->run($cmd);
        if ($r['code'] !== 0) {
            throw new RuntimeException("webpulse $cmd failed: {$r['stderr']}{$r['stdout']}");
        }
    }
    return $env = ['config' => $fr->configPath(), 'root' => $fr->root(), 'now' => $now, 'totals' => [], 'fr' => $fr];
};
$data = static function (array $r, string $what) {
    T::eq(200, $r['status'], "$what: " . substr($r['body'], 0, 300));
    return $r['json']['data'];
};

return [
    'dimension names agree with the engine' => function () {
        T::eq(WebPulse\Store\Dims::ALL, Dimension::ALL, 'Store\Dims::ALL == Reports\Dimension::ALL');
        T::eq(WebPulse\Store\Dims::NF_REF_SEP, Dimension::NF_REF_SEP);
        T::eq(WebPulse\Store\Dims::DURATION_BUCKETS, Dimension::DURATION_BUCKETS);
        T::eq(WebPulse\Store\Dims::PAGES_BUCKETS, Dimension::PAGES_BUCKETS);
    },
    'API on an engine-built database' => function () use ($engine, $data) {
        $env = $engine();
        $pdo = $env['fr']->db('alice', 'example.com');
        foreach ($pdo->query('SELECT DISTINCT dim FROM dim_daily')->fetchAll(PDO::FETCH_COLUMN) as $dim) {
            T::true(Dimension::isValid($dim), "engine wrote unknown dim '$dim'");
        }
        $sum = $pdo->query("SELECT SUM(visits) v, SUM(pageviews) p, SUM(bot_hits) b FROM daily WHERE day BETWEEN '2026-10-10' AND '2026-10-16'")->fetch(PDO::FETCH_ASSOC);

        $q = ['domain' => 'example.com', 'range' => '7d'];
        $ov = $data(SeedDb::api($env, 'user:alice', ['a' => 'overview', 'compare' => '1'] + $q), 'overview');
        T::eq('2026-10-10', $ov['range']['from']);
        T::eq((int) $sum['v'], $ov['kpi']['visits']['value']);
        T::eq((int) $sum['p'], $ov['kpi']['pageviews']['value']);
        T::greater(0, $ov['kpi']['visits']['value']);
        T::eq($ov['kpi']['visits']['value'], array_sum($ov['series']['visits']));
        T::eq($ov['kpi']['visits']['value'], array_sum($ov['hours']['visits']), 'hourly visits agree with daily');

        $page = $data(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'limit' => '500'] + $q), 'page');
        T::eq($ov['kpi']['pageviews']['value'], $page['total']['pageviews'], 'page dim covers every pageview');
        foreach ($page['rows'] as $row) {
            T::true($row['exit_rate'] >= 0 && $row['exit_rate'] <= 1, 'exit rate in [0,1] for ' . $row['key']);
        }
        $entry = $data(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'entry_page', 'limit' => '500'] + $q), 'entry');
        // visits made only of non-page hits (a 404, a download) have no entry page (METRICS.md)
        T::true($entry['total']['visits'] <= $ov['kpi']['visits']['value'] && $entry['total']['visits'] > 0.9 * $ov['kpi']['visits']['value'], 'about one entry per visit');
        $ch = $data(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'channel', 'series' => '1'] + $q), 'channel');
        T::eq($ov['kpi']['visits']['value'], $ch['total']['visits'], 'channels cover every visit');
        foreach ($ch['rows'] as $row) {
            T::true(in_array($row['key'], ['direct', 'organic', 'social', 'referral', 'paid', 'email', 'campaign'], true), 'channel ' . $row['key']);
        }
        $h = $data(SeedDb::api($env, 'user:alice', ['a' => 'health'] + $q), 'health');
        T::greater(0, count($h['status']));
        $bots = $data(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'bot', 'limit' => '500'] + $q), 'bot');
        T::eq((int) $sum['b'], $bots['total']['hits'], 'bot dim covers every bot hit');
        $vl = $data(SeedDb::api($env, 'user:alice', ['a' => 'visits_log', 'limit' => '5'] + $q), 'visits_log');
        T::greater(0, count($vl['visits']));
        T::greater(0, count($vl['visits'][0]['actions']));
        $data(SeedDb::api($env, 'user:alice', ['a' => 'realtime', 'domain' => 'example.com']), 'realtime');
        $csv = SeedDb::api($env, 'user:alice', ['a' => 'export', 'name' => 'referrer_host'] + $q);
        T::eq(200, $csv['status']);
        $d = $data(SeedDb::api($env, 'admin', ['a' => 'admin.overview']), 'admin.overview');
        T::greater(0, $d['totals']['30d']['visits']);
    },
    'webpulse report (CLI)' => function () use ($engine) {
        $env = $engine();
        $r = $env['fr']->run('report --user alice --domain example.com --range 7d');
        T::eq(0, $r['code'], $r['stderr']);
        T::contains('example.com  2026-10-10 → 2026-10-16', $r['stdout']);
        T::contains('bounce_rate', $r['stdout']);
        $j = $env['fr']->run('report --user alice --domain example.com --name page --limit 3 --json');
        T::eq(0, $j['code'], $j['stderr']);
        T::eq('page', $j['json']['name']);
        T::true(count($j['json']['rows']) <= 3);
        $bad = $env['fr']->run('report --user alice --domain example.com --name nope');
        T::eq(2, $bad['code'], 'usage error for a bad dimension');
        $missing = $env['fr']->run('report --user alice --domain nosuch.example');
        T::eq(1, $missing['code']);
    },
    'webpulse serve (principal options)' => function () {
        $spec = [WebPulse\Cli\Command\ServeCommand::class, 'principalSpec'];
        T::eq('user:alice', $spec('alice', false, null));
        T::eq('admin', $spec(null, true, null));
        T::eq('admin', $spec(null, false, null));
        T::eq('reseller:res1', $spec(null, false, 'res1'));
        T::throws(WebPulse\Cli\UsageException::class, fn () => $spec('alice', true, null));
        T::throws(WebPulse\Cli\UsageException::class, fn () => $spec('../x', false, null));
    },
    'dev router serves page and API over HTTP' => function () {
        $env = SeedDb::shared();
        $port = 18000 + random_int(0, 2000);
        $repo = dirname(__DIR__, 2);
        $envv = getenv();
        $envv['WEBPULSE_CONFIG'] = $env['config'];
        $envv['WEBPULSE_DEV_PRINCIPAL'] = 'user:alice';
        $envv['WEBPULSE_NOW'] = (string) $env['now'];
        $token = bin2hex(random_bytes(16));
        $envv['WEBPULSE_DEV_TOKEN'] = $token;
        $envv['WEBPULSE_DEV_PORT'] = (string) $port;
        $proc = proc_open([PHP_BINARY, '-S', "127.0.0.1:$port", $repo . '/lib/WebPulse/Dev/serve_router.php'],
            [0 => ['file', '/dev/null', 'r'], 1 => ['file', '/dev/null', 'w'], 2 => ['file', '/dev/null', 'w']], $pipes, $repo, $envv);
        try {
            $get = static function (string $path, array $headers = []) use ($port): array {
                for ($i = 0; $i < 50; $i++) {
                    $ctx = stream_context_create(['http' => ['ignore_errors' => true, 'timeout' => 5, 'follow_location' => 0,
                        'header' => implode("\r\n", $headers)]]);
                    $body = @file_get_contents("http://127.0.0.1:$port$path", false, $ctx);
                    if ($body !== false) {
                        return [$http_response_header[0] ?? '', $body, $http_response_header];
                    }
                    usleep(100000);
                }
                throw new RuntimeException('dev server did not start');
            };
            $cookie = 'Cookie: webpulse_dev=' . $token;
            // no token: refused everywhere
            T::contains('403', $get('/')[0]);
            T::contains('403', $get('/api?a=domains')[0]);
            T::contains('403', $get('/?token=' . str_repeat('0', 32))[0], 'wrong token');
            // the token in the URL sets the cookie and redirects to /
            [$st, , $hdrs] = $get('/?token=' . $token);
            T::contains('303', $st);
            $set = implode("\n", array_filter($hdrs, static fn ($h) => stripos($h, 'Set-Cookie:') === 0));
            T::contains("webpulse_dev=$token", $set);
            T::contains('SameSite=Strict', $set);
            T::contains('HttpOnly', $set);
            [$st, $html] = $get('/', [$cookie]);
            T::contains('200', $st);
            T::contains('window.WEBPULSE', $html);
            T::contains('403', $get('/api?a=domains', ["Cookie: webpulse_dev=x$token"])[0], 'wrong cookie');
            [$st, $json] = $get('/api?a=report&name=page&domain=example.com&limit=2', [$cookie]);
            T::contains('200', $st);
            T::eq('page', json_decode($json, true)['data']['name']);
            [$st] = $get('/api?a=domains', ['X-WebPulse-Dev-Token: ' . $token]);
            T::contains('200', $st, 'header instead of the cookie');
            // DNS rebinding: a foreign Host header is refused even with the token
            T::contains('403', $get('/api?a=domains', [$cookie, "Host: evil.example:$port"])[0], 'foreign Host');
            [$st] = $get('/api?a=domains', [$cookie, "Host: localhost:$port"]);
            T::contains('200', $st, 'localhost Host');
            [$st] = $get('/api?a=overview&domain=blog.net', [$cookie]);
            T::contains('404', $st);
            [$st] = $get('/../config.json', [$cookie]);
            T::contains('404', $st);
        } finally {
            proc_terminate($proc);
            proc_close($proc);
        }
    },
];
