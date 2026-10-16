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
 */
final class Router
{
    public const ENV = 'WEBPULSE_DEV_PRINCIPAL';

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
}
