<?php

declare(strict_types=1);

/*
 * Builds tests/tmp/<name> (default "dev") with 45 days of generated logs,
 * runs setup + collect + backfill and prints the config path. Used by
 * `make fixtures` / `make serve`.
 *
 *   php tests/support/devenv.php [name] [--lines N] [--seed N]
 */

require dirname(__DIR__, 2) . '/lib/autoload.php';
require __DIR__ . '/FakeRoot.php';

$name = 'dev';
$lines = 60000;
$seed = 42;
for ($i = 1; $i < $argc; $i++) {
    if ($argv[$i] === '--lines') {
        $lines = (int) $argv[++$i];
    } elseif ($argv[$i] === '--seed') {
        $seed = (int) $argv[++$i];
    } else {
        $name = $argv[$i];
    }
}
$fr = WebPulse\Tests\FakeRoot::create($name);
// use the DB-IP fixture (make geoip-fixture) when available so that countries show up
$geo = dirname(__DIR__) . '/fixtures/geoip/dbip-country-lite.mmdb';
if (is_file($geo)) {
    @mkdir($fr->path('var/webpulse/geoip'), 0755, true);
    copy($geo, $fr->path('var/webpulse/geoip/dbip-country-lite.mmdb'));
    $fr->patchConfig(['geoip' => ['enabled' => true]]);
}
$end = date('Y-m-d');
$sum = $fr->generateLogs(['seed' => $seed, 'days' => 45, 'lines' => $lines, 'end' => $end, 'max-ts' => time()]);
foreach (['setup --quiet', 'collect --quiet', 'backfill --user alice --quiet', 'rollup --quiet'] as $cmd) {
    $r = $fr->run($cmd);
    if ($r['code'] !== 0) {
        fwrite(STDERR, "webpulse $cmd failed ({$r['code']}): {$r['stderr']}{$r['stdout']}\n");
        exit(1);
    }
}
fwrite(STDERR, sprintf("generated %d lines (seed %d, 45 days to %s) in %s\n", $sum['lines'], $seed, $end, $fr->root()));
echo $fr->configPath(), "\n";
