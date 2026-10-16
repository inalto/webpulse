<?php

declare(strict_types=1);

namespace WebPulse\Util;

/**
 * IP address and CIDR helpers (IPv4 and IPv6).
 */
final class Ip
{
    /** True for a syntactically valid IPv4 or IPv6 address. */
    public static function isValid(string $ip): bool
    {
        return filter_var($ip, FILTER_VALIDATE_IP) !== false;
    }

    /** True for "addr" or "addr/prefix" with a prefix valid for the family. */
    public static function isValidCidr(string $cidr): bool
    {
        return self::parseCidr($cidr) !== null;
    }

    /**
     * Parses "addr[/bits]" into [packed network, bits]; null when invalid.
     *
     * @return array{0:string,1:int}|null
     */
    public static function parseCidr(string $cidr): ?array
    {
        $cidr = trim($cidr);
        $bits = null;
        if (str_contains($cidr, '/')) {
            [$addr, $b] = explode('/', $cidr, 2);
            if (!ctype_digit($b)) {
                return null;
            }
            $bits = (int) $b;
        } else {
            $addr = $cidr;
        }
        $packed = @inet_pton($addr);
        if ($packed === false || !self::isValid($addr)) {
            return null;
        }
        $max = strlen($packed) * 8;
        $bits ??= $max;
        if ($bits < 0 || $bits > $max) {
            return null;
        }
        return [self::mask($packed, $bits), $bits];
    }

    /** Zeroes every bit after the first $bits of a packed address. */
    public static function mask(string $packed, int $bits): string
    {
        $out = '';
        $len = strlen($packed);
        for ($i = 0; $i < $len; $i++) {
            $take = max(0, min(8, $bits - $i * 8));
            $m = $take === 0 ? 0 : (0xFF << (8 - $take)) & 0xFF;
            $out .= chr(ord($packed[$i]) & $m);
        }
        return $out;
    }

    /**
     * Compiles a list of IPs/CIDRs into a matcher list; invalid entries are
     * skipped.
     *
     * @param string[] $list
     * @return array<int,array{0:string,1:int}>
     */
    public static function compile(array $list): array
    {
        $out = [];
        foreach ($list as $c) {
            if (is_string($c) && ($p = self::parseCidr($c)) !== null) {
                $out[] = $p;
            }
        }
        return $out;
    }

    /**
     * True when $ip falls in one of the compiled networks. IPv4-mapped IPv6
     * addresses (::ffff:a.b.c.d) are matched as IPv4.
     *
     * @param array<int,array{0:string,1:int}> $compiled
     */
    public static function matches(string $ip, array $compiled): bool
    {
        if ($compiled === []) {
            return false;
        }
        $packed = @inet_pton($ip);
        if ($packed === false) {
            return false;
        }
        if (strlen($packed) === 16 && strncmp($packed, "\0\0\0\0\0\0\0\0\0\0\xff\xff", 12) === 0) {
            $packed = substr($packed, 12);
        }
        foreach ($compiled as [$net, $bits]) {
            if (strlen($net) === strlen($packed) && self::mask($packed, $bits) === $net) {
                return true;
            }
        }
        return false;
    }
}
