<?php

declare(strict_types=1);

use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\Lines as L;
use WebPulse\Tests\Q;
use WebPulse\Tests\T;

const SSL = 'etc/apache2/logs/domlogs/alice/example.com-ssl_log';
const PLAIN = 'etc/apache2/logs/domlogs/alice/example.com';

/** Fresh fake root, lines written to the ssl log, collected at $now. */
$scenario = static function (string $name, array $lines, string $now, array $config = []): array {
    $fr = FakeRoot::create('sess-' . $name, $config);
    $fr->writeLog(SSL, $lines);
    $fr->setNow(L::ts($now));
    $r = $fr->run('collect --json');
    T::eq(0, $r['code'], $r['stderr'] . $r['stdout']);
    return [$fr, $fr->db('alice', 'example.com'), $r['json']];
};
$collect = static function (FakeRoot $fr, string $now): array {
    $fr->setNow(L::ts($now));
    $r = $fr->run('collect --json');
    T::eq(0, $r['code'], $r['stderr'] . $r['stdout']);
    return $r['json'];
};

return [
    'visits, bounces, duration, entry and exit pages' => static function () use ($scenario): void {
        [$fr, $db] = $scenario('basic', [
            L::line('203.0.113.1', '2026-10-10 10:00:00', '/', 200, 1000, 'https://www.google.com/'),
            L::line('203.0.113.1', '2026-10-10 10:00:01', '/assets/app.css', 200, 500, 'https://example.com/'),
            L::line('203.0.113.1', '2026-10-10 10:01:00', '/pricing', 200, 2000, 'https://example.com/'),
            L::line('203.0.113.1', '2026-10-10 10:03:00', '/contact', 200, 3000, 'https://example.com/pricing'),
            L::line('198.51.100.2', '2026-10-10 10:05:00', '/', 200, 1000, null, L::IPHONE),
            L::line('66.249.66.1', '2026-10-10 10:06:00', '/', 200, 1500, null, L::GOOGLEBOT),
        ], '2026-10-10 12:00:00');
        $d = Q::daily($db, '2026-10-10');
        T::eq(2, $d['visits']);
        T::eq(2, $d['visitors']);
        T::eq(4, $d['pageviews']);
        T::eq(5, $d['hits']);
        T::eq(7500, $d['bytes']);
        T::eq(1, $d['bounces']);
        T::eq(180, $d['duration']);
        T::eq(1, $d['bot_hits']);
        T::eq(1500, $d['bot_bytes']);
        T::eq(['/' => 2], Q::col($db, 'entry_page', 'visits'));
        T::eq(['/' => 1], Q::col($db, 'entry_page', 'bounces'));
        T::eq(['/' => 1, '/contact' => 1], Q::col($db, 'exit_page', 'visits'));
        T::eq(['/' => 2, '/contact' => 1, '/pricing' => 1], Q::col($db, 'page', 'pageviews'));
        T::eq(['/' => 2, '/contact' => 1, '/pricing' => 1], Q::col($db, 'page', 'visits'));
        T::eq(['1' => 1, '3' => 1], Q::col($db, 'pages_per_visit', 'visits'));
        T::eq(['0-10s' => 1, '2-5m' => 1], Q::col($db, 'visit_duration', 'visits'));
        T::eq(['direct' => 1, 'organic' => 1], Q::col($db, 'channel', 'visits'));
        T::eq(['direct' => 1], Q::col($db, 'channel', 'bounces'));
        T::eq(['organic' => 180], Q::col($db, 'channel', 'duration'));
        T::eq(['direct' => 1, 'organic' => 3], Q::col($db, 'channel', 'pageviews'));
        T::eq(['google' => 1], Q::col($db, 'search_engine', 'visits'));
        T::eq(['desktop' => 1, 'mobile' => 1], Q::col($db, 'device', 'visits'));
        T::eq(['Googlebot' => 1], Q::col($db, 'bot', 'hits'));
        T::eq(['(unknown)' => 2], Q::col($db, 'country', 'visits'));
        T::eq(2, Q::count($db, 'visits'));
        T::eq(0, Q::count($db, 'open_visits'));
        T::eq(4, Q::count($db, 'events', "type = 'pageview'"));
        T::eq(0, Q::count($db, 'events', "type = 'bot'"));
        $v = $db->query('SELECT * FROM visits ORDER BY start_ts LIMIT 1')->fetch(PDO::FETCH_ASSOC);
        T::eq(['/', '/contact', 3, 180, 'organic', 'google', 'Chrome', 'Chrome 129', 'Windows 10/11', null],
            [$v['entry_page'], $v['exit_page'], (int) $v['pageviews'], (int) $v['duration'], $v['channel'], $v['search_engine'], $v['browser'], $v['browser_version'], $v['os_version'], $v['ip']]);
        T::eq(1, preg_match('/^[0-9a-f]{16}$/', $v['visitor']));
        T::eq(4, (int) $v['hits'], 'static hit attached to the visit');
        $h = $db->query('SELECT hour, visits, pageviews FROM hourly WHERE day = \'2026-10-10\' ORDER BY hour')->fetchAll(PDO::FETCH_NUM);
        T::eq([[10, 2, 4]], array_map(static fn ($r) => array_map('intval', $r), $h));
    },
    'session timeout splits visits; unique visitors per day' => static function () use ($scenario): void {
        [, $db] = $scenario('timeout', [
            L::line('203.0.113.1', '2026-10-10 10:00:00', '/a'),
            L::line('203.0.113.1', '2026-10-10 10:29:59', '/b'),
            L::line('203.0.113.1', '2026-10-10 10:59:59', '/c'),
            L::line('203.0.113.1', '2026-10-10 11:00:00', '/d'),
        ], '2026-10-10 13:00:00');
        $d = Q::daily($db, '2026-10-10');
        T::eq(2, $d['visits'], 'gap of exactly 30 min starts a new visit');
        T::eq(1, $d['visitors']);
        T::eq(0, $d['bounces']);
        T::eq(1799 + 1, $d['duration']);
        T::eq(['/a' => 1, '/c' => 1], Q::col($db, 'entry_page', 'visits'));
        T::eq(['/b' => 1, '/d' => 1], Q::col($db, 'exit_page', 'visits'));
    },
    'visit crossing midnight stays on its first day' => static function () use ($scenario): void {
        [, $db] = $scenario('midnight', [
            L::line('203.0.113.1', '2026-10-10 23:50:00', '/a'),
            L::line('203.0.113.1', '2026-10-11 00:10:00', '/b'),
            L::line('203.0.113.1', '2026-10-11 00:20:00', '/c'),
        ], '2026-10-11 05:00:00');
        $a = Q::daily($db, '2026-10-10');
        $b = Q::daily($db, '2026-10-11');
        T::eq([1, 1, 1, 0, 1800], [$a['visits'], $a['visitors'], $a['pageviews'], $a['bounces'], $a['duration']]);
        T::eq([0, 0, 2, 0, 0], [$b['visits'], $b['visitors'], $b['pageviews'], $b['bounces'], $b['duration']]);
        T::eq(['/c' => 1], Q::col($db, 'exit_page', 'visits', '2026-10-10'));
        T::eq(['3' => 1], Q::col($db, 'pages_per_visit', 'visits', '2026-10-10'));
        T::eq(['/b' => 1, '/c' => 1], Q::col($db, 'page', 'visits', '2026-10-11'));
        T::eq(1, Q::count($db, 'visits'));
    },
    'open visit survives runs: bounce -1, exit page move, duration' => static function () use ($scenario, $collect): void {
        [$fr, $db] = $scenario('open', [L::line('203.0.113.1', '2026-10-10 10:00:00', '/')], '2026-10-10 10:05:00');
        T::eq(1, Q::count($db, 'open_visits'));
        T::eq(0, Q::count($db, 'visits'));
        $d = Q::daily($db, '2026-10-10');
        T::eq([1, 1, 1], [$d['visits'], $d['pageviews'], $d['bounces']], 'open visit counted already');
        T::eq(['/' => 1], Q::col($db, 'exit_page', 'visits'));

        $fr->writeLog(SSL, [L::line('203.0.113.1', '2026-10-10 10:10:00', '/b')]);
        $collect($fr, '2026-10-10 10:12:00');
        $d = Q::daily($db, '2026-10-10');
        T::eq([1, 2, 0, 600], [$d['visits'], $d['pageviews'], $d['bounces'], $d['duration']]);
        T::eq(['/b' => 1], Q::col($db, 'exit_page', 'visits'));
        T::eq(['/' => 1], Q::col($db, 'entry_page', 'visits'));
        T::eq([], Q::col($db, 'entry_page', 'bounces'));
        T::eq(1, Q::count($db, 'open_visits'));
        T::eq(['direct' => 0], array_map(static fn ($r) => $r['bounces'], Q::dim($db, 'channel')));

        $fr->writeLog(SSL, [L::line('203.0.113.1', '2026-10-10 10:20:00', '/b')]);
        $collect($fr, '2026-10-10 10:21:00');
        T::eq(['/b' => 1], Q::col($db, 'exit_page', 'visits'), 'reload of the same page keeps the exit row');
        T::eq(1200, Q::daily($db, '2026-10-10')['duration']);

        $collect($fr, '2026-10-10 11:00:00');
        T::eq(0, Q::count($db, 'open_visits'));
        T::eq(1, Q::count($db, 'visits'));
        T::eq(['3' => 1], Q::col($db, 'pages_per_visit', 'visits'));
        T::eq(['10-30m' => 1], Q::col($db, 'visit_duration', 'visits'));
        $d = Q::daily($db, '2026-10-10');
        T::eq([1, 1, 3, 0, 1200], [$d['visits'], $d['visitors'], $d['pageviews'], $d['bounces'], $d['duration']]);
        $before = Q::snapshot($db);
        $collect($fr, '2026-10-10 12:00:00');
        T::eq($before, Q::snapshot($db), 're-run without new lines changes nothing');
    },
    'static, redirect and error hits do not start visits' => static function () use ($scenario): void {
        [, $db] = $scenario('nostart', [
            L::line('203.0.113.9', '2026-10-10 10:00:00', '/img/a.png', 200, 100),
            L::line('203.0.113.9', '2026-10-10 10:00:01', '/old', 301, 50),
            L::line('203.0.113.9', '2026-10-10 10:00:02', '/missing', 404, 70, 'https://www.bing.com/search?q=x'),
            L::line('203.0.113.9', '2026-10-10 10:00:03', '/broken', 500, 10),
            L::line('203.0.113.9', '2026-10-10 10:00:04', '/form', 204, 0, null, L::CHROME, 'POST'),
            L::line('203.0.113.8', '2026-10-10 10:00:05', '/files/doc.pdf', 200, 90000, 'https://blog.partner.org/x'),
        ], '2026-10-10 12:00:00');
        $d = Q::daily($db, '2026-10-10');
        T::eq(1, $d['visits'], 'only the download starts a visit');
        T::eq(0, $d['pageviews']);
        T::eq(6, $d['hits']);
        T::eq(1, $d['downloads']);
        T::eq(1, $d['errors_4xx']);
        T::eq(1, $d['errors_5xx']);
        T::eq(0, $d['bounces']);
        T::eq(['/missing' => 1], Q::col($db, 'not_found', 'hits'));
        T::eq(['/missing ← bing.com' => 1], Q::col($db, 'not_found_referrer', 'hits'));
        T::eq(['/broken' => 1], Q::col($db, 'server_error', 'hits'));
        T::eq(['/files/doc.pdf' => 1], Q::col($db, 'download', 'hits'));
        T::eq(['/img/a.png' => 1], Q::col($db, 'static_file', 'hits'));
        T::eq(['200' => 2, '204' => 1, '301' => 1, '404' => 1, '500' => 1], Q::col($db, 'status', 'hits'));
        T::eq(['GET' => 5, 'POST' => 1], Q::col($db, 'method', 'hits'));
        T::eq(['html' => 4, 'pdf' => 1, 'png' => 1], Q::col($db, 'file_type', 'hits'));
        T::eq(['referral' => 1], Q::col($db, 'channel', 'visits'));
        T::eq(['blog.partner.org' => 1], Q::col($db, 'referrer_host', 'visits'));
        T::eq(['blog.partner.org/x' => 1], Q::col($db, 'referrer_url', 'visits'));
        T::eq(['0-10s' => 1], Q::col($db, 'visit_duration', 'visits'));
        T::eq([], Q::col($db, 'pages_per_visit', 'visits'));
        T::eq(4, Q::count($db, 'events'), 'redirect, 404, 500, download');
        T::eq(1, Q::count($db, 'events', 'visit_id IS NOT NULL'));
    },
    'acquisition is decided by the first hit of the visit' => static function () use ($scenario): void {
        [, $db] = $scenario('attrib', [
            L::line('203.0.113.1', '2026-10-10 10:00:00', '/?utm_source=news&utm_medium=email&utm_campaign=oct', 200, 1000, 'https://mail.google.com/'),
            L::line('203.0.113.1', '2026-10-10 10:01:00', '/x', 200, 1000, 'https://l.facebook.com/'),
            L::line('203.0.113.2', '2026-10-10 10:00:00', '/?gclid=abc', 200, 1000, 'https://www.google.com/'),
            L::line('203.0.113.3', '2026-10-10 10:00:00', '/', 200, 1000, 'https://www.google.de/search?q=Web+Pulse'),
            L::line('203.0.113.4', '2026-10-10 10:00:00', '/', 200, 1000, 'https://t.co/xyz'),
            L::line('203.0.113.5', '2026-10-10 10:00:00', '/', 200, 1000, 'https://www.example.com/internal'),
        ], '2026-10-10 12:00:00');
        T::eq(['direct' => 1, 'email' => 1, 'organic' => 1, 'paid' => 1, 'social' => 1], Q::col($db, 'channel', 'visits'));
        T::eq(['direct' => 1, 'email' => 2, 'organic' => 1, 'paid' => 1, 'social' => 1], Q::col($db, 'channel', 'pageviews'));
        T::eq(['oct' => 1], Q::col($db, 'campaign', 'visits'));
        T::eq(['news' => 1], Q::col($db, 'utm_source', 'visits'));
        T::eq(['email' => 1], Q::col($db, 'utm_medium', 'visits'));
        T::eq(['news / email' => 1], Q::col($db, 'source_medium', 'visits'));
        T::eq(['web pulse' => 1], Q::col($db, 'keyword', 'visits'));
        T::eq(['google' => 2], Q::col($db, 'search_engine', 'visits'));
        T::eq(['google.com' => 1, 'google.de' => 1, 'mail.google.com' => 1, 't.co' => 1], Q::col($db, 'referrer_host', 'visits'));
        T::eq(['/' => 5, '/x' => 1], Q::col($db, 'page', 'visits'), 'query string stripped');
        $ev = $db->query("SELECT referrer FROM events WHERE path = '/x'")->fetchColumn();
        T::eq('l.facebook.com', $ev);
    },
    'http and https logs are merged by time into one visit' => static function (): void {
        $fr = FakeRoot::create('sess-merge');
        $fr->writeLog(PLAIN, [L::line('203.0.113.1', '2026-10-10 10:00:00', '/', 200, 100, null, L::CHROME, 'GET', 'Europe/Rome', 'HTTP/1.1')]);
        $fr->writeLog(SSL, [L::line('203.0.113.1', '2026-10-10 10:02:00', '/b'), L::line('203.0.113.1', '2026-10-10 10:04:00', '/c')]);
        $fr->writeLog(PLAIN, [L::line('203.0.113.1', '2026-10-10 10:03:00', '/d', 200, 100, null, L::CHROME, 'GET', 'Europe/Rome', 'HTTP/1.1')]);
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        T::eq(0, $fr->run('collect')['code']);
        $db = $fr->db('alice', 'example.com');
        T::eq(1, Q::daily($db, '2026-10-10')['visits']);
        T::eq(['/c' => 1], Q::col($db, 'exit_page', 'visits'));
        T::eq(['http' => 2, 'https' => 2], Q::col($db, 'protocol', 'hits'));
        T::eq(['HTTP/1.1' => 2, 'HTTP/2.0' => 2], Q::col($db, 'http_version', 'hits'));
        T::eq(240, Q::daily($db, '2026-10-10')['duration']);
    },
    'ignored lines: HEAD, excluded paths and IPs (server and user settings)' => static function (): void {
        $fr = FakeRoot::create('sess-ignore', ['exclude_ips' => ['192.0.2.0/24']]);
        @mkdir($fr->path('var/webpulse/data/alice/user'), 0700, true);
        file_put_contents($fr->path('var/webpulse/data/alice/user/settings.json'), json_encode(['version' => 1, 'domains' => [
            'example.com' => ['exclude_ips' => ['2001:db8::/32'], 'exclude_paths' => ['/staging/'], 'timezone' => null],
        ]]));
        $fr->writeLog(SSL, [
            L::line('203.0.113.1', '2026-10-10 10:00:00', '/', 200, 0, null, L::CHROME, 'HEAD'),
            L::line('203.0.113.1', '2026-10-10 10:00:01', '/.well-known/acme-challenge/x'),
            L::line('192.0.2.55', '2026-10-10 10:00:02', '/'),
            L::line('2001:db8::7', '2026-10-10 10:00:03', '/'),
            L::line('203.0.113.1', '2026-10-10 10:00:04', '/staging/page'),
            L::line('203.0.113.1', '2026-10-10 10:00:05', '/kept'),
            'complete garbage',
        ]);
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        $r = $fr->run('collect --json');
        T::eq(7, $r['json']['lines']);
        T::eq(1, $r['json']['bad']);
        T::eq(5, $r['json']['ignored']);
        $db = $fr->db('alice', 'example.com');
        T::eq(['/kept' => 1], Q::col($db, 'page', 'pageviews'));
        $cp = $db->query('SELECT lines, bad_lines, ignored_lines FROM checkpoints')->fetch(PDO::FETCH_NUM);
        T::eq([7, 1, 5], array_map('intval', $cp));
    },
    'bots never create visits; empty UA is a bot' => static function () use ($scenario): void {
        [, $db] = $scenario('bots', [
            L::line('66.249.66.1', '2026-10-10 10:00:00', '/', 200, 100, null, L::GOOGLEBOT),
            L::line('66.249.66.1', '2026-10-10 10:00:01', '/a', 404, 100, null, L::GOOGLEBOT),
            L::line('192.0.2.1', '2026-10-10 10:00:02', '/', 200, 100, null, '-'),
            L::line('192.0.2.2', '2026-10-10 10:00:03', '/', 200, 100, null, 'curl/8.0'),
        ], '2026-10-10 12:00:00');
        $d = Q::daily($db, '2026-10-10');
        T::eq([0, 0, 0, 4, 400, 0], [$d['visits'], $d['pageviews'], $d['hits'], $d['bot_hits'], $d['bot_bytes'], $d['errors_4xx']]);
        T::eq(['Empty user agent' => 1, 'Googlebot' => 2, 'curl' => 1], Q::col($db, 'bot', 'hits'));
        T::eq(4, (int) $db->query("SELECT SUM(bot_hits) FROM hourly")->fetchColumn());
    },
    'domain timezone from user settings decides the local day' => static function (): void {
        $fr = FakeRoot::create('sess-tz');
        @mkdir($fr->path('var/webpulse/data/alice/user'), 0700, true);
        file_put_contents($fr->path('var/webpulse/data/alice/user/settings.json'), json_encode(['version' => 1, 'domains' => ['example.com' => ['timezone' => 'America/New_York']]]));
        $fr->writeLog(SSL, [L::line('203.0.113.1', '2026-10-11 03:00:00', '/')]);  // = 2026-10-10 21:00 in New York
        $fr->setNow(L::ts('2026-10-11 12:00:00'));
        $fr->run('collect');
        $db = $fr->db('alice', 'example.com');
        T::eq(1, Q::daily($db, '2026-10-10')['visits']);
        T::eq(0, Q::daily($db, '2026-10-11')['visits']);
        T::eq('America/New_York', $db->query("SELECT value FROM meta WHERE key = 'timezone'")->fetchColumn());
        T::eq(21, (int) $db->query('SELECT hour FROM hourly')->fetchColumn());
        // change the timezone -> tz_changed_at recorded
        file_put_contents($fr->path('var/webpulse/data/alice/user/settings.json'), json_encode(['version' => 1, 'domains' => ['example.com' => ['timezone' => 'Asia/Tokyo']]]));
        $fr->run('collect');
        T::eq((string) L::ts('2026-10-11 12:00:00'), $db->query("SELECT value FROM meta WHERE key = 'tz_changed_at'")->fetchColumn());
    },
    'store_ip keeps the address only in visits.ip; invalid UTF-8 is cleaned' => static function () use ($scenario): void {
        [, $db] = $scenario('storeip', [
            L::line('203.0.113.77', '2026-10-10 10:00:00', '/caf%C3%A9/%FF', 200, 10, "https://ref.example.org/\xc3\x28"),
        ], '2026-10-10 12:00:00', ['store_ip' => true]);
        T::eq('203.0.113.77', $db->query('SELECT ip FROM visits')->fetchColumn());
        $key = array_key_first(Q::col($db, 'page', 'pageviews'));
        T::eq("/caf\u{e9}/?", $key);
        foreach ($db->query('SELECT key FROM dim_daily')->fetchAll(PDO::FETCH_COLUMN) as $k) {
            T::eq(1, preg_match('//u', (string) $k), 'valid UTF-8: ' . bin2hex((string) $k));
        }
        T::eq(0, Q::count($db, 'open_visits', "attrs LIKE '%203.0.113.77%' AND 0"));
    },
    'out-of-order lines within the timeout stay in the visit' => static function () use ($scenario): void {
        [, $db] = $scenario('order', [
            L::line('203.0.113.1', '2026-10-10 10:00:10', '/a'),
            L::line('203.0.113.1', '2026-10-10 10:00:05', '/b'),
            L::line('203.0.113.1', '2026-10-10 10:02:00', '/c'),
        ], '2026-10-10 12:00:00');
        $d = Q::daily($db, '2026-10-10');
        T::eq([1, 3, 0, 115], [$d['visits'], $d['pageviews'], $d['bounces'], $d['duration']]);
        T::eq(['/c' => 1], Q::col($db, 'exit_page', 'visits'));
    },
];
