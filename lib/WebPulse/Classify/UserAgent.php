<?php

declare(strict_types=1);

namespace WebPulse\Classify;

/**
 * User-Agent classifier: browser, browser_version ("Chrome 129"), os,
 * os_version ("Android 14"), device (desktop|mobile|tablet|bot|other) and the
 * bot family name (null for humans).
 */
final class UserAgent
{
    /** @var array<string,mixed> */
    private array $tables;
    /** @var array<string,array<string,?string>> */
    private array $cache = [];

    /** Loads the tables from data/user_agents.php. */
    public function __construct(private Bots $bots = new Bots())
    {
        $this->tables = require __DIR__ . '/data/user_agents.php';
    }

    /**
     * @return array{browser:string,browser_version:string,os:string,os_version:string,device:string,bot:?string}
     */
    public function parse(string $ua): array
    {
        if (isset($this->cache[$ua])) {
            return $this->cache[$ua];
        }
        if (count($this->cache) > 20000) {
            $this->cache = [];
        }
        $bot = $this->bots->match($ua);
        if ($bot !== null) {
            return $this->cache[$ua] = ['browser' => 'Bot', 'browser_version' => 'Bot', 'os' => 'Other', 'os_version' => 'Other', 'device' => 'bot', 'bot' => $bot];
        }
        [$os, $osv] = $this->os($ua);
        [$b, $bv] = $this->browser($ua);
        return $this->cache[$ua] = [
            'browser' => $b,
            'browser_version' => $bv,
            'os' => $os,
            'os_version' => $osv,
            'device' => $this->device($ua, $os),
            'bot' => null,
        ];
    }

    /** @return array{0:string,1:string} */
    private function browser(string $ua): array
    {
        foreach ($this->tables['in_app'] as [$re, $app]) {
            if (preg_match($re, $ua) === 1) {
                return ['In-app', 'In-app ' . $app];
            }
        }
        foreach ($this->tables['browsers'] as [$re, $name]) {
            if (preg_match($re, $ua, $m) === 1) {
                $v = $m[1] ?? '';
                if ($name === 'IE' && $v !== '' && (int) $v < 5) {
                    continue;
                }
                return [$name, $v !== '' ? $name . ' ' . $v : $name];
            }
        }
        return ['Other', 'Other'];
    }

    /** @return array{0:string,1:string} */
    private function os(string $ua): array
    {
        if (preg_match('~Windows Phone(?: OS)? ([\d.]+)?~', $ua)) {
            return ['Windows', 'Windows Phone'];
        }
        if (preg_match('~Windows NT (\d+\.\d)~', $ua, $m)) {
            return ['Windows', $this->tables['windows'][$m[1]] ?? 'Windows NT ' . $m[1]];
        }
        if (str_contains($ua, 'Windows')) {
            return ['Windows', 'Windows'];
        }
        if (preg_match('~(?:iPhone|iPad|iPod)(?:.*?) OS (\d+)[_\d]*~', $ua, $m)) {
            return ['iOS', 'iOS ' . $m[1]];
        }
        if (preg_match('~iPhone|iPad|iPod~', $ua)) {
            return ['iOS', 'iOS'];
        }
        if (preg_match('~Android[ /]?(\d+)?~', $ua, $m)) {
            return ['Android', isset($m[1]) && $m[1] !== '' ? 'Android ' . $m[1] : 'Android'];
        }
        if (str_contains($ua, 'CrOS')) {
            return ['ChromeOS', 'ChromeOS'];
        }
        if (preg_match('~Mac OS X (\d+)[_.](\d+)~', $ua, $m)) {
            $major = (int) $m[1];
            if ($major === 10) {
                // Safari/Chrome/Firefox freeze the reported version at 10.15
                return ['macOS', (int) $m[2] >= 15 ? 'macOS 10.15+' : 'macOS 10.' . $m[2]];
            }
            return ['macOS', 'macOS ' . $major];
        }
        if (str_contains($ua, 'Macintosh') || str_contains($ua, 'Mac OS')) {
            return ['macOS', 'macOS'];
        }
        if (preg_match('~Linux|X11|Ubuntu|Fedora|FreeBSD|OpenBSD~', $ua)) {
            return ['Linux', 'Linux'];
        }
        return ['Other', 'Other'];
    }

    private function device(string $ua, string $os): string
    {
        if (preg_match('~iPad|Tablet|Kindle|Silk/|PlayBook|Nexus (?:7|9|10)\b|SM-T\d|Tab\b~', $ua)) {
            return 'tablet';
        }
        if ($os === 'Android' && !str_contains($ua, 'Mobile')) {
            return 'tablet';
        }
        if (preg_match('~Mobi|iPhone|iPod|Windows Phone|BlackBerry|BB10|Opera Mini|IEMobile|webOS~', $ua)) {
            return 'mobile';
        }
        if ($os === 'Android') {
            return 'mobile';
        }
        if (in_array($os, ['Windows', 'macOS', 'Linux', 'ChromeOS'], true)) {
            return preg_match('~SmartTV|SMART-TV|Tizen|Web0S|PlayStation|Xbox|Nintendo~i', $ua) ? 'other' : 'desktop';
        }
        return 'other';
    }
}
