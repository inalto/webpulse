<?php

declare(strict_types=1);

namespace WebPulse\Collect;

use WebPulse\Paths;
use WebPulse\Util\Fs;
use WebPulse\Util\Ip;
use WebPulse\Util\Json;

/**
 * Per-account settings file /var/webpulse/data/<user>/user/settings.json,
 * written by the cPanel plugin (as the account) and read by the collector
 * (as root) with symlink, size and schema checks.
 *
 * Schema (DATA-MODEL.md):
 *   { "version": 1,
 *     "domains": { "<domain>": { "timezone": ?string, "exclude_ips": string[≤50], "exclude_paths": string[≤50] } },
 *     "ui": { "default_range": "30d", "dark": "auto"|"light"|"dark" } }
 */
final class UserSettings
{
    public const MAX_BYTES = 65536;
    public const MAX_ITEMS = 50;
    public const MAX_DOMAINS = 500;
    public const RANGES = ['today', 'yesterday', '7d', '30d', '90d', '12m', 'month', 'lastmonth', 'year', 'all'];

    /** @return array{version:int,domains:array<string,array{timezone:?string,exclude_ips:string[],exclude_paths:string[]}>,ui:array{default_range:string,dark:string}} */
    public static function defaults(): array
    {
        return ['version' => 1, 'domains' => [], 'ui' => ['default_range' => '30d', 'dark' => 'auto']];
    }

    /** Settings of one domain with defaults. */
    public static function domainDefaults(): array
    {
        return ['timezone' => null, 'exclude_ips' => [], 'exclude_paths' => []];
    }

    /**
     * Reads and validates the settings of $user. Any problem (missing file,
     * symlink, too large, invalid JSON or schema) yields the defaults; the
     * reason is available through $error.
     *
     * @return array<string,mixed>
     */
    public static function read(Paths $p, string $user, ?string &$error = null): array
    {
        $error = null;
        try {
            $dir = $p->userPrivateDir($user);
            $file = $p->userSettingsFile($user);
        } catch (\InvalidArgumentException $e) {
            $error = $e->getMessage();
            return self::defaults();
        }
        $dst = @lstat($dir);
        if ($dst === false) {
            return self::defaults();
        }
        if (($dst['mode'] & 0170000) !== 0040000) {
            $error = 'settings directory is not a directory';
            return self::defaults();
        }
        $st = @lstat($file);
        if ($st === false) {
            return self::defaults();
        }
        if (($st['mode'] & 0170000) !== 0100000) {
            $error = 'settings.json is not a regular file';
            return self::defaults();
        }
        if ($st['size'] > self::MAX_BYTES) {
            $error = 'settings.json is larger than 64 KiB';
            return self::defaults();
        }
        // The account may swap the file (even for a FIFO) between lstat() and
        // the open: non-blocking open + fstat() identity check + bounded read.
        try {
            $raw = Fs::readRegular($file, self::MAX_BYTES, $st);
        } catch (\RuntimeException $e) {
            $error = str_contains($e->getMessage(), 'larger than') ? 'settings.json is larger than 64 KiB'
                : (str_contains($e->getMessage(), 'cannot open') ? 'settings.json is not readable' : 'settings.json changed while reading');
            return self::defaults();
        }
        try {
            $data = Json::decode($raw);
            if (!is_array($data)) {
                throw new \InvalidArgumentException('settings must be a JSON object');
            }
            return self::validate($data);
        } catch (\JsonException $e) {
            $error = 'invalid JSON: ' . $e->getMessage();
        } catch (\InvalidArgumentException $e) {
            $error = $e->getMessage();
        }
        return self::defaults();
    }

    /**
     * Validates raw settings and returns them normalised with defaults.
     *
     * @param array<string,mixed> $raw
     * @return array<string,mixed>
     * @throws \InvalidArgumentException listing every problem ("; " separated)
     */
    public static function validate(array $raw): array
    {
        $errors = [];
        $out = self::defaults();
        foreach (array_keys($raw) as $k) {
            if (!in_array($k, ['version', 'domains', 'ui'], true)) {
                $errors[] = "unknown key '" . self::printable($k) . "'";
            }
        }
        if (isset($raw['version']) && $raw['version'] !== 1) {
            $errors[] = 'version must be 1';
        }
        $domains = $raw['domains'] ?? [];
        if (!is_array($domains) || ($domains !== [] && array_is_list($domains))) {
            $errors[] = 'domains must be an object';
            $domains = [];
        }
        if (count($domains) > self::MAX_DOMAINS) {
            $errors[] = 'too many domains';
            $domains = [];
        }
        $tzs = null;
        foreach ($domains as $d => $cfg) {
            $d = strtolower((string) $d);
            if (!Paths::isValidDomain($d)) {
                $errors[] = 'invalid domain ' . json_encode($d);
                continue;
            }
            if (!is_array($cfg) || ($cfg !== [] && array_is_list($cfg))) {
                $errors[] = "$d: must be an object";
                continue;
            }
            $dc = self::domainDefaults();
            foreach (array_keys($cfg) as $k) {
                if (!in_array($k, ['timezone', 'exclude_ips', 'exclude_paths'], true)) {
                    $errors[] = "$d: unknown key '" . self::printable($k) . "'";
                }
            }
            $tz = $cfg['timezone'] ?? null;
            if ($tz !== null && $tz !== '') {
                $tzs ??= array_flip(\DateTimeZone::listIdentifiers());
                if (!is_string($tz) || !isset($tzs[$tz])) {
                    $errors[] = "$d: invalid timezone " . json_encode($tz);
                } else {
                    $dc['timezone'] = $tz;
                }
            }
            foreach (['exclude_ips', 'exclude_paths'] as $k) {
                $list = $cfg[$k] ?? [];
                if (!is_array($list) || !array_is_list($list)) {
                    $errors[] = "$d: $k must be a list";
                    continue;
                }
                if (count($list) > self::MAX_ITEMS) {
                    $errors[] = "$d: $k accepts at most " . self::MAX_ITEMS . ' entries';
                    continue;
                }
                $clean = [];
                foreach ($list as $v) {
                    $v = is_string($v) ? trim($v) : $v;
                    $ok = $k === 'exclude_ips'
                        ? is_string($v) && Ip::isValidCidr($v)
                        : is_string($v) && $v !== '' && $v[0] === '/' && strlen($v) <= 255 && !preg_match('/[\x00-\x1f]/', $v);
                    if (!$ok) {
                        $errors[] = "$d: invalid " . ($k === 'exclude_ips' ? 'IP/CIDR ' : 'path prefix ') . json_encode($v);
                        continue;
                    }
                    $clean[] = $v;
                }
                $dc[$k] = array_values(array_unique($clean));
            }
            $out['domains'][$d] = $dc;
        }
        $ui = $raw['ui'] ?? [];
        if (!is_array($ui) || ($ui !== [] && array_is_list($ui))) {
            $errors[] = 'ui must be an object';
            $ui = [];
        }
        foreach (array_keys($ui) as $k) {
            if (!in_array($k, ['default_range', 'dark'], true)) {
                $errors[] = "ui: unknown key '" . self::printable($k) . "'";
            }
        }
        if (isset($ui['default_range'])) {
            if (!in_array($ui['default_range'], self::RANGES, true)) {
                $errors[] = 'ui.default_range must be one of ' . implode(', ', self::RANGES);
            } else {
                $out['ui']['default_range'] = $ui['default_range'];
            }
        }
        if (array_key_exists('dark', $ui)) {
            $dark = match ($ui['dark']) {
                'auto', null => 'auto',
                'dark', 'on', true => 'dark',
                'light', 'off', false => 'light',
                default => null,
            };
            if ($dark === null) {
                $errors[] = 'ui.dark must be auto, light or dark';
            } else {
                $out['ui']['dark'] = $dark;
            }
        }
        if ($errors !== []) {
            throw new \InvalidArgumentException(implode('; ', $errors));
        }
        ksort($out['domains']);
        return $out;
    }

    /** A user-supplied key made safe for messages and logs (no control characters, ≤ 64 bytes). */
    private static function printable(int|string $k): string
    {
        return \WebPulse\Util\Str::truncate(\WebPulse\Util\Str::clean((string) $k), 64, "\u{2026}");
    }

    /**
     * Settings of one domain (defaults when absent).
     *
     * @param array<string,mixed> $settings validated settings
     * @return array{timezone:?string,exclude_ips:string[],exclude_paths:string[]}
     */
    public static function forDomain(array $settings, string $domain): array
    {
        return $settings['domains'][strtolower($domain)] ?? self::domainDefaults();
    }

    /**
     * Validates and atomically writes the settings of $user. Meant for the
     * cPanel plugin, which runs as the account (the file stays owned by it).
     *
     * @param array<string,mixed> $raw
     * @return array<string,mixed> the normalised settings written
     * @throws \InvalidArgumentException|\RuntimeException
     */
    public static function write(Paths $p, string $user, array $raw): array
    {
        $data = self::validate($raw);
        $json = Json::encode($data, true) . "\n";
        if (strlen($json) > self::MAX_BYTES) {
            throw new \InvalidArgumentException('settings larger than 64 KiB');
        }
        $dir = $p->userPrivateDir($user);
        if (is_link($dir) || !is_dir($dir)) {
            throw new \RuntimeException('settings directory missing');
        }
        $file = $p->userSettingsFile($user);
        if (is_link($file)) {
            @unlink($file);
        }
        Fs::writeAtomic($file, $json, 0600);
        return $data;
    }
}
