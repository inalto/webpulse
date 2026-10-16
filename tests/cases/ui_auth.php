<?php

declare(strict_types=1);

use WebPulse\Http\DomainResolver;
use WebPulse\Http\Principal;
use WebPulse\Tests\SeedDb;
use WebPulse\Tests\T;

/*
 * WP2 — authorization matrix: who may see which domain (404 otherwise), the
 * reseller scope, admin-only actions and hostile domain/user names.
 */

require_once dirname(__DIR__) . '/support/seed_db.php';

$status = static fn (array $env, string $p, array $q, ?array $body = null): int => SeedDb::api($env, $p, $q, $body)['status'];
$code = static fn (array $env, string $p, array $q, ?array $body = null): ?string => SeedDb::api($env, $p, $q, $body)['json']['error']['code'] ?? null;

return [
    'domain visibility matrix' => function () use ($status) {
        $env = SeedDb::shared();
        // [principal, domain, user param, expected status]
        $matrix = [
            ['user:alice', 'example.com', null, 200],
            ['user:alice', 'shop.example.com', null, 200],
            ['user:alice', 'blog.net', null, 404],
            ['user:alice', 'blog.net', 'bob', 404],
            ['user:bob', 'blog.net', null, 200],
            ['user:bob', 'example.com', null, 404],
            ['user:bob', 'example.com', 'alice', 404],
            ['user:mallory', 'example.com', null, 404],
            ['reseller:reseller1', 'blog.net', null, 200],
            ['reseller:reseller1', 'example.com', null, 404],
            ['reseller:reseller1', 'example.com', 'alice', 404],
            ['reseller:nobody', 'blog.net', null, 404],
            ['admin', 'example.com', null, 200],
            ['admin', 'blog.net', 'bob', 200],
            ['admin', 'blog.net', 'alice', 404],
            ['admin', 'nosuch.example', null, 404],
        ];
        foreach (['overview', 'realtime', 'export'] as $a) {
            foreach ($matrix as [$p, $d, $u, $exp]) {
                $q = ['a' => $a, 'domain' => $d, 'name' => 'page'];
                if ($u !== null) {
                    $q['user'] = $u;
                }
                T::eq($exp, $status($env, $p, $q), "$a as $p on $d" . ($u ? " (user=$u)" : ''));
            }
        }
    },
    'domains list per principal' => function () {
        $env = SeedDb::shared();
        $list = static fn (string $p) => array_map(
            static fn ($d) => $d['user'] . '/' . $d['domain'],
            SeedDb::api($env, $p, ['a' => 'domains'])['json']['data']['domains']
        );
        T::eq(['alice/example.com', 'alice/shop.example.com'], $list('user:alice'));
        T::eq(['bob/blog.net'], $list('user:bob'));
        T::eq(['carol/carol.org'], $list('user:carol'), 'registered domain without data ("no data yet" state)');
        T::eq([], $list('user:mallory'), 'unknown account sees nothing');
        T::eq(['bob/blog.net', 'carol/carol.org'], $list('reseller:reseller1'));
        T::eq(['alice/example.com', 'alice/shop.example.com', 'bob/blog.net', 'carol/carol.org', 'dave/dave.io'], $list('admin'));
        $carol = array_values(array_filter(SeedDb::api($env, 'admin', ['a' => 'domains'])['json']['data']['domains'], static fn ($d) => $d['user'] === 'carol'))[0];
        T::false($carol['has_data'], 'registered without data');
    },
    'hostile domain and user parameters' => function () use ($status, $code) {
        $env = SeedDb::shared();
        foreach (['../bob/blog.net', 'blog.net/../../x', '..', '.', '/etc/passwd', 'EXAMPLE.COM/..', "example.com\0", 'exa mple.com',
            str_repeat('a', 300), '-rf', '.hidden', 'example.com.db', '%2e%2e'] as $bad) {
            T::eq(404, $status($env, 'user:alice', ['a' => 'overview', 'domain' => $bad]), 'domain ' . json_encode($bad));
        }
        T::eq(200, $status($env, 'user:alice', ['a' => 'overview', 'domain' => 'EXAMPLE.COM']), 'domains are case-insensitive');
        foreach (['../alice', 'alice/..', 'root;', ''] as $u) {
            T::true(in_array($status($env, 'admin', ['a' => 'overview', 'domain' => 'example.com', 'user' => $u]), [200, 404], true));
        }
        T::eq(404, $status($env, 'admin', ['a' => 'overview', 'domain' => 'example.com', 'user' => '../alice']));
        T::false(DomainResolver::validDomain('a..b'));
        T::false(DomainResolver::validDomain('a/b'));
        T::true(DomainResolver::validDomain('xn--bcher-kva.example'));
        T::eq('not_found', $code($env, 'user:alice', ['a' => 'overview', 'domain' => 'blog.net']), 'same answer as a missing domain');
    },
    'admin scope: users and resellers' => function () use ($status, $code) {
        $env = SeedDb::fresh('ui-auth-admin');
        foreach (['admin.overview', 'admin.accounts', 'admin.settings', 'admin.jobs'] as $a) {
            T::eq(403, $status($env, 'user:alice', ['a' => $a]), "$a as user");
            T::eq(200, $status($env, 'reseller:reseller1', ['a' => $a]), "$a as reseller");
            T::eq(200, $status($env, 'admin', ['a' => $a]), "$a as admin");
        }
        $acc = SeedDb::api($env, 'reseller:reseller1', ['a' => 'admin.accounts'])['json']['data']['accounts'];
        T::eq(['bob', 'carol'], array_column($acc, 'user'), 'reseller only sees owned accounts');
        $top = SeedDb::api($env, 'reseller:reseller1', ['a' => 'admin.overview'])['json']['data']['top_domains'];
        T::eq(['blog.net'], array_column($top, 'domain'));
        T::eq(404, $status($env, 'reseller:reseller1', ['a' => 'admin.set_account'], ['user' => 'alice', 'enabled' => false]), 'foreign account');
        T::eq(200, $status($env, 'reseller:reseller1', ['a' => 'admin.set_account'], ['user' => 'bob', 'enabled' => false]), 'own account');
        T::eq(404, $status($env, 'reseller:reseller1', ['a' => 'admin.set_domain'], ['user' => 'alice', 'domain' => 'example.com', 'enabled' => false]));
        T::eq(403, $status($env, 'reseller:reseller1', ['a' => 'admin.save_settings'], ['settings' => ['store_ip' => true]]), 'global settings are root only');
        T::eq(403, $status($env, 'reseller:reseller1', ['a' => 'admin.purge'], ['user' => 'bob', 'confirm' => 'DELETE']), 'purge is root only');
        T::eq(403, $status($env, 'reseller:reseller1', ['a' => 'admin.run'], ['type' => 'rollup']), 'rollup is root only');
        T::eq(400, $status($env, 'reseller:reseller1', ['a' => 'admin.run'], ['type' => 'collect']), 'reseller must pick an account');
        T::eq(404, $status($env, 'reseller:reseller1', ['a' => 'admin.run'], ['type' => 'collect', 'user' => 'alice']));
        T::eq(403, $status($env, 'user:alice', ['a' => 'admin.set_account'], ['user' => 'alice', 'enabled' => true]), 'user cannot toggle itself');
        T::eq('forbidden', $code($env, 'user:alice', ['a' => 'admin.purge'], ['user' => 'alice', 'confirm' => 'DELETE']));
        $jobs = SeedDb::api($env, 'reseller:reseller1', ['a' => 'admin.jobs'])['json']['data']['jobs'];
        foreach ($jobs as $j) {
            T::true(in_array(explode('/', (string) $j['scope'])[0], ['all', 'bob', 'carol', 'reseller1'], true), 'reseller jobs scope');
        }
    },
    'Principal' => function () {
        $u = new Principal(Principal::USER, 'alice');
        T::true($u->canSeeUser('alice'));
        T::false($u->canSeeUser('bob'));
        T::false($u->isAdmin());
        $r = new Principal(Principal::RESELLER, 'res', ['bob']);
        T::true($r->canSeeUser('bob'));
        T::true($r->canSeeUser('res'));
        T::false($r->canSeeUser('alice'));
        T::true((new Principal(Principal::ADMIN, 'root'))->canSeeUser('anyone'));
        T::throws(InvalidArgumentException::class, fn () => new Principal('god', 'x'));
    },
];
