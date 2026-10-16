<?php

declare(strict_types=1);

/*
 * Shared bootstrap of the WebPulse cPanel front-ends (index.live.php, api.live.php).
 *
 * Locates the WebPulse code base and loads its autoloader. Resolution order:
 *   1. $WEBPULSE_HOME (environment, used by tests)   /lib/autoload.php
 *   2. /usr/local/webpulse                           /lib/autoload.php
 *   3. the copy installed next to this file          ./autoload.php   (theme copy)
 *   4. the repository layout                         ../../../lib/autoload.php
 */

if (!defined('WEBPULSE_HOME')) {
    $webpulseEnvHome = getenv('WEBPULSE_HOME');
    define('WEBPULSE_HOME', is_string($webpulseEnvHome) && $webpulseEnvHome !== ''
        ? rtrim($webpulseEnvHome, '/')
        : '/usr/local/webpulse');
    unset($webpulseEnvHome);
}

/** @return string path of the first readable autoload.php, or '' */
function webpulse_find_autoload(): string
{
    $candidates = [
        WEBPULSE_HOME . '/lib/autoload.php',
        '/usr/local/webpulse/lib/autoload.php',
        __DIR__ . '/autoload.php',
        dirname(__DIR__, 3) . '/lib/autoload.php',
    ];
    foreach ($candidates as $file) {
        if (is_file($file) && is_readable($file)) {
            return $file;
        }
    }
    return '';
}

$webpulseAutoload = webpulse_find_autoload();
if ($webpulseAutoload === '') {
    throw new RuntimeException('WebPulse is not installed (autoload.php not found).');
}
require_once $webpulseAutoload;
require_once __DIR__ . '/LiveApiUser.php';
unset($webpulseAutoload);
