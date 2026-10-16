<?php

declare(strict_types=1);

namespace WebPulse\Http;

use WebPulse\App;
use WebPulse\Collect\UserSettings;
use WebPulse\Reports\Dimension;
use WebPulse\Reports\Range;

/**
 * Renders the single-page dashboard with its CSS and JavaScript inlined, a
 * per-request nonce, a strict Content-Security-Policy (full documents) and
 * the `window.WEBPULSE` bootstrap.
 */
final class Page
{
    public const JSON_FLAGS = JSON_HEX_TAG | JSON_HEX_AMP | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE;

    /** UI sources in load order; admin.js only for admin principals. */
    private const SCRIPTS = ['i18n.js', 'charts.js', 'app.js'];

    /**
     * Full HTML document, or (embedded) a `<div id="webpulse-app">` fragment
     * with its `<style>`/`<script>` tags for cPanel's header()/footer().
     */
    public static function render(App $app, Principal $p, PageOptions $o): string
    {
        $nonce = rtrim(strtr(base64_encode(random_bytes(18)), '+/', '-_'), '=');
        $ui = self::uiDir($app);
        $css = self::read($ui . '/app.css');
        $scripts = self::SCRIPTS;
        if ($p->hasAdminScope()) {
            $scripts[] = 'admin.js';
        }
        $js = '';
        foreach ($scripts as $s) {
            $js .= "<script nonce=\"$nonce\">\n" . self::guard(self::read($ui . '/' . $s)) . "\n</script>\n";
        }
        $boot = json_encode(self::bootstrap($app, $p, $o), self::JSON_FLAGS | JSON_THROW_ON_ERROR);
        $title = htmlspecialchars($o->title ?? 'WebPulse Analytics', ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');

        $body = '<div id="webpulse-app" class="wp-root" data-embedded="' . ($o->embedded ? '1' : '0') . '">'
            . '<noscript><p class="wp-noscript">WebPulse Analytics needs JavaScript enabled.</p></noscript>'
            . '<div class="wp-boot" aria-live="polite">Loading analytics…</div></div>' . "\n"
            . "<style nonce=\"$nonce\">\n" . self::guard($css, 'style') . "\n</style>\n"
            . "<script nonce=\"$nonce\">window.WEBPULSE = $boot;</script>\n"
            . $js
            . "<script nonce=\"$nonce\">WebPulseApp.start(document.getElementById('webpulse-app'), window.WEBPULSE);</script>\n";

        if ($o->embedded) {
            return $body;
        }
        $csp = "default-src 'none'; script-src 'nonce-$nonce'; style-src 'nonce-$nonce'; img-src data:; connect-src 'self'; base-uri 'none'; form-action 'none'";
        return "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n"
            . "<meta http-equiv=\"Content-Security-Policy\" content=\"$csp\">\n"
            . "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n"
            . "<meta name=\"referrer\" content=\"no-referrer\">\n"
            . "<meta name=\"color-scheme\" content=\"light dark\">\n"
            . "<title>$title</title>\n</head>\n<body class=\"wp-body\">\n" . $body . "</body>\n</html>\n";
    }

    /**
     * The `window.WEBPULSE` object.
     *
     * @return array<string,mixed>
     */
    public static function bootstrap(App $app, Principal $p, PageOptions $o): array
    {
        $domains = [];
        try {
            $domains = Api::domains($app, $p);
        } catch (\Throwable) {
            $domains = [];
        }
        $range = Range::DEFAULT;
        $dark = 'auto';
        if ($p->isUser()) {
            try {
                $s = UserSettings::read($app->paths, $p->name);
                $ui = is_array($s['ui'] ?? null) ? $s['ui'] : [];
                if (is_string($ui['default_range'] ?? null) && in_array($ui['default_range'], Range::SHORTCUTS, true)) {
                    $range = $ui['default_range'];
                }
                if (in_array($ui['dark'] ?? null, ['auto', 'light', 'dark', true, false], true)) {
                    $dark = is_bool($ui['dark']) ? ($ui['dark'] ? 'dark' : 'light') : $ui['dark'];
                }
            } catch (\Throwable) {
                // defaults
            }
        }
        return [
            'apiUrl' => $o->apiUrl,
            'principal' => $p->toArray(),
            'domains' => $domains,
            'defaults' => ['range' => $range, 'dark' => $dark],
            'version' => $app->version(),
            'embedded' => $o->embedded,
            'dims' => Dimension::meta(),
            'ranges' => Range::SHORTCUTS,
        ];
    }

    private static function uiDir(App $app): string
    {
        $dir = $app->paths->root() . '/ui';
        return is_dir($dir) ? $dir : dirname(__DIR__, 3) . '/ui';
    }

    private static function read(string $file): string
    {
        $s = @file_get_contents($file);
        if ($s === false) {
            throw new \RuntimeException('UI asset missing: ' . basename($file));
        }
        return $s;
    }

    /** Defensive: an inlined asset must never close its own element. */
    private static function guard(string $src, string $tag = 'script'): string
    {
        return str_ireplace('</' . $tag, '<\\/' . $tag, $src);
    }
}
