<?php

declare(strict_types=1);

namespace WebPulse\Util;

/**
 * flock() based lock file. The lock is released when the object is
 * destroyed or {@see Lock::release()} is called.
 */
final class Lock
{
    /** @var resource|null */
    private $fh;

    /** @param resource $fh */
    private function __construct($fh, private string $path)
    {
        $this->fh = $fh;
    }

    /**
     * Tries to take an exclusive lock on $path.
     *
     * @param float $waitSeconds 0 = non blocking; otherwise poll up to that long
     * @return self|null null when the lock is held by another process
     * @throws \RuntimeException when the lock file cannot be opened
     */
    public static function acquire(string $path, float $waitSeconds = 0.0): ?self
    {
        $dir = dirname($path);
        if (!is_dir($dir)) {
            @mkdir($dir, 0700, true);
        }
        $fh = @fopen($path, 'c');
        if ($fh === false) {
            throw new \RuntimeException("cannot open lock file $path");
        }
        @chmod($path, 0600);
        $deadline = microtime(true) + $waitSeconds;
        while (true) {
            if (flock($fh, LOCK_EX | LOCK_NB)) {
                ftruncate($fh, 0);
                fwrite($fh, (string) getmypid());
                fflush($fh);
                return new self($fh, $path);
            }
            if (microtime(true) >= $deadline) {
                fclose($fh);
                return null;
            }
            usleep(100000);
        }
    }

    /** True when somebody currently holds the lock on $path. */
    public static function isHeld(string $path): bool
    {
        if (!is_file($path)) {
            return false;
        }
        $fh = @fopen($path, 'r');
        if ($fh === false) {
            return false;
        }
        $free = flock($fh, LOCK_SH | LOCK_NB);
        if ($free) {
            flock($fh, LOCK_UN);
        }
        fclose($fh);
        return !$free;
    }

    /** Lock file path. */
    public function path(): string
    {
        return $this->path;
    }

    /** Releases the lock (idempotent). */
    public function release(): void
    {
        if ($this->fh !== null) {
            flock($this->fh, LOCK_UN);
            fclose($this->fh);
            $this->fh = null;
        }
    }

    public function __destruct()
    {
        $this->release();
    }
}
