<?php

declare(strict_types=1);

use WebPulse\App;
use WebPulse\Collect\UserSettings;
use WebPulse\Config;
use WebPulse\Discovery\CpanelUsers;
use WebPulse\Discovery\LogFiles;
use WebPulse\Paths;
use WebPulse\Registry\Registry;
use WebPulse\Store\DomainStore;
use WebPulse\Store\NotFound;
use WebPulse\Store\SchemaTooNew;
use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\T;

return [
    'App::boot contract: config, paths, version, WEBPULSE_CONFIG' => static function (): void {
        $fr = FakeRoot::create('reg-app');
        $app = App::boot($fr->configPath());
        T::eq('0.1.0', $app->version());
        T::eq($fr->path('var/webpulse/data'), $app->paths->dataDir());
        T::eq($fr->path('var/webpulse/data/alice'), $app->paths->userDir('alice'));
        T::eq($fr->path('var/webpulse/data/alice/example.com.db'), $app->paths->domainDb('alice', 'example.com'));
        T::eq($fr->path('var/webpulse/data/alice/user/settings.json'), $app->paths->userSettingsFile('alice'));
        T::eq($fr->path('var/webpulse/webpulse.db'), $app->paths->registryDb());
        T::eq(FakeRoot::repo(), $app->paths->root());
        T::eq('Europe/Rome', $app->config->get('timezone'));
        T::eq(30, $app->config->get('session_timeout_min'));
        T::eq('x', $app->config->get('no.such.key', 'x'));
        putenv('WEBPULSE_CONFIG=' . $fr->configPath());
        try {
            T::eq($fr->configPath(), App::boot()->configPath());
            putenv('WEBPULSE_CONFIG=/nonexistent/config.json');
            T::throws(\RuntimeException::class, static fn () => App::boot(), 'not found');
        } finally {
            putenv('WEBPULSE_CONFIG');
        }
        T::throws(NotFound::class, static fn () => $app->openDomain('alice', 'example.com', true));
        T::throws(\InvalidArgumentException::class, static fn () => $app->openDomain('../x', 'example.com', true));
        T::throws(\InvalidArgumentException::class, static fn () => $app->openDomain('alice', '../../etc/passwd', true));
    },
    'paths reject traversal' => static function (): void {
        foreach (['alice', 'a1', 'user_name', 'r-1'] as $u) {
            T::true(Paths::isValidUser($u), $u);
        }
        foreach (['', '..', 'a/b', 'Alice', 'a.b', str_repeat('a', 40), "a\0"] as $u) {
            T::false(Paths::isValidUser($u), json_encode($u));
        }
        foreach (['example.com', 'shop.example.com', 'xn--bcher-kva.example', 'a-b.co.uk', 'localhost'] as $d) {
            T::true(Paths::isValidDomain($d), $d);
        }
        foreach (['', '..', '../x', 'a/b', 'Example.com', '-a.com', 'a..b', 'a.com/', '*.example.com'] as $d) {
            T::false(Paths::isValidDomain($d), $d);
        }
    },
    'config precedence: defaults < file < registry overrides' => static function (): void {
        $fr = FakeRoot::create('reg-config', ['raw_retention_days' => 45]);
        $app = App::boot($fr->configPath());
        T::eq(45, $app->config->get('raw_retention_days'));
        T::eq(1000, $app->config->get('max_rows_per_dim'));
        $app->config->setOverride('raw_retention_days', 90);
        $app->config->setOverride('geoip.enabled', 'true');
        T::eq(90, $app->config->get('raw_retention_days'));
        T::eq(true, $app->config->get('geoip.enabled'));
        T::eq($fr->path('var/webpulse/geoip/dbip-country-lite.mmdb'), $app->config->get('geoip.country_db'), 'sibling keys kept');
        $fresh = App::boot($fr->configPath());
        T::eq(90, $fresh->config->get('raw_retention_days'));
        T::eq(90, $fresh->config->effective()['raw_retention_days']);
        T::eq(['geoip.enabled' => true, 'raw_retention_days' => 90], $fresh->registry()->settings());
        $fresh->config->unsetOverride('raw_retention_days');
        T::eq(45, $fresh->config->get('raw_retention_days'));
        T::throws(\InvalidArgumentException::class, static fn () => $app->config->setOverride('paths.data_dir', '/tmp'));
        T::throws(\InvalidArgumentException::class, static fn () => $app->config->setOverride('max_rows_per_dim', 5));
        T::throws(\InvalidArgumentException::class, static fn () => $app->config->setOverride('timezone', 'Mars/Base'));
        T::throws(\InvalidArgumentException::class, static fn () => $app->config->setOverride('exclude_ips', ['nope']));
        T::throws(\InvalidArgumentException::class, static fn () => $app->config->setOverride('exclude_paths', ['relative']));
        T::eq(['1.2.3.0/24'], Config::validate('exclude_ips', [' 1.2.3.0/24 ']));
        T::null(Config::validate('timezone', ''));
        // unreadable / missing file: defaults
        T::eq('/var/webpulse/data', Config::load('/nonexistent.json')->get('paths.data_dir'));
        $bad = $fr->dir() . '/bad.json';
        file_put_contents($bad, '{oops');
        T::throws(\RuntimeException::class, static fn () => Config::load($bad), 'invalid JSON');
        // relative paths are resolved against the config directory
        file_put_contents($fr->dir() . '/rel.json', json_encode(['paths' => ['data_dir' => 'data']]));
        T::eq($fr->dir() . '/data', Config::load($fr->dir() . '/rel.json')->get('paths.data_dir'));
    },
    'registry: effective enabled matrix' => static function (): void {
        foreach ([
            [null, null, true, true], [null, null, false, false],
            [true, null, false, true], [false, null, true, false],
            [null, true, false, true], [null, false, true, false],
            [true, false, true, false], [false, true, true, false], [true, true, false, true],
        ] as [$acc, $dom, $def, $exp]) {
            T::eq($exp, Registry::effective($acc, $dom, $def), json_encode([$acc, $dom, $def]));
        }
        $fr = FakeRoot::create('reg-switch');
        $app = App::boot($fr->configPath());
        $reg = $app->registry();
        T::true($reg->isEnabled('alice', 'example.com'), 'unknown -> default');
        $reg->setAccount('alice', false);
        T::false($reg->isEnabled('alice', 'example.com'));
        T::eq('root', $reg->account('alice')['owner'], 'owner filled from cPanel data');
        $reg->setDomain('alice', 'example.com', true);
        T::false($reg->isEnabled('alice', 'example.com'));
        $reg->setAccount('alice', null, 'Europe/Paris');
        T::true($reg->isEnabled('alice', 'example.com'));
        T::eq('Europe/Paris', $reg->account('alice')['timezone']);
        $reg->setAccount('alice', null);
        T::eq('Europe/Paris', $reg->account('alice')['timezone'], 'null timezone = unchanged');
        $reg->setAccount('alice', null, '');
        T::null($reg->account('alice')['timezone']);
        T::throws(\InvalidArgumentException::class, static fn () => $reg->setAccount('alice', true, 'Nowhere/Land'));
        T::throws(\InvalidArgumentException::class, static fn () => $reg->setDomain('alice', 'bad/../domain', true));
        $rows = $reg->domains('alice');
        T::eq([['example.com', true, true]], array_map(static fn ($r) => [$r['domain'], $r['enabled'], $r['enabled_override']], $rows));
    },
    'registry: sync, removal, owner filter, search' => static function (): void {
        $fr = FakeRoot::create('reg-sync');
        $app = App::boot($fr->configPath());
        (new WebPulse\Collect\Collector($app))->syncRegistry();
        $reg = $app->registry();
        T::eq(['alice', 'bob'], array_column($reg->accounts(), 'user'));
        T::eq(['bob'], array_column($reg->accounts('reseller1'), 'user'));
        T::eq(['alice'], array_column($reg->accounts(null, 'shop'), 'user'));
        T::eq(2, $reg->accounts(null, 'ali')[0]['domains']);
        unlink($fr->path('var/cpanel/users/bob'));
        $app2 = App::boot($fr->configPath());
        (new WebPulse\Collect\Collector($app2))->syncRegistry();
        $bob = $app2->registry()->account('bob');
        T::notNull($bob['removed_at']);
        T::notNull($app2->registry()->domains('bob')[0]['removed_at']);
        // a log file of a domain missing from userdomains is discovered too
        $fr->writeLog('etc/apache2/logs/domlogs/alice/old.example.org-ssl_log', ['x']);
        $fr->writeLog('etc/apache2/logs/domlogs/alice/example.com-bytes_log', ['1']);
        $app3 = App::boot($fr->configPath());
        (new WebPulse\Collect\Collector($app3))->syncRegistry('alice');
        T::eq(['example.com' => 'main', 'old.example.org' => 'unknown', 'shop.example.com' => 'sub'],
            array_column($app3->registry()->domains('alice'), 'kind', 'domain'));
    },
    'registry: salts, jobs, summaries' => static function (): void {
        $fr = FakeRoot::create('reg-misc');
        $reg = App::boot($fr->configPath())->registry();
        $a = $reg->salt('2026-10-10');
        T::eq(32, strlen($a));
        T::eq($a, $reg->salt('2026-10-10'));
        T::eq($a, App::boot($fr->configPath())->registry()->salt('2026-10-10'), 'persisted');
        T::true($a !== $reg->salt('2026-10-11'));
        T::eq(1, $reg->pruneSalts('2026-10-11'));
        T::true($a !== $reg->salt('2026-10-10'), 'pruned salt is regenerated');
        $ids = [];
        for ($i = 0; $i < 505; $i++) {
            $ids[] = $reg->startJob('collect', 'all');
        }
        T::eq(500, (int) $reg->pdo()->query('SELECT COUNT(*) FROM jobs')->fetchColumn());
        $reg->finishJob(end($ids), 'error', ['lines' => 7, 'files' => 2, 'domains' => 1], "bad \xff message");
        $j = $reg->jobs(1)[0];
        T::eq(['error', 7, 2, 1, 'bad ? message'], [$j['status'], $j['lines'], $j['files'], $j['domains'], $j['message']]);
        T::eq(50, count($reg->jobs()));
        T::eq([], $reg->summaries([]));
    },
    'discovery: cPanel users, kinds, owners, homes' => static function (): void {
        $fr = FakeRoot::create('reg-cpusers');
        $app = App::boot($fr->configPath());
        $cp = new CpanelUsers($app->config, $app->paths);
        $all = $cp->all();
        T::eq(['alice', 'bob'], array_keys($all));
        T::eq('root', $all['alice']['owner']);
        T::eq('reseller1', $cp->owner('bob'));
        T::null($cp->owner('ghost'));
        T::false($cp->exists('ghost'));
        T::false($cp->exists('../etc'));
        T::eq(['example.com' => 'main', 'shop.example.com' => 'sub'], $cp->domainsOf('alice'));
        T::eq(['example.com', 'shop.example.com'], $all['alice']['domains']);
        T::eq($fr->path('home/alice'), $all['alice']['home']);
        T::eq(['blog.net' => 'main'], $cp->domainsOf('bob'));
        // heuristics without userdata + richer files
        unlink($fr->path('var/cpanel/userdata/alice/main'));
        file_put_contents($fr->path('var/cpanel/users/carol'), "DNS=carol.io\nDNS1=other.net\nDNS2=blog.carol.io\nDNS3=parked.org\nOWNER=reseller1\n");
        file_put_contents($fr->path('etc/userdomains'), "dev.other.net: carol\n", FILE_APPEND);
        @mkdir($fr->path('var/cpanel/users/.hidden'));
        file_put_contents($fr->path('var/cpanel/users/system'), "DNS=x.com\n");
        $cp2 = new CpanelUsers($app->config, $app->paths);
        T::eq(['alice', 'bob', 'carol'], array_keys($cp2->all()));
        T::eq(['blog.carol.io' => 'sub', 'carol.io' => 'main', 'dev.other.net' => 'sub', 'other.net' => 'addon', 'parked.org' => 'addon'], $cp2->domainsOf('carol'));
        // userdata with addon / parked / sub
        @mkdir($fr->path('var/cpanel/userdata/carol'), 0755, true);
        file_put_contents($fr->path('var/cpanel/userdata/carol/main'), "---\naddon_domains:\n  other.net: other.carol.io\nmain_domain: carol.io\nparked_domains:\n  - parked.org\nsub_domains:\n  - blog.carol.io\n  - other.carol.io\n");
        $cp3 = new CpanelUsers($app->config, $app->paths);
        T::eq(['blog.carol.io' => 'sub', 'carol.io' => 'main', 'dev.other.net' => 'sub', 'other.net' => 'addon', 'parked.org' => 'parked'], $cp3->domainsOf('carol'));
    },
    'discovery: log files, ignore patterns, symlinked dirs, archives' => static function (): void {
        $fr = FakeRoot::create('reg-logfiles');
        $app = App::boot($fr->configPath());
        $lf = new LogFiles($app->config, $app->paths);
        foreach (['example.com', 'example.com-ssl_log', 'example.com-bytes_log', 'example.com-ftp_log', 'ftpxferlog', 'example.com.bkup', 'example.com.offset'] as $f) {
            $fr->writeLog("etc/apache2/logs/domlogs/alice/$f", ['x']);
        }
        // /usr/local/apache/domlogs/alice -> same directory (old layout symlink)
        @mkdir($fr->path('usr/local/apache/domlogs'), 0755, true);
        symlink($fr->path('etc/apache2/logs/domlogs/alice'), $fr->path('usr/local/apache/domlogs/alice'));
        $fr->writeLog('var/log/nginx/domains/shop.example.com', ['x']);
        $files = $lf->forDomain('alice', 'example.com');
        T::eq([[$fr->domlog('alice', 'example.com'), false], [$fr->domlog('alice', 'example.com-ssl_log'), true]],
            array_map(static fn ($f) => [$f['path'], $f['https']], $files));
        T::eq([$fr->path('var/log/nginx/domains/shop.example.com')], array_column($lf->forDomain('alice', 'shop.example.com'), 'path'));
        T::eq(['example.com'], $lf->domainsInUserDirs('alice'));
        foreach (['example.com-bytes_log', 'x-ftp_log', 'ftpxferlog.1', 'a.bkup2', 'a.offset', 'a.gz', '.hidden'] as $n) {
            T::true(LogFiles::isIgnored($n), $n);
        }
        T::false(LogFiles::isIgnored('example.com-ssl_log'));
        // archives
        $logs = $fr->path('home/alice/logs');
        file_put_contents("$logs/example.com-Sep-2026.gz", gzencode("x\n"));
        file_put_contents("$logs/example.com-ssl_log-Sep-2026.gz", gzencode("x\n"));
        file_put_contents("$logs/shop.example.com-Sep-2026.gz", gzencode("x\n"));
        file_put_contents("$logs/example.com-bytes_log-Sep-2026.gz", gzencode("x\n"));
        file_put_contents("$logs/unrelated.org-Sep-2026.gz", gzencode("x\n"));
        file_put_contents($fr->dir() . '/secret.gz', gzencode("secret\n"));
        symlink($fr->dir() . '/secret.gz', "$logs/example.com-Aug-2026.gz");
        $arch = $lf->archives('alice', ['example.com', 'shop.example.com']);
        T::eq(['example.com', 'shop.example.com'], array_keys($arch));
        T::eq(['example.com-Sep-2026.gz' => false, 'example.com-ssl_log-Sep-2026.gz' => true],
            array_column(array_map(static fn ($a) => ['n' => basename($a['path']), 'h' => $a['https']], $arch['example.com']), 'h', 'n'));
        T::eq(1, count($arch['shop.example.com']));
        // a symlinked archive directory is refused
        rename($logs, $logs . '.real');
        symlink($logs . '.real', $logs);
        T::eq([], $lf->archives('alice', ['example.com']));
    },
    'user settings: validation, defaults, symlink and size checks' => static function (): void {
        $v = UserSettings::validate(['version' => 1, 'domains' => ['Example.COM' => ['timezone' => 'Europe/Rome', 'exclude_ips' => ['1.2.3.4', '2001:db8::/32'], 'exclude_paths' => ['/x/']]], 'ui' => ['dark' => 'on']]);
        T::eq('Europe/Rome', $v['domains']['example.com']['timezone']);
        T::eq('dark', $v['ui']['dark']);
        T::eq('30d', $v['ui']['default_range']);
        T::eq(UserSettings::domainDefaults(), UserSettings::forDomain($v, 'other.com'));
        $e = T::throws(\InvalidArgumentException::class, static fn () => UserSettings::validate([
            'version' => 2, 'extra' => 1,
            'domains' => ['bad domain' => [], 'ok.com' => ['timezone' => 'Mars/X', 'exclude_ips' => ['999.1.1.1'], 'exclude_paths' => ['nope'], 'zzz' => 1]],
            'ui' => ['default_range' => 'forever', 'dark' => 'purple'],
        ]));
        foreach (['version must be 1', "unknown key 'extra'", 'invalid domain', 'invalid timezone', 'invalid IP/CIDR', 'invalid path prefix', "unknown key 'zzz'", 'ui.default_range', 'ui.dark'] as $msg) {
            T::contains($msg, $e->getMessage());
        }
        T::throws(\InvalidArgumentException::class, static fn () => UserSettings::validate(['domains' => ['a.com' => ['exclude_ips' => array_fill(0, 51, '1.1.1.1')]]]), 'at most 50');

        $fr = FakeRoot::create('reg-usersettings');
        $app = App::boot($fr->configPath());
        T::eq(UserSettings::defaults(), UserSettings::read($app->paths, 'alice', $err));
        T::null($err);
        DomainStore::prepareUserDir($app->paths, 'alice');
        $file = $app->paths->userSettingsFile('alice');
        $written = UserSettings::write($app->paths, 'alice', ['domains' => ['example.com' => ['exclude_paths' => ['/admin/']]]]);
        T::eq(['/admin/'], UserSettings::read($app->paths, 'alice')['domains']['example.com']['exclude_paths']);
        T::eq($written, UserSettings::read($app->paths, 'alice'));
        file_put_contents($file, '{not json');
        T::eq(UserSettings::defaults(), UserSettings::read($app->paths, 'alice', $err));
        T::contains('invalid JSON', (string) $err);
        file_put_contents($file, json_encode(['ui' => ['x' => str_repeat('a', 70000)]]));
        UserSettings::read($app->paths, 'alice', $err);
        T::contains('64 KiB', (string) $err);
        unlink($file);
        file_put_contents($fr->dir() . '/target.json', json_encode(['domains' => ['example.com' => ['exclude_paths' => ['/leak/']]]]));
        symlink($fr->dir() . '/target.json', $file);
        T::eq(UserSettings::defaults(), UserSettings::read($app->paths, 'alice', $err));
        T::contains('not a regular file', (string) $err);
        unlink($file);
        // symlinked user/ directory
        rmdir(dirname($file));
        symlink($fr->dir(), dirname($file));
        T::eq(UserSettings::defaults(), UserSettings::read($app->paths, 'alice', $err));
        T::contains('not a directory', (string) $err);
    },
    'domain store: creation, modes, read-only access, schema guard' => static function (): void {
        $fr = FakeRoot::create('reg-store');
        $app = App::boot($fr->configPath());
        $w = $app->openDomain('alice', 'example.com', false);
        T::false($w->readOnly());
        $m = $w->meta();
        T::eq(['1', 'alice', 'example.com', '1'], [$m['schema_version'], $m['user'], $m['domain'], $m['next_visit_id']]);
        T::eq('640', sprintf('%o', fileperms($w->path()) & 0777));
        T::eq('750', sprintf('%o', fileperms($app->paths->userDir('alice')) & 0777));
        T::eq('delete', $w->pdo()->query('PRAGMA journal_mode')->fetchColumn());
        $w->setMeta('timezone', 'Europe/Rome');
        $r = $app->openDomain('alice', 'example.com', true);
        T::true($r->readOnly());
        T::eq('Europe/Rome', $r->meta()['timezone']);
        T::throws(\PDOException::class, static fn () => $r->pdo()->exec("INSERT INTO meta VALUES ('x', 'y')"));
        T::throws(\LogicException::class, static fn () => $r->setMeta('x', 'y'));
        $w->setMeta('schema_version', '99');
        T::throws(SchemaTooNew::class, static fn () => $app->openDomain('alice', 'example.com', true));
        T::throws(SchemaTooNew::class, static fn () => $app->openDomain('alice', 'example.com', false));
        $tables = $app->openDomain('alice', 'shop.example.com', false)->pdo()->query("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")->fetchAll(PDO::FETCH_COLUMN);
        T::eq(['checkpoints', 'daily', 'dim_daily', 'events', 'hourly', 'imported_archives', 'meta', 'open_visits', 'visitor_day', 'visits'], $tables);
    },
];
