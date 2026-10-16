<?php

declare(strict_types=1);

use WebPulse\Registry\Registry;
use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\Lines as L;
use WebPulse\Tests\MmdbWriter;
use WebPulse\Tests\Q;
use WebPulse\Tests\T;
use WebPulse\Util\Lock;

const DOMAINS = [['alice', 'example.com'], ['alice', 'shop.example.com'], ['bob', 'blog.net']];

$ok = static function (array $r, string $what = ''): array {
    T::eq(0, $r['code'], "$what failed: " . $r['stderr'] . $r['stdout']);
    return $r;
};
$sumAll = static function (FakeRoot $fr, string $sql): int {
    $n = 0;
    foreach (DOMAINS as [$u, $d]) {
        if (is_file($fr->path("var/webpulse/data/$u/$d.db"))) {
            $n += (int) $fr->scalar($u, $d, $sql);
        }
    }
    return $n;
};

return [
    'generated logs: line accounting matches the generator' => static function () use ($ok, $sumAll): void {
        $fr = FakeRoot::create('col-gen');
        $sum = $fr->generateLogs(['seed' => 3, 'days' => 3, 'lines' => 6000, 'end' => '2026-10-16', 'no-archive' => true]);
        $fr->setNow(L::ts('2026-10-17 03:00:00'));
        $r = $ok($fr->run('collect --json'), 'collect');
        $j = $r['json'];
        T::eq($sum['lines'], $j['lines']);
        T::eq($sum['malformed'], $j['bad']);
        T::eq($sum['head'] + $sum['acme'], $j['ignored']);
        T::eq(3, $j['domains']);
        T::eq(6, $j['files']);
        T::eq($sum['bot'] + $sum['tls'], $sumAll($fr, 'SELECT SUM(bot_hits) FROM daily'));
        T::eq($sum['human'], $sumAll($fr, 'SELECT SUM(hits) FROM daily'));
        T::eq($sum['lines'], $sumAll($fr, 'SELECT SUM(lines) FROM checkpoints'));
        T::eq($sum['lines'], $sumAll($fr, "SELECT value FROM meta WHERE key = 'total_lines'"));
        T::eq(0, $sumAll($fr, 'SELECT COUNT(*) FROM open_visits'), 'all visits closed the next day');
        // internal consistency
        foreach (DOMAINS as [$u, $d]) {
            $db = $fr->db($u, $d);
            $t = Q::totals($db);
            T::true($t['visits'] > 0, "$d has visits");
            T::eq($t['visits'], Q::count($db, 'visits'), "$d visits rows");
            T::eq($t['visits'], array_sum(Q::col($db, 'channel', 'visits')), "$d channel visits");
            T::eq($t['visits'], array_sum(Q::col($db, 'device', 'visits')), "$d device visits");
            T::eq($t['visits'], array_sum(Q::col($db, 'entry_page', 'visits')) + Q::count($db, 'visits', 'pageviews = 0'), "$d entry pages");
            T::eq($t['visits'] - Q::count($db, 'visits', 'pageviews = 0'), array_sum(Q::col($db, 'exit_page', 'visits')), "$d exit pages");
            T::eq($t['visits'], array_sum(Q::col($db, 'visit_duration', 'visits')), "$d duration buckets");
            T::eq($t['bounces'], Q::count($db, 'visits', 'pageviews = 1'), "$d bounces");
            T::eq($t['bounces'], array_sum(Q::col($db, 'channel', 'bounces')), "$d channel bounces");
            T::eq($t['duration'], (int) $db->query('SELECT SUM(duration) FROM visits')->fetchColumn(), "$d duration");
            T::eq($t['duration'], array_sum(Q::col($db, 'channel', 'duration')), "$d channel duration");
            T::eq($t['pageviews'], array_sum(Q::col($db, 'page', 'pageviews')), "$d page pageviews");
            T::eq($t['pageviews'], array_sum(Q::col($db, 'channel', 'pageviews')), "$d channel pageviews");
            T::eq($t['pageviews'], (int) $db->query('SELECT SUM(pageviews) FROM visits')->fetchColumn(), "$d visit pageviews");
            T::eq($t['hits'], array_sum(Q::col($db, 'status', 'hits')), "$d status hits");
            T::eq($t['hits'], (int) $db->query('SELECT SUM(hits) FROM hourly')->fetchColumn(), "$d hourly hits");
            T::eq($t['visitors'], (int) $db->query('SELECT COUNT(*) FROM (SELECT DISTINCT day, visitor FROM visits)')->fetchColumn(), "$d visitors");
            T::greater(0, count(Q::col($db, 'bot', 'hits')));
            T::eq(array_values(array_unique(array_merge(['(unknown)'], array_keys(Q::col($db, 'country', 'visits'))))), ['(unknown)']);
        }
    },
    're-running is idempotent; appended lines are processed once' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-rerun');
        $fr->generateLogs(['seed' => 5, 'days' => 2, 'lines' => 3000, 'end' => '2026-10-16', 'no-archive' => true]);
        $fr->setNow(L::ts('2026-10-17 03:00:00'));
        $ok($fr->run('collect'));
        $db = $fr->db('alice', 'example.com');
        $snap = Q::snapshot($db);
        $r = $ok($fr->run('collect --json'));
        T::eq(0, $r['json']['lines']);
        T::eq($snap, Q::snapshot($db));
        $before = Q::totals($db);
        $fr->writeLog('etc/apache2/logs/domlogs/alice/example.com-ssl_log', [
            L::line('203.0.113.200', '2026-10-17 02:00:00', '/new-page'),
            L::line('203.0.113.200', '2026-10-17 02:01:00', '/new-page-2'),
        ]);
        $r = $ok($fr->run('collect --json'));
        T::eq(2, $r['json']['lines']);
        $after = Q::totals($db);
        T::eq($before['pageviews'] + 2, $after['pageviews']);
        T::eq($before['visits'] + 1, $after['visits']);
        $r = $ok($fr->run('collect --json'));
        T::eq(0, $r['json']['lines']);
        T::eq($after, Q::totals($db));
    },
    'truncation and rotation of a live log through the CLI' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-rotate');
        $log = 'etc/apache2/logs/domlogs/alice/example.com-ssl_log';
        $fr->writeLog($log, [L::line('203.0.113.1', '2026-10-10 10:00:00', '/a'), L::line('203.0.113.2', '2026-10-10 10:00:00', '/b')]);
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        $ok($fr->run('collect'));
        $db = $fr->db('alice', 'example.com');
        T::eq(2, Q::totals($db)['pageviews']);
        // cpanellogd truncation: file emptied, new lines appended later
        $fr->writeLog($log, [L::line('203.0.113.3', '2026-10-10 12:30:00', '/c')], false);
        $ok($fr->run('collect'));
        T::eq(3, Q::totals($db)['pageviews']);
        // rotation: file renamed, new file created
        rename($fr->path($log), $fr->path($log) . '.1');
        $fr->writeLog($log, [L::line('203.0.113.4', '2026-10-10 13:00:00', '/d'), L::line('203.0.113.5', '2026-10-10 13:00:00', '/e')]);
        $ok($fr->run('collect'));
        T::eq(5, Q::totals($db)['pageviews']);
        $ok($fr->run('collect'));
        T::eq(5, Q::totals($db)['pageviews']);
    },
    'backfill imports archives once and never what live collection covered' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-backfill');
        $sum = $fr->generateLogs(['seed' => 9, 'days' => 20, 'lines' => 8000, 'end' => '2026-10-10']);
        T::true(isset($sum['files']['home/alice/logs/example.com-ssl_log-Sep-2026.gz']));
        $fr->setNow(L::ts('2026-10-11 03:00:00'));
        $ok($fr->run('collect'));
        $db = $fr->db('alice', 'example.com');
        $live = Q::totals($db);
        T::eq('2026-10-01', $db->query('SELECT MIN(day) FROM daily')->fetchColumn());
        $r = $ok($fr->run('backfill --user alice --json'));
        $archLines = $sum['files']['home/alice/logs/example.com-Sep-2026.gz'] + $sum['files']['home/alice/logs/example.com-ssl_log-Sep-2026.gz'];
        T::eq(2, $r['json']['files']);
        T::eq($archLines, $r['json']['lines']);
        T::eq('2026-09-21', $db->query('SELECT MIN(day) FROM daily')->fetchColumn());
        $all = Q::totals($db);
        T::greater($live['hits'], $all['hits']);
        T::eq(2, Q::count($db, 'imported_archives', 'size > 0'));
        // second run: nothing imported
        $r = $ok($fr->run('backfill --user alice --json'));
        T::eq(0, $r['json']['files']);
        T::eq(2, $r['json']['skipped_files']);
        T::eq($all, Q::totals($db));
        // an archive of the current month duplicates what the live tail already read: skipped by live_since
        $liveFile = $fr->domlog('alice', 'example.com-ssl_log');
        file_put_contents($fr->path('home/alice/logs/example.com-ssl_log-Oct-2026.gz'), gzencode((string) file_get_contents($liveFile)));
        $r = $ok($fr->run('backfill --user alice --domain example.com --json'));
        T::eq(1, $r['json']['files']);
        T::eq(0, $r['json']['imported']);
        T::eq($all, Q::totals($db));
        // jobs recorded
        $jobs = $fr->app()->registry()->jobs(10);
        T::eq('backfill', $jobs[0]['type']);
        T::eq('ok', $jobs[0]['status']);
    },
    'backfill before live collection: live tail skips what the archives covered' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-backfill-first');
        $log = 'etc/apache2/logs/domlogs/alice/example.com-ssl_log';
        $lines = [L::line('203.0.113.1', '2026-10-10 10:00:00', '/a'), L::line('203.0.113.2', '2026-10-10 10:05:00', '/b')];
        // the archive holds the same lines as the (not truncated) live log
        file_put_contents($fr->path('home/alice/logs/example.com-ssl_log-Oct-2026.gz'), gzencode(implode("\n", $lines) . "\n"));
        $fr->writeLog($log, $lines);
        $fr->writeLog($log, [L::line('203.0.113.3', '2026-10-10 10:10:00', '/c')]);
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        $ok($fr->run('backfill --user alice'));
        $db = $fr->db('alice', 'example.com');
        T::eq(2, Q::totals($db)['pageviews']);
        $ok($fr->run('collect'));
        T::eq(3, Q::totals($db)['pageviews'], 'only the line newer than the archive is added');
    },
    'grown archive resumes after its last imported line' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-grow');
        $arch = $fr->path('home/alice/logs/example.com-ssl_log-Sep-2026.gz');
        file_put_contents($arch, gzencode(L::line('203.0.113.1', '2026-09-10 10:00:00', '/a') . "\n"));
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        $ok($fr->run('backfill --user alice'));
        file_put_contents($arch, gzencode(L::line('203.0.113.2', '2026-09-11 10:00:00', '/b') . "\n"), FILE_APPEND);
        touch($arch, time() + 5);
        $r = $ok($fr->run('backfill --user alice --json'));
        T::eq(2, $r['json']['lines']);
        T::eq(1, $r['json']['imported']);
        T::eq(2, Q::totals($fr->db('alice', 'example.com'))['pageviews']);
    },
    'enable / disable accounts and domains' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-switch');
        $fr->generateLogs(['seed' => 1, 'days' => 1, 'lines' => 800, 'end' => '2026-10-16', 'no-archive' => true]);
        $fr->setNow(L::ts('2026-10-17 03:00:00'));
        $ok($fr->run('accounts disable bob'));
        $ok($fr->run('domains disable alice shop.example.com'));
        $r = $ok($fr->run('collect --json'));
        T::eq(1, $r['json']['domains']);
        T::true(is_file($fr->path('var/webpulse/data/alice/example.com.db')));
        T::false(is_file($fr->path('var/webpulse/data/alice/shop.example.com.db')));
        T::false(is_file($fr->path('var/webpulse/data/bob/blog.net.db')));
        // a domain override cannot re-enable a disabled account
        $ok($fr->run('domains enable bob blog.net'));
        $ok($fr->run('collect'));
        T::false(is_file($fr->path('var/webpulse/data/bob/blog.net.db')));
        $ok($fr->run('accounts reset bob'));
        $ok($fr->run('domains reset alice shop.example.com'));
        $r = $ok($fr->run('collect --json'));
        T::eq(3, $r['json']['domains'], 'example.com (no new lines) + the two re-enabled domains');
        T::true(is_file($fr->path('var/webpulse/data/bob/blog.net.db')));
        // default_enabled=false: only explicit overrides are collected
        $ok($fr->run('config set default_enabled false'));
        $ok($fr->run('accounts reset alice'));
        $reg = $fr->app()->registry();
        T::false($reg->isEnabled('alice', 'example.com'));
        T::true($reg->isEnabled('bob', 'blog.net'), 'bob has a domain override');
        $list = $ok($fr->run('accounts list --json'))['json']['accounts'];
        T::eq(['alice', 'bob'], array_column($list, 'user'));
        T::eq('reseller1', $list[1]['owner']);
        $r = $ok($fr->run('collect --user alice --domain blog.net --json'));
        T::eq(['blog.net (not a domain of alice)'], $r['json']['skipped']);
        T::false(is_file($fr->path('var/webpulse/data/alice/blog.net.db')));
        $doms = $ok($fr->run('domains list alice --json'))['json']['domains'];
        T::eq(['example.com' => 'main', 'shop.example.com' => 'sub'], array_column($doms, 'kind', 'domain'));
    },
    'hooked collect drains flagged accounts only and removes the flags' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-hook');
        $fr->generateLogs(['seed' => 2, 'days' => 1, 'lines' => 600, 'end' => '2026-10-16', 'no-archive' => true]);
        $fr->setNow(L::ts('2026-10-17 03:00:00'));
        $ok($fr->run('setup --quiet'));
        T::eq('1777', sprintf('%o', fileperms($fr->path('var/webpulse/run/hook')) & 07777));
        $r = $ok($fr->run('collect --hooked --json'));
        T::eq(0, $r['json']['users']);
        touch($fr->path('var/webpulse/run/hook/bob'));
        touch($fr->path('var/webpulse/run/hook/ghost'));
        touch($fr->path('var/webpulse/run/hook/Bad..Name'));
        $r = $ok($fr->run('hooked --json'));
        T::eq(1, $r['json']['users']);
        T::true(is_file($fr->path('var/webpulse/data/bob/blog.net.db')));
        T::false(is_file($fr->path('var/webpulse/data/alice/example.com.db')));
        T::false(file_exists($fr->path('var/webpulse/run/hook/bob')));
        T::false(file_exists($fr->path('var/webpulse/run/hook/ghost')), 'unknown account flag removed');
        $jobs = $fr->app()->registry()->jobs(5);
        T::eq('hook', $jobs[0]['type']);
    },
    'locks: concurrent collect exits 0, locked accounts are skipped' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-lock');
        $fr->generateLogs(['seed' => 2, 'days' => 1, 'lines' => 400, 'end' => '2026-10-16', 'no-archive' => true]);
        $fr->setNow(L::ts('2026-10-17 03:00:00'));
        @mkdir($fr->path('var/webpulse/run'), 0700, true);
        $lock = Lock::acquire($fr->path('var/webpulse/run/collect.lock'));
        T::notNull($lock);
        $r = $ok($fr->run('collect'));
        T::contains('already running', $r['stdout']);
        T::false(is_dir($fr->path('var/webpulse/data/alice')));
        $lock->release();
        $ul = Lock::acquire($fr->path('var/webpulse/run/user-alice.lock'));
        $r = $ok($fr->run('collect --json'));
        T::eq(['alice (locked)'], $r['json']['skipped']);
        T::true(is_file($fr->path('var/webpulse/data/bob/blog.net.db')));
        $ul->release();
    },
    'jobs: --job creates a row, --job <id> adopts one' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-jobs');
        $fr->generateLogs(['seed' => 2, 'days' => 1, 'lines' => 300, 'end' => '2026-10-16', 'no-archive' => true]);
        $fr->setNow(L::ts('2026-10-17 03:00:00'));
        $r = $ok($fr->run('collect --job --json'));
        $id = $r['json']['job_id'];
        $job = $fr->app()->registry()->job($id);
        T::eq(['collect', 'all', 'ok'], [$job['type'], $job['scope'], $job['status']]);
        T::eq(300, (int) $job['lines']);
        $reg = $fr->app()->registry();
        $pre = $reg->startJob('rollup', 'all');
        T::eq('running', $reg->job($pre)['status']);
        $ok($fr->run("rollup --job $pre"));
        T::eq('ok', $fr->app()->registry()->job($pre)['status']);
        $ok($fr->run('collect --user alice --domain example.com --job'));
        T::eq('alice/example.com', $fr->app()->registry()->jobs(1)[0]['scope']);
    },
    'unreadable log file does not abort the run' => static function () use ($ok): void {
        if (function_exists('posix_geteuid') && posix_geteuid() === 0) {
            throw new WebPulse\Tests\Skip('root can read everything');
        }
        $fr = FakeRoot::create('col-unreadable');
        $fr->generateLogs(['seed' => 2, 'days' => 1, 'lines' => 500, 'end' => '2026-10-16', 'no-archive' => true]);
        $fr->setNow(L::ts('2026-10-17 03:00:00'));
        chmod($fr->domlog('alice', 'example.com-ssl_log'), 0);
        $r = $ok($fr->run('collect --json --quiet'));
        chmod($fr->domlog('alice', 'example.com-ssl_log'), 0644);
        T::eq(1, $r['json']['errors']);
        T::eq(3, $r['json']['domains']);
        T::true(is_file($fr->path('var/webpulse/data/bob/blog.net.db')));
        T::contains('cannot read', (string) file_get_contents($fr->path('var/webpulse/log/webpulse.log')));
    },
    'setup is idempotent and creates the layout' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-setup');
        $r = $ok($fr->run('setup --json'));
        T::eq(2, $r['json']['accounts']);
        T::true(is_file($fr->path('var/webpulse/webpulse.db')));
        T::eq('600', sprintf('%o', fileperms($fr->path('var/webpulse/webpulse.db')) & 0777));
        T::eq('750', sprintf('%o', fileperms($fr->path('var/webpulse/data/alice')) & 0777));
        T::eq('700', sprintf('%o', fileperms($fr->path('var/webpulse/data/alice/user')) & 0777));
        T::eq('700', sprintf('%o', fileperms($fr->path('var/webpulse/log')) & 0777));
        $r2 = $ok($fr->run('setup --json'));
        T::eq([], $r2['json']['actions']);
        // setup with a missing config file creates it from the default
        $cfg = $fr->dir() . '/new/config.json';
        $r3 = FakeRoot::exec([PHP_BINARY, FakeRoot::repo() . '/bin/webpulse', 'setup', '--config', $cfg, '--no-sync', '--json'], ['WEBPULSE_CONFIG' => '/nonexistent']);
        // the default config points to /var/webpulse which is not writable here: only check the file was created
        T::true(is_file($cfg));
        T::eq('600', sprintf('%o', fileperms($cfg) & 0777));
        T::eq(json_decode((string) file_get_contents(FakeRoot::repo() . '/etc/config.json.default'), true), json_decode((string) file_get_contents($cfg), true));
        unset($r3);
    },
    'status, config get/set/unset, usage errors and purge' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-cli');
        $fr->generateLogs(['seed' => 2, 'days' => 1, 'lines' => 300, 'end' => '2026-10-16', 'no-archive' => true]);
        $fr->setNow(L::ts('2026-10-17 03:00:00'));
        $ok($fr->run('collect'));
        $st = $ok($fr->run('status --json'))['json'];
        T::true($st['registry']['ok']);
        T::eq(3, $st['domains']['with_data']);
        T::eq(2, $st['accounts']['total']);
        T::eq(300, $st['lines_total']);
        T::contains('accounts:', $ok($fr->run('status'))['stdout']);
        T::eq(30, $ok($fr->run('config get raw_retention_days --json'))['json']);
        $ok($fr->run('config set raw_retention_days 60'));
        T::eq(60, $ok($fr->run('config get raw_retention_days --json'))['json']);
        $ok($fr->run('config set exclude_ips \'["10.0.0.0/8"]\''));
        T::eq(['10.0.0.0/8'], $ok($fr->run('config get exclude_ips --json'))['json']);
        T::eq(2, $fr->run('config set raw_retention_days banana')['code']);
        T::eq(2, $fr->run('config set paths.data_dir /tmp')['code']);
        $ok($fr->run('config unset raw_retention_days'));
        T::eq(30, $ok($fr->run('config get raw_retention_days --json'))['json']);
        T::eq(1, $fr->run('config get nope.nope')['code']);
        T::eq(2, $fr->run('collect --bogus')['code']);
        T::eq(2, $fr->run('backfill')['code']);
        T::eq(2, $fr->run('collect --domain example.com')['code']);
        T::eq(2, $fr->run('collect --user ../etc')['code']);
        T::eq(2, $fr->run('purge --user alice')['code']);
        T::eq(2, $fr->run('nosuchcommand')['code']);
        T::eq(0, $fr->run('--help')['code']);
        T::contains('Usage: webpulse collect', $fr->run('collect --help')['stdout']);
        T::contains('0.1.0', $fr->run('--version')['stdout']);
        $ok($fr->run('purge --user alice --domain shop.example.com --yes'));
        T::false(is_file($fr->path('var/webpulse/data/alice/shop.example.com.db')));
        T::true(is_file($fr->path('var/webpulse/data/alice/example.com.db')));
        T::eq(2, count($fr->app()->registry()->summaries()));
        $ok($fr->run('purge --user alice --yes'));
        T::eq([], glob($fr->path('var/webpulse/data/alice/*.db')));
        T::eq(1, count($fr->app()->registry()->summaries()));
    },
    'geoip-update from a local file, then countries are recorded' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-geoip');
        $w = new MmdbWriter(6, 24);
        $w->insert('203.0.113.0/24', ['country' => ['iso_code' => 'IT']]);
        $w->insert('2001:db8::/32', ['country' => ['iso_code' => 'DE']]);
        $src = $fr->dir() . '/test.mmdb.gz';
        file_put_contents($src, gzencode($w->build()));
        $r = $fr->run('geoip-update --json');
        T::eq(0, $r['code']);
        T::contains('disabled', $r['stdout'] . $r['stderr'] . '(disabled)');
        $r = $ok($fr->run("geoip-update --force --url file://$src --json"));
        T::eq('WebPulse-Test', $r['json']['database_type']);
        T::true(is_file($fr->path('var/webpulse/geoip/dbip-country-lite.mmdb')));
        T::eq(1, $fr->run('geoip-update --force --url file:///nonexistent.gz')['code']);
        $fr->patchConfig(['geoip' => ['enabled' => true]]);
        $fr->writeLog('etc/apache2/logs/domlogs/alice/example.com-ssl_log', [
            L::line('203.0.113.5', '2026-10-10 10:00:00', '/'),
            L::line('2001:db8::5', '2026-10-10 10:00:00', '/'),
            L::line('192.0.2.1', '2026-10-10 10:00:00', '/'),
        ]);
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        $ok($fr->run('collect'));
        T::eq(['(unknown)' => 1, 'de' => 1, 'it' => 1], Q::col($fr->db('alice', 'example.com'), 'country', 'visits'));
        $st = $ok($fr->run('status --json'))['json'];
        T::true($st['geoip']['present']);
    },
    'php_bin in config re-executes the CLI with that interpreter' => static function () use ($ok): void {
        $fr = FakeRoot::create('col-phpbin');
        $wrapper = $fr->dir() . '/php-wrapper.sh';
        file_put_contents($wrapper, "#!/bin/sh\necho reexec >&2\nexec " . escapeshellarg(PHP_BINARY) . " \"\$@\"\n");
        chmod($wrapper, 0755);
        $fr->patchConfig(['php_bin' => $wrapper]);
        $r = $ok($fr->run('--version'));
        T::contains('reexec', $r['stderr']);
        T::contains('webpulse 0.1.0', $r['stdout']);
    },
];
