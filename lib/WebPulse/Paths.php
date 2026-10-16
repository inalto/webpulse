<?php

declare(strict_types=1);

namespace WebPulse;

/**
 * Every filesystem location used by WebPulse, derived from the `paths`
 * section of the configuration. User and domain names are validated before
 * they are turned into paths (no traversal possible).
 */
final class Paths
{
    public const USER_RE = '/^[a-z0-9_][a-z0-9_-]{0,31}$/';
    public const DOMAIN_RE = '/^(?=.{1,253}$)[a-z0-9_](?:[a-z0-9_-]{0,62})(?:\.[a-z0-9_](?:[a-z0-9_-]{0,62}))*$/';

    /** Paths derived from $config; $root is the install directory (repo root). */
    public function __construct(private Config $config, private string $root)
    {
    }

    /** True for a syntactically valid cPanel user name. */
    public static function isValidUser(string $user): bool
    {
        return preg_match(self::USER_RE, $user) === 1 && !str_contains($user, '..');
    }

    /** True for a syntactically valid (lowercase) domain name. */
    public static function isValidDomain(string $domain): bool
    {
        return preg_match(self::DOMAIN_RE, $domain) === 1;
    }

    /** @throws \InvalidArgumentException */
    public static function assertUser(string $user): string
    {
        if (!self::isValidUser($user)) {
            throw new \InvalidArgumentException('invalid user name: ' . json_encode($user));
        }
        return $user;
    }

    /** @throws \InvalidArgumentException */
    public static function assertDomain(string $domain): string
    {
        if (!self::isValidDomain($domain)) {
            throw new \InvalidArgumentException('invalid domain name: ' . json_encode($domain));
        }
        return $domain;
    }

    /** Install dir (repo root): contains bin/, lib/, VERSION. */
    public function root(): string
    {
        return $this->root;
    }

    /** Directory holding one sub-directory per account (paths.data_dir). */
    public function dataDir(): string
    {
        return rtrim((string) $this->config->get('paths.data_dir'), '/');
    }

    /** /var/webpulse/data/<user> */
    public function userDir(string $user): string
    {
        return $this->dataDir() . '/' . self::assertUser($user);
    }

    /** /var/webpulse/data/<user>/<domain>.db */
    public function domainDb(string $user, string $domain): string
    {
        return $this->userDir($user) . '/' . self::assertDomain($domain) . '.db';
    }

    /** /var/webpulse/data/<user>/user (owned by the account) */
    public function userPrivateDir(string $user): string
    {
        return $this->userDir($user) . '/user';
    }

    /** /var/webpulse/data/<user>/user/settings.json */
    public function userSettingsFile(string $user): string
    {
        return $this->userPrivateDir($user) . '/settings.json';
    }

    /** Registry database file (paths.registry_db). */
    public function registryDb(): string
    {
        return (string) $this->config->get('paths.registry_db');
    }

    /** Directory of webpulse.log (paths.log_dir). */
    public function logDir(): string
    {
        return rtrim((string) $this->config->get('paths.log_dir'), '/');
    }

    /** Directory of lock files and hook flags (paths.run_dir). */
    public function runDir(): string
    {
        return rtrim((string) $this->config->get('paths.run_dir'), '/');
    }

    /** <run_dir>/hook — flag files written by the Stats::RunUser hook (1777). */
    public function hookDir(): string
    {
        return $this->runDir() . '/hook';
    }

    /** Directory holding GeoIP databases (dirname of geoip.country_db). */
    public function geoipDir(): string
    {
        return dirname((string) $this->config->get('geoip.country_db'));
    }

    /** @return string[] domlog directories with {user} substituted */
    public function domlogsDirs(string $user): array
    {
        $out = [];
        foreach ((array) $this->config->get('paths.domlogs_dirs', []) as $d) {
            if (is_string($d) && $d !== '') {
                $out[] = rtrim(str_replace('{user}', self::assertUser($user), $d), '/');
            }
        }
        return array_values(array_unique($out));
    }

    /** Archive directory of a user (default /home/<user>/logs). */
    public function archiveDir(string $user): string
    {
        return rtrim(str_replace('{user}', self::assertUser($user), (string) $this->config->get('paths.archive_dir')), '/');
    }

    /** /etc/userdomains (paths.userdomains_file). */
    public function userdomainsFile(): string
    {
        return (string) $this->config->get('paths.userdomains_file');
    }

    /** /var/cpanel/users (paths.cpanel_users_dir). */
    public function cpanelUsersDir(): string
    {
        return rtrim((string) $this->config->get('paths.cpanel_users_dir'), '/');
    }

    /** /var/cpanel/userdata (paths.cpanel_userdata_dir). */
    public function cpanelUserdataDir(): string
    {
        return rtrim((string) $this->config->get('paths.cpanel_userdata_dir'), '/');
    }

    /** Parent of the account home directories (paths.home_root). */
    public function homeRoot(): string
    {
        return rtrim((string) $this->config->get('paths.home_root'), '/');
    }

    /** Cron file whose presence `status` reports (paths.cron_file). */
    public function cronFile(): string
    {
        return (string) $this->config->get('paths.cron_file');
    }
}
