<?php

declare(strict_types=1);

use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\Lines as L;
use WebPulse\Tests\Q;
use WebPulse\Tests\T;

return [
    'rollup: caps dimensions, purges raw data, closes stale visits, refreshes summary' => static function (): void {
        $fr = FakeRoot::create('rollup', ['max_rows_per_dim' => 10, 'raw_retention_days' => 3]);
        $lines = [];
        for ($i = 0; $i < 30; $i++) {
            $lines[] = L::line('203.0.113.' . ($i + 1), '2026-10-05 10:00:' . sprintf('%02d', $i), "/page-$i", 200, 100 + $i, "https://site$i.example.org/");
        }
        for ($i = 0; $i < 5; $i++) {
            $lines[] = L::line('198.51.100.' . ($i + 1), '2026-10-05 11:00:00', '/page-0');
        }
        $lines[] = L::line('192.0.2.1', '2026-10-10 11:50:00', '/today');
        $fr->writeLog('etc/apache2/logs/domlogs/alice/example.com-ssl_log', $lines);
        $fr->setNow(L::ts('2026-10-10 11:55:00'));
        T::eq(0, $fr->run('collect')['code']);
        $db = $fr->db('alice', 'example.com');
        T::eq(30, count(Q::dim($db, 'page', '2026-10-05')));
        T::eq(1, Q::count($db, 'open_visits'));
        $before = Q::totals($db);
        $pvBefore = array_sum(Q::col($db, 'page', 'pageviews', '2026-10-05'));

        $fr->setNow(L::ts('2026-10-10 13:00:00'));
        $r = $fr->run('rollup --json');
        T::eq(0, $r['code'], $r['stderr']);
        $j = $r['json'];
        T::eq(1, $j['closed']);
        T::greater(0, $j['folded']);
        $pages = Q::dim($db, 'page', '2026-10-05');
        T::eq(10, count($pages));
        T::true(isset($pages['(other)']));
        T::eq(6, $pages['/page-0']['pageviews'], 'top row kept');
        T::eq($pvBefore, array_sum(array_column($pages, 'pageviews')), 'sums preserved');
        T::eq(10, count(Q::dim($db, 'referrer_host', '2026-10-05')));
        T::eq(30, (int) $db->query("SELECT SUM(visits) FROM dim_daily WHERE dim = 'referrer_host'")->fetchColumn(), 'referrer visits preserved');
        T::eq(1, count(Q::dim($db, 'page', '2026-10-10')), 'unfinished day untouched');
        T::eq($before, Q::totals($db));
        T::eq(0, Q::count($db, 'visits', "day < '2026-10-07'"));
        T::eq(0, Q::count($db, 'events', 'ts < ?', [L::ts('2026-10-07 00:00:00')]));
        T::eq(1, Q::count($db, 'visits'), 'the closed visit of today');
        T::eq(0, Q::count($db, 'open_visits'));
        T::eq(0, Q::count($db, 'visitor_day', "day < '2026-10-08'"));
        T::eq((string) L::ts('2026-10-10 13:00:00'), $db->query("SELECT value FROM meta WHERE key = 'last_rollup_at'")->fetchColumn());
        $sum = $fr->app()->registry()->summaries(['alice']);
        T::eq(1, count($sum));
        T::eq(1, $sum[0]['visits_today']);
        T::eq(36, $sum[0]['visits_7d']);
        T::true($sum[0]['db_bytes'] > 0);
        // idempotent
        $r2 = $fr->run('rollup --json');
        T::eq(0, $r2['json']['folded']);
        T::eq(10, count(Q::dim($db, 'page', '2026-10-05')));
        // salts older than the window are pruned
        $reg = $fr->app()->registry();
        T::eq(0, (int) $reg->pdo()->query("SELECT COUNT(*) FROM salts WHERE day < '2026-10-07'")->fetchColumn());
    },
];
