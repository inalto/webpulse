<?php

declare(strict_types=1);

/*
 * WP3: install.sh end to end inside a temp prefix with a fake cPanel root
 * (WEBPULSE_PREFIX / WEBPULSE_FAKE_CPANEL / stub cPanel binaries that log their
 * invocations) and a stub engine. Nothing outside tests/tmp is touched.
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
        foreach (['--geoip', '--ref', '--repo', '--no-cron', '--update', '--uninstall', '--purge', '--standalone', '-y', '--help'] as $flag) {
            PkgSupport::assertContains($flag, $out, 'help text');
        }
    },

    'pkg_installer: bad flags are rejected with exit 2, nothing is touched' => static function () use ($repo): void {
        foreach ([['--bogus'], ['--ref'], ['--repo', 'a/b/c'], ['--repo', 'nonsense'], ['--ref', '../x'], ['--purge']] as $args) {
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

    'pkg_installer: PHP detection rejects old PHP and PHP without pdo_sqlite' => static function () use ($repo): void {
        $dir = PkgSupport::tmpDir('fakephp');
        $mk = static function (string $name, string $version, string $modules) use ($dir): string {
            file_put_contents("$dir/$name", "#!/bin/sh\ncase \"\$1\" in -r) echo $version;; -m) printf '$modules';; esac\n");
            chmod("$dir/$name", 0755);
            return "$dir/$name";
        };
        $old = $mk('php74', '70400', '[PHP Modules]\npdo_sqlite\n');
        $nosql = $mk('php82nosql', '80200', '[PHP Modules]\njson\n');
        $good = $mk('php82', '80200', '[PHP Modules]\njson\nPDO_SQLITE\n');
        $check = static function (string $php) use ($repo): int {
            [$rc] = PkgSupport::run(['bash', '-c', 'source "$1"; wp_php_ok "$2"', 'x', "$repo/installer/lib.sh", $php]);
            return $rc;
        };
        PkgSupport::assertTrue($check($old) !== 0, 'PHP 7.4 rejected');
        PkgSupport::assertTrue($check($nosql) !== 0, 'PHP without pdo_sqlite rejected');
        PkgSupport::assertSame(0, $check($good), 'PHP 8.2 + pdo_sqlite accepted');
        PkgSupport::assertSame(0, $check(PHP_BINARY) | 0, 'the PHP running the tests is acceptable') ;
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

        // shebang points at cPanel's PHP
        $first = strtok((string) file_get_contents("$rel/bin/webpulse"), "\n");
        PkgSupport::assertSame('#!' . $p['cpanel'] . '/3rdparty/bin/php', $first, 'engine shebang');
        PkgSupport::assertTrue(is_executable("$rel/bin/webpulse"), 'engine executable');
        foreach (['hooks/stats_runall_pre.sh', 'hooks/stats_runuser_pre.sh', 'whm/cgi/index.cgi', 'whm/cgi/api.cgi'] as $f) {
            PkgSupport::assertTrue(is_executable("$rel/$f"), "$f executable");
        }

        // config seeded for the redirected layout, then engine setup
        $cfg = json_decode((string) file_get_contents($p['etc'] . '/config.json'), true, 16, JSON_THROW_ON_ERROR);
        PkgSupport::assertSame($p['var'] . '/data', $cfg['paths']['data_dir'], 'data_dir follows WEBPULSE_VAR');
        PkgSupport::assertSame($p['prefix'] . '/etc/userdomains', $cfg['paths']['userdomains_file'], 'system paths follow the prefix');
        PkgSupport::assertSame('{user}', basename($cfg['paths']['domlogs_dirs'][0]), 'placeholders kept');
        PkgSupport::assertContains('setup [cfg=' . $p['etc'] . '/config.json]', (string) file_get_contents($p['engine_log']));
        PkgSupport::assertTrue(is_dir($p['var'] . '/data'), 'engine setup created the data dir');

        // cPanel plugin
        $theme = $p['cpanel'] . '/base/frontend/jupiter/webpulse';
        foreach (['index.live.php', 'api.live.php', 'webpulse.svg', 'lib/LiveApiUser.php', 'lib/bootstrap.php', 'lib/autoload.php', 'lib/WebPulse/App.php', 'VERSION'] as $f) {
            PkgSupport::assertExists("$theme/$f");
        }
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
        file_put_contents($p['src'] . '/ui/app.css', "/* changed */\n");
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--update', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        $target = readlink($p['prefix'] . '/usr/local/webpulse');
        PkgSupport::assertTrue($target !== $p['prefix'] . '/usr/local/webpulse-releases/0.1.0', "new release expected, got $target");
        PkgSupport::assertContains('changed', (string) file_get_contents("$target/ui/app.css"));
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

    'pkg_installer: --geoip enables GeoIP and runs geoip-update' => static function (): void {
        $p = PkgSupport::sandbox();
        PkgSupport::makeSource($p);
        [$rc, $out] = PkgSupport::installer($p, $p['src'], ['--geoip', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
        $cfg = json_decode((string) file_get_contents($p['etc'] . '/config.json'), true, 16, JSON_THROW_ON_ERROR);
        PkgSupport::assertSame(true, $cfg['geoip']['enabled'], 'geoip.enabled');
        PkgSupport::assertSame($p['var'] . '/geoip/dbip-country-lite.mmdb', $cfg['geoip']['country_db'], 'other keys kept');
        PkgSupport::assertContains('geoip-update', (string) file_get_contents($p['engine_log']));
        PkgSupport::assertSame(0600, fileperms($p['etc'] . '/config.json') & 0777, 'config mode');
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

    'pkg_installer: curl | bash style (script on stdin) downloads the tarball and installs' => static function () use ($repo): void {
        $p = PkgSupport::sandbox();
        $src = PkgSupport::makeSource($p, '0.3.0', $p['base'] . '/pristine');
        // codeload layout: one top-level directory
        $tarball = $p['base'] . '/main.tar.gz';
        [$rc, $out] = PkgSupport::run(['tar', '-czf', $tarball, '--transform', 's,^\.,webpulse-main,', '-C', $src, '.']);
        PkgSupport::assertSame(0, $rc, $out);

        $env = [
            'WEBPULSE_PREFIX' => $p['prefix'], 'WEBPULSE_FAKE_CPANEL' => '1', 'WEBPULSE_CPANEL_ROOT' => $p['cpanel'],
            'WEBPULSE_ETC' => $p['etc'], 'WEBPULSE_VAR' => $p['var'], 'WEBPULSE_CRON_DIR' => $p['cron'],
            'WEBPULSE_APPS_DIR' => $p['apps'], 'WEBPULSE_SKIP_FIRST_COLLECT' => '1', 'STUB_ENGINE_LOG' => $p['engine_log'],
            'TMPDIR' => $p['base'], 'NO_COLOR' => '1', 'WEBPULSE_DOWNLOAD_URL' => 'file://' . $tarball,
        ];
        $script = (string) file_get_contents("$repo/install.sh");
        [$rc, $out] = PkgSupport::run(['bash', '-s', '--', '--no-cron'], $env, $script);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('Downloading file://', $out);
        PkgSupport::assertContains('WebPulse 0.3.0 is installed', $out);
        PkgSupport::assertSame($p['prefix'] . '/usr/local/webpulse-releases/0.3.0', readlink($p['prefix'] . '/usr/local/webpulse'), 'release');
        PkgSupport::assertSame([], glob($p['base'] . '/webpulse-install.*') ?: [], 'temp download dir removed');

        // the default URL is the codeload one
        $lib = (string) file_get_contents("$repo/install.sh");
        PkgSupport::assertContains('https://codeload.github.com/${WP_REPO}/tar.gz/${WP_REF}', $lib);
        PkgSupport::assertContains('WEBPULSE_REPO:-inalto/webpulse', $lib);
        PkgSupport::assertContains('WEBPULSE_REF:-main', $lib);

        // a tarball that is not WebPulse is refused before anything is installed
        $bad = PkgSupport::tmpDir('badtar');
        mkdir("$bad/x");
        file_put_contents("$bad/x/README", 'hi');
        PkgSupport::run(['tar', '-czf', "$bad/bad.tar.gz", '-C', $bad, 'x']);
        $q = PkgSupport::sandbox();
        $env['WEBPULSE_DOWNLOAD_URL'] = "file://$bad/bad.tar.gz";
        $env['WEBPULSE_PREFIX'] = $q['prefix'];
        [$rc, $out] = PkgSupport::run(['bash', '-s', '--', '-y'], $env, $script);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('does not look like WebPulse', $out);
        PkgSupport::assertMissing($q['prefix'] . '/usr/local/webpulse');

        // download failure
        $env['WEBPULSE_DOWNLOAD_URL'] = "file://$bad/missing.tar.gz";
        [$rc, $out] = PkgSupport::run(['bash', '-s', '--', '-y'], $env, $script);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('Download failed', $out);
    },

    'pkg_installer: real engine (when present) installs into the sandbox' => static function () use ($repo): void {
        if (!is_file("$repo/bin/webpulse") || !is_file("$repo/lib/WebPulse/App.php") || !is_file("$repo/etc/config.json.default")) {
            fwrite(STDERR, "SKIP engine package not present\n");
            return;
        }
        $p = PkgSupport::sandbox();
        [$rc, $out] = PkgSupport::installer($p, $repo, ['-y']);
        PkgSupport::assertSame(0, $rc, $out);
        $link = $p['prefix'] . '/usr/local/webpulse';
        PkgSupport::assertTrue(is_file($p['var'] . '/webpulse.db') || is_dir($p['var']), 'engine setup created the data dir');
        [$rc, $out] = PkgSupport::run(["$link/bin/webpulse", 'status'], ['WEBPULSE_CONFIG' => $p['etc'] . '/config.json']);
        PkgSupport::assertTrue($rc === 0 || $rc === 1, "bin/webpulse status ran (rc=$rc)\n$out");
        [$rc, $out] = PkgSupport::installer($p, $repo, ['--uninstall', '--purge', '-y']);
        PkgSupport::assertSame(0, $rc, $out);
    },
];
