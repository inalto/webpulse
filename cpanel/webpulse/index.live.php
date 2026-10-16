<?php

declare(strict_types=1);

/*
 * WebPulse Analytics - cPanel dashboard (LiveAPI PHP, runs as the account).
 *
 * Thin shim: the Jupiter chrome comes from $cpanel->header()/footer(); the dashboard
 * is the embedded fragment printed by `webpulse cgi --page --embedded`.
 */

if (!class_exists('CPANEL', false)) {
    require_once '/usr/local/cpanel/php/cpanel.php';
}
require_once __DIR__ . '/lib/LiveApiUser.php';
require_once __DIR__ . '/lib/CgiBridge.php';

$cpanel = new CPANEL();

try {
    print $cpanel->header('WebPulse Analytics');
} catch (Throwable $e) {
    // header() failing must not hide the dashboard error handling below
}

$webpulseOk = false;
try {
    $webpulseUser = WebPulse\Cpanel\LiveApiUser::resolve($cpanel);
    // body only: the cPanel header() above has already started the page
    $webpulseOk = WebPulse\Cpanel\CgiBridge::run(
        ['cgi', '--principal', 'user:' . $webpulseUser, '--page', '--embedded', '--api-url', 'api.live.php'],
        $webpulseUser,
        false,
        __DIR__
    );
} catch (Throwable $e) {
    error_log('webpulse index.live.php: ' . get_class($e));
}

if (!$webpulseOk) {
    print '<div class="body-content"><div class="alert alert-danger" role="alert">'
        . '<strong>WebPulse Analytics is not available right now.</strong> '
        . 'Please try again in a few minutes. If the problem persists, contact your hosting provider.'
        . '</div></div>';
}

try {
    print $cpanel->footer();
} catch (Throwable $e) {
    // ignore
}
$cpanel->end();
