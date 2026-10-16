<?php

declare(strict_types=1);

use WebPulse\Classify\Bots;
use WebPulse\Classify\Path;
use WebPulse\Classify\Referrer;
use WebPulse\Classify\UserAgent;
use WebPulse\Store\Dims;
use WebPulse\Tests\T;
use WebPulse\Util\Ip;
use WebPulse\Util\Str;

$noUtm = ['utm_source' => null, 'utm_medium' => null, 'utm_campaign' => null, 'click_id' => false];

return [
    'user agents: browsers, versions, os, devices' => static function (): void {
        $ua = new UserAgent();
        $cases = [
            ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36', 'Chrome', 'Chrome 129', 'Windows', 'Windows 10/11', 'desktop'],
            ['Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1', 'Safari', 'Safari 17', 'iOS', 'iOS 17', 'mobile'],
            ['Mozilla/5.0 (iPad; CPU OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1', 'Safari', 'Safari 16', 'iOS', 'iOS 16', 'tablet'],
            ['Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.0.0 Mobile Safari/537.36', 'Samsung Internet', 'Samsung Internet 25', 'Android', 'Android 14', 'mobile'],
            ['Mozilla/5.0 (Linux; Android 13; SM-X700) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36', 'Chrome', 'Chrome 128', 'Android', 'Android 13', 'tablet'],
            ['Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Safari/605.1.15', 'Safari', 'Safari 17', 'macOS', 'macOS 10.15+', 'desktop'],
            ['Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0', 'Firefox', 'Firefox 131', 'Linux', 'Linux', 'desktop'],
            ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36 Edg/129.0.2792.79', 'Edge', 'Edge 129', 'Windows', 'Windows 10/11', 'desktop'],
            ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36 OPR/114.0.0.0', 'Opera', 'Opera 114', 'Windows', 'Windows 10/11', 'desktop'],
            ['Mozilla/5.0 (iPhone; CPU iPhone OS 17_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Instagram 345.0.0.31.89', 'In-app', 'In-app Instagram', 'iOS', 'iOS 17', 'mobile'],
            ['Mozilla/5.0 (Linux; Android 13; SM-A536B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Mobile Safari/537.36 [FB_IAB/FB4A;FBAV/479.0.0.48.79;]', 'In-app', 'In-app Facebook', 'Android', 'Android 13', 'mobile'],
            ['Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/129.0.6668.69 Mobile/15E148 Safari/604.1', 'Chrome', 'Chrome 129', 'iOS', 'iOS 17', 'mobile'],
            ['Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36', 'Chrome', 'Chrome 129', 'ChromeOS', 'ChromeOS', 'desktop'],
            ['Mozilla/5.0 (compatible; MSIE 9.0; Windows NT 6.1; Trident/5.0)', 'IE', 'IE 9', 'Windows', 'Windows 7', 'desktop'],
            ['Mozilla/5.0 (Windows NT 6.1; Trident/7.0; rv:11.0) like Gecko', 'IE', 'IE 11', 'Windows', 'Windows 7', 'desktop'],
            ['Mozilla/5.0 (Linux; Android 10; CUBOT X30) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Mobile Safari/537.36', 'Chrome', 'Chrome 120', 'Android', 'Android 10', 'mobile'],
            ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 YaBrowser/24.7.0.0 Safari/537.36', 'Yandex Browser', 'Yandex Browser 24', 'Windows', 'Windows 10/11', 'desktop'],
        ];
        foreach ($cases as [$s, $b, $bv, $os, $osv, $dev]) {
            $r = $ua->parse($s);
            T::eq([$b, $bv, $os, $osv, $dev, null], [$r['browser'], $r['browser_version'], $r['os'], $r['os_version'], $r['device'], $r['bot']], $s);
        }
    },
    'bots: named families, AI crawlers, scripts, empty UA' => static function (): void {
        $b = new Bots();
        $cases = [
            'Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)' => 'Googlebot',
            'Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; GPTBot/1.2; +https://openai.com/gptbot)' => 'GPTBot',
            'Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; ClaudeBot/1.0; +claudebot@anthropic.com)' => 'ClaudeBot',
            'Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)' => 'bingbot',
            'CCBot/2.0 (https://commoncrawl.org/faq/)' => 'CCBot',
            'Mozilla/5.0 (compatible; AhrefsBot/7.0; +http://ahrefs.com/robot/)' => 'AhrefsBot',
            'facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)' => 'facebookexternalhit',
            'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/120.0.0.0 Safari/537.36' => 'HeadlessChrome',
            'Mozilla/5.0 (Linux; Android 11; moto g power (2022)) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/109.0.0.0 Mobile Safari/537.36 Chrome-Lighthouse' => 'Lighthouse',
            'curl/8.4.0' => 'curl',
            'python-requests/2.31.0' => 'python-requests',
            'Go-http-client/1.1' => 'Go-http-client',
            'Wget/1.21.4' => 'Wget',
            'Mozilla/5.0+(compatible; UptimeRobot/2.0; http://www.uptimerobot.com/)' => 'UptimeRobot',
            'Mozilla/5.0 (compatible; MyCustomCrawler/1.0)' => 'Other bot',
            'SomeTool/1.0' => 'Other bot',
            '' => 'Empty user agent',
            '-' => 'Empty user agent',
        ];
        foreach ($cases as $ua => $name) {
            T::eq($name, $b->match((string) $ua), (string) $ua);
        }
        T::null($b->match('Mozilla/5.0 (Linux; Android 10; CUBOT X30) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Mobile Safari/537.36'));
        T::eq('bot', (new UserAgent())->parse('curl/8.4.0')['device']);
    },
    'referrer: channels and search engines' => static function () use ($noUtm): void {
        $r = new Referrer();
        $cases = [
            [null, 'direct', null, null, null],
            ['-', 'direct', null, null, null],
            ['https://www.example.com/x', 'direct', null, null, null],
            ['https://example.com/', 'direct', null, null, null],
            ['https://www.google.com/', 'organic', 'google.com', 'google', null],
            ['https://www.google.co.uk/search?q=Foo+Bar', 'organic', 'google.co.uk', 'google', 'foo bar'],
            ['https://www.bing.com/search?q=web+stats', 'organic', 'bing.com', 'bing', 'web stats'],
            ['https://duckduckgo.com/', 'organic', 'duckduckgo.com', 'duckduckgo', null],
            ['https://yandex.ru/search/?text=%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82', 'organic', 'yandex.ru', 'yandex', 'привет'],
            ['https://www.baidu.com/s?wd=test', 'organic', 'baidu.com', 'baidu', 'test'],
            ['https://search.brave.com/search?q=x', 'organic', 'search.brave.com', 'brave', 'x'],
            ['android-app://com.google.android.googlequicksearchbox/', 'organic', 'com.google.android.googlequicksearchbox', 'google', null],
            ['https://l.facebook.com/l.php?u=x', 'social', 'l.facebook.com', null, null],
            ['https://t.co/abc', 'social', 't.co', null, null],
            ['https://www.linkedin.com/feed/', 'social', 'linkedin.com', null, null],
            ['https://mail.google.com/mail/u/0/', 'email', 'mail.google.com', null, null],
            ['https://outlook.live.com/', 'email', 'outlook.live.com', null, null],
            ['https://blog.other.org/post?x=1', 'referral', 'blog.other.org', null, null],
            ['https://shop.example.com/', 'referral', 'shop.example.com', null, null],
        ];
        foreach ($cases as [$ref, $ch, $host, $se, $kw]) {
            $c = $r->classify($ref, 'example.com', $noUtm);
            T::eq([$ch, $host, $se, $kw], [$c['channel'], $c['referrer_host'], $c['search_engine'], $c['keyword']], (string) $ref);
        }
        T::eq('blog.other.org/post', $r->classify('https://blog.other.org/post?x=1', 'example.com', $noUtm)['referrer_url']);
        T::null($r->classify('https://www.google.com/', 'example.com', $noUtm)['referrer_url']);
        T::null($r->externalHost('https://www.example.com/a', 'example.com'));
        T::eq('example.com', $r->anyHost('https://www.example.com/a'));
    },
    'referrer: UTM and click ids take precedence' => static function (): void {
        $r = new Referrer();
        $u = static fn (?string $s, ?string $m, ?string $c = null, bool $click = false): array => ['utm_source' => $s, 'utm_medium' => $m, 'utm_campaign' => $c, 'click_id' => $click];
        T::eq('paid', $r->classify('https://www.google.com/', 'example.com', $u('google', 'cpc'))['channel']);
        T::eq('paid', $r->classify('https://www.google.com/', 'example.com', $u(null, null, null, true))['channel']);
        T::eq('email', $r->classify(null, 'example.com', $u('news', 'email', 'oct'))['channel']);
        T::eq('campaign', $r->classify('https://www.google.com/', 'example.com', $u('partner', 'banner-free', 'launch'))['channel']);
        T::eq('social', $r->classify(null, 'example.com', $u('fb', 'social'))['channel']);
        T::eq('organic', $r->classify('https://www.google.com/', 'example.com', $u('google', 'organic'))['channel']);
    },
    'path normalisation' => static function (): void {
        $p = new Path();
        $a = $p->analyze('/a//b/index.php?utm_source=fb&utm_medium=cpc&utm_campaign=x&x=1');
        T::eq(['/a/b/', 'fb', 'cpc', 'x'], [$a['path'], $a['utm_source'], $a['utm_medium'], $a['utm_campaign']]);
        T::eq('/caf' . "\u{e9}" . '/', $p->analyze('/caf%C3%A9/')['path']);
        T::eq('/??.pdf', $p->analyze('/%ff%fe.pdf')['path']);
        T::eq('/', $p->analyze('/index.html')['path']);
        T::eq('/blog/', $p->analyze('/blog/INDEX.HTM')['path']);
        T::eq('/x', $p->analyze('/x#frag')['path']);
        $abs = $p->analyze('http://example.com/foo?gclid=1');
        T::eq(['/foo', 'http', true], [$abs['path'], $abs['scheme'], $abs['click_id']]);
        T::eq(Path::INVALID, $p->analyze('')['path']);
        T::eq('/a?b', $p->analyze('/a%3Fb')['path'], 'encoded ? is part of the path');
        $long = $p->analyze('/' . str_repeat("\u{e9}", 400));
        T::true(strlen($long['path']) <= 512);
        T::eq(1, preg_match('//u', $long['path']));
        T::true(strlen($long['key']) <= 255);
        T::true(str_ends_with($long['key'], "\u{2026}"));
        T::eq('/a?', $p->analyze('/a%00')['path'], 'NUL replaced');
    },
    'path file types, static and downloads' => static function (): void {
        $p = new Path(['/wp-content/uploads/', '/wp-includes/', '/wp-json/']);
        $t = static fn (string $x): array => [$p->analyze($x)['file_type'], $p->analyze($x)['is_static'], $p->analyze($x)['is_download']];
        T::eq(['html', false, false], $t('/'));
        T::eq(['html', false, false], $t('/page.php'));
        T::eq(['html', false, false], $t('/about.htm'));
        T::eq(['jpg', true, false], $t('/x.JPG'));
        T::eq(['css', true, false], $t('/a.css?v=3'));
        T::eq(['pdf', true, true], $t('/wp-content/uploads/a.pdf'));
        T::eq(['gz', false, true], $t('/doc.tar.gz'));
        T::eq(['html', true, false], $t('/wp-json/wp/v2/posts'));
        T::eq(['html', false, false], $t('/a.verylongextension'));
        T::eq(['woff2', true, false], $t('/f.woff2'));
        T::eq(['zip', false, true], $t('/app-1.2.zip'));
    },
    'dims list and buckets' => static function (): void {
        T::eq(32, count(Dims::ALL));
        T::eq(32, count(array_unique(Dims::ALL)));
        foreach (Dims::VISIT_LEVEL as $d) {
            T::true(in_array($d, Dims::ALL, true), $d);
        }
        T::eq(['0-10s', '0-10s', '11-30s', '31-60s', '1-2m', '2-5m', '5-10m', '10-30m', '30m+'],
            array_map([Dims::class, 'durationBucket'], [0, 10, 11, 60, 61, 300, 600, 1800, 1801]));
        T::eq(['1', '2', '5', '6-10', '10', '11-20', '21+'],
            array_map(static fn ($n) => $n === 10 ? '10' : Dims::pagesBucket($n), [1, 2, 5, 6, 10, 20, 21]));
    },
    'utf-8 cleaning and truncation' => static function (): void {
        T::eq('a?b', Str::clean("a\xffb"));
        T::eq('a?b', Str::scrub("a\xffb"));
        T::eq("\u{e9}?", Str::scrub("\xc3\xa9\xc3"));
        T::eq('a?b', Str::clean("a\x01b"));
        T::eq('ab', Str::truncate("ab\u{e9}", 3));
        T::eq("a\u{2026}", Str::truncate('abcdef', 4, "\u{2026}"));
        T::eq(1, preg_match('//u', Str::key(str_repeat("\u{20ac}", 200))));
        T::true(strlen(Str::key(str_repeat("\u{20ac}", 200))) <= 255);
        T::eq('GeoipUpdate', Str::studly('geoip-update'));
    },
    'ip / cidr matching (v4, v6, mapped)' => static function (): void {
        $c = Ip::compile(['203.0.113.0/24', '2001:db8::/32', '198.51.100.7', 'bogus', '10.0.0.0/33']);
        T::eq(3, count($c));
        T::true(Ip::matches('203.0.113.99', $c));
        T::false(Ip::matches('203.0.114.1', $c));
        T::true(Ip::matches('2001:db8:1::5', $c));
        T::false(Ip::matches('2001:db9::1', $c));
        T::true(Ip::matches('::ffff:198.51.100.7', $c));
        T::false(Ip::matches('not-an-ip', $c));
        T::true(Ip::isValidCidr('::/0'));
        T::false(Ip::isValidCidr('1.2.3.4/'));
    },
];
