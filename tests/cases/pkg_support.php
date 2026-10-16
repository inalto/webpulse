<?php

declare(strict_types=1);

/*
 * Shared helpers of the pkg_* test cases (WP3: packaging, installer, hooks).
 * Returns no tests itself; the other pkg_*.php files require_once it.
 */

if (!class_exists('PkgSkip', false)) {
    /** Thrown by a test that cannot run on this machine (reported as SKIP by tests/run.php). */
    final class PkgSkip extends \RuntimeException
    {
    }
}

if (!class_exists('PkgSupport', false)) {
    final class PkgSupport
    {
        /** @var list<string> */
        private static array $cleanup = [];

        public static function repo(): string
        {
            return dirname(__DIR__, 2);
        }

        public static function fail(string $message): never
        {
            throw new \RuntimeException($message);
        }

        public static function skip(string $reason): never
        {
            throw new PkgSkip($reason);
        }

        /** Is an external program available? */
        public static function have(string $program): bool
        {
            [$rc] = self::run(['sh', '-c', 'command -v "$1" >/dev/null 2>&1', 'x', $program]);
            return $rc === 0;
        }

        /** Write an executable script. */
        public static function script(string $path, string $body): string
        {
            if (!is_dir(dirname($path))) {
                mkdir(dirname($path), 0777, true);
            }
            file_put_contents($path, $body);
            chmod($path, 0755);
            return $path;
        }

        public static function assertTrue(mixed $cond, string $message): void
        {
            if ($cond !== true) {
                self::fail($message);
            }
        }

        public static function assertSame(mixed $expected, mixed $actual, string $message = ''): void
        {
            if ($expected !== $actual) {
                self::fail(($message !== '' ? $message . ': ' : '')
                    . 'expected ' . var_export($expected, true) . ', got ' . var_export($actual, true));
            }
        }

        public static function assertContains(string $needle, string $haystack, string $message = ''): void
        {
            if (!str_contains($haystack, $needle)) {
                self::fail(($message !== '' ? $message . ': ' : '') . "'$needle' not found in:\n" . self::clip($haystack));
            }
        }

        public static function assertNotContains(string $needle, string $haystack, string $message = ''): void
        {
            if (str_contains($haystack, $needle)) {
                self::fail(($message !== '' ? $message . ': ' : '') . "'$needle' unexpectedly found in:\n" . self::clip($haystack));
            }
        }

        public static function assertExists(string $path): void
        {
            if (!file_exists($path) && !is_link($path)) {
                self::fail("missing: $path");
            }
        }

        public static function assertMissing(string $path): void
        {
            if (file_exists($path) || is_link($path)) {
                self::fail("should not exist: $path");
            }
        }

        public static function clip(string $s, int $max = 4000): string
        {
            return strlen($s) > $max ? substr($s, 0, $max) . "\n...[clipped]" : $s;
        }

        /** @return array{0:int,1:string} exit code, stdout+stderr */
        public static function run(array $cmd, array $env = [], ?string $stdin = null, ?string $cwd = null): array
        {
            $envFull = array_merge(self::baseEnv(), $env);
            $proc = proc_open($cmd, [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['redirect', 1]], $pipes, $cwd, $envFull);
            if (!is_resource($proc)) {
                self::fail('cannot start ' . implode(' ', $cmd));
            }
            if ($stdin !== null) {
                fwrite($pipes[0], $stdin);
            }
            fclose($pipes[0]);
            $out = (string) stream_get_contents($pipes[1]);
            fclose($pipes[1]);
            return [proc_close($proc), $out];
        }

        /** @return array<string,string> */
        private static function baseEnv(): array
        {
            $env = [];
            foreach (['PATH', 'HOME', 'LANG', 'TMPDIR'] as $k) {
                $v = getenv($k);
                if (is_string($v)) {
                    $env[$k] = $v;
                }
            }
            $env['PATH'] ??= '/usr/local/bin:/usr/bin:/bin';
            return $env;
        }

        /** Make a throw-away directory below tests/tmp and remove it at shutdown. */
        public static function tmpDir(string $label = 'pkg'): string
        {
            $base = self::repo() . '/tests/tmp';
            if (!is_dir($base)) {
                @mkdir($base, 0777, true);
            }
            if (!is_dir($base) || !is_writable($base)) {
                $base = sys_get_temp_dir();
            }
            $dir = $base . '/' . $label . '-' . getmypid() . '-' . bin2hex(random_bytes(3));
            mkdir($dir, 0777, true);
            self::$cleanup[] = $dir;
            if (count(self::$cleanup) === 1) {
                register_shutdown_function([self::class, 'cleanupAll']);
            }
            return $dir;
        }

        public static function cleanupAll(): void
        {
            foreach (self::$cleanup as $dir) {
                self::rmrf($dir);
            }
            self::$cleanup = [];
        }

        public static function rmrf(string $path): void
        {
            if (is_link($path) || is_file($path)) {
                @unlink($path);
                return;
            }
            if (!is_dir($path)) {
                return;
            }
            @chmod($path, 0755);
            foreach (scandir($path) ?: [] as $e) {
                if ($e !== '.' && $e !== '..') {
                    self::rmrf($path . '/' . $e);
                }
            }
            @rmdir($path);
        }

        /** @return list<string> relative paths of every file/dir/symlink below $dir */
        public static function listTree(string $dir): array
        {
            $out = [];
            if (!is_dir($dir)) {
                return $out;
            }
            $it = new \RecursiveIteratorIterator(
                new \RecursiveDirectoryIterator($dir, \FilesystemIterator::SKIP_DOTS),
                \RecursiveIteratorIterator::SELF_FIRST
            );
            foreach ($it as $f) {
                $out[] = substr($f->getPathname(), strlen($dir) + 1);
            }
            sort($out);
            return $out;
        }

        // ------------------------------------------------------------------
        // Sandbox: fake cPanel root with logging stubs + a source tree to install
        // ------------------------------------------------------------------

        /**
         * @return array<string,string> paths: base, prefix, cpanel, etc, var, cron, apps, log, engine_log, state, src
         */
        public static function sandbox(): array
        {
            $base = self::tmpDir('inst');
            $p = [
                'base' => $base,
                'prefix' => "$base/prefix",
                'cpanel' => "$base/cpanel",
                'etc' => "$base/etc-webpulse",
                'var' => "$base/var-webpulse",
                'cron' => "$base/cron.d",
                'apps' => "$base/cpanel-apps",
                'log' => "$base/stub.log",
                'engine_log' => "$base/engine.log",
                'state' => "$base/stub-state",
                'src' => "$base/src",
            ];
            foreach (['prefix', 'cron', 'apps', 'state'] as $k) {
                mkdir($p[$k], 0777, true);
            }
            self::fakeCpanel($p);
            return $p;
        }

        /** @param array<string,string> $p */
        public static function fakeCpanel(array $p): void
        {
            $c = $p['cpanel'];
            foreach (['scripts', 'bin', '3rdparty/bin', 'base/frontend/jupiter', 'whostmgr/docroot/cgi', 'whostmgr/docroot/addon_plugins'] as $d) {
                mkdir("$c/$d", 0777, true);
            }
            file_put_contents("$c/cpanel", "#!/bin/sh\n");
            chmod("$c/cpanel", 0755);
            symlink(PHP_BINARY !== '' ? PHP_BINARY : '/usr/bin/php', "$c/3rdparty/bin/php");

            $log = $p['log'];
            $state = $p['state'];
            $apps = $p['apps'];
            $head = "#!/bin/bash\nname=\$(basename \"\$0\")\necho \"\$name \$*\" >> '$log'\n"
                . "if [ -e '$state/fail.'\"\$name\" ]; then echo \"stub \$name: simulated failure\" >&2; exit 1; fi\n";

            $stubs = [
                'scripts/install_plugin' => '
[ -f "$1/install.json" ] || { echo "install.json missing in $1" >&2; exit 2; }
theme=""; while [ $# -gt 0 ]; do [ "$1" = "--theme" ] && theme=$2; shift; done
touch "' . $state . '/plugin.$theme"
echo "Plugin installed for $theme"',
                'scripts/uninstall_plugin' => '
theme=""; while [ $# -gt 0 ]; do [ "$1" = "--theme" ] && theme=$2; shift; done
rm -f "' . $state . '/plugin.$theme"',
                'scripts/restartsrv_cpsrvd' => 'echo restarted',
                'bin/register_appconfig' => '
[ -f "$1" ] || { echo "no such conf $1" >&2; exit 2; }
cp "$1" "' . $apps . '/webpulse.conf"
echo "Registered webpulse. cpsrvd must restart to show the new entry."',
                'bin/unregister_appconfig' => 'rm -f "' . $apps . '/$1.conf"',
                'bin/manage_hooks' => '
store="' . $state . '/hooks"; touch "$store"
case "$1" in
  list) cat "$store" ;;
  add)
    path=$3; event=""; while [ $# -gt 0 ]; do [ "$1" = "--event" ] && event=$2; shift; done
    [ -x "$path" ] || { echo "hook script not executable: $path" >&2; exit 3; }
    desc=$("$path" --describe) || exit 4
    case "$desc" in *"\"event\":\"$event\""*) ;; *) echo "describe mismatch: $desc" >&2; exit 5 ;; esac
    grep -Fq "$path" "$store" && { echo "hook already exists" >&2; exit 6; }
    echo "$path $event" >> "$store" ;;
  delete)
    path=$3; grep -vF "$path" "$store" > "$store.tmp"; mv "$store.tmp" "$store" ;;
  *) exit 9 ;;
esac',
            ];
            foreach ($stubs as $rel => $body) {
                file_put_contents("$c/$rel", $head . ltrim($body, "\n") . "\n");
                chmod("$c/$rel", 0755);
            }
        }

        /** Body of the stub WebPulse binary: logs its arguments, fakes `setup`. */
        public static function stubBinaryBody(string $marker = 'stub'): string
        {
            $body = <<<'SH'
#!/bin/bash
# stub binary used by the packaging tests (@MARKER@)
if [ -n "$STUB_ENGINE_LOG" ]; then
    echo "$* [cfg=$WEBPULSE_CONFIG]" >> "$STUB_ENGINE_LOG"
fi
case "$1" in
    --version) echo "webpulse 0.0.0-@MARKER@"; exit 0 ;;
    setup)
        if [ -n "$STUB_SETUP_FAIL" ]; then echo "setup exploded" >&2; exit 7; fi
        [ -f "$WEBPULSE_CONFIG" ] || { echo "stub engine: no config" >&2; exit 1; }
        for k in data_dir log_dir run_dir; do
            d=$(sed -n "s/.*\"$k\": *\"\([^\"]*\)\".*/\1/p" "$WEBPULSE_CONFIG" | head -n 1)
            if [ -n "$d" ]; then mkdir -p "$d"; fi
        done
        ;;
esac
exit 0

SH;
            return str_replace('@MARKER@', $marker, $body);
        }

        /**
         * Create a self-contained unpacked release: the real packaging files plus a
         * stub binary that records its invocations (bin/webpulse) and a stub default
         * config (so the test does not depend on the progress of the Rust code).
         * This is the layout scripts/build_release.sh produces.
         */
        public static function makeSource(array $p, string $version = '0.1.0', ?string $dir = null, string $marker = 'stub'): string
        {
            $src = $dir ?? $p['src'];
            if (!is_dir($src)) {
                mkdir($src, 0777, true);
            }
            $repo = self::repo();
            foreach (['cpanel', 'whm', 'hooks', 'installer', 'install.sh', 'uninstall.sh'] as $item) {
                [$rc, $out] = self::run(['cp', '-a', "$repo/$item", "$src/$item"]);
                if ($rc !== 0) {
                    self::fail("cp $item: $out");
                }
            }
            // etc/: our cron file + a stub default config
            mkdir("$src/etc/cron.d", 0777, true);
            copy("$repo/etc/cron.d/webpulse", "$src/etc/cron.d/webpulse");
            file_put_contents("$src/etc/config.json.default", json_encode([
                'version' => 1,
                'exclude_paths' => ['/.well-known/', '/cpanel'],
                'geoip' => ['enabled' => false, 'country_db' => '/var/webpulse/geoip/dbip-country-lite.mmdb', 'city_db' => null],
                'php_bin' => null,
                'paths' => [
                    'data_dir' => '/var/webpulse/data',
                    'registry_db' => '/var/webpulse/webpulse.db',
                    'log_dir' => '/var/webpulse/log',
                    'run_dir' => '/var/webpulse/run',
                    'domlogs_dirs' => ['/etc/apache2/logs/domlogs/{user}', '/var/log/nginx/domains'],
                    'archive_dir' => '/home/{user}/logs',
                    'userdomains_file' => '/etc/userdomains',
                    'cpanel_users_dir' => '/var/cpanel/users',
                    'home_root' => '/home',
                    'cron_file' => '/etc/cron.d/webpulse',
                    'hooks_file' => '/var/cpanel/hooks/data/hooks.json',
                ],
            ], JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) . "\n");
            self::script("$src/bin/webpulse", self::stubBinaryBody($marker));
            foreach (['LICENSE', 'README.md'] as $f) {
                copy("$repo/$f", "$src/$f");
            }
            file_put_contents("$src/VERSION", $version . "\n");
            return $src;
        }

        /**
         * Pack an unpacked release like scripts/build_release.sh does: one top-level
         * directory webpulse-<ver>-<target>/, plus a .sha256 next to the tarball.
         *
         * @return array{0:string,1:string} tarball path, sha256 file path
         */
        public static function releaseTarball(string $srcDir, string $outDir, string $target = 'x86_64-unknown-linux-musl'): array
        {
            $ver = trim((string) file_get_contents("$srcDir/VERSION"));
            $name = "webpulse-$ver-$target";
            if (!is_dir($outDir)) {
                mkdir($outDir, 0777, true);
            }
            $tarball = "$outDir/$name.tar.gz";
            [$rc, $out] = self::run(['tar', '-czf', $tarball, '--transform', "s,^\\.,$name,", '-C', $srcDir, '.']);
            if ($rc !== 0) {
                self::fail("tar: $out");
            }
            file_put_contents("$tarball.sha256", hash_file('sha256', $tarball) . "  $name.tar.gz\n");
            return [$tarball, "$tarball.sha256"];
        }

        /** Environment that redirects install.sh into the sandbox. @return array<string,string> */
        public static function installerEnv(array $p): array
        {
            return [
                'WEBPULSE_PREFIX' => $p['prefix'],
                'WEBPULSE_FAKE_CPANEL' => '1',
                'WEBPULSE_CPANEL_ROOT' => $p['cpanel'],
                'WEBPULSE_ETC' => $p['etc'],
                'WEBPULSE_VAR' => $p['var'],
                'WEBPULSE_CRON_DIR' => $p['cron'],
                'WEBPULSE_APPS_DIR' => $p['apps'],
                'WEBPULSE_SKIP_FIRST_COLLECT' => '1',
                'WEBPULSE_CARGO' => '/nonexistent/cargo', // never build for real by accident
                'STUB_ENGINE_LOG' => $p['engine_log'],
                'TMPDIR' => $p['base'],
                'NO_COLOR' => '1',
            ];
        }

        /**
         * Run install.sh of $srcDir inside the sandbox.
         *
         * @param array<string,string> $p
         * @param list<string> $args
         * @param array<string,string> $env
         * @return array{0:int,1:string}
         */
        public static function installer(array $p, string $srcDir, array $args, array $env = []): array
        {
            return self::run(array_merge(['bash', "$srcDir/install.sh"], $args), array_merge(self::installerEnv($p), $env), '');
        }

        /**
         * Run the installer the way `curl | bash` does (script on stdin, no local source).
         *
         * @return array{0:int,1:string}
         */
        public static function installerPiped(array $p, array $args, array $env = []): array
        {
            $script = (string) file_get_contents(self::repo() . '/install.sh');
            return self::run(array_merge(['bash', '-s', '--'], $args), array_merge(self::installerEnv($p), $env), $script);
        }

        /** @return list<string> */
        public static function stubLog(array $p): array
        {
            return is_file($p['log']) ? array_values(array_filter(array_map('rtrim', file($p['log']) ?: []))) : [];
        }
    }
}

return [];
