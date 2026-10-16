<?php

declare(strict_types=1);

namespace WebPulse\Collect;

use WebPulse\App;
use WebPulse\Classify\Path;
use WebPulse\Classify\Referrer;
use WebPulse\Classify\UserAgent;
use WebPulse\Discovery\LogFiles;
use WebPulse\Log\Hit;
use WebPulse\Log\LogParser;
use WebPulse\Log\LogSource;
use WebPulse\Paths;
use WebPulse\Store\DomainStore;

/**
 * The collector pipeline (ARCHITECTURE.md): discovery -> LogSource ->
 * LogParser -> classifiers -> Sessionizer -> DomainStore.
 *
 * All the live logs of a domain (`<domain>` and `<domain>-ssl_log`, possibly
 * in several directories) are merged by timestamp so that a visit spanning
 * http and https is sessionized correctly. Counters and checkpoints are
 * written in one transaction per domain (every FLUSH_LINES lines and at the
 * end). A bad line or an unreadable file never aborts the run.
 *
 * The caller (CLI) holds the per-user lock.
 */
final class Collector
{
    /** Intermediate flush every N lines (bounds memory). */
    public const FLUSH_LINES = 100000;
    /** Sweep stale visits every N lines. */
    private const SWEEP_LINES = 10000;
    /** Tolerance for out-of-order lines when sweeping visits mid-run. */
    private const SWEEP_SLACK = 300;

    private LogParser $parser;
    private UserAgent $ua;
    private Referrer $referrer;
    private Path $path;
    private LogFiles $logFiles;

    /** Builds the parser and classifiers once for the whole run. */
    public function __construct(private App $app)
    {
        $this->parser = new LogParser();
        $this->ua = new UserAgent();
        $this->referrer = new Referrer();
        $prefixes = $app->config->get('static_prefixes', []);
        $this->path = new Path(is_array($prefixes) ? array_values(array_filter($prefixes, 'is_string')) : []);
        $this->logFiles = new LogFiles($app->config, $app->paths);
    }

    /** Log discovery helper used by the collector. */
    public function logFiles(): LogFiles
    {
        return $this->logFiles;
    }

    /**
     * Discovery -> registry for one user (or all users when null).
     *
     * @return string[] users seen
     */
    public function syncRegistry(?string $onlyUser = null): array
    {
        $cp = $this->app->cpanelUsers();
        $reg = $this->app->registry();
        if ($onlyUser !== null) {
            $info = Paths::isValidUser($onlyUser) && $onlyUser !== 'root' && $onlyUser !== 'system' ? $cp->get($onlyUser) : null;
            $users = $info !== null ? [$onlyUser => $info] : [];
        } else {
            $users = $cp->all();
        }
        foreach ($users as $user => $info) {
            $domains = $info['kinds'];
            foreach ($this->logFiles->domainsInUserDirs($user) as $d) {
                $domains[$d] ??= 'unknown';
            }
            $reg->syncUser($user, $info['owner'], $domains);
        }
        if ($onlyUser === null) {
            $reg->markRemovedExcept(array_keys($users));
        }
        return array_keys($users);
    }

    /**
     * Collects every enabled domain of a user.
     *
     * @return array<string,mixed> totals: domains, files, lines, bad, ignored, errors, skipped[]
     */
    public function collectUser(string $user, ?string $onlyDomain = null): array
    {
        Paths::assertUser($user);
        $tot = ['domains' => 0, 'files' => 0, 'lines' => 0, 'bad' => 0, 'ignored' => 0, 'errors' => 0, 'skipped' => []];
        $reg = $this->app->registry();
        $settings = UserSettings::read($this->app->paths, $user, $err);
        if ($err !== null) {
            $this->app->log->warn("ignoring settings.json of $user: $err");
        }
        $domains = [];
        foreach ($reg->domains($user) as $row) {
            if ($row['removed_at'] === null) {
                $domains[] = $row['domain'];
            }
        }
        if ($onlyDomain !== null) {
            $domains = array_values(array_filter($domains, static fn ($d) => $d === $onlyDomain));
            if ($domains === []) {
                // never collect a domain of another account into this one
                $tot['skipped'][] = "$onlyDomain (not a domain of $user)";
                return $tot;
            }
        }
        foreach ($domains as $domain) {
            if (!$reg->isEnabled($user, $domain)) {
                $tot['skipped'][] = "$domain (disabled)";
                continue;
            }
            try {
                $r = $this->collectDomain($user, $domain, $settings);
            } catch (\Throwable $e) {
                $tot['errors']++;
                $this->app->log->error("collect $user/$domain failed: " . $e->getMessage());
                continue;
            }
            if ($r === null) {
                continue;
            }
            $tot['domains']++;
            foreach (['files', 'lines', 'bad', 'ignored', 'errors'] as $k) {
                $tot[$k] += $r[$k];
            }
        }
        return $tot;
    }

    /**
     * True when cPanel lists $domain for $user (/var/cpanel/users/<user> or
     * /etc/userdomains). Registry rows alone do not count: they can be
     * created from WHM by a reseller.
     */
    public function ownsDomain(string $user, string $domain): bool
    {
        return isset($this->app->cpanelUsers()->domainsOf($user)[$domain]);
    }

    /**
     * Live logs of a domain that may be read for $user: files in per-account
     * directories always, files in shared directories (no {user} in the
     * template, e.g. /etc/apache2/logs/domlogs) only when the domain belongs
     * to the account — otherwise any tenant's log could be imported.
     *
     * @return array<int,array{path:string,https:bool,shared:bool}>
     */
    private function liveFiles(string $user, string $domain): array
    {
        $files = $this->logFiles->forDomain($user, $domain);
        $shared = array_filter($files, static fn (array $f): bool => $f['shared']);
        if ($shared !== [] && !$this->ownsDomain($user, $domain)) {
            $this->app->log->warn("not reading the shared logs of $domain for $user: cPanel does not list it for this account",
                ['files' => array_values(array_column($shared, 'path'))]);
            $files = array_values(array_filter($files, static fn (array $f): bool => !$f['shared']));
        }
        return $files;
    }

    /**
     * Tails the live logs of one domain.
     *
     * @param array<string,mixed>|null $settings validated user settings (read when null)
     * @return array<string,mixed>|null run stats; null when the domain has neither logs nor a database
     */
    public function collectDomain(string $user, string $domain, ?array $settings = null): ?array
    {
        $files = $this->liveFiles($user, $domain);
        $dbFile = $this->app->paths->domainDb($user, $domain);
        if ($files === [] && !is_file($dbFile)) {
            return null;
        }
        $settings ??= UserSettings::read($this->app->paths, $user);
        $ds = UserSettings::forDomain($settings, $domain);
        $store = DomainStore::open($this->app->paths, $user, $domain, false);
        $tz = $this->resolveTimezone($store, $user, $ds);
        $now = $this->app->now();
        $stats = ['files' => 0, 'lines' => 0, 'bad' => 0, 'ignored' => 0, 'errors' => 0, 'resets' => []];
        $cps = $this->checkpoints($store);
        if ($this->isIdle($store, $files, $cps, $tz, $now)) {
            $stats['files'] = count($files);
            $stats['idle'] = true;
            return $stats;
        }
        // newest line already processed: stored visits stale at that point can be closed while loading
        $watermark = null;
        foreach ($cps as $cp) {
            if ($cp['last_ts'] !== null) {
                $watermark = max($watermark ?? 0, (int) $cp['last_ts'] - self::SWEEP_SLACK);
            }
        }
        $run = $this->newRun($store, $domain, $tz, $ds, true, $watermark);

        $sources = [];
        foreach ($files as $f) {
            $src = new LogSource($f['path'], $f['https'], $cps[$f['path']] ?? null);
            try {
                $src->open();
            } catch (\Throwable $e) {
                $stats['errors']++;
                $this->app->log->error("cannot read {$f['path']}: " . $e->getMessage());
                continue;
            }
            if ($src->resetReason() !== null) {
                $stats['resets'][] = basename($f['path']) . ':' . $src->resetReason();
                $this->app->log->info("log {$f['path']} restarted from 0 ({$src->resetReason()})");
            }
            $sources[] = $src;
        }
        $stats['files'] = count($sources);
        $backfillUntil = (int) ($store->getMeta('backfill_until') ?? 0);
        $skip = $backfillUntil > 0 && $store->getMeta('live_since') === null
            ? static fn (Hit $h, int $i): bool => $h->ts <= $backfillUntil
            : null;

        $state = $this->runMerged($run, $sources, $stats, $skip, function (array $srcStats) use ($run, $sources, $now): void {
            $run->flush($this->checkpointRows($sources, $srcStats, $now));
        });

        $run->sessions->closeStale($now);
        $meta = ['last_collect_at' => (string) $now, 'timezone' => $tz];
        if ($store->getMeta('live_since') === null && $run->minTs() !== null) {
            $meta['live_since'] = (string) $run->minTs();
        }
        $run->flush($this->checkpointRows($sources, $state, $now), [], $meta);
        foreach ($sources as $s) {
            $s->close();
        }
        try {
            $this->app->registry()->refreshSummary($store, $tz, $now);
        } catch (\Throwable $e) {
            $this->app->log->warn("summary refresh failed for $user/$domain: " . $e->getMessage());
        }
        $stats['classes'] = $run->classes;
        if ($run->sessions->evictedCount() > 0) {
            $stats['visits_closed_early'] = $run->sessions->evictedCount();
            $this->app->log->warn("$user/$domain: more than {$run->sessions->maxOpen()} visits open at once (max_open_visits); "
                . $run->sessions->evictedCount() . ' closed early');
        }
        if ($stats['lines'] > 0 || $stats['errors'] > 0) {
            $this->app->log->info("collected $user/$domain", ['files' => $stats['files'], 'lines' => $stats['lines'], 'bad' => $stats['bad'], 'ignored' => $stats['ignored']]);
        }
        return $stats;
    }

    /**
     * Closes the visits of a domain whose last hit is older than the session
     * timeout (used by rollup and for domains without new lines).
     *
     * @return int visits closed
     */
    public function closeStaleVisits(DomainStore $store, string $tz): int
    {
        $now = $this->app->now();
        $run = $this->newRun($store, $store->domain(), $tz, UserSettings::domainDefaults(), true, $now);
        $run->sessions->closeStale($now);
        $n = $run->sessions->closedCount(); // includes the visits closed while loading
        if ($n > 0) {
            $run->flush();
        }
        return $n;
    }

    /**
     * Imports compressed archives of a user (never twice; lines already
     * covered by live collection are skipped).
     *
     * @return array<string,mixed> totals: domains, files, lines, imported, skipped_files, errors
     */
    public function backfillUser(string $user, ?string $onlyDomain = null, ?string $archiveDir = null): array
    {
        Paths::assertUser($user);
        $reg = $this->app->registry();
        $tot = ['domains' => 0, 'files' => 0, 'lines' => 0, 'imported' => 0, 'bad' => 0, 'ignored' => 0, 'skipped_files' => 0, 'errors' => 0, 'skipped' => []];
        $domains = [];
        foreach ($reg->domains($user) as $row) {
            $domains[] = $row['domain'];
        }
        foreach ($this->app->cpanelUsers()->domainsOf($user) as $d => $kind) {
            $domains[] = $d;
        }
        $domains = array_values(array_unique($domains));
        $archives = $this->logFiles->archives($user, $domains, $archiveDir);
        $settings = UserSettings::read($this->app->paths, $user);
        foreach ($archives as $domain => $list) {
            if ($onlyDomain !== null && $domain !== $onlyDomain) {
                continue;
            }
            if (!$reg->isEnabled($user, $domain)) {
                $tot['skipped'][] = "$domain (disabled)";
                continue;
            }
            try {
                $r = $this->backfillDomain($user, $domain, $list, $settings, $archiveDir !== null);
            } catch (\Throwable $e) {
                $tot['errors']++;
                $this->app->log->error("backfill $user/$domain failed: " . $e->getMessage());
                continue;
            }
            if (isset($r['skipped'])) {
                $tot['skipped'][] = "$domain ({$r['skipped']})";
                continue;
            }
            $tot['domains']++;
            foreach (['files', 'lines', 'imported', 'bad', 'ignored', 'skipped_files', 'errors'] as $k) {
                $tot[$k] += $r[$k];
            }
        }
        return $tot;
    }

    /**
     * @param array<int,array{path:string,https:bool,size:int,mtime:int}> $archives
     * @param array<string,mixed> $settings
     * @param bool $trustedDir the archives come from a directory given explicitly by root (--archive-dir)
     * @return array<string,mixed> stats; with a `skipped` reason when nothing was read
     */
    public function backfillDomain(string $user, string $domain, array $archives, array $settings, bool $trustedDir = false): array
    {
        if (!$trustedDir && $this->app->paths->archiveDirShared() && !$this->ownsDomain($user, $domain)) {
            // one archive directory for every account: only the owner's domains may be imported
            $this->app->log->warn("not importing the archives of $domain for $user: cPanel does not list it for this account");
            return ['files' => 0, 'lines' => 0, 'imported' => 0, 'bad' => 0, 'ignored' => 0, 'skipped_files' => 0, 'errors' => 0,
                'skipped' => "not a domain of $user"];
        }
        $ds = UserSettings::forDomain($settings, $domain);
        $store = DomainStore::open($this->app->paths, $user, $domain, false);
        $tz = $this->resolveTimezone($store, $user, $ds);
        $now = $this->app->now();
        $stats = ['files' => 0, 'lines' => 0, 'imported' => 0, 'bad' => 0, 'ignored' => 0, 'skipped_files' => 0, 'errors' => 0];
        $done = [];
        foreach ($store->pdo()->query('SELECT path, size, mtime, last_ts FROM imported_archives')->fetchAll(\PDO::FETCH_ASSOC) as $r) {
            $done[(string) $r['path']] = $r;
        }
        $sources = [];
        $resumeTs = [];
        foreach ($archives as $a) {
            $prev = $done[$a['path']] ?? null;
            if ($prev !== null && (int) $prev['size'] === $a['size'] && (int) $prev['mtime'] === $a['mtime']) {
                $stats['skipped_files']++;
                continue;
            }
            $src = new LogSource($a['path'], $a['https'], null, true);
            try {
                $src->open();
            } catch (\Throwable $e) {
                $stats['errors']++;
                $this->app->log->error("cannot read archive {$a['path']}: " . $e->getMessage());
                continue;
            }
            $resumeTs[count($sources)] = $prev !== null ? (int) ($prev['last_ts'] ?? 0) : 0;
            $sources[] = $src;
        }
        $stats['files'] = count($sources);
        if ($sources === []) {
            return $stats;
        }
        $liveSince = $store->getMeta('live_since');
        $limit = $liveSince === null ? PHP_INT_MAX : (int) $liveSince;
        $skip = static fn (Hit $h, int $i): bool => $h->ts >= $limit || $h->ts <= $resumeTs[$i];

        $run = $this->newRun($store, $domain, $tz, $ds, false);
        $rows = function (array $srcStats, bool $complete) use ($sources, $now): array {
            $out = [];
            foreach ($sources as $i => $s) {
                $out[] = [
                    'path' => $s->path(), 'size' => $complete ? $s->size() : -1, 'mtime' => $s->mtime(),
                    'lines' => $srcStats[$i]['lines'] ?? 0, 'imported_at' => $now, 'last_ts' => $srcStats[$i]['last_ts'] ?? null,
                ];
            }
            return $out;
        };
        $state = $this->runMerged($run, $sources, $stats, $skip, function (array $srcStats) use ($run, $rows): void {
            $run->flush([], $rows($srcStats, false));
        });
        $run->sessions->closeAll();
        $meta = [];
        if ($liveSince === null && $run->minTs() !== null) {
            $prevUntil = (int) ($store->getMeta('backfill_until') ?? 0);
            $maxTs = 0;
            foreach ($state as $s) {
                $maxTs = max($maxTs, (int) ($s['max_ts'] ?? 0));
            }
            $meta['backfill_until'] = (string) max($prevUntil, $maxTs);
        }
        $meta['timezone'] = $tz;
        $run->flush([], $rows($state, true), $meta);
        foreach ($sources as $s) {
            $s->close();
        }
        $stats['imported'] = $run->lines();
        try {
            $this->app->registry()->refreshSummary($store, $tz, $now);
        } catch (\Throwable $e) {
            $this->app->log->warn("summary refresh failed for $user/$domain: " . $e->getMessage());
        }
        $this->app->log->info("backfilled $user/$domain", ['files' => $stats['files'], 'lines' => $stats['lines'], 'imported' => $stats['imported']]);
        return $stats;
    }

    /**
     * Merges the sources by timestamp and feeds the run.
     *
     * @param LogSource[] $sources
     * @param array<string,mixed> $stats   totals updated in place (lines, bad, ignored)
     * @param (\Closure(Hit,int):bool)|null $skip  lines to drop silently (already imported)
     * @param \Closure(array):void $flush  intermediate flush with per-source stats
     * @return array<int,array<string,int|null>> per-source stats since the last flush
     */
    private function runMerged(DomainRun $run, array $sources, array &$stats, ?\Closure $skip, \Closure $flush): array
    {
        $n = count($sources);
        $heads = [];
        $src = [];
        for ($i = 0; $i < $n; $i++) {
            $src[$i] = ['lines' => 0, 'bad' => 0, 'ignored' => 0, 'offset' => $sources[$i]->offset(), 'last_ts' => null, 'max_ts' => null];
        }
        $fill = function (int $i) use (&$heads, &$src, $sources, &$stats, $run): void {
            $s = $sources[$i];
            try {
                while (($line = $s->next()) !== null) {
                    $src[$i]['lines']++;
                    $stats['lines']++;
                    $run->pendingLines++;
                    $hit = $line === '' ? null : $this->parser->parse($line);
                    if ($hit === null) {
                        $src[$i]['bad']++;
                        $stats['bad']++;
                        continue;
                    }
                    $heads[$i] = [$hit, $s->offset()];
                    return;
                }
            } catch (\Throwable $e) {
                $stats['errors']++;
                $this->app->log->error('read error on ' . $s->path() . ': ' . $e->getMessage());
            }
            $heads[$i] = null;
            $src[$i]['offset'] = $s->offset();
        };
        for ($i = 0; $i < $n; $i++) {
            $fill($i);
        }
        $count = 0;
        $watermark = 0;
        while (true) {
            $best = -1;
            $bestTs = PHP_INT_MAX;
            for ($i = 0; $i < $n; $i++) {
                if ($heads[$i] !== null && $heads[$i][0]->ts < $bestTs) {
                    $best = $i;
                    $bestTs = $heads[$i][0]->ts;
                }
            }
            if ($best < 0) {
                break;
            }
            [$hit, $off] = $heads[$best];
            if ($skip === null || !$skip($hit, $best)) {
                $class = $run->process($hit, $sources[$best]->https());
                if ($class === DomainRun::CLASS_IGNORED) {
                    $src[$best]['ignored']++;
                    $stats['ignored']++;
                }
            }
            $src[$best]['offset'] = $off;
            $src[$best]['last_ts'] = $hit->ts;
            if ($hit->ts > ($src[$best]['max_ts'] ?? 0)) {
                $src[$best]['max_ts'] = $hit->ts;
            }
            if ($hit->ts > $watermark) {
                $watermark = $hit->ts;
            }
            $fill($best);
            $count++;
            if ($count % self::SWEEP_LINES === 0) {
                $run->sessions->closeStale($watermark - self::SWEEP_SLACK);
                $run->sessions->forgetDaysBefore(\WebPulse\Util\DayClock::prevDay($run->clock()->day($watermark)));
            }
            if ($count % self::FLUSH_LINES === 0 || $run->counters->rawRows() > 200000) {
                $flush($src);
                foreach ($src as $i => $_) {
                    $src[$i]['lines'] = $src[$i]['bad'] = $src[$i]['ignored'] = 0;
                }
            }
        }
        return $src;
    }

    /**
     * @param LogSource[] $sources
     * @param array<int,array<string,int|null>> $srcStats
     * @return array<int,array<string,mixed>>
     */
    private function checkpointRows(array $sources, array $srcStats, int $now): array
    {
        $rows = [];
        foreach ($sources as $i => $s) {
            $st = $srcStats[$i] ?? [];
            $off = (int) ($st['offset'] ?? $s->offset());
            $rows[] = [
                'path' => $s->path(), 'inode' => $s->inode(), 'offset' => $off, 'size' => max($s->size(), $off),
                'first_line' => $s->fingerprint(), 'last_ts' => $st['last_ts'] ?? null,
                'lines' => (int) ($st['lines'] ?? 0), 'bad_lines' => (int) ($st['bad'] ?? 0),
                'ignored_lines' => (int) ($st['ignored'] ?? 0), 'updated_at' => $now,
            ];
        }
        return $rows;
    }

    /**
     * True when nothing would change: every log is at its checkpoint, no open
     * visit is due to close, the timezone is unchanged and the database was
     * touched less than an hour ago. Idle domains are not written at all
     * (cron runs every 5 minutes on servers with thousands of domains).
     *
     * @param array<int,array{path:string,https:bool}> $files
     * @param array<string,array<string,mixed>> $cps
     */
    private function isIdle(DomainStore $store, array $files, array $cps, string $tz, int $now): bool
    {
        if ($files === [] && $cps === []) {
            return false;
        }
        foreach ($files as $f) {
            $cp = $cps[$f['path']] ?? null;
            clearstatcache(true, $f['path']);
            $st = @stat($f['path']);
            if ($cp === null || $st === false || (int) $cp['inode'] !== (int) $st['ino'] || (int) $cp['offset'] !== (int) $st['size']) {
                return false;
            }
        }
        $meta = $store->meta();
        if (($meta['timezone'] ?? null) !== $tz || $now - (int) ($meta['last_collect_at'] ?? 0) >= 3600) {
            return false;
        }
        $timeout = max(60, (int) $this->app->config->get('session_timeout_min', 30) * 60);
        $st = $store->pdo()->prepare('SELECT COUNT(*) FROM open_visits WHERE last_ts <= ?');
        $st->execute([$now - $timeout]);
        if ((int) $st->fetchColumn() !== 0) {
            return false;
        }
        // more stored visits than the cap (e.g. after lowering max_open_visits): load to close the excess
        $cap = (int) $this->app->config->get('max_open_visits', Sessionizer::DEFAULT_MAX_OPEN);
        return $cap <= 0 || (int) $store->pdo()->query('SELECT COUNT(*) FROM open_visits')->fetchColumn() <= $cap;
    }

    /** @return array<string,array<string,mixed>> path => checkpoint row */
    private function checkpoints(DomainStore $store): array
    {
        $out = [];
        foreach ($store->pdo()->query('SELECT * FROM checkpoints')->fetchAll(\PDO::FETCH_ASSOC) as $r) {
            $out[(string) $r['path']] = $r;
        }
        return $out;
    }

    /**
     * @param array{timezone:?string,exclude_ips:string[],exclude_paths:string[]} $ds
     */
    private function newRun(DomainStore $store, string $domain, string $tz, array $ds, bool $persistent, ?int $watermark = null): DomainRun
    {
        $reg = $this->app->registry();
        return new DomainRun(
            $store,
            $domain,
            $tz,
            $ds,
            $this->app->config->effective(),
            $this->ua,
            $this->referrer,
            $this->path,
            $this->app->geoIp(),
            static fn (string $day): string => $reg->salt($day),
            $persistent,
            $watermark,
        );
    }

    /**
     * Domain timezone: user settings > registry account > config > server.
     * Records a change in meta.tz_changed_at.
     *
     * @param array{timezone:?string} $ds
     */
    public function resolveTimezone(DomainStore $store, string $user, array $ds): string
    {
        $tz = $ds['timezone'] ?? null;
        if ($tz === null) {
            $acc = $this->app->registry()->account($user);
            $tz = $acc['timezone'] ?? null;
        }
        $tz ??= $this->app->serverTimezone();
        $old = $store->getMeta('timezone');
        if ($old !== null && $old !== $tz) {
            $store->setMetaMany(['timezone' => $tz, 'tz_changed_at' => (string) $this->app->now()]);
            $this->app->log->info("timezone of {$store->domain()} changed from $old to $tz");
        }
        return $tz;
    }
}
