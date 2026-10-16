<?php

declare(strict_types=1);

/*
 * Parity driver: runs API requests through the legacy PHP implementation.
 *
 *   php tests/parity/php_api.php <legacy-lib-dir> < requests.tsv
 *
 * Each input line is `principal<TAB>method<TAB>query-string[<TAB>json-body]`
 * where principal is `user:NAME`, `whm:root` or `whm:RESELLER` (the WHM
 * front-end's REMOTE_USER rule: root = admin, else a reseller with the
 * accounts it owns, else 403). The configuration comes from WEBPULSE_CONFIG,
 * the clock from WEBPULSE_NOW. One JSON line is printed per request:
 * {"status":…,"type":…,"headers":{…},"body":"…"}.
 */

error_reporting(E_ALL);
$lib = rtrim($argv[1] ?? '', '/');
if ($lib === '' || !is_file($lib . '/autoload.php')) {
    fwrite(STDERR, "usage: php php_api.php <legacy lib dir> < requests.tsv\n");
    exit(2);
}
require $lib . '/autoload.php';

/** whm/lib/Edge.php of 0.1.0: anything but a plain GET/HEAD read needs POST and the header. */
function webpulse_parity_mutation_allowed(string $action, string $method, ?string $header): bool
{
    $method = strtoupper($method);
    $mutating = preg_match('/^(?:admin\.)?(?:set_|save_|run$|purge$|delete|reset|enable|disable)/', $action) === 1;
    if (!($method === 'GET' || $method === 'HEAD') || $mutating) {
        return $method === 'POST' && $header === '1';
    }
    return true;
}

while (($line = fgets(STDIN)) !== false) {
    $line = rtrim($line, "\r\n");
    if ($line === '' || $line[0] === '#') {
        continue;
    }
    $parts = explode("\t", $line, 4);
    [$who, $method, $qs] = array_pad($parts, 3, '');
    $body = $parts[3] ?? null;
    $app = WebPulse\App::boot();
    $principal = null;
    if (str_starts_with($who, 'user:')) {
        $principal = new WebPulse\Http\Principal(WebPulse\Http\Principal::USER, substr($who, 5));
    } elseif ($who === 'whm:root') {
        $principal = new WebPulse\Http\Principal(WebPulse\Http\Principal::ADMIN, 'root');
    } elseif (str_starts_with($who, 'whm:')) {
        $name = substr($who, 4);
        $owned = [];
        foreach ($app->cpanelUsers()->all() as $u => $info) {
            if (($info['owner'] ?? null) === $name) {
                $owned[] = (string) $u;
            }
        }
        sort($owned, SORT_STRING);
        if ($owned !== []) {
            $principal = new WebPulse\Http\Principal(WebPulse\Http\Principal::RESELLER, $name, $owned);
        }
    }
    $query = [];
    parse_str($qs, $query);
    $action = is_string($query['a'] ?? null) ? $query['a'] : '';
    $headers = $method === 'POST' ? ['X-WebPulse-Request' => '1'] : [];
    if ($principal === null) {
        $res = WebPulse\Http\Response::error(403, 'forbidden', 'Access denied.');
    } elseif (str_starts_with($who, 'whm:')
        && !webpulse_parity_mutation_allowed($action, $method, $headers['X-WebPulse-Request'] ?? null)) {
        // whm/cgi/api.cgi answers before the API (defence in depth)
        $res = WebPulse\Http\Response::error(405, 'method_not_allowed', 'This action requires POST and the X-WebPulse-Request: 1 header.');
    } else {
        $req = new WebPulse\Http\Request($method, $query, $body, $headers, '203.0.113.9');
        $res = WebPulse\Http\Api::dispatch($app, $req, $principal);
    }
    echo json_encode(
        ['status' => $res->status, 'type' => $res->contentType, 'headers' => (object) $res->headers, 'body' => $res->body],
        JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE
    ), "\n";
}
