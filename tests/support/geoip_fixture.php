<?php

declare(strict_types=1);

/*
 * Downloads the free DB-IP country lite database into
 * tests/fixtures/geoip/dbip-country-lite.mmdb (current month, else the
 * previous one). Used by `make geoip-fixture`; tests skip without it.
 */

require dirname(__DIR__, 2) . '/lib/autoload.php';

$dest = dirname(__DIR__) . '/fixtures/geoip/dbip-country-lite.mmdb';
@mkdir(dirname($dest), 0755, true);
$t = time();
foreach ([gmdate('Y-m', $t), gmdate('Y-m', (int) strtotime(gmdate('Y-m-01', $t) . ' -1 month'))] as $ym) {
    [$y, $m] = explode('-', $ym);
    $url = "https://download.db-ip.com/free/dbip-country-lite-$y-$m.mmdb.gz";
    try {
        $d = WebPulse\Cli\Command\GeoipUpdateCommand::fetch($url);
    } catch (Throwable $e) {
        fwrite(STDERR, $e->getMessage() . "\n");
        continue;
    }
    $raw = gzdecode($d);
    if ($raw === false) {
        fwrite(STDERR, "$url: not gzip data\n");
        continue;
    }
    file_put_contents($dest, $raw);
    echo "downloaded $url -> $dest (" . strlen($raw) . " bytes)\n";
    exit(0);
}
exit(1);
