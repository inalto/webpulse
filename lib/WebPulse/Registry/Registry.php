<?php

declare(strict_types=1);

namespace WebPulse\Registry;

use WebPulse\App;
use WebPulse\Config;
use WebPulse\Paths;
use WebPulse\Store\DomainStore;
use WebPulse\Store\Migrator;
use WebPulse\Store\Schema;
use WebPulse\Util\DayClock;
use WebPulse\Util\Fs;
use WebPulse\Util\Json;

/**
 * The registry database (/var/webpulse/webpulse.db, root 0600): account and
 * domain switches, settings overrides, jobs, daily salts and the per-domain
 * summary used by the WHM overview.
 *
 * AccountRow: user, owner, enabled (effective bool), enabled_override (?bool),
 *             timezone, first_seen, last_seen, removed_at, domains (int count)
 * DomainRow:  user, domain, kind, enabled (effective bool), enabled_override (?bool),
 *             account_enabled (bool), first_seen, last_seen, removed_at
 */
final class Registry
{
    private \PDO $pdo;
    /** @var array<string,string> */
    private array $saltCache = [];

    /**
     * Opens (creating and migrating when needed) the registry.
     *
     * @throws \RuntimeException|\PDOException
     */
    public function __construct(private string $path, private Config $config, private ?App $app = null)
    {
        $dir = dirname($path);
        if (!is_dir($dir)) {
            Fs::ensureDir($dir, 0711);
        }
        $fresh = !is_file($path);
        if (!$fresh && !is_readable($path)) {
            throw new \RuntimeException("registry $path is not readable");
        }
        $this->pdo = new \PDO('sqlite:' . $path, null, null, [
            \PDO::ATTR_ERRMODE => \PDO::ERRMODE_EXCEPTION,
            \PDO::ATTR_TIMEOUT => 5,
        ]);
        $this->pdo->exec('PRAGMA busy_timeout = 5000');
        if (is_writable($path) || $fresh) {
            $this->pdo->exec('PRAGMA journal_mode = DELETE');
            $this->pdo->exec('PRAGMA synchronous = NORMAL');
            Migrator::migrate($this->pdo, Schema::registryMigrations());
            if ($fresh) {
                @chmod($path, 0600);
                $st = $this->pdo->prepare('INSERT OR IGNORE INTO meta (key, value) VALUES (?, ?)');
                $st->execute(['installed_at', (string) App::clock()]);
            }
            if ($app !== null) {
                $st = $this->pdo->prepare("INSERT INTO meta (key, value) VALUES ('version', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value");
                $st->execute([$app->version()]);
            }
        }
    }

    public function pdo(): \PDO
    {
        return $this->pdo;
    }

    public function path(): string
    {
        return $this->path;
    }

    private function now(): int
    {
        return App::clock();
    }

    private function defaultEnabled(): bool
    {
        return (bool) $this->config->get('default_enabled', true);
    }

    private static function tri(mixed $v): ?bool
    {
        return $v === null ? null : ((int) $v) === 1;
    }

    // ------------------------------------------------------------------ accounts

    /**
     * Accounts with effective enabled flag.
     *
     * @param string|null $owner only accounts whose OWNER is $owner (reseller scope)
     * @param string|null $q     substring filter on user name or one of its domains
     * @return array<int,array<string,mixed>> AccountRow[]
     */
    public function accounts(?string $owner = null, ?string $q = null): array
    {
        $sql = 'SELECT a.*, (SELECT COUNT(*) FROM domains d WHERE d.user = a.user AND d.removed_at IS NULL) AS domains
                FROM accounts a WHERE 1 = 1';
        $args = [];
        if ($owner !== null) {
            $sql .= ' AND a.owner = ?';
            $args[] = $owner;
        }
        if ($q !== null && $q !== '') {
            $sql .= " AND (a.user LIKE ? ESCAPE '\\' OR EXISTS (SELECT 1 FROM domains d WHERE d.user = a.user AND d.domain LIKE ? ESCAPE '\\'))";
            $like = '%' . addcslashes(strtolower($q), '%_\\') . '%';
            $args[] = $like;
            $args[] = $like;
        }
        $sql .= ' ORDER BY a.user';
        $st = $this->pdo->prepare($sql);
        $st->execute($args);
        $def = $this->defaultEnabled();
        $out = [];
        foreach ($st->fetchAll(\PDO::FETCH_ASSOC) as $r) {
            $out[] = $this->accountRow($r, $def);
        }
        return $out;
    }

    /** @param array<string,mixed> $r */
    private function accountRow(array $r, bool $def): array
    {
        $ov = self::tri($r['enabled']);
        return [
            'user' => (string) $r['user'],
            'owner' => $r['owner'] === null ? null : (string) $r['owner'],
            'enabled' => $ov ?? $def,
            'enabled_override' => $ov,
            'timezone' => $r['timezone'] === null ? null : (string) $r['timezone'],
            'first_seen' => (int) $r['first_seen'],
            'last_seen' => (int) $r['last_seen'],
            'removed_at' => $r['removed_at'] === null ? null : (int) $r['removed_at'],
            'domains' => (int) ($r['domains'] ?? 0),
        ];
    }

    /** One AccountRow or null. */
    public function account(string $user): ?array
    {
        $st = $this->pdo->prepare('SELECT a.*, (SELECT COUNT(*) FROM domains d WHERE d.user = a.user AND d.removed_at IS NULL) AS domains FROM accounts a WHERE a.user = ?');
        $st->execute([$user]);
        $r = $st->fetch(\PDO::FETCH_ASSOC);
        return $r === false ? null : $this->accountRow($r, $this->defaultEnabled());
    }

    /**
     * Sets the account switch.
     *
     * @param bool|null   $enabled  true/false = override, null = follow default_enabled
     * @param string|null $timezone null = unchanged, '' = clear (server timezone), else an identifier
     * @throws \InvalidArgumentException
     */
    public function setAccount(string $user, ?bool $enabled, ?string $timezone = null): void
    {
        Paths::assertUser($user);
        if ($timezone !== null && $timezone !== '' && !in_array($timezone, \DateTimeZone::listIdentifiers(), true)) {
            throw new \InvalidArgumentException("invalid timezone '$timezone'");
        }
        $this->ensureAccount($user);
        $st = $this->pdo->prepare('UPDATE accounts SET enabled = ? WHERE user = ?');
        $st->execute([$enabled === null ? null : (int) $enabled, $user]);
        if ($timezone !== null) {
            $st = $this->pdo->prepare('UPDATE accounts SET timezone = ? WHERE user = ?');
            $st->execute([$timezone === '' ? null : $timezone, $user]);
        }
    }

    private function ensureAccount(string $user): void
    {
        $owner = null;
        if ($this->app !== null) {
            try {
                $owner = $this->app->cpanelUsers()->owner($user);
            } catch (\Throwable) {
                $owner = null;
            }
        }
        $now = $this->now();
        $st = $this->pdo->prepare('INSERT OR IGNORE INTO accounts (user, owner, enabled, timezone, first_seen, last_seen) VALUES (?, ?, NULL, NULL, ?, ?)');
        $st->execute([$user, $owner, $now, $now]);
    }

    /** True when collection is enabled for the account as a whole. */
    public function accountEnabled(string $user): bool
    {
        $st = $this->pdo->prepare('SELECT enabled FROM accounts WHERE user = ?');
        $st->execute([$user]);
        $v = $st->fetchColumn();
        $ov = $v === false ? null : self::tri($v);
        return $ov ?? $this->defaultEnabled();
    }

    // ------------------------------------------------------------------- domains

    /**
     * Domains (optionally of one user) with effective enabled flags.
     *
     * @return array<int,array<string,mixed>> DomainRow[]
     */
    public function domains(?string $user = null): array
    {
        $sql = 'SELECT d.*, a.enabled AS account_enabled FROM domains d LEFT JOIN accounts a ON a.user = d.user';
        $args = [];
        if ($user !== null) {
            $sql .= ' WHERE d.user = ?';
            $args[] = $user;
        }
        $sql .= ' ORDER BY d.user, d.domain';
        $st = $this->pdo->prepare($sql);
        $st->execute($args);
        $def = $this->defaultEnabled();
        $out = [];
        foreach ($st->fetchAll(\PDO::FETCH_ASSOC) as $r) {
            $acc = self::tri($r['account_enabled']);
            $dom = self::tri($r['enabled']);
            $out[] = [
                'user' => (string) $r['user'],
                'domain' => (string) $r['domain'],
                'kind' => $r['kind'] === null ? 'unknown' : (string) $r['kind'],
                'enabled' => self::effective($acc, $dom, $def),
                'enabled_override' => $dom,
                'account_enabled' => $acc ?? $def,
                'first_seen' => (int) $r['first_seen'],
                'last_seen' => (int) $r['last_seen'],
                'removed_at' => $r['removed_at'] === null ? null : (int) $r['removed_at'],
            ];
        }
        return $out;
    }

    /**
     * Effective switch: an account explicitly disabled turns off all of its
     * domains; otherwise a domain override wins, else the account setting,
     * else default_enabled.
     */
    public static function effective(?bool $account, ?bool $domain, bool $default): bool
    {
        if ($account === false) {
            return false;
        }
        return $domain ?? $account ?? $default;
    }

    /**
     * Sets the domain switch (null = follow the account).
     *
     * @throws \InvalidArgumentException
     */
    public function setDomain(string $user, string $domain, ?bool $enabled): void
    {
        Paths::assertUser($user);
        Paths::assertDomain($domain);
        $this->ensureAccount($user);
        $now = $this->now();
        $st = $this->pdo->prepare('INSERT INTO domains (user, domain, enabled, kind, first_seen, last_seen) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(user, domain) DO UPDATE SET enabled = excluded.enabled');
        $st->execute([$user, $domain, $enabled === null ? null : (int) $enabled, 'unknown', $now, $now]);
    }

    /** Effective collection switch for one domain. */
    public function isEnabled(string $user, string $domain): bool
    {
        $st = $this->pdo->prepare('SELECT a.enabled FROM accounts a WHERE a.user = ?');
        $st->execute([$user]);
        $a = $st->fetchColumn();
        $st = $this->pdo->prepare('SELECT enabled FROM domains WHERE user = ? AND domain = ?');
        $st->execute([$user, $domain]);
        $d = $st->fetchColumn();
        return self::effective($a === false ? null : self::tri($a), $d === false ? null : self::tri($d), $this->defaultEnabled());
    }

    /**
     * Records what discovery saw for one account (owner, domains with kinds)
     * and marks domains no longer present as removed.
     *
     * @param array<string,string> $domains domain => kind
     */
    public function syncUser(string $user, ?string $owner, array $domains): void
    {
        $now = $this->now();
        $this->pdo->exec('BEGIN IMMEDIATE');
        try {
            $st = $this->pdo->prepare('INSERT INTO accounts (user, owner, enabled, timezone, first_seen, last_seen) VALUES (?, ?, NULL, NULL, ?, ?)
                ON CONFLICT(user) DO UPDATE SET owner = excluded.owner, last_seen = excluded.last_seen, removed_at = NULL');
            $st->execute([$user, $owner, $now, $now]);
            $st = $this->pdo->prepare('INSERT INTO domains (user, domain, enabled, kind, first_seen, last_seen) VALUES (?, ?, NULL, ?, ?, ?)
                ON CONFLICT(user, domain) DO UPDATE SET kind = excluded.kind, last_seen = excluded.last_seen, removed_at = NULL');
            foreach ($domains as $d => $kind) {
                $st->execute([$user, $d, $kind, $now, $now]);
            }
            $known = array_keys($domains);
            $rm = $this->pdo->prepare('UPDATE domains SET removed_at = ? WHERE user = ? AND domain = ? AND removed_at IS NULL');
            $cur = $this->pdo->prepare('SELECT domain FROM domains WHERE user = ? AND removed_at IS NULL');
            $cur->execute([$user]);
            foreach ($cur->fetchAll(\PDO::FETCH_COLUMN) as $d) {
                if (!in_array($d, $known, true)) {
                    $rm->execute([$now, $user, $d]);
                }
            }
            $this->pdo->exec('COMMIT');
        } catch (\Throwable $e) {
            $this->pdo->exec('ROLLBACK');
            throw $e;
        }
    }

    /**
     * Marks every account not in $present (and its domains) as removed.
     *
     * @param string[] $present
     */
    public function markRemovedExcept(array $present): int
    {
        $now = $this->now();
        $n = 0;
        $st = $this->pdo->query('SELECT user FROM accounts WHERE removed_at IS NULL');
        $up = $this->pdo->prepare('UPDATE accounts SET removed_at = ? WHERE user = ?');
        $upd = $this->pdo->prepare('UPDATE domains SET removed_at = ? WHERE user = ? AND removed_at IS NULL');
        foreach ($st->fetchAll(\PDO::FETCH_COLUMN) as $u) {
            if (!in_array($u, $present, true)) {
                $up->execute([$now, $u]);
                $upd->execute([$now, $u]);
                $n++;
            }
        }
        return $n;
    }

    /** Deletes the registry rows of an account (after a purge of a removed account). */
    public function forgetAccount(string $user): void
    {
        foreach (['DELETE FROM domain_summary WHERE user = ?', 'DELETE FROM domains WHERE user = ?', 'DELETE FROM accounts WHERE user = ?'] as $sql) {
            $this->pdo->prepare($sql)->execute([$user]);
        }
    }

    // ------------------------------------------------------------------ summary

    /**
     * domain_summary rows (optionally restricted to some users).
     *
     * @param string[]|null $users
     * @return array<int,array<string,mixed>>
     */
    public function summaries(?array $users = null): array
    {
        $sql = 'SELECT * FROM domain_summary';
        $args = [];
        if ($users !== null) {
            if ($users === []) {
                return [];
            }
            $sql .= ' WHERE user IN (' . implode(',', array_fill(0, count($users), '?')) . ')';
            $args = array_values($users);
        }
        $sql .= ' ORDER BY user, domain';
        $st = $this->pdo->prepare($sql);
        $st->execute($args);
        $out = [];
        foreach ($st->fetchAll(\PDO::FETCH_ASSOC) as $r) {
            foreach ($r as $k => $v) {
                if ($k !== 'user' && $k !== 'domain' && $v !== null) {
                    $r[$k] = (int) $v;
                }
            }
            $out[] = $r;
        }
        return $out;
    }

    /**
     * Recomputes the domain_summary row from a (writable or read-only) store.
     *
     * @param string $timezone domain timezone used for "today"
     */
    public function refreshSummary(DomainStore $store, string $timezone, ?int $now = null): void
    {
        $now ??= $this->now();
        $clock = new DayClock($timezone);
        $today = $clock->day($now);
        $d7 = DayClock::addDays($today, -6);
        $d30 = DayClock::addDays($today, -29);
        $pdo = $store->pdo();
        $sum = static function (string $from) use ($pdo, $today): array {
            $st = $pdo->prepare('SELECT COALESCE(SUM(visits),0) v, COALESCE(SUM(pageviews),0) p, COALESCE(SUM(bytes),0) b, COALESCE(SUM(bot_hits),0) bh FROM daily WHERE day >= ? AND day <= ?');
            $st->execute([$from, $today]);
            return array_map('intval', (array) $st->fetch(\PDO::FETCH_ASSOC));
        };
        $t = $sum($today);
        $w = $sum($d7);
        $m = $sum($d30);
        $lastLine = $pdo->query('SELECT MAX(last_ts) FROM checkpoints')->fetchColumn();
        $meta = $store->meta();
        clearstatcache(true, $store->path());
        $row = [
            'user' => $store->user(),
            'domain' => $store->domain(),
            'db_bytes' => (int) @filesize($store->path()),
            'last_collect_at' => isset($meta['last_collect_at']) ? (int) $meta['last_collect_at'] : null,
            'last_line_ts' => $lastLine === false || $lastLine === null ? null : (int) $lastLine,
            'lines_total' => (int) ($meta['total_lines'] ?? 0),
            'visits_today' => $t['v'], 'pageviews_today' => $t['p'],
            'visits_7d' => $w['v'], 'pageviews_7d' => $w['p'], 'bytes_7d' => $w['b'],
            'visits_30d' => $m['v'], 'pageviews_30d' => $m['p'], 'bytes_30d' => $m['b'], 'bot_hits_30d' => $m['bh'],
        ];
        $cols = array_keys($row);
        $sql = 'INSERT INTO domain_summary (' . implode(',', $cols) . ') VALUES (' . implode(',', array_fill(0, count($cols), '?')) . ')
                ON CONFLICT(user, domain) DO UPDATE SET ' . implode(', ', array_map(static fn ($c) => "$c = excluded.$c", array_slice($cols, 2)));
        $this->pdo->prepare($sql)->execute(array_values($row));
    }

    /** Removes summary rows of a user (or one domain). */
    public function deleteSummary(string $user, ?string $domain = null): void
    {
        if ($domain === null) {
            $this->pdo->prepare('DELETE FROM domain_summary WHERE user = ?')->execute([$user]);
        } else {
            $this->pdo->prepare('DELETE FROM domain_summary WHERE user = ? AND domain = ?')->execute([$user, $domain]);
        }
    }

    // --------------------------------------------------------------------- jobs

    /**
     * Last jobs, newest first.
     *
     * @return array<int,array<string,mixed>>
     */
    public function jobs(int $limit = 50): array
    {
        $st = $this->pdo->prepare('SELECT * FROM jobs ORDER BY id DESC LIMIT ?');
        $st->bindValue(1, max(1, min(500, $limit)), \PDO::PARAM_INT);
        $st->execute();
        $out = [];
        foreach ($st->fetchAll(\PDO::FETCH_ASSOC) as $r) {
            foreach (['id', 'started_at', 'finished_at', 'lines', 'files', 'domains'] as $k) {
                $r[$k] = $r[$k] === null ? null : (int) $r[$k];
            }
            $out[] = $r;
        }
        return $out;
    }

    /** One job row or null. */
    public function job(int $id): ?array
    {
        $st = $this->pdo->prepare('SELECT * FROM jobs WHERE id = ?');
        $st->execute([$id]);
        $r = $st->fetch(\PDO::FETCH_ASSOC);
        return $r === false ? null : $r;
    }

    /**
     * Inserts a job in state "running" and returns its id (keeps the last 500).
     *
     * @param string      $type      collect | backfill | rollup | hook
     * @param string|null $scope     'all' or user[/domain]
     * @param int|null    $startedAt defaults to now
     */
    public function startJob(string $type, ?string $scope, ?int $startedAt = null): int
    {
        $st = $this->pdo->prepare("INSERT INTO jobs (type, scope, started_at, status) VALUES (?, ?, ?, 'running')");
        $st->execute([$type, $scope, $startedAt ?? $this->now()]);
        $id = (int) $this->pdo->lastInsertId();
        $this->pdo->exec('DELETE FROM jobs WHERE id <= (SELECT id FROM jobs ORDER BY id DESC LIMIT 1 OFFSET 500)');
        return $id;
    }

    /** Marks a job running again (used when the CLI adopts a job created by the UI). */
    public function touchJob(int $id): void
    {
        $st = $this->pdo->prepare("UPDATE jobs SET status = 'running', started_at = ? WHERE id = ?");
        $st->execute([$this->now(), $id]);
    }

    /**
     * Finishes a job.
     *
     * @param string $status ok | error | skipped
     * @param array{lines?:int,files?:int,domains?:int} $counters
     */
    public function finishJob(int $id, string $status, array $counters = [], ?string $message = null): void
    {
        $st = $this->pdo->prepare('UPDATE jobs SET finished_at = ?, status = ?, lines = ?, files = ?, domains = ?, message = ? WHERE id = ?');
        $st->execute([
            $this->now(), $status, (int) ($counters['lines'] ?? 0), (int) ($counters['files'] ?? 0),
            (int) ($counters['domains'] ?? 0), $message === null ? null : \WebPulse\Util\Str::truncate(\WebPulse\Util\Str::clean($message), 1000), $id,
        ]);
    }

    // -------------------------------------------------------------------- salts

    /**
     * Random 32-byte salt of a local day (created on first use).
     */
    public function salt(string $day): string
    {
        if (isset($this->saltCache[$day])) {
            return $this->saltCache[$day];
        }
        $st = $this->pdo->prepare('SELECT salt FROM salts WHERE day = ?');
        $st->execute([$day]);
        $s = $st->fetchColumn();
        if ($s === false) {
            $ins = $this->pdo->prepare('INSERT OR IGNORE INTO salts (day, salt) VALUES (?, ?)');
            $ins->bindValue(1, $day);
            $ins->bindValue(2, random_bytes(32), \PDO::PARAM_LOB);
            $ins->execute();
            $st->execute([$day]);
            $s = $st->fetchColumn();
        }
        if (is_resource($s)) {
            $s = stream_get_contents($s);
        }
        return $this->saltCache[$day] = (string) $s;
    }

    /** Deletes salts of days before $day; returns the number removed. */
    public function pruneSalts(string $beforeDay): int
    {
        $st = $this->pdo->prepare('DELETE FROM salts WHERE day < ?');
        $st->execute([$beforeDay]);
        $this->saltCache = [];
        return $st->rowCount();
    }

    // ----------------------------------------------------------------- settings

    /** @return array<string,mixed> dotted key => decoded value */
    public function settings(): array
    {
        $out = [];
        foreach ($this->pdo->query('SELECT key, value FROM settings ORDER BY key')->fetchAll(\PDO::FETCH_NUM) as [$k, $v]) {
            try {
                $out[(string) $k] = Json::decode((string) $v);
            } catch (\JsonException) {
                // ignore a corrupted value
            }
        }
        return $out;
    }

    /** Stores a raw setting override (validate with Config::validate first). */
    public function setSetting(string $k, mixed $v): void
    {
        $st = $this->pdo->prepare('INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value');
        $st->execute([$k, Json::encode($v)]);
        $this->config->refresh();
    }

    /** Removes a setting override. */
    public function deleteSetting(string $k): void
    {
        $this->pdo->prepare('DELETE FROM settings WHERE key = ?')->execute([$k]);
        $this->config->refresh();
    }

    /** meta table of the registry. */
    public function meta(): array
    {
        $out = [];
        foreach ($this->pdo->query('SELECT key, value FROM meta')->fetchAll(\PDO::FETCH_NUM) as [$k, $v]) {
            $out[(string) $k] = $v;
        }
        return $out;
    }
}
