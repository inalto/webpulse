<?php

declare(strict_types=1);

namespace WebPulse\Util;

/**
 * String helpers: valid UTF-8 enforcement and byte-safe truncation.
 *
 * Everything that comes out of an access log is attacker controlled; every
 * string stored in a database goes through {@see Str::clean()} first.
 */
final class Str
{
    /**
     * Returns $s as valid UTF-8: invalid byte sequences and C0 control
     * characters are replaced with "?".
     */
    public static function clean(string $s): string
    {
        if ($s === '') {
            return $s;
        }
        if (preg_match('//u', $s) !== 1) {
            $s = self::scrub($s);
        }
        if (strpbrk($s, "\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f\x7f") !== false) {
            $s = (string) preg_replace('/[\x00-\x1f\x7f]/', '?', $s);
        }
        return $s;
    }

    /** Replaces every invalid UTF-8 sequence with "?" (no mbstring needed). */
    public static function scrub(string $s): string
    {
        $out = preg_replace_callback(
            '/[\x00-\x7f]+|[\xc2-\xdf][\x80-\xbf]|\xe0[\xa0-\xbf][\x80-\xbf]|[\xe1-\xec\xee\xef][\x80-\xbf]{2}|\xed[\x80-\x9f][\x80-\xbf]'
            . '|\xf0[\x90-\xbf][\x80-\xbf]{2}|[\xf1-\xf3][\x80-\xbf]{3}|\xf4[\x80-\x8f][\x80-\xbf]{2}|(.)/s',
            static fn (array $m): string => isset($m[1]) ? '?' : $m[0],
            $s
        );
        return $out ?? '';
    }

    /**
     * Truncates valid UTF-8 $s to at most $maxBytes bytes without cutting a
     * multi-byte character; appends $ellipsis (counted in the limit) when cut.
     */
    public static function truncate(string $s, int $maxBytes, string $ellipsis = ''): string
    {
        if (strlen($s) <= $maxBytes) {
            return $s;
        }
        $limit = max(0, $maxBytes - strlen($ellipsis));
        $cut = substr($s, 0, $limit);
        // drop a partial trailing multi-byte sequence
        $i = strlen($cut);
        $back = 0;
        while ($i > 0 && $back < 4 && (ord($cut[$i - 1]) & 0xC0) === 0x80) {
            $i--;
            $back++;
        }
        if ($i > 0) {
            $lead = ord($cut[$i - 1]);
            if ($lead >= 0xC0) {
                $need = $lead >= 0xF0 ? 3 : ($lead >= 0xE0 ? 2 : 1);
                if ($back < $need) {
                    $cut = substr($cut, 0, $i - 1);
                }
            }
        }
        return $cut . $ellipsis;
    }

    /** Clean + truncate to $max bytes with "…" (the dim_daily key rule). */
    public static function key(string $s, int $max = 255): string
    {
        return self::truncate(self::clean($s), $max, "\u{2026}");
    }

    /** "geoip-update" -> "GeoipUpdate". */
    public static function studly(string $name): string
    {
        $parts = preg_split('/[-_]+/', strtolower($name)) ?: [];
        return implode('', array_map('ucfirst', $parts));
    }
}
