#!/usr/local/cpanel/3rdparty/bin/php
<?php
#WHMADDON:webpulse:WebPulse Analytics:webpulse.png
#ACLS:all

declare(strict_types=1);

/*
 * WebPulse Analytics - WHM dashboard (runs as root under cpsrvd).
 */

$webpulseEnv = getenv('WEBPULSE_HOME');
$webpulseHome = is_string($webpulseEnv) && $webpulseEnv !== '' ? rtrim($webpulseEnv, '/') : '/usr/local/webpulse';

/** Print a complete CGI response and stop. */
function webpulse_cgi_page(int $status, string $title, string $message): never
{
    $reason = [403 => 'Forbidden', 500 => 'Internal Server Error'][$status] ?? 'Error';
    print "Status: {$status} {$reason}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\r\n";
    print '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>'
        . htmlspecialchars($title, ENT_QUOTES) . '</title></head><body><h2>'
        . htmlspecialchars($title, ENT_QUOTES) . '</h2><p>'
        . htmlspecialchars($message, ENT_QUOTES) . '</p></body></html>';
    exit(0);
}

try {
    require_once $webpulseHome . '/lib/autoload.php';
    require_once $webpulseHome . '/whm/lib/Edge.php';

    $app = WebPulse\App::boot();
    $principal = WebPulse\Whm\Edge::principal(
        getenv('REMOTE_USER') ?: null,
        $app->cpanelUsers()
    );
    if ($principal === null) {
        webpulse_cgi_page(403, 'Access denied', 'WebPulse Analytics is available to the server administrator and to resellers that own accounts.');
    }

    $html = WebPulse\Http\Page::render(
        $app,
        $principal,
        new WebPulse\Http\PageOptions(apiUrl: 'api.cgi', embedded: false)
    );
    print "Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\r\n";
    print $html;
} catch (Throwable $e) {
    error_log('webpulse index.cgi: ' . get_class($e) . ': ' . $e->getMessage());
    webpulse_cgi_page(500, 'WebPulse Analytics is not available', 'The dashboard could not be loaded. See the cPanel error log for details.');
}
