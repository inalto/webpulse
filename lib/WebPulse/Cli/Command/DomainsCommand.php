<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Cli\UsageException;
use WebPulse\Paths;

/**
 * webpulse domains list [user] | enable|disable|reset <user> <domain>
 */
final class DomainsCommand implements CommandInterface
{
    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'list domains or switch collection per domain';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse domains list [USER]
               webpulse domains enable|disable|reset USER DOMAIN

        A domain override wins over the account setting, except when the account
        is explicitly disabled (which turns off all of its domains).
        TXT;
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return [];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        $action = $args->arg(0) ?? 'list';
        $app = $cli->app();
        $reg = $app->registry();
        if ($action === 'list') {
            $user = $args->arg(1);
            if ($user !== null && !Paths::isValidUser($user)) {
                throw new UsageException("invalid user name '$user'");
            }
            $rows = $reg->domains($user);
            foreach ($rows as &$r) {
                $r['has_data'] = is_file($app->paths->domainDb($r['user'], $r['domain']));
            }
            unset($r);
            $cli->result(['domains' => $rows], static fn (): string => $rows === [] ? 'no domains' : Cli::table(
                ['USER', 'DOMAIN', 'KIND', 'ENABLED', 'OVERRIDE', 'DATA', 'REMOVED'],
                array_map(static fn (array $r): array => [
                    $r['user'], $r['domain'], $r['kind'], $r['enabled'],
                    $r['enabled_override'] === null ? 'account' : ($r['enabled_override'] ? 'on' : 'off'),
                    $r['has_data'], $r['removed_at'] === null ? '' : date('Y-m-d', $r['removed_at']),
                ], $rows)
            ));
            return Cli::EXIT_OK;
        }
        if (!in_array($action, ['enable', 'disable', 'reset'], true)) {
            throw new UsageException("unknown action '$action'");
        }
        $user = $args->arg(1);
        $domain = $args->arg(2) === null ? null : strtolower((string) $args->arg(2));
        if ($user === null || !Paths::isValidUser($user) || $domain === null || !Paths::isValidDomain($domain)) {
            throw new UsageException('USER and DOMAIN are required');
        }
        $reg->setDomain($user, $domain, match ($action) {
            'enable' => true,
            'disable' => false,
            default => null,
        });
        $on = $reg->isEnabled($user, $domain);
        $cli->result(['user' => $user, 'domain' => $domain, 'enabled' => $on], static fn (): string => sprintf('%s/%s: collection %s', $user, $domain, $on ? 'enabled' : 'disabled'));
        return Cli::EXIT_OK;
    }
}
