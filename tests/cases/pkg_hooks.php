<?php

declare(strict_types=1);

/* WP3: Standardized Hook scripts (--describe, flag file for Stats::RunUser). */

require_once __DIR__ . '/pkg_support.php';

$repo = PkgSupport::repo();

return [
    'pkg_hooks: --describe prints the registration JSON' => static function () use ($repo): void {
        foreach ([['stats_runall_pre.sh', 'RunAll'], ['stats_runuser_pre.sh', 'RunUser']] as [$file, $event]) {
            $path = "$repo/hooks/$file";
            [$rc, $out] = PkgSupport::run([$path, '--describe']);
            PkgSupport::assertSame(0, $rc, "$file --describe exit");
            $json = json_decode($out, true, 8, JSON_THROW_ON_ERROR);
            PkgSupport::assertTrue(is_array($json) && count($json) === 1, 'one hook described');
            PkgSupport::assertSame([
                'category' => 'Stats', 'event' => $event, 'stage' => 'pre',
                'hook' => $path, 'exectype' => 'script',
            ], $json[0], "$file describe");
        }
    },

    'pkg_hooks: --describe keeps the logical path of a symlinked install dir' => static function () use ($repo): void {
        $tmp = PkgSupport::tmpDir('hooklink');
        symlink($repo, "$tmp/webpulse");
        [, $out] = PkgSupport::run(["$tmp/webpulse/hooks/stats_runall_pre.sh", '--describe']);
        $json = json_decode($out, true, 8, JSON_THROW_ON_ERROR);
        PkgSupport::assertSame("$tmp/webpulse/hooks/stats_runall_pre.sh", $json[0]['hook'], 'hook path through the symlink');
    },

    'pkg_hooks: RunUser touches the flag for a valid user' => static function () use ($repo): void {
        $dir = PkgSupport::tmpDir('hookflag');
        [$rc, $out] = PkgSupport::run(["$repo/hooks/stats_runuser_pre.sh"], ['WEBPULSE_HOOK_DIR' => $dir],
            '{"context":{"event":"RunUser"},"data":{"user":"alice","domain":"example.com"}}');
        PkgSupport::assertSame(0, $rc, 'exit status');
        PkgSupport::assertSame('', $out, 'silent');
        PkgSupport::assertExists("$dir/alice");
        PkgSupport::assertSame(['alice'], PkgSupport::listTree($dir), 'only the flag was created');
    },

    'pkg_hooks: RunUser ignores invalid users and bad payloads, exits 0' => static function () use ($repo): void {
        $dir = PkgSupport::tmpDir('hookbad');
        $payloads = [
            '{"data":{"user":"../x"}}', '{"data":{"user":"a/b"}}', '{"data":{"user":""}}', '{"data":{"user":"Root"}}',
            '{"data":{"user":"-x"}}', '{"data":{"user":' . json_encode(str_repeat('a', 33)) . '}}',
            '{"data":{"user":["alice"]}}', '{"data":{"user":"alice\\n"}}', '{"data":{}}', '{"data":"alice"}', 'not json', '', '[]',
        ];
        foreach ($payloads as $payload) {
            [$rc] = PkgSupport::run(["$repo/hooks/stats_runuser_pre.sh"], ['WEBPULSE_HOOK_DIR' => $dir], $payload);
            PkgSupport::assertSame(0, $rc, "exit status for $payload");
        }
        PkgSupport::assertSame([], PkgSupport::listTree($dir), 'no flag created');
        PkgSupport::assertMissing("$dir/../x");
        // a missing flag directory is not an error either
        [$rc] = PkgSupport::run(["$repo/hooks/stats_runuser_pre.sh"], ['WEBPULSE_HOOK_DIR' => "$dir/nope"], '{"data":{"user":"alice"}}');
        PkgSupport::assertSame(0, $rc, 'missing dir');
    },

    'pkg_hooks: RunUser does not follow a planted symlink' => static function () use ($repo): void {
        $dir = PkgSupport::tmpDir('hooklnk');
        $target = "$dir/victim";
        file_put_contents($target, 'x');
        touch($target, 1000000000);
        mkdir("$dir/flags");
        symlink($target, "$dir/flags/alice");
        PkgSupport::run(["$repo/hooks/stats_runuser_pre.sh"], ['WEBPULSE_HOOK_DIR' => "$dir/flags"], '{"data":{"user":"alice"}}');
        clearstatcache();
        PkgSupport::assertSame(1000000000, filemtime($target), 'symlink target untouched');
    },

    'pkg_hooks: RunAll runs "collect --quiet" and always exits 0' => static function () use ($repo): void {
        $home = PkgSupport::tmpDir('hookall');
        mkdir("$home/bin");
        $log = "$home/calls.log";
        file_put_contents("$home/bin/webpulse", "#!/bin/sh\necho \"\$*\" >> '$log'\nexit 3\n");
        chmod("$home/bin/webpulse", 0755);
        [$rc, $out] = PkgSupport::run(["$repo/hooks/stats_runall_pre.sh"], ['WEBPULSE_HOME' => $home], '{"context":{},"data":{}}');
        PkgSupport::assertSame(0, $rc, 'exit status even when collect fails');
        PkgSupport::assertSame('', $out, 'silent');
        PkgSupport::assertSame("collect --quiet\n", (string) file_get_contents($log), 'collect invocation');
        // missing engine is not an error
        [$rc] = PkgSupport::run(["$repo/hooks/stats_runall_pre.sh"], ['WEBPULSE_HOME' => "$home/none"], '');
        PkgSupport::assertSame(0, $rc, 'no engine');
    },

    'pkg_hooks: RunAll uses a 20 minute timeout' => static function () use ($repo): void {
        $src = (string) file_get_contents("$repo/hooks/stats_runall_pre.sh");
        PkgSupport::assertContains('timeout 1200', $src);
    },
];
