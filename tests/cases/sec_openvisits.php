<?php

declare(strict_types=1);

use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\Lines as L;
use WebPulse\Tests\T;

/*
 * Security review H2 — a flood of distinct visitors (one user agent each)
 * must not exhaust the collector's memory: open visits are capped
 * (max_open_visits, the oldest closed early with their counters), stored
 * visits are closed in batches instead of all being loaded, and each account
 * of the global run is collected in its own process.
 */

const SEC_OV_LOG = 'etc/apache2/logs/domlogs/alice/example.com-ssl_log';

/** Writes $n pageviews of distinct visitors within 30 minutes from 10:00 local time. */
$flood = static function (FakeRoot $fr, int $n): void {
    $base = L::ts('2026-10-10 10:00:00');
    $tz = new DateTimeZone('Europe/Rome');
    $file = $fr->path(SEC_OV_LOG);
    @mkdir(dirname($file), 0755, true);
    $f = fopen($file, 'w');
    for ($i = 0; $i < $n; $i++) {
        $d = (new DateTimeImmutable('@' . ($base + intdiv($i * 1700, $n))))->setTimezone($tz);
        fwrite($f, sprintf("203.0.%d.%d - - [%s] \"GET /p%d HTTP/2.0\" 200 1000 \"-\" \"Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0 u%d\"\n",
            ($i >> 8) & 255, $i & 255, $d->format('d/M/Y:H:i:s O'), $i % 500, $i));
    }
    fclose($f);
};

/** Every visit is counted once and every closed visit has its engagement bucket and row. */
$consistent = static function (FakeRoot $fr, int $n, string $what): array {
    $db = $fr->db('alice', 'example.com');
    $open = (int) $db->query('SELECT COUNT(*) FROM open_visits')->fetchColumn();
    $closed = (int) $db->query('SELECT COUNT(*) FROM visits')->fetchColumn();
    T::eq($n, (int) $db->query('SELECT SUM(visits) FROM daily')->fetchColumn(), "$what: visits counted");
    T::eq($n, $open + $closed, "$what: open + closed visits");
    T::eq($closed, (int) $db->query("SELECT COALESCE(SUM(visits),0) FROM dim_daily WHERE dim = 'visit_duration'")->fetchColumn(), "$what: duration buckets of closed visits");
    T::eq($n, (int) $db->query("SELECT SUM(pageviews) FROM daily")->fetchColumn(), "$what: pageviews");
    return [$open, $closed];
};

return [
    'max_open_visits: 60k distinct visitors in 30 minutes stay under 200 MB' => static function () use ($flood, $consistent): void {
        $fr = FakeRoot::create('sec-openvisits');
        $n = 60000;
        $flood($fr, $n);
        $fr->setNow(L::ts('2026-10-10 10:29:00'));
        $r = $fr->run('collect --json');
        T::eq(0, $r['code'], $r['stderr'] . $r['stdout']);
        T::eq($n, $r['json']['lines']);
        T::eq(0, $r['json']['errors']);
        T::less(200 * 1048576, $r['json']['peak_memory'], 'peak memory of the account process');
        [$open] = $consistent($fr, $n, 'first run');
        T::true($open <= 50000, "open visits ($open) within the default cap");
        T::contains('closed early', (string) file_get_contents($fr->path('var/webpulse/log/webpulse.log')));

        // a lower cap: the stored visits beyond it are closed in batches while loading
        $fr->patchConfig(['max_open_visits' => 1000]);
        $fr->setNow(L::ts('2026-10-10 10:35:00'));
        $r = $fr->run('collect --json');
        T::eq(0, $r['code'], $r['stderr'] . $r['stdout']);
        T::less(200 * 1048576, $r['json']['peak_memory']);
        [$open] = $consistent($fr, $n, 'second run');
        T::true($open <= 1000, "open visits ($open) within the lowered cap");

        // and everything closes normally once stale
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        T::eq(0, $fr->run('collect --json')['code']);
        [$open, $closed] = $consistent($fr, $n, 'third run');
        T::eq(0, $open);
        T::eq($n, $closed);
    },
    'global collect: a fatal error in one account does not stop the others' => static function (): void {
        $fr = FakeRoot::create('sec-openvisits-child');
        $fr->writeLog(SEC_OV_LOG, [L::line('203.0.113.1', '2026-10-10 10:00:00', '/')]);
        $fr->writeLog('etc/apache2/logs/domlogs/bob/blog.net', [L::line('198.51.100.1', '2026-10-10 10:00:00', '/')]);
        // php_bin wrapper: the process collecting alice dies of a real PHP fatal (memory exhausted, exit 255)
        $wrapper = $fr->dir() . '/php-crash-alice.sh';
        $php = escapeshellarg(PHP_BINARY);
        file_put_contents($wrapper, "#!/bin/sh\ncase \"\$*\" in\n  *'--user alice'*) exec $php -n -d memory_limit=16M -r '\$a = str_repeat(\"x\", 64 << 20);' ;;\nesac\nexec $php \"\$@\"\n");
        chmod($wrapper, 0755);
        $fr->patchConfig(['php_bin' => $wrapper]);
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        $r = $fr->run('collect --json');
        T::eq(0, $r['code'], $r['stderr'] . $r['stdout']);
        T::eq(1, $r['json']['errors'], 'the crashed account is an error');
        T::eq(1, $r['json']['users'], 'bob was collected');
        T::eq(1, (int) $fr->scalar('bob', 'blog.net', 'SELECT SUM(hits) FROM daily'));
        T::false(is_file($fr->path('var/webpulse/data/alice/example.com.db')), 'alice untouched');
        T::contains('collect alice failed: child process exited with code 255', (string) file_get_contents($fr->path('var/webpulse/log/webpulse.log')));
        // --user keeps running in-process (and --child requires it)
        T::eq(2, $fr->run('collect --child')['code']);
    },
];
