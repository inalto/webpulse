<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Cli\UsageException;
use WebPulse\Dev\Router;
use WebPulse\Http\DomainResolver;

/**
 * webpulse serve [--port 8099] (--user U | --admin | --reseller R)
 *
 * Starts PHP's built-in web server on 127.0.0.1 with the dev router: `/`
 * is the dashboard, `/api` the JSON API, with a fixed principal. For local
 * development and screenshots only.
 */
final class ServeCommand implements CommandInterface
{
    public static function summary(): string
    {
        return 'local dev dashboard on http://127.0.0.1:<port> (no cPanel needed)';
    }

    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse serve [--port 8099] (--user USER | --admin | --reseller NAME) [--config FILE]

        Runs `php -S 127.0.0.1:PORT` with the WebPulse dev router. The dashboard
        is served at / and the API at /api, acting as the given principal:
          --user USER       a cPanel account (sees its own domains)
          --admin           the WHM root administrator (default)
          --reseller NAME   a reseller (sees the accounts it owns)
        The server only listens on 127.0.0.1. Stop it with Ctrl-C.
        TXT;
    }

    public static function options(): array
    {
        return ['port' => Args::VALUE, 'user' => Args::VALUE, 'admin' => Args::FLAG, 'reseller' => Args::VALUE];
    }

    public function run(Cli $cli, Args $args): int
    {
        $port = (string) ($args->opt('port') ?? '8099');
        if (!ctype_digit($port) || (int) $port < 1 || (int) $port > 65535) {
            throw new UsageException('--port must be a number between 1 and 65535');
        }
        $spec = self::principalSpec($args->opt('user'), $args->has('admin'), $args->opt('reseller'));

        $app = $cli->app();
        Router::principal($app, $spec); // validates early
        $config = $app->configPath();
        $router = $app->paths->root() . '/lib/WebPulse/Dev/serve_router.php';
        $env = getenv();
        $env[Router::ENV] = $spec;
        if ($config !== null && is_file($config)) {
            $env['WEBPULSE_CONFIG'] = (string) realpath($config);
        }
        $cli->out("WebPulse dev server: http://127.0.0.1:$port/  (principal $spec, config " . ($env['WEBPULSE_CONFIG'] ?? 'default') . ')');
        $cmd = [PHP_BINARY, '-S', '127.0.0.1:' . $port, '-t', $app->paths->root() . '/ui', $router];
        $proc = proc_open($cmd, [0 => STDIN, 1 => STDOUT, 2 => STDERR], $pipes, $app->paths->root(), $env);
        if (!is_resource($proc)) {
            $cli->err('could not start php -S');
            return Cli::EXIT_ERROR;
        }
        return proc_close($proc) === 0 ? Cli::EXIT_OK : Cli::EXIT_ERROR;
    }

    /**
     * `user:<name>`, `admin` or `reseller:<name>` from the options.
     *
     * @throws UsageException
     */
    public static function principalSpec(mixed $user, bool $admin, mixed $reseller): string
    {
        $given = (int) ($user !== null) + (int) $admin + (int) ($reseller !== null);
        if ($given > 1) {
            throw new UsageException('use only one of --user, --admin, --reseller');
        }
        if (is_string($user)) {
            if (!DomainResolver::validUser($user)) {
                throw new UsageException("invalid user name '$user'");
            }
            return 'user:' . $user;
        }
        if (is_string($reseller)) {
            if (!DomainResolver::validUser($reseller)) {
                throw new UsageException("invalid reseller name '$reseller'");
            }
            return 'reseller:' . $reseller;
        }
        return 'admin';
    }
}
