<?php

declare(strict_types=1);

/* WP3: scripts/build_dist.sh builds an archive that install.sh can consume. */

require_once __DIR__ . '/pkg_support.php';

return [
    'pkg_dist: build_dist.sh archives HEAD and the installer accepts the tarball' => static function (): void {
        [$rc] = PkgSupport::run(['sh', '-c', 'command -v git']);
        if ($rc !== 0) {
            fwrite(STDERR, "SKIP git not installed\n");
            return;
        }
        $p = PkgSupport::sandbox();
        $repo = PkgSupport::makeSource($p, '0.4.2', $p['base'] . '/gitrepo');
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
        foreach (['webpulse-0.4.2/bin/webpulse', 'webpulse-0.4.2/installer/lib.sh', 'webpulse-0.4.2/install.sh', 'webpulse-0.4.2/VERSION'] as $f) {
            PkgSupport::assertContains($f . "\n", $listing, 'archive content');
        }
        PkgSupport::assertNotContains('/.git/', $listing, 'no .git in the archive');
        [, $version] = PkgSupport::run(['tar', '-xzOf', $tarball, 'webpulse-0.4.2/VERSION']);
        PkgSupport::assertSame("0.4.2\n", $version, 'committed VERSION, not the working copy');

        // the tarball is what codeload would serve: install from it
        $q = PkgSupport::sandbox();
        [$rc, $out] = PkgSupport::run(['bash', '-s', '--', '-y'], [
            'WEBPULSE_PREFIX' => $q['prefix'], 'WEBPULSE_FAKE_CPANEL' => '1', 'WEBPULSE_CPANEL_ROOT' => $q['cpanel'],
            'WEBPULSE_ETC' => $q['etc'], 'WEBPULSE_VAR' => $q['var'], 'WEBPULSE_CRON_DIR' => $q['cron'],
            'WEBPULSE_APPS_DIR' => $q['apps'], 'WEBPULSE_SKIP_FIRST_COLLECT' => '1', 'STUB_ENGINE_LOG' => $q['engine_log'],
            'TMPDIR' => $q['base'], 'NO_COLOR' => '1', 'WEBPULSE_DOWNLOAD_URL' => "file://$tarball",
        ], (string) file_get_contents(PkgSupport::repo() . '/install.sh'));
        PkgSupport::assertSame(0, $rc, $out);
        PkgSupport::assertContains('WebPulse 0.4.2 is installed', $out);
    },
];
