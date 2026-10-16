<?php

declare(strict_types=1);

namespace WebPulse\Classify;

/**
 * Pure-PHP reader for MaxMind DB files (.mmdb, format 2.0): DB-IP lite,
 * GeoLite2 Country/City and any other database following the spec.
 *
 * Supports 24/28/32-bit search-tree records, IPv4 and IPv6 trees (IPv4
 * lookups in an IPv6 tree go through the ::/96 subtree), and every data
 * type: pointer, string, double, bytes, uint16/32/64/128, int32, map, array,
 * boolean, float, data-cache container and end marker.
 *
 * Files up to $maxMemory bytes are loaded in memory; larger ones are read
 * with fseek/fread.
 *
 * Decoding is bounded for hostile files: one record may expand to at most
 * MAX_NODES values (pointers re-used through the cache are charged at their
 * full size), a pointer may not point to another pointer, and nesting is
 * limited to 64 levels.
 */
final class MmdbReader
{
    private const MARKER = "\xAB\xCD\xEFMaxMind.com";
    /** Maximum number of values one record (or the metadata) may decode to. */
    public const MAX_NODES = 20000;

    private string $data = '';
    /** @var resource|null */
    private $fh = null;
    private int $fileSize;
    /** @var array<string,mixed> */
    private array $meta;
    private int $nodeCount;
    private int $recordSize;
    private int $ipVersion;
    private int $treeSize;
    private int $dataStart;
    private int $ipv4Start = 0;
    private int $nodeBytes;
    /** Values left in the budget of the current decode. */
    private int $budget = 0;
    /** @var array<int,array{0:mixed,1:int}> pointer target => [value, nodes] (current decode only) */
    private array $ptrCache = [];

    /**
     * @throws \RuntimeException when the file is missing or not a valid MMDB
     */
    public function __construct(private string $file, int $maxMemory = 64 * 1024 * 1024)
    {
        if (!is_file($file) || !is_readable($file)) {
            throw new \RuntimeException("cannot read $file");
        }
        $this->fileSize = (int) filesize($file);
        if ($this->fileSize <= $maxMemory) {
            $d = file_get_contents($file);
            if ($d === false) {
                throw new \RuntimeException("cannot read $file");
            }
            $this->data = $d;
        } else {
            $fh = fopen($file, 'rb');
            if ($fh === false) {
                throw new \RuntimeException("cannot open $file");
            }
            $this->fh = $fh;
        }
        $tailLen = min($this->fileSize, 128 * 1024);
        $tail = $this->read($this->fileSize - $tailLen, $tailLen);
        $pos = strrpos($tail, self::MARKER);
        if ($pos === false) {
            throw new \RuntimeException("$file is not a MaxMind DB (metadata marker not found)");
        }
        $metaStart = $this->fileSize - $tailLen + $pos + strlen(self::MARKER);
        $meta = $this->decodeRoot($metaStart, $metaStart);
        if (!is_array($meta) || !isset($meta['node_count'], $meta['record_size'], $meta['ip_version'])) {
            throw new \RuntimeException("$file has invalid metadata");
        }
        // validate the metadata before any use of the search tree
        foreach (['node_count', 'record_size', 'ip_version'] as $k) {
            if (!is_int($meta[$k])) {
                throw new \RuntimeException("$file has invalid metadata ($k)");
            }
        }
        $this->meta = $meta;
        $this->nodeCount = $meta['node_count'];
        $this->recordSize = $meta['record_size'];
        $this->ipVersion = $meta['ip_version'];
        if (!in_array($this->recordSize, [24, 28, 32], true)) {
            throw new \RuntimeException("$file: unsupported record size {$this->recordSize}");
        }
        if ($this->ipVersion !== 4 && $this->ipVersion !== 6) {
            throw new \RuntimeException("$file: unsupported IP version {$this->ipVersion}");
        }
        if ($this->nodeCount < 1) {
            throw new \RuntimeException("$file: invalid node count {$this->nodeCount}");
        }
        $this->nodeBytes = intdiv($this->recordSize * 2, 8);
        $this->treeSize = $this->nodeBytes * $this->nodeCount;
        $this->dataStart = $this->treeSize + 16;
        if ($this->dataStart > $metaStart - strlen(self::MARKER)) {
            throw new \RuntimeException("$file: search tree larger than the file");
        }
        if ($this->ipVersion === 6) {
            $node = 0;
            for ($i = 0; $i < 96 && $node < $this->nodeCount; $i++) {
                $node = $this->readNode($node, 0);
            }
            $this->ipv4Start = $node;
        }
    }

    /** @return array<string,mixed> the metadata map (database_type, build_epoch, languages…) */
    public function metadata(): array
    {
        return $this->meta;
    }

    /**
     * Record for $ip, or null when not found / invalid address.
     */
    public function get(string $ip): mixed
    {
        return $this->getWithPrefix($ip)[0];
    }

    /**
     * Record and network prefix length for $ip.
     *
     * @return array{0:mixed,1:int}
     */
    public function getWithPrefix(string $ip): array
    {
        $packed = @inet_pton($ip);
        if ($packed === false) {
            return [null, 0];
        }
        if (strlen($packed) === 16 && strncmp($packed, "\0\0\0\0\0\0\0\0\0\0\xff\xff", 12) === 0) {
            $packed = substr($packed, 12); // IPv4-mapped IPv6 (dual-stack listeners)
        }
        if (strlen($packed) === 16 && $this->ipVersion === 4) {
            return [null, 0];
        }
        $node = strlen($packed) === 4 && $this->ipVersion === 6 ? $this->ipv4Start : 0;
        $bits = strlen($packed) * 8;
        $i = 0;
        for (; $i < $bits && $node < $this->nodeCount; $i++) {
            $bit = (ord($packed[$i >> 3]) >> (7 - ($i & 7))) & 1;
            $node = $this->readNode($node, $bit);
        }
        if ($node === $this->nodeCount || $node < $this->nodeCount) {
            return [null, $i];
        }
        $offset = $this->treeSize + ($node - $this->nodeCount);
        if ($offset >= $this->fileSize) {
            throw new \RuntimeException("{$this->file}: invalid data pointer");
        }
        return [$this->decodeRoot($offset, $this->dataStart), $i];
    }

    /** Decodes one record (or the metadata) with a fresh node budget and pointer cache. */
    private function decodeRoot(int $offset, int $base): mixed
    {
        $this->budget = self::MAX_NODES;
        $this->ptrCache = [];
        try {
            return $this->decode($offset, $base)[0];
        } finally {
            $this->ptrCache = [];
        }
    }

    private function readNode(int $node, int $index): int
    {
        $base = $node * $this->nodeBytes;
        switch ($this->recordSize) {
            case 24:
                $b = $this->read($base + $index * 3, 3);
                return (ord($b[0]) << 16) | (ord($b[1]) << 8) | ord($b[2]);
            case 28:
                $b = $this->read($base, 7);
                if ($index === 0) {
                    return ((ord($b[3]) & 0xF0) << 20) | (ord($b[0]) << 16) | (ord($b[1]) << 8) | ord($b[2]);
                }
                return ((ord($b[3]) & 0x0F) << 24) | (ord($b[4]) << 16) | (ord($b[5]) << 8) | ord($b[6]);
            default:
                $b = $this->read($base + $index * 4, 4);
                return (int) unpack('N', $b)[1];
        }
    }

    private function read(int $offset, int $len): string
    {
        if ($len <= 0) {
            return '';
        }
        if ($offset < 0 || $offset + $len > $this->fileSize) {
            throw new \RuntimeException("{$this->file}: read beyond end of file");
        }
        if ($this->fh === null) {
            return substr($this->data, $offset, $len);
        }
        fseek($this->fh, $offset);
        $s = fread($this->fh, $len);
        if ($s === false || strlen($s) !== $len) {
            throw new \RuntimeException("{$this->file}: short read");
        }
        return $s;
    }

    /**
     * Decodes the data field at $offset; pointers are relative to $base.
     *
     * @return array{0:mixed,1:int} [value, offset after the field]
     */
    private function decode(int $offset, int $base, int $depth = 0): array
    {
        if ($depth > 64) {
            throw new \RuntimeException("{$this->file}: data nesting too deep");
        }
        if (--$this->budget < 0) {
            throw new \RuntimeException("{$this->file}: data record too large (more than " . self::MAX_NODES . ' values)');
        }
        $ctrl = ord($this->read($offset, 1));
        $offset++;
        $type = $ctrl >> 5;
        if ($type === 1) {
            $ss = ($ctrl >> 3) & 0x3;
            $vvv = $ctrl & 0x7;
            $b = $this->read($offset, $ss + 1);
            switch ($ss) {
                case 0:
                    $p = ($vvv << 8) | ord($b[0]);
                    break;
                case 1:
                    $p = (($vvv << 16) | (ord($b[0]) << 8) | ord($b[1])) + 2048;
                    break;
                case 2:
                    $p = (($vvv << 24) | (ord($b[0]) << 16) | (ord($b[1]) << 8) | ord($b[2])) + 526336;
                    break;
                default:
                    $p = (int) unpack('N', $b)[1];
            }
            $target = $base + $p;
            if (isset($this->ptrCache[$target])) {
                [$value, $nodes] = $this->ptrCache[$target];
                $this->budget -= $nodes;
                if ($this->budget < 0) {
                    throw new \RuntimeException("{$this->file}: data record too large (more than " . self::MAX_NODES . ' values)');
                }
                return [$value, $offset + $ss + 1];
            }
            if ((ord($this->read($target, 1)) >> 5) === 1) {
                throw new \RuntimeException("{$this->file}: invalid pointer to a pointer at $offset");
            }
            $left = $this->budget;
            [$value] = $this->decode($target, $base, $depth + 1);
            $this->ptrCache[$target] = [$value, $left - $this->budget];
            return [$value, $offset + $ss + 1];
        }
        if ($type === 0) {
            $type = 7 + ord($this->read($offset, 1));
            $offset++;
        }
        $size = $ctrl & 0x1f;
        if ($size >= 29) {
            $n = $size - 28;
            $b = $this->read($offset, $n);
            $offset += $n;
            $size = match ($n) {
                1 => 29 + ord($b[0]),
                2 => 285 + ((ord($b[0]) << 8) | ord($b[1])),
                default => 65821 + ((ord($b[0]) << 16) | (ord($b[1]) << 8) | ord($b[2])),
            };
        }
        switch ($type) {
            case 2: // utf8 string
            case 4: // bytes
                return [$this->read($offset, $size), $offset + $size];
            case 3: // double
                if ($size !== 8) {
                    throw new \RuntimeException("{$this->file}: invalid double size $size");
                }
                return [(float) unpack('E', $this->read($offset, 8))[1], $offset + 8];
            case 15: // float
                if ($size !== 4) {
                    throw new \RuntimeException("{$this->file}: invalid float size $size");
                }
                return [(float) unpack('G', $this->read($offset, 4))[1], $offset + 4];
            case 5: // uint16
            case 6: // uint32
            case 9: // uint64
            case 10: // uint128
                return [self::unsigned($this->read($offset, $size)), $offset + $size];
            case 8: // int32
                $b = $this->read($offset, $size);
                $v = 0;
                for ($i = 0; $i < $size; $i++) {
                    $v = ($v << 8) | ord($b[$i]);
                }
                if ($size === 4 && $v >= 0x80000000) {
                    $v -= 0x100000000;
                }
                return [$v, $offset + $size];
            case 7: // map
                $map = [];
                for ($i = 0; $i < $size; $i++) {
                    [$k, $offset] = $this->decode($offset, $base, $depth + 1);
                    [$v, $offset] = $this->decode($offset, $base, $depth + 1);
                    $map[(string) $k] = $v;
                }
                return [$map, $offset];
            case 11: // array
                $arr = [];
                for ($i = 0; $i < $size; $i++) {
                    [$v, $offset] = $this->decode($offset, $base, $depth + 1);
                    $arr[] = $v;
                }
                return [$arr, $offset];
            case 14: // boolean (value in the size field)
                return [$size !== 0, $offset];
            case 12: // data cache container
            case 13: // end marker
                return [null, $offset];
            default:
                throw new \RuntimeException("{$this->file}: unknown data type $type");
        }
    }

    /** Big-endian unsigned -> int, or decimal string when it does not fit in PHP_INT_MAX. */
    private static function unsigned(string $b): int|string
    {
        $b = ltrim($b, "\0");
        $n = strlen($b);
        if ($n === 0) {
            return 0;
        }
        if ($n < 8 || ($n === 8 && ord($b[0]) < 0x80)) {
            $v = 0;
            for ($i = 0; $i < $n; $i++) {
                $v = ($v << 8) | ord($b[$i]);
            }
            return $v;
        }
        // arbitrary precision decimal conversion (base 256 -> base 10)
        $digits = [0];
        for ($i = 0; $i < $n; $i++) {
            $carry = ord($b[$i]);
            foreach ($digits as $k => $d) {
                $x = $d * 256 + $carry;
                $digits[$k] = $x % 10;
                $carry = intdiv($x, 10);
            }
            while ($carry > 0) {
                $digits[] = $carry % 10;
                $carry = intdiv($carry, 10);
            }
        }
        return implode('', array_reverse($digits));
    }

    public function __destruct()
    {
        if ($this->fh !== null) {
            fclose($this->fh);
        }
    }
}
