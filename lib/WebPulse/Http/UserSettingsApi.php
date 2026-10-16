<?php

declare(strict_types=1);

namespace WebPulse\Http;

use WebPulse\App;
use WebPulse\Collect\UserSettings;

/**
 * `a=user_settings` / `a=save_user_settings`: the cPanel account's own
 * per-domain settings file (`<data_dir>/<user>/user/settings.json`).
 * Only `user` principals may read or write it.
 */
final class UserSettingsApi
{
    /**
     * @return array<string,mixed>
     */
    public static function get(App $app, Request $r, Principal $p): array
    {
        if (!$p->isUser()) {
            throw ApiError::forbidden('Settings are managed by each account in cPanel');
        }
        return [
            'settings' => UserSettings::read($app->paths, $p->name),
            'remote_addr' => $r->remoteAddr,
            'domains' => array_map(static fn (DomainRef $d) => $d->domain, DomainResolver::visibleDomains($p, null, $app->paths)),
            'server_timezone' => $app->serverTimezone(),
        ];
    }

    /**
     * @return array<string,mixed>
     */
    public static function save(App $app, Request $r, Principal $p): array
    {
        if (!$p->isUser()) {
            throw ApiError::forbidden('Settings can only be changed from the cPanel account');
        }
        $body = $r->json();
        if ($body === []) {
            throw ApiError::badRequest('Request body with the settings is required');
        }
        $raw = isset($body['settings']) && is_array($body['settings']) ? $body['settings'] : $body;
        $visible = array_map(static fn (DomainRef $d) => $d->domain, DomainResolver::visibleDomains($p, null, $app->paths));
        foreach (array_keys(is_array($raw['domains'] ?? null) ? $raw['domains'] : []) as $dom) {
            if (!in_array((string) $dom, $visible, true)) {
                throw ApiError::badRequest('Unknown domain "' . substr((string) $dom, 0, 60) . '"');
            }
        }
        $raw['version'] = 1;
        $clean = UserSettings::validate($raw); // InvalidArgumentException → 400
        $json = json_encode($clean, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR) . "\n";
        if (strlen($json) > 65536) {
            throw ApiError::badRequest('Settings are too large');
        }
        self::writeAtomic($app->paths->userSettingsFile($p->name), $json);
        return ['settings' => $clean, 'saved' => true];
    }

    /**
     * Writes via a temp file in the same directory and rename(); refuses a
     * symlinked directory or target.
     */
    public static function writeAtomic(string $file, string $data): void
    {
        $dir = dirname($file);
        if (is_link($dir) || is_link($file)) {
            throw new ApiError(500, 'settings_unwritable', 'The settings location is not safe to write');
        }
        if (!is_dir($dir) || !is_writable($dir)) {
            throw new ApiError(500, 'settings_unwritable', 'The settings directory is missing or not writable; ask your administrator to run "webpulse setup"');
        }
        $tmp = @tempnam($dir, '.settings-');
        if ($tmp === false || dirname($tmp) !== $dir) {
            if (is_string($tmp)) {
                @unlink($tmp);
            }
            throw new ApiError(500, 'settings_unwritable', 'Could not create a temporary file for the settings');
        }
        if (@file_put_contents($tmp, $data) !== strlen($data) || !@chmod($tmp, 0644) || !@rename($tmp, $file)) {
            @unlink($tmp);
            throw new ApiError(500, 'settings_unwritable', 'Could not save the settings');
        }
    }
}
