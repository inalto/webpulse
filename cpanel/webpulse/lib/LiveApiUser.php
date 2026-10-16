<?php

declare(strict_types=1);

namespace WebPulse\Cpanel;

/**
 * Resolves the cPanel account name for the current LiveAPI request.
 *
 * Cascade (first valid answer wins):
 *   1. uapi Variables::get_user_information name=user   (array or object result)
 *   2. getenv('REMOTE_USER')
 *   3. posix_getpwuid(posix_geteuid())['name']
 *
 * The result is only ever used to pick /var/webpulse/data/<user>/, so it is
 * validated against the cPanel user name grammar and never taken from request
 * parameters.
 */
final class LiveApiUser
{
    public const PATTERN = '/^[a-z0-9_][a-z0-9_-]{0,31}\z/';

    /**
     * @param object|null $cpanel a LiveAPI `CPANEL` instance (anything with uapi()) or null
     * @throws \RuntimeException when no valid account name can be determined
     */
    public static function resolve(?object $cpanel = null): string
    {
        if ($cpanel !== null && method_exists($cpanel, 'uapi')) {
            try {
                $user = self::fromResult($cpanel->uapi('Variables', 'get_user_information', ['name' => 'user']));
                if ($user !== null) {
                    return $user;
                }
            } catch (\Throwable) {
                // fall through to the next source
            }
        }

        $env = getenv('REMOTE_USER');
        if (is_string($env) && self::valid($env)) {
            return $env;
        }

        if (function_exists('posix_getpwuid') && function_exists('posix_geteuid')) {
            $pw = @posix_getpwuid(posix_geteuid());
            if (is_array($pw) && isset($pw['name']) && self::valid((string) $pw['name'])) {
                return (string) $pw['name'];
            }
        }

        throw new \RuntimeException('Unable to determine the cPanel account name.');
    }

    /**
     * Extract the user name from a uapi() return value. Accepts the array shape
     * (['cpanelresult' => ['result' => ['data' => ['user' => 'x']]]]), the object
     * shape, the bare result and a few variants seen across cPanel versions
     * (data as list, user as a plain string).
     */
    public static function fromResult(mixed $result): ?string
    {
        $result = self::toArray($result);
        if ($result === null) {
            return null;
        }

        $data = null;
        foreach ([
            ['cpanelresult', 'result', 'data'],
            ['result', 'data'],
            ['cpanelresult', 'data'],
            ['data'],
        ] as $path) {
            $node = $result;
            foreach ($path as $key) {
                if (!is_array($node) || !array_key_exists($key, $node)) {
                    $node = null;
                    break;
                }
                $node = $node[$key];
            }
            if ($node !== null) {
                $data = $node;
                break;
            }
        }
        if ($data === null) {
            return null;
        }

        if (is_string($data)) {
            $candidate = $data;
        } elseif (is_array($data) && isset($data['user'])) {
            $candidate = $data['user'];
        } elseif (is_array($data) && isset($data[0]) && is_array($data[0]) && isset($data[0]['user'])) {
            $candidate = $data[0]['user'];
        } else {
            return null;
        }

        return is_string($candidate) && self::valid($candidate) ? $candidate : null;
    }

    public static function valid(string $user): bool
    {
        return preg_match(self::PATTERN, $user) === 1;
    }

    /** Recursively convert objects (stdClass, ArrayObject, JsonSerializable …) to arrays. */
    private static function toArray(mixed $value): ?array
    {
        if (is_array($value)) {
            return array_map(static fn ($v) => is_array($v) || is_object($v) ? (self::toArray($v) ?? []) : $v, $value);
        }
        if (is_object($value)) {
            $decoded = json_decode((string) json_encode($value), true);
            return is_array($decoded) ? $decoded : null;
        }
        return null;
    }
}
