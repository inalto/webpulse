<?php

declare(strict_types=1);

namespace WebPulse\Reports;

/**
 * Human-readable labels for dimension keys (country names, HTTP status
 * texts, channel/device/bucket names). Used for the `label` field of report
 * rows, the CSV exporter and the CLI.
 */
final class Pretty
{
    /** @var array<string,string>|null */
    private static ?array $countries = null;

    private const STATUS = [
        100 => 'Continue', 101 => 'Switching Protocols', 200 => 'OK', 201 => 'Created', 202 => 'Accepted',
        203 => 'Non-Authoritative Information', 204 => 'No Content', 205 => 'Reset Content', 206 => 'Partial Content',
        300 => 'Multiple Choices', 301 => 'Moved Permanently', 302 => 'Found', 303 => 'See Other', 304 => 'Not Modified',
        307 => 'Temporary Redirect', 308 => 'Permanent Redirect', 400 => 'Bad Request', 401 => 'Unauthorized',
        402 => 'Payment Required', 403 => 'Forbidden', 404 => 'Not Found', 405 => 'Method Not Allowed',
        406 => 'Not Acceptable', 408 => 'Request Timeout', 409 => 'Conflict', 410 => 'Gone', 411 => 'Length Required',
        412 => 'Precondition Failed', 413 => 'Content Too Large', 414 => 'URI Too Long', 415 => 'Unsupported Media Type',
        416 => 'Range Not Satisfiable', 418 => "I'm a teapot", 421 => 'Misdirected Request', 422 => 'Unprocessable Content',
        429 => 'Too Many Requests', 431 => 'Request Header Fields Too Large', 444 => 'No Response',
        451 => 'Unavailable For Legal Reasons', 499 => 'Client Closed Request', 500 => 'Internal Server Error',
        501 => 'Not Implemented', 502 => 'Bad Gateway', 503 => 'Service Unavailable', 504 => 'Gateway Timeout',
        505 => 'HTTP Version Not Supported', 508 => 'Loop Detected',
    ];

    private const CHANNELS = [
        'direct' => 'Direct', 'organic' => 'Organic search', 'social' => 'Social', 'referral' => 'Referral',
        'paid' => 'Paid', 'email' => 'Email', 'campaign' => 'Campaign',
    ];

    private const DEVICES = ['desktop' => 'Desktop', 'mobile' => 'Mobile', 'tablet' => 'Tablet', 'other' => 'Other'];

    private const ENGINES = [
        'google' => 'Google', 'bing' => 'Bing', 'duckduckgo' => 'DuckDuckGo', 'yahoo' => 'Yahoo', 'yandex' => 'Yandex',
        'baidu' => 'Baidu', 'ecosia' => 'Ecosia', 'qwant' => 'Qwant', 'startpage' => 'Startpage', 'brave' => 'Brave Search',
        'naver' => 'Naver', 'seznam' => 'Seznam', 'ask' => 'Ask', 'aol' => 'AOL',
    ];

    /**
     * Label for a key of the given dim.
     */
    public static function label(string $dim, string $key): string
    {
        return match (Dimension::pretty($dim)) {
            'country' => self::country($key),
            'status' => self::status($key),
            'channel' => self::CHANNELS[$key] ?? ucfirst($key),
            'device' => self::DEVICES[$key] ?? ucfirst($key),
            'engine' => self::ENGINES[$key] ?? ucfirst($key),
            'bucket' => self::bucket($dim, $key),
            'upper' => strtoupper($key),
            default => $key,
        };
    }

    /** True when the dim's label differs from the key (worth sending to the UI). */
    public static function hasLabels(string $dim): bool
    {
        return in_array(Dimension::pretty($dim), ['country', 'status', 'channel', 'device', 'engine', 'bucket', 'upper'], true);
    }

    /** English country name for a lowercase alpha-2 code, `Unknown` otherwise. */
    public static function country(string $code): string
    {
        if (self::$countries === null) {
            /** @var array<string,string> $t */
            $t = require __DIR__ . '/data/countries.php';
            self::$countries = $t;
        }
        $c = strtolower($code);
        return self::$countries[$c] ?? ($c === '' || $c[0] === '(' ? 'Unknown' : strtoupper($code));
    }

    /** `404 Not Found`. */
    public static function status(string|int $code): string
    {
        $n = (int) $code;
        $text = self::STATUS[$n] ?? null;
        return $text === null ? (string) $code : $n . ' ' . $text;
    }

    /** Engagement bucket label (`2–5 min`, `6–10 pages`). */
    public static function bucket(string $dim, string $key): string
    {
        if ($dim === 'pages_per_visit') {
            return $key === '1' ? '1 page' : str_replace('-', '–', $key) . ' pages';
        }
        $map = [
            '0-10s' => '0–10 s', '11-30s' => '11–30 s', '31-60s' => '31–60 s', '1-2m' => '1–2 min',
            '2-5m' => '2–5 min', '5-10m' => '5–10 min', '10-30m' => '10–30 min', '30m+' => '30 min +',
        ];
        return $map[$key] ?? $key;
    }

    /** `1.2 MB` (1024-based). */
    public static function bytes(int|float $n): string
    {
        $u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
        $i = 0;
        $v = (float) $n;
        while ($v >= 1024 && $i < count($u) - 1) {
            $v /= 1024;
            $i++;
        }
        return ($i === 0 ? (string) (int) $v : number_format($v, $v >= 100 ? 0 : 1)) . ' ' . $u[$i];
    }

    /** `1m 32s`. */
    public static function duration(int|float $s): string
    {
        $s = (int) round($s);
        if ($s < 60) {
            return $s . 's';
        }
        if ($s < 3600) {
            return intdiv($s, 60) . 'm ' . ($s % 60) . 's';
        }
        return intdiv($s, 3600) . 'h ' . intdiv($s % 3600, 60) . 'm';
    }
}
