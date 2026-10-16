<?php

declare(strict_types=1);

namespace WebPulse\Cli;

/**
 * Parsed command-line arguments of one sub-command:
 * `--name value`, `--name=value`, `--flag`, positionals; `--` ends options.
 */
final class Args
{
    public const VALUE = 'value';
    public const FLAG = 'flag';
    public const OPTIONAL = 'optional';

    /**
     * @param string[] $positional
     * @param array<string,string|bool> $options
     */
    public function __construct(private array $positional = [], private array $options = [])
    {
    }

    /**
     * @param string[] $argv
     * @param array<string,string> $spec name => VALUE|FLAG|OPTIONAL
     * @throws UsageException
     */
    public static function parse(array $argv, array $spec): self
    {
        $pos = [];
        $opts = [];
        $n = count($argv);
        for ($i = 0; $i < $n; $i++) {
            $a = $argv[$i];
            if ($a === '--') {
                array_push($pos, ...array_slice($argv, $i + 1));
                break;
            }
            if (strncmp($a, '--', 2) !== 0 || $a === '-') {
                $pos[] = $a;
                continue;
            }
            $name = substr($a, 2);
            $value = null;
            if (str_contains($name, '=')) {
                [$name, $value] = explode('=', $name, 2);
            }
            $kind = $spec[$name] ?? null;
            if ($kind === null) {
                throw new UsageException("unknown option --$name");
            }
            if ($kind === self::FLAG) {
                if ($value !== null) {
                    throw new UsageException("option --$name takes no value");
                }
                $opts[$name] = true;
            } elseif ($kind === self::VALUE) {
                if ($value === null) {
                    if ($i + 1 >= $n) {
                        throw new UsageException("option --$name requires a value");
                    }
                    $value = $argv[++$i];
                }
                $opts[$name] = $value;
            } else { // OPTIONAL: consumes the next token only when it is numeric
                if ($value === null && $i + 1 < $n && ctype_digit($argv[$i + 1])) {
                    $value = $argv[++$i];
                }
                $opts[$name] = $value ?? true;
            }
        }
        return new self($pos, $opts);
    }

    /** @return string[] */
    public function positional(): array
    {
        return $this->positional;
    }

    /** Positional argument $i (0-based) or null. */
    public function arg(int $i): ?string
    {
        return $this->positional[$i] ?? null;
    }

    /** True when option --$name was given (with or without value). */
    public function has(string $name): bool
    {
        return isset($this->options[$name]);
    }

    /** Boolean value of a flag option. */
    public function flag(string $name): bool
    {
        return isset($this->options[$name]) && $this->options[$name] !== false;
    }

    /** String value of an option (null when absent or given without value). */
    public function opt(string $name): ?string
    {
        $v = $this->options[$name] ?? null;
        return is_string($v) ? $v : null;
    }

    /** @return array<string,string|bool> */
    public function all(): array
    {
        return $this->options;
    }
}
