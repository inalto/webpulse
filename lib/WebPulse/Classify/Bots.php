<?php

declare(strict_types=1);

namespace WebPulse\Classify;

/**
 * Bot detection from the User-Agent (METRICS.md "bot" class). An empty UA
 * (or "-") is a bot too.
 */
final class Bots
{
    /** Generic fragments meaning "automated client" (after the named table). */
    private const GENERIC = '~bot\b|bot[/_;:.)\- ]|crawl|spider|slurp|scrap|fetcher|scanner|preview|monitor|checker|http[-_ ]?client|headless~i';
    /** Real browsers / devices whose UA contains a generic fragment. */
    private const FALSE_POSITIVES = '~cubot|bot-?tle|robotics~i';

    /** @var array<int,array{0:string,1:string}> */
    private array $table;
    /** @var array<string,string|null> */
    private array $cache = [];

    /** Loads the table from data/bots.php. */
    public function __construct()
    {
        $this->table = require __DIR__ . '/data/bots.php';
    }

    /**
     * Bot family name when $ua is a bot, else null.
     * Empty UA -> "Empty user agent"; generic match -> "Other bot".
     */
    public function match(string $ua): ?string
    {
        if (array_key_exists($ua, $this->cache)) {
            return $this->cache[$ua];
        }
        if (count($this->cache) > 20000) {
            $this->cache = [];
        }
        return $this->cache[$ua] = $this->detect($ua);
    }

    private function detect(string $ua): ?string
    {
        $t = trim($ua);
        if ($t === '' || $t === '-') {
            return 'Empty user agent';
        }
        $l = strtolower($t);
        foreach ($this->table as [$needle, $name]) {
            if (str_contains($l, $needle)) {
                return $name;
            }
        }
        if (preg_match(self::GENERIC, $l) === 1 && preg_match(self::FALSE_POSITIVES, $l) !== 1) {
            return 'Other bot';
        }
        // no browser-like token at all: scripts and libraries
        if (!str_contains($l, 'mozilla/') && !str_contains($l, 'opera') && preg_match('~^[a-z][\w.-]*/[\d.]+\s*(\(|$)~', $l) === 1) {
            return 'Other bot';
        }
        return null;
    }
}
