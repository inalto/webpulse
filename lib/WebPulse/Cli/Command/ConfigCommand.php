<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Cli\UsageException;
use WebPulse\Config;
use WebPulse\Util\Json;

/**
 * webpulse config get [key] | set <key> <value> | unset <key>
 */
final class ConfigCommand implements CommandInterface
{
    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'show the effective configuration or change a run-time setting';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return "Usage: webpulse config get [KEY]\n"
            . "       webpulse config set KEY VALUE     (VALUE is JSON, or a plain string)\n"
            . "       webpulse config unset KEY\n\n"
            . "Run-time settings are stored in the registry and override config.json:\n  "
            . implode(', ', Config::EDITABLE) . "\nOther keys (paths.*) are edited in config.json.";
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return [];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        $action = $args->arg(0) ?? 'get';
        $app = $cli->app();
        $key = $args->arg(1);
        switch ($action) {
            case 'get':
                if ($key === null) {
                    $data = $app->config->effective();
                    $cli->result($data, static fn (): string => Json::encode($data, true));
                    return Cli::EXIT_OK;
                }
                $missing = new \stdClass();
                $v = $app->config->get($key, $missing);
                if ($v === $missing) {
                    $cli->err("unknown key '$key'");
                    return Cli::EXIT_ERROR;
                }
                $cli->result($v, static fn (): string => is_string($v) ? $v : Json::encode($v, true));
                return Cli::EXIT_OK;
            case 'set':
                $raw = $args->arg(2);
                if ($key === null || $raw === null) {
                    throw new UsageException('KEY and VALUE are required');
                }
                try {
                    $value = Json::decode($raw);
                } catch (\JsonException) {
                    $value = $raw;
                }
                try {
                    $app->config->setOverride($key, $value);
                } catch (\InvalidArgumentException $e) {
                    $cli->err($e->getMessage());
                    return Cli::EXIT_USAGE;
                }
                $nv = $app->config->get($key);
                $cli->result([$key => $nv], static fn (): string => "$key = " . Json::encode($nv));
                return Cli::EXIT_OK;
            case 'unset':
                if ($key === null) {
                    throw new UsageException('KEY is required');
                }
                $app->config->unsetOverride($key);
                $nv = $app->config->get($key);
                $cli->result([$key => $nv], static fn (): string => "$key = " . Json::encode($nv) . ' (from config.json / default)');
                return Cli::EXIT_OK;
            default:
                throw new UsageException("unknown action '$action'");
        }
    }
}
