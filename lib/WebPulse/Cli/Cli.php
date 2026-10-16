<?php

declare(strict_types=1);

namespace WebPulse\Cli;

use WebPulse\App;
use WebPulse\Util\Json;
use WebPulse\Util\Lock;
use WebPulse\Util\Str;

/**
 * `webpulse` dispatcher. Global flags (anywhere on the line):
 *   --config <file>  --verbose|-v  --quiet|-q  --json  --help|-h  --version
 * Exit codes: 0 ok, 1 error, 2 usage.
 */
final class Cli
{
    public const EXIT_OK = 0;
    public const EXIT_ERROR = 1;
    public const EXIT_USAGE = 2;

    private ?App $app = null;
    private ?string $configPath = null;
    private bool $verbose = false;
    private bool $quiet = false;
    private bool $json = false;
    /** @var resource */
    private $stdout;
    /** @var resource */
    private $stderr;

    /**
     * @param resource|null $stdout
     * @param resource|null $stderr
     */
    public function __construct($stdout = null, $stderr = null)
    {
        $this->stdout = $stdout ?? STDOUT;
        $this->stderr = $stderr ?? STDERR;
    }

    /** @param string[] $argv full argv (argv[0] = script) */
    public static function main(array $argv): int
    {
        return (new self())->dispatch(array_slice($argv, 1));
    }

    /**
     * @param string[] $args argv without the script name
     */
    public function dispatch(array $args): int
    {
        $rest = [];
        $help = false;
        $n = count($args);
        for ($i = 0; $i < $n; $i++) {
            $a = $args[$i];
            if ($a === '--') {
                array_push($rest, ...array_slice($args, $i));
                break;
            }
            switch (true) {
                case $a === '--config':
                    if ($i + 1 >= $n) {
                        return $this->usageError('option --config requires a value');
                    }
                    $this->configPath = $args[++$i];
                    break;
                case str_starts_with($a, '--config='):
                    $this->configPath = substr($a, 9);
                    break;
                case $a === '--verbose' || $a === '-v':
                    $this->verbose = true;
                    break;
                case $a === '--quiet' || $a === '-q':
                    $this->quiet = true;
                    break;
                case $a === '--json':
                    $this->json = true;
                    break;
                case $a === '--help' || $a === '-h':
                    $help = true;
                    break;
                case $a === '--version':
                    $rest = ['version'];
                    break 2;
                default:
                    $rest[] = $a;
            }
        }
        $name = $rest[0] ?? null;
        if ($name === null || $name === 'help') {
            $target = $rest[1] ?? null;
            if ($target !== null && ($cls = self::commandClass($target)) !== null) {
                $this->out($cls::usage());
                return self::EXIT_OK;
            }
            $this->out($this->help());
            return $name === null && !$help ? self::EXIT_USAGE : self::EXIT_OK;
        }
        if ($name === 'version') {
            $this->out('webpulse ' . $this->readVersion());
            return self::EXIT_OK;
        }
        $cls = self::commandClass($name);
        if ($cls === null) {
            return $this->usageError("unknown command '$name'\n\n" . $this->help());
        }
        if ($help) {
            $this->out($cls::usage());
            return self::EXIT_OK;
        }
        try {
            $parsed = Args::parse(array_slice($rest, 1), $cls::options());
            /** @var CommandInterface $cmd */
            $cmd = new $cls();
            return $cmd->run($this, $parsed);
        } catch (UsageException $e) {
            return $this->usageError($e->getMessage() . "\n\n" . $cls::usage());
        } catch (\Throwable $e) {
            $this->err('error: ' . $e->getMessage());
            if ($this->app !== null) {
                $this->app->log->error("$name failed: " . $e->getMessage(), ['at' => $e->getFile() . ':' . $e->getLine()]);
            }
            if ($this->verbose) {
                $this->err($e->getTraceAsString());
            }
            return self::EXIT_ERROR;
        }
    }

    /**
     * Class implementing a command name, or null.
     *
     * @return class-string<CommandInterface>|null
     */
    public static function commandClass(string $name): ?string
    {
        if (!preg_match('/^[a-z][a-z0-9-]{0,40}$/', $name)) {
            return null;
        }
        $cls = 'WebPulse\\Cli\\Command\\' . Str::studly($name) . 'Command';
        if (!class_exists($cls) || !is_subclass_of($cls, CommandInterface::class)) {
            return null;
        }
        return $cls;
    }

    /**
     * Every available command (scans Cli/Command/*Command.php, so commands
     * added by other packages show up automatically).
     *
     * @return array<string,class-string<CommandInterface>> name => class
     */
    public static function commands(): array
    {
        $out = [];
        foreach (glob(__DIR__ . '/Command/*Command.php') ?: [] as $f) {
            $studly = substr(basename($f, '.php'), 0, -7);
            $name = strtolower((string) preg_replace('/(?<!^)[A-Z]/', '-$0', $studly));
            $cls = self::commandClass($name);
            if ($cls !== null) {
                $out[$name] = $cls;
            }
        }
        ksort($out);
        return $out;
    }

    private function help(): string
    {
        $lines = ['WebPulse ' . $this->readVersion() . ' — log-based web analytics for cPanel & WHM', '',
            'Usage: webpulse <command> [options]', '', 'Commands:'];
        foreach (self::commands() as $name => $cls) {
            $lines[] = sprintf('  %-14s %s', $name, $cls::summary());
        }
        $lines[] = '';
        $lines[] = 'Global options: --config <file>  --verbose  --quiet  --json  --help  --version';
        $lines[] = 'Run "webpulse <command> --help" for the options of a command.';
        return implode("\n", $lines);
    }

    private function readVersion(): string
    {
        $v = @file_get_contents(dirname(__DIR__, 3) . '/VERSION');
        return $v === false ? '0.0.0' : trim($v);
    }

    private function usageError(string $msg): int
    {
        $this->err($msg);
        return self::EXIT_USAGE;
    }

    /** The application (booted on first use with --config / $WEBPULSE_CONFIG). */
    public function app(): App
    {
        if ($this->app === null) {
            $this->app = App::boot($this->configPath);
            $this->app->log->setVerbose($this->verbose);
            $this->app->log->setStderrErrors(!$this->quiet && !$this->verbose);
        }
        return $this->app;
    }

    /** Config path given with --config (null = environment / default). */
    public function configOption(): ?string
    {
        return $this->configPath;
    }

    /** True with --verbose. */
    public function verbose(): bool
    {
        return $this->verbose;
    }

    /** True with --quiet. */
    public function quiet(): bool
    {
        return $this->quiet;
    }

    /** True with --json. */
    public function json(): bool
    {
        return $this->json;
    }

    /** Informational output (suppressed by --quiet and --json). */
    public function out(string $msg): void
    {
        if ($this->quiet) {
            return;
        }
        fwrite($this->stdout, $msg . "\n");
    }

    /** Notice (suppressed by --quiet, goes to stdout). */
    public function info(string $msg): void
    {
        if ($this->quiet || $this->json) {
            return;
        }
        fwrite($this->stdout, $msg . "\n");
    }

    /** Error output (always, stderr). */
    public function err(string $msg): void
    {
        fwrite($this->stderr, $msg . "\n");
    }

    /**
     * Prints a result: JSON with --json (always, even with --quiet), else
     * the human rendering (suppressed by --quiet).
     *
     * @param \Closure():string|null $human
     */
    public function result(mixed $data, ?\Closure $human = null): void
    {
        if ($this->json) {
            fwrite($this->stdout, Json::encode($data, true) . "\n");
            return;
        }
        if ($this->quiet) {
            return;
        }
        fwrite($this->stdout, ($human !== null ? $human() : Json::encode($data, true)) . "\n");
    }

    /**
     * Renders rows as an aligned text table.
     *
     * @param string[] $headers
     * @param array<int,array<int,mixed>> $rows
     */
    public static function table(array $headers, array $rows): string
    {
        $w = array_map('strlen', $headers);
        $cells = [];
        foreach ($rows as $r) {
            $line = [];
            foreach (array_values($r) as $i => $c) {
                $s = is_bool($c) ? ($c ? 'yes' : 'no') : ($c === null ? '-' : (string) $c);
                $line[] = $s;
                $w[$i] = max($w[$i] ?? 0, strlen($s));
            }
            $cells[] = $line;
        }
        $fmt = static function (array $line) use ($w): string {
            $o = [];
            foreach ($line as $i => $s) {
                $o[] = str_pad($s, $w[$i]);
            }
            return rtrim(implode('  ', $o));
        };
        $out = [$fmt($headers), $fmt(array_map(static fn ($x) => str_repeat('-', $x), $w))];
        foreach ($cells as $l) {
            $out[] = $fmt($l);
        }
        return implode("\n", $out);
    }

    /** Lock file path in the run directory. */
    public function lockPath(string $name): string
    {
        return $this->app()->paths->runDir() . '/' . $name . '.lock';
    }

    /** Takes the per-user lock (waiting up to $wait seconds); null when busy. */
    public function userLock(string $user, float $wait): ?Lock
    {
        return Lock::acquire($this->lockPath('user-' . $user), $wait);
    }
}
