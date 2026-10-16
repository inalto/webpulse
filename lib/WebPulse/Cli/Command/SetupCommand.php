<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\App;
use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Collect\Collector;
use WebPulse\Store\DomainStore;
use WebPulse\Util\Fs;

/**
 * webpulse setup — creates config, directories and registry (idempotent).
 */
final class SetupCommand implements CommandInterface
{
    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'create config, directories and registry with the right modes (idempotent)';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse setup [--no-sync]

        Creates (or repairs) the configuration file from etc/config.json.default,
        the data/log/run/geoip directories, the hook flag directory (1777), the
        registry database (0600) and, unless --no-sync, one data directory per
        cPanel account (0750 root:<user>, with a user-owned "user/" sub-directory).
        TXT;
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return ['no-sync' => Args::FLAG];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        $cfgPath = $cli->configOption();
        if ($cfgPath === null) {
            $env = getenv('WEBPULSE_CONFIG');
            $cfgPath = is_string($env) && $env !== '' ? $env : App::DEFAULT_CONFIG;
        }
        $done = [];
        if (!is_file($cfgPath)) {
            Fs::ensureDir(dirname($cfgPath), 0755, 'root', 'root');
            $default = dirname(__DIR__, 4) . '/etc/config.json.default';
            $data = @file_get_contents($default);
            if ($data === false) {
                $cli->err("cannot read $default");
                return Cli::EXIT_ERROR;
            }
            Fs::writeAtomic($cfgPath, $data, 0600);
            $done[] = "created $cfgPath";
        } else {
            @chmod($cfgPath, 0600);
        }
        $app = $cli->app();
        $p = $app->paths;
        $dirs = [
            [dirname($p->registryDb()), 0711],
            [$p->dataDir(), 0711],
            [$p->logDir(), 0700],
            [$p->runDir(), 0711],
            [$p->hookDir(), 01777],
            [$p->geoipDir(), 0755],
        ];
        foreach ($dirs as [$dir, $mode]) {
            $existed = is_dir($dir);
            Fs::ensureDir($dir, $mode, 'root', 'root');
            @chmod($dir, $mode);
            if (!$existed) {
                $done[] = sprintf('created %s (%o)', $dir, $mode);
            }
        }
        $reg = $app->registry();
        @chmod($p->registryDb(), 0600);
        $users = [];
        if (!$args->flag('no-sync')) {
            $users = (new Collector($app))->syncRegistry();
            foreach ($users as $u) {
                try {
                    DomainStore::prepareUserDir($p, $u);
                } catch (\Throwable $e) {
                    $cli->err("cannot prepare data directory of $u: " . $e->getMessage());
                }
            }
        }
        $summary = [
            'config' => $cfgPath,
            'registry' => $p->registryDb(),
            'data_dir' => $p->dataDir(),
            'accounts' => count($users),
            'registry_version' => $reg->meta()['schema_version'] ?? null,
            'actions' => $done,
        ];
        $cli->result($summary, static fn (): string => implode("\n", array_merge($done, [
            sprintf('setup complete: config %s, registry %s, %d account(s)', $cfgPath, $p->registryDb(), count($users)),
        ])));
        return Cli::EXIT_OK;
    }
}
