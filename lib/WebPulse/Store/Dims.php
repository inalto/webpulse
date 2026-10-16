<?php

declare(strict_types=1);

namespace WebPulse\Store;

/**
 * Dimension names written to `dim_daily.dim` (DATA-MODEL.md). Mirrored by
 * WebPulse\Reports\Dimension::ALL on the read side; the values are fixed.
 */
final class Dims
{
    public const ALL = [
        'page', 'entry_page', 'exit_page', 'referrer_host', 'referrer_url', 'channel', 'search_engine',
        'keyword', 'campaign', 'utm_source', 'utm_medium', 'source_medium', 'country', 'city', 'browser',
        'browser_version', 'os', 'os_version', 'device', 'status', 'method', 'protocol', 'http_version',
        'file_type', 'static_file', 'download', 'not_found', 'not_found_referrer', 'server_error', 'bot',
        'visit_duration', 'pages_per_visit',
    ];

    /**
     * Visit-level dimensions: attributed once per visit (first hit) on the
     * visit's day; they carry visits, pageviews, bounces and duration.
     */
    public const VISIT_LEVEL = [
        'channel', 'referrer_host', 'referrer_url', 'search_engine', 'keyword', 'campaign', 'utm_source',
        'utm_medium', 'source_medium', 'country', 'city', 'browser', 'browser_version', 'os', 'os_version', 'device',
    ];

    /** Separator used in `not_found_referrer` keys: "/path ← host". */
    public const NF_REF_SEP = ' ← ';

    /** Key used when rollup folds the tail of a dimension. */
    public const OTHER = '(other)';

    /** Engagement bucket keys (METRICS.md). */
    public const DURATION_BUCKETS = ['0-10s', '11-30s', '31-60s', '1-2m', '2-5m', '5-10m', '10-30m', '30m+'];
    public const PAGES_BUCKETS = ['1', '2', '3', '4', '5', '6-10', '11-20', '21+'];

    /** dim_daily value columns, in storage order. */
    public const COLUMNS = ['visits', 'pageviews', 'hits', 'bytes', 'bounces', 'duration'];

    /** Bucket key for a visit duration in seconds. */
    public static function durationBucket(int $sec): string
    {
        return match (true) {
            $sec <= 10 => '0-10s',
            $sec <= 30 => '11-30s',
            $sec <= 60 => '31-60s',
            $sec <= 120 => '1-2m',
            $sec <= 300 => '2-5m',
            $sec <= 600 => '5-10m',
            $sec <= 1800 => '10-30m',
            default => '30m+',
        };
    }

    /** Bucket key for a pageview count (>= 1). */
    public static function pagesBucket(int $pv): string
    {
        return match (true) {
            $pv <= 5 => (string) max(1, $pv),
            $pv <= 10 => '6-10',
            $pv <= 20 => '11-20',
            default => '21+',
        };
    }
}
