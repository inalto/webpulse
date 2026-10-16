<?php

declare(strict_types=1);

namespace WebPulse\Classify;

use WebPulse\Util\Str;

/**
 * Referrer / acquisition classifier (METRICS.md "Acquisition").
 *
 * Channel precedence: paid > email > campaign(UTM) > direct > organic >
 * social > referral. utm_medium "social"/"organic" map to those channels.
 */
final class Referrer
{
    /** @var array<string,mixed> */
    private array $t;
    private string $searchRe;
    /** @var array<int,array{0:string,1:string,2:string[]}> */
    private array $search;
    /** @var array<int,array{0:string,1:string}> */
    private array $social;
    private string $emailRe;
    /** @var array<string,array<string,?string>> */
    private array $cache = [];

    /** Loads the tables from data/referrers.php. */
    public function __construct()
    {
        $this->t = require __DIR__ . '/data/referrers.php';
        $this->search = $this->t['search'];
        $this->social = $this->t['social'];
        $this->emailRe = '~^(?:' . implode('|', $this->t['email']) . ')$~';
        $this->searchRe = '';
    }

    /**
     * Parses a Referer value into [host (lowercase, "www." stripped), path, query];
     * null when it has no usable host. android-app://<package>/ is supported.
     *
     * @return array{0:string,1:string,2:string}|null
     */
    public static function split(string $ref): ?array
    {
        $ref = trim($ref);
        if ($ref === '' || $ref === '-') {
            return null;
        }
        if (!preg_match('~^([a-z][a-z0-9+.-]*):(?://)?([^/?#:]*)(?::\d+)?([^?#]*)(?:\?([^#]*))?~i', $ref, $m)) {
            return null;
        }
        $host = strtolower(rtrim($m[2], '.'));
        if ($host === '' || !preg_match('/^[a-z0-9._-]+\z/', $host)) {
            return null;
        }
        if (str_starts_with($host, 'www.')) {
            $host = substr($host, 4);
        }
        return [$host, $m[3] ?? '', $m[4] ?? ''];
    }

    /** True when $host (www. stripped) is the domain itself or its www. variant. */
    public static function isInternal(string $host, string $domain): bool
    {
        $d = strtolower($domain);
        if (str_starts_with($d, 'www.')) {
            $d = substr($d, 4);
        }
        return $host === $d;
    }

    /** External referrer host of a hit ("www." stripped), null when none or internal. */
    public function externalHost(?string $ref, string $domain): ?string
    {
        if ($ref === null) {
            return null;
        }
        $p = self::split($ref);
        if ($p === null || self::isInternal($p[0], $domain)) {
            return null;
        }
        return Str::key($p[0]);
    }

    /** Referrer host including internal ones (for not_found_referrer), null when none. */
    public function anyHost(?string $ref): ?string
    {
        if ($ref === null) {
            return null;
        }
        $p = self::split($ref);
        return $p === null ? null : Str::key($p[0]);
    }

    /**
     * Classifies the first hit of a visit.
     *
     * @param array{utm_source:?string,utm_medium:?string,utm_campaign:?string,click_id:bool} $utm
     * @return array{channel:string,referrer_host:?string,referrer_url:?string,search_engine:?string,keyword:?string,social:?string}
     */
    public function classify(?string $ref, string $domain, array $utm): array
    {
        $out = ['channel' => 'direct', 'referrer_host' => null, 'referrer_url' => null, 'search_engine' => null, 'keyword' => null, 'social' => null];
        $p = $ref === null ? null : self::split($ref);
        $external = $p !== null && !self::isInternal($p[0], $domain);
        $kind = null;
        if ($external) {
            [$host, $path, $query] = $p;
            $out['referrer_host'] = Str::key($host);
            $info = $this->hostInfo($host);
            $kind = $info['kind'];
            if ($kind === 'search') {
                $out['search_engine'] = $info['name'];
                $out['keyword'] = $this->keyword($query, $info['params']);
            } elseif ($kind === 'social') {
                $out['social'] = $info['name'];
            }
            if ($kind === null || $kind === 'social') {
                $out['referrer_url'] = Str::key($host . ($path === '' ? '/' : $path));
            }
        }
        $medium = $utm['utm_medium'] !== null ? strtolower($utm['utm_medium']) : null;
        $hasUtm = $utm['utm_source'] !== null || $utm['utm_medium'] !== null || $utm['utm_campaign'] !== null;
        if (($medium !== null && in_array($medium, $this->t['paid_mediums'], true)) || $utm['click_id']) {
            $out['channel'] = 'paid';
        } elseif (($medium !== null && in_array($medium, $this->t['email_mediums'], true)) || $kind === 'email') {
            $out['channel'] = 'email';
        } elseif ($medium !== null && in_array($medium, $this->t['social_mediums'], true)) {
            $out['channel'] = 'social';
        } elseif ($medium !== null && in_array($medium, $this->t['organic_mediums'], true)) {
            $out['channel'] = 'organic';
        } elseif ($hasUtm) {
            $out['channel'] = 'campaign';
        } elseif (!$external) {
            $out['channel'] = 'direct';
        } elseif ($kind === 'search') {
            $out['channel'] = 'organic';
        } elseif ($kind === 'social') {
            $out['channel'] = 'social';
        } else {
            $out['channel'] = 'referral';
        }
        return $out;
    }

    /**
     * @return array{kind:?string,name:?string,params:string[]}
     */
    public function hostInfo(string $host): array
    {
        if (isset($this->cache[$host])) {
            return $this->cache[$host];
        }
        if (count($this->cache) > 20000) {
            $this->cache = [];
        }
        $r = ['kind' => null, 'name' => null, 'params' => []];
        if (preg_match($this->emailRe, $host) === 1) {
            $r = ['kind' => 'email', 'name' => null, 'params' => []];
        } else {
            foreach ($this->search as [$re, $name, $params]) {
                if (preg_match('~^(?:' . $re . ')$~', $host) === 1) {
                    $r = ['kind' => 'search', 'name' => $name, 'params' => $params];
                    break;
                }
            }
            if ($r['kind'] === null) {
                foreach ($this->social as [$re, $name]) {
                    if (preg_match('~^(?:' . $re . ')$~', $host) === 1) {
                        $r = ['kind' => 'social', 'name' => $name, 'params' => []];
                        break;
                    }
                }
            }
        }
        return $this->cache[$host] = $r;
    }

    /** @param string[] $params */
    private function keyword(string $query, array $params): ?string
    {
        if ($query === '' || $params === []) {
            return null;
        }
        parse_str($query, $q);
        foreach ($params as $p) {
            if (isset($q[$p]) && is_string($q[$p])) {
                $k = trim(preg_replace('/\s+/u', ' ', Str::clean($q[$p])) ?? '');
                if ($k !== '') {
                    return Str::key(self::lower($k), 128);
                }
            }
        }
        return null;
    }

    /** Lowercases UTF-8 text with mbstring when available, else ASCII only. */
    private static function lower(string $s): string
    {
        return function_exists('mb_strtolower') ? mb_strtolower($s, 'UTF-8') : strtolower($s);
    }
}
