<?php

declare(strict_types=1);

namespace WebPulse;

/**
 * Minimal file logger: <log_dir>/webpulse.log, rotated by size (5 x 5 MiB).
 * Never throws: when the file cannot be written (e.g. the cPanel plugin runs
 * as the account) messages are dropped, or mirrored to stderr when verbose.
 */
final class Log
{
    public const LEVELS = ['debug' => 0, 'info' => 1, 'warn' => 2, 'error' => 3];
    private const MAX_BYTES = 5 * 1024 * 1024;
    private const KEEP = 5;

    private int $min = 1;
    private bool $stderr = false;
    private bool $stderrErrors = false;
    private ?string $file;
    private bool $mkdirTried = false;

    /** Logs to <$logDir>/webpulse.log; null disables the file. */
    public function __construct(?string $logDir)
    {
        $this->file = $logDir !== null && $logDir !== '' ? rtrim($logDir, '/') . '/webpulse.log' : null;
    }

    /** --verbose: mirror every message (debug included) to stderr. */
    public function setVerbose(bool $on): void
    {
        $this->stderr = $on;
        $this->min = $on ? 0 : 1;
    }

    /** Mirror warnings and errors to stderr (CLI default when not --quiet). */
    public function setStderrErrors(bool $on): void
    {
        $this->stderrErrors = $on;
    }

    /** Path of the log file (null when disabled). */
    public function file(): ?string
    {
        return $this->file;
    }

    /** @param array<string,mixed> $ctx */
    public function debug(string $msg, array $ctx = []): void
    {
        $this->write('debug', $msg, $ctx);
    }

    /** @param array<string,mixed> $ctx */
    public function info(string $msg, array $ctx = []): void
    {
        $this->write('info', $msg, $ctx);
    }

    /** @param array<string,mixed> $ctx */
    public function warn(string $msg, array $ctx = []): void
    {
        $this->write('warn', $msg, $ctx);
    }

    /** @param array<string,mixed> $ctx */
    public function error(string $msg, array $ctx = []): void
    {
        $this->write('error', $msg, $ctx);
    }

    /** @param array<string,mixed> $ctx */
    public function write(string $level, string $msg, array $ctx = []): void
    {
        $lv = self::LEVELS[$level] ?? 1;
        if ($lv < $this->min) {
            return;
        }
        $line = date('Y-m-d\TH:i:sP') . ' [' . $level . '] ' . getmypid() . ' ' . $msg;
        if ($ctx !== []) {
            $line .= ' ' . json_encode($ctx, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE);
        }
        $line .= "\n";
        if ($this->stderr || ($this->stderrErrors && $lv >= 2)) {
            @fwrite(STDERR, $line);
        }
        if ($this->file === null) {
            return;
        }
        $dir = dirname($this->file);
        if (!is_dir($dir)) {
            if ($this->mkdirTried || !is_dir(dirname($dir)) || !is_writable(dirname($dir))) {
                return;
            }
            $this->mkdirTried = true;
            if (!@mkdir($dir, 0700, true)) {
                return;
            }
        }
        if (!is_writable($dir)) {
            return;
        }
        $this->rotate();
        $fresh = !file_exists($this->file);
        if (@file_put_contents($this->file, $line, FILE_APPEND | LOCK_EX) !== false && $fresh) {
            @chmod($this->file, 0600);
        }
    }

    private function rotate(): void
    {
        $size = @filesize((string) $this->file);
        if ($size === false || $size < self::MAX_BYTES) {
            return;
        }
        for ($i = self::KEEP - 1; $i >= 1; $i--) {
            if (file_exists($this->file . '.' . $i)) {
                @rename($this->file . '.' . $i, $this->file . '.' . ($i + 1));
            }
        }
        @rename((string) $this->file, $this->file . '.1');
    }
}
