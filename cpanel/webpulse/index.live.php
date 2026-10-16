<?php

declare(strict_types=1);

/*
 * WebPulse Analytics - cPanel dashboard (LiveAPI PHP, runs as the account).
 * The Jupiter chrome comes from $cpanel->header()/footer(); the dashboard is an
 * embedded fragment rendered by WebPulse\Http\Page.
 */

if (!class_exists('CPANEL', false)) {
    require_once '/usr/local/cpanel/php/cpanel.php';
}

$cpanel = new CPANEL();

try {
    print $cpanel->header('WebPulse Analytics');
} catch (Throwable $e) {
    // header() failing must not hide the dashboard error handling below
}

try {
    require_once __DIR__ . '/lib/bootstrap.php';

    $user = WebPulse\Cpanel\LiveApiUser::resolve($cpanel);
    $app = WebPulse\App::boot();
    $principal = new WebPulse\Http\Principal(WebPulse\Http\Principal::USER, $user);

    print WebPulse\Http\Page::render(
        $app,
        $principal,
        new WebPulse\Http\PageOptions(apiUrl: 'api.live.php', embedded: true)
    );
} catch (Throwable $e) {
    error_log('webpulse index.live.php: ' . get_class($e) . ': ' . $e->getMessage());
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
