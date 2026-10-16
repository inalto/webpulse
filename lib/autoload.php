<?php

declare(strict_types=1);

/*
 * PSR-4 autoloader for the WebPulse\ namespace (no Composer).
 *
 *   WebPulse\Foo\Bar  ->  lib/WebPulse/Foo/Bar.php
 */
spl_autoload_register(static function (string $class): void {
    if (strncmp($class, 'WebPulse\\', 9) !== 0) {
        return;
    }
    $file = __DIR__ . '/' . str_replace('\\', '/', $class) . '.php';
    if (is_file($file)) {
        require $file;
    }
});
