<?php

declare(strict_types=1);

namespace WebPulse\Reports;

use InvalidArgumentException;

/**
 * `a=report&name=<dim>`: ranked table for one dimension over a range, with
 * derived rates, optional previous-period values and an optional per-bucket
 * series of the top keys (for stacked trend charts).
 */
final class Report
{
    public const DEFAULT_LIMIT = 10;
    public const MAX_LIMIT = 500;
    public const EXPORT_MAX = 10000;
    public const SERIES_KEYS = 7;
    public const OTHER = '(other)';

    private const SORTABLE = [
        'key' => 'key',
        'visits' => 'visits',
        'pageviews' => 'pageviews',
        'hits' => 'hits',
        'bytes' => 'bytes',
        'bounces' => 'bounces',
        'duration' => 'duration',
        'bounce_rate' => 'CAST(bounces AS REAL) / MAX(visits, 1)',
        'avg_duration' => 'CAST(duration AS REAL) / MAX(visits, 1)',
    ];

    /**
     * Validated report parameters.
     *
     * @param array<string,mixed> $q query parameters
     * @return array{dim:string,sort:string,desc:bool,limit:int,offset:int,q:string,compare:bool,series:bool}
     * @throws InvalidArgumentException
     */
    public static function params(array $q, bool $export = false): array
    {
        $dim = $q['name'] ?? '';
        if (!is_string($dim) || !Dimension::isValid($dim)) {
            throw new InvalidArgumentException('Unknown report "' . (is_string($dim) ? substr($dim, 0, 40) : '') . '"');
        }
        $sort = $q['sort'] ?? '';
        if (!is_string($sort) || $sort === '') {
            $sort = Dimension::defaultSort($dim);
        }
        $desc = str_starts_with($sort, '-');
        $col = ltrim($sort, '-');
        if ($col === 'share') {
            $col = Dimension::primaryMetric($dim);
        }
        if ($col !== 'bucket' && !isset(self::SORTABLE[$col])) {
            throw new InvalidArgumentException('Invalid sort column "' . substr($col, 0, 40) . '"');
        }
        if ($col === 'bucket' && !Dimension::isBucket($dim)) {
            throw new InvalidArgumentException('Sort "bucket" only applies to engagement reports');
        }
        $max = $export ? self::EXPORT_MAX : self::MAX_LIMIT;
        $limit = self::int($q['limit'] ?? null, $export ? self::EXPORT_MAX : self::DEFAULT_LIMIT, 'limit');
        if ($limit < 1) {
            throw new InvalidArgumentException('"limit" must be at least 1');
        }
        $offset = self::int($q['offset'] ?? null, 0, 'offset');
        if ($offset < 0 || $offset > 1000000) {
            throw new InvalidArgumentException('"offset" is out of range');
        }
        $filter = $q['q'] ?? '';
        $filter = is_string($filter) ? mb_substr(trim($filter), 0, 200) : '';
        return [
            'dim' => $dim,
            'sort' => $col,
            'desc' => $desc,
            'limit' => min($limit, $max),
            'offset' => $offset,
            'q' => $filter,
            'compare' => ($q['compare'] ?? '') === '1' || ($q['compare'] ?? '') === 'true',
            'series' => ($q['series'] ?? '') === '1' || ($q['series'] ?? '') === 'true',
        ];
    }

    private static function int(mixed $v, int $default, string $name): int
    {
        if ($v === null || $v === '') {
            return $default;
        }
        if (is_int($v)) {
            return $v;
        }
        if (!is_string($v) || !preg_match('/^-?\d{1,9}\z/', $v)) {
            throw new InvalidArgumentException('"' . $name . '" must be an integer');
        }
        return (int) $v;
    }

    /**
     * @param array{dim:string,sort:string,desc:bool,limit:int,offset:int,q:string,compare:bool,series:bool} $p
     * @return array<string,mixed>
     */
    public static function build(DomainData $d, Range $r, array $p): array
    {
        $dim = $p['dim'];
        [$where, $args] = self::where($dim, $r->from, $r->to, $p['q']);

        $total = $d->row(
            'SELECT COALESCE(SUM(visits),0) AS visits, COALESCE(SUM(pageviews),0) AS pageviews, COALESCE(SUM(hits),0) AS hits,'
            . ' COALESCE(SUM(bytes),0) AS bytes, COALESCE(SUM(bounces),0) AS bounces, COALESCE(SUM(duration),0) AS duration'
            . ' FROM dim_daily WHERE dim = ? AND day BETWEEN ? AND ?',
            [$dim, $r->from, $r->to]
        );
        $total = array_map('intval', $total);
        $count = (int) ($d->row("SELECT COUNT(DISTINCT key) AS n FROM dim_daily WHERE $where", $args)['n'] ?? 0);
        $total['rows'] = $count;

        $rows = self::query($d, $where, $args, $p['sort'], $p['desc'], $p['limit'] + 1, $p['offset']);
        $hasMore = count($rows) > $p['limit'];
        $rows = array_slice($rows, 0, $p['limit']);
        $rows = self::decorate($d, $r, $dim, $rows, $total);

        if ($p['compare'] && $rows !== []) {
            $prev = self::byKeys($d, $dim, $r->prevFrom, $r->prevTo, array_column($rows, 'key'));
            foreach ($rows as &$row) {
                $pr = $prev[$row['key']] ?? null;
                $row['prev'] = [
                    'visits' => (int) ($pr['visits'] ?? 0),
                    'pageviews' => (int) ($pr['pageviews'] ?? 0),
                    'hits' => (int) ($pr['hits'] ?? 0),
                    'bytes' => (int) ($pr['bytes'] ?? 0),
                ];
            }
            unset($row);
        }

        $out = [
            'name' => $dim,
            'label' => Dimension::label($dim),
            'range' => $r->toArray(),
            'total' => $total,
            'rows' => $rows,
            'has_more' => $hasMore,
        ];
        if ($p['series']) {
            $out['series'] = self::series($d, $r, $dim);
        }
        return $out;
    }

    /**
     * @return array{0:string,1:list<string>}
     */
    private static function where(string $dim, string $from, string $to, string $q): array
    {
        $where = 'dim = ? AND day BETWEEN ? AND ?';
        $args = [$dim, $from, $to];
        if ($q !== '') {
            $where .= " AND key LIKE ? ESCAPE '\\'";
            $args[] = '%' . addcslashes($q, '%_\\') . '%';
        }
        return [$where, $args];
    }

    /**
     * @param list<string> $args
     * @return list<array<string,mixed>>
     */
    private static function query(DomainData $d, string $where, array $args, string $sort, bool $desc, int $limit, int $offset): array
    {
        if ($sort === 'bucket') {
            $cases = [];
            foreach (array_merge(Dimension::DURATION_BUCKETS, Dimension::PAGES_BUCKETS) as $i => $k) {
                $cases[$k] = $cases[$k] ?? ($i % 8);
            }
            $expr = 'CASE key';
            foreach ($cases as $k => $i) {
                $expr .= " WHEN '" . $k . "' THEN " . $i;
            }
            $expr .= ' ELSE 99 END';
        } else {
            $expr = self::SORTABLE[$sort];
        }
        $dir = $desc ? 'DESC' : 'ASC';
        $sql = 'SELECT key, SUM(visits) AS visits, SUM(pageviews) AS pageviews, SUM(hits) AS hits, SUM(bytes) AS bytes,'
            . ' SUM(bounces) AS bounces, SUM(duration) AS duration, MAX(day) AS last_day'
            . " FROM dim_daily WHERE $where GROUP BY key ORDER BY $expr $dir, key ASC LIMIT ? OFFSET ?";
        $args[] = $limit;
        $args[] = $offset;
        $st = $d->pdo->prepare($sql);
        foreach ($args as $i => $v) {
            $st->bindValue($i + 1, $v, is_int($v) ? \PDO::PARAM_INT : \PDO::PARAM_STR);
        }
        $st->execute();
        return $st->fetchAll(\PDO::FETCH_ASSOC);
    }

    /**
     * Sums per key for a set of keys.
     *
     * @param list<string> $keys
     * @return array<string,array<string,int>>
     */
    public static function byKeys(DomainData $d, string $dim, string $from, string $to, array $keys): array
    {
        if ($keys === []) {
            return [];
        }
        $out = [];
        foreach (array_chunk($keys, 400) as $chunk) {
            $in = implode(',', array_fill(0, count($chunk), '?'));
            $rows = $d->rows(
                'SELECT key, SUM(visits) AS visits, SUM(pageviews) AS pageviews, SUM(hits) AS hits, SUM(bytes) AS bytes'
                . " FROM dim_daily WHERE dim = ? AND day BETWEEN ? AND ? AND key IN ($in) GROUP BY key",
                array_merge([$dim, $from, $to], $chunk)
            );
            foreach ($rows as $row) {
                $out[(string) $row['key']] = array_map('intval', array_diff_key($row, ['key' => 1]));
            }
        }
        return $out;
    }

    /**
     * Casts numbers and adds share, labels and derived rates.
     *
     * @param list<array<string,mixed>> $rows
     * @param array<string,int>         $total
     * @return list<array<string,mixed>>
     */
    public static function decorate(DomainData $d, Range $r, string $dim, array $rows, array $total): array
    {
        $shareBy = Dimension::primaryMetric($dim);
        $labels = Pretty::hasLabels($dim);
        $keys = array_map(static fn ($x) => (string) $x['key'], $rows);

        $exits = [];
        $pageviews = [];
        if ($dim === 'page') {
            $exits = self::byKeys($d, 'exit_page', $r->from, $r->to, $keys);
        } elseif ($dim === 'exit_page') {
            $pageviews = self::byKeys($d, 'page', $r->from, $r->to, $keys);
        }

        $out = [];
        foreach ($rows as $row) {
            $k = (string) $row['key'];
            $o = ['key' => $k];
            if ($labels) {
                $o['label'] = Pretty::label($dim, $k);
            }
            foreach (Dimension::COLUMNS as $c) {
                $o[$c] = (int) ($row[$c] ?? 0);
            }
            if ($dim === 'bot' && isset($row['last_day'])) {
                $o['last_day'] = (string) $row['last_day'];
            }
            $o['share'] = $total[$shareBy] > 0 ? round($o[$shareBy] / $total[$shareBy], 4) : 0;
            if ($dim === 'page') {
                $ex = (int) ($exits[$k]['visits'] ?? 0);
                $o['exits'] = $ex;
                $o['exit_rate'] = $o['pageviews'] > 0 ? round(min(1, $ex / $o['pageviews']), 4) : 0;
            } elseif ($dim === 'exit_page') {
                $pv = (int) ($pageviews[$k]['pageviews'] ?? 0);
                $o['exit_rate'] = $pv > 0 ? round(min(1, $o['visits'] / $pv), 4) : null;
            }
            if (Dimension::hasBounces($dim)) {
                $o['bounce_rate'] = $o['visits'] > 0 ? round($o['bounces'] / $o['visits'], 4) : 0;
            }
            if (Dimension::isVisitLevel($dim) && Dimension::hasBounces($dim)) {
                $o['avg_duration'] = $o['visits'] > 0 ? (int) round($o['duration'] / $o['visits']) : 0;
            }
            $out[] = $o;
        }
        return $out;
    }

    /**
     * Per-bucket values of the primary metric for the top keys + `(other)`.
     *
     * @return array{labels:list<string>,granularity:string,metric:string,keys:list<string>,key_labels:list<string>,values:list<list<int>>}
     */
    public static function series(DomainData $d, Range $r, string $dim): array
    {
        $m = Dimension::primaryMetric($dim);
        $g = $r->granularity === 'hour' ? 'day' : $r->granularity;
        $labels = $r->labels($g);
        $idx = $r->dayIndex($g);
        $n = count($labels);

        $top = array_map(
            static fn ($x) => (string) $x['key'],
            $d->rows(
                "SELECT key, SUM($m) AS v FROM dim_daily WHERE dim = ? AND day BETWEEN ? AND ? GROUP BY key ORDER BY v DESC, key LIMIT " . self::SERIES_KEYS,
                [$dim, $r->from, $r->to]
            )
        );
        $values = [];
        foreach ($top as $k) {
            $values[$k] = array_fill(0, $n, 0);
        }
        $other = array_fill(0, $n, 0);
        foreach ($d->rows("SELECT day, key, SUM($m) AS v FROM dim_daily WHERE dim = ? AND day BETWEEN ? AND ? GROUP BY day, key", [$dim, $r->from, $r->to]) as $row) {
            $i = $idx[$row['day']] ?? null;
            if ($i === null) {
                continue;
            }
            $k = (string) $row['key'];
            if (isset($values[$k])) {
                $values[$k][$i] += (int) $row['v'];
            } else {
                $other[$i] += (int) $row['v'];
            }
        }
        $keys = $top;
        $vals = array_values($values);
        if (array_sum($other) > 0) {
            $keys[] = self::OTHER;
            $vals[] = $other;
        }
        return [
            'labels' => $labels,
            'granularity' => $g,
            'metric' => $m,
            'keys' => $keys,
            'key_labels' => array_map(static fn ($k) => $k === self::OTHER ? 'Other' : Pretty::label($dim, $k), $keys),
            'values' => $vals,
        ];
    }
}
