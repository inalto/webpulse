<?php

declare(strict_types=1);

namespace WebPulse\Reports;

use InvalidArgumentException;

/**
 * `a=visits_log`: Matomo-style visitor log, newest first, cursor-paginated
 * by start timestamp (`before`, plus `before_id` to break ties). Includes
 * still-open visits (flagged `live`).
 */
final class VisitsLog
{
    public const DEFAULT_LIMIT = 20;
    public const MAX_LIMIT = 100;
    public const MAX_ACTIONS = 100;

    private const ATTRS = [
        'channel', 'referrer_host', 'search_engine', 'keyword', 'campaign', 'country', 'city',
        'browser', 'browser_version', 'os', 'os_version', 'device', 'ip',
    ];

    /**
     * @param array<string,mixed> $q
     * @return array<string,mixed>
     */
    public static function build(DomainData $d, Range $r, array $q): array
    {
        $limit = self::int($q['limit'] ?? null, self::DEFAULT_LIMIT, 'limit');
        if ($limit < 1) {
            throw new InvalidArgumentException('"limit" must be at least 1');
        }
        $limit = min($limit, self::MAX_LIMIT);
        $before = self::int($q['before'] ?? null, PHP_INT_MAX, 'before');
        $beforeId = self::int($q['before_id'] ?? null, PHP_INT_MAX, 'before_id');

        $cursor = '(start_ts < ? OR (start_ts = ? AND visit_id < ?))';
        $args = [$r->from, $r->to, $before, $before, $beforeId];
        $closed = $d->rows(
            "SELECT * FROM visits WHERE day BETWEEN ? AND ? AND $cursor ORDER BY start_ts DESC, visit_id DESC LIMIT " . ($limit + 1),
            $args
        );
        $open = $d->rows(
            "SELECT * FROM open_visits WHERE day BETWEEN ? AND ? AND $cursor ORDER BY start_ts DESC, visit_id DESC LIMIT " . ($limit + 1),
            $args
        );

        $all = [];
        foreach ($closed as $row) {
            $all[] = self::fromClosed($row);
        }
        foreach ($open as $row) {
            $all[] = self::fromOpen($row);
        }
        usort($all, static fn ($a, $b) => [$b['start_ts'], $b['visit_id']] <=> [$a['start_ts'], $a['visit_id']]);
        $more = count($all) > $limit;
        $all = array_slice($all, 0, $limit);

        $actions = self::actions($d, array_column($all, 'visit_id'));
        $visits = [];
        foreach ($all as $v) {
            $id = $v['visit_id'];
            $v['actions'] = array_map(static fn ($a) => [
                'ts' => $d->iso((int) $a['ts']),
                'type' => (string) $a['type'],
                'path' => (string) $a['path'],
                'status' => (int) $a['status'],
                'bytes' => (int) $a['bytes'],
            ], $actions[$id] ?? []);
            $v['start'] = $d->iso($v['start_ts']);
            $v['end'] = $d->iso($v['end_ts']);
            unset($v['start_ts'], $v['end_ts']);
            $visits[] = $v;
        }
        $last = end($all);
        return [
            'visits' => $visits,
            'next_before' => $more && $last !== false ? $last['start_ts'] : null,
            'next_before_id' => $more && $last !== false ? $last['visit_id'] : null,
            'retention_days' => $d->rawRetentionDays,
        ];
    }

    /** `Chrome 129` from browser `Chrome` and version `129` or `Chrome 129`. */
    public static function withVersion(?string $name, ?string $version): ?string
    {
        if ($name === null || $name === '') {
            return $version !== null && $version !== '' ? $version : null;
        }
        if ($version === null || $version === '') {
            return $name;
        }
        return str_starts_with($version, $name) ? $version : $name . ' ' . $version;
    }

    /** @param array<string,mixed> $row */
    private static function fromClosed(array $row): array
    {
        $v = self::base((int) $row['visit_id'], (int) $row['start_ts'], (int) $row['end_ts'], (int) $row['duration'], $row);
        foreach (self::ATTRS as $k) {
            $v[$k] = isset($row[$k]) && $row[$k] !== '' ? (string) $row[$k] : null;
        }
        $v['live'] = false;
        return self::finish($v);
    }

    /** @param array<string,mixed> $row */
    private static function fromOpen(array $row): array
    {
        $start = (int) $row['start_ts'];
        $v = self::base((int) $row['visit_id'], $start, (int) $row['last_ts'], max(0, (int) $row['last_pv_ts'] - $start), $row);
        $a = json_decode((string) ($row['attrs'] ?? '{}'), true);
        $a = is_array($a) ? $a : [];
        foreach (self::ATTRS as $k) {
            $v[$k] = isset($a[$k]) && is_scalar($a[$k]) && (string) $a[$k] !== '' ? (string) $a[$k] : null;
        }
        $v['live'] = true;
        return self::finish($v);
    }

    /** @param array<string,mixed> $row */
    private static function base(int $id, int $start, int $end, int $duration, array $row): array
    {
        return [
            'visit_id' => $id,
            'start_ts' => $start,
            'end_ts' => $end,
            'duration' => $duration,
            'pageviews' => (int) ($row['pageviews'] ?? 0),
            'hits' => (int) ($row['hits'] ?? 0),
            'bytes' => (int) ($row['bytes'] ?? 0),
            'entry_page' => isset($row['entry_page']) ? (string) $row['entry_page'] : null,
            'exit_page' => isset($row['exit_page']) ? (string) $row['exit_page'] : null,
        ];
    }

    /** Joins name+version for browser/os and drops the IP when not stored. */
    private static function finish(array $v): array
    {
        $v['browser'] = self::withVersion($v['browser'], $v['browser_version']);
        $v['os'] = self::withVersion($v['os'], $v['os_version']);
        unset($v['browser_version'], $v['os_version']);
        if ($v['ip'] === null) {
            unset($v['ip']);
        }
        return $v;
    }

    /**
     * @param list<int> $ids
     * @return array<int,list<array<string,mixed>>>
     */
    private static function actions(DomainData $d, array $ids): array
    {
        if ($ids === []) {
            return [];
        }
        $in = implode(',', array_fill(0, count($ids), '?'));
        $out = [];
        foreach ($d->rows("SELECT visit_id, ts, type, path, status, bytes FROM events WHERE visit_id IN ($in) ORDER BY ts, id", $ids) as $row) {
            $id = (int) $row['visit_id'];
            if (count($out[$id] ?? []) < self::MAX_ACTIONS) {
                $out[$id][] = $row;
            }
        }
        return $out;
    }

    private static function int(mixed $v, int $default, string $name): int
    {
        if ($v === null || $v === '') {
            return $default;
        }
        if (!is_string($v) && !is_int($v) || !preg_match('/^\d{1,19}$/', (string) $v)) {
            throw new InvalidArgumentException('"' . $name . '" must be a non-negative integer');
        }
        return (int) $v;
    }
}
