<?php

declare(strict_types=1);

/*
 * WP3 static checks: syntax of every shell/PHP/Python packaging file, plugin
 * descriptors against docs/CPANEL-INTEGRATION.md, icons, cron file.
 */

require_once __DIR__ . '/pkg_support.php';

$repo = PkgSupport::repo();

$shellScripts = ['install.sh', 'uninstall.sh', 'installer/lib.sh', 'hooks/stats_runall_pre.sh',
    'hooks/stats_runuser_pre.sh', 'scripts/build_dist.sh'];
$phpFiles = ['cpanel/webpulse/index.live.php', 'cpanel/webpulse/api.live.php', 'cpanel/webpulse/lib/LiveApiUser.php',
    'cpanel/webpulse/lib/bootstrap.php', 'whm/cgi/index.cgi', 'whm/cgi/api.cgi', 'whm/lib/Edge.php'];

return [
    'pkg_static: bash -n on every shell script' => static function () use ($repo, $shellScripts): void {
        foreach ($shellScripts as $f) {
            [$rc, $out] = PkgSupport::run(['bash', '-n', "$repo/$f"]);
            PkgSupport::assertSame(0, $rc, "bash -n $f\n$out");
        }
    },

    'pkg_static: shellcheck (warning level) when available' => static function () use ($repo, $shellScripts): void {
        [$rc] = PkgSupport::run(['sh', '-c', 'command -v shellcheck']);
        if ($rc !== 0) {
            fwrite(STDERR, "SKIP shellcheck not installed\n");
            return;
        }
        foreach ($shellScripts as $f) {
            [$rc, $out] = PkgSupport::run(['shellcheck', '-S', 'warning', "$repo/$f"]);
            PkgSupport::assertSame(0, $rc, "shellcheck $f\n$out");
        }
    },

    'pkg_static: scripts are executable and have a shebang' => static function () use ($repo, $shellScripts): void {
        foreach (array_merge(array_diff($shellScripts, ['installer/lib.sh']), ['scripts/make_icons.py', 'whm/cgi/index.cgi', 'whm/cgi/api.cgi']) as $f) {
            PkgSupport::assertTrue(is_executable("$repo/$f"), "$f must be executable");
            PkgSupport::assertSame('#!', substr((string) file_get_contents("$repo/$f"), 0, 2), "$f shebang");
        }
    },

    'pkg_static: php -l on live.php, lib and CGI front-ends' => static function () use ($repo, $phpFiles): void {
        foreach ($phpFiles as $f) {
            [$rc, $out] = PkgSupport::run([PHP_BINARY, '-l', "$repo/$f"]);
            PkgSupport::assertSame(0, $rc, "php -l $f\n$out");
        }
    },

    'pkg_static: CGI files start with the cPanel shebang and WHM markers' => static function () use ($repo): void {
        foreach (['index.cgi', 'api.cgi'] as $f) {
            $lines = explode("\n", (string) file_get_contents("$repo/whm/cgi/$f"));
            PkgSupport::assertSame('#!/usr/local/cpanel/3rdparty/bin/php', $lines[0], "$f shebang");
            PkgSupport::assertSame('<?php', $lines[1], "$f opening tag");
            PkgSupport::assertSame('#WHMADDON:webpulse:WebPulse Analytics:webpulse.png', $lines[2], "$f WHMADDON");
            PkgSupport::assertSame('#ACLS:all', $lines[3], "$f ACLS");
        }
    },

    'pkg_static: python scripts parse' => static function () use ($repo): void {
        [$rc] = PkgSupport::run(['sh', '-c', 'command -v python3']);
        if ($rc !== 0) {
            fwrite(STDERR, "SKIP python3 not installed\n");
            return;
        }
        [$rc, $out] = PkgSupport::run(['python3', '-I', '-c', 'import ast,sys; ast.parse(open(sys.argv[1]).read())', "$repo/scripts/make_icons.py"]);
        PkgSupport::assertSame(0, $rc, $out);
    },

    'pkg_static: install.json matches CPANEL-INTEGRATION.md' => static function () use ($repo): void {
        $json = json_decode((string) file_get_contents("$repo/cpanel/plugin/install.json"), true, 16, JSON_THROW_ON_ERROR);
        PkgSupport::assertTrue(is_array($json) && count($json) === 1, 'install.json is a one-element list');
        $e = $json[0];
        $expected = ['id' => 'webpulse', 'name' => 'WebPulse Analytics',
            'description' => 'Visitor statistics generated from your access logs',
            'uri' => 'webpulse/index.live.php', 'icon' => 'webpulse.svg', 'group_id' => 'metrics',
            'order' => 1, 'type' => 'link', 'featuremanager' => 1, 'feature' => 'webpulse'];
        PkgSupport::assertSame($expected, $e, 'install.json entry');
        PkgSupport::assertExists("$repo/cpanel/plugin/" . $e['icon']);
        PkgSupport::assertExists("$repo/cpanel/{$e['uri']}");
    },

    'pkg_static: webpulse.conf (AppConfig) matches CPANEL-INTEGRATION.md' => static function () use ($repo): void {
        $conf = [];
        foreach (explode("\n", (string) file_get_contents("$repo/whm/webpulse.conf")) as $line) {
            if (trim($line) === '' || $line[0] === '#') {
                continue;
            }
            PkgSupport::assertTrue(str_contains($line, '='), "bad AppConfig line: $line");
            [$k, $v] = explode('=', $line, 2);
            $conf[$k] = $v;
        }
        PkgSupport::assertSame([
            'name' => 'webpulse', 'service' => 'whostmgr', 'url' => '/cgi/webpulse/index.cgi',
            'url2' => '/cgi/webpulse/api.cgi', 'user' => 'root', 'acls' => 'all,list-accts',
            'displayname' => 'WebPulse Analytics', 'entryurl' => 'webpulse/index.cgi', 'icon' => 'webpulse.png',
            'target' => '_self', 'searchtext' => 'analytics statistics visitors logs awstats webalizer',
        ], $conf, 'AppConfig keys');
        PkgSupport::assertExists("$repo/whm/addon_plugins/" . $conf['icon']);
        PkgSupport::assertExists("$repo/whm/cgi/" . basename($conf['entryurl']));
    },

    'pkg_static: PNG icon is a 48x48 RGBA image with transparency' => static function () use ($repo): void {
        $d = (string) file_get_contents("$repo/whm/addon_plugins/webpulse.png");
        PkgSupport::assertSame("\x89PNG\r\n\x1a\n", substr($d, 0, 8), 'PNG magic');
        PkgSupport::assertSame('IHDR', substr($d, 12, 4), 'first chunk');
        $h = unpack('Nw/Nh/Cdepth/Ccolor', substr($d, 16, 10));
        PkgSupport::assertSame(48, $h['w'], 'width');
        PkgSupport::assertSame(48, $h['h'], 'height');
        PkgSupport::assertSame(8, $h['depth'], 'bit depth');
        PkgSupport::assertSame(6, $h['color'], 'colour type RGBA');
        PkgSupport::assertSame('IEND', substr($d, -8, 4), 'last chunk');
        // decode: corner pixel fully transparent, centre opaque
        $pos = 8;
        $idat = '';
        while ($pos < strlen($d)) {
            $len = unpack('N', substr($d, $pos, 4))[1];
            if (substr($d, $pos + 4, 4) === 'IDAT') {
                $idat .= substr($d, $pos + 8, $len);
            }
            $pos += 12 + $len;
        }
        $raw = zlib_decode($idat);
        PkgSupport::assertSame(48 * (1 + 48 * 4), strlen((string) $raw), 'decoded size');
        PkgSupport::assertSame(0, ord($raw[1 + 3]), 'top-left alpha');
        $row = 24 * (1 + 48 * 4) + 1 + 4 * 6;
        PkgSupport::assertSame(255, ord($raw[$row + 3]), 'inside alpha');
    },

    'pkg_static: SVG icons are well-formed 48x48 and identical' => static function () use ($repo): void {
        $paths = ["$repo/cpanel/webpulse/webpulse.svg", "$repo/cpanel/plugin/webpulse.svg"];
        foreach ($paths as $p) {
            $xml = simplexml_load_string((string) file_get_contents($p));
            PkgSupport::assertTrue($xml !== false, "$p parses");
            PkgSupport::assertSame('0 0 48 48', (string) $xml['viewBox'], 'viewBox');
            PkgSupport::assertSame('48', (string) $xml['width'], 'width');
        }
        PkgSupport::assertSame(file_get_contents($paths[0]), file_get_contents($paths[1]), 'plugin icon copy');
    },

    'pkg_static: cron file is valid cron.d syntax' => static function () use ($repo): void {
        $lines = array_map('rtrim', file("$repo/etc/cron.d/webpulse") ?: []);
        $jobs = [];
        $vars = [];
        foreach ($lines as $l) {
            if ($l === '' || $l[0] === '#') {
                continue;
            }
            if (preg_match('/^([A-Z]+)=(.*)$/', $l, $m)) {
                $vars[$m[1]] = $m[2];
                continue;
            }
            PkgSupport::assertTrue(
                preg_match('#^(\S+)\s+(\S+)\s+(\S+)\s+(\S+)\s+(\S+)\s+root\s+/usr/local/webpulse/bin/webpulse\s+(.+)$#', $l, $m) === 1,
                "bad cron line: $l"
            );
            $jobs[] = [implode(' ', array_slice($m, 1, 5)), $m[6]];
        }
        PkgSupport::assertSame('""', $vars['MAILTO'] ?? null, 'MAILTO');
        PkgSupport::assertTrue(isset($vars['PATH']) && str_contains($vars['PATH'], '/usr/bin'), 'PATH');
        PkgSupport::assertSame([
            ['*/5 * * * *', 'collect --quiet'],
            ['* * * * *', 'collect --hooked --quiet'],
            ['10 2 * * *', 'rollup --quiet'],
            ['30 4 1 * *', 'geoip-update --quiet'],
        ], $jobs, 'cron jobs');
    },

    'pkg_static: CHANGELOG, INSTALL doc and CI workflow exist' => static function () use ($repo): void {
        PkgSupport::assertContains('## [0.1.0]', (string) file_get_contents("$repo/CHANGELOG.md"));
        PkgSupport::assertContains('Troubleshooting', (string) file_get_contents("$repo/docs/INSTALL.md"));
        $ci = (string) file_get_contents("$repo/.github/workflows/ci.yml");
        PkgSupport::assertContains('shivammathur/setup-php', $ci);
        PkgSupport::assertContains('make test', $ci);
    },
];
