<?php

declare(strict_types=1);

/*
 * WebPulse Analytics - JSON API for the cPanel dashboard (LiveAPI PHP, runs as the account).
 * The principal is derived from the cPanel session, never from request parameters.
 */

if (!class_exists('CPANEL', false)) {
    require_once '/usr/local/cpanel/php/cpanel.php';
}

try {
    $cpanel = new CPANEL();
    require_once __DIR__ . '/lib/bootstrap.php';

    $user = WebPulse\Cpanel\LiveApiUser::resolve($cpanel);
    $app = WebPulse\App::boot();

    WebPulse\Http\Api::dispatch(
        $app,
        WebPulse\Http\Request::fromGlobals(),
        new WebPulse\Http\Principal(WebPulse\Http\Principal::USER, $user)
    )->send();

    $cpanel->end();
} catch (Throwable $e) {
    error_log('webpulse api.live.php: ' . get_class($e) . ': ' . $e->getMessage());
    if (!headers_sent()) {
        http_response_code(500);
        header('Content-Type: application/json; charset=utf-8');
        header('Cache-Control: no-store');
    }
    print json_encode(
        ['ok' => false, 'error' => ['code' => 'internal_error', 'message' => 'Internal error']],
        JSON_UNESCAPED_SLASHES
    );
}
