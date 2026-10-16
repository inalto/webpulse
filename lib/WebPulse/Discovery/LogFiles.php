<?php

declare(strict_types=1);

namespace WebPulse\Discovery;

use WebPulse\Config;
use WebPulse\Paths;
use WebPulse\Util\Fs;

/**
 * Finds the access logs of a domain in the configured domlog directories and
 * the compressed archives in the account's home.
 */
final class LogFiles
{
    /** File-name patterns that are never access logs (fnmatch syntax). */
    public const IGNORE = ['*-bytes_log', '*-ftp_log', 'ftpxferlog*', '*.bkup*', '*.offset*', '*.gz', '*.bz2', '*.zip', '*~', '.*'];

    public function __construct(private Config $config, private Paths $paths)
    {
    }

    /** True when a file name must be skipped. */
    public static function isIgnored(string $name): bool
    {
        foreach (self::IGNORE as $p) {
            if (fnmatch($p, $name)) {
                return true;
            }
        }
        return false;
    }

    /**
     * Candidate live logs of a domain: `<domain>` and `<domain>-ssl_log` in
     * every domlogs_dirs entry. The same physical file reached through two
     * directories (symlinked layouts) is returned once. `shared` is true for
     * files found in a directory common to every account (no {user} in its
     * template): the caller must make sure the domain belongs to $user before
     * reading those.
     *
     * @return array<int,array{path:string,https:bool,shared:bool}>
     */
    public function forDomain(string $user, string $domain): array
    {
        Paths::assertDomain($domain);
        $out = [];
        $seen = [];
        foreach ($this->paths->domlogsDirMap($user) as $dir => $shared) {
            foreach ([$domain => false, $domain . '-ssl_log' => true] as $name => $https) {
                $file = $dir . '/' . $name;
                if (!is_file($file)) {
                    continue;
                }
                $st = @stat($file);
                $id = $st ? $st['dev'] . ':' . $st['ino'] : (realpath($file) ?: $file);
                if (isset($seen[$id])) {
                    continue;
                }
                $seen[$id] = true;
                $out[] = ['path' => $file, 'https' => $https, 'shared' => $shared];
            }
        }
        return $out;
    }

    /**
     * Domains that have a log file in the per-user domlog directories (those
     * configured with a {user} placeholder) — catches domains missing from
     * /etc/userdomains.
     *
     * @return string[]
     */
    public function domainsInUserDirs(string $user): array
    {
        $out = [];
        foreach ((array) $this->config->get('paths.domlogs_dirs', []) as $tpl) {
            if (!is_string($tpl) || !str_contains($tpl, '{user}')) {
                continue;
            }
            $dir = rtrim(str_replace('{user}', Paths::assertUser($user), $tpl), '/');
            foreach (@scandir($dir) ?: [] as $name) {
                if (self::isIgnored($name) || !is_file($dir . '/' . $name)) {
                    continue;
                }
                $d = strtolower(preg_replace('/-ssl_log$/', '', $name) ?? $name);
                if (Paths::isValidDomain($d) && str_contains($d, '.')) {
                    $out[$d] = true;
                }
            }
        }
        $list = array_keys($out);
        sort($list);
        return $list;
    }

    /**
     * Compressed archives of the account (`<domain>[-ssl_log]-<Mon>-<YYYY>.gz`
     * or any `*.gz` starting with "<domain>-"), assigned to the longest
     * matching domain. Symlinks and files not owned by the account (or root)
     * are skipped: the archive directory is user-writable.
     *
     * @param string[] $domains
     * @return array<string,array<int,array{path:string,https:bool,size:int,mtime:int}>> domain => archives
     */
    public function archives(string $user, array $domains, ?string $dir = null): array
    {
        $dir = rtrim($dir ?? $this->paths->archiveDir($user), '/');
        if ($dir === '' || is_link($dir) || !is_dir($dir)) {
            return [];
        }
        $uid = Fs::isRoot() ? Fs::userId($user) : null;
        usort($domains, static fn (string $a, string $b): int => strlen($b) <=> strlen($a));
        $out = [];
        foreach (@scandir($dir) ?: [] as $name) {
            if (!str_ends_with($name, '.gz') || $name[0] === '.' || str_contains($name, 'bytes_log') || str_contains($name, 'ftp_log')) {
                continue;
            }
            $file = $dir . '/' . $name;
            $st = @lstat($file);
            if ($st === false || ($st['mode'] & 0170000) !== 0100000) {
                continue; // not a regular file (symlink, dir, fifo…)
            }
            if (Fs::isRoot() && $st['uid'] !== 0 && $uid !== null && $st['uid'] !== $uid) {
                continue;
            }
            if (Fs::isRoot() && $st['uid'] === 0 && $st['nlink'] > 1) {
                continue;
            }
            foreach ($domains as $d) {
                if (str_starts_with($name, $d . '-')) {
                    $rest = substr($name, strlen($d) + 1);
                    $out[$d][] = [
                        'path' => $file,
                        'https' => str_starts_with($rest, 'ssl_log'),
                        'size' => (int) $st['size'],
                        'mtime' => (int) $st['mtime'],
                    ];
                    break;
                }
            }
        }
        foreach ($out as &$list) {
            usort($list, static fn (array $a, array $b): int => [$a['mtime'], $a['path']] <=> [$b['mtime'], $b['path']]);
        }
        unset($list);
        ksort($out);
        return $out;
    }
}
