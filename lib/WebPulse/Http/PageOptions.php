<?php

declare(strict_types=1);

namespace WebPulse\Http;

/**
 * How the dashboard page is rendered by Page::render().
 */
final class PageOptions
{
    /**
     * @param string      $apiUrl   URL of the JSON API relative to the page (`api.live.php`, `api.cgi`, `/api`)
     * @param bool        $embedded true inside cPanel chrome: return a fragment instead of a document
     * @param string|null $title    document title (default "WebPulse Analytics")
     */
    public function __construct(
        public string $apiUrl,
        public bool $embedded = false,
        public ?string $title = null,
    ) {
    }
}
