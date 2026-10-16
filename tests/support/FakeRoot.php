<?php

declare(strict_types=1);

namespace WebPulse\Tests;

use WebPulse\App;

/**
 * Builds a fake cPanel server under tests/tmp/<name>/ (skeleton copied from
 * tests/fixtures/fakeroot) and a config.json whose paths point into it.
 *
 *   $fr = FakeRoot::create('collect');
 *   $fr->generateLogs(['seed' => 42, 'days' => 3, 'lines' => 2000, 'end' => '2026-10-16']);
 *   ['code' => $c, 'stdout' => $o] = $fr->run('collect --json');
 *   $pdo = $fr->db('alice', 'example.com');
 *
 * Layout: <dir>/config.json, <dir>/root/{etc,var,home,usr}/…, webpulse data
 * in <dir>/root/var/webpulse/{data,webpulse.db,log,run,geoip}.
 */
final class FakeRoot
{
    private ?int $now = null;

    private function __construct(private string $dir)
    {
    }

    /**
     * Creates (wiping any previous content) tests/tmp/<name>.
     *
     * @param array<string,mixed> $config overrides merged into the generated config.json
     */
    public static function create(string $name, array $config = []): self
    {
        if (!preg_match('/^[A-Za-z0-9_.-]+$/', $name)) {
            throw new \InvalidArgumentException("bad fake root name $name");
        }
        $dir = self::repo() . '/tests/tmp/' . $name;
        self::rm($dir);
        mkdir($dir . '/root', 0755, true);
        self::copy(self::repo() . '/tests/fixtures/fakeroot', $dir . '/root');
        $fr = new self($dir);
        foreach (['etc/apache2/logs/domlogs/alice', 'etc/apache2/logs/domlogs/bob', 'home/alice/logs', 'home/bob/logs',
            'var/webpulse', 'var/log/nginx/domains', 'etc/cron.d'] as $d) {
            @mkdir($fr->root() . '/' . $d, 0755, true);
        }
        $fr->writeConfig(array_replace_recursive($fr->defaultConfig(), $config));
        return $fr;
    }

    /** Repository root. */
    public static function repo(): string
    {
        return dirname(__DIR__, 2);
    }

    /** tests/tmp/<name> */
    public function dir(): string
    {
        return $this->dir;
    }

    /** The fake server root (tests/tmp/<name>/root). */
    public function root(): string
    {
        return $this->dir . '/root';
    }

    public function configPath(): string
    {
        return $this->dir . '/config.json';
    }

    /** Absolute path of a file inside the fake root. */
    public function path(string $rel): string
    {
        return $this->root() . '/' . ltrim($rel, '/');
    }

    /** Path of a live domlog: domlog('alice', 'example.com-ssl_log'). */
    public function domlog(string $user, string $file): string
    {
        return $this->path("etc/apache2/logs/domlogs/$user/$file");
    }

    /** @return array<string,mixed> */
    public function defaultConfig(): array
    {
        $r = $this->root();
        return [
            'version' => 1,
            'default_enabled' => true,
            'timezone' => 'Europe/Rome',
            'raw_retention_days' => 30,
            'max_rows_per_dim' => 1000,
            'session_timeout_min' => 30,
            'store_ip' => false,
            'exclude_ips' => [],
            'exclude_paths' => ['/.well-known/', '/cpanel', '/whm', '/webmail', '/cgi-sys/'],
            'count_head_requests' => false,
            'geoip' => ['enabled' => false, 'country_db' => "$r/var/webpulse/geoip/dbip-country-lite.mmdb", 'city_db' => null],
            'php_bin' => null,
            'paths' => [
                'data_dir' => "$r/var/webpulse/data",
                'registry_db' => "$r/var/webpulse/webpulse.db",
                'log_dir' => "$r/var/webpulse/log",
                'run_dir' => "$r/var/webpulse/run",
                'domlogs_dirs' => ["$r/etc/apache2/logs/domlogs/{user}", "$r/etc/apache2/logs/domlogs", "$r/usr/local/apache/domlogs/{user}", "$r/var/log/nginx/domains"],
                'archive_dir' => "$r/home/{user}/logs",
                'userdomains_file' => "$r/etc/userdomains",
                'cpanel_users_dir' => "$r/var/cpanel/users",
                'cpanel_userdata_dir' => "$r/var/cpanel/userdata",
                'home_root' => "$r/home",
                'cron_file' => "$r/etc/cron.d/webpulse",
            ],
        ];
    }

    /** @param array<string,mixed> $cfg */
    public function writeConfig(array $cfg): void
    {
        file_put_contents($this->configPath(), json_encode($cfg, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) . "\n");
    }

    /** Merges $patch into config.json. */
    public function patchConfig(array $patch): void
    {
        $cfg = json_decode((string) file_get_contents($this->configPath()), true);
        $this->writeConfig(array_replace_recursive($cfg, $patch));
    }

    /** Pins the clock of subsequent run() calls and app() instances (null = real time). */
    public function setNow(?int $ts): void
    {
        $this->now = $ts;
        putenv($ts === null ? 'WEBPULSE_NOW' : 'WEBPULSE_NOW=' . $ts);
    }

    /**
     * Appends (or writes) lines to a file inside the fake root.
     *
     * @param string|string[] $lines
     */
    public function writeLog(string $rel, string|array $lines, bool $append = true): string
    {
        $file = $this->path($rel);
        @mkdir(dirname($file), 0755, true);
        $data = is_array($lines) ? ($lines === [] ? '' : implode("\n", $lines) . "\n") : $lines;
        file_put_contents($file, $data, $append ? FILE_APPEND : 0);
        return $file;
    }

    /**
     * Runs tests/gen_logs.php into this root; returns its JSON summary.
     *
     * @param array<string,scalar> $opts seed, days, lines, end, tz, no-archive…
     * @return array<string,mixed>
     */
    public function generateLogs(array $opts = []): array
    {
        $opts += ['seed' => 42, 'days' => 3, 'lines' => 3000, 'end' => '2026-10-16'];
        $args = [PHP_BINARY, self::repo() . '/tests/gen_logs.php', '--out', $this->root()];
        foreach ($opts as $k => $v) {
            if ($v === true) {
                $args[] = "--$k";
            } elseif ($v !== false && $v !== null) {
                $args[] = "--$k";
                $args[] = (string) $v;
            }
        }
        $r = self::exec($args, []);
        if ($r['code'] !== 0) {
            throw new \RuntimeException("gen_logs failed: {$r['stderr']}");
        }
        $sum = json_decode($r['stdout'], true);
        if (!is_array($sum)) {
            throw new \RuntimeException('gen_logs printed no JSON summary: ' . $r['stdout']);
        }
        return $sum;
    }

    /**
     * Runs bin/webpulse with this root's config.
     *
     * @param string $args shell-like argument string (split on spaces; quote with ')
     * @param array<string,string> $env extra environment
     * @return array{code:int,stdout:string,stderr:string,json:mixed}
     */
    public function run(string $args, array $env = []): array
    {
        $argv = self::split($args);
        $env += ['WEBPULSE_CONFIG' => $this->configPath()];
        if ($this->now !== null) {
            $env += ['WEBPULSE_NOW' => (string) $this->now];
        }
        $r = self::exec(array_merge([PHP_BINARY, self::repo() . '/bin/webpulse'], $argv), $env);
        $r['json'] = json_decode($r['stdout'], true);
        return $r;
    }

    /** Boots an App on this root in-process. */
    public function app(): App
    {
        return App::boot($this->configPath());
    }

    /** Read-write PDO on a domain database (throws when missing). */
    public function db(string $user, string $domain): \PDO
    {
        $f = $this->path("var/webpulse/data/$user/$domain.db");
        if (!is_file($f)) {
            throw new \RuntimeException("no database $f");
        }
        $pdo = new \PDO('sqlite:' . $f);
        $pdo->setAttribute(\PDO::ATTR_ERRMODE, \PDO::ERRMODE_EXCEPTION);
        return $pdo;
    }

    /** Single value of a query on a domain database. */
    public function scalar(string $user, string $domain, string $sql, array $params = []): mixed
    {
        $st = $this->db($user, $domain)->prepare($sql);
        $st->execute($params);
        return $st->fetchColumn();
    }

    /**
     * @param string[] $argv
     * @param array<string,string> $env
     * @return array{code:int,stdout:string,stderr:string}
     */
    public static function exec(array $argv, array $env): array
    {
        $fullEnv = getenv();
        unset($fullEnv['WEBPULSE_NOW'], $fullEnv['WEBPULSE_CONFIG']);
        $fullEnv = array_merge($fullEnv, $env);
        $proc = proc_open($argv, [1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes, self::repo(), $fullEnv);
        if (!is_resource($proc)) {
            throw new \RuntimeException('cannot run ' . implode(' ', $argv));
        }
        $out = stream_get_contents($pipes[1]);
        $err = stream_get_contents($pipes[2]);
        fclose($pipes[1]);
        fclose($pipes[2]);
        $code = proc_close($proc);
        return ['code' => $code, 'stdout' => (string) $out, 'stderr' => (string) $err];
    }

    /** @return string[] */
    private static function split(string $args): array
    {
        preg_match_all("/'([^']*)'|(\\S+)/", $args, $m, PREG_SET_ORDER);
        return array_map(static fn (array $x): string => ($x[2] ?? '') !== '' ? $x[2] : $x[1], $m);
    }

    private static function copy(string $src, string $dst): void
    {
        foreach (scandir($src) ?: [] as $e) {
            if ($e === '.' || $e === '..') {
                continue;
            }
            if (is_dir("$src/$e")) {
                @mkdir("$dst/$e", 0755, true);
                self::copy("$src/$e", "$dst/$e");
            } else {
                copy("$src/$e", "$dst/$e");
            }
        }
    }

    /** Recursively removes a directory (only below tests/tmp). */
    public static function rm(string $dir): void
    {
        if (!str_starts_with($dir, self::repo() . '/tests/tmp/')) {
            throw new \LogicException("refusing to delete $dir");
        }
        if (is_link($dir) || is_file($dir)) {
            unlink($dir);
            return;
        }
        if (!is_dir($dir)) {
            return;
        }
        foreach (scandir($dir) ?: [] as $e) {
            if ($e !== '.' && $e !== '..') {
                self::rm("$dir/$e");
            }
        }
        @chmod($dir, 0755);
        rmdir($dir);
    }
}
