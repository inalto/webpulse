<?php

declare(strict_types=1);

namespace WebPulse\Classify;

use WebPulse\Log;

/**
 * Country / city lookups through {@see MmdbReader}. Without a database every
 * lookup returns null (the collector then records "(unknown)").
 */
final class GeoIp
{
    private ?MmdbReader $country = null;
    private ?MmdbReader $city = null;
    /** @var array<string,?string> */
    private array $countryCache = [];
    /** @var array<string,?array<string,mixed>> */
    private array $cityCache = [];

    /** Opens the given databases; unusable files are logged and ignored. */
    public function __construct(?string $countryDb = null, ?string $cityDb = null, ?Log $log = null)
    {
        foreach (['country' => $countryDb, 'city' => $cityDb] as $prop => $file) {
            if ($file === null || $file === '' || !is_file($file)) {
                continue;
            }
            try {
                $this->{$prop} = new MmdbReader($file);
            } catch (\Throwable $e) {
                $log?->warn('geoip: cannot use database', ['file' => $file, 'error' => $e->getMessage()]);
            }
        }
    }

    /** True when at least one database is loaded. */
    public function available(): bool
    {
        return $this->country !== null || $this->city !== null;
    }

    /** True when a city database is loaded. */
    public function hasCity(): bool
    {
        return $this->city !== null;
    }

    /** Lowercase ISO-3166 alpha-2 code, or null. */
    public function country(string $ip): ?string
    {
        if (array_key_exists($ip, $this->countryCache)) {
            return $this->countryCache[$ip];
        }
        if (count($this->countryCache) > 50000) {
            $this->countryCache = [];
        }
        $code = null;
        $reader = $this->country ?? $this->city;
        if ($reader !== null) {
            try {
                $rec = $reader->get($ip);
                $code = self::iso($rec);
            } catch (\Throwable) {
                $code = null;
            }
        }
        return $this->countryCache[$ip] = $code;
    }

    /**
     * City record: ['name' => 'Rome', 'country' => 'it', 'region' => 'Lazio'|null,
     * 'lat' => float|null, 'lon' => float|null], or null without a city database.
     *
     * @return array{name:string,country:?string,region:?string,lat:?float,lon:?float}|null
     */
    public function city(string $ip): ?array
    {
        if ($this->city === null) {
            return null;
        }
        if (array_key_exists($ip, $this->cityCache)) {
            return $this->cityCache[$ip];
        }
        if (count($this->cityCache) > 50000) {
            $this->cityCache = [];
        }
        $out = null;
        try {
            $rec = $this->city->get($ip);
            if (is_array($rec) && isset($rec['city']['names']) && is_array($rec['city']['names'])) {
                $names = $rec['city']['names'];
                $name = $names['en'] ?? reset($names);
                if (is_string($name) && $name !== '') {
                    $region = $rec['subdivisions'][0]['names']['en'] ?? null;
                    $out = [
                        'name' => $name,
                        'country' => self::iso($rec),
                        'region' => is_string($region) ? $region : null,
                        'lat' => isset($rec['location']['latitude']) ? (float) $rec['location']['latitude'] : null,
                        'lon' => isset($rec['location']['longitude']) ? (float) $rec['location']['longitude'] : null,
                    ];
                }
            }
        } catch (\Throwable) {
            $out = null;
        }
        return $this->cityCache[$ip] = $out;
    }

    /** Metadata of the country (or city) database, null without one. */
    public function metadata(): ?array
    {
        $r = $this->country ?? $this->city;
        return $r?->metadata();
    }

    private static function iso(mixed $rec): ?string
    {
        if (!is_array($rec)) {
            return null;
        }
        foreach (['country', 'registered_country', 'represented_country'] as $k) {
            $c = $rec[$k]['iso_code'] ?? null;
            if (is_string($c) && preg_match('/^[A-Za-z]{2}$/', $c)) {
                return strtolower($c);
            }
        }
        return null;
    }
}
