#!/usr/local/cpanel/3rdparty/bin/php
<?php
#WHMADDON:webpulse:WebPulse Analytics:webpulse.png
#ACLS:all

declare(strict_types=1);

/*
 * WebPulse Analytics - JSON API for the WHM dashboard (runs as root under cpsrvd).
 */

$webpulseEnv = getenv('WEBPULSE_HOME');
$webpulseHome = is_string($webpulseEnv) && $webpulseEnv !== '' ? rtrim($webpulseEnv, '/') : '/usr/local/webpulse';

/** Print a complete JSON error response and stop. */
function webpulse_cgi_json_error(int $status, string $code, string $message): never
{
    $reason = [400 => 'Bad Request', 403 => 'Forbidden', 405 => 'Method Not Allowed', 500 => 'Internal Server Error'][$status] ?? 'Error';
    print "Status: {$status} {$reason}\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\n\r\n";
    print json_encode(['ok' => false, 'error' => ['code' => $code, 'message' => $message]], JSON_UNESCAPED_SLASHES);
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
        webpulse_cgi_json_error(403, 'forbidden', 'Access denied.');
    }

    $request = WebPulse\Whm\Edge::request();
    $query = [];
    parse_str((string) getenv('QUERY_STRING'), $query);
    $action = is_string($query['a'] ?? null) ? $query['a'] : '';
    $header = getenv('HTTP_X_WEBPULSE_REQUEST');
    if (!WebPulse\Whm\Edge::mutationAllowed($action, (string) (getenv('REQUEST_METHOD') ?: 'GET'), is_string($header) ? $header : null)) {
        webpulse_cgi_json_error(405, 'method_not_allowed', 'This action requires POST and the X-WebPulse-Request: 1 header.');
    }

    WebPulse\Http\Api::dispatch($app, $request, $principal)->cgi(true)->send();
} catch (Throwable $e) {
    error_log('webpulse api.cgi: ' . get_class($e) . ': ' . $e->getMessage());
    webpulse_cgi_json_error(500, 'internal_error', 'Internal error');
}
