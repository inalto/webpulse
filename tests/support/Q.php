<?php

declare(strict_types=1);

namespace WebPulse\Tests;

/**
 * Query helpers for assertions on a domain database.
 */
final class Q
{
    /** @return array<string,int> daily row (zeros when missing) */
    public static function daily(\PDO $pdo, string $day): array
    {
        $st = $pdo->prepare('SELECT * FROM daily WHERE day = ?');
        $st->execute([$day]);
        $r = $st->fetch(\PDO::FETCH_ASSOC);
        $cols = ['visits', 'visitors', 'pageviews', 'hits', 'bytes', 'bounces', 'duration', 'downloads', 'errors_4xx', 'errors_5xx', 'bot_hits', 'bot_bytes'];
        $out = [];
        foreach ($cols as $c) {
            $out[$c] = $r === false ? 0 : (int) $r[$c];
        }
        return $out;
    }

    /** @return array<string,int> sums of daily over all days */
    public static function totals(\PDO $pdo): array
    {
        $r = $pdo->query('SELECT SUM(visits) visits, SUM(visitors) visitors, SUM(pageviews) pageviews, SUM(hits) hits, SUM(bytes) bytes,
            SUM(bounces) bounces, SUM(duration) duration, SUM(downloads) downloads, SUM(errors_4xx) errors_4xx, SUM(errors_5xx) errors_5xx,
            SUM(bot_hits) bot_hits, SUM(bot_bytes) bot_bytes FROM daily')->fetch(\PDO::FETCH_ASSOC);
        return array_map('intval', $r);
    }

    /**
     * Rows of one dimension summed over days, keyed by key, without all-zero rows.
     *
     * @return array<string,array<string,int>>
     */
    public static function dim(\PDO $pdo, string $dim, ?string $day = null): array
    {
        $sql = 'SELECT key, SUM(visits) visits, SUM(pageviews) pageviews, SUM(hits) hits, SUM(bytes) bytes, SUM(bounces) bounces, SUM(duration) duration
                FROM dim_daily WHERE dim = ?' . ($day !== null ? ' AND day = ?' : '') . ' GROUP BY key ORDER BY key';
        $st = $pdo->prepare($sql);
        $st->execute($day !== null ? [$dim, $day] : [$dim]);
        $out = [];
        foreach ($st->fetchAll(\PDO::FETCH_ASSOC) as $r) {
            $k = (string) $r['key'];
            unset($r['key']);
            $r = array_map('intval', $r);
            if (array_sum($r) !== 0) {
                $out[$k] = $r;
            }
        }
        return $out;
    }

    /** Map key => one column of a dimension. */
    public static function col(\PDO $pdo, string $dim, string $col, ?string $day = null): array
    {
        $out = [];
        foreach (self::dim($pdo, $dim, $day) as $k => $r) {
            if ($r[$col] !== 0) {
                $out[$k] = $r[$col];
            }
        }
        return $out;
    }

    public static function count(\PDO $pdo, string $table, string $where = '1=1', array $params = []): int
    {
        $st = $pdo->prepare("SELECT COUNT(*) FROM $table WHERE $where");
        $st->execute($params);
        return (int) $st->fetchColumn();
    }

    /** Stable dump of the aggregate tables (to compare runs). */
    public static function snapshot(\PDO $pdo): string
    {
        $parts = [];
        foreach (['SELECT * FROM daily ORDER BY day', 'SELECT * FROM hourly ORDER BY day, hour',
            'SELECT * FROM dim_daily ORDER BY day, dim, key', 'SELECT * FROM visits ORDER BY visit_id',
            'SELECT visitor, visit_id, day, start_ts, last_ts, pageviews FROM open_visits ORDER BY visitor', 'SELECT COUNT(*) FROM events'] as $sql) {
            $parts[] = json_encode($pdo->query($sql)->fetchAll(\PDO::FETCH_NUM));
        }
        return implode("\n", $parts);
    }
}
