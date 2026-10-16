<?php

declare(strict_types=1);

namespace WebPulse;

use WebPulse\Classify\GeoIp;
use WebPulse\Discovery\CpanelUsers;
use WebPulse\Registry\Registry;
use WebPulse\Store\DomainStore;

/**
 * Application container: configuration, paths, logger and lazily opened
 * services. The only global state in WebPulse; built once by the entry point.
 */
final class App
{
    public const DEFAULT_CONFIG = '/etc/webpulse/config.json';

    public Config $config;
    public Paths $paths;
    public Log $log;

    private ?Registry $registry = null;
    private ?CpanelUsers $cpanelUsers = null;
    private ?GeoIp $geoIp = null;
    private ?string $version = null;

    private function __construct(Config $config, private ?string $configPath)
    {
        $this->config = $config;
        $this->paths = new Paths($config, dirname(__DIR__, 2));
        $this->log = new Log($this->paths->logDir());
        $config->attachOverrides(
            function (): array {
                if (!is_file($this->paths->registryDb())) {
                    return [];
                }
                return $this->registry()->settings();
            },
            function (string $k, mixed $v): void {
                $this->registry()->setSetting($k, $v);
            },
            function (string $k): void {
                $this->registry()->deleteSetting($k);
            }
        );
    }

    /**
     * Boots the application.
     *
     * @param string|null $configPath config file; null = $WEBPULSE_CONFIG, else /etc/webpulse/config.json
     * @throws \RuntimeException when an explicitly given config file is missing or invalid
     */
    public static function boot(?string $configPath = null): App
    {
        $explicit = $configPath !== null;
        if ($configPath === null) {
            $env = getenv('WEBPULSE_CONFIG');
            $configPath = is_string($env) && $env !== '' ? $env : self::DEFAULT_CONFIG;
            $explicit = is_string($env) && $env !== '';
        }
        $config = Config::load($configPath);
        if ($explicit && $config->loadError() === 'missing') {
            throw new \RuntimeException("config file not found: $configPath");
        }
        return new self($config, $configPath);
    }

    /** Path of the configuration file in use (it may not exist yet). */
    public function configPath(): ?string
    {
        return $this->configPath;
    }

    /** Registry database (opened and migrated on first use). */
    public function registry(): Registry
    {
        return $this->registry ??= new Registry($this->paths->registryDb(), $this->config, $this);
    }

    /**
     * Opens a domain database.
     *
     * @throws Store\NotFound when $readOnly and the database does not exist
     * @throws \InvalidArgumentException for invalid user/domain names
     */
    public function openDomain(string $user, string $domain, bool $readOnly): DomainStore
    {
        return DomainStore::open($this->paths, $user, $domain, $readOnly);
    }

    /** Version string from the VERSION file. */
    public function version(): string
    {
        if ($this->version === null) {
            $v = @file_get_contents($this->paths->root() . '/VERSION');
            $this->version = $v === false ? '0.0.0' : trim($v);
        }
        return $this->version;
    }

    /** cPanel account discovery (lazy). */
    public function cpanelUsers(): CpanelUsers
    {
        return $this->cpanelUsers ??= new CpanelUsers($this->config, $this->paths);
    }

    /** GeoIP lookups; returns an instance without databases when disabled. */
    public function geoIp(): GeoIp
    {
        if ($this->geoIp === null) {
            $on = (bool) $this->config->get('geoip.enabled', false);
            $country = $on ? $this->config->get('geoip.country_db') : null;
            $city = $on ? $this->config->get('geoip.city_db') : null;
            $this->geoIp = new GeoIp(is_string($country) ? $country : null, is_string($city) ? $city : null, $this->log);
        }
        return $this->geoIp;
    }

    /**
     * Current unix time. Tests pin it with the WEBPULSE_NOW environment
     * variable (unix timestamp or any strtotime() string).
     */
    public function now(): int
    {
        return self::clock();
    }

    /** Static variant of {@see App::now()}. */
    public static function clock(): int
    {
        $env = getenv('WEBPULSE_NOW');
        if (is_string($env) && $env !== '') {
            if (ctype_digit($env)) {
                return (int) $env;
            }
            $t = strtotime($env);
            if ($t !== false) {
                return $t;
            }
        }
        return time();
    }

    /** Server default timezone name (config timezone or PHP default). */
    public function serverTimezone(): string
    {
        $tz = $this->config->get('timezone');
        return is_string($tz) && $tz !== '' ? $tz : date_default_timezone_get();
    }
}
