<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Util\Lock;

/**
 * webpulse status [--json]
 */
final class StatusCommand implements CommandInterface
{
    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'collection health: paths, accounts, lag, last jobs, GeoIP';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return "Usage: webpulse status [--json]\n\nExit code 1 when the registry cannot be opened.";
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return [];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        $app = $cli->app();
        $p = $app->paths;
        $now = $app->now();
        $st = [
            'version' => $app->version(),
            'php' => PHP_VERSION,
            'config' => ['path' => $app->config->path(), 'error' => $app->config->loadError()],
            'paths' => [],
            'registry' => ['ok' => false, 'error' => null],
            'accounts' => ['total' => 0, 'enabled' => 0, 'removed' => 0],
            'domains' => ['total' => 0, 'enabled' => 0, 'with_data' => 0],
            'collector' => ['last_collect_at' => null, 'newest_line_ts' => null, 'lag_seconds' => null,
                'running' => Lock::isHeld($p->runDir() . '/collect.lock'), 'hook_flags' => 0,
                'cron_installed' => is_file($p->cronFile())],
            'geoip' => ['enabled' => (bool) $app->config->get('geoip.enabled'), 'db' => $app->config->get('geoip.country_db'),
                'present' => false, 'db_date' => null],
            'jobs' => [],
            'lines_total' => 0,
            'data_bytes' => 0,
        ];
        foreach (['data_dir' => $p->dataDir(), 'registry_db' => $p->registryDb(), 'log_dir' => $p->logDir(), 'run_dir' => $p->runDir()] as $k => $path) {
            $st['paths'][$k] = ['path' => $path, 'exists' => file_exists($path), 'writable' => is_writable($path)];
        }
        $st['collector']['hook_flags'] = count(array_filter(@scandir($p->hookDir()) ?: [], static fn ($f) => $f[0] !== '.'));
        $code = Cli::EXIT_OK;
        try {
            $reg = $app->registry();
            $st['registry']['ok'] = true;
            foreach ($reg->accounts() as $a) {
                $st['accounts']['total']++;
                $st['accounts']['enabled'] += $a['enabled'] && $a['removed_at'] === null ? 1 : 0;
                $st['accounts']['removed'] += $a['removed_at'] !== null ? 1 : 0;
            }
            foreach ($reg->domains() as $d) {
                if ($d['removed_at'] !== null) {
                    continue;
                }
                $st['domains']['total']++;
                $st['domains']['enabled'] += $d['enabled'] ? 1 : 0;
            }
            foreach ($reg->summaries() as $s) {
                $st['domains']['with_data']++;
                $st['data_bytes'] += (int) $s['db_bytes'];
                $st['lines_total'] += (int) $s['lines_total'];
                if ($s['last_collect_at'] !== null && $s['last_collect_at'] > ($st['collector']['last_collect_at'] ?? 0)) {
                    $st['collector']['last_collect_at'] = $s['last_collect_at'];
                }
                if ($s['last_line_ts'] !== null && $s['last_line_ts'] > ($st['collector']['newest_line_ts'] ?? 0)) {
                    $st['collector']['newest_line_ts'] = $s['last_line_ts'];
                }
            }
            if ($st['collector']['newest_line_ts'] !== null) {
                $st['collector']['lag_seconds'] = max(0, $now - $st['collector']['newest_line_ts']);
            }
            $st['jobs'] = $reg->jobs(5);
        } catch (\Throwable $e) {
            $st['registry']['error'] = $e->getMessage();
            $code = Cli::EXIT_ERROR;
        }
        $db = (string) $st['geoip']['db'];
        if ($db !== '' && is_file($db)) {
            $st['geoip']['present'] = true;
            $meta = $app->geoIp()->metadata();
            $st['geoip']['db_date'] = isset($meta['build_epoch']) ? gmdate('Y-m-d', (int) $meta['build_epoch']) : gmdate('Y-m-d', (int) filemtime($db));
        }
        $cli->result($st, static function () use ($st): string {
            $t = static fn (?int $ts): string => $ts === null ? 'never' : date('Y-m-d H:i:s', $ts);
            $l = [];
            $l[] = "WebPulse {$st['version']} (PHP {$st['php']})";
            $l[] = 'config:      ' . ($st['config']['path'] ?? '(defaults)') . ($st['config']['error'] ? " [{$st['config']['error']}]" : '');
            foreach ($st['paths'] as $k => $p) {
                $l[] = sprintf('%-12s %s%s', $k . ':', $p['path'], $p['exists'] ? '' : ' [missing]');
            }
            $l[] = 'registry:    ' . ($st['registry']['ok'] ? 'ok' : 'ERROR ' . $st['registry']['error']);
            $l[] = sprintf('accounts:    %d (%d enabled, %d removed)', $st['accounts']['total'], $st['accounts']['enabled'], $st['accounts']['removed']);
            $l[] = sprintf('domains:     %d (%d enabled, %d with data, %.1f MiB)', $st['domains']['total'], $st['domains']['enabled'], $st['domains']['with_data'], $st['data_bytes'] / 1048576);
            $l[] = 'last collect: ' . $t($st['collector']['last_collect_at']) . ($st['collector']['running'] ? ' (running now)' : '');
            $l[] = 'newest line: ' . $t($st['collector']['newest_line_ts']) . ($st['collector']['lag_seconds'] !== null ? sprintf(' (lag %ds)', $st['collector']['lag_seconds']) : '');
            $l[] = 'cron:        ' . ($st['collector']['cron_installed'] ? 'installed' : 'NOT installed') . ', hook flags pending: ' . $st['collector']['hook_flags'];
            $l[] = 'geoip:       ' . ($st['geoip']['enabled'] ? 'enabled' : 'disabled') . ($st['geoip']['present'] ? ', db ' . $st['geoip']['db_date'] : ', no database');
            if ($st['jobs'] !== []) {
                $l[] = 'last jobs:';
                foreach ($st['jobs'] as $j) {
                    $l[] = sprintf('  #%d %-8s %-20s %-7s %s %d lines%s', $j['id'], $j['type'], (string) $j['scope'], $j['status'], $t($j['started_at']), $j['lines'], $j['message'] ? ' — ' . $j['message'] : '');
                }
            }
            return implode("\n", $l);
        });
        return $code;
    }
}
