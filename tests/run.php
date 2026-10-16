<?php

declare(strict_types=1);

/*
 * Minimal test runner for the packaging tests (installer, shims, hooks, static
 * checks). The engine and the web layer are tested by `cargo test`; PHP is only
 * used here because the packaging has PHP shims and the cPanel box has PHP.
 *
 *   php tests/run.php [filter...]
 *
 * Every tests/cases/pkg_*.php returns an array of name => callable. A test fails
 * when it throws; PkgSkip marks it skipped. Output is TAP-like; the exit code is
 * 1 when a test failed. A filter matches the case file name (e.g. "pkg_hooks") or
 * a substring of "file: test name".
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
require_once $root . '/tests/cases/pkg_support.php'; // PkgSupport, PkgSkip
@mkdir($root . '/tests/tmp', 0755, true);

$filters = array_slice($argv, 1);
$files = glob($root . '/tests/cases/pkg_*.php') ?: [];
sort($files);
$n = $pass = $fail = $skip = 0;
$failures = [];
$t0 = microtime(true);
foreach ($files as $file) {
    $base = basename($file, '.php');
    if ($base === 'pkg_support') {
        require_once $file;
        continue;
    }
    $tests = require $file;
    if (!is_array($tests)) {
        echo "not ok - $base: case file must return an array\n";
        $fail++;
        continue;
    }
    foreach ($tests as $name => $fn) {
        $id = str_starts_with($name, $base) ? $name : "$base: $name";
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
        try {
            $fn();
            $pass++;
            printf("ok %d - %s (%d ms)\n", $n, $id, (microtime(true) - $t) * 1000);
        } catch (PkgSkip $e) {
            $skip++;
            printf("ok %d - %s # SKIP %s\n", $n, $id, $e->getMessage());
        } catch (Throwable $e) {
            $fail++;
            $failures[] = $id;
            printf("not ok %d - %s\n", $n, $id);
            $where = $e->getFile() . ':' . $e->getLine();
            foreach ($e->getTrace() as $fr) {
                if (isset($fr['file']) && str_contains($fr['file'], '/tests/cases/')) {
                    $where = $fr['file'] . ':' . ($fr['line'] ?? 0);
                    break;
                }
            }
            echo '  # ' . get_class($e) . ': ' . str_replace("\n", "\n  # ", $e->getMessage()) . "\n  # at $where\n";
        }
    }
}
printf("1..%d\n# tests %d, passed %d, failed %d, skipped %d, time %.1fs\n", $n, $n, $pass, $fail, $skip, microtime(true) - $t0);
foreach ($failures as $f) {
    echo "# FAILED: $f\n";
}
exit($fail > 0 ? 1 : 0);
