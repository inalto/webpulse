<?php

declare(strict_types=1);

namespace WebPulse\Http;

use WebPulse\App;

/**
 * Starts `bin/webpulse` in the background (nohup, detached from the web
 * request) so that WHM can trigger collection, backfill and rollup.
 */
final class JobRunner
{
    /**
     * @param list<string> $args CLI arguments after `bin/webpulse`
     * @return string|null the shell command, or null when it could not start
     */
    public static function spawn(App $app, array $args): ?string
    {
        $php = $app->config->get('php_bin');
        if (!is_string($php) || $php === '' || !is_executable($php)) {
            $php = self::phpBinary();
        }
        $bin = $app->paths->root() . '/bin/webpulse';
        $parts = array_map('escapeshellarg', array_merge([$php, $bin], $args));
        $cmd = implode(' ', $parts);
        $log = $app->paths->logDir() . '/jobs.out';
        $redirect = is_dir(dirname($log)) && (is_writable(dirname($log)) || is_writable($log)) ? escapeshellarg($log) : '/dev/null';
        $shell = 'nohup ' . $cmd . ' >>' . $redirect . ' 2>&1 </dev/null &';
        $env = null;
        $cfg = $app->configPath();
        if ($cfg !== null && is_file($cfg)) {
            $env = array_merge(getenv(), ['WEBPULSE_CONFIG' => $cfg]);
        }
        $proc = @proc_open(['/bin/sh', '-c', $shell], [0 => ['file', '/dev/null', 'r'], 1 => ['file', '/dev/null', 'w'], 2 => ['file', '/dev/null', 'w']], $pipes, $app->paths->root(), $env);
        if (!is_resource($proc)) {
            return null;
        }
        $code = proc_close($proc);
        return $code === 0 ? $cmd : null;
    }

    /** The CLI PHP binary (PHP_BINARY may be php-cgi under a web server). */
    public static function phpBinary(): string
    {
        $candidates = [PHP_BINARY, PHP_BINDIR . '/php', '/usr/local/cpanel/3rdparty/bin/php', '/usr/bin/php'];
        foreach ($candidates as $c) {
            if ($c !== '' && is_executable($c) && !preg_match('/(cgi|fpm)/', basename($c))) {
                return $c;
            }
        }
        return 'php';
    }
}
