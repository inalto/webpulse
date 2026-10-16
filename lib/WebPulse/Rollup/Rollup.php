<?php

declare(strict_types=1);

namespace WebPulse\Rollup;

use WebPulse\App;
use WebPulse\Collect\Collector;
use WebPulse\Paths;
use WebPulse\Store\DomainStore;
use WebPulse\Store\Dims;
use WebPulse\Util\DayClock;

/**
 * Nightly maintenance of the domain databases (ARCHITECTURE.md step 7):
 * close stale visits, cap high-cardinality dimensions per day, purge raw
 * tables, prune visitor_day / salts, refresh domain_summary, optimise and
 * vacuum. The caller (CLI) holds the per-user lock.
 */
final class Rollup
{
    /** Dims never folded (fixed small key sets). */
    private const NO_CAP = ['visit_duration', 'pages_per_visit', 'channel', 'device'];

    /** Uses $collector to close stale visits (a new one by default). */
    public function __construct(private App $app, private ?Collector $collector = null)
    {
        $this->collector ??= new Collector($app);
    }

    /**
     * Users that have a data directory.
     *
     * @return string[]
     */
    public function users(): array
    {
        $out = [];
        foreach (@scandir($this->app->paths->dataDir()) ?: [] as $u) {
            if ($u[0] !== '.' && Paths::isValidUser($u) && is_dir($this->app->paths->dataDir() . '/' . $u)) {
                $out[] = $u;
            }
        }
        return $out;
    }

    /**
     * Domains of a user that have a database.
     *
     * @return string[]
     */
    public function domainsWithData(string $user): array
    {
        $out = [];
        foreach (glob($this->app->paths->userDir($user) . '/*.db') ?: [] as $f) {
            $d = basename($f, '.db');
            if (Paths::isValidDomain($d) && !is_link($f)) {
                $out[] = $d;
            }
        }
        sort($out);
        return $out;
    }

    /** Prunes registry salts (older than 2 days). */
    public function pruneSalts(): int
    {
        $today = (new DayClock($this->app->serverTimezone()))->day($this->app->now());
        // keep two days of history on top of the server day so that every domain timezone is covered
        return $this->app->registry()->pruneSalts(DayClock::addDays($today, -3));
    }

    /**
     * Rolls up one domain database.
     *
     * @return array<string,int>
     */
    public function rollupDomain(string $user, string $domain): array
    {
        $store = DomainStore::open($this->app->paths, $user, $domain, false);
        $pdo = $store->pdo();
        $tz = $store->getMeta('timezone') ?? $this->app->serverTimezone();
        $now = $this->app->now();
        $clock = new DayClock($tz);
        $today = $clock->day($now);
        $stats = ['closed' => 0, 'folded' => 0, 'purged_visits' => 0, 'purged_events' => 0, 'pruned_visitor_day' => 0, 'vacuum' => 0];

        $stats['closed'] = $this->collector->closeStaleVisits($store, $tz);

        // cap dimensions of finished days
        $max = max(10, (int) $this->app->config->get('max_rows_per_dim', 1000));
        $finished = DayClock::addDays($today, -1);
        $st = $pdo->prepare('SELECT day, dim, COUNT(*) AS n FROM dim_daily WHERE day < ? GROUP BY day, dim HAVING COUNT(*) > ?');
        $st->bindValue(1, $finished);
        $st->bindValue(2, $max, \PDO::PARAM_INT);
        $st->execute();
        $groups = $st->fetchAll(\PDO::FETCH_ASSOC);
        if ($groups !== []) {
            $pdo->exec('BEGIN IMMEDIATE');
            try {
                foreach ($groups as $g) {
                    if (in_array($g['dim'], self::NO_CAP, true)) {
                        continue;
                    }
                    $stats['folded'] += $this->fold($pdo, (string) $g['day'], (string) $g['dim'], $max);
                }
                $pdo->exec('COMMIT');
            } catch (\Throwable $e) {
                $pdo->exec('ROLLBACK');
                throw $e;
            }
        }

        // raw retention
        $days = max(1, (int) $this->app->config->get('raw_retention_days', 30));
        $cutDay = DayClock::addDays($today, -$days);
        $cutTs = (new \DateTimeImmutable($cutDay . ' 00:00:00', new \DateTimeZone($tz)))->getTimestamp();
        $pdo->exec('BEGIN IMMEDIATE');
        try {
            $d = $pdo->prepare('DELETE FROM visits WHERE day < ?');
            $d->execute([$cutDay]);
            $stats['purged_visits'] = $d->rowCount();
            $d = $pdo->prepare('DELETE FROM events WHERE ts < ?');
            $d->execute([$cutTs]);
            $stats['purged_events'] = $d->rowCount();
            $d = $pdo->prepare('DELETE FROM visitor_day WHERE day < ?');
            $d->execute([DayClock::addDays($today, -2)]);
            $stats['pruned_visitor_day'] = $d->rowCount();
            $store->setMetaMany(['last_rollup_at' => (string) $now]);
            $pdo->exec('COMMIT');
        } catch (\Throwable $e) {
            $pdo->exec('ROLLBACK');
            throw $e;
        }

        $pdo->exec('PRAGMA optimize');
        $pages = (int) $pdo->query('PRAGMA page_count')->fetchColumn();
        $free = (int) $pdo->query('PRAGMA freelist_count')->fetchColumn();
        if ($pages > 0 && $free / $pages > 0.25) {
            $pdo->exec('VACUUM');
            $stats['vacuum'] = 1;
        }
        try {
            $this->app->registry()->refreshSummary($store, $tz, $now);
        } catch (\Throwable $e) {
            $this->app->log->warn("summary refresh failed for $user/$domain: " . $e->getMessage());
        }
        return $stats;
    }

    /**
     * Keeps the top ($max - 1) keys of one (day, dim) and folds the rest
     * into "(other)".
     *
     * @return int rows folded
     */
    private function fold(\PDO $pdo, string $day, string $dim, int $max): int
    {
        $order = match (true) {
            $dim === 'page' => 'pageviews DESC, visits DESC, hits DESC',
            in_array($dim, Dims::VISIT_LEVEL, true), in_array($dim, ['entry_page', 'exit_page'], true) => 'visits DESC, pageviews DESC, hits DESC',
            default => 'hits DESC, bytes DESC',
        };
        $st = $pdo->prepare("SELECT key FROM dim_daily WHERE day = ? AND dim = ? AND key <> ? ORDER BY $order, key LIMIT -1 OFFSET ?");
        $st->bindValue(1, $day);
        $st->bindValue(2, $dim);
        $st->bindValue(3, Dims::OTHER);
        $st->bindValue(4, $max - 1, \PDO::PARAM_INT);
        $st->execute();
        $tail = $st->fetchAll(\PDO::FETCH_COLUMN);
        if ($tail === []) {
            return 0;
        }
        $sum = [0, 0, 0, 0, 0, 0];
        $sel = $pdo->prepare('SELECT visits, pageviews, hits, bytes, bounces, duration FROM dim_daily WHERE day = ? AND dim = ? AND key = ?');
        $del = $pdo->prepare('DELETE FROM dim_daily WHERE day = ? AND dim = ? AND key = ?');
        foreach ($tail as $k) {
            $sel->execute([$day, $dim, $k]);
            $r = $sel->fetch(\PDO::FETCH_NUM);
            if ($r !== false) {
                foreach ($r as $i => $v) {
                    $sum[$i] += (int) $v;
                }
            }
            $del->execute([$day, $dim, $k]);
        }
        $ins = $pdo->prepare('INSERT INTO dim_daily (day, dim, key, visits, pageviews, hits, bytes, bounces, duration) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(day, dim, key) DO UPDATE SET visits = visits + excluded.visits, pageviews = pageviews + excluded.pageviews,
            hits = hits + excluded.hits, bytes = bytes + excluded.bytes, bounces = bounces + excluded.bounces, duration = duration + excluded.duration');
        $ins->execute(array_merge([$day, $dim, Dims::OTHER], $sum));
        return count($tail);
    }
}
