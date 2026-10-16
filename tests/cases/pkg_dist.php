<?php

declare(strict_types=1);

/*
 * scripts/build_dist.sh (source tarball, git archive) and scripts/build_release.sh
 * (binary tarball): the archives have the layout install.sh consumes.
 */

require_once __DIR__ . '/pkg_support.php';

return [
    'pkg_dist: build_dist.sh archives HEAD and the installer accepts the source tarball' => static function (): void {
        if (!PkgSupport::have('git')) {
            PkgSupport::skip('git not installed');
        }
        $p = PkgSupport::sandbox();
        $repo = PkgSupport::makeSource($p, '0.4.2', $p['base'] . '/gitrepo');
        PkgSupport::run(['cp', '-a', PkgSupport::repo() . '/scripts', "$repo/scripts"]);
        $git = static fn (array $a): array => PkgSupport::run(array_merge(
            ['git', '-c', 'user.name=t', '-c', 'user.email=t@example.com', '-c', 'init.defaultBranch=main', '-c', 'commit.gpgsign=false'],
            $a
        ), [], null, $repo);
        foreach ([['init', '-q'], ['add', '-A'], ['commit', '-q', '-m', 'test']] as $cmd) {
            [$rc, $out] = $git($cmd);
            PkgSupport::assertSame(0, $rc, implode(' ', $cmd) . ": $out");
        }
        // an uncommitted change must not end up in the archive
        file_put_contents("$repo/VERSION", "9.9.9\n");

        [$rc, $out] = PkgSupport::run(['bash', "$repo/scripts/build_dist.sh", "$repo/dist"], [], null, $repo);
        PkgSupport::assertSame(0, $rc, $out);
        $tarball = "$repo/dist/webpulse-0.4.2.tar.gz";
        PkgSupport::assertExists($tarball);
        PkgSupport::assertContains('uncommitted changes', $out, 'warning about the dirty tree');
        [, $listing] = PkgSupport::run(['tar', '-tzf', $tarball]);
        foreach (['webpulse-0.4.2/installer/lib.sh', 'webpulse-0.4.2/install.sh', 'webpulse-0.4.2/VERSION'] as $f) {
            PkgSupport::assertContains($f . "\n", $listing, 'archive content');
        }
        PkgSupport::assertNotContains('/.git/', $listing, 'no .git in the archive');
        [, $version] = PkgSupport::run(['tar', '-xzOf', $tarball, 'webpulse-0.4.2/VERSION']);
        PkgSupport::assertSame("0.4.2\n", $version, 'committed VERSION, not the working copy');

        // the tarball is what codeload serves; with --binary only the support files are taken from it
        $q = PkgSupport::sandbox();
        $custom = PkgSupport::script($q['base'] . '/my-webpulse', PkgSupport::stubBinaryBody('dist-test'));
        [$rc, $out] = PkgSupport::installerPiped($q, ['-y', '--binary', $custom], ['WEBPULSE_SOURCE_URL' => "file://$tarball"]);
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('WebPulse 0.4.2 is installed', $out);
    },

    'pkg_dist: build_release.sh packs the binary tarball with the layout install.sh needs, and the installer installs it' => static function (): void {
        $repo = PkgSupport::repo();
        $p = PkgSupport::sandbox();
        $stub = PkgSupport::script($p['base'] . '/built-webpulse', PkgSupport::stubBinaryBody('release-script'));
        $out = $p['base'] . '/dist';
        [$rc, $log] = PkgSupport::run(['bash', "$repo/scripts/build_release.sh", '--binary', $stub, '--target', 'x86_64-unknown-linux-musl', '--out', $out], ['SOURCE_DATE_EPOCH' => '1700000000']);
        PkgSupport::assertSame(0, $rc, $log);
        $version = trim((string) file_get_contents("$repo/VERSION"));
        $name = "webpulse-$version-x86_64-unknown-linux-musl";
        $tarball = "$out/$name.tar.gz";
        PkgSupport::assertExists($tarball);
        PkgSupport::assertExists("$tarball.sha256");

        // checksum file: "<hash>  <file name>", matches the tarball
        $sha = trim((string) file_get_contents("$tarball.sha256"));
        PkgSupport::assertSame(hash_file('sha256', $tarball) . "  $name.tar.gz", $sha, 'sha256 file');

        [, $listing] = PkgSupport::run(['tar', '-tzf', $tarball]);
        foreach (['bin/webpulse', 'installer/lib.sh', 'install.sh', 'uninstall.sh', 'VERSION', 'LICENSE', 'README.md', 'etc/config.json.default',
            'etc/cron.d/webpulse', 'cpanel/plugin/install.json', 'cpanel/webpulse/index.live.php', 'cpanel/webpulse/api.live.php',
            'cpanel/webpulse/lib/LiveApiUser.php', 'cpanel/webpulse/lib/CgiBridge.php', 'whm/cgi/index.cgi', 'whm/cgi/api.cgi',
            'whm/webpulse.conf', 'whm/addon_plugins/webpulse.png', 'hooks/stats_runall_pre.sh', 'hooks/stats_runuser_pre.sh'] as $f) {
            PkgSupport::assertContains("$name/$f\n", $listing, 'archive content');
        }
        foreach (['/src/', '/target/', '/tests/', '/lib/WebPulse', '/ui/', '/.git/'] as $absent) {
            PkgSupport::assertNotContains("$name$absent", $listing, 'not part of the binary tarball');
        }
        [, $mode] = PkgSupport::run(['tar', '-tvzf', $tarball, "$name/bin/webpulse"]);
        PkgSupport::assertTrue(str_starts_with(trim($mode), '-rwxr-xr-x'), "binary mode in the archive: $mode");
        PkgSupport::assertSame("$name/", strtok($listing, "\n"), 'single top-level directory');
        PkgSupport::assertSame(1, count(array_unique(array_map(static fn ($l) => explode('/', $l)[0], array_filter(explode("\n", $listing))))), 'one top-level entry only');

        // reproducible with the same SOURCE_DATE_EPOCH
        $out2 = $p['base'] . '/dist2';
        PkgSupport::run(['bash', "$repo/scripts/build_release.sh", '--binary', $stub, '--target', 'x86_64-unknown-linux-musl', '--out', $out2], ['SOURCE_DATE_EPOCH' => '1700000000']);
        PkgSupport::assertSame(hash_file('sha256', $tarball), hash_file('sha256', "$out2/$name.tar.gz"), 'reproducible tarball');

        // the installer consumes exactly this tarball (the stub config default is not needed: the real one is shipped)
        [$rc, $res] = PkgSupport::installerPiped($p, ['-y'], ['WEBPULSE_DOWNLOAD_URL' => "file://$tarball", 'STUB_ENGINE_LOG' => $p['engine_log']]);
        PkgSupport::assertSame(0, $rc, $res);
        PkgSupport::assertContains("WebPulse $version is installed", $res);
        $link = $p['prefix'] . '/usr/local/webpulse';
        PkgSupport::assertContains('release-script', (string) file_get_contents("$link/bin/webpulse"));
        PkgSupport::assertExists($p['cpanel'] . '/base/frontend/jupiter/webpulse/lib/CgiBridge.php');
        PkgSupport::assertExists($p['cpanel'] . '/whostmgr/docroot/cgi/webpulse/index.cgi');
        $cfg = json_decode((string) file_get_contents($p['etc'] . '/config.json'), true);
        PkgSupport::assertTrue(is_array($cfg) && ($cfg['paths']['data_dir'] ?? '') === $p['var'] . '/data', 'the shipped default config is seeded for the redirected layout');
    },

    'pkg_dist: build_release.sh refuses a missing binary, a bad target and a tag that is not VERSION' => static function (): void {
        $repo = PkgSupport::repo();
        $p = PkgSupport::sandbox();
        $stub = PkgSupport::script($p['base'] . '/b', "#!/bin/sh\n");
        [$rc] = PkgSupport::run(['bash', "$repo/scripts/build_release.sh", '--binary', $p['base'] . '/missing', '--out', $p['base'] . '/o']);
        PkgSupport::assertSame(1, $rc, 'missing binary');
        [$rc] = PkgSupport::run(['bash', "$repo/scripts/build_release.sh", '--binary', $stub, '--target', '../x', '--out', $p['base'] . '/o']);
        PkgSupport::assertSame(2, $rc, 'bad target');
        [$rc, $out] = PkgSupport::run(['bash', "$repo/scripts/build_release.sh", '--binary', $stub, '--out', $p['base'] . '/o'], ['WEBPULSE_RELEASE_TAG' => 'v99.0.0']);
        PkgSupport::assertSame(1, $rc, $out);
        PkgSupport::assertContains('does not match VERSION', $out);
    },
];
