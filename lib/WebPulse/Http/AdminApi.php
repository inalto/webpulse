<?php

declare(strict_types=1);

namespace WebPulse\Http;

use WebPulse\App;
use WebPulse\Config;

/**
 * `admin.*` actions (WHM): server overview, account/domain switches,
 * settings, jobs and purge. Root sees everything; a reseller is limited to
 * the accounts it owns and cannot change global settings, run rollups or
 * purge data.
 */
final class AdminApi
{
    /** Settings exposed to the WHM settings form (dotted keys). */
    public const SETTINGS = [
        'default_enabled', 'raw_retention_days', 'max_rows_per_dim', 'session_timeout_min', 'store_ip',
        'exclude_ips', 'exclude_paths', 'geoip.enabled', 'timezone',
    ];

    public const JOB_TYPES = ['collect', 'backfill', 'rollup'];
    /** Jobs started from WHM that may run at the same time. */
    public const MAX_RUNNING_JOBS = 3;
    /** A job still "running" after this many seconds is considered dead (crashed without finishing). */
    public const JOB_STALE_SECONDS = 6 * 3600;

    /**
     * @return array<string,mixed>
     */
    public static function handle(App $app, Request $r, Principal $p, string $action): array
    {
        return match ($action) {
            'overview' => self::overview($app, $p),
            'accounts' => self::accounts($app, $r, $p),
            'set_account' => self::setAccount($app, $r, $p),
            'set_domain' => self::setDomain($app, $r, $p),
            'settings' => self::settings($app, $p),
            'save_settings' => self::saveSettings($app, $r, $p),
            'run' => self::run($app, $r, $p),
            'jobs' => ['jobs' => self::jobs($app, $p, 50)],
            'purge' => self::purge($app, $r, $p),
            default => throw ApiError::badRequest('Unknown action "admin.' . substr($action, 0, 40) . '"'),
        };
    }

    /** Users in scope, or null for "all" (root). */
    private static function scope(App $app, Principal $p): ?array
    {
        if ($p->isAdmin()) {
            return null;
        }
        return array_keys(DomainResolver::visibleUsers($p, $app->registry(), $app->paths));
    }

    /**
     * @return array<string,mixed>
     */
    private static function overview(App $app, Principal $p): array
    {
        $reg = $app->registry();
        $users = self::scope($app, $p);
        $sums = $reg->summaries($users);
        $now = App::clock();
        $tot = ['today' => ['visits' => 0, 'pageviews' => 0], '7d' => ['visits' => 0, 'pageviews' => 0, 'bytes' => 0],
            '30d' => ['visits' => 0, 'pageviews' => 0, 'bytes' => 0, 'bot_hits' => 0]];
        $lastCollect = null;
        $lastLine = null;
        foreach ($sums as $s) {
            $tot['today']['visits'] += $s['visits_today'];
            $tot['today']['pageviews'] += $s['pageviews_today'];
            $tot['7d']['visits'] += $s['visits_7d'];
            $tot['7d']['pageviews'] += $s['pageviews_7d'];
            $tot['7d']['bytes'] += $s['bytes_7d'];
            $tot['30d']['visits'] += $s['visits_30d'];
            $tot['30d']['pageviews'] += $s['pageviews_30d'];
            $tot['30d']['bytes'] += $s['bytes_30d'];
            $tot['30d']['bot_hits'] += $s['bot_hits_30d'];
            $lastCollect = max($lastCollect ?? 0, (int) ($s['last_collect_at'] ?? 0)) ?: null;
            $lastLine = max($lastLine ?? 0, (int) ($s['last_line_ts'] ?? 0)) ?: null;
        }

        // Bot share needs human hits, which the summary does not carry: read
        // the daily table of each database (cheap single-range scan).
        $humanHits = 0;
        $botHits = 0;
        $partial = false;
        $refs = array_values(array_filter(
            DomainResolver::visibleDomains($p, $reg, $app->paths),
            static fn (DomainRef $d) => $d->hasData
        ));
        if (count($refs) <= 300) {
            $since = date('Y-m-d', $now - 29 * 86400);
            foreach ($refs as $ref) {
                try {
                    $st = $app->openDomain($ref->user, $ref->domain, true)->pdo()
                        ->prepare('SELECT COALESCE(SUM(hits),0), COALESCE(SUM(bot_hits),0) FROM daily WHERE day >= ?');
                    $st->execute([$since]);
                    [$h, $b] = $st->fetch(\PDO::FETCH_NUM);
                    $humanHits += (int) $h;
                    $botHits += (int) $b;
                } catch (\Throwable) {
                    $partial = true;
                }
            }
        } else {
            $partial = true;
        }

        $top = $sums;
        usort($top, static fn ($a, $b) => $b['visits_30d'] <=> $a['visits_30d']);
        $top = array_map(static fn ($s) => [
            'user' => $s['user'], 'domain' => $s['domain'], 'visits_30d' => $s['visits_30d'], 'pageviews_30d' => $s['pageviews_30d'],
            'bytes_30d' => $s['bytes_30d'], 'bot_hits_30d' => $s['bot_hits_30d'], 'visits_today' => $s['visits_today'],
            'db_bytes' => $s['db_bytes'],
            'last_collect_at' => $s['last_collect_at'] ? date('c', (int) $s['last_collect_at']) : null,
        ], array_slice($top, 0, 10));

        $accounts = $reg->accounts($p->isReseller() ? $p->name : null);
        if ($users !== null) {
            $accounts = array_values(array_filter($accounts, static fn ($a) => in_array($a['user'], $users, true)));
        }
        $enabled = count(array_filter($accounts, static fn ($a) => $a['enabled'] && $a['removed_at'] === null));

        $geoDb = (string) $app->config->get('geoip.country_db', '');
        $hooks = (string) $app->config->get('paths.hooks_file', '/var/cpanel/hooks.yaml');

        return [
            'totals' => $tot,
            'bot_share_30d' => ($humanHits + $botHits) > 0 ? round($botHits / ($humanHits + $botHits), 4) : null,
            'bot_share_partial' => $partial,
            'top_domains' => $top,
            'accounts' => ['total' => count($accounts), 'enabled' => $enabled, 'disabled' => count($accounts) - $enabled],
            'domains' => ['total' => count($sums), 'with_data' => count($refs)],
            'disk' => self::disk($app, $users),
            'jobs' => self::jobs($app, $p, 8),
            'collector' => [
                'last_collect_at' => $lastCollect ? date('c', $lastCollect) : null,
                'lag_seconds' => $lastLine ? max(0, $now - $lastLine) : null,
                'cron_installed' => is_file($app->paths->cronFile()),
                'hook_installed' => is_file($hooks) && str_contains((string) @file_get_contents($hooks, false, null, 0, 4 << 20), 'webpulse'),
                'geoip' => [
                    'enabled' => (bool) $app->config->get('geoip.enabled', false),
                    'db_date' => $geoDb !== '' && is_file($geoDb) ? date('Y-m-d', (int) filemtime($geoDb)) : null,
                ],
            ],
            'version' => $app->version(),
            'can_edit_settings' => $p->isAdmin(),
        ];
    }

    /**
     * Bytes used by domain databases (users in scope, or all) and the registry.
     *
     * @param string[]|null $users
     * @return array{data_bytes:int,registry_bytes:int|null,databases:int}
     */
    private static function disk(App $app, ?array $users): array
    {
        $bytes = 0;
        $n = 0;
        $dirs = $users === null
            ? (glob($app->paths->dataDir() . '/*', GLOB_ONLYDIR) ?: [])
            : array_map(static fn ($u) => $app->paths->userDir($u), $users);
        foreach ($dirs as $dir) {
            foreach (glob($dir . '/*.db') ?: [] as $f) {
                if (!is_link($f)) {
                    $bytes += (int) @filesize($f);
                    $n++;
                }
            }
        }
        $reg = $app->paths->registryDb();
        return ['data_bytes' => $bytes, 'registry_bytes' => $users === null && is_file($reg) ? (int) filesize($reg) : null, 'databases' => $n];
    }

    /**
     * @return array<string,mixed>
     */
    private static function accounts(App $app, Request $r, Principal $p): array
    {
        $reg = $app->registry();
        $q = mb_substr(trim((string) $r->get('q', '')), 0, 100);
        $limit = max(1, min(500, (int) ($r->get('limit') ?? 50)));
        $offset = max(0, (int) ($r->get('offset') ?? 0));
        $users = self::scope($app, $p);

        $accounts = $reg->accounts($p->isReseller() ? $p->name : null, $q === '' ? null : $q);
        if ($users !== null) {
            $accounts = array_values(array_filter($accounts, static fn ($a) => in_array($a['user'], $users, true)));
        }
        $total = count($accounts);
        $page = array_slice($accounts, $offset, $limit);
        $names = array_column($page, 'user');

        $domains = [];
        foreach ($reg->domains() as $d) {
            if (in_array($d['user'], $names, true) && $d['removed_at'] === null) {
                $domains[$d['user']][$d['domain']] = $d;
            }
        }
        $sums = [];
        foreach ($reg->summaries($names) as $s) {
            $sums[$s['user'] . '/' . $s['domain']] = $s;
        }

        $out = [];
        foreach ($page as $a) {
            $u = $a['user'];
            $doms = $domains[$u] ?? [];
            // databases without a registry row (e.g. removed domains with data)
            foreach (glob($app->paths->userDir($u) . '/*.db') ?: [] as $f) {
                $dn = substr(basename($f), 0, -3);
                if (!isset($doms[$dn]) && DomainResolver::validDomain($dn)) {
                    $doms[$dn] = ['domain' => $dn, 'kind' => 'unknown', 'enabled' => $a['enabled'], 'enabled_override' => null];
                }
            }
            $list = [];
            foreach ($doms as $dn => $d) {
                $file = $app->paths->domainDb($u, (string) $dn);
                $s = $sums[$u . '/' . $dn] ?? null;
                $list[] = [
                    'domain' => (string) $dn,
                    'kind' => (string) ($d['kind'] ?? 'unknown'),
                    'enabled' => (bool) $d['enabled'],
                    'enabled_override' => $d['enabled_override'],
                    'has_data' => is_file($file),
                    'db_bytes' => is_file($file) ? (int) filesize($file) : 0,
                    'visits_30d' => $s === null ? 0 : $s['visits_30d'],
                    'last_collect_at' => $s !== null && $s['last_collect_at'] ? date('c', (int) $s['last_collect_at']) : null,
                ];
            }
            $rank = ['main' => 0, 'addon' => 1, 'sub' => 2, 'parked' => 3];
            usort($list, static fn ($x, $y) => [$rank[$x['kind']] ?? 4, $x['domain']] <=> [$rank[$y['kind']] ?? 4, $y['domain']]);
            $out[] = [
                'user' => $u,
                'owner' => $a['owner'],
                'enabled' => $a['enabled'],
                'enabled_override' => $a['enabled_override'],
                'timezone' => $a['timezone'],
                'removed' => $a['removed_at'] !== null,
                'db_bytes' => array_sum(array_column($list, 'db_bytes')),
                'visits_30d' => array_sum(array_column($list, 'visits_30d')),
                'domains' => $list,
            ];
        }
        return [
            'accounts' => $out,
            'total' => $total,
            'has_more' => $offset + $limit < $total,
            'default_enabled' => (bool) $app->config->get('default_enabled', true),
        ];
    }

    /** Validates `user` from a body and checks the principal's scope (404 otherwise). */
    private static function scopedUser(Principal $p, mixed $user): string
    {
        if (!is_string($user) || !DomainResolver::validUser($user) || !$p->canSeeUser($user)) {
            throw ApiError::notFound('Unknown account');
        }
        return $user;
    }

    /**
     * Validates `domain` from a body and checks that it belongs to $user
     * (404 otherwise): a live registry row of the account, a domain cPanel
     * lists for it (/var/cpanel/users, /etc/userdomains) or an existing
     * database in its data directory. A syntactically valid name alone is not
     * enough — it would let a reseller register (and get collected) a domain
     * of another tenant whose logs live in a shared domlog directory.
     */
    private static function ownedDomain(App $app, string $user, mixed $domain): string
    {
        if (!is_string($domain) || !DomainResolver::validDomain(strtolower($domain))) {
            throw ApiError::badRequest('Invalid domain');
        }
        $domain = strtolower($domain);
        if (!self::belongsTo($app, $user, $domain)) {
            throw ApiError::notFound('Unknown domain');
        }
        return $domain;
    }

    /** True when $domain is a domain of $user (see ownedDomain()). */
    public static function belongsTo(App $app, string $user, string $domain): bool
    {
        try {
            foreach ($app->registry()->domains($user) as $d) {
                if ($d['domain'] === $domain && $d['removed_at'] === null) {
                    return true;
                }
            }
        } catch (\Throwable) {
            // registry unreadable: fall back to cPanel data and databases
        }
        if (isset($app->cpanelUsers()->domainsOf($user)[$domain])) {
            return true;
        }
        $db = $app->paths->domainDb($user, $domain);
        return is_file($db) && !is_link($db);
    }

    /** `true|false|null` (key required). */
    private static function tri(array $body): ?bool
    {
        if (!array_key_exists('enabled', $body)) {
            throw ApiError::badRequest('"enabled" is required (true, false or null)');
        }
        $v = $body['enabled'];
        if ($v !== null && !is_bool($v)) {
            throw ApiError::badRequest('"enabled" must be true, false or null');
        }
        return $v;
    }

    private static function setAccount(App $app, Request $r, Principal $p): array
    {
        $b = $r->json();
        $user = self::scopedUser($p, $b['user'] ?? null);
        $enabled = self::tri($b);
        $tz = $b['timezone'] ?? null;
        if ($tz !== null && (!is_string($tz) || ($tz !== '' && !in_array($tz, \DateTimeZone::listIdentifiers(), true)))) {
            throw ApiError::badRequest('Invalid timezone');
        }
        $app->registry()->setAccount($user, $enabled, $tz);
        return ['account' => $app->registry()->account($user)];
    }

    private static function setDomain(App $app, Request $r, Principal $p): array
    {
        $b = $r->json();
        $user = self::scopedUser($p, $b['user'] ?? null);
        $domain = self::ownedDomain($app, $user, $b['domain'] ?? null);
        $enabled = self::tri($b);
        $app->registry()->setDomain($user, $domain, $enabled);
        foreach ($app->registry()->domains($user) as $d) {
            if ($d['domain'] === $domain) {
                return ['domain' => $d];
            }
        }
        return ['domain' => null];
    }

    /**
     * @return array<string,mixed>
     */
    private static function settings(App $app, Principal $p): array
    {
        if (!$p->isAdmin()) {
            throw ApiError::forbidden('Only the server administrator can see global settings');
        }
        $values = [];
        foreach (self::SETTINGS as $k) {
            $values[$k] = $app->config->get($k);
        }
        return [
            'settings' => $values,
            'overridden' => array_values(array_intersect(self::SETTINGS, array_keys($app->config->overrides()))),
            'editable' => $p->isAdmin(),
            'server_timezone' => date_default_timezone_get(),
        ];
    }

    private static function saveSettings(App $app, Request $r, Principal $p): array
    {
        if (!$p->isAdmin()) {
            throw ApiError::forbidden('Only the server administrator can change global settings');
        }
        $b = $r->json();
        $in = isset($b['settings']) && is_array($b['settings']) ? $b['settings'] : $b;
        if (isset($in['geoip']) && is_array($in['geoip']) && array_key_exists('enabled', $in['geoip'])) {
            $in['geoip.enabled'] = $in['geoip']['enabled'];
        }
        unset($in['geoip']);
        $clean = [];
        foreach ($in as $k => $v) {
            if (!in_array($k, self::SETTINGS, true)) {
                throw ApiError::badRequest('Setting "' . substr((string) $k, 0, 40) . '" cannot be changed here');
            }
            $clean[$k] = Config::validate($k, $v); // throws InvalidArgumentException → 400
        }
        foreach ($clean as $k => $v) {
            $app->config->setOverride($k, $v);
        }
        return self::settings($app, $p);
    }

    /**
     * Spawns `bin/webpulse <type> … --job` detached and returns the job id.
     * Refused (409) when the same job (type and scope) is already running or
     * when MAX_RUNNING_JOBS jobs are running.
     *
     * @return array{job_id:int}
     */
    private static function run(App $app, Request $r, Principal $p): array
    {
        $b = $r->json();
        $type = $b['type'] ?? null;
        if (!is_string($type) || !in_array($type, self::JOB_TYPES, true)) {
            throw ApiError::badRequest('"type" must be one of ' . implode(', ', self::JOB_TYPES));
        }
        $user = $b['user'] ?? null;
        $domain = $b['domain'] ?? null;
        if ($user !== null && $user !== '') {
            $user = self::scopedUser($p, $user);
        } else {
            $user = null;
        }
        if ($domain !== null && $domain !== '') {
            if (!is_string($domain) || !DomainResolver::validDomain(strtolower($domain)) || $user === null) {
                throw ApiError::badRequest('"domain" requires a valid "user"');
            }
            $domain = self::ownedDomain($app, $user, $domain);
        } else {
            $domain = null;
        }
        if ($type === 'rollup' && !$p->isAdmin()) {
            throw ApiError::forbidden('Only the server administrator can run the rollup');
        }
        if ($type === 'backfill' && $user === null) {
            throw ApiError::badRequest('Backfill needs an account ("user")');
        }
        if (!$p->isAdmin() && $user === null) {
            throw ApiError::badRequest('Pick one of your accounts');
        }

        $scope = $user === null ? 'all' : ($domain === null ? $user : "$user/$domain");
        try {
            $jobId = $app->registry()->startJobLimited($type, $scope, App::clock() - self::JOB_STALE_SECONDS, self::MAX_RUNNING_JOBS);
        } catch (\OverflowException $e) {
            throw new ApiError(409, 'conflict', $e->getMessage());
        }
        $args = [$type, '--job', (string) $jobId, '--quiet'];
        if ($user !== null) {
            array_push($args, '--user', $user);
        }
        if ($domain !== null) {
            array_push($args, '--domain', $domain);
        }
        $cfg = $app->configPath();
        if ($cfg !== null && is_file($cfg)) {
            array_push($args, '--config', $cfg);
        }
        if (JobRunner::spawn($app, $args) === null) {
            $app->registry()->finishJob($jobId, 'error', [], 'could not start the job process');
            throw new ApiError(500, 'spawn_failed', 'Could not start the job');
        }
        return ['job_id' => $jobId];
    }

    /**
     * Last jobs (resellers only see jobs of their accounts and global ones).
     *
     * @return list<array<string,mixed>>
     */
    private static function jobs(App $app, Principal $p, int $limit): array
    {
        $out = [];
        foreach ($app->registry()->jobs($p->isAdmin() ? $limit : 200) as $j) {
            $scope = (string) ($j['scope'] ?? '');
            if (!$p->isAdmin() && $scope !== 'all' && $scope !== '' && !$p->canSeeUser(explode('/', $scope)[0])) {
                continue;
            }
            $j['started_at_iso'] = $j['started_at'] ? date('c', (int) $j['started_at']) : null;
            $j['finished_at_iso'] = $j['finished_at'] ? date('c', (int) $j['finished_at']) : null;
            $out[] = $j;
            if (count($out) >= $limit) {
                break;
            }
        }
        return $out;
    }

    /**
     * Deletes the database(s) of an account or one domain (root only).
     *
     * @return array{deleted:list<string>}
     */
    private static function purge(App $app, Request $r, Principal $p): array
    {
        if (!$p->isAdmin()) {
            throw ApiError::forbidden('Only the server administrator can purge data');
        }
        $b = $r->json();
        if (($b['confirm'] ?? null) !== 'DELETE') {
            throw ApiError::badRequest('Type DELETE to confirm');
        }
        $user = self::scopedUser($p, $b['user'] ?? null);
        $domain = $b['domain'] ?? null;
        $dataDir = realpath($app->paths->dataDir());
        if ($domain !== null && $domain !== '') {
            $domain = self::ownedDomain($app, $user, $domain);
            $files = [$app->paths->domainDb($user, $domain)];
        } else {
            $domain = null;
            $files = glob($app->paths->userDir($user) . '/*.db') ?: [];
        }
        $deleted = [];
        foreach ($files as $f) {
            if (!is_file($f) || is_link($f)) {
                continue;
            }
            $real = realpath($f);
            if ($dataDir === false || $real === false || !str_starts_with($real, $dataDir . '/')) {
                continue;
            }
            if (@unlink($real)) {
                @unlink($real . '-journal');
                $deleted[] = substr(basename($real), 0, -3);
            }
        }
        try {
            $app->registry()->deleteSummary($user, $domain);
        } catch (\Throwable) {
            // summary rows are refreshed by the next rollup anyway
        }
        return ['deleted' => $deleted];
    }
}
