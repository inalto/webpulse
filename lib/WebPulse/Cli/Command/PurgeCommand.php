<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Cli\UsageException;
use WebPulse\Paths;
use WebPulse\Util\Fs;

/**
 * webpulse purge --user U [--domain D] --yes
 */
final class PurgeCommand implements CommandInterface
{
    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'delete the statistics of an account or domain (irreversible)';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse purge --user USER [--domain DOMAIN] --yes

        Deletes the domain database(s). Without --domain every database of the
        account is deleted; when the account no longer exists in cPanel its data
        directory and registry rows are removed too. --yes is mandatory.
        TXT;
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return ['user' => Args::VALUE, 'domain' => Args::VALUE, 'yes' => Args::FLAG];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        $user = $args->opt('user');
        if ($user === null || !Paths::isValidUser($user)) {
            throw new UsageException('--user USER is required');
        }
        $domain = $args->opt('domain');
        if ($domain !== null) {
            $domain = strtolower($domain);
            if (!Paths::isValidDomain($domain)) {
                throw new UsageException("invalid domain '$domain'");
            }
        }
        if (!$args->flag('yes')) {
            throw new UsageException('refusing to delete data without --yes');
        }
        $app = $cli->app();
        $lock = $cli->userLock($user, CollectCommand::USER_LOCK_WAIT);
        if ($lock === null) {
            $cli->err("account $user is locked by another process");
            return Cli::EXIT_ERROR;
        }
        $reg = $app->registry();
        $deleted = [];
        $files = $domain !== null ? [$app->paths->domainDb($user, $domain)] : (glob($app->paths->userDir($user) . '/*.db') ?: []);
        foreach ($files as $f) {
            if (is_file($f) || is_link($f)) {
                @unlink($f);
                @unlink($f . '-journal');
                $deleted[] = basename($f, '.db');
            }
        }
        if ($domain !== null) {
            $reg->deleteSummary($user, $domain);
        } else {
            $reg->deleteSummary($user);
            $acc = $reg->account($user);
            if ($acc !== null && $acc['removed_at'] !== null && !$app->cpanelUsers()->exists($user)) {
                Fs::removeTree($app->paths->userDir($user));
                $reg->forgetAccount($user);
            }
        }
        $lock->release();
        $app->log->info("purged $user" . ($domain !== null ? "/$domain" : ''), ['deleted' => $deleted]);
        $cli->result(['user' => $user, 'deleted' => $deleted], static fn (): string => $deleted === []
            ? 'nothing to delete'
            : 'deleted: ' . implode(', ', $deleted));
        return Cli::EXIT_OK;
    }
}
