<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Cli\Job;
use WebPulse\Cli\UsageException;
use WebPulse\Collect\Collector;
use WebPulse\Http\JobRunner;
use WebPulse\Paths;
use WebPulse\Util\Lock;

/**
 * webpulse collect [--user U [--domain D]] [--hooked] [--job [id]]
 *
 * The global run (no --user) collects every account in its own child
 * process (`collect --user U --child --json`): a fatal error in one account
 * (memory exhausted, say) cannot stop the accounts after it. Each domain is
 * committed transactionally, so a child that dies leaves its checkpoints
 * consistent and the next run resumes from there.
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
          --child       (internal) one account of a global run: skip it when
                        locked, never record a job row

        Without --user every enabled account is processed, each in a child
        process; a second concurrent run exits 0 immediately, and accounts whose
        lock is held are skipped.
        TXT;
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return ['user' => Args::VALUE, 'domain' => Args::VALUE, 'hooked' => Args::FLAG, 'job' => Args::OPTIONAL, 'child' => Args::FLAG];
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
        $child = $args->flag('child');
        if ($child && $user === null) {
            throw new UsageException('--child requires --user');
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
            $lock = $cli->userLock($user, $child ? 0.0 : self::USER_LOCK_WAIT);
            if ($lock === null) {
                if ($child) {
                    $tot['skipped'][] = "$user (locked)";
                    return self::report($cli, $job, $tot, $t0, false);
                }
                $cli->err("account $user is locked by another process");
                $job->finish('skipped', [], 'account locked', true);
                return Cli::EXIT_ERROR;
            }
            $collector->syncRegistry($user);
            self::add($tot, $collector->collectUser($user, $domain));
            $tot['users'] = 1;
            $lock->release();
            if ($child) {
                return self::report($cli, $job, $tot, $t0, false);
            }
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
                $r = self::collectInChild($cli, $u);
                if ($r === null) {
                    $tot['errors']++;
                    continue;
                }
                self::add($tot, $r);
                $tot['users'] += (int) ($r['users'] ?? 0);
                $tot['peak_memory'] = max((int) ($tot['peak_memory'] ?? 0), (int) ($r['peak_memory'] ?? 0));
            }
            $global->release();
        }
        return self::report($cli, $job, $tot, $t0, $tot['lines'] > 0 || $tot['errors'] > 0);
    }

    /**
     * Collects one account in a child process (`collect --user U --child
     * --json`) and returns its JSON totals, or null when the child failed
     * (fatal error, crash, unparsable output) — logged, never fatal here.
     *
     * @return array<string,mixed>|null
     */
    public static function collectInChild(Cli $cli, string $user): ?array
    {
        $app = $cli->app();
        $php = $app->config->get('php_bin');
        if (!is_string($php) || $php === '' || !is_executable($php)) {
            $php = PHP_BINARY !== '' && is_executable(PHP_BINARY) && !preg_match('/(cgi|fpm)/', basename(PHP_BINARY))
                ? PHP_BINARY : JobRunner::phpBinary();
        }
        $argv = [$php, $app->paths->root() . '/bin/webpulse', 'collect', '--user', Paths::assertUser($user), '--child', '--json'];
        if ($cli->quiet()) {
            $argv[] = '--quiet';
        }
        if ($cli->verbose()) {
            $argv[] = '--verbose';
        }
        $cfg = $app->configPath();
        if ($cfg !== null && $cfg !== '' && is_file($cfg)) {
            array_push($argv, '--config', (string) (realpath($cfg) ?: $cfg));
        }
        $stderr = defined('STDERR') ? STDERR : ['file', '/dev/null', 'w'];
        $proc = @proc_open($argv, [0 => ['file', '/dev/null', 'r'], 1 => ['pipe', 'w'], 2 => $stderr], $pipes, getcwd() ?: $app->paths->root());
        if (!is_resource($proc)) {
            $app->log->error("collect $user failed: cannot start a child process");
            return null;
        }
        $out = (string) stream_get_contents($pipes[1]);
        fclose($pipes[1]);
        $code = proc_close($proc);
        $r = json_decode($out, true);
        if ($code !== 0 || !is_array($r)) {
            $app->log->error("collect $user failed: child process exited with code $code", ['output' => \WebPulse\Util\Str::truncate(\WebPulse\Util\Str::clean(trim($out)), 500)]);
            return null;
        }
        return $r;
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
        $tot['peak_memory'] = max((int) ($tot['peak_memory'] ?? 0), memory_get_peak_usage(true)); // children included
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
