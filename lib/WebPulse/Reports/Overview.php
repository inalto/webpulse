<?php

declare(strict_types=1);

namespace WebPulse\Reports;

/**
 * `a=overview`: KPIs with previous-period deltas, the main time series,
 * hour-of-day / weekday distributions and the weekday × hour heatmap.
 */
final class Overview
{
    private const DAILY_COLS = [
        'visits', 'visitors', 'pageviews', 'hits', 'bytes', 'bounces', 'duration', 'downloads',
        'errors_4xx', 'errors_5xx', 'bot_hits', 'bot_bytes',
    ];

    /** Series returned in `series` (API.md). */
    private const SERIES = ['visits', 'visitors', 'pageviews', 'hits', 'bytes', 'bot_hits', 'bounces', 'duration', 'errors_4xx', 'errors_5xx'];

    /** Series the `hourly` table can provide. */
    private const HOURLY = ['visits', 'pageviews', 'hits', 'bytes', 'bot_hits'];

    /**
     * @return array<string,mixed>
     */
    public static function build(DomainData $d, Range $r, bool $compare): array
    {
        $cur = self::totals($d, $r->from, $r->to);
        $prev = self::totals($d, $r->prevFrom, $r->prevTo);

        $series = self::series($d, $r);
        if ($compare) {
            $p = self::series($d, $r->previous(), $r->granularity);
            $series['prev'] = [
                'labels' => $p['labels'],
                'visits' => $p['visits'],
                'pageviews' => $p['pageviews'],
                'visitors' => $p['visitors'],
                'hits' => $p['hits'],
                'bytes' => $p['bytes'],
                'bot_hits' => $p['bot_hits'],
                'bounces' => $p['bounces'],
                'duration' => $p['duration'],
                'errors_4xx' => $p['errors_4xx'],
                'errors_5xx' => $p['errors_5xx'],
            ];
        }

        [$hours, $heat] = self::hourly($d, $r);

        return [
            'range' => $r->toArray(),
            'kpi' => self::kpis($cur, $prev),
            'series' => $series,
            'hours' => $hours,
            'weekdays' => self::weekdays($d, $r),
            'heatmap' => $heat,
            'meta' => $d->metaBlock(),
        ];
    }

    /**
     * Sums of the `daily` table between two days.
     *
     * @return array<string,int>
     */
    public static function totals(DomainData $d, string $from, string $to): array
    {
        $sel = implode(', ', array_map(static fn ($c) => "COALESCE(SUM($c),0) AS $c", self::DAILY_COLS));
        $row = $d->row("SELECT $sel FROM daily WHERE day BETWEEN ? AND ?", [$from, $to]);
        $out = [];
        foreach (self::DAILY_COLS as $c) {
            $out[$c] = (int) ($row[$c] ?? 0);
        }
        return $out;
    }

    /**
     * @param array<string,int> $c current totals
     * @param array<string,int> $p previous totals
     * @return array<string,array{value:int|float,prev:int|float,change:float|null}>
     */
    public static function kpis(array $c, array $p): array
    {
        $derive = static function (array $t): array {
            $v = $t['visits'];
            $allHits = $t['hits'] + $t['bot_hits'];
            return [
                'visits' => $t['visits'],
                'visitors' => $t['visitors'],
                'pageviews' => $t['pageviews'],
                'pages_per_visit' => $v > 0 ? round($t['pageviews'] / $v, 2) : 0,
                'avg_duration' => $v > 0 ? (int) round($t['duration'] / $v) : 0,
                'bounce_rate' => $v > 0 ? round($t['bounces'] / $v, 4) : 0,
                'bytes' => $t['bytes'],
                'bot_share' => $allHits > 0 ? round($t['bot_hits'] / $allHits, 4) : 0,
                'downloads' => $t['downloads'],
                'errors_4xx' => $t['errors_4xx'],
                'errors_5xx' => $t['errors_5xx'],
                'hits' => $t['hits'],
                'bot_hits' => $t['bot_hits'],
            ];
        };
        $a = $derive($c);
        $b = $derive($p);
        $out = [];
        foreach ($a as $k => $v) {
            $out[$k] = ['value' => $v, 'prev' => $b[$k], 'change' => self::change($v, $b[$k])];
        }
        return $out;
    }

    /** `(value - prev) / prev`, null when prev is 0. */
    public static function change(int|float $value, int|float $prev): ?float
    {
        if ($prev == 0) {
            return null;
        }
        return round(($value - $prev) / $prev, 4);
    }

    /**
     * Main series bucketed by the range's granularity (or $granularity for
     * the previous period, so both series align).
     *
     * @return array<string,mixed>
     */
    public static function series(DomainData $d, Range $r, ?string $granularity = null): array
    {
        $g = $granularity ?? $r->granularity;
        $labels = $r->labels($g);
        $n = count($labels);
        $zero = array_fill(0, $n, 0);
        $out = ['labels' => $labels];

        if ($g === 'hour') {
            foreach (self::SERIES as $s) {
                $out[$s] = in_array($s, self::HOURLY, true) ? $zero : null;
            }
            $pos = array_flip($r->dayList());
            $rows = $d->rows('SELECT day, hour, visits, pageviews, hits, bytes, bot_hits FROM hourly WHERE day BETWEEN ? AND ?', [$r->from, $r->to]);
            foreach ($rows as $row) {
                $h = (int) $row['hour'];
                if (!isset($pos[$row['day']]) || $h < 0 || $h > 23) {
                    continue;
                }
                $i = $pos[$row['day']] * 24 + $h;
                foreach (self::HOURLY as $s) {
                    $out[$s][$i] += (int) $row[$s];
                }
            }
            return $out;
        }

        foreach (self::SERIES as $s) {
            $out[$s] = $zero;
        }
        $idx = $r->dayIndex($g);
        $cols = implode(', ', self::SERIES);
        foreach ($d->rows("SELECT day, $cols FROM daily WHERE day BETWEEN ? AND ?", [$r->from, $r->to]) as $row) {
            $i = $idx[$row['day']] ?? null;
            if ($i === null) {
                continue;
            }
            foreach (self::SERIES as $s) {
                $out[$s][$i] += (int) $row[$s];
            }
        }
        return $out;
    }

    /**
     * Hour-of-day totals and the weekday × hour heatmap (visits), Monday first.
     *
     * @return array{0:array{visits:list<int>,pageviews:list<int>},1:list<list<int>>}
     */
    private static function hourly(DomainData $d, Range $r): array
    {
        $hours = ['visits' => array_fill(0, 24, 0), 'pageviews' => array_fill(0, 24, 0)];
        $heat = array_fill(0, 7, array_fill(0, 24, 0));
        $rows = $d->rows(
            'SELECT day, hour, SUM(visits) AS visits, SUM(pageviews) AS pageviews FROM hourly WHERE day BETWEEN ? AND ? GROUP BY day, hour',
            [$r->from, $r->to]
        );
        $dow = [];
        foreach ($rows as $row) {
            $h = (int) $row['hour'];
            if ($h < 0 || $h > 23) {
                continue;
            }
            $day = (string) $row['day'];
            $w = $dow[$day] ??= self::weekday($day);
            $hours['visits'][$h] += (int) $row['visits'];
            $hours['pageviews'][$h] += (int) $row['pageviews'];
            $heat[$w][$h] += (int) $row['visits'];
        }
        return [$hours, $heat];
    }

    /**
     * @return array{visits:list<int>,pageviews:list<int>}
     */
    private static function weekdays(DomainData $d, Range $r): array
    {
        $out = ['visits' => array_fill(0, 7, 0), 'pageviews' => array_fill(0, 7, 0)];
        foreach ($d->rows('SELECT day, visits, pageviews FROM daily WHERE day BETWEEN ? AND ?', [$r->from, $r->to]) as $row) {
            $w = self::weekday((string) $row['day']);
            $out['visits'][$w] += (int) $row['visits'];
            $out['pageviews'][$w] += (int) $row['pageviews'];
        }
        return $out;
    }

    /** 0 = Monday … 6 = Sunday. */
    public static function weekday(string $day): int
    {
        $ts = gmmktime(0, 0, 0, (int) substr($day, 5, 2), (int) substr($day, 8, 2), (int) substr($day, 0, 4));
        return ((int) gmdate('N', (int) $ts)) - 1;
    }
}
