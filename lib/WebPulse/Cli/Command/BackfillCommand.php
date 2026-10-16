<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Cli\Job;
use WebPulse\Cli\UsageException;
use WebPulse\Collect\Collector;
use WebPulse\Paths;

/**
 * webpulse backfill --user U [--domain D] [--archive-dir DIR] [--job [id]]
 */
final class BackfillCommand implements CommandInterface
{
    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'import archived logs (/home/<user>/logs/*.gz), never twice';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse backfill --user USER [--domain DOMAIN] [--archive-dir DIR] [--job [ID]]

          --user U           account whose archives are imported (required)
          --domain D         only archives of this domain
          --archive-dir DIR  read archives from DIR instead of paths.archive_dir
          --job [ID]         record the run in registry.jobs (adopt job ID when given)

        Archives already imported (same path, size and mtime) are skipped; lines
        newer than the start of live collection are not imported again.
        TXT;
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return ['user' => Args::VALUE, 'domain' => Args::VALUE, 'archive-dir' => Args::VALUE, 'job' => Args::OPTIONAL];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        $user = $args->opt('user');
        if ($user === null) {
            throw new UsageException('--user is required');
        }
        if (!Paths::isValidUser($user)) {
            throw new UsageException("invalid user name '$user'");
        }
        $domain = $args->opt('domain');
        if ($domain !== null && !Paths::isValidDomain(strtolower($domain))) {
            throw new UsageException("invalid domain '$domain'");
        }
        $domain = $domain === null ? null : strtolower($domain);
        $dir = $args->opt('archive-dir');
        if ($dir !== null && !is_dir($dir)) {
            throw new UsageException("archive directory $dir does not exist");
        }
        $app = $cli->app();
        $t0 = microtime(true);
        $job = new Job($cli, 'backfill', $user . ($domain !== null ? '/' . $domain : ''), $args);
        $lock = $cli->userLock($user, CollectCommand::USER_LOCK_WAIT);
        if ($lock === null) {
            $job->finish('skipped', [], 'account locked', true);
            $cli->err("account $user is locked by another process");
            return Cli::EXIT_ERROR;
        }
        $collector = new Collector($app);
        $collector->syncRegistry($user);
        $tot = $collector->backfillUser($user, $domain, $dir);
        $lock->release();
        $elapsed = microtime(true) - $t0;
        $tot['elapsed'] = round($elapsed, 3);
        $job->finish($tot['errors'] > 0 ? 'error' : 'ok', $tot, $tot['errors'] > 0 ? $tot['errors'] . ' error(s), see webpulse.log' : null, true);
        $tot['job_id'] = $job->id();
        $cli->result($tot, static fn (): string => sprintf(
            'backfilled %d archive(s) of %d domain(s): %d line(s) read, %d imported, %d archive(s) already imported, %d error(s) in %.2fs',
            $tot['files'], $tot['domains'], $tot['lines'], $tot['imported'], $tot['skipped_files'], $tot['errors'], $elapsed
        ));
        return Cli::EXIT_OK;
    }
}
