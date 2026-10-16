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
use WebPulse\Util\Lock;

/**
 * webpulse collect [--user U [--domain D]] [--hooked] [--job [id]]
 */
final class CollectCommand implements CommandInterface
{
    /** Seconds an explicit --user / hooked run waits for the user lock. */
    public const USER_LOCK_WAIT = 300.0;

    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'tail the access logs and update the statistics (cron runs this)';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse collect [--user USER [--domain DOMAIN]] [--hooked] [--job [ID]]

          --user U      only this account (waits for its lock)
          --domain D    only this domain of --user
          --hooked      process the accounts flagged by the Stats::RunUser hook
                        (<run_dir>/hook/<user>) and remove the flags
          --job [ID]    record the run in registry.jobs (adopt job ID when given)

        Without --user every enabled account is processed; a second concurrent
        run exits 0 immediately, and accounts whose lock is held are skipped.
        TXT;
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return ['user' => Args::VALUE, 'domain' => Args::VALUE, 'hooked' => Args::FLAG, 'job' => Args::OPTIONAL];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        if ($args->positional() !== []) {
            throw new UsageException('unexpected argument ' . $args->positional()[0]);
        }
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
        $domain = $domain === null ? null : strtolower($domain);
        if ($args->flag('hooked')) {
            if ($user !== null) {
                throw new UsageException('--hooked cannot be combined with --user');
            }
            return self::hooked($cli, $args);
        }

        $app = $cli->app();
        $t0 = microtime(true);
        $scope = $user === null ? 'all' : $user . ($domain !== null ? '/' . $domain : '');
        $job = new Job($cli, 'collect', $scope, $args);
        $collector = new Collector($app);
        $tot = ['users' => 0, 'domains' => 0, 'files' => 0, 'lines' => 0, 'bad' => 0, 'ignored' => 0, 'errors' => 0, 'skipped' => []];

        if ($user !== null) {
            $lock = $cli->userLock($user, self::USER_LOCK_WAIT);
            if ($lock === null) {
                $cli->err("account $user is locked by another process");
                $job->finish('skipped', [], 'account locked', true);
                return Cli::EXIT_ERROR;
            }
            $collector->syncRegistry($user);
            self::add($tot, $collector->collectUser($user, $domain));
            $tot['users'] = 1;
            $lock->release();
        } else {
            $global = Lock::acquire($cli->lockPath('collect'));
            if ($global === null) {
                $cli->info('another collect is already running; nothing to do');
                $job->finish('skipped', [], 'another collect is running');
                return Cli::EXIT_OK;
            }
            $users = $collector->syncRegistry();
            foreach ($users as $u) {
                $acc = $app->registry()->account($u);
                if ($acc !== null && !$acc['enabled'] && !self::hasDomainOverride($app, $u)) {
                    continue;
                }
                $lock = $cli->userLock($u, 0);
                if ($lock === null) {
                    $tot['skipped'][] = "$u (locked)";
                    continue;
                }
                try {
                    self::add($tot, $collector->collectUser($u));
                    $tot['users']++;
                } catch (\Throwable $e) {
                    $tot['errors']++;
                    $app->log->error("collect $u failed: " . $e->getMessage());
                } finally {
                    $lock->release();
                }
            }
            $global->release();
        }
        return self::report($cli, $job, $tot, $t0, $tot['lines'] > 0 || $tot['errors'] > 0);
    }

    /**
     * collect --hooked: drain the accounts flagged by the Stats::RunUser hook.
     */
    public static function hooked(Cli $cli, Args $args): int
    {
        $app = $cli->app();
        $dir = $app->paths->hookDir();
        $flags = [];
        foreach (@scandir($dir) ?: [] as $f) {
            if ($f[0] === '.' || !Paths::isValidUser($f)) {
                continue;
            }
            $st = @lstat($dir . '/' . $f);
            if ($st === false || ($st['mode'] & 0170000) !== 0100000) {
                @unlink($dir . '/' . $f);
                continue;
            }
            $flags[$f] = (int) $st['mtime'];
        }
        if ($flags === []) {
            if ($args->has('job')) {
                (new Job($cli, 'hook', null, $args))->finish('skipped', [], 'no hook flags');
            }
            $cli->result(['users' => 0, 'lines' => 0], static fn () => 'no hook flags');
            return Cli::EXIT_OK;
        }
        $global = Lock::acquire($cli->lockPath('hooked'));
        if ($global === null) {
            if ($args->has('job')) {
                (new Job($cli, 'hook', null, $args))->finish('skipped', [], 'another hooked collect is running');
            }
            $cli->info('another hooked collect is running');
            return Cli::EXIT_OK;
        }
        $t0 = microtime(true);
        $job = new Job($cli, 'hook', implode(',', array_keys($flags)), $args);
        $collector = new Collector($app);
        $tot = ['users' => 0, 'domains' => 0, 'files' => 0, 'lines' => 0, 'bad' => 0, 'ignored' => 0, 'errors' => 0, 'skipped' => []];
        foreach ($flags as $u => $mtime) {
            if (!$app->cpanelUsers()->exists($u)) {
                @unlink($dir . '/' . $u);
                continue;
            }
            $lock = $cli->userLock($u, self::USER_LOCK_WAIT);
            if ($lock === null) {
                $tot['skipped'][] = "$u (locked)";
                continue;
            }
            try {
                $collector->syncRegistry($u);
                self::add($tot, $collector->collectUser($u));
                $tot['users']++;
                clearstatcache(true, $dir . '/' . $u);
                $st = @lstat($dir . '/' . $u);
                if ($st !== false && (int) $st['mtime'] === $mtime) {
                    @unlink($dir . '/' . $u);
                }
            } catch (\Throwable $e) {
                $tot['errors']++;
                $app->log->error("hooked collect $u failed: " . $e->getMessage());
            } finally {
                $lock->release();
            }
        }
        $global->release();
        return self::report($cli, $job, $tot, $t0, true);
    }

    private static function hasDomainOverride(\WebPulse\App $app, string $user): bool
    {
        foreach ($app->registry()->domains($user) as $d) {
            if ($d['enabled']) {
                return true;
            }
        }
        return false;
    }

    /**
     * @param array<string,mixed> $tot
     * @param array<string,mixed> $r
     */
    private static function add(array &$tot, array $r): void
    {
        foreach (['domains', 'files', 'lines', 'bad', 'ignored', 'errors'] as $k) {
            $tot[$k] += (int) ($r[$k] ?? 0);
        }
        foreach ($r['skipped'] ?? [] as $s) {
            $tot['skipped'][] = $s;
        }
    }

    /** @param array<string,mixed> $tot */
    private static function report(Cli $cli, Job $job, array $tot, float $t0, bool $record): int
    {
        $elapsed = microtime(true) - $t0;
        $tot['elapsed'] = round($elapsed, 3);
        $tot['lines_per_sec'] = $elapsed > 0 ? (int) round($tot['lines'] / $elapsed) : 0;
        $tot['peak_memory'] = memory_get_peak_usage(true);
        $tot['job_id'] = $job->id();
        $status = $tot['errors'] > 0 ? 'error' : 'ok';
        $job->finish($status, $tot, $tot['errors'] > 0 ? $tot['errors'] . ' error(s), see webpulse.log' : null, $record);
        $tot['job_id'] = $job->id();
        $cli->result($tot, static fn (): string => sprintf(
            'collected %d line(s) from %d file(s) in %d domain(s) of %d account(s) in %.2fs (%d lines/s, %d bad, %d ignored, %d error(s))%s',
            $tot['lines'], $tot['files'], $tot['domains'], $tot['users'], $elapsed, $tot['lines_per_sec'], $tot['bad'],
            $tot['ignored'], $tot['errors'], $tot['skipped'] !== [] ? "\nskipped: " . implode(', ', $tot['skipped']) : ''
        ));
        return Cli::EXIT_OK;
    }
}
