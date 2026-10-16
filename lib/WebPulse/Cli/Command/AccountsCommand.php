<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Cli\UsageException;
use WebPulse\Collect\Collector;
use WebPulse\Paths;

/**
 * webpulse accounts list|enable|disable|reset <user>
 */
final class AccountsCommand implements CommandInterface
{
    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'list accounts or switch collection per account';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse accounts list [--owner RESELLER] [--q TEXT]
               webpulse accounts enable|disable|reset USER [--timezone TZ]

          enable / disable  explicit override for the account
          reset             back to the server default (default_enabled)
          --timezone TZ     also set the account timezone ("" = server timezone)
        TXT;
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return ['owner' => Args::VALUE, 'q' => Args::VALUE, 'timezone' => Args::VALUE, 'no-sync' => Args::FLAG];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        $action = $args->arg(0) ?? 'list';
        $app = $cli->app();
        $reg = $app->registry();
        if ($action === 'list') {
            if (!$args->flag('no-sync')) {
                (new Collector($app))->syncRegistry();
            }
            $rows = $reg->accounts($args->opt('owner'), $args->opt('q'));
            $cli->result(['accounts' => $rows], static fn (): string => $rows === [] ? 'no accounts' : Cli::table(
                ['USER', 'OWNER', 'ENABLED', 'OVERRIDE', 'DOMAINS', 'TIMEZONE', 'REMOVED'],
                array_map(static fn (array $r): array => [
                    $r['user'], $r['owner'], $r['enabled'], $r['enabled_override'] === null ? 'default' : ($r['enabled_override'] ? 'on' : 'off'),
                    $r['domains'], $r['timezone'], $r['removed_at'] === null ? '' : date('Y-m-d', $r['removed_at']),
                ], $rows)
            ));
            return Cli::EXIT_OK;
        }
        if (!in_array($action, ['enable', 'disable', 'reset'], true)) {
            throw new UsageException("unknown action '$action'");
        }
        $user = $args->arg(1);
        if ($user === null || !Paths::isValidUser($user)) {
            throw new UsageException('a valid user name is required');
        }
        if (!$app->cpanelUsers()->exists($user) && $reg->account($user) === null) {
            $cli->err("warning: $user is not a known cPanel account");
        }
        $tz = $args->opt('timezone');
        $reg->setAccount($user, match ($action) {
            'enable' => true,
            'disable' => false,
            default => null,
        }, $tz);
        $row = $reg->account($user);
        $cli->result(['account' => $row], static fn (): string => sprintf('%s: collection %s (%s)', $user, $row['enabled'] ? 'enabled' : 'disabled',
            $row['enabled_override'] === null ? 'server default' : 'override'));
        return Cli::EXIT_OK;
    }
}
