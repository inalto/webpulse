<?php

declare(strict_types=1);

namespace WebPulse\Reports;

/**
 * The whitelist of dimension names stored in `dim_daily.dim` and the
 * per-dimension metadata used by the API, the exporter and the UI.
 *
 * `ALL` is the single source of truth (DATA-MODEL.md); WP1's engine writes
 * exactly these names.
 */
final class Dimension
{
    /** @var list<string> */
    public const ALL = [
        'page', 'entry_page', 'exit_page', 'referrer_host', 'referrer_url', 'channel', 'search_engine',
        'keyword', 'campaign', 'utm_source', 'utm_medium', 'source_medium', 'country', 'city', 'browser',
        'browser_version', 'os', 'os_version', 'device', 'status', 'method', 'protocol', 'http_version',
        'file_type', 'static_file', 'download', 'not_found', 'not_found_referrer', 'server_error', 'bot',
        'visit_duration', 'pages_per_visit',
    ];

    /** Separator inside `not_found_referrer` keys: `path ← referrer_host`. */
    public const NF_REF_SEP = ' ← ';

    /** Fixed order of engagement buckets (METRICS.md). */
    public const DURATION_BUCKETS = ['0-10s', '11-30s', '31-60s', '1-2m', '2-5m', '5-10m', '10-30m', '30m+'];
    public const PAGES_BUCKETS = ['1', '2', '3', '4', '5', '6-10', '11-20', '21+'];

    /** Visit-level dims also carry bounces and duration (DATA-MODEL.md). */
    private const WITH_BOUNCES = [
        'entry_page', 'channel', 'country', 'device', 'browser', 'os', 'referrer_host', 'search_engine', 'campaign',
    ];

    /** Visit-level dims (one row per visit, counted in `visits`). */
    private const VISIT_LEVEL = [
        'entry_page', 'exit_page', 'referrer_host', 'referrer_url', 'channel', 'search_engine', 'keyword',
        'campaign', 'utm_source', 'utm_medium', 'source_medium', 'country', 'city', 'browser', 'browser_version',
        'os', 'os_version', 'device', 'visit_duration', 'pages_per_visit',
    ];

    /** Hit-level dims (counted in `hits`/`bytes`). */
    private const HIT_LEVEL = [
        'status', 'method', 'protocol', 'http_version', 'file_type', 'static_file', 'download',
        'not_found', 'not_found_referrer', 'server_error', 'bot',
    ];

    /** Base columns of dim_daily that can be summed. */
    public const COLUMNS = ['visits', 'pageviews', 'hits', 'bytes', 'bounces', 'duration'];

    private const LABELS = [
        'page' => 'Pages', 'entry_page' => 'Entry pages', 'exit_page' => 'Exit pages',
        'referrer_host' => 'Referrers', 'referrer_url' => 'Referrer URLs', 'channel' => 'Channels',
        'search_engine' => 'Search engines', 'keyword' => 'Keywords', 'campaign' => 'Campaigns',
        'utm_source' => 'UTM sources', 'utm_medium' => 'UTM media', 'source_medium' => 'Source / medium',
        'country' => 'Countries', 'city' => 'Cities', 'browser' => 'Browsers', 'browser_version' => 'Browser versions',
        'os' => 'Operating systems', 'os_version' => 'OS versions', 'device' => 'Devices', 'status' => 'HTTP status',
        'method' => 'Methods', 'protocol' => 'Protocols', 'http_version' => 'HTTP versions', 'file_type' => 'File types',
        'static_file' => 'Static files', 'download' => 'Downloads', 'not_found' => 'Not found (404)',
        'not_found_referrer' => '404 referrers', 'server_error' => 'Server errors (5xx)', 'bot' => 'Crawlers',
        'visit_duration' => 'Visit duration', 'pages_per_visit' => 'Pages per visit',
    ];

    /** Prettifier hint per dim (used by the UI and the CSV/CLI label column). */
    private const PRETTY = [
        'page' => 'path', 'entry_page' => 'path', 'exit_page' => 'path', 'not_found' => 'path',
        'server_error' => 'path', 'static_file' => 'path', 'download' => 'path', 'referrer_url' => 'url',
        'referrer_host' => 'host', 'country' => 'country', 'status' => 'status', 'channel' => 'channel',
        'device' => 'device', 'visit_duration' => 'bucket', 'pages_per_visit' => 'bucket',
        'search_engine' => 'engine', 'protocol' => 'upper', 'method' => 'upper',
    ];

    /**
     * True when $dim is a known dimension name.
     */
    public static function isValid(string $dim): bool
    {
        return in_array($dim, self::ALL, true);
    }

    public static function label(string $dim): string
    {
        return self::LABELS[$dim] ?? $dim;
    }

    public static function isVisitLevel(string $dim): bool
    {
        return in_array($dim, self::VISIT_LEVEL, true);
    }

    public static function isHitLevel(string $dim): bool
    {
        return in_array($dim, self::HIT_LEVEL, true);
    }

    public static function hasBounces(string $dim): bool
    {
        return in_array($dim, self::WITH_BOUNCES, true);
    }

    /** Bucket dims are ordered by bucket, not by value. */
    public static function isBucket(string $dim): bool
    {
        return $dim === 'visit_duration' || $dim === 'pages_per_visit';
    }

    /**
     * Metric used for `share` (and the default sort).
     */
    public static function primaryMetric(string $dim): string
    {
        if ($dim === 'page') {
            return 'pageviews';
        }
        if ($dim === 'static_file' || $dim === 'file_type') {
            return 'bytes';
        }
        if (self::isHitLevel($dim)) {
            return 'hits';
        }
        return 'visits';
    }

    /** Default `sort` value. */
    public static function defaultSort(string $dim): string
    {
        return self::isBucket($dim) ? 'bucket' : '-' . self::primaryMetric($dim);
    }

    /**
     * Columns (base and derived) that are meaningful for the dim, in display
     * order. The API returns all base columns; this list drives the UI and
     * the CSV exporter.
     *
     * @return list<string>
     */
    public static function metrics(string $dim): array
    {
        return match (true) {
            $dim === 'page' => ['pageviews', 'visits', 'hits', 'bytes', 'exit_rate'],
            $dim === 'entry_page' => ['visits', 'pageviews', 'bounces', 'bounce_rate', 'avg_duration'],
            $dim === 'exit_page' => ['visits', 'exit_rate'],
            self::isBucket($dim) => ['visits'],
            self::hasBounces($dim) => ['visits', 'pageviews', 'bounce_rate', 'avg_duration'],
            self::isVisitLevel($dim) => ['visits', 'pageviews'],
            $dim === 'bot' || $dim === 'static_file' || $dim === 'download' || $dim === 'file_type' => ['hits', 'bytes'],
            default => ['hits', 'bytes'],
        };
    }

    /** Prettifier hint: path|url|host|country|status|channel|device|bucket|engine|upper|text. */
    public static function pretty(string $dim): string
    {
        return self::PRETTY[$dim] ?? 'text';
    }

    /**
     * Metadata for every dim, shipped to the UI in the bootstrap.
     *
     * @return array<string,array{label:string,visit_level:bool,bounces:bool,sort:string,metrics:list<string>,pretty:string,share:string}>
     */
    public static function meta(): array
    {
        $out = [];
        foreach (self::ALL as $dim) {
            $out[$dim] = [
                'label' => self::label($dim),
                'visit_level' => self::isVisitLevel($dim),
                'bounces' => self::hasBounces($dim),
                'sort' => self::defaultSort($dim),
                'metrics' => self::metrics($dim),
                'pretty' => self::pretty($dim),
                'share' => self::primaryMetric($dim),
            ];
        }
        return $out;
    }
}
