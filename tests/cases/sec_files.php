<?php

declare(strict_types=1);

use WebPulse\Collect\UserSettings;
use WebPulse\Log;
use WebPulse\Log\LogSource;
use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\Skip;
use WebPulse\Tests\T;
use WebPulse\Util\Fs;

/*
 * Security review M1, L1, L4, L6 — files in account-writable places:
 * FIFOs swapped in after lstat() (must never block the collector),
 * decompression bombs in ~/logs, removal of data directories without
 * following symlinks, and log injection through file names / JSON keys.
 */

/**
 * Runs PHP code in a child process (autoloader loaded) with a deadline, so a
 * regression that blocks cannot hang the test suite.
 *
 * @return array{code:int,out:string,err:string,timed_out:bool}
 */
$php = static function (string $code, float $timeout = 20.0, string $memory = '512M'): array {
    $script = '<?php declare(strict_types=1); require ' . var_export(FakeRoot::repo() . '/lib/autoload.php', true) . ";\n" . $code;
    $file = FakeRoot::repo() . '/tests/tmp/sec-files-' . getmypid() . '-' . bin2hex(random_bytes(3)) . '.php';
    file_put_contents($file, $script);
    $proc = proc_open([PHP_BINARY, '-d', 'memory_limit=' . $memory, $file], [0 => ['file', '/dev/null', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
    stream_set_blocking($pipes[1], false);
    stream_set_blocking($pipes[2], false);
    $out = $err = '';
    $deadline = microtime(true) + $timeout;
    $timedOut = false;
    while (true) {
        $out .= (string) stream_get_contents($pipes[1]);
        $err .= (string) stream_get_contents($pipes[2]);
        $st = proc_get_status($proc);
        if (!$st['running']) {
            $code = $st['exitcode'];
            break;
        }
        if (microtime(true) > $deadline) {
            proc_terminate($proc, 9);
            $timedOut = true;
            $code = -1;
            break;
        }
        usleep(20000);
    }
    $out .= (string) stream_get_contents($pipes[1]);
    $err .= (string) stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    proc_close($proc);
    @unlink($file);
    return ['code' => $code, 'out' => $out, 'err' => $err, 'timed_out' => $timedOut];
};

return [
    'M1: a FIFO swapped in after lstat() is refused without blocking' => static function () use ($php): void {
        if (!function_exists('posix_mkfifo')) {
            throw new Skip('posix_mkfifo unavailable');
        }
        $fr = FakeRoot::create('sec-files-fifo');
        $dir = $fr->path('var/webpulse/data/alice/user');
        @mkdir($dir, 0700, true);
        $settings = $dir . '/settings.json';
        file_put_contents($dir . '/regular.json', '{"version":1}');
        posix_mkfifo($settings, 0600);
        $archive = $fr->path('home/alice/logs/example.com-Oct-2026.gz');
        posix_mkfifo($archive, 0600);
        $live = $fr->path('etc/apache2/logs/domlogs/alice/example.com');
        @mkdir(dirname($live), 0755, true);
        posix_mkfifo($live, 0600);
        $r = $php(sprintf(<<<'PHP'
            $out = [];
            // the race: lstat() saw a regular file, the path is now a FIFO with no writer
            $lst = lstat(%1$s);
            try { WebPulse\Util\Fs::readRegular(%2$s, 65536, $lst); $out[] = 'read'; } catch (RuntimeException $e) { $out[] = $e->getMessage(); }
            $cfg = WebPulse\Config::load(%3$s);
            $s = WebPulse\Collect\UserSettings::read(new WebPulse\Paths($cfg, '/'), 'alice', $err);
            $out[] = $s === WebPulse\Collect\UserSettings::defaults() ? 'defaults' : 'custom';
            $out[] = $err;
            foreach ([[%4$s, true], [%5$s, false]] as [$f, $gz]) {
                $src = new WebPulse\Log\LogSource($f, false, null, $gz);
                try { $src->open(); $out[] = 'opened'; } catch (RuntimeException $e) { $out[] = $e->getMessage(); }
            }
            echo json_encode($out);
            PHP, var_export($dir . '/regular.json', true), var_export($settings, true), var_export($fr->configPath(), true),
            var_export($archive, true), var_export($live, true)), 10.0);
        T::false($r['timed_out'], 'the reads must not block on the FIFO');
        T::eq(0, $r['code'], $r['err']);
        $o = json_decode($r['out'], true);
        T::contains('changed while opening', $o[0]);
        T::eq('defaults', $o[1]);
        T::contains('not a regular file', (string) $o[2]);
        T::contains('not a regular file', $o[3], 'archive FIFO');
        T::contains('not a regular file', $o[4], 'live log FIFO');
        // the regular file is read normally, with the size cap
        T::eq('{"version":1}', Fs::readRegular($dir . '/regular.json', 100));
        T::throws(RuntimeException::class, static fn () => Fs::readRegular($dir . '/regular.json', 5), 'larger than');
        unlink($settings);
        file_put_contents($settings, '{"version":1,"ui":{"dark":"dark"}}');
        T::eq('dark', UserSettings::read($fr->app()->paths, 'alice', $err)['ui']['dark']);
        T::null($err);
    },
    'L1: a gzip bomb in ~/logs is read in bounded memory and counted as bad lines' => static function () use ($php): void {
        $fr = FakeRoot::create('sec-files-bomb');
        $bomb = $fr->path('home/alice/logs/example.com-Oct-2026.gz');
        $z = deflate_init(ZLIB_ENCODING_GZIP, ['level' => 9]);
        $fh = fopen($bomb, 'wb');
        $mb = str_repeat('a', 1048576);
        for ($i = 0; $i < 200; $i++) {
            fwrite($fh, deflate_add($z, $mb, ZLIB_NO_FLUSH));
        }
        fwrite($fh, deflate_add($z, '', ZLIB_FINISH));
        fclose($fh);
        T::less(1048576, filesize($bomb), 'tiny archive');
        $r = $php(sprintf(<<<'PHP'
            $src = new WebPulse\Log\LogSource(%s, false, null, true);
            $src->open();
            $parser = new WebPulse\Log\LogParser();
            $lines = $bad = 0;
            while (($l = $src->next()) !== null) {
                $lines++;
                if ($parser->parse($l) === null) { $bad++; }
            }
            echo json_encode(['lines' => $lines, 'bad' => $bad, 'peak' => memory_get_peak_usage(true)]);
            PHP, var_export($bomb, true)), 120.0, '128M');
        T::false($r['timed_out']);
        T::eq(0, $r['code'], $r['err'] . $r['out']);
        $o = json_decode($r['out'], true);
        T::true($o['lines'] >= 1 && $o['bad'] === $o['lines'], 'the bomb is one (cut) bad line: ' . $r['out']);
        T::less(128 * 1048576, $o['peak']);
        // and the backfill survives it, counting a bad line
        $fr->setNow(strtotime('2026-11-02 12:00:00 UTC'));
        $b = $fr->run('backfill --user alice --json');
        T::eq(0, $b['code'], $b['stderr'] . $b['stdout']);
        T::true($b['json']['bad'] >= 1);
        T::eq(0, $b['json']['imported']);
        // normal archives still decompress (several members, slices across member boundaries)
        $multi = $fr->dir() . '/multi.gz';
        $lines = [];
        for ($i = 0; $i < 20000; $i++) {
            $lines[] = "line $i " . str_repeat(chr(97 + $i % 26), $i % 50);
        }
        file_put_contents($multi, gzencode(implode("\n", array_slice($lines, 0, 7000)) . "\n") . gzencode(implode("\n", array_slice($lines, 7000)) . "\n"));
        $src = new LogSource($multi, false, null, true);
        $src->open();
        $got = [];
        while (($l = $src->next()) !== null) {
            $got[] = $l;
        }
        T::eq($lines, $got);
    },
    'L4: removeTree never follows symlinks and hands foreign directories to rm' => static function (): void {
        $fr = FakeRoot::create('sec-files-rm');
        $outside = $fr->dir() . '/outside';
        mkdir($outside . '/sub', 0755, true);
        file_put_contents($outside . '/keep.txt', 'keep');
        file_put_contents($outside . '/sub/keep.txt', 'keep');
        $build = static function (string $d) use ($outside): void {
            mkdir($d . '/user', 0700, true);
            file_put_contents($d . '/example.com.db', 'x');
            file_put_contents($d . '/user/settings.json', '{}');
            symlink($outside, $d . '/user/link-dir');
            symlink($outside . '/keep.txt', $d . '/user/link-file');
            mkdir($d . '/user/nested/deeper', 0755, true);
            symlink($outside . '/sub', $d . '/user/nested/deeper/x');
        };
        $a = $fr->dir() . '/data-a';
        $build($a);
        T::true(Fs::removeTree($a));
        T::false(file_exists($a));
        $b = $fr->dir() . '/data-b';
        $build($b);
        // pretend every directory belongs to someone else: the walk goes through `rm -rf --one-file-system`
        T::true(Fs::removeTree($b, -1));
        T::false(file_exists($b));
        // a symlink given as the root is removed, not followed
        symlink($outside, $fr->dir() . '/root-link');
        T::true(Fs::removeTree($fr->dir() . '/root-link'));
        T::eq('keep', file_get_contents($outside . '/keep.txt'));
        T::eq('keep', file_get_contents($outside . '/sub/keep.txt'));
        T::true(Fs::removeTree($fr->dir() . '/does-not-exist'));
    },
    'L6: log messages cannot forge lines (file names, settings keys)' => static function (): void {
        $fr = FakeRoot::create('sec-files-log');
        $app = $fr->app();
        $app->log->warn("cannot read archive /home/alice/logs/x.gz\n2026-01-01T00:00:00+00:00 [error] 1 forged\r\e[2J");
        try {
            UserSettings::validate(["evil\n2026-01-01 [error] forged" => 1, 'domains' => ['example.com' => ["k\r\nx" => 1]]]);
        } catch (InvalidArgumentException $e) {
            T::false(str_contains($e->getMessage(), "\n") || str_contains($e->getMessage(), "\r"), 'validation message is one line');
            $app->log->warn('ignoring settings.json of alice: ' . $e->getMessage());
        }
        // archive names with control characters reach the log through the backfill
        file_put_contents($fr->path("home/alice/logs/example.com-\n[error] forged.gz"), 'not gzip');
        $fr->setNow(strtotime('2026-11-02 12:00:00 UTC'));
        $fr->run('backfill --user alice');
        $log = (string) file_get_contents($fr->path('var/webpulse/log/webpulse.log'));
        foreach (explode("\n", rtrim($log, "\n")) as $line) {
            T::true(preg_match('/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d \[(debug|info|warn|error)\] \d+ /', $line) === 1, "well-formed log line: $line");
        }
        T::false(str_contains($log, "\e"), 'no escape sequences');
        T::true(substr_count($log, 'forged') >= 3, 'the content is kept, neutralised');
        T::eq('a?b', Log::sanitize("a\nb"));
        T::eq(Log::MAX_MESSAGE, strlen(Log::sanitize(str_repeat('x', 100000))));
    },
];
