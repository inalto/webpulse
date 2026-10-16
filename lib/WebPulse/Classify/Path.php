<?php

declare(strict_types=1);

namespace WebPulse\Classify;

use WebPulse\Util\Str;

/**
 * Request-target normaliser (METRICS.md "Path normalisation"): extracts UTM
 * parameters and ad click ids, strips the query, decodes once, collapses
 * slashes and trailing index files, truncates to 512 bytes, forces UTF-8;
 * derives file_type, is_static and is_download.
 */
final class Path
{
    public const MAX_PATH = 512;
    public const INVALID = '(invalid request)';

    /** @var array<string,bool> */
    private array $download;
    /** @var array<string,bool> */
    private array $static;
    /** @var array<string,bool> */
    private array $page;
    /** @var string[] */
    private array $clickIds;
    /** @var array<string,array<string,mixed>> */
    private array $cache = [];

    /**
     * @param string[] $staticPrefixes path prefixes treated as static assets
     */
    public function __construct(private array $staticPrefixes = ['/wp-content/uploads/', '/wp-includes/', '/wp-json/'])
    {
        $ext = require __DIR__ . '/data/extensions.php';
        $this->download = array_fill_keys($ext['download'], true);
        $this->static = array_fill_keys($ext['static'], true);
        $this->page = array_fill_keys($ext['page'], true);
        $this->clickIds = (require __DIR__ . '/data/referrers.php')['click_ids'];
    }

    /**
     * @return array{path:string,key:string,file_type:string,is_static:bool,is_download:bool,utm_source:?string,utm_medium:?string,utm_campaign:?string,click_id:bool,scheme:?string}
     */
    public function analyze(string $target): array
    {
        if (isset($this->cache[$target])) {
            return $this->cache[$target];
        }
        if (count($this->cache) > 50000) {
            $this->cache = [];
        }
        return $this->cache[$target] = $this->compute($target);
    }

    /** @return array<string,mixed> */
    private function compute(string $target): array
    {
        $r = ['path' => self::INVALID, 'key' => self::INVALID, 'file_type' => 'html', 'is_static' => false, 'is_download' => false,
            'utm_source' => null, 'utm_medium' => null, 'utm_campaign' => null, 'click_id' => false, 'scheme' => null];
        if ($target === '') {
            return $r;
        }
        if ($target === '*') {
            $r['path'] = $r['key'] = '*';
            return $r;
        }
        // absolute-form request target (proxies): http://host/path
        if (preg_match('~^(https?)://[^/?#]*(.*)$~i', $target, $m)) {
            $r['scheme'] = strtolower($m[1]);
            $target = $m[2] === '' ? '/' : $m[2];
        }
        $hash = strpos($target, '#');
        if ($hash !== false) {
            $target = substr($target, 0, $hash);
        }
        $qpos = strpos($target, '?');
        $path = $qpos === false ? $target : substr($target, 0, $qpos);
        if ($qpos !== false) {
            $query = substr($target, $qpos + 1);
            if ($query !== '' && (str_contains($query, 'utm_') || str_contains($query, 'clid') || str_contains($query, 'braid') || str_contains($query, 'li_fat_id'))) {
                parse_str($query, $q);
                foreach (['utm_source', 'utm_medium', 'utm_campaign'] as $k) {
                    if (isset($q[$k]) && is_string($q[$k]) && trim($q[$k]) !== '') {
                        $r[$k] = Str::key(trim($q[$k]), 128);
                    }
                }
                foreach ($this->clickIds as $c) {
                    if (isset($q[$c])) {
                        $r['click_id'] = true;
                        break;
                    }
                }
            }
        }
        if (str_contains($path, '%')) {
            $path = rawurldecode($path);
        }
        if ($path === '' || $path[0] !== '/') {
            $path = '/' . $path;
        }
        if (str_contains($path, '//')) {
            $path = (string) preg_replace('~/{2,}~', '/', $path);
        }
        $path = Str::truncate(Str::clean($path), self::MAX_PATH);
        $path = (string) preg_replace('~/index\.(?:php|html?)$~i', '/', $path);
        $r['path'] = $path;
        $r['key'] = Str::truncate($path, 255, "\u{2026}");

        $base = substr($path, (int) strrpos($path, '/') + 1);
        $dot = strrpos($base, '.');
        $ext = $dot === false ? '' : strtolower(substr($base, $dot + 1));
        if ($ext !== '' && strlen($ext) <= 8 && ctype_alnum($ext)) {
            $r['file_type'] = isset($this->page[$ext]) ? 'html' : $ext;
            $r['is_download'] = isset($this->download[$ext]);
            $r['is_static'] = isset($this->static[$ext]);
        }
        if (!$r['is_static']) {
            foreach ($this->staticPrefixes as $p) {
                if ($p !== '' && str_starts_with($path, $p)) {
                    $r['is_static'] = true;
                    break;
                }
            }
        }
        return $r;
    }
}
