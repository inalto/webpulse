<?php

declare(strict_types=1);

namespace WebPulse\Collect;

use WebPulse\Store\Dims;
use WebPulse\Util\DayClock;
use WebPulse\Util\Json;

/**
 * Turns hits into visits (METRICS.md "Visitor and visit").
 *
 * - visitor hash = first 16 hex of sha256(salt(local day) . ip . "\n" . ua);
 * - a visit starts on a pageview or download when the visitor has no open
 *   visit or the gap since its last hit is >= the session timeout; other hits
 *   (static, redirect, error…) only extend an open visit;
 * - a visit crossing local midnight continues: during the first `timeout`
 *   seconds of a day the previous day's hash is tried too;
 * - visit-level counters (visits, visitors, entry/exit, bounce +1/−1,
 *   running duration) are attributed to the visit's day; the visit-level
 *   dimension rows (channel, country, browser…) receive their pageview,
 *   bounce and duration deltas at every flush and at close;
 * - engagement buckets and the `visits` row are written at close.
 *
 * In persistent mode open visits are loaded from / saved to `open_visits`
 * so that a visit survives across collector runs.
 */
final class Sessionizer
{
    public const MAX_PAGES_SEEN = 200;

    /** @var array<string,Visit> visitor key => visit */
    private array $open = [];
    /** @var array<string,bool> keys to delete from open_visits at flush */
    private array $deleted = [];
    /** @var array<string,array<string,bool>> day => visitor => seen */
    private array $seenDay = [];
    private int $closedCount = 0;
    private int $startedCount = 0;
    /** @var array<string,string> day => salt (local cache) */
    private array $salts = [];

    /**
     * @param \Closure(string):string $salt  day => 32-byte salt
     */
    public function __construct(
        private Counters $c,
        private DayClock $clock,
        private \Closure $salt,
        private int $timeout = 1800,
        private int $nextVisitId = 1,
        private ?\PDO $pdo = null,
    ) {
        if ($pdo !== null) {
            // visitor_day is pruned to a few days by rollup: keep it in memory
            foreach ($pdo->query('SELECT day, visitor FROM visitor_day')->fetchAll(\PDO::FETCH_NUM) as [$d, $vk]) {
                $this->seenDay[(string) $d][(string) $vk] = true;
            }
        }
    }

    /** Visitor hash for a local day. */
    public function hash(string $day, string $ip, string $ua): string
    {
        $salt = $this->salts[$day] ??= ($this->salt)($day);
        return substr(hash('sha256', $salt . $ip . "\n" . $ua), 0, 16);
    }

    /** Next visit id to allocate (persisted in meta.next_visit_id). */
    public function nextVisitId(): int
    {
        return $this->nextVisitId;
    }

    /** Number of visits currently open in memory. */
    public function openCount(): int
    {
        return count($this->open);
    }

    /** Visits started during this run. */
    public function startedCount(): int
    {
        return $this->startedCount;
    }

    /** Visits closed during this run. */
    public function closedCount(): int
    {
        return $this->closedCount;
    }

    /** Loads the open visits of a domain database (persistent mode). */
    public function load(\PDO $pdo): void
    {
        foreach ($pdo->query('SELECT * FROM open_visits')->fetchAll(\PDO::FETCH_ASSOC) as $r) {
            try {
                $attrs = Json::decode((string) $r['attrs']);
                $pages = Json::decode((string) $r['pages_seen']);
            } catch (\JsonException) {
                continue;
            }
            if (!is_array($attrs) || !is_array($pages)) {
                continue;
            }
            $internal = is_array($attrs['_'] ?? null) ? $attrs['_'] : [];
            unset($attrs['_']);
            $v = new Visit((string) $r['visitor'], (int) $r['visit_id'], (string) $r['day'], (int) $r['start_ts'], (int) $r['last_ts'], $attrs);
            $v->pv = (int) $r['pageviews'];
            $v->hits = (int) $r['hits'];
            $v->bytes = (int) $r['bytes'];
            $v->lastPv = (int) $r['last_pv_ts'];
            $v->firstPv = $v->pv > 0 && isset($internal['first_pv']) ? (int) $internal['first_pv'] : null;
            $v->entry = $r['entry_page'] === null ? null : (string) $r['entry_page'];
            $v->exit = $r['exit_page'] === null ? null : (string) $r['exit_page'];
            $v->pages = array_fill_keys(array_map('strval', $pages), true);
            $v->cPv = (int) ($internal['c_pv'] ?? $v->pv);
            $v->cBounce = (int) ($internal['c_bounce'] ?? ($v->pv === 1 ? 1 : 0));
            $v->cDuration = (int) ($internal['c_duration'] ?? $v->duration());
            $v->dirty = false;
            $this->open[$v->key] = $v;
            if ($v->id >= $this->nextVisitId) {
                $this->nextVisitId = $v->id + 1;
            }
        }
    }

    /**
     * Feeds one human hit.
     *
     * @param bool     $isPageview hit classified as pageview
     * @param bool     $canStart   may open a new visit (pageview or download)
     * @param string   $key        dim key of the path (clean, ≤255 bytes)
     * @param \Closure():array<string,mixed> $attrs lazily computed visit attributes (first hit)
     * @return int|null visit id the hit was attributed to (null: no visit)
     */
    public function hit(int $ts, string $ip, string $ua, bool $isPageview, bool $canStart, string $key, int $bytes, \Closure $attrs): ?int
    {
        [$day, $hour, $midnight] = $this->clock->at($ts);
        $vk = $this->hash($day, $ip, $ua);
        $v = $this->open[$vk] ?? null;
        if ($v === null && $ts - $midnight < $this->timeout) {
            $prev = $this->hash(DayClock::prevDay($day), $ip, $ua);
            if (isset($this->open[$prev]) && $ts - $this->open[$prev]->last < $this->timeout) {
                $vk = $prev;
                $v = $this->open[$prev];
            }
        }
        if ($v !== null && $ts - $v->last >= $this->timeout) {
            $this->close($v);
            $v = null;
        }
        if ($v === null) {
            if (!$canStart) {
                return null;
            }
            $v = $this->start($vk, $ts, $day, $hour, $attrs());
        }
        if ($ts > $v->last) {
            $v->last = $ts;
        }
        $v->hits++;
        $v->bytes += $bytes;
        $v->dirty = true;
        if ($isPageview) {
            $this->pageview($v, $ts, $day, $key);
        }
        return $v->id;
    }

    /** @param array<string,mixed> $attrs */
    private function start(string $vk, int $ts, string $day, int $hour, array $attrs): Visit
    {
        $v = new Visit($vk, $this->nextVisitId++, $day, $ts, $ts, $attrs);
        $this->startedCount++;
        $c = $this->c;
        $c->day($day, 'visits');
        $c->hour($day, $hour, 'visits');
        if ($this->newVisitorDay($day, $vk)) {
            $c->day($day, 'visitors');
        }
        foreach ($v->vdims as [$dim, $k]) {
            $c->dim($day, $dim, $k, Counters::V);
        }
        unset($this->deleted[$vk]);
        $this->open[$vk] = $v;
        return $v;
    }

    private function pageview(Visit $v, int $ts, string $hitDay, string $key): void
    {
        $c = $this->c;
        $vday = $v->day;
        $v->pv++;
        if ($v->pv === 1) {
            $v->firstPv = $ts;
            $v->lastPv = $ts;
            $v->entry = $key;
            $v->exit = $key;
            $c->dim($vday, 'entry_page', $key, Counters::V);
            $c->dim($vday, 'entry_page', $key, Counters::BO);
            $c->dim($vday, 'exit_page', $key, Counters::V);
            $c->day($vday, 'bounces');
        } else {
            if ($v->pv === 2) {
                $c->day($vday, 'bounces', -1);
                $c->dim($vday, 'entry_page', (string) $v->entry, Counters::BO, -1);
            }
            $old = $v->lastPv - $v->firstPv;
            $latest = $ts >= $v->lastPv;
            if ($ts < $v->firstPv) {
                $v->firstPv = $ts;
            }
            if ($latest) {
                $v->lastPv = $ts;
            }
            $delta = ($v->lastPv - $v->firstPv) - $old;
            if ($delta !== 0) {
                $c->day($vday, 'duration', $delta);
                $c->dim($vday, 'entry_page', (string) $v->entry, Counters::DU, $delta);
            }
            if ($latest && $v->exit !== $key) {
                $c->dim($vday, 'exit_page', (string) $v->exit, Counters::V, -1);
                $c->dim($vday, 'exit_page', $key, Counters::V);
                $v->exit = $key;
            }
        }
        if (!isset($v->pages[$key]) && count($v->pages) < self::MAX_PAGES_SEEN) {
            $v->pages[$key] = true;
            $c->dim($hitDay, 'page', $key, Counters::V);
        }
    }

    /** Writes the pending pageview / bounce / duration deltas of a visit to its visit-level dims. */
    private function commitDims(Visit $v): void
    {
        $dpv = $v->pv - $v->cPv;
        $bounce = $v->pv === 1 ? 1 : 0;
        $dbo = $bounce - $v->cBounce;
        $dur = $v->duration();
        $ddu = $dur - $v->cDuration;
        if ($dpv === 0 && $dbo === 0 && $ddu === 0) {
            return;
        }
        $c = $this->c;
        foreach ($v->vdims as [$dim, $k]) {
            if ($dpv !== 0) {
                $c->dim($v->day, $dim, $k, Counters::PV, $dpv);
            }
            if ($dbo !== 0) {
                $c->dim($v->day, $dim, $k, Counters::BO, $dbo);
            }
            if ($ddu !== 0) {
                $c->dim($v->day, $dim, $k, Counters::DU, $ddu);
            }
        }
        $v->cPv = $v->pv;
        $v->cBounce = $bounce;
        $v->cDuration = $dur;
        $v->dirty = true;
    }

    /** Commits the visit-level dim deltas of every open visit (call before flushing the counters). */
    public function commitAll(): void
    {
        foreach ($this->open as $v) {
            if ($v->dirty) {
                $this->commitDims($v);
            }
        }
    }

    private function newVisitorDay(string $day, string $vk): bool
    {
        if (isset($this->seenDay[$day][$vk])) {
            return false;
        }
        $this->seenDay[$day][$vk] = true;
        $this->c->visitorDay($day, $vk);
        return true;
    }

    /** Closes one open visit: engagement buckets + `visits` row. */
    private function close(Visit $v): void
    {
        $this->commitDims($v);
        unset($this->open[$v->key]);
        $this->deleted[$v->key] = true;
        $this->closedCount++;
        $c = $this->c;
        $duration = $v->duration();
        $c->dim($v->day, 'visit_duration', Dims::durationBucket($duration), Counters::V);
        if ($v->pv > 0) {
            $c->dim($v->day, 'pages_per_visit', Dims::pagesBucket($v->pv), Counters::V);
        }
        $a = $v->attrs;
        $c->visit([
            'visit_id' => $v->id, 'visitor' => $v->key, 'day' => $v->day, 'start_ts' => $v->start, 'end_ts' => $v->last,
            'duration' => $duration, 'pageviews' => $v->pv, 'hits' => $v->hits, 'bytes' => $v->bytes,
            'entry_page' => $v->entry, 'exit_page' => $v->exit, 'channel' => $a['channel'] ?? null,
            'referrer_host' => $a['referrer_host'] ?? null, 'search_engine' => $a['search_engine'] ?? null,
            'keyword' => $a['keyword'] ?? null, 'campaign' => $a['campaign'] ?? null, 'country' => $a['country'] ?? null,
            'city' => $a['city'] ?? null, 'browser' => $a['browser'] ?? null, 'browser_version' => $a['browser_version'] ?? null,
            'os' => $a['os'] ?? null, 'os_version' => $a['os_version'] ?? null, 'device' => $a['device'] ?? null,
            'ip' => $a['ip'] ?? null,
        ]);
    }

    /**
     * Closes visits whose last hit is older than $now - timeout.
     *
     * @return int visits closed
     */
    public function closeStale(int $now): int
    {
        $n = 0;
        foreach ($this->open as $v) {
            if ($now - $v->last >= $this->timeout) {
                $this->close($v);
                $n++;
            }
        }
        return $n;
    }

    /** Closes every open visit (backfill end). */
    public function closeAll(): int
    {
        $n = count($this->open);
        foreach ($this->open as $v) {
            $this->close($v);
        }
        return $n;
    }

    /** Frees "seen visitor" memos of days before $day. */
    public function forgetDaysBefore(string $day): void
    {
        foreach (array_keys($this->seenDay) as $d) {
            if ((string) $d < $day) {
                unset($this->seenDay[$d]);
            }
        }
        if (count($this->salts) > 8) {
            $this->salts = [];
        }
    }

    /**
     * Persists open visits (persistent mode) inside the caller's transaction.
     * Call {@see Sessionizer::commitAll()} before flushing the counters.
     */
    public function flush(\PDO $pdo): void
    {
        if ($this->deleted !== []) {
            $del = $pdo->prepare('DELETE FROM open_visits WHERE visitor = ?');
            foreach (array_keys($this->deleted) as $vk) {
                $vk = (string) $vk;
                if (!isset($this->open[$vk])) {
                    $del->execute([$vk]);
                }
            }
            $this->deleted = [];
        }
        $ins = null;
        foreach ($this->open as $v) {
            if (!$v->dirty) {
                continue;
            }
            $ins ??= $pdo->prepare('INSERT OR REPLACE INTO open_visits (visitor, visit_id, day, start_ts, last_ts, last_pv_ts, pageviews, hits, bytes,
                entry_page, exit_page, pages_seen, attrs) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)');
            $attrs = $v->attrs;
            $attrs['_'] = ['first_pv' => $v->firstPv, 'c_pv' => $v->cPv, 'c_bounce' => $v->cBounce, 'c_duration' => $v->cDuration];
            $ins->execute([
                $v->key, $v->id, $v->day, $v->start, $v->last, $v->lastPv, $v->pv, $v->hits, $v->bytes,
                $v->entry, $v->exit, Json::encode(array_map('strval', array_keys($v->pages))), Json::encode($attrs),
            ]);
            $v->dirty = false;
        }
    }

    /**
     * Visit-level dimension rows of a visit, from its attributes.
     *
     * @param array<string,mixed> $a
     * @return array<int,array{0:string,1:string}>
     */
    public static function visitDims(array $a): array
    {
        $out = [];
        foreach (['channel', 'referrer_host', 'referrer_url', 'search_engine', 'keyword', 'campaign', 'utm_source', 'utm_medium',
            'country', 'city', 'browser', 'browser_version', 'os', 'os_version', 'device'] as $d) {
            if (isset($a[$d]) && is_string($a[$d]) && $a[$d] !== '') {
                $out[] = [$d, $a[$d]];
            }
        }
        if (!empty($a['utm_source']) || !empty($a['utm_medium'])) {
            $out[] = ['source_medium', ($a['utm_source'] ?: '(not set)') . ' / ' . ($a['utm_medium'] ?: '(not set)')];
        }
        return $out;
    }
}
