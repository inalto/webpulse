<?php

declare(strict_types=1);

use WebPulse\Collect\Collector;
use WebPulse\Tests\FakeRoot;
use WebPulse\Tests\Lines as L;
use WebPulse\Tests\SeedDb;
use WebPulse\Tests\T;

/*
 * Security review H1 — a reseller must not get another tenant's domain
 * collected into one of its accounts (shared domlog directories such as
 * /etc/apache2/logs/domlogs hold every tenant's logs).
 */

require_once dirname(__DIR__) . '/support/seed_db.php';

$status = static fn (array $env, string $p, array $q, ?array $body = null): int => SeedDb::api($env, $p, $q, $body)['status'];

return [
    'admin API: set_domain/run/purge refuse a domain that is not the account\'s' => static function () use ($status): void {
        $env = SeedDb::fresh('sec-tenant-api');
        // example.com is alice's: reseller1 (owner of bob) must not attach it to bob
        T::eq(404, $status($env, 'reseller:reseller1', ['a' => 'admin.set_domain'], ['user' => 'bob', 'domain' => 'example.com', 'enabled' => true]), 'foreign domain via own account');
        T::eq(404, $status($env, 'reseller:reseller1', ['a' => 'admin.set_domain'], ['user' => 'bob', 'domain' => 'victim.example', 'enabled' => true]), 'unknown domain');
        T::eq(404, $status($env, 'admin', ['a' => 'admin.set_domain'], ['user' => 'bob', 'domain' => 'example.com', 'enabled' => true]), 'root cannot mis-attach either');
        $reg = \WebPulse\App::boot($env['config'])->registry();
        T::eq(['blog.net'], array_column($reg->domains('bob'), 'domain'), 'no registry row was created');
        // own domains still work (registry row, cPanel files, existing database)
        T::eq(200, $status($env, 'reseller:reseller1', ['a' => 'admin.set_domain'], ['user' => 'bob', 'domain' => 'blog.net', 'enabled' => false]));
        T::eq(200, $status($env, 'admin', ['a' => 'admin.set_domain'], ['user' => 'alice', 'domain' => 'SHOP.example.com', 'enabled' => null]), 'case-insensitive');
        T::eq(404, $status($env, 'reseller:reseller1', ['a' => 'admin.run'], ['type' => 'collect', 'user' => 'bob', 'domain' => 'example.com']), 'run with foreign domain');
        T::eq(404, $status($env, 'admin', ['a' => 'admin.purge'], ['user' => 'bob', 'domain' => 'example.com', 'confirm' => 'DELETE']), 'purge with foreign domain');
        T::true(is_file($env['root'] . '/var/webpulse/data/alice/example.com.db'));
    },
    'collector: a registry row alone never opens shared domlogs of another tenant' => static function (): void {
        $fr = FakeRoot::create('sec-tenant-collect');
        $line = L::line('203.0.113.7', '2026-10-10 10:00:00', '/secret-page');
        // alice's example.com logs in the two shared (no {user}) directories
        $fr->writeLog('etc/apache2/logs/domlogs/example.com', [$line]);
        $fr->writeLog('var/log/nginx/domains/example.com-ssl_log', [$line]);
        $fr->writeLog('etc/apache2/logs/domlogs/bob/blog.net', [L::line('198.51.100.1', '2026-10-10 10:00:00', '/')]);
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        $app = $fr->app();
        $collector = new Collector($app);
        $collector->syncRegistry();
        // the race of the review: the row is inserted after the global sync
        $app->registry()->setDomain('bob', 'example.com', true);
        $r = $collector->collectUser('bob');
        T::false(is_file($fr->path('var/webpulse/data/bob/example.com.db')), 'no database for the foreign domain');
        T::eq(1, $r['domains'], 'only blog.net collected');
        T::eq(1, $r['lines']);
        T::false($collector->ownsDomain('bob', 'example.com'));
        T::true($collector->ownsDomain('alice', 'example.com'));
        T::contains('not reading the shared logs of example.com for bob', (string) file_get_contents($fr->path('var/webpulse/log/webpulse.log')));
        // the owner still gets them
        $r = $collector->collectUser('alice');
        T::eq(2, (int) $fr->scalar('alice', 'example.com', "SELECT SUM(hits) FROM daily"), 'both shared logs read for the owner');
        // a domain only present in a per-user directory (no cPanel entry) is still collected from there
        $fr->writeLog('etc/apache2/logs/domlogs/bob/extra.example', [L::line('198.51.100.2', '2026-10-10 10:00:00', '/')]);
        $fr->writeLog('etc/apache2/logs/domlogs/extra.example', [L::line('198.51.100.3', '2026-10-10 10:00:00', '/')]);
        $collector->syncRegistry('bob');
        $collector->collectUser('bob');
        T::eq(1, (int) $fr->scalar('bob', 'extra.example', 'SELECT SUM(hits) FROM daily'), 'per-user dir only, not the shared copy');
    },
    'collector: the global run re-syncs each account before collecting it' => static function (): void {
        $fr = FakeRoot::create('sec-tenant-global');
        $fr->writeLog('etc/apache2/logs/domlogs/example.com', [L::line('203.0.113.7', '2026-10-10 10:00:00', '/')]);
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        $fr->app()->registry()->setDomain('bob', 'example.com', true);
        $r = $fr->run('collect --json');
        T::eq(0, $r['code'], $r['stderr']);
        T::false(is_file($fr->path('var/webpulse/data/bob/example.com.db')));
        $rows = array_values(array_filter($fr->app()->registry()->domains('bob'), static fn ($d) => $d['domain'] === 'example.com'));
        T::true($rows[0]['removed_at'] !== null, 'injected row marked removed');
    },
    'backfill: a shared archive directory only yields the account\'s own domains' => static function (): void {
        $fr = FakeRoot::create('sec-tenant-backfill');
        $fr->patchConfig(['paths' => ['archive_dir' => $fr->path('var/archives')]]);
        @mkdir($fr->path('var/archives'), 0755, true);
        file_put_contents($fr->path('var/archives/example.com-Oct-2026.gz'), gzencode(L::line('203.0.113.7', '2026-10-01 10:00:00', '/') . "\n"));
        file_put_contents($fr->path('var/archives/blog.net-Oct-2026.gz'), gzencode(L::line('198.51.100.1', '2026-10-01 10:00:00', '/') . "\n"));
        $fr->setNow(L::ts('2026-10-10 12:00:00'));
        $app = $fr->app();
        $collector = new Collector($app);
        $collector->syncRegistry();
        $app->registry()->setDomain('bob', 'example.com', true);
        $r = $collector->backfillUser('bob');
        T::true(in_array('example.com (not a domain of bob)', $r['skipped'], true), json_encode($r));
        T::false(is_file($fr->path('var/webpulse/data/bob/example.com.db')));
        T::eq(1, (int) $fr->scalar('bob', 'blog.net', 'SELECT SUM(hits) FROM daily'));
    },
];
