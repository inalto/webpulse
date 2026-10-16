<?php

declare(strict_types=1);

use WebPulse\Classify\GeoIp;
use WebPulse\Classify\MmdbReader;
use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\MmdbWriter;
use WebPulse\Tests\Skip;
use WebPulse\Tests\T;

$tmp = static function (string $name, string $bin): string {
    $dir = FakeRoot::repo() . '/tests/tmp/geoip';
    @mkdir($dir, 0755, true);
    file_put_contents("$dir/$name", $bin);
    return "$dir/$name";
};

$build = static function (int $ipVersion, int $recordSize, int $filler = 0): string {
    $w = new MmdbWriter($ipVersion, $recordSize, ['IT', 'Italy', 'Europe'], $filler);
    $it = ['country' => ['iso_code' => 'IT', 'names' => ['en' => 'Italy', 'de' => 'Italien']], 'continent' => ['code' => 'EU', 'names' => ['en' => 'Europe']]];
    $w->insert('151.0.0.0/8', $it);
    $w->insert('2.32.0.0/11', $it);
    $w->insert('8.8.8.0/24', ['country' => ['iso_code' => 'US'], 'location' => ['latitude' => 37.751, 'longitude' => -97.822]]);
    $w->insert('81.2.69.0/24', [
        'city' => ['names' => ['en' => 'London']], 'country' => ['iso_code' => 'GB'],
        'subdivisions' => [['names' => ['en' => 'England']]],
        'location' => ['latitude' => 51.5142, 'longitude' => -0.0931],
    ]);
    $w->insert('203.0.113.0/24', ['registered_country' => ['iso_code' => 'JP']]);
    $w->insert('198.51.100.0/24', [
        'u16' => MmdbWriter::type('uint16', 65535),
        'u32' => MmdbWriter::type('uint32', 4294967295),
        'u64' => MmdbWriter::type('uint64', '18446744073709551615'),
        'u64s' => MmdbWriter::type('uint64', 1234),
        'u128' => MmdbWriter::type('uint128', '340282366920938463463374607431768211455'),
        'i32' => MmdbWriter::type('int32', -5),
        'i32p' => MmdbWriter::type('int32', 70000),
        'd' => 3.25,
        'f' => MmdbWriter::type('float', 1.5),
        'b' => MmdbWriter::type('bytes', "\x00\x01\xff"),
        't' => true,
        'fl' => false,
        'arr' => [1, 'two', ['three' => 3]],
        'long' => str_repeat('x', 300),
        'longer' => str_repeat('y', 70000),
        'empty' => [],
        'shared' => 'Europe',
    ]);
    if ($ipVersion === 6) {
        $w->insert('2001:db8::/32', ['country' => ['iso_code' => 'DE']]);
        $w->insert('2a01:4f8::/29', ['country' => ['iso_code' => 'DE']]);
    }
    return $w->build();
};

$cases = [];
foreach ([[6, 24], [6, 28], [6, 32], [4, 24], [4, 28], [4, 32]] as [$v, $rs]) {
    $cases["mmdb lookups: ipv$v tree, $rs-bit records"] = static function () use ($v, $rs, $build, $tmp): void {
        $file = $tmp("t$v-$rs.mmdb", $build($v, $rs));
        $r = new MmdbReader($file);
        T::eq($rs, $r->metadata()['record_size']);
        T::eq('WebPulse-Test', $r->metadata()['database_type']);
        T::eq('IT', $r->get('151.12.34.56')['country']['iso_code']);
        T::eq('Italien', $r->get('151.1.1.1')['country']['names']['de']);
        T::eq('Europe', $r->get('2.40.1.1')['continent']['names']['en'], 'shared string through pointer');
        T::eq('US', $r->get('8.8.8.8')['country']['iso_code']);
        T::null($r->get('8.8.9.8'));
        T::null($r->get('10.0.0.1'));
        T::null($r->get('not an ip'));
        [$rec, $prefix] = $r->getWithPrefix('8.8.8.8');
        T::eq(24, $prefix, "prefix in IPv4 bits");
        T::notNull($rec);
        T::eq('US', $r->get('::ffff:8.8.8.8')['country']['iso_code'] ?? null, 'IPv4-mapped');
        if ($v === 6) {
            T::eq('DE', $r->get('2001:db8:1::1')['country']['iso_code']);
            T::eq('DE', $r->get('2a01:4f8:c0c:1::1')['country']['iso_code']);
            T::null($r->get('2001:db9::1'));
        } else {
            T::null($r->get('2001:db8:1::1'));
        }
    };
}
$cases['mmdb data types and pointer sizes'] = static function () use ($build, $tmp): void {
    foreach ([0, 3000, 600000] as $filler) {
        $file = $tmp("types-$filler.mmdb", $build(6, 28, $filler));
        $x = (new MmdbReader($file))->get('198.51.100.1');
        T::eq(65535, $x['u16']);
        T::eq(4294967295, $x['u32']);
        T::eq('18446744073709551615', $x['u64']);
        T::eq(1234, $x['u64s']);
        T::eq('340282366920938463463374607431768211455', $x['u128']);
        T::eq(-5, $x['i32']);
        T::eq(70000, $x['i32p']);
        T::eq(3.25, $x['d']);
        T::eq(1.5, $x['f']);
        T::eq("\x00\x01\xff", $x['b']);
        T::eq(true, $x['t']);
        T::eq(false, $x['fl']);
        T::eq([1, 'two', ['three' => 3]], $x['arr']);
        T::eq(300, strlen($x['long']));
        T::eq(70000, strlen($x['longer']));
        T::eq([], $x['empty']);
        T::eq('Europe', $x['shared'], "pointer with filler $filler");
        T::eq('IT', (new MmdbReader($file))->get('151.1.1.1')['country']['iso_code'], "pointer with filler $filler");
    }
};
$cases['mmdb reader without loading the file in memory'] = static function () use ($build, $tmp): void {
    $file = $tmp('stream.mmdb', $build(6, 32));
    $r = new MmdbReader($file, 0);
    T::eq('IT', $r->get('151.2.3.4')['country']['iso_code']);
    T::eq('GB', $r->get('81.2.69.160')['country']['iso_code']);
};
$cases['mmdb rejects non-mmdb files'] = static function () use ($tmp): void {
    $f = $tmp('bad.mmdb', str_repeat('nope', 100));
    T::throws(\RuntimeException::class, static fn () => new MmdbReader($f), 'metadata marker');
    T::throws(\RuntimeException::class, static fn () => new MmdbReader($f . '.missing'));
};
$cases['geoip facade: country, city, registered country fallback, no db'] = static function () use ($build, $tmp): void {
    $file = $tmp('facade.mmdb', $build(6, 24));
    $g = new GeoIp($file, $file);
    T::true($g->available());
    T::eq('it', $g->country('151.1.1.1'));
    T::eq('jp', $g->country('203.0.113.5'));
    T::null($g->country('10.1.1.1'));
    $c = $g->city('81.2.69.160');
    T::eq(['London', 'gb', 'England'], [$c['name'], $c['country'], $c['region']]);
    T::null($g->city('8.8.8.8'), 'no city name');
    $none = new GeoIp(null, null);
    T::false($none->available());
    T::null($none->country('8.8.8.8'));
    T::null($none->city('8.8.8.8'));
    $broken = new GeoIp($tmp('broken.mmdb', 'xx'), null);
    T::false($broken->available());
};
$cases['real DB-IP country lite database'] = static function (): void {
    $f = FakeRoot::repo() . '/tests/fixtures/geoip/dbip-country-lite.mmdb';
    if (!is_file($f)) {
        throw new Skip('run `make geoip-fixture` to download the DB-IP database');
    }
    $g = new GeoIp($f);
    T::eq('us', $g->country('8.8.8.8'));
    T::eq('it', $g->country('151.1.1.1'));
    T::eq('gb', $g->country('81.2.69.160'));
    T::null($g->country('127.0.0.1'));
    T::null($g->country('::1'));
    T::notNull($g->country('2001:4860:4860::8888'));
    $meta = $g->metadata();
    T::eq(6, $meta['ip_version']);
    T::true(str_contains((string) $meta['database_type'], 'Country'));
};
return $cases;
