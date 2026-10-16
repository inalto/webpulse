<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Cli\Job;
use WebPulse\Cli\UsageException;
use WebPulse\Paths;
use WebPulse\Rollup\Rollup;
use WebPulse\Util\Lock;

/**
 * webpulse rollup [--user U [--domain D]] [--job [id]]
 */
final class RollupCommand implements CommandInterface
{
    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'nightly compaction: close visits, cap dimensions, retention, vacuum';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse rollup [--user USER [--domain DOMAIN]] [--job [ID]]

        Closes stale visits, folds the tail of high-cardinality dimensions into
        "(other)" (max_rows_per_dim per day), purges raw visits/events older than
        raw_retention_days, prunes visitor_day and salts, refreshes the WHM summary,
        runs PRAGMA optimize and VACUUM when more than 25 % of the pages are free.
        TXT;
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return ['user' => Args::VALUE, 'domain' => Args::VALUE, 'job' => Args::OPTIONAL];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        $user = $args->opt('user');
        $domain = $args->opt('domain');
        if ($domain !== null && $user === null) {
            throw new UsageException('--domain requires --user');
        }
        if ($user !== null && !Paths::isValidUser($user)) {
            throw new UsageException("invalid user name '$user'");
        }
        if ($domain !== null && !Paths::isValidDomain(strtolower($domain))) {
            throw new UsageException("invalid domain '$domain'");
        }
        $app = $cli->app();
        $global = Lock::acquire($cli->lockPath('rollup'));
        if ($global === null) {
            if ($args->has('job')) {
                (new Job($cli, 'rollup', null, $args))->finish('skipped', [], 'another rollup is running');
            }
            $cli->info('another rollup is running');
            return Cli::EXIT_OK;
        }
        $t0 = microtime(true);
        $job = new Job($cli, 'rollup', $user === null ? 'all' : $user . ($domain !== null ? '/' . strtolower($domain) : ''), $args);
        $rollup = new Rollup($app);
        $tot = ['domains' => 0, 'closed' => 0, 'folded' => 0, 'purged_visits' => 0, 'purged_events' => 0, 'pruned_visitor_day' => 0, 'vacuum' => 0, 'errors' => 0, 'salts_pruned' => 0];
        $users = $user !== null ? [$user] : $rollup->users();
        foreach ($users as $u) {
            $lock = $cli->userLock($u, 600);
            if ($lock === null) {
                $tot['errors']++;
                $app->log->warn("rollup: account $u stayed locked, skipped");
                continue;
            }
            foreach ($rollup->domainsWithData($u) as $d) {
                if ($domain !== null && $d !== strtolower($domain)) {
                    continue;
                }
                try {
                    foreach ($rollup->rollupDomain($u, $d) as $k => $v) {
                        $tot[$k] += $v;
                    }
                    $tot['domains']++;
                } catch (\Throwable $e) {
                    $tot['errors']++;
                    $app->log->error("rollup $u/$d failed: " . $e->getMessage());
                }
            }
            $lock->release();
        }
        try {
            $tot['salts_pruned'] = $rollup->pruneSalts();
        } catch (\Throwable $e) {
            $tot['errors']++;
            $app->log->error('salt pruning failed: ' . $e->getMessage());
        }
        $global->release();
        $tot['elapsed'] = round(microtime(true) - $t0, 3);
        $job->finish($tot['errors'] > 0 ? 'error' : 'ok', ['domains' => $tot['domains']], $tot['errors'] > 0 ? $tot['errors'] . ' error(s)' : null, true);
        $cli->result($tot, static fn (): string => sprintf(
            'rollup of %d domain(s): %d visit(s) closed, %d row(s) folded, %d visit(s) and %d event(s) purged, %d vacuum(s), %d error(s) in %.2fs',
            $tot['domains'], $tot['closed'], $tot['folded'], $tot['purged_visits'], $tot['purged_events'], $tot['vacuum'], $tot['errors'], $tot['elapsed']
        ));
        return Cli::EXIT_OK;
    }
}
