<?php

declare(strict_types=1);

namespace WebPulse\Whm;

/**
 * Edge checks of the WHM CGI front-ends (testable without cPanel).
 *
 *  - principal(): REMOTE_USER -> Principal (root = admin, reseller = scoped, else null)
 *  - mutationAllowed(): defence in depth in front of Api's own POST + header check
 */
final class Edge
{
    public const USER_PATTERN = '/^[a-z0-9_][a-z0-9_-]{0,31}\z/';

    /** Action names that change state (anything else is a read). */
    private const MUTATING = '/^(?:admin\.)?(?:set_|save_|run$|purge$|delete|reset|enable|disable)/';

    /**
     * @param object $users WebPulse\Discovery\CpanelUsers (anything with all(): array<string, array{owner?: ?string}>)
     * @return \WebPulse\Http\Principal|null null = forbidden
     */
    public static function principal(?string $remoteUser, object $users): ?object
    {
        $who = $remoteUser ?? '';
        if ($who === '') {
            return null;
        }
        if ($who === 'root') {
            return new \WebPulse\Http\Principal(\WebPulse\Http\Principal::ADMIN, 'root');
        }
        if (preg_match(self::USER_PATTERN, $who) !== 1) {
            return null;
        }

        $owned = [];
        foreach ($users->all() as $account => $info) {
            if (is_array($info) && ($info['owner'] ?? null) === $who) {
                $owned[] = (string) $account;
            }
        }
        if ($owned === []) {
            return null; // not a reseller with accounts of its own
        }
        sort($owned, SORT_STRING);

        return new \WebPulse\Http\Principal(\WebPulse\Http\Principal::RESELLER, $who, $owned);
    }

    public static function isMutating(string $action): bool
    {
        return preg_match(self::MUTATING, $action) === 1;
    }

    /**
     * Mutating actions need POST and X-WebPulse-Request: 1. Anything that is not
     * a plain GET/HEAD read must satisfy the same rule.
     */
    public static function mutationAllowed(string $action, string $method, ?string $requestHeader): bool
    {
        $method = strtoupper($method);
        $readOnlyMethod = $method === 'GET' || $method === 'HEAD';
        if (!$readOnlyMethod || self::isMutating($action)) {
            return $method === 'POST' && $requestHeader === '1';
        }
        return true;
    }

    /**
     * Build the Request from the CGI environment. The php CLI binary used for
     * CGIs does not fill $_GET and php://input, so parse QUERY_STRING and read
     * the POST body from STDIN (capped at 1 MiB) ourselves.
     *
     * @param array<string, mixed>|null $env defaults to getenv()
     */
    public static function request(?array $env = null, ?string $stdin = null): \WebPulse\Http\Request
    {
        $env ??= getenv();
        $method = strtoupper((string) ($env['REQUEST_METHOD'] ?? 'GET'));

        $query = [];
        parse_str((string) ($env['QUERY_STRING'] ?? ''), $query);

        $headers = [];
        foreach ($env as $key => $value) {
            if (!is_string($key) || !is_string($value)) {
                continue;
            }
            if (str_starts_with($key, 'HTTP_')) {
                $name = str_replace(' ', '-', ucwords(strtolower(str_replace('_', ' ', substr($key, 5)))));
                $headers[$name] = $value;
            } elseif ($key === 'CONTENT_TYPE') {
                $headers['Content-Type'] = $value;
            }
        }

        $body = null;
        if ($method !== 'GET' && $method !== 'HEAD') {
            $length = min((int) ($env['CONTENT_LENGTH'] ?? 0), 1048576);
            if ($stdin !== null) {
                $body = substr($stdin, 0, 1048576);
            } elseif ($length > 0) {
                $body = (string) stream_get_contents(STDIN, $length);
            }
        }

        return new \WebPulse\Http\Request($method, $query, $body, $headers, (string) ($env['REMOTE_ADDR'] ?? ''));
    }

    /** Resolve the WebPulse code base directory (WEBPULSE_HOME env, else /usr/local/webpulse). */
    public static function home(): string
    {
        $env = getenv('WEBPULSE_HOME');
        return is_string($env) && $env !== '' ? rtrim($env, '/') : '/usr/local/webpulse';
    }
}
