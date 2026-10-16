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

    /** Recursively removes a directory tree (does not follow symlinks). */
    public static function removeTree(string $path): void
    {
        if (is_link($path) || is_file($path)) {
            @unlink($path);
            return;
        }
        if (!is_dir($path)) {
            return;
        }
        foreach (scandir($path) ?: [] as $e) {
            if ($e !== '.' && $e !== '..') {
                self::removeTree($path . '/' . $e);
            }
        }
        @rmdir($path);
    }
}
