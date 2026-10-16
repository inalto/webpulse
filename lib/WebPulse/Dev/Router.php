<?php

declare(strict_types=1);

namespace WebPulse\Dev;

use WebPulse\App;
use WebPulse\Http\Api;
use WebPulse\Http\Page;
use WebPulse\Http\PageOptions;
use WebPulse\Http\Principal;
use WebPulse\Http\Request;
use WebPulse\Http\Response;

/**
 * Router for `webpulse serve` (PHP's built-in web server): `/` renders the
 * dashboard, `/api` the JSON API, with a fixed principal taken from the
 * WEBPULSE_DEV_PRINCIPAL environment variable (`user:alice`, `admin`,
 * `reseller:reseller1`). For local development only — it binds to 127.0.0.1.
 *
 * Binding to 127.0.0.1 is not authentication (other local users, and pages
 * open in the browser through DNS rebinding, can reach it), so every request
 * must carry a Host of 127.0.0.1:PORT or localhost:PORT and the random token
 * of WEBPULSE_DEV_TOKEN: as `?token=` on the first page load (answered with
 * a SameSite=Strict, HttpOnly cookie and a redirect to `/`), then as that
 * cookie or the X-WebPulse-Dev-Token header.
 */
final class Router
{
    public const ENV = 'WEBPULSE_DEV_PRINCIPAL';
    public const ENV_TOKEN = 'WEBPULSE_DEV_TOKEN';
    public const ENV_PORT = 'WEBPULSE_DEV_PORT';
    public const COOKIE = 'webpulse_dev';
    public const HEADER = 'X-WebPulse-Dev-Token';

    /**
     * Parses `user:<name>`, `admin` or `reseller:<name>`.
     */
    public static function principal(App $app, string $spec): Principal
    {
        [$type, $name] = array_pad(explode(':', $spec, 2), 2, '');
        if ($type === Principal::ADMIN) {
            return new Principal(Principal::ADMIN, 'root');
        }
        if ($type === Principal::RESELLER && $name !== '') {
            $owned = [];
            try {
                foreach ($app->cpanelUsers()->all() as $user => $info) {
                    if (is_array($info) && ($info['owner'] ?? null) === $name) {
                        $owned[] = (string) $user;
                    }
                }
            } catch (\Throwable) {
                $owned = [];
            }
            return new Principal(Principal::RESELLER, $name, $owned);
        }
        if ($type === Principal::USER && $name !== '') {
            return new Principal(Principal::USER, $name);
        }
        throw new \InvalidArgumentException('invalid principal "' . $spec . '" (user:<name>, admin, reseller:<name>)');
    }

    /**
     * Handles the current request; returns false to let `php -S` serve it
     * (never used: everything is answered here).
     */
    public static function handle(): bool
    {
        $path = parse_url((string) ($_SERVER['REQUEST_URI'] ?? '/'), PHP_URL_PATH) ?: '/';
        $token = (string) (getenv(self::ENV_TOKEN) ?: '');
        [$status, $why, $setCookie] = self::gate($path, $_SERVER, $_GET, $_COOKIE, $token, (string) (getenv(self::ENV_PORT) ?: ''));
        if ($status !== 200) {
            (new Response($status, 'text/plain; charset=utf-8', "webpulse serve: $why\n", ['Cache-Control' => 'no-store']))->send();
            return true;
        }
        if ($setCookie) {
            // keep the token out of the address bar and history
            (new Response(303, 'text/plain; charset=utf-8', '', [
                'Set-Cookie' => self::COOKIE . '=' . $token . '; Path=/; HttpOnly; SameSite=Strict',
                'Location' => '/',
                'Cache-Control' => 'no-store',
            ]))->send();
            return true;
        }
        try {
            $app = App::boot();
            $principal = self::principal($app, (string) (getenv(self::ENV) ?: 'admin'));
        } catch (\Throwable $e) {
            (new Response(500, 'text/plain; charset=utf-8', 'webpulse serve: ' . $e->getMessage() . "\n"))->send();
            return true;
        }
        if ($path === '/' || $path === '/index.html') {
            $html = Page::render($app, $principal, new PageOptions('/api', false, 'WebPulse Analytics (dev)'));
            (new Response(200, 'text/html; charset=utf-8', $html, ['Cache-Control' => 'no-store']))->send();
        } elseif ($path === '/api') {
            Api::dispatch($app, Request::fromGlobals(), $principal)->send();
        } elseif ($path === '/favicon.ico') {
            (new Response(204, 'image/x-icon', ''))->send();
        } else {
            (new Response(404, 'text/plain; charset=utf-8', "Not found\n"))->send();
        }
        return true;
    }

    /**
     * Access check of one request.
     *
     * @param array<string,mixed> $server  $_SERVER
     * @param array<string,mixed> $query   $_GET
     * @param array<string,mixed> $cookies $_COOKIE
     * @param string $token expected token ('' = not configured: everything is refused)
     * @param string $port  listening port ('' = SERVER_PORT)
     * @return array{0:int,1:string,2:bool} [200 when allowed else 403, reason, answer with the cookie + redirect]
     */
    public static function gate(string $path, array $server, array $query, array $cookies, string $token, string $port): array
    {
        if (strlen($token) < 16) {
            return [403, 'no access token configured; start the server with "webpulse serve"', false];
        }
        if ($port === '') {
            $port = (string) ($server['SERVER_PORT'] ?? '');
        }
        $host = strtolower((string) ($server['HTTP_HOST'] ?? ''));
        if ($port === '' || !in_array($host, ["127.0.0.1:$port", "localhost:$port"], true)) {
            return [403, 'unexpected Host header', false];
        }
        $ok = static fn (mixed $t): bool => is_string($t) && $t !== '' && hash_equals($token, $t);
        if (($path === '/' || $path === '/index.html') && $ok($query['token'] ?? null)) {
            return [200, '', true];
        }
        if ($ok($cookies[self::COOKIE] ?? null) || $ok($server['HTTP_X_WEBPULSE_DEV_TOKEN'] ?? null)) {
            return [200, '', false];
        }
        return [403, 'missing or wrong access token; open the URL printed by "webpulse serve"', false];
    }
}
