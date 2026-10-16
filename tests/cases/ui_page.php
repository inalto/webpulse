<?php

declare(strict_types=1);

use WebPulse\App;
use WebPulse\Http\Page;
use WebPulse\Http\PageOptions;
use WebPulse\Http\Principal;
use WebPulse\Http\Response;
use WebPulse\Tests\SeedDb;
use WebPulse\Tests\T;

/*
 * WP2 — Page::render (document vs embedded fragment, CSP nonce, escaped
 * bootstrap), Response CGI mode, UI asset checks (node --check, size,
 * no innerHTML, no external URLs).
 */

require_once dirname(__DIR__) . '/support/seed_db.php';

$repo = dirname(__DIR__, 2);

return [
    'full document with nonce CSP' => function () {
        $env = SeedDb::shared();
        $html = Page::render(App::boot($env['config']), new Principal(Principal::USER, 'alice'), new PageOptions('api.live.php'));
        T::true(str_starts_with($html, "<!doctype html>\n<html lang=\"en\">"), 'document');
        T::true((bool) preg_match('/script-src \'nonce-([A-Za-z0-9_-]{20,})\'/', $html, $m), 'nonce in CSP');
        $nonce = $m[1];
        T::contains("default-src 'none'; script-src 'nonce-$nonce'; style-src 'nonce-$nonce'; img-src data:; connect-src 'self'", $html);
        preg_match_all('/<(script|style)\b([^>]*)>/', $html, $tags, PREG_SET_ORDER);
        T::true(count($tags) >= 5, 'style + bootstrap + 3 scripts + start');
        foreach ($tags as $t) {
            T::contains("nonce=\"$nonce\"", $t[2], "every {$t[1]} carries the nonce");
        }
        T::false(str_contains($html, 'admin.js') || str_contains($html, 'WebPulseAdmin = {'), 'no admin code for users');
        T::false((bool) preg_match('/\b(src|href)=["\']https?:/i', $html), 'no external resources');
        T::contains('<div id="webpulse-app" class="wp-root" data-embedded="0">', $html);
        $html2 = Page::render(App::boot($env['config']), new Principal(Principal::USER, 'alice'), new PageOptions('api.live.php'));
        preg_match('/nonce-([A-Za-z0-9_-]+)/', $html2, $m2);
        T::true($m2[1] !== $nonce, 'fresh nonce per render');
    },
    'embedded fragment for cPanel chrome' => function () {
        $env = SeedDb::shared();
        $html = Page::render(App::boot($env['config']), new Principal(Principal::USER, 'alice'), new PageOptions('api.live.php', true));
        T::true(str_starts_with($html, '<div id="webpulse-app" class="wp-root" data-embedded="1">'), 'fragment');
        T::false(str_contains($html, '<html') || str_contains($html, '<head') || str_contains($html, '<body'), 'no document tags');
        T::contains('<style nonce=', $html);
        T::contains('WebPulseApp.start(', $html);
    },
    'admin pages include admin.js' => function () {
        $env = SeedDb::shared();
        $html = Page::render(App::boot($env['config']), new Principal(Principal::ADMIN, 'root'), new PageOptions('api.cgi'));
        T::contains('window.WebPulseAdmin = { render: render };', $html);
        T::contains('"principal":{"type":"admin","name":"root"}', $html);
    },
    'bootstrap JSON is escaped for <script>' => function () {
        $env = SeedDb::shared();
        $evil = 'x</script><script>alert(1)</script>&<!--';
        $html = Page::render(App::boot($env['config']), new Principal(Principal::USER, 'alice'), new PageOptions($evil, false, '<b>T</b>'));
        T::false(str_contains($html, '</script><script>alert(1)'), 'no raw closing tag');
        $u = static fn (string $hex) => chr(92) . 'u00' . $hex; // JSON_HEX_TAG / JSON_HEX_AMP escapes
        T::contains('"apiUrl":"x' . $u('3C') . '/script' . $u('3E') . $u('3C') . 'script' . $u('3E') . 'alert(1)' . $u('3C') . '/script' . $u('3E') . $u('26') . $u('3C') . '!--"', $html);
        T::contains('<title>&lt;b&gt;T&lt;/b&gt;</title>', $html);
        preg_match('/window\.WEBPULSE = (\{.*?\});<\/script>/s', $html, $m);
        $boot = json_decode($m[1], true);
        T::eq($evil, $boot['apiUrl'], 'round-trips');
        T::eq(['example.com', 'shop.example.com'], array_column($boot['domains'], 'domain'));
        T::eq(32, count($boot['dims']));
        T::eq('30d', $boot['defaults']['range']);
    },
    'Response: CGI status line, JSON envelope' => function () {
        $r = Response::error(404, 'not_found', 'Unknown domain')->cgi(true);
        $lines = $r->headerLines();
        T::eq('Status: 404 Not Found', $lines[0]);
        T::contains('Content-Type: application/json; charset=utf-8', implode("\n", $lines));
        ob_start();
        $r->send();
        $out = (string) ob_get_clean();
        T::true(str_starts_with($out, "Status: 404 Not Found\r\nContent-Type: application/json; charset=utf-8\r\n"));
        T::contains("\r\n\r\n{\"ok\":false,\"error\":{\"code\":\"not_found\",\"message\":\"Unknown domain\"}}", $out);
        $j = json_decode(Response::json(['n' => 1.5, 'z' => 0], microtime(true))->body, true);
        T::eq(['n' => 1.5, 'z' => 0], $j['data']);
        T::true(isset($j['meta']['generated_at']));
        $h = (new Response(200, 'text/plain', 'x', ['X-Evil' => "a\r\nSet-Cookie: b"]))->headerLines();
        T::contains('X-Evil: aSet-Cookie: b', implode("\n", $h), 'header injection stripped');
    },
    'UI JavaScript parses (node --check)' => function () use ($repo) {
        $node = trim((string) shell_exec('command -v node 2>/dev/null'));
        if ($node === '') {
            throw new WebPulse\Tests\Skip('node not installed');
        }
        foreach (glob($repo . '/ui/*.js') ?: [] as $f) {
            exec(escapeshellarg($node) . ' --check ' . escapeshellarg($f) . ' 2>&1', $out, $rc);
            T::eq(0, $rc, basename($f) . ': ' . implode("\n", $out));
        }
    },
    'UI assets: size budget, no innerHTML, no external requests' => function () use ($repo) {
        $total = 0;
        foreach (['app.css', 'charts.js', 'i18n.js', 'app.js', 'admin.js'] as $f) {
            $src = (string) file_get_contents("$repo/ui/$f");
            $total += strlen($src);
            T::false((bool) preg_match('/\.(innerHTML|outerHTML)\s*=|insertAdjacentHTML|document\.write/', $src), "$f must not inject HTML");
            T::false((bool) preg_match('#https?://(?!www\.w3\.org/2000/svg)#', $src), "$f must not reference external URLs");
            if (str_ends_with($f, '.js')) {
                T::contains("'use strict';", $src, "$f strict mode");
            }
        }
        T::less(200 * 1024, $total, 'inline payload under 200 KB');
    },
];
