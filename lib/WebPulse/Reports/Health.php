<?php

declare(strict_types=1);

namespace WebPulse\Reports;

/**
 * `a=health`: the technical panel (GoAccess inspired): status codes, error
 * trend, 404s with their referrers, 5xx pages, bandwidth by file type, heavy
 * static files, downloads, protocol / HTTP version / method and peak hour.
 */
final class Health
{
    /**
     * @return array<string,mixed>
     */
    public static function build(DomainData $d, Range $r): array
    {
        $series = Overview::series($d, $r, $r->granularity === 'hour' ? 'day' : null);

        $notFound = self::top($d, $r, 'not_found', 'hits', 10);
        $refs = self::notFoundReferrers($d, $r, array_column($notFound, 'key'));
        foreach ($notFound as &$row) {
            $row['referrers'] = $refs[$row['key']] ?? [];
        }
        unset($row);

        $peak = $d->row('SELECT day, hour, hits FROM hourly WHERE day BETWEEN ? AND ? ORDER BY hits DESC, day DESC LIMIT 1', [$r->from, $r->to]);
        $peakHour = $d->row(
            'SELECT hour, SUM(hits) AS hits FROM hourly WHERE day BETWEEN ? AND ? GROUP BY hour ORDER BY hits DESC LIMIT 1',
            [$r->from, $r->to]
        );

        return [
            'range' => $r->toArray(),
            'status' => self::top($d, $r, 'status', 'hits', 50),
            'errors_series' => [
                'labels' => $series['labels'],
                'errors_4xx' => $series['errors_4xx'],
                'errors_5xx' => $series['errors_5xx'],
                'hits' => self::hitsSeries($d, $r, $series['labels']),
                'bot_hits' => $series['bot_hits'],
            ],
            'not_found' => $notFound,
            'server_error' => self::top($d, $r, 'server_error', 'hits', 10),
            'file_type' => self::top($d, $r, 'file_type', 'bytes', 30),
            'static_file' => self::top($d, $r, 'static_file', 'bytes', 10),
            'download' => self::top($d, $r, 'download', 'hits', 10),
            'protocol' => self::top($d, $r, 'protocol', 'hits', 10),
            'http_version' => self::top($d, $r, 'http_version', 'hits', 10),
            'method' => self::top($d, $r, 'method', 'hits', 10),
            'peak' => $peak === [] ? null : ['day' => (string) $peak['day'], 'hour' => (int) $peak['hour'], 'hits' => (int) $peak['hits']],
            'peak_hour' => $peakHour === [] ? null : ['hour' => (int) $peakHour['hour'], 'hits' => (int) $peakHour['hits']],
        ];
    }

    /**
     * Top rows of a dim by a metric, decorated like `report` rows.
     *
     * @return list<array<string,mixed>>
     */
    public static function top(DomainData $d, Range $r, string $dim, string $metric, int $limit): array
    {
        return Report::build($d, $r, [
            'dim' => $dim, 'sort' => $metric, 'desc' => true, 'limit' => $limit, 'offset' => 0,
            'q' => '', 'compare' => false, 'series' => false,
        ])['rows'];
    }

    /**
     * Daily (or bucketed) human hits, from `daily.hits`.
     *
     * @param list<string> $labels
     * @return list<int>
     */
    private static function hitsSeries(DomainData $d, Range $r, array $labels): array
    {
        $g = $r->granularity === 'hour' ? 'day' : $r->granularity;
        $idx = $r->dayIndex($g);
        $out = array_fill(0, count($labels), 0);
        foreach ($d->rows('SELECT day, hits FROM daily WHERE day BETWEEN ? AND ?', [$r->from, $r->to]) as $row) {
            if (isset($idx[$row['day']])) {
                $out[$idx[$row['day']]] += (int) $row['hits'];
            }
        }
        return $out;
    }

    /**
     * Top 5 referrer hosts per 404 path from the `not_found_referrer` dim.
     *
     * @param list<string> $paths
     * @return array<string,list<array{key:string,hits:int}>>
     */
    public static function notFoundReferrers(DomainData $d, Range $r, array $paths): array
    {
        $out = [];
        $sep = Dimension::NF_REF_SEP;
        foreach ($paths as $p) {
            $like = addcslashes($p . $sep, '%_\\') . '%';
            $rows = $d->rows(
                "SELECT key, SUM(hits) AS hits FROM dim_daily WHERE dim = 'not_found_referrer' AND day BETWEEN ? AND ?"
                . " AND key LIKE ? ESCAPE '\\' GROUP BY key ORDER BY hits DESC LIMIT 5",
                [$r->from, $r->to, $like]
            );
            foreach ($rows as $row) {
                $k = (string) $row['key'];
                $host = substr($k, strlen($p . $sep));
                if (strncmp($k, $p . $sep, strlen($p . $sep)) !== 0) {
                    continue; // LIKE is case-insensitive; keep exact prefixes only
                }
                $out[$p][] = ['key' => $host, 'hits' => (int) $row['hits']];
            }
        }
        return $out;
    }
}
