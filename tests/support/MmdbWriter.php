<?php

declare(strict_types=1);

namespace WebPulse\Tests;

/**
 * Minimal MaxMind DB writer for tests: builds a search tree for a list of
 * networks and encodes their records (every data type, pointers included),
 * with 24, 28 or 32-bit records and IPv4 or IPv6 trees.
 *
 * Values: PHP int (uint32 by default), MmdbWriter::type($type, $value) for an
 * explicit type ('uint16','uint32','uint64','uint128','int32','double',
 * 'float','bytes','boolean'), strings, bool, float (double), lists (array),
 * assoc arrays (map). Strings listed in $shared are emitted once and then
 * referenced through pointers.
 */
final class MmdbWriter
{
    /** @var array<int,array{0:int|string|null,1:int|string|null}> */
    private array $nodes = [[null, null]];
    private string $data = '';
    /** @var array<string,int> */
    private array $stringOffsets = [];

    /** @param string[] $shared strings to share through pointers */
    public function __construct(private int $ipVersion = 6, private int $recordSize = 28, private array $shared = [], private int $filler = 0)
    {
        if ($filler > 0) {
            // a large bytes field at the start of the data section pushes later offsets past the 2048 / 526336 pointer thresholds
            $this->data .= $this->encode(self::type('bytes', str_repeat("\0", $filler)));
        }
    }

    /** Typed value wrapper. */
    public static function type(string $type, mixed $value): array
    {
        return ['__mmdb_type' => $type, 'v' => $value];
    }

    /** Adds a network ("1.2.3.0/24", "2001:db8::/32") with its record. */
    public function insert(string $cidr, mixed $record): void
    {
        [$addr, $bits] = explode('/', $cidr) + [1 => null];
        $packed = inet_pton($addr);
        $len = strlen($packed) * 8;
        $bits = $bits === null ? $len : (int) $bits;
        if (strlen($packed) === 4 && $this->ipVersion === 6) {
            $packed = str_repeat("\0", 12) . $packed;
            $bits += 96;
        }
        $bin = $this->encode($record); // may append shared strings first
        $offset = strlen($this->data);
        $this->data .= $bin;
        $node = 0;
        for ($i = 0; $i < $bits; $i++) {
            $bit = (ord($packed[$i >> 3]) >> (7 - ($i & 7))) & 1;
            if ($i === $bits - 1) {
                $this->nodes[$node][$bit] = 'd' . $offset;
                break;
            }
            $next = $this->nodes[$node][$bit];
            if (!is_int($next)) {
                $this->nodes[] = [null, null];
                $next = count($this->nodes) - 1;
                $this->nodes[$node][$bit] = $next;
            }
            $node = $next;
        }
    }

    /** Serialises the database. */
    public function build(array $meta = []): string
    {
        $count = count($this->nodes);
        $tree = '';
        foreach ($this->nodes as [$l, $r]) {
            $vals = [];
            foreach ([$l, $r] as $x) {
                if ($x === null) {
                    $vals[] = $count;
                } elseif (is_int($x)) {
                    $vals[] = $x;
                } else {
                    $vals[] = $count + 16 + (int) substr($x, 1);
                }
            }
            $tree .= $this->node($vals[0], $vals[1]);
        }
        $metadata = $meta + [
            'binary_format_major_version' => self::type('uint16', 2),
            'binary_format_minor_version' => self::type('uint16', 0),
            'build_epoch' => self::type('uint64', 1790000000),
            'database_type' => 'WebPulse-Test',
            'description' => ['en' => 'test database'],
            'ip_version' => self::type('uint16', $this->ipVersion),
            'languages' => ['en'],
            'node_count' => self::type('uint32', $count),
            'record_size' => self::type('uint16', $this->recordSize),
        ];
        $saveShared = $this->shared;
        $this->shared = [];
        $saveOffsets = $this->stringOffsets;
        $this->stringOffsets = [];
        $metaBin = $this->encode($metadata);
        $this->shared = $saveShared;
        $this->stringOffsets = $saveOffsets;
        return $tree . str_repeat("\0", 16) . $this->data . "\xAB\xCD\xEFMaxMind.com" . $metaBin;
    }

    private function node(int $l, int $r): string
    {
        switch ($this->recordSize) {
            case 24:
                return substr(pack('N', $l), 1) . substr(pack('N', $r), 1);
            case 28:
                return substr(pack('N', $l), 1) . chr((($l >> 24) & 0x0F) << 4 | (($r >> 24) & 0x0F)) . substr(pack('N', $r), 1);
            default:
                return pack('N', $l) . pack('N', $r);
        }
    }

    private function ctrl(int $type, int $size): string
    {
        if ($size < 29) {
            $sb = $size;
            $extra = '';
        } elseif ($size < 285) {
            $sb = 29;
            $extra = chr($size - 29);
        } elseif ($size < 65821) {
            $sb = 30;
            $extra = pack('n', $size - 285);
        } else {
            $sb = 31;
            $extra = substr(pack('N', $size - 65821), 1);
        }
        if ($type <= 7) {
            return chr(($type << 5) | $sb) . $extra;
        }
        return chr($sb) . chr($type - 7) . $extra;
    }

    private function pointer(int $p): string
    {
        if ($p < 2048) {
            return chr(0x20 | ($p >> 8)) . chr($p & 0xFF);
        }
        if ($p < 526336) {
            $v = $p - 2048;
            return chr(0x20 | (1 << 3) | ($v >> 16)) . chr(($v >> 8) & 0xFF) . chr($v & 0xFF);
        }
        if ($p < 134744064) {
            $v = $p - 526336;
            return chr(0x20 | (2 << 3) | ($v >> 24)) . chr(($v >> 16) & 0xFF) . chr(($v >> 8) & 0xFF) . chr($v & 0xFF);
        }
        return chr(0x20 | (3 << 3)) . pack('N', $p);
    }

    private static function uint(int|string $v, int $max): string
    {
        if (is_string($v)) {
            // decimal string -> big-endian bytes
            $bytes = '';
            while ($v !== '0' && $v !== '') {
                $rem = 0;
                $q = '';
                foreach (str_split($v) as $d) {
                    $cur = $rem * 10 + (int) $d;
                    $q .= (string) intdiv($cur, 256);
                    $rem = $cur % 256;
                }
                $bytes = chr($rem) . $bytes;
                $v = ltrim($q, '0');
            }
            return $bytes;
        }
        $b = '';
        while ($v > 0) {
            $b = chr($v & 0xFF) . $b;
            $v >>= 8;
        }
        return substr($b, -$max);
    }

    /** Encodes one value at the current end of the data section. */
    private function encode(mixed $v): string
    {
        if (is_array($v) && isset($v['__mmdb_type'])) {
            $x = $v['v'];
            return match ($v['__mmdb_type']) {
                'uint16' => $this->ctrl(5, strlen($b = self::uint($x, 2))) . $b,
                'uint32' => $this->ctrl(6, strlen($b = self::uint($x, 4))) . $b,
                'uint64' => $this->ctrl(9, strlen($b = self::uint($x, 8))) . $b,
                'uint128' => $this->ctrl(10, strlen($b = self::uint($x, 16))) . $b,
                'int32' => $this->ctrl(8, 4) . pack('N', $x & 0xFFFFFFFF),
                'double' => $this->ctrl(3, 8) . pack('E', $x),
                'float' => $this->ctrl(15, 4) . pack('G', $x),
                'bytes' => $this->ctrl(4, strlen($x)) . $x,
                'boolean' => $this->ctrl(14, $x ? 1 : 0),
            };
        }
        if (is_string($v)) {
            if (in_array($v, $this->shared, true)) {
                if (isset($this->stringOffsets[$v])) {
                    return $this->pointer($this->stringOffsets[$v]);
                }
                // first use: append the string to the data section now and point to it
                $off = strlen($this->data);
                $this->data .= $this->ctrl(2, strlen($v)) . $v;
                $this->stringOffsets[$v] = $off;
                return $this->pointer($off);
            }
            return $this->ctrl(2, strlen($v)) . $v;
        }
        if (is_bool($v)) {
            return $this->ctrl(14, $v ? 1 : 0);
        }
        if (is_float($v)) {
            return $this->ctrl(3, 8) . pack('E', $v);
        }
        if (is_int($v)) {
            return $this->ctrl(6, strlen($b = self::uint($v, 4))) . $b;
        }
        if (is_array($v) && array_is_list($v)) {
            $out = $this->ctrl(11, count($v));
            foreach ($v as $e) {
                $out .= $this->encode($e);
            }
            return $out;
        }
        if (is_array($v)) {
            $out = $this->ctrl(7, count($v));
            foreach ($v as $k => $e) {
                $out .= $this->encode((string) $k) . $this->encode($e);
            }
            return $out;
        }
        throw new \InvalidArgumentException('cannot encode ' . gettype($v));
    }
}
