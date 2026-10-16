<?php

declare(strict_types=1);

use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\Lines as L;
use WebPulse\Tests\Skip;
use WebPulse\Tests\T;

return [
    'performance: 200k lines collected in < 10 s and < 128 MB' => static function (): void {
        if (getenv('WEBPULSE_SKIP_PERF') === '1') {
            throw new Skip('WEBPULSE_SKIP_PERF=1');
        }
        $fr = FakeRoot::create('perf');
        $file = $fr->domlog('alice', 'example.com-ssl_log');
        $r = FakeRoot::exec([PHP_BINARY, FakeRoot::repo() . '/tests/gen_logs.php', '--seed', '11', '--days', '14', '--lines', '200000',
            '--end', '2026-10-16', '--no-archive', '--single', $file], []);
        T::eq(0, $r['code'], $r['stderr']);
        // keep exactly 200 000 lines
        $in = fopen($file, 'rb');
        $out = fopen($file . '.tmp', 'wb');
        for ($i = 0; $i < 200000 && ($l = fgets($in)) !== false; $i++) {
            fwrite($out, $l);
        }
        fclose($in);
        fclose($out);
        rename($file . '.tmp', $file);
        T::eq(200000, $i, 'generator produced enough lines');
        $fr->setNow(L::ts('2026-10-17 03:00:00'));
        $t = microtime(true);
        $r = $fr->run('collect --json');
        $wall = microtime(true) - $t;
        T::eq(0, $r['code'], $r['stderr']);
        $j = $r['json'];
        T::eq(200000, $j['lines']);
        printf("  # perf: %d lines in %.2fs wall (%.2fs in collect) = %d lines/s, peak memory %.1f MiB\n",
            $j['lines'], $wall, $j['elapsed'], $j['lines_per_sec'], $j['peak_memory'] / 1048576);
        T::less(10.0, $wall, 'wall time');
        T::less(128 * 1048576, $j['peak_memory'], 'peak memory');
        // a second run over the same file is a no-op and fast
        $t = microtime(true);
        $r2 = $fr->run('collect --json');
        T::eq(0, $r2['json']['lines']);
        T::less(2.0, microtime(true) - $t, 'idle run');
    },
];
