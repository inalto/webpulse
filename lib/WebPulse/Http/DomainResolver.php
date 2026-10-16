<?php

declare(strict_types=1);

namespace WebPulse\Http;

use WebPulse\Paths;

/**
 * The only place that decides which domains a principal may see.
 *
 * * user     → `<data_dir>/<user>/*.db` (plus registry rows without data,
 *              when the registry is readable — it is not for cPanel users);
 * * reseller → the accounts it owns (Principal::$ownedUsers, else
 *              registry accounts with `owner = name`) and its own account;
 * * admin    → every account directory and every registry row.
 *
 * Domain and user names that do not match a strict pattern are rejected
 * before any filesystem access.
 */
final class DomainResolver
{
    public const DOMAIN_RE = '/^[a-z0-9.-]{1,253}\z/';
    /** Same rule as the engine (Paths::USER_RE): one definition of a valid account name. */
    public const USER_RE = Paths::USER_RE;

    public static function validDomain(string $d): bool
    {
        return preg_match(self::DOMAIN_RE, $d) === 1 && !str_contains($d, '..') && $d[0] !== '.' && $d[0] !== '-';
    }

    public static function validUser(string $u): bool
    {
        return Paths::isValidUser($u);
    }

    /**
     * @param object|null $registry WebPulse\Registry\Registry (null when not readable)
     * @return list<DomainRef>
     */
    public static function visibleDomains(Principal $p, ?object $registry, Paths $paths): array
    {
        $users = self::visibleUsers($p, $registry, $paths);
        $regRows = [];
        if ($registry !== null) {
            try {
                foreach ($registry->domains($p->isUser() ? $p->name : null) as $row) {
                    $row = (array) $row;
                    $u = (string) ($row['user'] ?? '');
                    if (isset($users[$u]) && empty($row['removed_at'])) {
                        $regRows[$u][(string) ($row['domain'] ?? '')] = $row;
                    }
                }
            } catch (\Throwable) {
                $regRows = [];
            }
        }

        $out = [];
        foreach (array_keys($users) as $user) {
            $seen = [];
            $dir = $paths->userDir($user);
            $files = is_dir($dir) && !is_link($dir) ? (glob($dir . '/*.db') ?: []) : [];
            foreach ($files as $file) {
                $domain = substr(basename($file), 0, -3);
                if (!self::validDomain($domain) || is_link($file) || !is_file($file)) {
                    continue;
                }
                $row = $regRows[$user][$domain] ?? [];
                $out[] = new DomainRef($user, $domain, (string) ($row['kind'] ?? 'unknown'), self::bool($row['enabled'] ?? null), true, $file);
                $seen[$domain] = true;
            }
            foreach ($regRows[$user] ?? [] as $domain => $row) {
                $domain = (string) $domain;
                if (isset($seen[$domain]) || !self::validDomain($domain)) {
                    continue;
                }
                $out[] = new DomainRef($user, $domain, (string) ($row['kind'] ?? 'unknown'), self::bool($row['enabled'] ?? null), false, $paths->domainDb($user, $domain));
            }
        }
        $rank = ['main' => 0, 'addon' => 1, 'sub' => 2, 'parked' => 3];
        usort($out, static fn (DomainRef $a, DomainRef $b) => [$a->user, $rank[$a->kind] ?? 4, $a->domain] <=> [$b->user, $rank[$b->kind] ?? 4, $b->domain]);
        return $out;
    }

    /**
     * Account names the principal may see (as keys).
     *
     * @return array<string,true>
     */
    public static function visibleUsers(Principal $p, ?object $registry, Paths $paths): array
    {
        $users = [];
        if ($p->isUser()) {
            return self::validUser($p->name) ? [$p->name => true] : [];
        }
        if ($p->isReseller()) {
            $owned = $p->ownedUsers;
            if ($owned === null && $registry !== null) {
                try {
                    $owned = array_map(static fn ($r) => (string) ((array) $r)['user'], $registry->accounts($p->name));
                } catch (\Throwable) {
                    $owned = [];
                }
            }
            foreach (array_merge($owned ?? [], [$p->name]) as $u) {
                if (self::validUser((string) $u)) {
                    $users[(string) $u] = true;
                }
            }
            return $users;
        }
        foreach (glob($paths->dataDir() . '/*', GLOB_ONLYDIR) ?: [] as $dir) {
            $u = basename($dir);
            if (self::validUser($u) && !is_link($dir)) {
                $users[$u] = true;
            }
        }
        if ($registry !== null) {
            try {
                foreach ($registry->accounts(null) as $r) {
                    $u = (string) (((array) $r)['user'] ?? '');
                    if (self::validUser($u)) {
                        $users[$u] = true;
                    }
                }
            } catch (\Throwable) {
                // registry unavailable: data directories only
            }
        }
        ksort($users);
        return $users;
    }

    /**
     * Resolves a `domain` (+ optional `user`) request parameter.
     *
     * @throws ApiError 400 for malformed names, 404 when not visible
     */
    public static function resolve(Principal $p, ?object $registry, Paths $paths, ?string $domain, ?string $user = null): DomainRef
    {
        if ($domain === null || $domain === '') {
            throw ApiError::badRequest('Parameter "domain" is required');
        }
        $domain = strtolower($domain);
        if (!self::validDomain($domain) || str_contains($domain, '/')) {
            throw ApiError::notFound('Unknown domain');
        }
        if ($user !== null && $user !== '' && (!self::validUser($user) || !$p->canSeeUser($user))) {
            throw ApiError::notFound('Unknown domain');
        }
        // Fast path for cPanel users: no registry needed.
        if ($p->isUser()) {
            $file = $paths->domainDb($p->name, $domain);
            if (is_file($file) && !is_link($file)) {
                return new DomainRef($p->name, $domain, 'unknown', null, true, $file);
            }
        }
        $match = null;
        foreach (self::visibleDomains($p, $registry, $paths) as $ref) {
            if ($ref->domain !== $domain || ($user !== null && $user !== '' && $ref->user !== $user)) {
                continue;
            }
            if ($match === null || ($ref->hasData && !$match->hasData)) {
                $match = $ref;
            }
        }
        if ($match === null) {
            throw ApiError::notFound('Unknown domain');
        }
        return $match;
    }

    private static function bool(mixed $v): ?bool
    {
        return $v === null || $v === '' ? null : (bool) $v;
    }
}
