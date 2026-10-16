<?php

declare(strict_types=1);

namespace WebPulse\Tests;

/**
 * Minimal assertions; every failure throws \AssertionError with a diff-like message.
 */
final class T
{
    public static function eq(mixed $expected, mixed $actual, string $msg = ''): void
    {
        if ($expected !== $actual) {
            throw new \AssertionError(($msg !== '' ? "$msg\n" : '') . 'expected ' . self::dump($expected) . "\n     got " . self::dump($actual));
        }
    }

    /** Loose numeric / array equality (int vs string numbers). */
    public static function same(mixed $expected, mixed $actual, string $msg = ''): void
    {
        if ($expected != $actual) {
            throw new \AssertionError(($msg !== '' ? "$msg\n" : '') . 'expected ' . self::dump($expected) . "\n     got " . self::dump($actual));
        }
    }

    public static function true(mixed $cond, string $msg = 'expected true'): void
    {
        if ($cond !== true) {
            throw new \AssertionError($msg . ' (got ' . self::dump($cond) . ')');
        }
    }

    public static function false(mixed $cond, string $msg = 'expected false'): void
    {
        if ($cond !== false) {
            throw new \AssertionError($msg . ' (got ' . self::dump($cond) . ')');
        }
    }

    public static function null(mixed $v, string $msg = 'expected null'): void
    {
        if ($v !== null) {
            throw new \AssertionError($msg . ' (got ' . self::dump($v) . ')');
        }
    }

    public static function notNull(mixed $v, string $msg = 'expected a value'): void
    {
        if ($v === null) {
            throw new \AssertionError($msg);
        }
    }

    public static function contains(string $needle, string $haystack, string $msg = ''): void
    {
        if (!str_contains($haystack, $needle)) {
            throw new \AssertionError(($msg !== '' ? "$msg\n" : '') . 'expected to find ' . self::dump($needle) . ' in ' . self::dump(substr($haystack, 0, 2000)));
        }
    }

    public static function greater(int|float $min, int|float $actual, string $msg = ''): void
    {
        if (!($actual > $min)) {
            throw new \AssertionError(($msg !== '' ? "$msg: " : '') . "expected > $min, got $actual");
        }
    }

    public static function less(int|float $max, int|float $actual, string $msg = ''): void
    {
        if (!($actual < $max)) {
            throw new \AssertionError(($msg !== '' ? "$msg: " : '') . "expected < $max, got $actual");
        }
    }

    /** Asserts that $fn throws $class (optionally with $msgPart in the message). */
    public static function throws(string $class, callable $fn, ?string $msgPart = null): \Throwable
    {
        try {
            $fn();
        } catch (\Throwable $e) {
            if (!$e instanceof $class) {
                throw new \AssertionError("expected $class, got " . get_class($e) . ': ' . $e->getMessage());
            }
            if ($msgPart !== null && !str_contains($e->getMessage(), $msgPart)) {
                throw new \AssertionError("exception message '{$e->getMessage()}' does not contain '$msgPart'");
            }
            return $e;
        }
        throw new \AssertionError("expected $class to be thrown");
    }

    public static function dump(mixed $v): string
    {
        $s = json_encode($v, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE | JSON_PRESERVE_ZERO_FRACTION);
        return $s === false ? var_export($v, true) : (strlen($s) > 3000 ? substr($s, 0, 3000) . '…' : $s);
    }
}
