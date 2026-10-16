<?php

declare(strict_types=1);

namespace WebPulse\Log;

/**
 * Tolerant parser for Apache/Nginx "combined" lines:
 *
 *   [%v[:%p] ]%h %l %u %t "%r" %>s %b "%{Referer}i" "%{User-Agent}i"[ extra…]
 *
 * Accepts IPv6 clients, "-" sizes and statuses, missing referrer/UA fields,
 * escaped quotes, and malformed request lines (`"-"`, TLS garbage such as
 * `"\x16\x03\x01…"`), which yield a Hit with an empty method.
 */
final class LogParser
{
    private const RE = '~^(?:(\S+) )?(\S+) \S+ (?:"[^"\\\\]*(?:\\\\.[^"\\\\]*)*"|\S+) \[(\d\d/[A-Za-z]{3}/\d{4}:\d\d:\d\d:\d\d) ([+-]\d{4})\] "([^"\\\\]*(?:\\\\.[^"\\\\]*)*)" (\d{1,3}|-) (\d+|-)(?: "([^"\\\\]*(?:\\\\.[^"\\\\]*)*)"(?: "([^"\\\\]*(?:\\\\.[^"\\\\]*)*)")?)?~';

    private const MONTHS = ['Jan' => 1, 'Feb' => 2, 'Mar' => 3, 'Apr' => 4, 'May' => 5, 'Jun' => 6,
        'Jul' => 7, 'Aug' => 8, 'Sep' => 9, 'Oct' => 10, 'Nov' => 11, 'Dec' => 12];

    /** @var array<string,int|null> "dd/Mon/yyyy" => UTC midnight */
    private array $dayCache = [];

    /** Parses one line; null when it is not a combined-format line. */
    public function parse(string $line): ?Hit
    {
        if (preg_match(self::RE, $line, $m) !== 1) {
            return null;
        }
        $ts = $this->timestamp($m[3], $m[4]);
        if ($ts === null) {
            return null;
        }
        $vhost = $m[1] !== '' ? $m[1] : null;
        $req = $m[5];
        if (str_contains($req, '\\')) {
            $req = self::unescape($req);
        }
        $method = '';
        $target = '';
        $protocol = '';
        $parts = explode(' ', $req, 4);
        $n = count($parts);
        if (($n === 3 || $n === 2) && $parts[1] !== '' && self::isMethod($parts[0])) {
            if ($n === 2 || self::isProtocol($parts[2])) {
                $method = $parts[0];
                $target = $parts[1];
                $protocol = $n === 3 ? strtoupper($parts[2]) : '';
            }
        }
        $ref = $m[8] ?? '';
        $ua = $m[9] ?? '';
        if ($ref !== '' && str_contains($ref, '\\')) {
            $ref = self::unescape($ref);
        }
        if ($ua !== '' && str_contains($ua, '\\')) {
            $ua = self::unescape($ua);
        }
        return new Hit(
            $m[2],
            $ts,
            self::offsetSeconds($m[4]),
            $method,
            $target,
            $protocol,
            $m[6] === '-' ? 0 : (int) $m[6],
            $m[7] === '-' ? 0 : (int) $m[7],
            $ref === '' || $ref === '-' ? null : $ref,
            $ua === '-' ? '' : $ua,
            $vhost,
        );
    }

    private static function isMethod(string $m): bool
    {
        $l = strlen($m);
        return $l > 0 && $l <= 20 && $m[0] >= 'A' && $m[0] <= 'Z' && strspn($m, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ_-') === $l;
    }

    /** HTTP/1.0, HTTP/1.1, HTTP/2, HTTP/2.0, HTTP/3 (any case). */
    private static function isProtocol(string $p): bool
    {
        $l = strlen($p);
        return ($l === 6 || $l === 8) && strncasecmp($p, 'HTTP/', 5) === 0 && ctype_digit($p[5]) && ($l === 6 || ($p[6] === '.' && ctype_digit($p[7])));
    }

    /** "+0200" -> 7200 */
    private static function offsetSeconds(string $tz): int
    {
        $s = ((int) substr($tz, 1, 2)) * 3600 + ((int) substr($tz, 3, 2)) * 60;
        return $tz[0] === '-' ? -$s : $s;
    }

    /** "10/Oct/2026:13:55:36" + "+0200" -> unix ts */
    private function timestamp(string $t, string $tz): ?int
    {
        if ($t === $this->lastT && $tz === $this->lastTz) {
            return $this->lastTs;
        }
        $this->lastT = $t;
        $this->lastTz = $tz;
        return $this->lastTs = $this->computeTimestamp($t, $tz);
    }

    private ?string $lastT = null;
    private ?string $lastTz = null;
    private ?int $lastTs = null;

    private function computeTimestamp(string $t, string $tz): ?int
    {
        $dkey = substr($t, 0, 11);
        if (!array_key_exists($dkey, $this->dayCache)) {
            if (count($this->dayCache) > 4096) {
                $this->dayCache = [];
            }
            $mon = self::MONTHS[ucfirst(strtolower(substr($t, 3, 3)))] ?? null;
            $d = (int) substr($t, 0, 2);
            $y = (int) substr($t, 7, 4);
            $this->dayCache[$dkey] = $mon !== null && checkdate($mon, $d, $y) ? gmmktime(0, 0, 0, $mon, $d, $y) : null;
        }
        $base = $this->dayCache[$dkey];
        if ($base === null) {
            return null;
        }
        $h = (int) substr($t, 12, 2);
        $i = (int) substr($t, 15, 2);
        $s = (int) substr($t, 18, 2);
        if ($h > 23 || $i > 59 || $s > 60) {
            return null;
        }
        return $base + $h * 3600 + $i * 60 + $s - self::offsetSeconds($tz);
    }

    /** Undoes Apache's escaping: \" \\ \xhh \n \t. */
    public static function unescape(string $s): string
    {
        return (string) preg_replace_callback('/\\\\(x[0-9a-fA-F]{2}|.)/s', static function (array $m): string {
            $c = $m[1];
            if ($c[0] === 'x' && strlen($c) === 3) {
                return chr((int) hexdec(substr($c, 1)));
            }
            return match ($c) {
                'n' => "\n",
                't' => "\t",
                default => $c,
            };
        }, $s);
    }
}
