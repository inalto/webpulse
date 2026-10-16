<?php

declare(strict_types=1);

namespace WebPulse\Collect;

/**
 * In-memory accumulator for one domain: daily, hourly and dim_daily
 * counters plus the raw rows (events, closed visits, visitor_day). Flushed
 * with batched upserts inside the caller's transaction, so each
 * (day, dim, key) row is written at most once per flush.
 */
final class Counters
{
    public const DAILY = ['visits', 'visitors', 'pageviews', 'hits', 'bytes', 'bounces', 'duration', 'downloads', 'errors_4xx', 'errors_5xx', 'bot_hits', 'bot_bytes'];
    public const HOURLY = ['visits', 'pageviews', 'hits', 'bytes', 'bot_hits'];

    // dim_daily column indexes
    public const V = 0;
    public const PV = 1;
    public const H = 2;
    public const B = 3;
    public const BO = 4;
    public const DU = 5;

    private const BATCH = 100;

    /** @var array<string,array<string,int>> day => field => n */
    public array $daily = [];
    /** @var array<string,array<int,array<string,int>>> day => hour => field => n */
    public array $hourly = [];
    /** @var array<string,array<string,array<string,array<int,int>>>> day => dim => key => [6 ints] */
    public array $dims = [];
    /** @var array<int,array<int,mixed>> */
    public array $events = [];
    /** @var array<int,array<string,mixed>> */
    public array $visits = [];
    /** @var array<int,array{0:string,1:string}> */
    public array $visitorDay = [];

    public function day(string $day, string $field, int $n = 1): void
    {
        $this->daily[$day][$field] = ($this->daily[$day][$field] ?? 0) + $n;
    }

    public function hour(string $day, int $hour, string $field, int $n = 1): void
    {
        $this->hourly[$day][$hour][$field] = ($this->hourly[$day][$hour][$field] ?? 0) + $n;
    }

    /** Adds $n to column $col (V, PV, H, B, BO, DU) of a dim row. */
    public function dim(string $day, string $dim, string $key, int $col, int $n = 1): void
    {
        if (!isset($this->dims[$day][$dim][$key])) {
            $this->dims[$day][$dim][$key] = [0, 0, 0, 0, 0, 0];
        }
        $this->dims[$day][$dim][$key][$col] += $n;
    }

    /** Adds one hit (hits+1, bytes) to a dim row. */
    public function dimHit(string $day, string $dim, string $key, int $bytes): void
    {
        if (!isset($this->dims[$day][$dim][$key])) {
            $this->dims[$day][$dim][$key] = [0, 0, 0, 0, 0, 0];
        }
        $this->dims[$day][$dim][$key][self::H]++;
        $this->dims[$day][$dim][$key][self::B] += $bytes;
    }

    public function event(int $ts, ?int $visitId, string $type, string $path, int $status, int $bytes, ?string $referrer): void
    {
        $this->events[] = [$ts, $visitId, $type, $path, $status, $bytes, $referrer];
    }

    /** @param array<string,mixed> $row a `visits` row */
    public function visit(array $row): void
    {
        $this->visits[] = $row;
    }

    public function visitorDay(string $day, string $visitor): void
    {
        $this->visitorDay[] = [$day, $visitor];
    }

    /** Number of buffered raw rows (used to trigger intermediate flushes). */
    public function rawRows(): int
    {
        return count($this->events) + count($this->visits) + count($this->visitorDay);
    }

    /**
     * Writes everything to $pdo (the caller owns the transaction) and clears
     * the buffers.
     *
     * @return int rows written
     */
    public function flush(\PDO $pdo): int
    {
        $rows = 0;
        if ($this->daily !== []) {
            $cols = self::DAILY;
            $sql = 'INSERT INTO daily (day, ' . implode(', ', $cols) . ') VALUES (?' . str_repeat(', ?', count($cols)) . ')
                ON CONFLICT(day) DO UPDATE SET ' . implode(', ', array_map(static fn ($c) => "$c = $c + excluded.$c", $cols));
            $st = $pdo->prepare($sql);
            foreach ($this->daily as $day => $f) {
                $vals = [$day];
                $nz = false;
                foreach ($cols as $c) {
                    $v = $f[$c] ?? 0;
                    $nz = $nz || $v !== 0;
                    $vals[] = $v;
                }
                if ($nz) {
                    $st->execute($vals);
                    $rows++;
                }
            }
        }
        if ($this->hourly !== []) {
            $cols = self::HOURLY;
            $sql = 'INSERT INTO hourly (day, hour, ' . implode(', ', $cols) . ') VALUES (?, ?' . str_repeat(', ?', count($cols)) . ')
                ON CONFLICT(day, hour) DO UPDATE SET ' . implode(', ', array_map(static fn ($c) => "$c = $c + excluded.$c", $cols));
            $st = $pdo->prepare($sql);
            foreach ($this->hourly as $day => $hours) {
                foreach ($hours as $h => $f) {
                    $vals = [$day, $h];
                    $nz = false;
                    foreach ($cols as $c) {
                        $v = $f[$c] ?? 0;
                        $nz = $nz || $v !== 0;
                        $vals[] = $v;
                    }
                    if ($nz) {
                        $st->execute($vals);
                        $rows++;
                    }
                }
            }
        }
        $rows += $this->flushDims($pdo);
        $rows += $this->flushEvents($pdo);
        if ($this->visits !== []) {
            $cols = ['visit_id', 'visitor', 'day', 'start_ts', 'end_ts', 'duration', 'pageviews', 'hits', 'bytes', 'entry_page', 'exit_page',
                'channel', 'referrer_host', 'search_engine', 'keyword', 'campaign', 'country', 'city', 'browser', 'browser_version',
                'os', 'os_version', 'device', 'ip'];
            $rowSql = '(?' . str_repeat(', ?', count($cols) - 1) . ')';
            $stmts = [];
            foreach (array_chunk($this->visits, 40) as $chunk) {
                $n = count($chunk);
                $stmts[$n] ??= $pdo->prepare('INSERT OR REPLACE INTO visits (' . implode(', ', $cols) . ') VALUES ' . implode(', ', array_fill(0, $n, $rowSql)));
                $vals = [];
                foreach ($chunk as $v) {
                    foreach ($cols as $c) {
                        $vals[] = $v[$c] ?? null;
                    }
                }
                $stmts[$n]->execute($vals);
                $rows += $n;
            }
        }
        if ($this->visitorDay !== []) {
            $stmts = [];
            foreach (array_chunk($this->visitorDay, 400) as $chunk) {
                $n = count($chunk);
                $stmts[$n] ??= $pdo->prepare('INSERT OR IGNORE INTO visitor_day (day, visitor) VALUES ' . implode(', ', array_fill(0, $n, '(?, ?)')));
                $stmts[$n]->execute(array_merge(...$chunk));
                $rows += $n;
            }
        }
        $this->reset();
        return $rows;
    }

    private function flushDims(\PDO $pdo): int
    {
        $buf = [];
        $n = 0;
        $stmts = [];
        $exec = static function (array $buf) use ($pdo, &$stmts): void {
            $c = intdiv(count($buf), 9);
            if (!isset($stmts[$c])) {
                $stmts[$c] = $pdo->prepare('INSERT INTO dim_daily (day, dim, key, visits, pageviews, hits, bytes, bounces, duration) VALUES '
                    . implode(', ', array_fill(0, $c, '(?, ?, ?, ?, ?, ?, ?, ?, ?)'))
                    . ' ON CONFLICT(day, dim, key) DO UPDATE SET visits = visits + excluded.visits, pageviews = pageviews + excluded.pageviews,'
                    . ' hits = hits + excluded.hits, bytes = bytes + excluded.bytes, bounces = bounces + excluded.bounces,'
                    . ' duration = duration + excluded.duration');
            }
            $stmts[$c]->execute($buf);
        };
        foreach ($this->dims as $day => $dims) {
            foreach ($dims as $dim => $keys) {
                foreach ($keys as $key => $v) {
                    if ($v[0] === 0 && $v[1] === 0 && $v[2] === 0 && $v[3] === 0 && $v[4] === 0 && $v[5] === 0) {
                        continue;
                    }
                    array_push($buf, $day, $dim, (string) $key, $v[0], $v[1], $v[2], $v[3], $v[4], $v[5]);
                    $n++;
                    if (count($buf) >= self::BATCH * 9) {
                        $exec($buf);
                        $buf = [];
                    }
                }
            }
        }
        if ($buf !== []) {
            $exec($buf);
        }
        return $n;
    }

    private function flushEvents(\PDO $pdo): int
    {
        if ($this->events === []) {
            return 0;
        }
        $stmts = [];
        foreach (array_chunk($this->events, self::BATCH) as $chunk) {
            $c = count($chunk);
            $stmts[$c] ??= $pdo->prepare('INSERT INTO events (ts, visit_id, type, path, status, bytes, referrer) VALUES '
                . implode(', ', array_fill(0, $c, '(?, ?, ?, ?, ?, ?, ?)')));
            $stmts[$c]->execute(array_merge(...$chunk));
        }
        return count($this->events);
    }

    /** Clears every buffer. */
    public function reset(): void
    {
        $this->daily = [];
        $this->hourly = [];
        $this->dims = [];
        $this->events = [];
        $this->visits = [];
        $this->visitorDay = [];
    }
}
