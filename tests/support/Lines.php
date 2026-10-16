<?php

declare(strict_types=1);

namespace WebPulse\Tests;

/**
 * Builds combined-format log lines for hand-written scenarios.
 */
final class Lines
{
    public const CHROME = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36';
    public const IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1';
    public const GOOGLEBOT = 'Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)';

    /**
     * @param string $local "Y-m-d H:i:s" in $tz
     */
    public static function line(string $ip, string $local, string $path = '/', int $status = 200, int $bytes = 1000,
        ?string $ref = null, string $ua = self::CHROME, string $method = 'GET', string $tz = 'Europe/Rome', string $proto = 'HTTP/2.0'): string
    {
        $d = new \DateTimeImmutable($local, new \DateTimeZone($tz));
        return sprintf('%s - - [%s] "%s %s %s" %d %s "%s" "%s"', $ip, $d->format('d/M/Y:H:i:s O'), $method, $path, $proto,
            $status, $bytes === 0 ? '-' : (string) $bytes, $ref ?? '-', $ua);
    }

    /** Unix timestamp of a local time. */
    public static function ts(string $local, string $tz = 'Europe/Rome'): int
    {
        return (new \DateTimeImmutable($local, new \DateTimeZone($tz)))->getTimestamp();
    }
}
