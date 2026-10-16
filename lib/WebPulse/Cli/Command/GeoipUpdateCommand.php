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
 */
final class GeoipUpdateCommand implements CommandInterface
{
    public const DEFAULT_URL = 'https://download.db-ip.com/free/dbip-country-lite-{YYYY}-{MM}.mmdb.gz';

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
        $data = null;
        $used = null;
        $errors = [];
        foreach (array_unique($months) as $ym) {
            [$y, $m] = explode('-', $ym);
            $url = str_replace(['{YYYY}', '{MM}'], [$y, $m], $tpl);
            try {
                $data = self::fetch($url);
                $used = $url;
                break;
            } catch (\RuntimeException $e) {
                $errors[] = $e->getMessage();
            }
            if ($url === $tpl) {
                break; // no placeholder: no point in retrying
            }
        }
        if ($data === null) {
            $cli->err('GeoIP download failed: ' . implode('; ', $errors));
            return Cli::EXIT_ERROR;
        }
        if (strncmp($data, "\x1f\x8b", 2) === 0) {
            $data = self::gunzip($data);
        }
        Fs::ensureDir(dirname($target), 0755);
        $tmp = dirname($target) . '/.' . basename($target) . '.' . getmypid() . '.tmp';
        if (@file_put_contents($tmp, $data) === false) {
            $cli->err("cannot write $tmp");
            return Cli::EXIT_ERROR;
        }
        try {
            $meta = (new MmdbReader($tmp))->metadata();
        } catch (\Throwable $e) {
            @unlink($tmp);
            $cli->err('downloaded file is not a valid MaxMind DB: ' . $e->getMessage());
            return Cli::EXIT_ERROR;
        }
        @chmod($tmp, 0644);
        if (!@rename($tmp, $target)) {
            @unlink($tmp);
            $cli->err("cannot move the database to $target");
            return Cli::EXIT_ERROR;
        }
        $info = [
            'file' => $target,
            'source' => $used,
            'database_type' => $meta['database_type'] ?? null,
            'build_epoch' => $meta['build_epoch'] ?? null,
            'build_date' => isset($meta['build_epoch']) ? gmdate('Y-m-d', (int) $meta['build_epoch']) : null,
            'bytes' => strlen($data),
        ];
        $app->log->info('geoip database updated', $info);
        $cli->result($info, static fn (): string => sprintf('installed %s (%s, built %s)', $target, $info['database_type'] ?? '?', $info['build_date'] ?? '?'));
        return Cli::EXIT_OK;
    }

    /** Downloads (or reads) $url. @throws \RuntimeException */
    public static function fetch(string $url): string
    {
        if (str_starts_with($url, 'file://') || !preg_match('~^[a-z]+://~i', $url)) {
            $path = str_starts_with($url, 'file://') ? substr($url, 7) : $url;
            $d = @file_get_contents($path);
            if ($d === false) {
                throw new \RuntimeException("cannot read $path");
            }
            return $d;
        }
        if (!preg_match('~^https?://~i', $url)) {
            throw new \RuntimeException("unsupported URL $url");
        }
        if (ini_get('allow_url_fopen') && in_array('https', stream_get_wrappers(), true)) {
            $ctx = stream_context_create(['http' => ['timeout' => 120, 'follow_location' => 1, 'user_agent' => 'WebPulse geoip-update', 'ignore_errors' => true]]);
            $d = @file_get_contents($url, false, $ctx);
            $status = 0;
            foreach ($http_response_header ?? [] as $h) {
                if (preg_match('~^HTTP/\S+\s+(\d{3})~', $h, $m)) {
                    $status = (int) $m[1];
                }
            }
            if ($d !== false && $status === 200 && $d !== '') {
                return $d;
            }
            if ($status !== 0 && $status !== 200) {
                throw new \RuntimeException("$url: HTTP $status");
            }
        }
        $proc = @proc_open(['curl', '-fsSL', '--max-time', '120', $url], [1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
        if (!is_resource($proc)) {
            throw new \RuntimeException("cannot download $url (no https wrapper and no curl)");
        }
        $d = stream_get_contents($pipes[1]);
        $e = stream_get_contents($pipes[2]);
        fclose($pipes[1]);
        fclose($pipes[2]);
        $code = proc_close($proc);
        if ($code !== 0 || $d === false || $d === '') {
            throw new \RuntimeException("$url: " . trim((string) $e));
        }
        return $d;
    }

    /** @throws \RuntimeException */
    private static function gunzip(string $data): string
    {
        if (function_exists('gzdecode')) {
            $d = @gzdecode($data);
            if ($d !== false) {
                return $d;
            }
            throw new \RuntimeException('corrupt gzip data');
        }
        $tmp = tempnam(sys_get_temp_dir(), 'wpgz');
        file_put_contents((string) $tmp, $data);
        $proc = proc_open(['gzip', '-dc', (string) $tmp], [1 => ['pipe', 'w'], 2 => ['file', '/dev/null', 'w']], $pipes);
        $out = is_resource($proc) ? stream_get_contents($pipes[1]) : false;
        if (is_resource($proc)) {
            fclose($pipes[1]);
            proc_close($proc);
        }
        @unlink((string) $tmp);
        if ($out === false || $out === '') {
            throw new \RuntimeException('gzip -dc failed');
        }
        return $out;
    }
}
