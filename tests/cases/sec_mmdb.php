<?php

declare(strict_types=1);

use WebPulse\Classify\MmdbReader;
use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\T;

/*
 * Security review L3 — hostile MaxMind DB files: pointer chains, pointer
 * fan-out (exponential expansion) and absurd metadata must fail fast with a
 * clear error instead of exhausting CPU or memory.
 */

/** Minimal MMDB encoder for hand-crafted (invalid) data sections. */
final class SecMmdb
{
    public static function ctrl(int $type, int $size): string
    {
        $extra = '';
        if ($size >= 29) {
            $extra = chr($size - 29);
            $size = 29;
        }
        return $type <= 7 ? chr(($type << 5) | $size) . $extra : chr($size) . chr($type - 7) . $extra;
    }

    public static function str(string $s): string
    {
        return self::ctrl(2, strlen($s)) . $s;
    }

    public static function u16(int $v): string
    {
        return self::ctrl(5, 2) . pack('n', $v);
    }

    public static function u32(int $v): string
    {
        return self::ctrl(6, 4) . pack('N', $v);
    }

    /** Pointer (relative to the data section) below 2048. */
    public static function ptr(int $p): string
    {
        return chr(0x20 | ($p >> 8)) . chr($p & 0xFF);
    }

    /** @param array<string,string> $entries key => encoded value */
    public static function map(array $entries): string
    {
        $out = self::ctrl(7, count($entries));
        foreach ($entries as $k => $v) {
            $out .= self::str((string) $k) . $v;
        }
        return $out;
    }

    /**
     * IPv4 database with one node whose two records point to $rootOffset of
     * $data; metadata fields can be overridden with encoded values.
     *
     * @param array<string,string> $meta
     */
    public static function file(string $data, int $rootOffset, array $meta = []): string
    {
        $rec = 1 + 16 + $rootOffset; // node_count + 16 + offset
        $tree = substr(pack('N', $rec), 1) . substr(pack('N', $rec), 1);
        $meta += ['node_count' => self::u32(1), 'record_size' => self::u16(24), 'ip_version' => self::u16(4),
            'database_type' => self::str('Hostile'), 'binary_format_major_version' => self::u16(2)];
        return $tree . str_repeat("\0", 16) . $data . "\xAB\xCD\xEFMaxMind.com" . self::map($meta);
    }
}

$write = static function (string $name, string $bin): string {
    $dir = FakeRoot::repo() . '/tests/tmp/sec-mmdb';
    @mkdir($dir, 0755, true);
    file_put_contents("$dir/$name.mmdb", $bin);
    return "$dir/$name.mmdb";
};

return [
    'L3: a pointer to a pointer is rejected' => static function () use ($write): void {
        // 0: pointer -> 2: pointer -> 4: pointer -> ... -> string; root map: 28 entries all pointing to 0
        $data = '';
        for ($i = 0; $i < 10; $i++) {
            $data .= SecMmdb::ptr(strlen($data) + 2);
        }
        $data .= SecMmdb::str('end');
        $entries = [];
        for ($i = 0; $i < 28; $i++) {
            $entries["k$i"] = SecMmdb::ptr(0);
        }
        $root = strlen($data);
        $data .= SecMmdb::map($entries);
        $r = new MmdbReader($write('chain', SecMmdb::file($data, $root)));
        T::eq('Hostile', $r->metadata()['database_type']);
        $t = microtime(true);
        T::throws(RuntimeException::class, static fn () => $r->get('1.2.3.4'), 'pointer to a pointer');
        T::less(1.0, microtime(true) - $t);
    },
    'L3: pointer fan-out cannot expand exponentially' => static function () use ($write): void {
        // level k: an array of 28 pointers to level k+1; 8 levels = 28^8 values once expanded
        $data = SecMmdb::str('leaf');
        $prev = 0;
        for ($level = 0; $level < 8; $level++) {
            $off = strlen($data);
            $data .= SecMmdb::ctrl(11, 28) . str_repeat(SecMmdb::ptr($prev), 28);
            $prev = $off;
        }
        $root = strlen($data);
        $entries = [];
        for ($i = 0; $i < 28; $i++) {
            $entries["k$i"] = SecMmdb::ptr($prev);
        }
        $data .= SecMmdb::map($entries);
        $r = new MmdbReader($write('fanout', SecMmdb::file($data, $root)));
        $t = microtime(true);
        $m = memory_get_usage();
        T::throws(RuntimeException::class, static fn () => $r->get('1.2.3.4'), 'too large');
        T::less(1.0, microtime(true) - $t, 'fails fast');
        T::less(8 * 1048576, memory_get_usage() - $m);
        // a moderate fan-out (shared strings, the normal use of pointers) still decodes
        $small = SecMmdb::str('IT');
        $map = SecMmdb::map(['a' => SecMmdb::ptr(0), 'b' => SecMmdb::ptr(0), 'c' => SecMmdb::map(['iso_code' => SecMmdb::ptr(0)])]);
        $r2 = new MmdbReader($write('shared', SecMmdb::file($small . $map, strlen($small))));
        T::eq(['a' => 'IT', 'b' => 'IT', 'c' => ['iso_code' => 'IT']], $r2->get('1.2.3.4'));
    },
    'L3: metadata is validated before the search tree is used' => static function () use ($write): void {
        $data = SecMmdb::str('x');
        $bad = [
            'huge node count' => ['node_count' => SecMmdb::u32(0x7FFFFFFF)],
            'zero node count' => ['node_count' => SecMmdb::u32(0)],
            'bad ip version' => ['ip_version' => SecMmdb::u16(5)],
            'bad record size' => ['record_size' => SecMmdb::u16(20)],
            'string node count' => ['node_count' => SecMmdb::str('1')],
        ];
        foreach ($bad as $what => $meta) {
            $f = $write('meta-' . str_replace(' ', '-', $what), SecMmdb::file($data, 0, $meta));
            T::throws(RuntimeException::class, static fn () => new MmdbReader($f), null);
        }
        // metadata that fans out is bounded too
        $big = SecMmdb::ctrl(11, 28) . str_repeat(SecMmdb::ctrl(11, 28) . str_repeat(SecMmdb::ctrl(11, 28) . str_repeat(SecMmdb::str('z'), 28), 28), 28);
        $f = $write('meta-big', SecMmdb::file($data, 0, ['languages' => $big]));
        T::throws(RuntimeException::class, static fn () => new MmdbReader($f), 'too large');
    },
];
