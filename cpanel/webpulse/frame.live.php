<?php

declare(strict_types=1);

/*
 * WebPulse Analytics - standalone dashboard document (LiveAPI PHP, runs as the account).
 *
 * Used by the Meridian hub, which embeds it in an iframe: no cPanel header/footer,
 * the full page comes from `webpulse cgi --page` (api calls go to api.live.php).
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
        ['cgi', '--principal', 'user:' . $webpulseUser, '--page', '--api-url', 'api.live.php'],
        $webpulseUser,
        true,
        __DIR__
    );
} catch (Throwable $e) {
    error_log('webpulse frame.live.php: ' . get_class($e));
}

if (!$webpulseOk) {
    if (!headers_sent()) {
        http_response_code(500);
        header('Content-Type: text/html; charset=utf-8');
    }
    print '<!doctype html><meta charset="utf-8"><title>WebPulse</title>'
        . '<p style="font:15px system-ui;margin:2em">WebPulse is not available right now. '
        . 'Please contact your hosting provider.</p>';
}

try {
    if ($cpanel !== null) {
        $cpanel->end();
    }
} catch (Throwable $e) {
    // ignore
}
