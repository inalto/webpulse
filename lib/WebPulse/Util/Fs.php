<?php

declare(strict_types=1);

namespace WebPulse\Util;

/**
 * Filesystem helpers aware of root vs. unprivileged execution.
 */
final class Fs
{
    /** True when running as root (posix optional). */
    public static function isRoot(): bool
    {
        if (function_exists('posix_geteuid')) {
            return posix_geteuid() === 0;
        }
        return getenv('USER') === 'root';
    }

    /** gid of a group name, or null when unknown / posix missing. */
    public static function groupId(string $group): ?int
    {
        if (!function_exists('posix_getgrnam')) {
            return null;
        }
        $g = @posix_getgrnam($group);
        return is_array($g) ? (int) $g['gid'] : null;
    }

    /** uid of a user name, or null when unknown / posix missing. */
    public static function userId(string $user): ?int
    {
        if (!function_exists('posix_getpwnam')) {
            return null;
        }
        $u = @posix_getpwnam($user);
        return is_array($u) ? (int) $u['uid'] : null;
    }

    /**
     * Creates $dir (recursively) when missing and enforces $mode. When
     * running as root, ownership is set to $owner/$group if those exist.
     *
     * @throws \RuntimeException when the directory cannot be created
     */
    public static function ensureDir(string $dir, int $mode, ?string $owner = null, ?string $group = null): void
    {
        if (is_link($dir)) {
            throw new \RuntimeException("refusing to use symlinked directory $dir");
        }
        if (!is_dir($dir) && !@mkdir($dir, $mode, true) && !is_dir($dir)) {
            throw new \RuntimeException("cannot create directory $dir");
        }
        @chmod($dir, $mode);
        self::own($dir, $owner, $group);
    }

    /** chown/chgrp $path when root and the principal exists (best effort). */
    public static function own(string $path, ?string $owner, ?string $group): void
    {
        if (!self::isRoot()) {
            return;
        }
        if ($owner !== null && self::userId($owner) !== null) {
            @chown($path, $owner);
        }
        if ($group !== null && self::groupId($group) !== null) {
            @chgrp($path, $group);
        }
    }

    /**
     * Atomically writes $data to $file (temp file + rename) with $mode.
     *
     * @throws \RuntimeException
     */
    public static function writeAtomic(string $file, string $data, int $mode = 0644): void
    {
        $dir = dirname($file);
        $tmp = $dir . '/.' . basename($file) . '.' . getmypid() . '.' . bin2hex(random_bytes(4)) . '.tmp';
        if (@file_put_contents($tmp, $data) === false) {
            throw new \RuntimeException("cannot write $tmp");
        }
        @chmod($tmp, $mode);
        if (!@rename($tmp, $file)) {
            @unlink($tmp);
            throw new \RuntimeException("cannot rename $tmp to $file");
        }
    }

    /**
     * Opens an existing regular file for reading without ever blocking and
     * without following a file swapped in after the caller's lstat(): the
     * open is non-blocking (a FIFO renamed into place returns at once instead
     * of hanging the collector, which holds the global lock) and the opened
     * descriptor must be a regular file with the same device and inode as
     * $lst. Meant for files in directories writable by an account.
     *
     * @param array<int|string,int>|null $lst lstat() of $path taken by the caller (taken here when null)
     * @return resource
     * @throws \RuntimeException
     */
    public static function openRegular(string $path, ?array $lst = null)
    {
        if ($lst === null) {
            clearstatcache(true, $path);
            $lst = @lstat($path);
            if ($lst === false) {
                throw new \RuntimeException("cannot stat $path");
            }
        }
        if (((int) $lst['mode'] & 0170000) !== 0100000) {
            throw new \RuntimeException("$path is not a regular file");
        }
        $fh = @fopen($path, 'rbn'); // n = O_NONBLOCK
        if ($fh === false) {
            throw new \RuntimeException("cannot open $path");
        }
        $st = fstat($fh);
        if ($st === false || ($st['mode'] & 0170000) !== 0100000 || $st['ino'] !== (int) $lst['ino'] || $st['dev'] !== (int) $lst['dev']) {
            fclose($fh);
            throw new \RuntimeException("$path changed while opening");
        }
        return $fh;
    }

    /**
     * Reads a whole regular file of at most $max bytes through
     * {@see Fs::openRegular()} (non-blocking reads, bounded by $max).
     *
     * @param array<int|string,int>|null $lst
     * @throws \RuntimeException when the file is not a regular file, changed, or is larger than $max
     */
    public static function readRegular(string $path, int $max, ?array $lst = null): string
    {
        $fh = self::openRegular($path, $lst);
        try {
            $st = fstat($fh);
            if ($st === false || $st['size'] > $max) {
                throw new \RuntimeException("$path is larger than $max bytes");
            }
            stream_set_blocking($fh, false);
            $data = '';
            while (strlen($data) <= $max) {
                $chunk = fread($fh, $max + 1 - strlen($data));
                if ($chunk === false || $chunk === '') {
                    break;
                }
                $data .= $chunk;
            }
            if (strlen($data) > $max) {
                throw new \RuntimeException("$path is larger than $max bytes");
            }
            return $data;
        } finally {
            fclose($fh);
        }
    }

    /**
     * Removes a file or a directory tree without following symlinks.
     *
     * Every entry is lstat()ed; symlinks and other non-directories are
     * unlinked (unlink never follows the last component). The recursion only
     * descends into directories owned by the effective user: a directory
     * owned by someone else (e.g. the account-owned `user/` directory of a
     * data dir, whose content the account can swap at any time) is handed to
     * `rm -rf --one-file-system`, which walks it race-free with *at() calls.
     *
     * @param int|null $euid owner required to descend (default: the effective uid; tests)
     * @return bool true when $path is gone
     */
    public static function removeTree(string $path, ?int $euid = null): bool
    {
        clearstatcache(true, $path);
        $st = @lstat($path);
        if ($st === false) {
            return true;
        }
        if (($st['mode'] & 0170000) !== 0040000) {
            return @unlink($path);
        }
        $euid ??= function_exists('posix_geteuid') ? posix_geteuid() : null;
        if ($euid !== null && $st['uid'] !== $euid) {
            return self::rmRf($path);
        }
        foreach (@scandir($path) ?: [] as $e) {
            if ($e !== '.' && $e !== '..') {
                self::removeTree($path . '/' . $e, $euid);
            }
        }
        return @rmdir($path);
    }

    /** `rm -rf --one-file-system -- $path` (no shell). */
    private static function rmRf(string $path): bool
    {
        $proc = @proc_open(['rm', '-rf', '--one-file-system', '--', $path], [0 => ['file', '/dev/null', 'r'], 1 => ['file', '/dev/null', 'w'], 2 => ['file', '/dev/null', 'w']], $pipes);
        if (!is_resource($proc)) {
            return false;
        }
        proc_close($proc);
        clearstatcache(true, $path);
        return @lstat($path) === false;
    }
}
