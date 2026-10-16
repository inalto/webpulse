<?php

declare(strict_types=1);

namespace WebPulse\Discovery;

use WebPulse\Config;
use WebPulse\Paths;

/**
 * Reads cPanel account data without any cPanel API:
 *
 * - /var/cpanel/users/<user>: `DNS=` (main domain), `DNS<n>=` (other
 *   domains), `OWNER=` (reseller or root);
 * - /etc/userdomains: `domain: user` lines (adds subdomains);
 * - /var/cpanel/userdata/<user>/main (optional): exact kinds of the
 *   addon / parked / sub domains.
 *
 * Without the userdata file kinds are inferred: `DNS=` is main, a domain
 * ending in ".<main>" (or ".<another domain of the user>") is sub, other
 * `DNS<n>=` entries are addon, anything else is unknown.
 */
final class CpanelUsers
{
    /** @var array<string,array<string,mixed>>|null */
    private ?array $all = null;
    /** @var array<string,string[]>|null user => domains (userdomains file) */
    private ?array $userdomains = null;

    public function __construct(private Config $config, private Paths $paths)
    {
    }

    /**
     * Every account.
     *
     * @return array<string,array{owner:?string,domains:string[],kinds:array<string,string>,main:?string,home:string}>
     */
    public function all(): array
    {
        if ($this->all !== null) {
            return $this->all;
        }
        $out = [];
        $dir = $this->paths->cpanelUsersDir();
        foreach (@scandir($dir) ?: [] as $name) {
            if ($name[0] === '.' || $name === 'system' || $name === 'root' || !Paths::isValidUser($name)) {
                continue;
            }
            $file = $dir . '/' . $name;
            if (is_link($file) || !is_file($file)) {
                continue;
            }
            $info = $this->read($name);
            if ($info !== null) {
                $out[$name] = $info;
            }
        }
        ksort($out);
        return $this->all = $out;
    }

    /**
     * One account (same shape as an all() entry), or null when it does not
     * exist. Cheaper than all() on servers with many accounts.
     *
     * @return array{owner:?string,domains:string[],kinds:array<string,string>,main:?string,home:string}|null
     */
    public function get(string $user): ?array
    {
        if (isset($this->all[$user])) {
            return $this->all[$user];
        }
        if ($this->all !== null || !$this->exists($user)) {
            return null;
        }
        $file = $this->paths->cpanelUsersDir() . '/' . $user;
        if (is_link($file) || !is_file($file)) {
            return null;
        }
        return $this->read($user);
    }

    /** True when /var/cpanel/users/<user> exists. */
    public function exists(string $user): bool
    {
        return Paths::isValidUser($user) && is_file($this->paths->cpanelUsersDir() . '/' . $user);
    }

    /** Reseller owning the account (OWNER=), or null when unknown. */
    public function owner(string $user): ?string
    {
        if (!$this->exists($user)) {
            return null;
        }
        $kv = $this->parseUserFile($user);
        $o = $kv['OWNER'] ?? null;
        return is_string($o) && $o !== '' ? $o : null;
    }

    /**
     * Domains of an account with their kind.
     *
     * @return array<string,string> domain => main|addon|parked|sub|unknown
     */
    public function domainsOf(string $user): array
    {
        $info = $this->all[$user] ?? ($this->exists($user) ? $this->read($user) : null);
        return $info === null ? [] : $info['kinds'];
    }

    /** Home directory of the account. */
    public function home(string $user): string
    {
        if (function_exists('posix_getpwnam') && ($pw = @posix_getpwnam($user)) && is_array($pw) && !empty($pw['dir'])
            && $this->paths->homeRoot() === '/home') {
            return (string) $pw['dir'];
        }
        return $this->paths->homeRoot() . '/' . $user;
    }

    /** @return array{owner:?string,domains:string[],kinds:array<string,string>,main:?string,home:string}|null */
    private function read(string $user): ?array
    {
        $kv = $this->parseUserFile($user);
        $main = isset($kv['DNS']) && Paths::isValidDomain(strtolower($kv['DNS'])) ? strtolower($kv['DNS']) : null;
        $dns = [];
        foreach ($kv as $k => $v) {
            if (preg_match('/^DNS\d+$/', $k) && Paths::isValidDomain(strtolower($v))) {
                $dns[] = strtolower($v);
            }
        }
        $extra = $this->userdomains()[$user] ?? [];
        $all = array_values(array_unique(array_merge($main !== null ? [$main] : [], $dns, $extra)));
        $kinds = $this->kinds($user, $main, $dns, $all);
        ksort($kinds);
        return [
            'owner' => isset($kv['OWNER']) && $kv['OWNER'] !== '' ? $kv['OWNER'] : null,
            'domains' => array_keys($kinds),
            'kinds' => $kinds,
            'main' => $main,
            'home' => $this->home($user),
        ];
    }

    /**
     * @param string[] $dns
     * @param string[] $all
     * @return array<string,string>
     */
    private function kinds(string $user, ?string $main, array $dns, array $all): array
    {
        $ud = $this->userdata($user);
        $kinds = [];
        foreach ($all as $d) {
            if ($ud !== null && isset($ud[$d])) {
                $kinds[$d] = $ud[$d];
                continue;
            }
            if ($d === $main) {
                $kinds[$d] = 'main';
                continue;
            }
            $sub = false;
            foreach ($all as $other) {
                if ($other !== $d && str_ends_with($d, '.' . $other)) {
                    $sub = true;
                    break;
                }
            }
            if ($sub) {
                $kinds[$d] = 'sub';
            } elseif (in_array($d, $dns, true)) {
                $kinds[$d] = 'addon';
            } else {
                $kinds[$d] = 'unknown';
            }
        }
        return $kinds;
    }

    /** @return array<string,string> KEY => value of /var/cpanel/users/<user> */
    private function parseUserFile(string $user): array
    {
        $file = $this->paths->cpanelUsersDir() . '/' . Paths::assertUser($user);
        if (is_link($file)) {
            return [];
        }
        $raw = @file_get_contents($file, false, null, 0, 1024 * 1024);
        if ($raw === false) {
            return [];
        }
        $kv = [];
        foreach (preg_split('/\r?\n/', $raw) ?: [] as $line) {
            if ($line === '' || $line[0] === '#' || !str_contains($line, '=')) {
                continue;
            }
            [$k, $v] = explode('=', $line, 2);
            $kv[trim($k)] = trim($v);
        }
        return $kv;
    }

    /** @return array<string,string[]> user => domains from /etc/userdomains */
    private function userdomains(): array
    {
        if ($this->userdomains !== null) {
            return $this->userdomains;
        }
        $this->userdomains = [];
        $raw = @file_get_contents($this->paths->userdomainsFile());
        if ($raw === false) {
            return $this->userdomains;
        }
        foreach (preg_split('/\r?\n/', $raw) ?: [] as $line) {
            if (!str_contains($line, ':')) {
                continue;
            }
            [$d, $u] = array_map('trim', explode(':', $line, 2));
            $d = strtolower($d);
            if ($d === '*' || $u === 'nobody' || !Paths::isValidUser($u) || !Paths::isValidDomain($d)) {
                continue;
            }
            $this->userdomains[$u][] = $d;
        }
        return $this->userdomains;
    }

    /**
     * Kinds from /var/cpanel/userdata/<user>/main (YAML subset), null when absent.
     *
     * @return array<string,string>|null
     */
    private function userdata(string $user): ?array
    {
        $file = $this->paths->cpanelUserdataDir() . '/' . $user . '/main';
        if (!is_file($file) || is_link($file)) {
            return null;
        }
        $raw = @file_get_contents($file, false, null, 0, 4 * 1024 * 1024);
        if ($raw === false) {
            return null;
        }
        $out = [];
        $section = null;
        foreach (preg_split('/\r?\n/', $raw) ?: [] as $line) {
            if (preg_match('/^([a-z_]+):\s*(.*)$/', $line, $m)) {
                $section = $m[1];
                if ($section === 'main_domain' && Paths::isValidDomain(strtolower(trim($m[2], " '\"")))) {
                    $out[strtolower(trim($m[2], " '\""))] = 'main';
                }
                continue;
            }
            if ($section === null) {
                continue;
            }
            $item = null;
            if (preg_match('/^\s+-\s+[\'"]?([^\'"\s]+)/', $line, $m)) {
                $item = $m[1];
            } elseif (preg_match('/^\s+[\'"]?([^\'":\s]+)[\'"]?\s*:/', $line, $m)) {
                $item = $m[1];
            }
            if ($item === null || !Paths::isValidDomain(strtolower($item))) {
                continue;
            }
            $kind = match ($section) {
                'addon_domains' => 'addon',
                'parked_domains' => 'parked',
                'sub_domains' => 'sub',
                default => null,
            };
            if ($kind !== null && !isset($out[strtolower($item)])) {
                $out[strtolower($item)] = $kind;
            }
        }
        return $out;
    }
}
