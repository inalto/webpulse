<?php

declare(strict_types=1);

namespace WebPulse\Util;

/**
 * JSON helpers with exceptions and stable flags.
 */
final class Json
{
    public const FLAGS = JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE;

    /** Encodes $v (throws \JsonException). */
    public static function encode(mixed $v, bool $pretty = false): string
    {
        return json_encode($v, self::FLAGS | JSON_THROW_ON_ERROR | ($pretty ? JSON_PRETTY_PRINT : 0));
    }

    /** Decodes to arrays (throws \JsonException). */
    public static function decode(string $s): mixed
    {
        return json_decode($s, true, 64, JSON_THROW_ON_ERROR);
    }
}
