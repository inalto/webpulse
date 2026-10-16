<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Classify\MmdbReader;
use WebPulse\Util\Fs;

/**
 * webpulse geoip-update [--force] [--url URL] [--out FILE]
 *
 * The download is streamed to a temporary file next to the target (never
 * held in memory), capped at MAX_DOWNLOAD bytes (MAX_DATABASE once
 * decompressed); at most one redirect is followed, and only to https.
 */
final class GeoipUpdateCommand implements CommandInterface
{
    public const DEFAULT_URL = 'https://download.db-ip.com/free/dbip-country-lite-{YYYY}-{MM}.mmdb.gz';
    /** Largest download accepted (bytes). */
    public const MAX_DOWNLOAD = 64 * 1024 * 1024;
    /** Largest decompressed database accepted (bytes). */
    public const MAX_DATABASE = 512 * 1024 * 1024;

    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'download the free DB-IP country database (when geoip.enabled)';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse geoip-update [--force] [--url URL] [--out FILE]

        Downloads dbip-country-lite-YYYY-MM.mmdb.gz for the current month (falls
        back to the previous month), validates it and atomically replaces
        geoip.country_db. Does nothing unless geoip.enabled, or --force.

          --url URL   source (may contain {YYYY} and {MM}; file:// and local paths work)
          --out FILE  destination instead of geoip.country_db
        TXT;
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return ['force' => Args::FLAG, 'url' => Args::VALUE, 'out' => Args::VALUE];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        $app = $cli->app();
        if (!$args->flag('force') && !$app->config->get('geoip.enabled', false)) {
            $cli->info('GeoIP is disabled (geoip.enabled = false); use --force to download anyway');
            return Cli::EXIT_OK;
        }
        $target = $args->opt('out') ?? (string) $app->config->get('geoip.country_db');
        $tpl = $args->opt('url') ?? (string) $app->config->get('geoip.url', self::DEFAULT_URL);
        $now = $app->now();
        $months = [gmdate('Y-m', $now), gmdate('Y-m', (int) strtotime(gmdate('Y-m-01', $now) . ' -1 month'))];
        Fs::ensureDir(dirname($target), 0755);
        $base = dirname($target) . '/.' . basename($target) . '.' . getmypid();
        $download = $base . '.download';
        $tmp = $base . '.tmp';
        $used = null;
        $errors = [];
        try {
            foreach (array_unique($months) as $ym) {
                [$y, $m] = explode('-', $ym);
                $url = str_replace(['{YYYY}', '{MM}'], [$y, $m], $tpl);
                try {
                    self::fetchTo($url, $download);
                    $used = $url;
                    break;
                } catch (\RuntimeException $e) {
                    $errors[] = $e->getMessage();
                }
                if ($url === $tpl) {
                    break; // no placeholder: no point in retrying
                }
            }
            if ($used === null) {
                $cli->err('GeoIP download failed: ' . implode('; ', $errors));
                return Cli::EXIT_ERROR;
            }
            $head = (string) @file_get_contents($download, false, null, 0, 2);
            if ($head === "\x1f\x8b") {
                self::gunzipTo($download, $tmp);
                @unlink($download);
            } elseif (!@rename($download, $tmp)) {
                $cli->err("cannot write $tmp");
                return Cli::EXIT_ERROR;
            }
            try {
                $meta = (new MmdbReader($tmp))->metadata();
            } catch (\Throwable $e) {
                $cli->err('downloaded file is not a valid MaxMind DB: ' . $e->getMessage());
                return Cli::EXIT_ERROR;
            }
            @chmod($tmp, 0644);
            $bytes = (int) filesize($tmp);
            if (!@rename($tmp, $target)) {
                $cli->err("cannot move the database to $target");
                return Cli::EXIT_ERROR;
            }
        } catch (\RuntimeException $e) {
            $cli->err('GeoIP update failed: ' . $e->getMessage());
            return Cli::EXIT_ERROR;
        } finally {
            @unlink($download);
            @unlink($tmp);
        }
        $info = [
            'file' => $target,
            'source' => $used,
            'database_type' => $meta['database_type'] ?? null,
            'build_epoch' => $meta['build_epoch'] ?? null,
            'build_date' => isset($meta['build_epoch']) ? gmdate('Y-m-d', (int) $meta['build_epoch']) : null,
            'bytes' => $bytes,
        ];
        $app->log->info('geoip database updated', $info);
        $cli->result($info, static fn (): string => sprintf('installed %s (%s, built %s)', $target, $info['database_type'] ?? '?', $info['build_date'] ?? '?'));
        return Cli::EXIT_OK;
    }

    /**
     * Downloads (or copies) $url into the file $dest, streaming, at most
     * $max bytes. Redirects are not followed automatically: one redirect is
     * accepted, and only to an https:// URL (never a downgrade to http).
     *
     * @throws \RuntimeException
     */
    public static function fetchTo(string $url, string $dest, int $max = self::MAX_DOWNLOAD): void
    {
        if (str_starts_with($url, 'file://') || !preg_match('~^[a-z][a-z0-9+.-]*://~i', $url)) {
            $path = str_starts_with($url, 'file://') ? substr($url, 7) : $url;
            $in = @fopen($path, 'rb');
            if ($in === false) {
                throw new \RuntimeException("cannot read $path");
            }
            self::copyCapped($in, $dest, $max, $path);
            return;
        }
        if (!preg_match('~^https?://~i', $url)) {
            throw new \RuntimeException("unsupported URL $url");
        }
        if (ini_get('allow_url_fopen') && in_array('https', stream_get_wrappers(), true)) {
            $current = $url;
            for ($hop = 0; $hop <= 1; $hop++) {
                $ctx = stream_context_create(['http' => ['timeout' => 120, 'follow_location' => 0, 'max_redirects' => 0,
                    'user_agent' => 'WebPulse geoip-update', 'ignore_errors' => true]]);
                $in = @fopen($current, 'rb', false, $ctx);
                if ($in === false) {
                    break; // no answer at all: try curl below
                }
                $headers = stream_get_meta_data($in)['wrapper_data'] ?? [];
                [$status, $location, $length] = self::responseInfo(is_array($headers) ? $headers : []);
                if ($status >= 300 && $status < 400) {
                    fclose($in);
                    $next = $location === null ? null : self::redirectTarget($current, $location);
                    if ($hop === 1 || $next === null) {
                        throw new \RuntimeException("$url: refusing redirect to " . ($location ?? '(none)') . ' (only one redirect, to https)');
                    }
                    $current = $next;
                    continue;
                }
                if ($status !== 200) {
                    fclose($in);
                    throw new \RuntimeException("$url: HTTP $status");
                }
                if ($length !== null && $length > $max) {
                    fclose($in);
                    throw new \RuntimeException("$url: download larger than $max bytes");
                }
                self::copyCapped($in, $dest, $max, $url);
                return;
            }
        }
        $proc = @proc_open(['curl', '-fsS', '--proto', '=http,https', '--proto-redir', '=https', '--location', '--max-redirs', '1',
            '--max-filesize', (string) $max, '--max-time', '120', '-o', $dest, '--', $url],
            [1 => ['file', '/dev/null', 'w'], 2 => ['pipe', 'w']], $pipes);
        if (!is_resource($proc)) {
            throw new \RuntimeException("cannot download $url (no https wrapper and no curl)");
        }
        $e = stream_get_contents($pipes[2]);
        fclose($pipes[2]);
        $code = proc_close($proc);
        clearstatcache(true, $dest);
        if ($code !== 0 || !is_file($dest) || filesize($dest) === 0 || filesize($dest) > $max) {
            @unlink($dest);
            throw new \RuntimeException("$url: " . (trim((string) $e) ?: "curl exit code $code"));
        }
    }

    /**
     * Where a redirect may go: an absolute https:// URL, or a path/relative
     * reference resolved against an https:// source. Anything else (http,
     * other schemes, protocol-relative downgrades) gives null.
     */
    public static function redirectTarget(string $from, string $location): ?string
    {
        $location = trim($location);
        if ($location === '' || preg_match('/[\x00-\x20\x7f]/', $location)) {
            return null;
        }
        if (preg_match('~^[a-z][a-z0-9+.-]*:~i', $location)) {
            return stripos($location, 'https://') === 0 ? $location : null;
        }
        if (stripos($from, 'https://') !== 0) {
            return null; // relative redirect of an http:// source stays on http
        }
        if (str_starts_with($location, '//')) {
            return 'https:' . $location;
        }
        $origin = (string) preg_replace('~^(https://[^/?#]+).*$~i', '$1', $from);
        if ($location[0] === '/') {
            return $origin . $location;
        }
        $path = (string) parse_url($from, PHP_URL_PATH);
        $dir = str_contains($path, '/') ? substr($path, 0, (int) strrpos($path, '/') + 1) : '/';
        return $origin . $dir . $location;
    }

    /**
     * Status of the last response in a header list and its Location /
     * Content-Length.
     *
     * @param array<int,string> $headers
     * @return array{0:int,1:?string,2:?int}
     */
    private static function responseInfo(array $headers): array
    {
        $status = 0;
        $location = null;
        $length = null;
        foreach ($headers as $h) {
            if (preg_match('~^HTTP/\S+\s+(\d{3})~', $h, $m)) {
                $status = (int) $m[1];
                $location = null;
                $length = null;
            } elseif (preg_match('~^Location:\s*(.*)$~i', $h, $m)) {
                $location = trim($m[1]);
            } elseif (preg_match('~^Content-Length:\s*(\d+)~i', $h, $m)) {
                $length = (int) $m[1];
            }
        }
        return [$status, $location, $length];
    }

    /**
     * Streams $in into the file $dest; fails (and removes $dest) past $max bytes.
     *
     * @param resource $in
     * @throws \RuntimeException
     */
    private static function copyCapped($in, string $dest, int $max, string $what): void
    {
        $out = @fopen($dest, 'wb');
        if ($out === false) {
            fclose($in);
            throw new \RuntimeException("cannot write $dest");
        }
        $n = 0;
        try {
            while (!feof($in)) {
                $chunk = fread($in, 1 << 16);
                if ($chunk === false) {
                    throw new \RuntimeException("$what: read error");
                }
                $n += strlen($chunk);
                if ($n > $max) {
                    throw new \RuntimeException("$what: download larger than $max bytes");
                }
                if ($chunk !== '' && fwrite($out, $chunk) !== strlen($chunk)) {
                    throw new \RuntimeException("cannot write $dest");
                }
            }
            if ($n === 0) {
                throw new \RuntimeException("$what: empty download");
            }
        } catch (\RuntimeException $e) {
            fclose($out);
            @unlink($dest);
            throw $e;
        } finally {
            fclose($in);
        }
        fclose($out);
    }

    /**
     * Decompresses the gzip file $src into $dest, streaming, at most
     * MAX_DATABASE bytes of output.
     *
     * @throws \RuntimeException
     */
    private static function gunzipTo(string $src, string $dest): void
    {
        if (function_exists('gzopen')) {
            $in = @gzopen($src, 'rb');
            if ($in === false) {
                throw new \RuntimeException('corrupt gzip data');
            }
            $out = @fopen($dest, 'wb');
            if ($out === false) {
                gzclose($in);
                throw new \RuntimeException("cannot write $dest");
            }
            $n = 0;
            try {
                while (!gzeof($in)) {
                    $chunk = gzread($in, 1 << 16);
                    if ($chunk === false) {
                        throw new \RuntimeException('corrupt gzip data');
                    }
                    if ($chunk === '') {
                        break;
                    }
                    $n += strlen($chunk);
                    if ($n > self::MAX_DATABASE) {
                        throw new \RuntimeException('decompressed database larger than ' . self::MAX_DATABASE . ' bytes');
                    }
                    fwrite($out, $chunk);
                }
            } catch (\RuntimeException $e) {
                fclose($out);
                @unlink($dest);
                throw $e;
            } finally {
                gzclose($in);
            }
            fclose($out);
            return;
        }
        $proc = proc_open(['gzip', '-dc', '--', $src], [1 => ['pipe', 'w'], 2 => ['file', '/dev/null', 'w']], $pipes);
        if (!is_resource($proc)) {
            throw new \RuntimeException('cannot run gzip -dc');
        }
        try {
            self::copyCapped($pipes[1], $dest, self::MAX_DATABASE, 'gzip -dc');
        } finally {
            proc_terminate($proc);
            proc_close($proc);
        }
    }
}
