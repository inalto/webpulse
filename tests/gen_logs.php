<?php

declare(strict_types=1);

/*
 * Deterministic generator of realistic Apache "combined" access logs for the
 * fixture domains (example.com + shop.example.com of alice, blog.net of bob).
 *
 *   php tests/gen_logs.php --seed 42 --days 45 --lines 60000 --out <fakeroot>
 *        [--end YYYY-MM-DD] [--tz Europe/Rome] [--no-archive] [--single FILE] [--max-ts UNIX]
 *
 * Features: diurnal curve, weekend dip, ~30 % bots (search, SEO, AI crawlers,
 * scripts, scanners), search/social/referral/email/paid/UTM acquisition,
 * desktop/mobile/tablet/in-app user agents, IPv6 clients, static assets,
 * downloads, 404s with referrers, a few 5xx, http->https redirects, HEAD
 * monitoring, ACME challenges, malformed lines, an SSL log per domain, and
 * gzip archives (/home/alice/logs/example.com[-ssl_log]-Mon-YYYY.gz) for the
 * months before the end month.
 *
 * Prints a JSON summary (line counts per kind and per file) on stdout.
 * --single FILE generates example.com traffic only and writes every line to
 * FILE (used by the performance test).
 */

ini_set('memory_limit', '-1');

$o = getopt('', ['seed:', 'days:', 'lines:', 'out:', 'end:', 'tz:', 'no-archive', 'single:', 'max-ts:']);
$seed = (int) ($o['seed'] ?? 42);
$days = max(1, (int) ($o['days'] ?? 45));
$target = max(10, (int) ($o['lines'] ?? 60000));
$out = $o['out'] ?? null;
$tz = new DateTimeZone($o['tz'] ?? 'Europe/Rome');
$endDay = $o['end'] ?? date('Y-m-d');
$archive = !isset($o['no-archive']);
$single = $o['single'] ?? null;
if ($out === null && $single === null) {
    fwrite(STDERR, "usage: gen_logs.php --out <fakeroot> [--seed N] [--days N] [--lines N] [--end YYYY-MM-DD]\n");
    exit(2);
}
mt_srand($seed);

function pick(array $weighted): mixed
{
    static $sums = [];
    $id = count($weighted) . ':' . md5(serialize(array_column($weighted, 1)));
    $sum = $sums[$id] ??= array_sum(array_column($weighted, 1));
    $r = mt_rand(1, $sum);
    foreach ($weighted as [$v, $w]) {
        $r -= $w;
        if ($r <= 0) {
            return $v;
        }
    }
    return $weighted[0][0];
}

function one(array $list): mixed
{
    return $list[mt_rand(0, count($list) - 1)];
}

$endTs = (new DateTimeImmutable($endDay . ' 23:59:59', $tz))->getTimestamp();
if (isset($o['max-ts'])) {
    $endTs = min($endTs, (int) $o['max-ts'] + 3600);
}
$startTs = (new DateTimeImmutable($endDay . ' 00:00:00', $tz))->modify('-' . ($days - 1) . ' days')->getTimestamp();

$sites = [
    'example.com' => ['user' => 'alice', 'weight' => 65,
        'pages' => array_merge(['/', '/', '/', '/pricing', '/features', '/about', '/contact', '/blog/', '/docs/getting-started', '/docs/api', '/index.php'],
            array_map(static fn ($i) => "/blog/post-$i", range(1, 20))),
        'downloads' => ['/files/brochure.pdf', '/files/whitepaper.pdf', '/downloads/app-1.2.zip'],
        'assets' => ['/assets/app.css', '/assets/app.js', '/img/logo.svg', '/img/hero.webp', '/favicon.ico', '/wp-content/uploads/2026/09/photo.jpg', '/fonts/inter.woff2']],
    'shop.example.com' => ['user' => 'alice', 'weight' => 20,
        'pages' => array_merge(['/', '/', '/cart', '/category/shoes', '/category/bags', '/checkout'], array_map(static fn ($i) => "/products/item-$i", range(1, 30))),
        'downloads' => ['/files/size-guide.pdf'],
        'assets' => ['/static/shop.css', '/static/shop.js', '/static/img/p1.jpg', '/static/img/p2.jpg', '/favicon.ico']],
    'blog.net' => ['user' => 'bob', 'weight' => 15,
        'pages' => array_merge(['/', '/', '/about/', '/feed/', '/tag/php/', '/tag/linux/'], array_map(static fn ($i) => "/2026/09/post-slug-$i/", range(1, 25))),
        'downloads' => ['/wp-content/uploads/2026/09/slides.pdf'],
        'assets' => ['/wp-content/themes/t/style.css', '/wp-includes/js/jquery.min.js', '/wp-content/uploads/2026/09/cover.png', '/favicon.ico']],
];

$uas = [
    ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36', 30],
    ['Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1', 20],
    ['Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Mobile Safari/537.36', 15],
    ['Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15', 8],
    ['Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0', 3],
    ['Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:131.0) Gecko/20100101 Firefox/131.0', 3],
    ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36 Edg/129.0.2792.79', 8],
    ['Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.0.0 Mobile Safari/537.36', 4],
    ['Mozilla/5.0 (iPad; CPU OS 17_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Mobile/15E148 Safari/604.1', 3],
    ['Mozilla/5.0 (iPhone; CPU iPhone OS 17_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Instagram 345.0.0.31.89', 3],
    ['Mozilla/5.0 (Linux; Android 13; SM-A536B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.6613.127 Mobile Safari/537.36 [FB_IAB/FB4A;FBAV/479.0.0.48.79;]', 2],
    ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36 OPR/114.0.0.0', 1],
    ['Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36', 1],
];

$bots = [
    ['Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)', 25],
    ['Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)', 10],
    ['Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; GPTBot/1.2; +https://openai.com/gptbot)', 8],
    ['Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; ClaudeBot/1.0; +claudebot@anthropic.com)', 6],
    ['Mozilla/5.0 (compatible; AhrefsBot/7.0; +http://ahrefs.com/robot/)', 10],
    ['Mozilla/5.0 (compatible; SemrushBot/7~bl; +http://www.semrush.com/bot.html)', 8],
    ['Mozilla/5.0 (compatible; YandexBot/3.0; +http://yandex.com/bots)', 4],
    ['Mozilla/5.0 (Linux; Android 7.0;) AppleWebKit/537.36 (KHTML, like Gecko) Mobile Safari/537.36 (compatible; PetalBot;+https://webmaster.petalsearch.com/site/petalbot)', 4],
    ['Mozilla/5.0 (Linux; Android 5.0) AppleWebKit/537.36 (KHTML, like Gecko) Mobile Safari/537.36 (compatible; Bytespider; spider-feedback@bytedance.com)', 4],
    ['CCBot/2.0 (https://commoncrawl.org/faq/)', 3],
    ['Mozilla/5.0 (compatible; Amazonbot/0.1; +https://developer.amazon.com/support/amazonbot)', 2],
    ['facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)', 3],
    ['curl/8.4.0', 3],
    ['python-requests/2.31.0', 3],
    ['Go-http-client/1.1', 2],
    ['-', 3],
];

$refs = [
    [['direct', null], 30],
    [['organic', 'https://www.google.com/'], 26],
    [['organic', 'https://www.google.it/'], 4],
    [['organic', 'https://www.bing.com/search?q=web+analytics+cpanel'], 5],
    [['organic', 'https://duckduckgo.com/'], 3],
    [['organic', 'https://www.ecosia.org/'], 1],
    [['social', 'https://l.facebook.com/'], 4],
    [['social', 'https://t.co/AbCdEf'], 2],
    [['social', 'https://l.instagram.com/'], 2],
    [['social', 'https://www.reddit.com/r/webdev/comments/abc/'], 1],
    [['referral', 'https://news.ycombinator.example.org/item?id=1'], 2],
    [['referral', 'https://blog.partner.org/best-tools/'], 4],
    [['referral', 'https://forum.example.net/thread/42'], 2],
    [['email', 'https://mail.google.com/'], 2],
    [['utm_email', null], 3],
    [['paid', null], 4],
    [['campaign', null], 3],
];

$countries = [['81.2.69.%d', 6], ['151.%d.10.20', 18], ['93.%d.40.1', 8], ['2.%d.120.7', 8], ['8.8.%d.4', 6], ['203.0.113.%d', 10], ['198.51.100.%d', 10], ['185.%d.12.9', 10], ['5.%d.66.3', 8], ['31.%d.200.1', 6]];
$v6prefix = ['2001:db8:%x::%x', '2a01:4f8:%x::%x', '2606:4700:%x::%x'];

$lines = [];   // [ts, fileKey, line]
$count = 0;
$sum = ['lines' => 0, 'human' => 0, 'bot' => 0, 'head' => 0, 'acme' => 0, 'malformed' => 0, 'tls' => 0, 'pageviews_approx' => 0, 'files' => []];

$fmt = static function (int $ts) use ($tz): string {
    static $cache = [];
    $h = intdiv($ts, 900);
    if (!isset($cache[$h])) {
        $off = $tz->getOffset(new DateTimeImmutable('@' . $ts));
        $cache[$h] = [$off, sprintf('%s%02d%02d', $off < 0 ? '-' : '+', intdiv(abs($off), 3600), intdiv(abs($off) % 3600, 60))];
    }
    [$off, $z] = $cache[$h];
    return gmdate('d/M/Y:H:i:s', $ts + $off) . ' ' . $z;
};
$emit = static function (int $ts, string $domain, bool $ssl, string $ip, string $req, int $status, int $bytes, ?string $ref, string $ua, string $kind) use (&$lines, &$count, &$sum, $fmt): void {
    $line = sprintf('%s - - [%s] "%s" %d %s "%s" "%s"', $ip, $fmt($ts), $req, $status, $bytes === 0 ? '-' : (string) $bytes, $ref ?? '-', $ua);
    $lines[] = [$ts, $domain . ($ssl ? '-ssl_log' : ''), $line];
    $count++;
    $sum[$kind]++;
};

$dayTs = static function (int $i) use ($startTs, $tz): int {
    return (new DateTimeImmutable('@' . $startTs))->setTimezone($tz)->modify("+$i days")->getTimestamp();
};
// hour weights: diurnal curve
$hourW = [1, 1, 1, 1, 1, 1, 2, 3, 5, 7, 8, 8, 7, 7, 8, 8, 8, 7, 7, 8, 9, 8, 5, 2];
$dayList = [];
for ($i = 0; $i < $days; $i++) {
    $t = $dayTs($i);
    $wd = (int) (new DateTimeImmutable('@' . $t))->setTimezone($tz)->format('N');
    $dayList[] = [$t, $wd >= 6 ? 6 : 10];
}
$dayPick = array_map(static fn ($x) => [$x[0], $x[1]], $dayList);
$hourPick = array_map(static fn ($hh) => [$hh, $hourW[$hh]], range(0, 23));
$randomTs = static function () use ($dayPick, $hourPick, $endTs): int {
    $d = pick($dayPick);
    $h = pick($hourPick);
    return min($endTs - 3600, $d + $h * 3600 + mt_rand(0, 3599));
};

$visitors = [];
$newVisitor = static function () use ($countries, $v6prefix, $uas): array {
    if (mt_rand(1, 100) <= 12) {
        $ip = sprintf(one($v6prefix), mt_rand(1, 0xffff), mt_rand(1, 0xffff));
    } else {
        $ip = sprintf(pick($countries), mt_rand(1, 254));
    }
    return ['ip' => $ip, 'ua' => pick($uas)];
};

$siteW = array_map(static fn ($k, $s) => [$k, $s['weight']], array_keys($sites), $sites);
if ($single !== null) {
    $siteW = [['example.com', 1]];
}
while ($count < $target) {
    $r = mt_rand(1, 1000);
    if ($r <= 540) {
        // bot burst
        $domain = pick($siteW);
        $s = $sites[$domain];
        $ua = pick($bots);
        $ts = $randomTs();
        $ip = sprintf('66.249.%d.%d', mt_rand(64, 95), mt_rand(1, 254));
        $n = mt_rand(1, 6);
        for ($i = 0; $i < $n && $count < $target; $i++) {
            $path = mt_rand(1, 10) === 1 ? one(['/robots.txt', '/sitemap.xml', '/wp-login.php', '/.env', '/xmlrpc.php']) : one($s['pages']);
            $status = in_array($path, ['/wp-login.php', '/.env', '/xmlrpc.php'], true) ? 404 : 200;
            $emit($ts + $i * mt_rand(1, 20), $domain, mt_rand(1, 10) > 2, $ip, "GET $path HTTP/1.1", $status, mt_rand(300, 40000), null, $ua, 'bot');
        }
        continue;
    }
    if ($r <= 550) {
        $domain = pick($siteW);
        $emit($randomTs(), $domain, true, '216.144.250.150', 'HEAD / HTTP/1.1', 200, 0, null, 'Mozilla/5.0+(compatible; UptimeRobot/2.0; http://www.uptimerobot.com/)', 'head');
        continue;
    }
    if ($r <= 553) {
        $domain = pick($siteW);
        $emit($randomTs(), $domain, false, '3.3.3.' . mt_rand(1, 9), 'GET /.well-known/acme-challenge/' . bin2hex(pack('N', mt_rand())) . ' HTTP/1.1', 200, 87, null, 'Mozilla/5.0 (compatible; Let\'s Encrypt validation server; +https://www.letsencrypt.org)', 'acme');
        continue;
    }
    if ($r <= 558) {
        $domain = pick($siteW);
        $ts = $randomTs();
        $kind = mt_rand(1, 3);
        if ($kind === 1) {
            $line = sprintf('45.%d.1.1 - - [%s] "\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03" 400 226 "-" "-"', mt_rand(1, 200), $fmt($ts));
            $sum['tls']++;
        } elseif ($kind === 2) {
            $line = 'garbage ' . bin2hex(pack('N', mt_rand())) . ' not a log line';
            $sum['malformed']++;
        } else {
            $line = sprintf('10.0.0.%d - - [%s] "GET /truncated', mt_rand(1, 200), $fmt($ts));
            $sum['malformed']++;
        }
        $lines[] = [$ts, $domain . '-ssl_log', $line];
        $count++;
        continue;
    }
    // human visit
    $domain = pick($siteW);
    $s = $sites[$domain];
    if ($visitors !== [] && mt_rand(1, 100) <= 25) {
        $vis = one($visitors);
    } else {
        $vis = $newVisitor();
        if (count($visitors) < 5000) {
            $visitors[] = $vis;
        }
    }
    [$channel, $ref] = pick($refs);
    $ts = $randomTs();
    $q = '';
    if ($channel === 'utm_email') {
        $q = '?utm_source=newsletter&utm_medium=email&utm_campaign=october';
    } elseif ($channel === 'paid') {
        $q = mt_rand(0, 1) ? '?gclid=' . bin2hex(pack('N', mt_rand())) : '?utm_source=google&utm_medium=cpc&utm_campaign=brand';
    } elseif ($channel === 'campaign') {
        $q = '?utm_source=partner&utm_medium=banner-free&utm_campaign=launch';
    }
    $n = pick([[1, 45], [2, 15], [3, 15], [mt_rand(4, 8), 20], [mt_rand(9, 20), 5]]);
    if (mt_rand(1, 10) === 1) {
        // plain http first: redirect to https
        $emit($ts, $domain, false, $vis['ip'], 'GET ' . $s['pages'][0] . ' HTTP/1.1', 301, 230, $ref, $vis['ua'], 'human');
        $ts += 1;
    }
    $prev = $ref;
    for ($i = 0; $i < $n && $count < $target; $i++) {
        $roll = mt_rand(1, 1000);
        if ($i === 0) {
            $path = mt_rand(1, 3) === 1 ? one($s['pages']) : $s['pages'][0];
            $path .= $q;
        } else {
            $path = one($s['pages']);
        }
        $method = $path === '/contact' && mt_rand(1, 4) === 1 ? 'POST' : 'GET';
        if ($roll <= 30) {
            $emit($ts, $domain, true, $vis['ip'], 'GET /old-page-' . mt_rand(1, 5) . ' HTTP/1.1', 404, 1200, $prev ?? 'https://www.bing.com/', $vis['ua'], 'human');
        } elseif ($roll <= 33) {
            $emit($ts, $domain, true, $vis['ip'], "$method $path HTTP/2.0", one([500, 502, 503]), 400, $prev, $vis['ua'], 'human');
        } elseif ($roll <= 55) {
            $emit($ts, $domain, true, $vis['ip'], 'GET ' . one($s['downloads']) . ' HTTP/2.0', 200, mt_rand(100000, 5000000), $prev, $vis['ua'], 'human');
        } else {
            $emit($ts, $domain, true, $vis['ip'], "$method $path HTTP/2.0", mt_rand(1, 20) === 1 ? 304 : 200, mt_rand(4000, 60000), $prev, $vis['ua'], 'human');
            $sum['pageviews_approx']++;
            $page = 'https://' . ($domain === 'blog.net' ? 'www.' : '') . $domain . $path;
            $na = mt_rand(1, 4);
            for ($k = 0; $k < $na && $count < $target; $k++) {
                $emit($ts + mt_rand(0, 2), $domain, true, $vis['ip'], 'GET ' . one($s['assets']) . ' HTTP/2.0', mt_rand(1, 4) === 1 ? 304 : 200, mt_rand(500, 90000), $page, $vis['ua'], 'human');
            }
            $prev = $page;
        }
        $ts += pick([[mt_rand(3, 30), 40], [mt_rand(30, 180), 45], [mt_rand(180, 900), 15]]);
    }
}

usort($lines, static fn (array $a, array $b): int => $a[0] <=> $b[0]);

// write: lines before the end month of example.com go to archives
$endMonth = (new DateTimeImmutable($endDay, $tz))->format('Y-m');
$handles = [];
$open = static function (string $file, bool $gz) use (&$handles) {
    if (!isset($handles[$file])) {
        @mkdir(dirname($file), 0755, true);
        $handles[$file] = $gz ? gzopen($file, 'wb6') : fopen($file, 'wb');
    }
    return $handles[$file];
};
$domainUser = array_map(static fn ($s) => $s['user'], $sites);
foreach ($lines as [$ts, $key, $line]) {
    $domain = preg_replace('/-ssl_log$/', '', $key);
    $user = $domainUser[$domain];
    if ($single !== null) {
        if ($domain !== 'example.com') {
            continue;
        }
        $file = $single;
        $gz = false;
    } elseif ($archive && $domain === 'example.com' && ($local = (new DateTimeImmutable('@' . $ts))->setTimezone($tz))->format('Y-m') < $endMonth) {
        $file = "$out/home/$user/logs/$key-" . $local->format('M-Y') . '.gz';
        $gz = true;
    } else {
        $file = "$out/etc/apache2/logs/domlogs/$user/$key";
        $gz = false;
    }
    $h = $open($file, $gz);
    if ($gz) {
        gzwrite($h, $line . "\n");
    } else {
        fwrite($h, $line . "\n");
    }
    $rel = $out !== null && str_starts_with($file, $out . '/') ? substr($file, strlen($out) + 1) : $file;
    $sum['files'][$rel] = ($sum['files'][$rel] ?? 0) + 1;
    $sum['lines']++;
}
foreach ($handles as $f => $h) {
    str_ends_with($f, '.gz') ? gzclose($h) : fclose($h);
}
ksort($sum['files']);
$sum['seed'] = $seed;
$sum['start'] = date('c', $startTs);
$sum['end'] = date('c', $endTs);
echo json_encode($sum, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES), "\n";
