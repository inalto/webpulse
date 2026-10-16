<?php

declare(strict_types=1);

namespace WebPulse\Reports;

/**
 * `a=realtime`: the last 30 minutes, from `open_visits` and `events`.
 * "Real time" is bounded by the collector cadence (every 5 minutes).
 */
final class Realtime
{
    public const WINDOW = 1800;
    public const RECENT = 20;

    /**
     * @return array<string,mixed>
     */
    public static function build(DomainData $d): array
    {
        $now = $d->now()->getTimestamp();
        $since = $now - self::WINDOW;
        $active = (int) ($d->row('SELECT COUNT(*) AS n FROM open_visits WHERE last_ts >= ?', [$since])['n'] ?? 0);

        $perMinute = array_fill(0, 30, 0);
        $pv = 0;
        foreach ($d->rows("SELECT ts FROM events WHERE ts >= ? AND ts <= ? AND type = 'pageview'", [$since, $now]) as $row) {
            $i = intdiv((int) $row['ts'] - $since, 60);
            $perMinute[max(0, min(29, $i))]++;
            $pv++;
        }

        $recent = $d->rows(
            "SELECT ts, path, status, type, referrer, visit_id FROM events WHERE ts >= ? AND type <> 'bot' ORDER BY ts DESC, id DESC LIMIT " . self::RECENT,
            [$since]
        );
        $attrs = self::visitAttrs($d, array_values(array_unique(array_filter(array_map(static fn ($r) => $r['visit_id'], $recent), static fn ($v) => $v !== null))));

        $out = [];
        foreach ($recent as $row) {
            $a = $attrs[(int) $row['visit_id']] ?? [];
            $out[] = [
                'ts' => $d->iso((int) $row['ts']),
                'path' => (string) $row['path'],
                'status' => (int) $row['status'],
                'type' => (string) $row['type'],
                'country' => $a['country'] ?? null,
                'device' => $a['device'] ?? null,
                'browser' => $a['browser'] ?? null,
                'referrer' => $row['referrer'] !== null && $row['referrer'] !== '' ? (string) $row['referrer'] : ($a['referrer_host'] ?? null),
                'visit_pageviews' => isset($a['pageviews']) ? (int) $a['pageviews'] : null,
            ];
        }
        $last = $d->row('SELECT MAX(ts) AS ts FROM events')['ts'] ?? null;

        return [
            'active_visits' => $active,
            'pageviews_30m' => $pv,
            'per_minute' => $perMinute,
            'recent' => $out,
            'last_event_at' => $last === null ? null : $d->iso((int) $last),
            'last_collect_at' => $d->iso($d->metaString('last_collect_at')),
        ];
    }

    /**
     * Country/device/browser of visits, from `visits` or `open_visits`.
     *
     * @param list<int|string> $ids
     * @return array<int,array<string,mixed>>
     */
    public static function visitAttrs(DomainData $d, array $ids): array
    {
        if ($ids === []) {
            return [];
        }
        $in = implode(',', array_fill(0, count($ids), '?'));
        $out = [];
        foreach ($d->rows("SELECT visit_id, country, device, browser, browser_version, referrer_host, pageviews FROM visits WHERE visit_id IN ($in)", $ids) as $r) {
            $out[(int) $r['visit_id']] = [
                'country' => $r['country'], 'device' => $r['device'],
                'browser' => VisitsLog::withVersion($r['browser'], $r['browser_version']),
                'referrer_host' => $r['referrer_host'], 'pageviews' => (int) $r['pageviews'],
            ];
        }
        foreach ($d->rows("SELECT visit_id, pageviews, attrs FROM open_visits WHERE visit_id IN ($in)", $ids) as $r) {
            $a = json_decode((string) $r['attrs'], true);
            $a = is_array($a) ? $a : [];
            $out[(int) $r['visit_id']] = [
                'country' => self::s($a['country'] ?? null), 'device' => self::s($a['device'] ?? null),
                'browser' => VisitsLog::withVersion(self::s($a['browser'] ?? null), self::s($a['browser_version'] ?? null)),
                'referrer_host' => self::s($a['referrer_host'] ?? null), 'pageviews' => (int) $r['pageviews'],
            ];
        }
        return $out;
    }

    private static function s(mixed $v): ?string
    {
        return is_scalar($v) && (string) $v !== '' ? (string) $v : null;
    }
}
