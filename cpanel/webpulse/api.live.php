<?php

declare(strict_types=1);

/*
 * WebPulse Analytics - JSON API for the cPanel dashboard (LiveAPI PHP, runs as the account).
 *
 * Thin shim: the principal is derived from the cPanel session, never from request
 * parameters, and everything else is done by `webpulse cgi --api`.
 */

if (!class_exists('CPANEL', false)) {
    require_once '/usr/local/cpanel/php/cpanel.php';
}
require_once __DIR__ . '/lib/LiveApiUser.php';
require_once __DIR__ . '/lib/CgiBridge.php';

$webpulseOk = false;
$cpanel = null;
try {
    $cpanel = new CPANEL();
    $webpulseUser = WebPulse\Cpanel\LiveApiUser::resolve($cpanel);
    $webpulseOk = WebPulse\Cpanel\CgiBridge::run(
        ['cgi', '--principal', 'user:' . $webpulseUser, '--api'],
        $webpulseUser,
        true,
        __DIR__
    );
} catch (Throwable $e) {
    error_log('webpulse api.live.php: ' . get_class($e));
}

if (!$webpulseOk) {
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

try {
    if ($cpanel !== null) {
        $cpanel->end();
    }
} catch (Throwable $e) {
    // ignore
}
