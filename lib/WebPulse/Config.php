<?php

declare(strict_types=1);

namespace WebPulse;

use WebPulse\Util\Ip;
use WebPulse\Util\Json;

/**
 * Effective configuration: built-in defaults < config.json < registry.settings.
 *
 * Only the keys listed in {@see Config::EDITABLE} can be overridden from the
 * registry (WHM settings / `webpulse config set`); `paths.*` always come from
 * the file so that the registry location itself is never circular.
 */
final class Config
{
    /** Built-in defaults (same content as etc/config.json.default plus engine-only keys). */
    public const DEFAULTS = [
        'version' => 1,
        'default_enabled' => true,
        'timezone' => null,
        'raw_retention_days' => 30,
        'max_rows_per_dim' => 1000,
        'session_timeout_min' => 30,
        'store_ip' => false,
        'exclude_ips' => [],
        'exclude_paths' => ['/.well-known/', '/cpanel', '/whm', '/webmail', '/cgi-sys/'],
        'count_head_requests' => false,
        'static_prefixes' => ['/wp-content/uploads/', '/wp-includes/', '/wp-json/', '/wp-admin/admin-ajax.php', '/wp-cron.php', '/xmlrpc.php'],
        'geoip' => [
            'enabled' => false,
            'country_db' => '/var/webpulse/geoip/dbip-country-lite.mmdb',
            'city_db' => null,
            'url' => 'https://download.db-ip.com/free/dbip-country-lite-{YYYY}-{MM}.mmdb.gz',
        ],
        'php_bin' => null,
        'paths' => [
            'data_dir' => '/var/webpulse/data',
            'registry_db' => '/var/webpulse/webpulse.db',
            'log_dir' => '/var/webpulse/log',
            'run_dir' => '/var/webpulse/run',
            'domlogs_dirs' => ['/etc/apache2/logs/domlogs/{user}', '/etc/apache2/logs/domlogs', '/usr/local/apache/domlogs/{user}', '/var/log/nginx/domains'],
            'archive_dir' => '/home/{user}/logs',
            'userdomains_file' => '/etc/userdomains',
            'cpanel_users_dir' => '/var/cpanel/users',
            'cpanel_userdata_dir' => '/var/cpanel/userdata',
            'home_root' => '/home',
            'cron_file' => '/etc/cron.d/webpulse',
            'hooks_file' => '/var/cpanel/hooks.yaml',
        ],
    ];

    /** Keys that registry.settings may override, with their validators. */
    public const EDITABLE = [
        'default_enabled', 'timezone', 'raw_retention_days', 'max_rows_per_dim', 'session_timeout_min',
        'store_ip', 'exclude_ips', 'exclude_paths', 'count_head_requests', 'geoip.enabled', 'static_prefixes',
    ];

    /** @var array<string,mixed> */
    private array $file;
    /** @var array<string,mixed>|null dotted key => value */
    private ?array $overrides = null;
    /** @var (\Closure():array<string,mixed>)|null */
    private ?\Closure $overrideLoader = null;
    /** @var (\Closure(string,mixed):void)|null */
    private ?\Closure $overrideWriter = null;
    /** @var (\Closure(string):void)|null */
    private ?\Closure $overrideRemover = null;
    /** @var array<string,mixed>|null */
    private ?array $merged = null;

    /**
     * @param array<string,mixed> $fileData decoded config.json (may be empty)
     * @param string|null $path the file it came from (null = defaults only)
     */
    public function __construct(array $fileData = [], private ?string $path = null, private ?string $loadError = null)
    {
        $this->file = $fileData;
        if ($path !== null) {
            $this->resolveRelativePaths(dirname($path));
        }
    }

    /**
     * Loads a config file. A missing or unreadable file yields the defaults
     * (the cPanel plugin runs as the account and cannot read the root-only
     * file); a file that exists but is invalid JSON throws.
     *
     * @throws \RuntimeException
     */
    public static function load(?string $path): self
    {
        if ($path === null || $path === '') {
            return new self([], null);
        }
        if (!file_exists($path)) {
            return new self([], $path, 'missing');
        }
        if (!is_readable($path)) {
            return new self([], $path, 'unreadable');
        }
        $raw = @file_get_contents($path);
        if ($raw === false) {
            return new self([], $path, 'unreadable');
        }
        try {
            $data = Json::decode($raw);
        } catch (\JsonException $e) {
            throw new \RuntimeException("invalid JSON in $path: " . $e->getMessage());
        }
        if (!is_array($data)) {
            throw new \RuntimeException("$path must contain a JSON object");
        }
        return new self($data, $path);
    }

    /** Path of the loaded file (null when running on defaults). */
    public function path(): ?string
    {
        return $this->path;
    }

    /** "missing" / "unreadable" when the file could not be used, else null. */
    public function loadError(): ?string
    {
        return $this->loadError;
    }

    /** Wires the registry: loader of overrides, writer and remover (done by App). */
    public function attachOverrides(\Closure $loader, \Closure $writer, \Closure $remover): void
    {
        $this->overrideLoader = $loader;
        $this->overrideWriter = $writer;
        $this->overrideRemover = $remover;
        $this->overrides = null;
        $this->merged = null;
    }

    /**
     * Returns a value by dotted key (e.g. "geoip.enabled", "paths.data_dir"),
     * merged: defaults < file < registry.settings.
     */
    public function get(string $dottedKey, mixed $default = null): mixed
    {
        $v = self::dig($this->effective(), $dottedKey, $missing);
        return $missing ? $default : $v;
    }

    /**
     * The full merged configuration (for admin.settings / `config get`).
     *
     * @return array<string,mixed>
     */
    public function effective(): array
    {
        if ($this->merged !== null) {
            return $this->merged;
        }
        $m = self::mergeDeep(self::DEFAULTS, $this->file);
        foreach ($this->overrides() as $k => $v) {
            if (in_array($k, self::EDITABLE, true)) {
                self::set($m, $k, $v);
            }
        }
        return $this->merged = $m;
    }

    /**
     * Stores an override in registry.settings (validated).
     *
     * @throws \InvalidArgumentException for unknown keys or invalid values
     * @throws \RuntimeException when no registry is attached
     */
    public function setOverride(string $dottedKey, mixed $value): void
    {
        $value = self::validate($dottedKey, $value);
        if ($this->overrideWriter === null) {
            throw new \RuntimeException('no registry attached to the configuration');
        }
        ($this->overrideWriter)($dottedKey, $value);
        $this->overrides = null;
        $this->merged = null;
    }

    /** Removes an override (back to the file / default value). */
    public function unsetOverride(string $dottedKey): void
    {
        if ($this->overrideRemover === null) {
            throw new \RuntimeException('no registry attached to the configuration');
        }
        ($this->overrideRemover)($dottedKey);
        $this->overrides = null;
        $this->merged = null;
    }

    /** @return array<string,mixed> current registry overrides (dotted keys) */
    public function overrides(): array
    {
        if ($this->overrides === null) {
            $this->overrides = [];
            if ($this->overrideLoader !== null) {
                try {
                    $this->overrides = ($this->overrideLoader)();
                } catch (\Throwable) {
                    $this->overrides = [];
                }
            }
        }
        return $this->overrides;
    }

    /** Drops cached merged values (after an external change). */
    public function refresh(): void
    {
        $this->overrides = null;
        $this->merged = null;
    }

    /**
     * Validates and normalises a value for an editable key.
     *
     * @throws \InvalidArgumentException
     */
    public static function validate(string $key, mixed $value): mixed
    {
        if (!in_array($key, self::EDITABLE, true)) {
            throw new \InvalidArgumentException("setting '$key' cannot be changed at run time (edit config.json)");
        }
        $int = static function (mixed $v, int $min, int $max) use ($key): int {
            if (is_string($v) && preg_match('/^-?\d+$/', $v)) {
                $v = (int) $v;
            }
            if (!is_int($v) || $v < $min || $v > $max) {
                throw new \InvalidArgumentException("$key must be an integer between $min and $max");
            }
            return $v;
        };
        $bool = static function (mixed $v) use ($key): bool {
            if (is_bool($v)) {
                return $v;
            }
            if (in_array($v, ['1', 'true', 'on', 'yes', 1], true)) {
                return true;
            }
            if (in_array($v, ['0', 'false', 'off', 'no', 0], true)) {
                return false;
            }
            throw new \InvalidArgumentException("$key must be a boolean");
        };
        $list = static function (mixed $v, int $max, callable $ok, string $what) use ($key): array {
            if (!is_array($v) || !array_is_list($v)) {
                throw new \InvalidArgumentException("$key must be a list");
            }
            if (count($v) > $max) {
                throw new \InvalidArgumentException("$key accepts at most $max entries");
            }
            $out = [];
            foreach ($v as $e) {
                if (!is_string($e) || !$ok(trim($e))) {
                    throw new \InvalidArgumentException("$key: invalid $what " . json_encode($e));
                }
                $out[] = trim($e);
            }
            return array_values(array_unique($out));
        };
        $pathPrefix = static fn (string $p): bool => $p !== '' && $p[0] === '/' && strlen($p) <= 255 && !preg_match('/[\x00-\x1f]/', $p);
        return match ($key) {
            'default_enabled', 'store_ip', 'count_head_requests', 'geoip.enabled' => $bool($value),
            'raw_retention_days' => $int($value, 1, 3650),
            'max_rows_per_dim' => $int($value, 10, 100000),
            'session_timeout_min' => $int($value, 5, 240),
            'timezone' => self::validateTz($value),
            'exclude_ips' => $list($value, 500, [Ip::class, 'isValidCidr'], 'IP/CIDR'),
            'exclude_paths', 'static_prefixes' => $list($value, 200, $pathPrefix, 'path prefix'),
        };
    }

    private static function validateTz(mixed $v): ?string
    {
        if ($v === null || $v === '' || $v === 'null') {
            return null;
        }
        if (!is_string($v) || !in_array($v, \DateTimeZone::listIdentifiers(), true)) {
            throw new \InvalidArgumentException('timezone must be a valid identifier such as Europe/Rome');
        }
        return $v;
    }

    /**
     * @param array<string,mixed> $a
     * @param array<string,mixed> $b
     * @return array<string,mixed>
     */
    private static function mergeDeep(array $a, array $b): array
    {
        foreach ($b as $k => $v) {
            if (is_array($v) && !array_is_list($v) && isset($a[$k]) && is_array($a[$k]) && !array_is_list($a[$k])) {
                $a[$k] = self::mergeDeep($a[$k], $v);
            } else {
                $a[$k] = $v;
            }
        }
        return $a;
    }

    /** @param array<string,mixed> $a */
    private static function dig(array $a, string $key, ?bool &$missing = null): mixed
    {
        $missing = false;
        $cur = $a;
        foreach (explode('.', $key) as $part) {
            if (!is_array($cur) || !array_key_exists($part, $cur)) {
                $missing = true;
                return null;
            }
            $cur = $cur[$part];
        }
        return $cur;
    }

    /** @param array<string,mixed> $a */
    private static function set(array &$a, string $key, mixed $value): void
    {
        $parts = explode('.', $key);
        $ref = &$a;
        foreach ($parts as $i => $p) {
            if ($i === count($parts) - 1) {
                $ref[$p] = $value;
                break;
            }
            if (!isset($ref[$p]) || !is_array($ref[$p])) {
                $ref[$p] = [];
            }
            $ref = &$ref[$p];
        }
    }

    private function resolveRelativePaths(string $base): void
    {
        $fix = static fn (mixed $p): mixed => is_string($p) && $p !== '' && $p[0] !== '/' ? $base . '/' . $p : $p;
        if (isset($this->file['paths']) && is_array($this->file['paths'])) {
            foreach ($this->file['paths'] as $k => $v) {
                $this->file['paths'][$k] = is_array($v) ? array_map($fix, $v) : $fix($v);
            }
        }
        foreach (['country_db', 'city_db'] as $k) {
            if (isset($this->file['geoip'][$k])) {
                $this->file['geoip'][$k] = $fix($this->file['geoip'][$k]);
            }
        }
    }
}
