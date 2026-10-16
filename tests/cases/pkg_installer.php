<?php

declare(strict_types=1);

/*
 * install.sh end to end inside a temp prefix with a fake cPanel root
 * (WEBPULSE_PREFIX / WEBPULSE_FAKE_CPANEL / stub cPanel binaries that log their
 * invocations) and a stub WebPulse binary in the release layout. Download tests use
 * file:// URLs and a fake curl/uname on PATH. Nothing outside tests/tmp is touched.
 */

require_once __DIR__ . '/pkg_support.php';

$repo = PkgSupport::repo();

/** Names of the files/dirs that belong to WebPulse and must be gone after an uninstall. */
$leftovers = static function (array $p): array {
    $found = [];
    foreach ([$p['prefix'], $p['cron'], $p['apps'], $p['cpanel']] as $root) {
        foreach (PkgSupport::listTree($root) as $rel) {
            if (str_contains($rel, 'webpulse') && !str_starts_with($rel, '3rdparty')) {
                $found[] = $root . '/' . $rel;
            }
        }
    }
    return $found;
};

$count = static fn (array $log, string $prefix): int => count(array_filter($log, static fn ($l) => str_starts_with($l, $prefix)));

return [
    'pkg_installer: --help exits 0 and documents every flag' => static function () use ($repo): void {
        [$rc, $out] = PkgSupport::run(['bash', "$repo/install.sh", '--help']);
        PkgSupport::assertSame(0, $rc, 'exit status');
        foreach (['--geoip', '--ref', '--repo', '--build', '--binary', '--no-cron', '--update', '--uninstall', '--purge', '--standalone', '--all-themes', '--theme', '-y', '--help'] as $flag) {
            PkgSupport::assertContains($flag, $out, 'help text');
        }
    },

    'pkg_installer: bad flags are rejected with exit 2, nothing is touched' => static function () use ($repo): void {
        foreach ([['--bogus'], ['--ref'], ['--repo', 'a/b/c'], ['--repo', 'nonsense'], ['--ref', '../x'], ['--purge'], ['--binary'],
            ['--binary', '/nonexistent/webpulse'], ['--build', '--binary', __FILE__]] as $args) {
            [$rc, $out] = PkgSupport::run(array_merge(['bash', "$repo/install.sh"], $args), ['WEBPULSE_PREFIX' => PkgSupport::tmpDir('noop')]);
            PkgSupport::assertSame(2, $rc, implode(' ', $args) . "\n$out");
        }
    },

    'pkg_installer: refuses to run as non-root without a test prefix' => static function (): void {
        if (function_exists('posix_geteuid') && posix_geteuid() === 0) {
            fwrite(STDERR, "SKIP running as root\n");
            return;
        }
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::run(['bash', $p['src'] . '/install.sh', '-y']);
        PkgSupport::assertSame(1, $rc, 'exit status');
        PkgSupport::assertContains('must be run as root', $out);
    },

    'pkg_installer: fails clearly when cPanel is missing (and --standalone is not given)' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::rmrf($p['cpanel']);
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('cPanel & WHM not found', $out);
        PkgSupport::assertContains('--standalone', $out);
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse');
    },

    'pkg_installer: no PHP is needed to install; cPanel PHP is only an informational check' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        unlink($p['cpanel'] . '/3rdparty/bin/php');
        // a PATH without any php
        $bin = PkgSupport::tmpDir('nophp-bin');
        foreach (['bash', 'sh', 'env', 'tar', 'cp', 'mv', 'ln', 'rm', 'mkdir', 'chmod', 'chown', 'find', 'sed', 'grep', 'diff', 'awk', 'cut', 'tr',
            'head', 'date', 'id', 'uname', 'cat', 'sort', 'readlink', 'dirname', 'basename', 'hostname', 'cmp', 'mktemp', 'sha256sum', 'touch', 'printf', 'wc', 'tail', 'sleep', 'nohup', 'gzip'] as $tool) {
            [$rc, $path] = PkgSupport::run(['sh', '-c', 'command -v "$1"', 'x', $tool]);
            if ($rc === 0 && trim($path) !== '' && str_starts_with(trim($path), '/')) {
                @symlink(trim($path), "$bin/$tool");
            }
        }
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y'], ['PATH' => $bin]);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains("cPanel's PHP", $out, 'informational warning');
        PkgSupport::assertContains('WebPulse 0.1.0 is installed', $out);
        PkgSupport::assertTrue(is_link($p['prefix'] . '/usr/local/webpulse'), 'installed');
    },

    'pkg_installer: fresh install registers everything' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('WebPulse 0.1.0 is installed', $out);
        PkgSupport::assertContains(':2087/cgi/webpulse/index.cgi', $out);

        $link = $p['prefix'] . '/usr/local/webpulse';
        $rel = $p['prefix'] . '/usr/local/webpulse-releases/0.1.0';
        PkgSupport::assertTrue(is_link($link), 'install dir is a symlink');
        PkgSupport::assertSame($rel, readlink($link), 'symlink target');

        // the binary is installed as is (no shebang rewriting), root-owned 0755
        PkgSupport::assertSame(file_get_contents($p['src'] . '/bin/webpulse'), file_get_contents("$rel/bin/webpulse"), 'binary copied unchanged');
        PkgSupport::assertSame(0755, fileperms("$rel/bin/webpulse") & 0777, 'binary mode');
        PkgSupport::assertContains('--version', (string) file_get_contents($p['engine_log']), 'binary self-test');
        foreach (['lib', 'ui', 'bin/webpulse.php', 'Makefile', 'scripts'] as $gone) {
            PkgSupport::assertMissing("$rel/$gone");
        }
        foreach (['cpanel/plugin/install.json', 'whm/webpulse.conf', 'etc/cron.d/webpulse', 'etc/config.json.default', 'installer/lib.sh', 'install.sh', 'uninstall.sh', 'VERSION', 'LICENSE'] as $f) {
            PkgSupport::assertExists("$rel/$f");
        }
        foreach (['hooks/stats_runall_pre.sh', 'hooks/stats_runuser_pre.sh', 'whm/cgi/index.cgi', 'whm/cgi/api.cgi'] as $f) {
            PkgSupport::assertTrue(is_executable("$rel/$f"), "$f executable");
        }

        // config seeded for the redirected layout, then engine setup
        $cfg = json_decode((string) file_get_contents($p['etc'] . '/config.json'), true, 16, JSON_THROW_ON_ERROR);
        PkgSupport::assertSame($p['var'] . '/data', $cfg['paths']['data_dir'], 'data_dir follows WEBPULSE_VAR');
        PkgSupport::assertSame($p['prefix'] . '/etc/userdomains', $cfg['paths']['userdomains_file'], 'system paths follow the prefix');
        PkgSupport::assertSame('{user}', basename($cfg['paths']['domlogs_dirs'][0]), 'placeholders kept');
        PkgSupport::assertSame($p['cron'] . '/webpulse', $cfg['paths']['cron_file'], 'cron_file follows WEBPULSE_CRON_DIR');
        PkgSupport::assertSame($p['prefix'] . '/var/cpanel/hooks/data/hooks.json', $cfg['paths']['hooks_file'], 'every path follows the prefix');
        PkgSupport::assertSame(['/.well-known/', '/cpanel'], $cfg['exclude_paths'], 'URL paths are not prefixed');
        PkgSupport::assertSame(0600, fileperms($p['etc'] . '/config.json') & 0777, 'config mode');
        PkgSupport::assertContains('setup [cfg=' . $p['etc'] . '/config.json]', (string) file_get_contents($p['engine_log']));
        PkgSupport::assertTrue(is_dir($p['var'] . '/data'), 'engine setup created the data dir');

        // cPanel plugin
        $theme = $p['cpanel'] . '/base/frontend/jupiter/webpulse';
        foreach (['index.live.php', 'api.live.php', 'webpulse.svg', 'lib/LiveApiUser.php', 'lib/CgiBridge.php', 'VERSION'] as $f) {
            PkgSupport::assertExists("$theme/$f");
        }
        foreach (['lib/autoload.php', 'lib/WebPulse', 'lib/bootstrap.php', 'ui'] as $f) {
            PkgSupport::assertMissing("$theme/$f"); // no PHP library any more
        }
        PkgSupport::assertSame(0644, fileperms("$theme/index.live.php") & 0777, 'shim mode');
        // WHM plugin
        $cgi = $p['cpanel'] . '/whostmgr/docroot/cgi/webpulse';
        PkgSupport::assertTrue(is_executable("$cgi/index.cgi") && is_executable("$cgi/api.cgi"), 'CGIs executable');
        PkgSupport::assertSame(file_get_contents(PkgSupport::repo() . '/whm/addon_plugins/webpulse.png'),
            file_get_contents($p['cpanel'] . '/whostmgr/docroot/addon_plugins/webpulse.png'), 'WHM icon');
        PkgSupport::assertSame(file_get_contents(PkgSupport::repo() . '/whm/webpulse.conf'), file_get_contents($p['apps'] . '/webpulse.conf'), 'AppConfig');

        // hooks + flag dir
        $hooks = (string) file_get_contents($p['state'] . '/hooks');
        PkgSupport::assertContains("$link/hooks/stats_runall_pre.sh RunAll", $hooks);
        PkgSupport::assertContains("$link/hooks/stats_runuser_pre.sh RunUser", $hooks);
        PkgSupport::assertSame(01777, fileperms($p['var'] . '/run/hook') & 07777, 'hook dir mode');

        // cron
        $cron = (string) file_get_contents($p['cron'] . '/webpulse');
        PkgSupport::assertContains("*/5 * * * * root $link/bin/webpulse collect --quiet", $cron);
        PkgSupport::assertContains('collect --hooked --quiet', $cron);
        PkgSupport::assertNotContains(' root /usr/local/webpulse', $cron, 'paths follow the prefix');
        PkgSupport::assertSame(0644, fileperms($p['cron'] . '/webpulse') & 0777, 'cron mode');

        // cPanel tools were driven exactly as documented
        $log = PkgSupport::stubLog($p);
        PkgSupport::assertTrue(in_array("install_plugin $link/cpanel/plugin --theme jupiter", $log, true), 'install_plugin: ' . implode("|", $log));
        PkgSupport::assertTrue(in_array("register_appconfig $link/whm/webpulse.conf", $log, true), 'register_appconfig');
        PkgSupport::assertTrue(in_array("manage_hooks add script $link/hooks/stats_runall_pre.sh --manual --category Stats --event RunAll --stage pre", $log, true), 'hook RunAll');
        PkgSupport::assertTrue(in_array("manage_hooks add script $link/hooks/stats_runuser_pre.sh --manual --category Stats --event RunUser --stage pre", $log, true), 'hook RunUser');
        PkgSupport::assertTrue(file_exists($p['state'] . '/plugin.jupiter'), 'plugin marker');
    },

    'pkg_installer: --update is idempotent, upgrades via a new release and prunes nothing needed' => static function () use ($count): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y']);
        PkgSupport::assertSame(0, $rc, $out);

        $link = $p['prefix'] . '/usr/local/webpulse';
        $releases = $p['prefix'] . '/usr/local/webpulse-releases';
        $before = PkgSupport::listTree($releases);

        // same version again: nothing new on disk, hooks not registered twice
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--update', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('updating in place', $out);
        PkgSupport::assertContains('already registered', $out);
        PkgSupport::assertSame($before, PkgSupport::listTree($releases), 'release tree unchanged');
        $log = PkgSupport::stubLog($p);
        PkgSupport::assertSame(2, $count($log, 'manage_hooks add'), 'hooks added exactly once each');
        PkgSupport::assertSame(2, $count($log, 'install_plugin'), 'install_plugin re-run');

        // new version: new release dir, symlink swapped, old release kept, config untouched
        file_put_contents($p['etc'] . '/config.json', file_get_contents($p['etc'] . '/config.json') . "\n");
        $cfgBefore = file_get_contents($p['etc'] . '/config.json');
        $src2 = PkgSupport::makeSource($p, '0.2.0', $p['base'] . '/src2');
        [$rc, $out] = PkgSupport::installer($p, $src2, ['--update', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertSame("$releases/0.2.0", readlink($link), 'symlink moved');
        PkgSupport::assertExists("$releases/0.1.0/bin/webpulse");
        PkgSupport::assertSame($cfgBefore, file_get_contents($p['etc'] . '/config.json'), 'config preserved');
        PkgSupport::assertSame('0.2.0', trim((string) file_get_contents($p['cpanel'] . '/base/frontend/jupiter/webpulse/VERSION')), 'plugin files updated');
        PkgSupport::assertSame(2, $count(PkgSupport::stubLog($p), 'manage_hooks add'), 'still no duplicate hooks');
        PkgSupport::assertSame([], glob($releases . '/.stage.*') ?: [], 'no staging leftovers');
    },

    'pkg_installer: same version with changed content gets its own release directory' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        PkgSupport::installer($p, $p['src'], ['-y']);
        file_put_contents($p['src'] . '/README.md', "changed\n");
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--update', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        $target = readlink($p['prefix'] . '/usr/local/webpulse');
        PkgSupport::assertTrue($target !== $p['prefix'] . '/usr/local/webpulse-releases/0.1.0', "new release expected, got $target");
        PkgSupport::assertContains('changed', (string) file_get_contents("$target/README.md"));
    },

    'pkg_installer: --no-cron and --standalone' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--no-cron', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertMissing($p['cron'] . '/webpulse');
        PkgSupport::assertTrue(is_link($p['prefix'] . '/usr/local/webpulse'), 'installed');
        PkgSupport::assertContains('--no-cron', $out);

        $q = PkgSupport::sandbox();
        PkgSupport::rmrf($q['cpanel']); // no cPanel at all
        PkgSupport::makeSource($q);
        [$rc, $out] = PkgSupport::installer($q, $q['src'], ['--standalone', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertTrue(is_link($q['prefix'] . '/usr/local/webpulse'), 'CLI installed');
        PkgSupport::assertMissing($q['cron'] . '/webpulse');
        PkgSupport::assertSame([], PkgSupport::stubLog($q), 'no cPanel tool invoked');
        PkgSupport::assertContains('Standalone mode', $out);
        PkgSupport::assertTrue(is_file($q['etc'] . '/config.json'), 'config created');
    },

    'pkg_installer: --all-themes and --theme install into the extra themes' => static function (): void {
        $p = PkgSupport::sandbox();
        mkdir($p['cpanel'] . '/base/frontend/paper_lantern', 0777, true);
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--all-themes', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        foreach (['jupiter', 'paper_lantern'] as $t) {
            PkgSupport::assertExists($p['cpanel'] . "/base/frontend/$t/webpulse/index.live.php");
            PkgSupport::assertExists($p['state'] . "/plugin.$t");
        }
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--theme', 'nosuchtheme', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains("Theme 'nosuchtheme' not found", $out);
    },

    'pkg_installer: --geoip enables GeoIP through the binary and runs geoip-update' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--geoip', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        $cfg = json_decode((string) file_get_contents($p['etc'] . '/config.json'), true, 16, JSON_THROW_ON_ERROR);
        PkgSupport::assertSame($p['var'] . '/geoip/dbip-country-lite.mmdb', $cfg['geoip']['country_db'], 'country_db follows WEBPULSE_VAR');
        $log = (string) file_get_contents($p['engine_log']);
        PkgSupport::assertContains('config set geoip.enabled true [cfg=' . $p['etc'] . '/config.json]', $log);
        PkgSupport::assertContains('geoip-update', $log);
        PkgSupport::assertTrue(strpos($log, 'config set geoip.enabled true') < strpos($log, 'geoip-update'), 'enabled before the download');
    },

    'pkg_installer: a failing cPanel step prints the command and rolls everything back' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        touch($p['state'] . '/fail.register_appconfig');
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('Step failed: register_appconfig', $out);
        PkgSupport::assertContains('Command: ' . $p['cpanel'] . '/bin/register_appconfig', $out);
        PkgSupport::assertContains('simulated failure', $out);
        PkgSupport::assertContains('Rolling back', $out);

        $log = PkgSupport::stubLog($p);
        PkgSupport::assertTrue(in_array('uninstall_plugin ' . $p['prefix'] . '/usr/local/webpulse/cpanel/plugin --theme jupiter', $log, true), 'plugin unregistered');
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse');
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse-releases/0.1.0');
        PkgSupport::assertMissing($p['cpanel'] . '/base/frontend/jupiter/webpulse');
        PkgSupport::assertMissing($p['cpanel'] . '/whostmgr/docroot/cgi/webpulse');
        PkgSupport::assertMissing($p['cron'] . '/webpulse');
        PkgSupport::assertMissing($p['apps'] . '/webpulse.conf');
        PkgSupport::assertMissing($p['state'] . '/plugin.jupiter');
        PkgSupport::assertMissing($p['etc'] . '/config.json');
    },

    'pkg_installer: a failing hook registration rolls back, including the first hook' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        // make the stub refuse the RunUser hook only
        $stub = $p['cpanel'] . '/bin/manage_hooks';
        file_put_contents($stub, str_replace('add)', "add)\n    case \"\$*\" in *RunUser*) echo 'cannot add RunUser' >&2; exit 1;; esac", (string) file_get_contents($stub)));
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('Step failed: register Stats::RunUser hook', $out);
        PkgSupport::assertSame('', trim((string) file_get_contents($p['state'] . '/hooks')), 'RunAll hook removed again');
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse');
        PkgSupport::assertMissing($p['apps'] . '/webpulse.conf');
    },

    'pkg_installer: a failing update restores the previous release' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y']);
        PkgSupport::assertSame(0, $rc, $out);
        $link = $p['prefix'] . '/usr/local/webpulse';
        $releases = $p['prefix'] . '/usr/local/webpulse-releases';

        $src2 = PkgSupport::makeSource($p, '0.2.0', $p['base'] . '/src2');
        touch($p['state'] . '/fail.register_appconfig');
        [$rc, $out] = PkgSupport::installer($p, $src2, ['--update', '-y']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertSame("$releases/0.1.0", readlink($link), 'previous release active again');
        PkgSupport::assertMissing("$releases/0.2.0");
        PkgSupport::assertSame('0.1.0', trim((string) file_get_contents($p['cpanel'] . '/base/frontend/jupiter/webpulse/VERSION')), 'plugin files restored');
        PkgSupport::assertExists($p['apps'] . '/webpulse.conf'); // earlier registration is untouched
        PkgSupport::assertExists($p['cron'] . '/webpulse');
    },

    'pkg_installer: engine setup failure rolls back before anything is activated' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y'], ['STUB_SETUP_FAIL' => '1']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('setup exploded', $out);
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse');
        PkgSupport::assertSame([], PkgSupport::stubLog($p), 'no cPanel tool invoked');
    },

    'pkg_installer: --uninstall reverses everything and keeps data; --purge removes it' => static function () use ($leftovers): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y']);
        PkgSupport::assertSame(0, $rc, $out);
        file_put_contents($p['var'] . '/webpulse.db', 'precious');
        PkgSupport::assertTrue(count($leftovers($p)) > 10, 'installed files present');
        $link = $p['prefix'] . '/usr/local/webpulse';

        // uninstall from the installed copy, as documented
        [$rc, $out] = PkgSupport::run(['bash', "$link/uninstall.sh", '-y'], [
            'WEBPULSE_PREFIX' => $p['prefix'], 'WEBPULSE_FAKE_CPANEL' => '1', 'WEBPULSE_CPANEL_ROOT' => $p['cpanel'],
            'WEBPULSE_ETC' => $p['etc'], 'WEBPULSE_VAR' => $p['var'], 'WEBPULSE_CRON_DIR' => $p['cron'],
            'WEBPULSE_APPS_DIR' => $p['apps'], 'NO_COLOR' => '1',
        ], '');
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('uninstalled', $out);
        PkgSupport::assertSame([], $leftovers($p), 'nothing of WebPulse is left in prefix/cron/apps/cpanel');

        $log = PkgSupport::stubLog($p);
        PkgSupport::assertTrue(in_array('unregister_appconfig webpulse', $log, true), 'unregister_appconfig');
        PkgSupport::assertTrue(in_array('uninstall_plugin ' . $link . '/cpanel/plugin --theme jupiter', $log, true), 'uninstall_plugin');
        $deletes = array_filter($log, static fn ($l) => str_starts_with($l, 'manage_hooks delete script '));
        PkgSupport::assertSame(2, count($deletes), 'both hooks deleted');
        PkgSupport::assertSame('', trim((string) file_get_contents($p['state'] . '/hooks')), 'hook registry empty');
        PkgSupport::assertExists($p['var'] . '/webpulse.db');
        PkgSupport::assertExists($p['etc'] . '/config.json');

        // a second uninstall is a harmless no-op
        [$rc, $out] = PkgSupport::installer($p, PkgSupport::repo(), ['--uninstall', '-y']);
        PkgSupport::assertSame(0, $rc, $out);

        // reinstall keeps the data, then purge
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertSame('precious', file_get_contents($p['var'] . '/webpulse.db'), 'data survived reinstall');
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--uninstall', '--purge', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertMissing($p['var']);
        PkgSupport::assertMissing($p['etc']);
        PkgSupport::assertSame([], $leftovers($p), 'clean after purge');
        PkgSupport::assertSame([], PkgSupport::listTree($p['prefix']) === [] ? [] : array_filter(PkgSupport::listTree($p['prefix']), static fn ($e) => str_contains($e, 'webpulse')), 'prefix clean');
    },

    'pkg_installer: curl | bash style (script on stdin) downloads, verifies and installs the release tarball' => static function () use ($repo): void {
        $p = PkgSupport::sandbox();
        $src = PkgSupport::makeSource($p, '0.3.0', $p['base'] . '/pristine', 'downloaded');
        [$tarball] = PkgSupport::releaseTarball($src, $p['base'] . '/dl');

        [$rc, $out] = PkgSupport::installerPiped($p, ['--no-cron'], ['WEBPULSE_DOWNLOAD_URL' => 'file://' . $tarball]);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('Downloading file://', $out);
        PkgSupport::assertContains('Checksum OK', $out);
        PkgSupport::assertContains('WebPulse 0.3.0 is installed', $out);
        $rel = $p['prefix'] . '/usr/local/webpulse-releases/0.3.0';
        PkgSupport::assertSame($rel, readlink($p['prefix'] . '/usr/local/webpulse'), 'release');
        PkgSupport::assertContains('downloaded', (string) file_get_contents("$rel/bin/webpulse"), 'the downloaded binary is installed');
        PkgSupport::assertSame([], glob($p['base'] . '/webpulse-install.*') ?: [], 'temp download dir removed');

        // the defaults point at GitHub releases
        $script = (string) file_get_contents("$repo/install.sh");
        PkgSupport::assertContains('releases/download/${tag}/webpulse-${ver}-${WP_TARGET}.tar.gz', $script);
        PkgSupport::assertContains('api.github.com/repos/${WP_REPO}/releases/latest', $script);
        PkgSupport::assertContains('WEBPULSE_REPO:-inalto/webpulse', $script);
        PkgSupport::assertContains('WEBPULSE_REF:-main', $script);

        // a tarball that is not WebPulse is refused before anything is installed
        $bad = PkgSupport::tmpDir('badtar');
        mkdir("$bad/x");
        file_put_contents("$bad/x/README", 'hi');
        PkgSupport::run(['tar', '-czf', "$bad/bad.tar.gz", '-C', $bad, 'x']);
        file_put_contents("$bad/bad.tar.gz.sha256", hash_file('sha256', "$bad/bad.tar.gz") . "  bad.tar.gz\n");
        $q = PkgSupport::sandbox();
        [$rc, $out] = PkgSupport::installerPiped($q, ['-y'], ['WEBPULSE_DOWNLOAD_URL' => "file://$bad/bad.tar.gz"]);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('does not look like WebPulse', $out);
        PkgSupport::assertMissing($q['prefix'] . '/usr/local/webpulse');

        // not a tar.gz at all
        file_put_contents("$bad/junk.tar.gz", 'not an archive');
        file_put_contents("$bad/junk.tar.gz.sha256", hash_file('sha256', "$bad/junk.tar.gz") . "  junk.tar.gz\n");
        [$rc, $out] = PkgSupport::installerPiped($q, ['-y'], ['WEBPULSE_DOWNLOAD_URL' => "file://$bad/junk.tar.gz"]);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('not a valid tar.gz', $out);
    },

    'pkg_installer: the checksum is mandatory and a mismatch installs nothing' => static function (): void {
        $p = PkgSupport::sandbox();
        $src = PkgSupport::makeSource($p, '0.3.0', $p['base'] . '/pristine');
        [$tarball, $sha] = PkgSupport::releaseTarball($src, $p['base'] . '/dl');
        $url = ['WEBPULSE_DOWNLOAD_URL' => 'file://' . $tarball];

        // tampered checksum
        file_put_contents($sha, str_repeat('0', 64) . "  x.tar.gz\n");
        [$rc, $out] = PkgSupport::installerPiped($p, ['-y'], $url);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('Checksum mismatch', $out);
        PkgSupport::assertContains('Nothing was installed', $out);
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse');
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse-releases');

        // garbage checksum file
        file_put_contents($sha, "oops\n");
        [$rc, $out] = PkgSupport::installerPiped($p, ['-y'], $url);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('Invalid checksum file', $out);

        // missing checksum file
        unlink($sha);
        [$rc, $out] = PkgSupport::installerPiped($p, ['-y'], $url);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('Cannot verify the download', $out);
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse');

        // the hash can also be given explicitly
        [$rc, $out] = PkgSupport::installerPiped($p, ['-y', '--no-cron'], $url + ['WEBPULSE_DOWNLOAD_SHA256' => strtoupper(hash_file('sha256', $tarball))]);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('Checksum OK', $out);
    },

    'pkg_installer: a missing release asset without cargo fails with a clear message' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installerPiped($p, ['-y'], ['WEBPULSE_DOWNLOAD_URL' => 'file://' . $p['base'] . '/nope.tar.gz']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('No prebuilt binary', $out);
        PkgSupport::assertContains('no release asset', $out);
        PkgSupport::assertContains('cargo is not installed', $out);
        PkgSupport::assertContains('--build', $out);
        PkgSupport::assertContains('--binary', $out);
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse');
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse-releases');
    },

    'pkg_installer: release resolution (latest tag, --ref, architecture) against GitHub URLs' => static function (): void {
        if (!PkgSupport::have('sha256sum')) {
            PkgSupport::skip('sha256sum not installed');
        }
        $p = PkgSupport::sandbox();
        $src = PkgSupport::makeSource($p, '0.5.0', $p['base'] . '/pristine', 'aarch64-build');
        [$tarball, $sha] = PkgSupport::releaseTarball($src, $p['base'] . '/dl', 'aarch64-unknown-linux-musl');
        $fake = PkgSupport::tmpDir('fakebin');
        $curlLog = $p['base'] . '/curl.log';
        PkgSupport::script("$fake/uname", "#!/bin/sh\ncase \"\$1\" in -m) echo \"\${FAKE_ARCH:-aarch64}\";; -s) echo Linux;; *) exec /usr/bin/env uname \"\$@\";; esac\n");
        $tag = 'v0.5.0';
        $base = 'https://github.com/inalto/webpulse/releases/download';
        PkgSupport::script("$fake/curl", <<<SH
#!/bin/bash
out=''; url=''
while [ \$# -gt 0 ]; do
    case "\$1" in
        -o) out=\$2; shift ;;
        -w|--retry|--connect-timeout) shift ;;
        -*) ;;
        *) url=\$1 ;;
    esac
    shift
done
echo "\$url" >> '$curlLog'
code=404
case "\$url" in
    https://api.github.com/repos/inalto/webpulse/releases/latest)
        printf '{"url":"x","tag_name": "$tag","name":"WebPulse"}' > "\$out"; code=200 ;;
    $base/$tag/webpulse-0.5.0-aarch64-unknown-linux-musl.tar.gz) cp '$tarball' "\$out"; code=200 ;;
    $base/$tag/webpulse-0.5.0-aarch64-unknown-linux-musl.tar.gz.sha256) cp '$sha' "\$out"; code=200 ;;
esac
[ "\$code" = 200 ] || : > "\$out"
printf '%s' "\$code"
[ "\$code" = 200 ]
SH);
        $path = $fake . ':' . (getenv('PATH') ?: '/usr/bin:/bin');

        // no --ref: the latest release, arm64 asset
        [$rc, $out] = PkgSupport::installerPiped($p, ['-y', '--no-cron'], ['PATH' => $path]);
        PkgSupport::assertSame(0, $rc, $out);
        $urls = array_map('rtrim', file($curlLog) ?: []);
        PkgSupport::assertTrue(in_array('https://api.github.com/repos/inalto/webpulse/releases/latest', $urls, true), 'asked the API: ' . implode("\n", $urls));
        PkgSupport::assertTrue(in_array("$base/$tag/webpulse-0.5.0-aarch64-unknown-linux-musl.tar.gz", $urls, true), 'asset URL: ' . implode("\n", $urls));
        PkgSupport::assertTrue(in_array("$base/$tag/webpulse-0.5.0-aarch64-unknown-linux-musl.tar.gz.sha256", $urls, true), 'checksum URL');
        PkgSupport::assertContains('WebPulse 0.5.0 is installed', $out);
        PkgSupport::assertContains('aarch64-build', (string) file_get_contents($p['prefix'] . '/usr/local/webpulse/bin/webpulse'));

        // --ref 0.5.0 and --ref v0.5.0 name the tag directly (no API call)
        foreach (['0.5.0', 'v0.5.0'] as $ref) {
            unlink($curlLog);
            $q = PkgSupport::sandbox();
            [$rc, $out] = PkgSupport::installerPiped($q, ['-y', '--no-cron', '--ref', $ref], ['PATH' => $path]);
            PkgSupport::assertSame(0, $rc, "--ref $ref\n$out");
            PkgSupport::assertNotContains('api.github.com', (string) file_get_contents($curlLog), "--ref $ref skips the API");
        }

        // a tag without assets, no cargo: clear failure naming the URL
        $q = PkgSupport::sandbox();
        [$rc, $out] = PkgSupport::installerPiped($q, ['-y', '--ref', 'v9.9.9'], ['PATH' => $path]);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains("no release asset at $base/v9.9.9/webpulse-9.9.9-aarch64-unknown-linux-musl.tar.gz", $out);

        // a branch has no binary
        [$rc, $out] = PkgSupport::installerPiped($q, ['-y', '--ref', 'feature/x'], ['PATH' => $path]);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains("'feature/x' is a branch", $out);

        // x86_64 picks the other asset name
        unlink($curlLog);
        [$rc] = PkgSupport::installerPiped($q, ['-y', '--ref', 'v0.5.0'], ['PATH' => $path, 'FAKE_ARCH' => 'x86_64']);
        PkgSupport::assertSame(1, $rc, 'no x86_64 asset in the fake release');
        PkgSupport::assertContains('webpulse-0.5.0-x86_64-unknown-linux-musl.tar.gz', (string) file_get_contents($curlLog));

        // unsupported architecture
        [$rc, $out] = PkgSupport::installerPiped($q, ['-y'], ['PATH' => $path, 'FAKE_ARCH' => 'riscv64']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains("Unsupported CPU architecture 'riscv64'", $out);
    },

    'pkg_installer: --binary installs the given binary instead of the one in the release' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p, '0.1.0', null, 'bundled');
        $custom = PkgSupport::script($p['base'] . '/my-webpulse', PkgSupport::stubBinaryBody('custom-build'));
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--binary', $custom, '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('Using the provided binary', $out);
        $installed = (string) file_get_contents($p['prefix'] . '/usr/local/webpulse/bin/webpulse');
        PkgSupport::assertContains('custom-build', $installed);
        PkgSupport::assertNotContains('bundled', $installed);
        PkgSupport::assertSame(0755, fileperms($p['prefix'] . '/usr/local/webpulse/bin/webpulse') & 0777, 'mode');

        // WEBPULSE_BINARY is the same thing; a binary that cannot run is rejected before activation
        $q = PkgSupport::sandbox();
        PkgSupport::makeSource($q);
        $broken = PkgSupport::script($q['base'] . '/broken', "#!/nonexistent/interpreter\n");
        [$rc, $out] = PkgSupport::installer($q, $q['src'], ['-y'], ['WEBPULSE_BINARY' => $broken]);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('does not run on this machine', $out);
        PkgSupport::assertMissing($q['prefix'] . '/usr/local/webpulse');
        PkgSupport::assertMissing($q['prefix'] . '/usr/local/webpulse-releases/0.1.0');
    },

    'pkg_installer: a source checkout needs a built binary, --binary or --build' => static function (): void {
        $p = PkgSupport::sandbox();
        $src = PkgSupport::makeSource($p);
        unlink("$src/bin/webpulse");
        rmdir("$src/bin");
        file_put_contents("$src/Cargo.toml", "[package]\nname = \"webpulse\"\nversion = \"0.1.0\"\n");
        mkdir("$src/src");

        [$rc, $out] = PkgSupport::installer($p, $src, ['-y']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('source checkout without a built binary', $out);
        PkgSupport::assertContains('cargo build --release', $out);
        PkgSupport::assertMissing($p['prefix'] . '/usr/local/webpulse');

        // an existing target/release/webpulse is used
        PkgSupport::script("$src/target/release/webpulse", PkgSupport::stubBinaryBody('from-target'));
        [$rc, $out] = PkgSupport::installer($p, $src, ['-y']);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('Using the existing build', $out);
        PkgSupport::assertContains('from-target', (string) file_get_contents($p['prefix'] . '/usr/local/webpulse/bin/webpulse'));
        PkgSupport::assertSame([], array_filter(PkgSupport::listTree($p['prefix'] . '/usr/local/webpulse-releases/0.1.0'),
            static fn ($e) => str_starts_with($e, 'src') || str_starts_with($e, 'target') || $e === 'Cargo.toml'), 'sources are not installed');

        // --build in a checkout without cargo fails clearly and installs nothing
        $q = PkgSupport::sandbox();
        [$rc, $out] = PkgSupport::installer($q, $src, ['--build', '-y']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('cargo (the Rust toolchain) was not found', $out);
        PkgSupport::assertContains('--binary', $out);
        PkgSupport::assertMissing($q['prefix'] . '/usr/local/webpulse');
    },

    'pkg_installer: --build downloads the source tarball and runs cargo build --release --locked' => static function (): void {
        $p = PkgSupport::sandbox();
        // source tarball = the release layout without bin/, plus Cargo.toml, Cargo.lock and src/
        $srcTree = PkgSupport::makeSource($p, '0.7.0', $p['base'] . '/srctree');
        unlink("$srcTree/bin/webpulse");
        rmdir("$srcTree/bin");
        file_put_contents("$srcTree/Cargo.toml", "[package]\nname = \"webpulse\"\nversion = \"0.7.0\"\n");
        file_put_contents("$srcTree/Cargo.lock", "# lock\n");
        mkdir("$srcTree/src");
        $tarball = $p['base'] . '/source.tar.gz';
        [$rc, $out] = PkgSupport::run(['tar', '-czf', $tarball, '--transform', 's,^\.,webpulse-main,', '-C', $srcTree, '.']);
        PkgSupport::assertSame(0, $rc, $out);

        $cargoLog = $p['base'] . '/cargo.log';
        $cargo = PkgSupport::script($p['base'] . '/fake-cargo', "#!/bin/bash\necho \"\$* (cwd=\$(basename \"\$PWD\"))\" >> '$cargoLog'\n"
            . "mkdir -p target/release\ncat > target/release/webpulse <<'EOS'\n" . PkgSupport::stubBinaryBody('built-from-source') . "EOS\nchmod 755 target/release/webpulse\n");
        [$rc, $out] = PkgSupport::installerPiped($p, ['--build', '-y', '--no-cron'], [
            'WEBPULSE_SOURCE_URL' => "file://$tarball", 'WEBPULSE_CARGO' => $cargo,
        ]);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('Building WebPulse from source', $out);
        PkgSupport::assertContains('WebPulse 0.7.0 is installed', $out);
        PkgSupport::assertContains('built-from-source', (string) file_get_contents($p['prefix'] . '/usr/local/webpulse/bin/webpulse'));
        PkgSupport::assertContains('build --release --locked', (string) file_get_contents($cargoLog));

        // without cargo: clear failure, nothing installed
        $q = PkgSupport::sandbox();
        [$rc, $out] = PkgSupport::installerPiped($q, ['--build', '-y'], ['WEBPULSE_SOURCE_URL' => "file://$tarball"]);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('cargo (the Rust toolchain) was not found', $out);
        PkgSupport::assertMissing($q['prefix'] . '/usr/local/webpulse');

        // a failing build is reported and nothing is installed
        $badCargo = PkgSupport::script($p['base'] . '/bad-cargo', "#!/bin/sh\necho 'error: boom' >&2\nexit 101\n");
        $r = PkgSupport::sandbox();
        [$rc, $out] = PkgSupport::installerPiped($r, ['--build', '-y'], ['WEBPULSE_SOURCE_URL' => "file://$tarball", 'WEBPULSE_CARGO' => $badCargo]);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('cargo build failed', $out);
        PkgSupport::assertMissing($r['prefix'] . '/usr/local/webpulse');

        // no release asset + cargo present: falls back to the source build
        $s = PkgSupport::sandbox();
        [$rc, $out] = PkgSupport::installerPiped($s, ['-y', '--no-cron'], [
            'WEBPULSE_DOWNLOAD_URL' => 'file://' . $s['base'] . '/missing.tar.gz', 'WEBPULSE_SOURCE_URL' => "file://$tarball", 'WEBPULSE_CARGO' => $cargo,
        ]);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('falling back to a build from source', $out);
        PkgSupport::assertContains('built-from-source', (string) file_get_contents($s['prefix'] . '/usr/local/webpulse/bin/webpulse'));
    },

    'pkg_installer: --binary with curl | bash fetches only the support files (source tarball)' => static function (): void {
        $p = PkgSupport::sandbox();
        $srcTree = PkgSupport::makeSource($p, '0.8.0', $p['base'] . '/srctree');
        unlink("$srcTree/bin/webpulse");
        rmdir("$srcTree/bin");
        $tarball = $p['base'] . '/source.tar.gz';
        PkgSupport::run(['tar', '-czf', $tarball, '--transform', 's,^\.,webpulse-main,', '-C', $srcTree, '.']);
        $custom = PkgSupport::script($p['base'] . '/my-webpulse', PkgSupport::stubBinaryBody('offline-binary'));
        [$rc, $out] = PkgSupport::installerPiped($p, ['--binary', $custom, '-y', '--no-cron'], ['WEBPULSE_SOURCE_URL' => "file://$tarball"]);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('WebPulse 0.8.0 is installed', $out);
        PkgSupport::assertContains('offline-binary', (string) file_get_contents($p['prefix'] . '/usr/local/webpulse/bin/webpulse'));
    },

    'pkg_installer: an update run from the installed copy fetches the new release instead of reinstalling itself' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p, '0.1.0', null, 'old');
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['-y']);
        PkgSupport::assertSame(0, $rc, $out);
        $link = $p['prefix'] . '/usr/local/webpulse';

        $new = PkgSupport::makeSource($p, '0.2.0', $p['base'] . '/new', 'new');
        [$tarball] = PkgSupport::releaseTarball($new, $p['base'] . '/dl');
        [$rc, $out] = PkgSupport::run(['bash', "$link/install.sh", '--update', '-y'], PkgSupport::installerEnv($p) + ['WEBPULSE_DOWNLOAD_URL' => 'file://' . $tarball], '');
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('Downloading file://', $out);
        PkgSupport::assertSame($p['prefix'] . '/usr/local/webpulse-releases/0.2.0', readlink($link), 'moved to the new release');
        PkgSupport::assertContains('new', (string) file_get_contents("$link/bin/webpulse"));
    },

    'pkg_installer: real binary (when built) installs into the sandbox' => static function () use ($repo): void {
        $bin = "$repo/target/release/webpulse";
        if (!is_file($bin) || !is_executable($bin)) {
            PkgSupport::skip('target/release/webpulse not built (cargo build --release)');
        }
        [$rc] = PkgSupport::run([$bin, 'setup', '--help']);
        if ($rc !== 0) {
            PkgSupport::skip('the binary has no usable setup command yet');
        }
        $p = PkgSupport::sandbox();
        [$rc, $out] = PkgSupport::installer($p, $repo, ['-y'], ['STUB_ENGINE_LOG' => '/dev/null']);
        PkgSupport::assertSame(0, $rc, $out);
        $link = $p['prefix'] . '/usr/local/webpulse';
        PkgSupport::assertTrue(is_dir($p['var']), 'setup created the data dir');
        PkgSupport::assertTrue(is_file($p['etc'] . '/config.json'), 'config seeded');
        [$rc, $out] = PkgSupport::run(["$link/bin/webpulse", 'status'], ['WEBPULSE_CONFIG' => $p['etc'] . '/config.json']);
        PkgSupport::assertTrue($rc === 0 || $rc === 1, "status ran (rc=$rc)\n$out");
        [$rc, $out] = PkgSupport::installer($p, $repo, ['--uninstall', '--purge', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
    },
];
