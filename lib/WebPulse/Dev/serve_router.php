<?php

declare(strict_types=1);

/*
 * Router script for `php -S 127.0.0.1:<port> lib/WebPulse/Dev/serve_router.php`
 * (started by `webpulse serve`). Configuration comes from WEBPULSE_CONFIG,
 * the principal from WEBPULSE_DEV_PRINCIPAL.
 */

require dirname(__DIR__, 2) . '/autoload.php';

return WebPulse\Dev\Router::handle();
