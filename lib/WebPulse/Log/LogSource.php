<?php

declare(strict_types=1);

namespace WebPulse\Log;

/**
 * Reads complete lines from a live log file starting at its checkpoint, or
 * from a gzip archive (backfill).
 *
 * Checkpoint rules (live files):
 *  - no checkpoint                 -> start at 0
 *  - inode changed                 -> start at 0 of the new file ("inode")
 *  - size < stored offset          -> file truncated, start at 0 ("truncated")
 *  - first bytes differ from the stored fingerprint -> replaced, start at 0 ("replaced")
 *  - otherwise                     -> continue at the stored offset
 *
 * The file is read in 1 MiB chunks, never as a whole; a trailing line
 * without "\n" is left for the next run. {@see LogSource::offset()} only
 * advances over lines handed out by {@see LogSource::next()}, so the caller
 * stores it after its own transaction commits.
 */
final class LogSource
{
    public const CHUNK = 1048576;
    public const FINGERPRINT_BYTES = 256;
    /** A "line" longer than this is cut (and will be rejected by the parser). */
    public const MAX_LINE = 65536;

    /** Tests: pretend zlib is missing to exercise the `gzip -dc` fallback. */
    public static bool $forceGzipCli = false;

    /** @var resource|null */
    private $fh = null;
    /** @var resource|null */
    private $proc = null;
    /** @var resource|null */
    private $pipe = null;
    private mixed $inflate = null;
    private string $pending = '';
    private string $buf = '';
    private int $pos = 0;
    private bool $eof = false;
    private bool $skipping = false;
    private int $offset = 0;
    private int $start = 0;
    private int $inode = 0;
    private int $size = 0;
    private int $mtime = 0;
    private ?string $resetReason = null;

    /**
     * @param array<string,mixed>|null $checkpoint row of the `checkpoints` table (live files only)
     */
    public function __construct(
        private string $path,
        private bool $https = false,
        private ?array $checkpoint = null,
        private bool $gzip = false,
    ) {
    }

    public function path(): string
    {
        return $this->path;
    }

    public function https(): bool
    {
        return $this->https;
    }

    public function isGzip(): bool
    {
        return $this->gzip;
    }

    /**
     * Opens the file and positions it according to the checkpoint.
     *
     * @throws \RuntimeException when the file cannot be opened
     */
    public function open(): void
    {
        clearstatcache(true, $this->path);
        $lst = @lstat($this->path);
        if ($lst === false) {
            throw new \RuntimeException("cannot stat {$this->path}");
        }
        if ($this->gzip && ($lst['mode'] & 0170000) !== 0100000) {
            throw new \RuntimeException("{$this->path} is not a regular file");
        }
        $fh = @fopen($this->path, 'rb');
        if ($fh === false) {
            throw new \RuntimeException("cannot open {$this->path}");
        }
        $st = fstat($fh);
        if ($st === false || ($st['mode'] & 0170000) !== 0100000) {
            fclose($fh);
            throw new \RuntimeException("{$this->path} is not a regular file");
        }
        if ($this->gzip && ($st['ino'] !== $lst['ino'] || $st['dev'] !== $lst['dev'])) {
            fclose($fh);
            throw new \RuntimeException("{$this->path} changed while opening");
        }
        $this->fh = $fh;
        $this->inode = (int) $st['ino'];
        $this->size = (int) $st['size'];
        $this->mtime = (int) $st['mtime'];

        if ($this->gzip) {
            $this->openGzip();
            return;
        }

        $start = 0;
        $cp = $this->checkpoint;
        if ($cp !== null) {
            $off = (int) $cp['offset'];
            if ((int) $cp['inode'] !== $this->inode) {
                $this->resetReason = 'inode';
            } elseif ($this->size < $off) {
                $this->resetReason = 'truncated';
            } elseif (!$this->fingerprintMatches((string) ($cp['first_line'] ?? ''))) {
                $this->resetReason = 'replaced';
            } else {
                $start = $off;
            }
        }
        if ($start > 0 && fseek($this->fh, $start) !== 0) {
            $start = 0;
            $this->resetReason = 'seek';
            fseek($this->fh, 0);
        }
        $this->start = $start;
        $this->offset = $start;
    }

    /** Why the stored checkpoint was discarded (inode|truncated|replaced|seek), or null. */
    public function resetReason(): ?string
    {
        return $this->resetReason;
    }

    /** Offset the read started from. */
    public function startOffset(): int
    {
        return $this->start;
    }

    /** Bytes consumed up to the end of the last line returned by next(). */
    public function offset(): int
    {
        return $this->offset;
    }

    public function inode(): int
    {
        return $this->inode;
    }

    /** Size at open time (compressed size for archives). */
    public function size(): int
    {
        return $this->size;
    }

    public function mtime(): int
    {
        return $this->mtime;
    }

    /**
     * "<len>:<sha1 of the first len bytes>" with len = min(256, size).
     * Read with pread semantics (the current position is preserved).
     */
    public function fingerprint(): string
    {
        if ($this->fh === null || $this->gzip) {
            return '';
        }
        $pos = ftell($this->fh);
        clearstatcache(true, $this->path);
        $st = fstat($this->fh);
        $len = (int) min(self::FINGERPRINT_BYTES, $st === false ? 0 : $st['size']);
        if ($len === 0) {
            return '0:';
        }
        fseek($this->fh, 0);
        $head = (string) fread($this->fh, $len);
        fseek($this->fh, (int) $pos);
        return strlen($head) . ':' . sha1($head);
    }

    private function fingerprintMatches(string $stored): bool
    {
        if ($stored === '' || !str_contains($stored, ':')) {
            return true; // no fingerprint recorded: trust inode + size
        }
        [$len, $hash] = explode(':', $stored, 2);
        $len = (int) $len;
        if ($len === 0) {
            return true;
        }
        if ($this->size < $len) {
            return false;
        }
        fseek($this->fh, 0);
        $head = (string) fread($this->fh, $len);
        fseek($this->fh, 0);
        return sha1($head) === $hash;
    }

    /**
     * Returns the next complete line (without the line terminator) or null
     * when no complete line is left.
     *
     * @throws \RuntimeException on read / decompression errors
     */
    public function next(): ?string
    {
        while (true) {
            $nl = strpos($this->buf, "\n", $this->pos);
            if ($nl !== false) {
                $line = substr($this->buf, $this->pos, $nl - $this->pos);
                $this->offset += $nl - $this->pos + 1;
                $this->pos = $nl + 1;
                if ($this->skipping) {
                    $this->skipping = false;
                    continue;
                }
                if ($line !== '' && $line[strlen($line) - 1] === "\r") {
                    $line = substr($line, 0, -1);
                }
                return $line;
            }
            if ($this->pos > 0) {
                $this->buf = substr($this->buf, $this->pos);
                $this->pos = 0;
            }
            if (strlen($this->buf) >= 2 * self::CHUNK) {
                // absurdly long line: hand out a cut version once, skip the rest
                $wasSkipping = $this->skipping;
                $line = substr($this->buf, 0, self::MAX_LINE);
                $this->offset += strlen($this->buf);
                $this->buf = '';
                $this->skipping = true;
                if (!$wasSkipping) {
                    return $line;
                }
                continue;
            }
            if ($this->eof) {
                if ($this->gzip && $this->buf !== '') {
                    // archives: the last line may lack "\n"
                    $line = rtrim($this->buf, "\r");
                    $this->offset += strlen($this->buf);
                    $this->buf = '';
                    return $this->skipping ? null : $line;
                }
                return null;
            }
            $chunk = $this->gzip ? $this->readGzip() : $this->readPlain();
            if ($chunk === null) {
                $this->eof = true;
                continue;
            }
            $this->buf .= $chunk;
        }
    }

    private function readPlain(): ?string
    {
        if ($this->fh === null) {
            return null;
        }
        $data = fread($this->fh, self::CHUNK);
        if ($data === false) {
            throw new \RuntimeException("read error on {$this->path}");
        }
        if ($data === '') {
            return null;
        }
        return $data;
    }

    private function openGzip(): void
    {
        if (!self::$forceGzipCli && function_exists('inflate_init')) {
            $this->inflate = inflate_init(ZLIB_ENCODING_GZIP);
            return;
        }
        // zlib missing: stream the already opened descriptor through gzip -dc
        $proc = @proc_open(['gzip', '-dc'], [0 => $this->fh, 1 => ['pipe', 'w'], 2 => ['file', '/dev/null', 'w']], $pipes);
        if (!is_resource($proc)) {
            throw new \RuntimeException('cannot run gzip -dc');
        }
        $this->proc = $proc;
        $this->pipe = $pipes[1];
    }

    private function readGzip(): ?string
    {
        if ($this->pipe !== null) {
            if (feof($this->pipe)) {
                return null;
            }
            $data = fread($this->pipe, self::CHUNK);
            if ($data === false || $data === '') {
                return feof($this->pipe) ? null : '';
            }
            return $data;
        }
        while (true) {
            if ($this->pending === '') {
                if ($this->fh === null || feof($this->fh)) {
                    return null;
                }
                $this->pending = (string) fread($this->fh, self::CHUNK);
                if ($this->pending === '') {
                    continue;
                }
            }
            $in = $this->pending;
            $this->pending = '';
            $before = inflate_get_read_len($this->inflate);
            $out = @inflate_add($this->inflate, $in, ZLIB_SYNC_FLUSH);
            if ($out === false) {
                if (trim($in, "\0") === '') {
                    return null; // zero padding after the last member
                }
                throw new \RuntimeException("corrupt gzip data in {$this->path}");
            }
            if (inflate_get_status($this->inflate) === ZLIB_STREAM_END) {
                $used = inflate_get_read_len($this->inflate) - $before;
                $this->pending = (string) substr($in, $used);
                $this->inflate = inflate_init(ZLIB_ENCODING_GZIP);
                if (trim($this->pending, "\0") === '' && ($this->fh === null || feof($this->fh))) {
                    $this->pending = '';
                }
            }
            if ($out !== '') {
                return $out;
            }
        }
    }

    /** Closes the file (and the gzip helper process). */
    public function close(): void
    {
        if ($this->pipe !== null) {
            fclose($this->pipe);
            $this->pipe = null;
        }
        if ($this->proc !== null) {
            proc_close($this->proc);
            $this->proc = null;
        }
        if ($this->fh !== null) {
            fclose($this->fh);
            $this->fh = null;
        }
    }

    public function __destruct()
    {
        $this->close();
    }
}
