<?php

declare(strict_types=1);

use WebPulse\Reports\Dimension;
use WebPulse\Tests\SeedDb;
use WebPulse\Tests\T;

/*
 * WP2 — every API action against the seeded fixture (shape checks against
 * docs/API.md), validation errors and CSV export.
 */

require_once dirname(__DIR__) . '/support/seed_db.php';

$ok = static function (array $r, string $what = ''): array {
    T::eq(200, $r['status'], "$what: HTTP status (" . substr($r['body'], 0, 300) . ')');
    T::true($r['json']['ok'] ?? null, "$what: ok flag");
    T::true(is_int($r['json']['meta']['took_ms'] ?? null), "$what: meta.took_ms");
    return $r['json']['data'];
};
$err = static function (array $r, int $status, string $code, string $what = ''): void {
    T::eq($status, $r['status'], "$what: status (" . substr($r['body'], 0, 200) . ')');
    T::eq(false, $r['json']['ok'] ?? null, "$what: ok=false");
    T::eq($code, $r['json']['error']['code'] ?? null, "$what: error code");
    T::true(is_string($r['json']['error']['message'] ?? null), "$what: message");
};
$keys = static function (array $expected, array $actual, string $what): void {
    foreach ($expected as $k) {
        T::true(array_key_exists($k, $actual), "$what: missing key '$k' in " . implode(',', array_keys($actual)));
    }
};
$q = static fn (array $extra = []) => array_merge(['domain' => 'example.com'], $extra);

return [
    'domains' => function () use ($ok, $keys) {
        $env = SeedDb::shared();
        $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'domains']), 'domains');
        T::eq(['example.com', 'shop.example.com'], array_column($d['domains'], 'domain'));
        $row = $d['domains'][0];
        $keys(['domain', 'user', 'kind', 'enabled', 'has_data', 'first_day', 'last_day', 'last_collect_at', 'visits_today',
            'pageviews_today', 'timezone', 'db_bytes'], $row, 'domain row');
        T::eq('alice', $row['user']);
        T::true($row['has_data']);
        T::eq('Europe/Rome', $row['timezone']);
        T::true(is_int($row['visits_today']) && $row['visits_today'] > 0, 'visits_today is an int > 0');
        T::true(is_int($row['db_bytes']) && $row['db_bytes'] > 0);
        T::true((bool) preg_match('/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d$/', $row['last_collect_at']), 'ISO-8601 with offset');
    },
    'overview shape and totals' => function () use ($ok, $keys, $q) {
        $env = SeedDb::shared();
        $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'overview', 'domain' => 'example.com', 'range' => '30d', 'compare' => '1']), 'overview');
        $keys(['range', 'kpi', 'series', 'hours', 'weekdays', 'heatmap', 'meta'], $d, 'overview');
        T::eq(['from' => '2026-09-17', 'to' => '2026-10-16', 'days' => 30, 'granularity' => 'day', 'prev' => ['from' => '2026-08-18', 'to' => '2026-09-16']], $d['range']);
        foreach (['visits', 'visitors', 'pageviews', 'pages_per_visit', 'avg_duration', 'bounce_rate', 'bytes', 'bot_share', 'downloads', 'errors_4xx', 'errors_5xx'] as $k) {
            $keys(['value', 'prev', 'change'], $d['kpi'][$k], "kpi.$k");
            T::true(is_int($d['kpi'][$k]['value']) || is_float($d['kpi'][$k]['value']), "kpi.$k numeric");
        }
        $k = $d['kpi']['visits'];
        T::eq(round(($k['value'] - $k['prev']) / $k['prev'], 4), $k['change']);
        T::eq(30, count($d['series']['labels']));
        foreach (['visits', 'visitors', 'pageviews', 'bytes', 'bot_hits', 'bounces', 'duration', 'errors_4xx', 'errors_5xx'] as $s) {
            T::eq(30, count($d['series'][$s]), "series.$s length");
        }
        T::eq($k['value'], array_sum($d['series']['visits']), 'series sums to the KPI');
        T::eq(30, count($d['series']['prev']['visits']));
        T::eq(24, count($d['hours']['visits']));
        T::eq(7, count($d['weekdays']['visits']));
        T::eq(7, count($d['heatmap']));
        T::eq(24, count($d['heatmap'][0]));
        T::eq($k['value'], array_sum($d['hours']['visits']), 'hourly visits = daily visits');
        T::eq($k['value'], array_sum(array_map('array_sum', $d['heatmap'])));
        T::eq($k['value'], array_sum($d['weekdays']['visits']));
        $keys(['domain', 'timezone', 'first_day', 'last_day', 'last_collect_at', 'tz_changed_at', 'raw_retention_days'], $d['meta'], 'meta');
        T::eq(30, $d['meta']['raw_retention_days']);
        // without compare: no prev series
        $d2 = $ok(SeedDb::api($env, 'user:alice', ['a' => 'overview'] + $q()), 'overview');
        T::false(isset($d2['series']['prev']));
    },
    'overview hour granularity and week bucketing' => function () use ($ok, $q) {
        $env = SeedDb::shared();
        $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'overview', 'range' => 'today'] + $q()), 'today');
        T::eq('hour', $d['range']['granularity']);
        T::eq(24, count($d['series']['labels']));
        T::eq($d['kpi']['visits']['value'], array_sum($d['series']['visits']));
        T::null($d['series']['bounces'], 'hourly table has no bounces');
        $w = $ok(SeedDb::api($env, 'user:alice', ['a' => 'overview', 'range' => '12m'] + $q()), '12m');
        T::eq('week', $w['range']['granularity']);
        T::eq($w['kpi']['pageviews']['value'], array_sum($w['series']['pageviews']), 'weekly buckets keep the total');
    },
    'report: every dimension' => function () use ($ok, $keys, $q) {
        $env = SeedDb::shared();
        foreach (Dimension::ALL as $dim) {
            $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => $dim, 'limit' => '5'] + $q()), "report $dim");
            $keys(['name', 'total', 'rows', 'has_more'], $d, $dim);
            T::eq($dim, $d['name']);
            $keys(['visits', 'pageviews', 'hits', 'bytes', 'rows'], $d['total'], "$dim total");
            T::true(count($d['rows']) <= 5, "$dim limit");
            foreach ($d['rows'] as $row) {
                $keys(['key', 'visits', 'pageviews', 'hits', 'bytes', 'bounces', 'duration', 'share'], $row, "$dim row");
                T::true(is_string($row['key']) && is_int($row['visits']), "$dim types");
            }
        }
    },
    'report: page exit rate, compare, sort, q, paging' => function () use ($ok, $q) {
        $env = SeedDb::shared();
        $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'compare' => '1', 'limit' => '3'] + $q()), 'page');
        T::eq('/', $d['rows'][0]['key']);
        T::true($d['has_more']);
        T::true(isset($d['rows'][0]['exit_rate'], $d['rows'][0]['prev']['pageviews']), 'exit_rate and prev');
        T::true($d['rows'][0]['pageviews'] >= $d['rows'][1]['pageviews'], 'default sort -pageviews');
        T::eq(round($d['rows'][0]['pageviews'] / $d['total']['pageviews'], 4), $d['rows'][0]['share']);
        $asc = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'sort' => 'pageviews', 'limit' => '2'] + $q()));
        T::true($asc['rows'][0]['pageviews'] <= $asc['rows'][1]['pageviews'], 'ascending sort');
        $f = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'q' => 'BLOG'] + $q()));
        T::true(count($f['rows']) >= 2);
        foreach ($f['rows'] as $row) {
            T::contains('blog', $row['key']);
        }
        T::eq(count($f['rows']), $f['total']['rows']);
        $like = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'q' => '%'] + $q()));
        T::eq(0, count($like['rows']), 'LIKE wildcards are escaped');
        $p1 = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'limit' => '2', 'offset' => '2'] + $q()));
        $all = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'limit' => '4'] + $q()));
        T::eq(array_slice(array_column($all['rows'], 'key'), 2, 2), array_column($p1['rows'], 'key'), 'offset');
        $big = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'limit' => '100000'] + $q()));
        T::eq(14, count($big['rows']), 'limit is capped, not rejected');
    },
    'report: labels for countries, status, buckets; bucket order' => function () use ($ok, $q) {
        $env = SeedDb::shared();
        $c = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'country'] + $q()));
        T::eq(['it', 'Italy'], [$c['rows'][0]['key'], $c['rows'][0]['label']]);
        T::true(isset($c['rows'][0]['bounce_rate'], $c['rows'][0]['avg_duration']));
        $s = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'status', 'sort' => 'key', 'limit' => '50'] + $q()));
        $byKey = array_column($s['rows'], 'label', 'key');
        T::eq('404 Not Found', $byKey['404']);
        $b = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'visit_duration', 'limit' => '20'] + $q()));
        T::eq(Dimension::DURATION_BUCKETS, array_column($b['rows'], 'key'), 'bucket order');
        $p = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'pages_per_visit', 'limit' => '20'] + $q()));
        T::eq(Dimension::PAGES_BUCKETS, array_column($p['rows'], 'key'));
        T::eq('6–10 pages', $p['rows'][5]['label']);
    },
    'report: series=1 gives top 7 + other per day' => function () use ($ok, $q) {
        $env = SeedDb::shared();
        $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'country', 'series' => '1'] + $q()));
        $se = $d['series'];
        T::eq(30, count($se['labels']));
        T::eq(8, count($se['keys']), '7 keys + (other)');
        T::eq('(other)', $se['keys'][7]);
        T::eq(count($se['keys']), count($se['values']));
        T::eq('visits', $se['metric']);
        $sum = 0;
        foreach ($se['values'] as $v) {
            T::eq(30, count($v));
            $sum += array_sum($v);
        }
        T::eq($d['total']['visits'], $sum, 'series covers every visit');
    },
    'realtime' => function () use ($ok, $keys, $q) {
        $env = SeedDb::shared();
        $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'realtime'] + $q()), 'realtime');
        $keys(['active_visits', 'pageviews_30m', 'per_minute', 'recent'], $d, 'realtime');
        T::eq(6, $d['active_visits']);
        T::eq(30, count($d['per_minute']));
        T::eq($d['pageviews_30m'], array_sum($d['per_minute']));
        T::true(count($d['recent']) > 0);
        $keys(['ts', 'path', 'status', 'type', 'country', 'device', 'browser', 'referrer', 'visit_pageviews'], $d['recent'][0], 'recent');
        foreach ($d['recent'] as $e) {
            T::true($e['type'] !== 'bot', 'bots are not listed');
        }
    },
    'visits_log with cursor paging' => function () use ($ok, $keys, $q) {
        $env = SeedDb::shared();
        $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'visits_log', 'range' => '7d', 'limit' => '10'] + $q()), 'visits_log');
        T::eq(10, count($d['visits']));
        $v = $d['visits'][0];
        $keys(['visit_id', 'start', 'end', 'duration', 'pageviews', 'bytes', 'entry_page', 'exit_page', 'channel', 'referrer_host',
            'search_engine', 'keyword', 'campaign', 'country', 'city', 'browser', 'os', 'device', 'actions'], $v, 'visit');
        T::true($v['live'], 'open visits come first');
        T::false(isset($v['ip']), 'no IP unless stored');
        $keys(['ts', 'type', 'path', 'status', 'bytes'], $v['actions'][0], 'action');
        T::true(is_int($d['next_before']));
        $n = $ok(SeedDb::api($env, 'user:alice', ['a' => 'visits_log', 'range' => '7d', 'limit' => '10', 'before' => (string) $d['next_before'], 'before_id' => (string) $d['next_before_id']] + $q()));
        $ids = array_merge(array_column($d['visits'], 'visit_id'), array_column($n['visits'], 'visit_id'));
        T::eq(count($ids), count(array_unique($ids)), 'pages do not overlap');
        T::true(strcmp($n['visits'][0]['start'], $d['visits'][9]['start']) <= 0, 'older on the next page');
        $cap = $ok(SeedDb::api($env, 'user:alice', ['a' => 'visits_log', 'limit' => '1000'] + $q()));
        T::true(count($cap['visits']) <= 100, 'limit ≤ 100');
    },
    'health' => function () use ($ok, $keys, $q) {
        $env = SeedDb::shared();
        $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'health'] + $q()), 'health');
        $keys(['status', 'errors_series', 'not_found', 'server_error', 'file_type', 'static_file', 'download', 'protocol', 'http_version', 'method', 'peak'], $d, 'health');
        $keys(['labels', 'errors_4xx', 'errors_5xx'], $d['errors_series'], 'errors_series');
        $old = array_values(array_filter($d['not_found'], static fn ($r) => $r['key'] === '/old-pricing'))[0];
        T::eq(['bing.com', 'oldforum.example.net'], array_column($old['referrers'], 'key'), '404 referrers');
        $keys(['day', 'hour', 'hits'], $d['peak'], 'peak');
        T::eq('/assets/hero-2400.jpg', $d['static_file'][0]['key'], 'static files by bandwidth');
    },
    'export CSV' => function () use ($q) {
        $env = SeedDb::shared();
        $r = SeedDb::api($env, 'user:alice', ['a' => 'export', 'name' => 'page', 'format' => 'csv', 'range' => '30d'] + $q());
        T::eq(200, $r['status']);
        $res = $r['response'];
        T::eq('text/csv; charset=utf-8', $res->contentType);
        T::eq('attachment; filename="example.com-page-2026-09-17_2026-10-16.csv"', $res->headers['Content-Disposition']);
        $lines = array_values(array_filter(explode("\n", $r['body'])));
        T::eq('key,pageviews,visits,hits,bytes,exit_rate,share', $lines[0]);
        T::eq(15, count($lines), 'header + every row (no limit)');
        T::contains('/,', $lines[1]);
        $c = SeedDb::api($env, 'user:alice', ['a' => 'export', 'name' => 'country'] + $q());
        T::contains('key,label,visits', $c['body']);
        T::contains('"United States"', $c['body']);
        T::eq("'=cmd()", WebPulse\Reports\Export::safe('=cmd()'), 'formula injection neutralised');
        T::eq('/ok', WebPulse\Reports\Export::safe('/ok'));
        $bad = SeedDb::api($env, 'user:alice', ['a' => 'export', 'name' => 'page', 'format' => 'xlsx'] + $q());
        T::eq(400, $bad['status']);
    },
    'validation errors' => function () use ($err, $q) {
        $env = SeedDb::shared();
        $err(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'nope'] + $q()), 400, 'bad_request', 'bad dim');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page; DROP TABLE daily'] + $q()), 400, 'bad_request', 'sql in dim');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'sort' => 'key DESC; --'] + $q()), 400, 'bad_request', 'bad sort');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'report', 'name' => 'page', 'limit' => 'ten'] + $q()), 400, 'bad_request', 'bad limit');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'overview', 'range' => 'forever'] + $q()), 400, 'bad_request', 'bad range');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'overview', 'from' => '2026-13-01', 'to' => '2026-12-01'] + $q()), 400, 'bad_request', 'bad date');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'visits_log', 'before' => '-5'] + $q()), 400, 'bad_request', 'bad cursor');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'overview']), 400, 'bad_request', 'missing domain');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'teleport']), 400, 'bad_request', 'unknown action');
        $err(SeedDb::api($env, 'user:alice', []), 400, 'bad_request', 'no action');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'overview'] + $q(), null, 'POST'), 405, 'method_not_allowed', 'POST on a read');
    },
    'mutations need POST and the X-WebPulse-Request header' => function () use ($err) {
        $env = SeedDb::fresh('ui-mut');
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.set_account'], null, 'GET'), 405, 'method_not_allowed', 'GET');
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.set_account'], ['user' => 'bob', 'enabled' => false], 'POST', []), 403, 'forbidden', 'no header');
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.set_account'], ['user' => 'bob', 'enabled' => 'yes']), 400, 'bad_request', 'bad enabled');
        $r = SeedDb::api($env, 'admin', ['a' => 'admin.set_account'], ['user' => 'bob', 'enabled' => false]);
        T::eq(200, $r['status'], $r['body']);
        T::eq(false, $r['json']['data']['account']['enabled']);
        T::eq(false, $r['json']['data']['account']['enabled_override']);
    },
    'user settings: read, validate, atomic save' => function () use ($ok, $err) {
        $env = SeedDb::fresh('ui-settings');
        $d = $ok(SeedDb::api($env, 'user:alice', ['a' => 'user_settings']), 'user_settings');
        T::eq('203.0.113.9', $d['remote_addr']);
        T::eq(['example.com', 'shop.example.com'], $d['domains']);
        $new = ['domains' => ['example.com' => ['timezone' => 'Europe/Berlin', 'exclude_ips' => ['203.0.113.0/24'], 'exclude_paths' => ['/staging/']]],
            'ui' => ['default_range' => '7d', 'dark' => 'dark']];
        $s = $ok(SeedDb::api($env, 'user:alice', ['a' => 'save_user_settings'], ['settings' => $new]), 'save');
        T::true($s['saved']);
        $file = $env['root'] . '/var/webpulse/data/alice/user/settings.json';
        T::true(is_file($file));
        $disk = json_decode((string) file_get_contents($file), true);
        T::eq('Europe/Berlin', $disk['domains']['example.com']['timezone']);
        T::eq([], glob(dirname($file) . '/.settings-*') ?: [], 'no temp files left');
        $back = $ok(SeedDb::api($env, 'user:alice', ['a' => 'user_settings']));
        T::eq(['203.0.113.0/24'], $back['settings']['domains']['example.com']['exclude_ips']);
        $err(SeedDb::api($env, 'user:alice', ['a' => 'save_user_settings'], ['domains' => ['example.com' => ['timezone' => 'Mars/Olympus']]]), 400, 'bad_request', 'bad tz');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'save_user_settings'], ['domains' => ['example.com' => ['exclude_ips' => ['not-an-ip']]]]), 400, 'bad_request', 'bad ip');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'save_user_settings'], ['domains' => ['blog.net' => []]]), 400, 'bad_request', 'foreign domain');
        $err(SeedDb::api($env, 'admin', ['a' => 'save_user_settings'], ['domains' => []]), 403, 'forbidden', 'admin cannot save');
        $err(SeedDb::api($env, 'admin', ['a' => 'user_settings']), 403, 'forbidden', 'admin cannot read');
        $err(SeedDb::api($env, 'user:alice', ['a' => 'save_user_settings'], null, 'POST'), 400, 'bad_request', 'empty body is validated');
    },
    'admin actions' => function () use ($ok, $keys, $err) {
        $env = SeedDb::fresh('ui-admin');
        $o = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.overview']), 'admin.overview');
        $keys(['totals', 'top_domains', 'disk', 'jobs', 'collector', 'bot_share_30d'], $o, 'admin.overview');
        $keys(['last_collect_at', 'lag_seconds', 'cron_installed', 'hook_installed', 'geoip'], $o['collector'], 'collector');
        T::eq(['example.com', 'blog.net', 'shop.example.com'], array_column($o['top_domains'], 'domain'));
        T::true($o['bot_share_30d'] > 0.2 && $o['bot_share_30d'] < 0.6, 'bot share plausible');
        $a = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.accounts']), 'admin.accounts');
        T::eq(['alice', 'bob', 'carol', 'dave'], array_column($a['accounts'], 'user'));
        $alice = $a['accounts'][0];
        $keys(['user', 'owner', 'enabled', 'enabled_override', 'domains'], $alice, 'account');
        $keys(['domain', 'kind', 'enabled', 'has_data', 'db_bytes', 'visits_30d', 'last_collect_at'], $alice['domains'][0], 'account domain');
        T::eq(false, $a['accounts'][3]['enabled'], 'dave is disabled');
        $f = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.accounts', 'q' => 'blog']));
        T::eq(['bob'], array_column($f['accounts'], 'user'), 'q filter matches domains');
        $sd = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.set_domain'], ['user' => 'alice', 'domain' => 'shop.example.com', 'enabled' => false]));
        T::eq(false, $sd['domain']['enabled']);
        $s = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.settings']));
        $keys(['default_enabled', 'raw_retention_days', 'max_rows_per_dim', 'session_timeout_min', 'store_ip', 'exclude_ips', 'exclude_paths', 'geoip.enabled', 'timezone'], $s['settings'], 'settings');
        $saved = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.save_settings'], ['settings' => ['raw_retention_days' => 60, 'exclude_ips' => ['10.0.0.0/8'], 'geoip' => ['enabled' => true]]]));
        T::eq(60, $saved['settings']['raw_retention_days']);
        T::eq(true, $saved['settings']['geoip.enabled']);
        T::true(in_array('raw_retention_days', $saved['overridden'], true));
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.save_settings'], ['settings' => ['raw_retention_days' => 0]]), 400, 'bad_request', 'out of range');
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.save_settings'], ['settings' => ['paths.data_dir' => '/tmp']]), 400, 'bad_request', 'not editable');
        $j = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.jobs']));
        T::eq(13, count($j['jobs']));
        $keys(['id', 'type', 'scope', 'started_at', 'finished_at', 'status', 'lines'], $j['jobs'][0], 'job');
    },
    'admin.run spawns a detached job and returns its id' => function () use ($ok, $err) {
        $env = SeedDb::fresh('ui-run');
        $r = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.run'], ['type' => 'rollup']), 'admin.run');
        T::true(is_int($r['job_id']) && $r['job_id'] > 13, 'job id');
        T::eq(['job_id'], array_keys($r), 'only the job id is returned (no command line)');
        $deadline = microtime(true) + 20;
        $status = 'running';
        while (microtime(true) < $deadline) {
            $jobs = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.jobs']))['jobs'];
            $status = array_values(array_filter($jobs, static fn ($j) => $j['id'] === $r['job_id']))[0]['status'] ?? 'missing';
            if ($status !== 'running') {
                break;
            }
            usleep(200000);
        }
        T::true(in_array($status, ['ok', 'skipped'], true), "job finished (status $status)");
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.run'], ['type' => 'rm -rf']), 400, 'bad_request', 'bad type');
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.run'], ['type' => 'backfill']), 400, 'bad_request', 'backfill needs user');
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.run'], ['type' => 'collect', 'user' => '../etc']), 404, 'not_found', 'bad user');
    },
    'admin.purge needs DELETE confirmation' => function () use ($ok, $err) {
        $env = SeedDb::fresh('ui-purge');
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.purge'], ['user' => 'alice', 'domain' => 'shop.example.com']), 400, 'bad_request', 'no confirm');
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.purge'], ['user' => 'alice', 'domain' => 'shop.example.com', 'confirm' => 'delete']), 400, 'bad_request', 'wrong case');
        $err(SeedDb::api($env, 'admin', ['a' => 'admin.purge'], ['user' => 'alice', 'domain' => '../bob/blog.net', 'confirm' => 'DELETE']), 400, 'bad_request', 'traversal');
        $d = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.purge'], ['user' => 'alice', 'domain' => 'shop.example.com', 'confirm' => 'DELETE']));
        T::eq(['shop.example.com'], $d['deleted']);
        T::false(is_file($env['root'] . '/var/webpulse/data/alice/shop.example.com.db'));
        T::true(is_file($env['root'] . '/var/webpulse/data/alice/example.com.db'), 'other domain kept');
        $d = $ok(SeedDb::api($env, 'admin', ['a' => 'admin.purge'], ['user' => 'bob', 'confirm' => 'DELETE']));
        T::eq(['blog.net'], $d['deleted']);
    },
    'no-data and schema states' => function () use ($err) {
        $env = SeedDb::fresh('ui-states');
        $err(SeedDb::api($env, 'admin', ['a' => 'overview', 'domain' => 'carol.org', 'user' => 'carol']), 404, 'no_data', 'registered domain without data');
        $pdo = new PDO('sqlite:' . $env['root'] . '/var/webpulse/data/alice/example.com.db');
        $pdo->exec("UPDATE meta SET value = '999' WHERE key = 'schema_version'");
        $pdo = null;
        $err(SeedDb::api($env, 'user:alice', ['a' => 'overview', 'domain' => 'example.com']), 503, 'schema_too_new', 'newer schema');
    },
];
