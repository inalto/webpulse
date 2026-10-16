<?php

declare(strict_types=1);

/*
 * Dependency-free test runner.
 *
 *   php tests/run.php [filter…]
 *
 * Every tests/cases/*.php returns an array of name => callable. A test fails
 * when it throws; WebPulse\Tests\Skip marks it skipped. Output is TAP-like;
 * the exit code is 1 when a test failed. A filter matches the case file name
 * (e.g. "engine_") or "file:test name" substrings.
 */

error_reporting(E_ALL);
ini_set('display_errors', '1');
ini_set('memory_limit', '512M');
set_error_handler(static function (int $no, string $msg, string $file, int $line): bool {
    if (!(error_reporting() & $no)) {
        return false;
    }
    throw new ErrorException($msg, 0, $no, $file, $line);
});

$root = dirname(__DIR__);
require $root . '/lib/autoload.php';
spl_autoload_register(static function (string $class) use ($root): void {
    if (str_starts_with($class, 'WebPulse\\Tests\\')) {
        $f = $root . '/tests/support/' . str_replace('\\', '/', substr($class, 15)) . '.php';
        if (is_file($f)) {
            require $f;
        }
    }
});
@mkdir($root . '/tests/tmp', 0755, true);

$filters = array_slice($argv, 1);
$files = glob($root . '/tests/cases/*.php') ?: [];
sort($files);
$n = $pass = $fail = $skip = 0;
$failures = [];
$t0 = microtime(true);
foreach ($files as $file) {
    $base = basename($file, '.php');
    $tests = require $file;
    if (!is_array($tests)) {
        echo "not ok - $base: case file must return an array\n";
        $fail++;
        continue;
    }
    foreach ($tests as $name => $fn) {
        $id = "$base: $name";
        if ($filters !== []) {
            $hit = false;
            foreach ($filters as $f) {
                if (str_contains($id, $f) || str_contains($base, $f)) {
                    $hit = true;
                }
            }
            if (!$hit) {
                continue;
            }
        }
        $n++;
        $t = microtime(true);
        $saveEnv = getenv('WEBPULSE_NOW');
        try {
            $fn();
            $pass++;
            printf("ok %d - %s (%d ms)\n", $n, $id, (microtime(true) - $t) * 1000);
        } catch (WebPulse\Tests\Skip $e) {
            $skip++;
            printf("ok %d - %s # SKIP %s\n", $n, $id, $e->getMessage());
        } catch (Throwable $e) {
            $fail++;
            $failures[] = $id;
            printf("not ok %d - %s\n", $n, $id);
            $where = $e->getFile() . ':' . $e->getLine();
            foreach ($e->getTrace() as $fr) {
                if (isset($fr['file']) && str_contains($fr['file'], '/tests/cases/')) {
                    $where = $fr['file'] . ':' . $fr['line'];
                    break;
                }
            }
            echo '  # ' . get_class($e) . ': ' . str_replace("\n", "\n  # ", $e->getMessage()) . "\n  # at $where\n";
        } finally {
            putenv($saveEnv === false ? 'WEBPULSE_NOW' : 'WEBPULSE_NOW=' . $saveEnv);
        }
    }
}
printf("1..%d\n# tests %d, passed %d, failed %d, skipped %d, time %.1fs, peak memory %.1f MiB\n",
    $n, $n, $pass, $fail, $skip, microtime(true) - $t0, memory_get_peak_usage(true) / 1048576);
foreach ($failures as $f) {
    echo "# FAILED: $f\n";
}
exit($fail > 0 ? 1 : 0);
