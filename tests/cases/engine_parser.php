<?php

declare(strict_types=1);

use WebPulse\Log\LogParser;
use WebPulse\Tests\T;
use WebPulse\Util\Str;

$fixture = static fn (): array => file(dirname(__DIR__) . '/fixtures/logs/edge_cases.log', FILE_IGNORE_NEW_LINES);

return [
    'combined line with UTM query' => static function () use ($fixture): void {
        $h = (new LogParser())->parse($fixture()[0]);
        T::notNull($h);
        T::eq('203.0.113.5', $h->ip);
        T::eq(gmmktime(11, 55, 36, 10, 10, 2026), $h->ts);
        T::eq(7200, $h->offset);
        T::eq('GET', $h->method);
        T::eq('/pricing?utm_source=news&utm_medium=email', $h->target);
        T::eq('HTTP/1.1', $h->protocol);
        T::eq(200, $h->status);
        T::eq(2326, $h->bytes);
        T::eq('https://mail.google.com/', $h->referrer);
        T::contains('Chrome/129', $h->ua);
        T::null($h->vhost);
    },
    'leading vhost with port and IPv6 client' => static function () use ($fixture): void {
        $h = (new LogParser())->parse($fixture()[1]);
        T::eq('example.com:443', $h->vhost);
        T::eq('2001:db8:85a3::8a2e:370:7334', $h->ip);
        T::eq('HTTP/2.0', $h->protocol);
        T::null($h->referrer);
    },
    'leading vhost without port, dash size' => static function () use ($fixture): void {
        $h = (new LogParser())->parse($fixture()[2]);
        T::eq('example.com', $h->vhost);
        T::eq('198.51.100.7', $h->ip);
        T::eq(304, $h->status);
        T::eq(0, $h->bytes);
    },
    'escaped quotes in request and UA, authenticated user' => static function () use ($fixture): void {
        $h = (new LogParser())->parse($fixture()[3]);
        T::notNull($h);
        T::eq('Agent with "quotes" inside', $h->ua);
        T::eq(404, $h->status);
        // the request line contains spaces inside the escaped quotes: not a valid 3-part request
        T::false($h->requestValid());
    },
    'TLS garbage request line is a hit with empty method' => static function () use ($fixture): void {
        $h = (new LogParser())->parse($fixture()[4]);
        T::notNull($h);
        T::eq('', $h->method);
        T::eq(400, $h->status);
        T::eq('', $h->ua);
    },
    'dash request line (408)' => static function () use ($fixture): void {
        $h = (new LogParser())->parse($fixture()[5]);
        T::eq('', $h->method);
        T::eq(408, $h->status);
        T::eq(0, $h->bytes);
    },
    'common log format without referrer and UA' => static function () use ($fixture): void {
        $h = (new LogParser())->parse($fixture()[6]);
        T::notNull($h);
        T::null($h->referrer);
        T::eq('', $h->ua);
    },
    'extra trailing fields are ignored' => static function () use ($fixture): void {
        $h = (new LogParser())->parse($fixture()[7]);
        T::eq('/extra', $h->target);
        T::eq('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', $h->ua);
    },
    'binary escapes are decoded and later cleaned to UTF-8' => static function () use ($fixture): void {
        $h = (new LogParser())->parse($fixture()[9]);
        T::notNull($h);
        T::eq("/bin\xff\xfe", $h->target);
        T::eq('Mozilla/5.0 ?? garbage', Str::clean($h->ua));
        T::eq(1, preg_match('//u', Str::clean((string) $h->referrer)));
    },
    'garbage and impossible dates are rejected' => static function () use ($fixture): void {
        $p = new LogParser();
        $f = $fixture();
        T::null($p->parse($f[10]));
        T::null($p->parse($f[11]), '31 Feb');
        T::null($p->parse($f[12]), 'hour 25');
        T::null($p->parse(''));
        T::null($p->parse('1.2.3.4 - - [10/Oct/2026:13:55:36 +0200] "GET / HTTP/1.1"'));
    },
    'unknown methods and OPTIONS *' => static function () use ($fixture): void {
        $p = new LogParser();
        $f = $fixture();
        T::eq('PROPFIND', $p->parse($f[13])->method);
        $h = $p->parse($f[14]);
        T::eq('OPTIONS', $h->method);
        T::eq('*', $h->target);
        T::eq(-25200, $h->offset);
        T::eq(gmmktime(20, 55, 47, 10, 10, 2026), $h->ts);
    },
    'every fixture line yields a hit or null without warnings' => static function () use ($fixture): void {
        $p = new LogParser();
        $ok = 0;
        foreach ($fixture() as $l) {
            if ($p->parse($l) !== null) {
                $ok++;
            }
        }
        T::eq(12, $ok);
    },
    'HTTP/0.9 style request without protocol' => static function (): void {
        $h = (new LogParser())->parse('1.2.3.4 - - [10/Oct/2026:13:55:36 +0000] "GET /old" 200 5 "-" "x"');
        T::eq('GET', $h->method);
        T::eq('/old', $h->target);
        T::eq('', $h->protocol);
    },
    'very long request line' => static function (): void {
        $path = '/' . str_repeat('a', 20000);
        $h = (new LogParser())->parse('1.2.3.4 - - [10/Oct/2026:13:55:36 +0000] "GET ' . $path . ' HTTP/1.1" 414 0 "-" "Mozilla/5.0"');
        T::notNull($h);
        T::eq(20001, strlen($h->target));
    },
];
