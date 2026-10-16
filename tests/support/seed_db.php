<?php

declare(strict_types=1);

namespace WebPulse\Tests;

use PDO;

/**
 * Fast, deterministic fixture for the reports/API tests (WP2): builds domain
 * databases straight from the DDL in docs/DATA-MODEL.md with ~45 days of
 * plausible, internally consistent data, plus a registry database and a fake
 * server root (config.json, /var/cpanel/users, /etc/userdomains).
 *
 * Kept even with the real engine available: it is ~50x faster than
 * generating and collecting logs and its totals are known exactly.
 */
final class SeedDb
{
    public const DOMAIN_DDL = <<<'SQL'
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE daily (day TEXT PRIMARY KEY, visits INTEGER NOT NULL DEFAULT 0, visitors INTEGER NOT NULL DEFAULT 0,
  pageviews INTEGER NOT NULL DEFAULT 0, hits INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0,
  bounces INTEGER NOT NULL DEFAULT 0, duration INTEGER NOT NULL DEFAULT 0, downloads INTEGER NOT NULL DEFAULT 0,
  errors_4xx INTEGER NOT NULL DEFAULT 0, errors_5xx INTEGER NOT NULL DEFAULT 0, bot_hits INTEGER NOT NULL DEFAULT 0,
  bot_bytes INTEGER NOT NULL DEFAULT 0);
CREATE TABLE hourly (day TEXT NOT NULL, hour INTEGER NOT NULL, visits INTEGER NOT NULL DEFAULT 0,
  pageviews INTEGER NOT NULL DEFAULT 0, hits INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0,
  bot_hits INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (day, hour));
CREATE TABLE dim_daily (day TEXT NOT NULL, dim TEXT NOT NULL, key TEXT NOT NULL, visits INTEGER NOT NULL DEFAULT 0,
  pageviews INTEGER NOT NULL DEFAULT 0, hits INTEGER NOT NULL DEFAULT 0, bytes INTEGER NOT NULL DEFAULT 0,
  bounces INTEGER NOT NULL DEFAULT 0, duration INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (day, dim, key)) WITHOUT ROWID;
CREATE INDEX dim_daily_dim_day ON dim_daily (dim, day);
CREATE TABLE open_visits (visitor TEXT PRIMARY KEY, visit_id INTEGER NOT NULL, day TEXT NOT NULL, start_ts INTEGER NOT NULL,
  last_ts INTEGER NOT NULL, last_pv_ts INTEGER NOT NULL, pageviews INTEGER NOT NULL, hits INTEGER NOT NULL,
  bytes INTEGER NOT NULL, entry_page TEXT, exit_page TEXT, pages_seen TEXT NOT NULL, attrs TEXT NOT NULL);
CREATE INDEX open_visits_last ON open_visits (last_ts);
CREATE TABLE visits (visit_id INTEGER PRIMARY KEY, visitor TEXT NOT NULL, day TEXT NOT NULL, start_ts INTEGER NOT NULL,
  end_ts INTEGER NOT NULL, duration INTEGER NOT NULL, pageviews INTEGER NOT NULL, hits INTEGER NOT NULL, bytes INTEGER NOT NULL,
  entry_page TEXT, exit_page TEXT, channel TEXT, referrer_host TEXT, search_engine TEXT, keyword TEXT, campaign TEXT,
  country TEXT, city TEXT, browser TEXT, browser_version TEXT, os TEXT, os_version TEXT, device TEXT, ip TEXT);
CREATE INDEX visits_day ON visits (day, start_ts);
CREATE TABLE events (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, visit_id INTEGER, type TEXT NOT NULL, path TEXT NOT NULL,
  status INTEGER NOT NULL, bytes INTEGER NOT NULL, referrer TEXT);
CREATE INDEX events_ts ON events (ts);
CREATE INDEX events_visit ON events (visit_id);
CREATE TABLE visitor_day (day TEXT NOT NULL, visitor TEXT NOT NULL, PRIMARY KEY (day, visitor)) WITHOUT ROWID;
CREATE TABLE checkpoints (path TEXT PRIMARY KEY, inode INTEGER NOT NULL, offset INTEGER NOT NULL, size INTEGER NOT NULL,
  first_line TEXT, last_ts INTEGER, lines INTEGER NOT NULL DEFAULT 0, bad_lines INTEGER NOT NULL DEFAULT 0,
  ignored_lines INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL);
CREATE TABLE imported_archives (path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL, lines INTEGER NOT NULL,
  imported_at INTEGER NOT NULL);
SQL;

    public const REGISTRY_DDL = <<<'SQL'
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE accounts (user TEXT PRIMARY KEY, owner TEXT, enabled INTEGER, timezone TEXT, first_seen INTEGER NOT NULL,
  last_seen INTEGER NOT NULL, removed_at INTEGER);
CREATE TABLE domains (user TEXT NOT NULL, domain TEXT NOT NULL, enabled INTEGER, kind TEXT, first_seen INTEGER NOT NULL,
  last_seen INTEGER NOT NULL, removed_at INTEGER, PRIMARY KEY (user, domain));
CREATE TABLE domain_summary (user TEXT NOT NULL, domain TEXT NOT NULL, db_bytes INTEGER NOT NULL DEFAULT 0,
  last_collect_at INTEGER, last_line_ts INTEGER, lines_total INTEGER NOT NULL DEFAULT 0, visits_today INTEGER NOT NULL DEFAULT 0,
  pageviews_today INTEGER NOT NULL DEFAULT 0, visits_7d INTEGER NOT NULL DEFAULT 0, pageviews_7d INTEGER NOT NULL DEFAULT 0,
  bytes_7d INTEGER NOT NULL DEFAULT 0, visits_30d INTEGER NOT NULL DEFAULT 0, pageviews_30d INTEGER NOT NULL DEFAULT 0,
  bytes_30d INTEGER NOT NULL DEFAULT 0, bot_hits_30d INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (user, domain));
CREATE TABLE salts (day TEXT PRIMARY KEY, salt BLOB NOT NULL);
CREATE TABLE jobs (id INTEGER PRIMARY KEY, type TEXT NOT NULL, scope TEXT, started_at INTEGER NOT NULL, finished_at INTEGER,
  status TEXT NOT NULL, lines INTEGER NOT NULL DEFAULT 0, files INTEGER NOT NULL DEFAULT 0, domains INTEGER NOT NULL DEFAULT 0,
  message TEXT);
SQL;

    /** Weighted key lists per dim (weights are relative). */
    private const KEYS = [
        'page' => ['/' => 30, '/pricing' => 12, '/blog/' => 9, '/blog/log-analytics-without-cookies' => 7, '/contact' => 6,
            '/about' => 5, '/docs/getting-started' => 5, '/blog/core-web-vitals-2026' => 4, '/features' => 4,
            '/shop/espresso-cups' => 3, '/careers' => 2, '/privacy' => 2, '/docs/api' => 2, '/search' => 1],
        'channel' => ['direct' => 34, 'organic' => 38, 'social' => 9, 'referral' => 11, 'paid' => 3, 'email' => 3, 'campaign' => 2],
        'referrer_host' => ['google.com' => 50, 'bing.com' => 8, 'duckduckgo.com' => 6, 'news.ycombinator.com' => 6,
            'facebook.com' => 5, 'linkedin.com' => 4, 'reddit.com' => 4, 'github.com' => 3, 't.co' => 3, 'ecosia.org' => 2],
        'referrer_url' => ['news.ycombinator.com/item' => 6, 'reddit.com/r/webdev/comments' => 4, 'github.com/inalto/webpulse' => 3,
            'dev.to/someone/log-analytics' => 2, 'blog.example.org/links' => 1],
        'search_engine' => ['google' => 75, 'bing' => 11, 'duckduckgo' => 8, 'ecosia' => 3, 'yahoo' => 2, 'qwant' => 1],
        'keyword' => ['web analytics without cookies' => 3, 'cpanel analytics plugin' => 2, 'awstats alternative' => 2, 'log analyzer' => 1],
        'campaign' => ['autumn-sale' => 5, 'newsletter-oct' => 4, 'launch' => 2],
        'utm_source' => ['newsletter' => 5, 'twitter' => 3, 'partner' => 2],
        'utm_medium' => ['email' => 5, 'social' => 3, 'cpc' => 2],
        'source_medium' => ['newsletter / email' => 5, 'twitter / social' => 3, 'partner / cpc' => 2],
        'country' => ['it' => 30, 'us' => 18, 'de' => 12, 'fr' => 8, 'gb' => 7, 'es' => 6, 'ch' => 4, 'nl' => 3, 'br' => 3,
            'in' => 3, 'jp' => 2, 'ca' => 2, '(unknown)' => 2],
        'city' => [],
        'browser' => ['Chrome' => 55, 'Safari' => 22, 'Firefox' => 9, 'Edge' => 8, 'Samsung Internet' => 3, 'Opera' => 2, 'Other' => 1],
        'browser_version' => ['Chrome 129' => 35, 'Chrome 128' => 20, 'Safari 18' => 14, 'Safari 17' => 8, 'Firefox 131' => 9,
            'Edge 129' => 8, 'Samsung Internet 26' => 3, 'Opera 113' => 2],
        'os' => ['Windows' => 36, 'Android' => 24, 'iOS' => 20, 'macOS' => 14, 'Linux' => 5, 'ChromeOS' => 1],
        'os_version' => ['Windows 10/11' => 36, 'Android 14' => 16, 'iOS 18' => 12, 'iOS 17' => 8, 'macOS 14' => 14, 'Android 13' => 8],
        'device' => ['desktop' => 54, 'mobile' => 41, 'tablet' => 5],
        'status' => ['200' => 80, '304' => 9, '301' => 3, '404' => 5, '302' => 1, '403' => 1, '500' => 0.4, '503' => 0.2],
        'method' => ['GET' => 96, 'POST' => 4],
        'protocol' => ['https' => 93, 'http' => 7],
        'http_version' => ['HTTP/2.0' => 70, 'HTTP/1.1' => 29, 'HTTP/1.0' => 1],
        'file_type' => ['html' => 25, 'js' => 20, 'css' => 12, 'webp' => 15, 'png' => 10, 'jpg' => 9, 'woff2' => 5, 'svg' => 3, 'pdf' => 1],
        'static_file' => ['/assets/hero-2400.jpg' => 30, '/assets/app.js' => 22, '/assets/app.css' => 12, '/wp-content/uploads/team.png' => 10,
            '/fonts/inter.woff2' => 8, '/assets/logo.svg' => 2],
        'download' => ['/files/brochure-2026.pdf' => 6, '/files/price-list.pdf' => 4, '/downloads/webpulse-0.1.0.tar.gz' => 3],
        'not_found' => ['/wp-login.php' => 9, '/old-pricing' => 6, '/.env' => 5, '/blog/2019/hello-world' => 3, '/favicon.png' => 2],
        'server_error' => ['/search' => 3, '/api/checkout' => 1],
        'bot' => ['Googlebot' => 30, 'bingbot' => 12, 'GPTBot' => 10, 'ClaudeBot' => 6, 'AhrefsBot' => 9, 'SemrushBot' => 7,
            'Bytespider' => 5, 'Amazonbot' => 4, 'CCBot' => 3, 'Other bot' => 6],
        'visit_duration' => ['0-10s' => 46, '11-30s' => 9, '31-60s' => 9, '1-2m' => 10, '2-5m' => 12, '5-10m' => 8, '10-30m' => 5, '30m+' => 1],
        'pages_per_visit' => ['1' => 46, '2' => 18, '3' => 12, '4' => 8, '5' => 5, '6-10' => 7, '11-20' => 3, '21+' => 1],
    ];

    /**
     * Creates a domain database with `$days` days of data ending on `$endDay`.
     *
     * @param array{days?:int,end?:string,timezone?:string,scale?:float,seed?:int,user?:string,now?:int,schema_version?:int} $o
     * @return array<string,int> totals of the daily table
     */
    public static function domain(string $path, string $domain, array $o = []): array
    {
        $tzName = $o['timezone'] ?? 'Europe/Rome';
        $tz = new \DateTimeZone($tzName);
        $now = $o['now'] ?? time();
        $end = $o['end'] ?? (new \DateTimeImmutable('@' . $now))->setTimezone($tz)->format('Y-m-d');
        $days = $o['days'] ?? 45;
        $scale = $o['scale'] ?? 1.0;
        mt_srand(($o['seed'] ?? 42) + crc32($domain));

        @unlink($path);
        if (!is_dir(dirname($path))) {
            mkdir(dirname($path), 0750, true);
        }
        $pdo = new PDO('sqlite:' . $path, null, null, [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION]);
        $pdo->exec('PRAGMA journal_mode=DELETE');
        $pdo->exec(self::DOMAIN_DDL);
        $pdo->beginTransaction();

        $insDaily = $pdo->prepare('INSERT INTO daily VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)');
        $insHourly = $pdo->prepare('INSERT INTO hourly VALUES (?,?,?,?,?,?,?)');
        $insDim = $pdo->prepare('INSERT INTO dim_daily VALUES (?,?,?,?,?,?,?,?,?)');
        $diurnal = [2, 1, 1, 1, 1, 2, 4, 7, 10, 12, 13, 12, 11, 12, 13, 13, 12, 11, 10, 10, 9, 8, 6, 4];
        $totals = array_fill_keys(['visits', 'visitors', 'pageviews', 'hits', 'bytes', 'bounces', 'duration', 'downloads', 'errors_4xx', 'errors_5xx', 'bot_hits', 'bot_bytes'], 0);
        $endDt = \DateTimeImmutable::createFromFormat('!Y-m-d', $end, $tz);
        $first = null;

        for ($i = $days - 1; $i >= 0; $i--) {
            $dt = $endDt->modify("-$i day");
            $day = $dt->format('Y-m-d');
            $first ??= $day;
            $weekend = (int) $dt->format('N') >= 6;
            $trend = 1 + 0.35 * (($days - $i) / $days);
            $visits = (int) round((120 + mt_rand(0, 60)) * $trend * ($weekend ? 0.62 : 1.0) * $scale);
            if ($i === 0) {
                $visits = (int) round($visits * 0.55); // today is partial
            }
            $pageviews = (int) round($visits * (2.4 + mt_rand(0, 60) / 100));
            $bounces = (int) round($visits * (0.40 + mt_rand(0, 10) / 100));
            $duration = $visits * (80 + mt_rand(0, 50));
            $hitsSplit = self::split($pageviews * 3, self::KEYS['status']);
            $hits = array_sum($hitsSplit);
            $bytes = $hits * (18000 + mt_rand(0, 9000));
            $statusKeys = array_keys(self::KEYS['status']);
            $st = array_combine($statusKeys, $hitsSplit);
            $e4 = $st['404'] + $st['403'];
            $e5 = $st['500'] + $st['503'];
            $downloads = (int) round($visits * 0.04);
            $botHits = (int) round($hits * (0.38 + mt_rand(0, 12) / 100));
            $botBytes = $botHits * 9000;
            $row = [
                'visits' => $visits, 'visitors' => (int) round($visits * 0.83), 'pageviews' => $pageviews, 'hits' => $hits,
                'bytes' => $bytes, 'bounces' => $bounces, 'duration' => $duration, 'downloads' => $downloads,
                'errors_4xx' => $e4, 'errors_5xx' => $e5, 'bot_hits' => $botHits, 'bot_bytes' => $botBytes,
            ];
            $insDaily->execute(array_merge([$day], array_values($row)));
            foreach ($row as $k => $v) {
                $totals[$k] += $v;
            }

            $hv = self::split($visits, $diurnal);
            $hp = self::split($pageviews, $diurnal);
            $hh = self::split($hits, $diurnal);
            $hb = self::split($bytes, $diurnal);
            $hbot = self::split($botHits, array_fill(0, 24, 1));
            for ($h = 0; $h < 24; $h++) {
                $insHourly->execute([$day, $h, $hv[$h], $hp[$h], $hh[$h], $hb[$h], $hbot[$h]]);
            }

            self::dims($insDim, $day, $row, $st);
        }

        self::raw($pdo, $endDt, $tz, $now);

        $meta = [
            'schema_version' => (string) ($o['schema_version'] ?? 1), 'user' => $o['user'] ?? 'alice', 'domain' => $domain,
            'created_at' => (string) ($now - 86400 * $days), 'timezone' => $tzName, 'first_day' => $first, 'last_day' => $end,
            'last_collect_at' => (string) ($now - 140), 'last_rollup_at' => (string) ($now - 3600 * 5),
            'total_lines' => (string) ($totals['hits'] + $totals['bot_hits']), 'next_visit_id' => '100000',
        ];
        $insMeta = $pdo->prepare('INSERT INTO meta VALUES (?, ?)');
        foreach ($meta as $k => $v) {
            $insMeta->execute([$k, $v]);
        }
        $pdo->commit();
        return $totals;
    }

    /**
     * Splits an integer total over weights (largest remainder, exact sum).
     *
     * @param array<array-key,int|float> $weights
     * @return list<int>
     */
    public static function split(int $total, array $weights): array
    {
        $w = array_values($weights);
        $sum = array_sum($w);
        if ($sum <= 0 || $total <= 0) {
            return array_fill(0, count($w), 0);
        }
        $raw = array_map(static fn ($x) => $total * $x / $sum, $w);
        $out = array_map(static fn ($x) => (int) floor($x), $raw);
        $rem = $total - array_sum($out);
        $frac = [];
        foreach ($raw as $i => $x) {
            $frac[$i] = $x - floor($x);
        }
        arsort($frac);
        foreach (array_keys($frac) as $i) {
            if ($rem-- <= 0) {
                break;
            }
            $out[$i]++;
        }
        return $out;
    }

    /**
     * @param array<string,int> $row
     * @param array<string,int> $status hits per status code
     */
    private static function dims(\PDOStatement $ins, string $day, array $row, array $status): void
    {
        $jitter = static function (array $weights): array {
            $out = [];
            foreach ($weights as $k => $w) {
                $out[$k] = $w * (0.75 + mt_rand(0, 50) / 100);
            }
            return $out;
        };
        $put = static function (string $dim, array $keys, array $cols) use ($ins, $day): void {
            foreach ($keys as $i => $k) {
                $vals = [];
                foreach (['visits', 'pageviews', 'hits', 'bytes', 'bounces', 'duration'] as $c) {
                    $vals[] = $cols[$c][$i] ?? 0;
                }
                if (array_sum($vals) === 0) {
                    continue;
                }
                $ins->execute(array_merge([$day, $dim, (string) $k], $vals));
            }
        };
        $v = $row['visits'];
        // page-level
        $w = $jitter(self::KEYS['page']);
        $keys = array_keys($w);
        $pv = self::split($row['pageviews'], $w);
        $put('page', $keys, [
            'pageviews' => $pv, 'visits' => array_map(static fn ($x) => (int) round($x * 0.8), $pv),
            'hits' => $pv, 'bytes' => array_map(static fn ($x) => $x * 42000, $pv),
        ]);
        $entries = self::split($v, $w);
        $put('entry_page', $keys, [
            'visits' => $entries, 'pageviews' => $entries,
            'bounces' => self::split($row['bounces'], $w), 'duration' => self::split($row['duration'], $w),
        ]);
        $put('exit_page', $keys, ['visits' => self::split($v, $jitter(self::KEYS['page']))]);

        // visit-level dims
        $visitDims = ['channel', 'country', 'browser', 'browser_version', 'os', 'os_version', 'device', 'search_engine',
            'referrer_host', 'referrer_url', 'keyword', 'campaign', 'utm_source', 'utm_medium', 'source_medium', 'visit_duration', 'pages_per_visit'];
        $fraction = ['search_engine' => 0.38, 'referrer_host' => 0.55, 'referrer_url' => 0.12, 'keyword' => 0.03,
            'campaign' => 0.06, 'utm_source' => 0.06, 'utm_medium' => 0.06, 'source_medium' => 0.06];
        foreach ($visitDims as $dim) {
            $w = in_array($dim, ['visit_duration', 'pages_per_visit'], true) ? self::KEYS[$dim] : $jitter(self::KEYS[$dim]);
            $tv = (int) round($v * ($fraction[$dim] ?? 1.0));
            $vs = self::split($tv, $w);
            $cols = ['visits' => $vs, 'pageviews' => self::split((int) round($row['pageviews'] * $tv / max(1, $v)), $w)];
            if (in_array($dim, ['channel', 'country', 'device', 'browser', 'os', 'referrer_host', 'search_engine', 'campaign'], true)) {
                $b = [];
                $dur = [];
                $i = 0;
                foreach ($w as $k => $_) {
                    $rate = 0.3 + (crc32($dim . $k) % 40) / 100;
                    $b[] = (int) round($vs[$i] * $rate);
                    $dur[] = $vs[$i] * (50 + crc32($k) % 120);
                    $i++;
                }
                $cols['bounces'] = $b;
                $cols['duration'] = $dur;
            }
            $put($dim, array_keys($w), $cols);
        }

        // hit-level dims
        $put('status', array_keys($status), ['hits' => array_values($status), 'bytes' => array_map(static fn ($x) => $x * 15000, array_values($status))]);
        foreach (['method', 'protocol', 'http_version', 'file_type'] as $dim) {
            $w = $jitter(self::KEYS[$dim]);
            $h = self::split($row['hits'], $w);
            $put($dim, array_keys($w), ['hits' => $h, 'bytes' => self::split($row['bytes'], $w)]);
        }
        $w = $jitter(self::KEYS['static_file']);
        $sh = self::split((int) round($row['hits'] * 0.5), $w);
        $sizes = [2400000, 180000, 60000, 900000, 48000, 3000];
        $put('static_file', array_keys($w), ['hits' => $sh, 'bytes' => array_map(static fn ($x, $s) => $x * $s, $sh, $sizes)]);
        $w = $jitter(self::KEYS['download']);
        $dh = self::split($row['downloads'], $w);
        $put('download', array_keys($w), ['hits' => $dh, 'bytes' => array_map(static fn ($x) => $x * 2100000, $dh)]);
        $w = $jitter(self::KEYS['not_found']);
        $nf = self::split($status['404'], $w);
        $put('not_found', array_keys($w), ['hits' => $nf, 'bytes' => array_map(static fn ($x) => $x * 800, $nf)]);
        $refs = [];
        foreach (array_keys($w) as $i => $p) {
            if ($p === '/old-pricing' || $p === '/blog/2019/hello-world') {
                $hosts = $p === '/old-pricing' ? ['bing.com' => 3, 'oldforum.example.net' => 2] : ['google.com' => 1];
                foreach (self::split((int) floor($nf[$i] * 0.7), $hosts) as $j => $n) {
                    $refs[$p . ' ← ' . array_keys($hosts)[$j]] = $n;
                }
            }
        }
        $put('not_found_referrer', array_keys($refs), ['hits' => array_values($refs)]);
        $w = self::KEYS['server_error'];
        $put('server_error', array_keys($w), ['hits' => self::split($status['500'] + $status['503'], $w)]);
        $w = $jitter(self::KEYS['bot']);
        $bh = self::split($row['bot_hits'], $w);
        $put('bot', array_keys($w), ['hits' => $bh, 'bytes' => self::split($row['bot_bytes'], $w)]);
    }

    /** Raw visits/events for the last 3 days plus open visits for "now". */
    private static function raw(PDO $pdo, \DateTimeImmutable $endDt, \DateTimeZone $tz, int $now): void
    {
        $insV = $pdo->prepare('INSERT INTO visits VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)');
        $insE = $pdo->prepare('INSERT INTO events (ts, visit_id, type, path, status, bytes, referrer) VALUES (?,?,?,?,?,?,?)');
        $insO = $pdo->prepare('INSERT INTO open_visits VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)');
        $pages = array_keys(self::KEYS['page']);
        $profiles = [
            ['organic', 'google.com', 'google', null, null, 'it', 'Chrome', '129', 'Windows', '10/11', 'desktop'],
            ['direct', null, null, null, null, 'us', 'Safari', '18', 'iOS', '18', 'mobile'],
            ['social', 'linkedin.com', null, null, null, 'de', 'Firefox', '131', 'Linux', null, 'desktop'],
            ['referral', 'news.ycombinator.com', null, null, null, 'gb', 'Chrome', '128', 'Android', '14', 'mobile'],
            ['campaign', null, null, null, 'autumn-sale', 'fr', 'Edge', '129', 'Windows', '10/11', 'desktop'],
            ['organic', 'duckduckgo.com', 'duckduckgo', null, null, 'ch', 'Safari', '17', 'macOS', '14', 'desktop'],
            ['email', 'mail.google.com', null, null, 'newsletter-oct', 'es', 'Samsung Internet', '26', 'Android', '13', 'tablet'],
        ];
        $id = 1000;
        $start = $now - 3 * 86400;
        for ($t = $start; $t < $now - 2400; $t += 600 + mt_rand(0, 900)) {
            $p = $profiles[mt_rand(0, count($profiles) - 1)];
            $n = mt_rand(0, 9) < 4 ? 1 : mt_rand(2, 7);
            $ts = $t;
            $acts = [];
            $bytes = 0;
            for ($k = 0; $k < $n; $k++) {
                $path = $k === 0 ? $pages[mt_rand(0, 4)] : $pages[mt_rand(0, count($pages) - 1)];
                $status = mt_rand(0, 30) === 0 ? 404 : 200;
                $b = mt_rand(8000, 60000);
                $acts[] = [$ts, $status === 404 ? 'error' : 'pageview', $path, $status, $b];
                $bytes += $b;
                $ts += mt_rand(15, 200);
            }
            if (mt_rand(0, 6) === 0) {
                $acts[] = [$ts, 'download', '/files/brochure-2026.pdf', 200, 2100000];
            }
            $last = end($acts)[0];
            $day = (new \DateTimeImmutable('@' . $t))->setTimezone($tz)->format('Y-m-d');
            $pv = count(array_filter($acts, static fn ($a) => $a[1] === 'pageview'));
            $insV->execute([$id, substr(hash('sha256', (string) $id), 0, 16), $day, $t, $last, $n === 1 ? 0 : $last - $t, $pv, count($acts), $bytes,
                $acts[0][2], end($acts)[2], $p[0], $p[1], $p[2], $p[3], $p[4], $p[5], null, $p[6], $p[7], $p[8], $p[9], $p[10], null]);
            foreach ($acts as $i => $a) {
                $insE->execute([$a[0], $id, $a[1], $a[2], $a[3], $a[4], $i === 0 ? $p[1] : null]);
            }
            $id++;
        }
        // bot events
        for ($t = $now - 1700; $t < $now; $t += 97) {
            $insE->execute([$t, null, 'bot', '/robots.txt', 200, 120, null]);
        }
        // open visits active within the last 30 minutes
        for ($k = 0; $k < 6; $k++) {
            $p = $profiles[$k % count($profiles)];
            $t0 = $now - 1500 + $k * 200;
            $n = 1 + $k % 4;
            $ts = $t0;
            for ($j = 0; $j < $n; $j++) {
                $insE->execute([$ts, $id, 'pageview', $pages[($k + $j) % count($pages)], 200, 24000, $j === 0 ? $p[1] : null]);
                $last = $ts;
                $ts += 90;
            }
            $attrs = json_encode(['channel' => $p[0], 'referrer_host' => $p[1], 'search_engine' => $p[2], 'campaign' => $p[4],
                'country' => $p[5], 'browser' => $p[6], 'browser_version' => $p[7], 'os' => $p[8], 'os_version' => $p[9], 'device' => $p[10]]);
            $day = (new \DateTimeImmutable('@' . $t0))->setTimezone($tz)->format('Y-m-d');
            $insO->execute([substr(hash('sha256', 'open' . $k), 0, 16), $id, $day, $t0, $last, $last, $n, $n, $n * 24000,
                $pages[$k % count($pages)], $pages[($k + $n - 1) % count($pages)], '[]', $attrs]);
            $id++;
        }
    }

    /**
     * Creates the registry database.
     *
     * @param list<array{user:string,owner:string,enabled?:?int,domains:array<string,string>}> $accounts
     */
    public static function registry(string $path, array $accounts, int $now): void
    {
        @unlink($path);
        $pdo = new PDO('sqlite:' . $path, null, null, [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION]);
        $pdo->exec(self::REGISTRY_DDL);
        $pdo->exec("INSERT INTO meta VALUES ('schema_version', '1'), ('installed_at', '" . ($now - 86400 * 50) . "'), ('version', '0.1.0')");
        $a = $pdo->prepare('INSERT INTO accounts VALUES (?,?,?,?,?,?,NULL)');
        $d = $pdo->prepare('INSERT INTO domains VALUES (?,?,?,?,?,?,NULL)');
        foreach ($accounts as $acc) {
            $a->execute([$acc['user'], $acc['owner'], $acc['enabled'] ?? null, null, $now - 86400 * 50, $now]);
            foreach ($acc['domains'] as $dom => $kind) {
                $d->execute([$acc['user'], $dom, null, $kind, $now - 86400 * 50, $now]);
            }
        }
        $j = $pdo->prepare('INSERT INTO jobs (type, scope, started_at, finished_at, status, lines, files, domains, message) VALUES (?,?,?,?,?,?,?,?,?)');
        for ($k = 12; $k >= 1; $k--) {
            $t = $now - $k * 300;
            $j->execute(['collect', 'all', $t, $t + 2, 'ok', 300 + $k * 17, 4, 3, null]);
        }
        $j->execute(['rollup', 'all', $now - 30000, $now - 29990, 'ok', 0, 0, 3, null]);
    }

    /**
     * Fills `domain_summary` from the domain databases (what the engine's
     * rollup does), so the admin overview has data.
     *
     * @param array<string,array<string,string>> $dbs user => [domain => db path]
     */
    public static function summaries(string $registry, array $dbs, int $now, string $tz = 'Europe/Rome'): void
    {
        $pdo = new PDO('sqlite:' . $registry, null, null, [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION]);
        $ins = $pdo->prepare('INSERT OR REPLACE INTO domain_summary VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)');
        $zone = new \DateTimeZone($tz);
        $today = (new \DateTimeImmutable('@' . $now))->setTimezone($zone);
        $d7 = $today->modify('-6 day')->format('Y-m-d');
        $d30 = $today->modify('-29 day')->format('Y-m-d');
        $t = $today->format('Y-m-d');
        foreach ($dbs as $user => $doms) {
            foreach ($doms as $dom => $file) {
                $db = new PDO('sqlite:' . $file);
                $q = static fn (string $from) => $db->query("SELECT COALESCE(SUM(visits),0) v, COALESCE(SUM(pageviews),0) p, COALESCE(SUM(bytes),0) b, COALESCE(SUM(bot_hits),0) bh, COALESCE(SUM(hits),0) h FROM daily WHERE day >= '$from' AND day <= '$t'")->fetch(PDO::FETCH_ASSOC);
                [$a, $b, $c] = [$q($t), $q($d7), $q($d30)];
                $ins->execute([$user, $dom, filesize($file), $now - 140, $now - 200, (int) $c['h'] + (int) $c['bh'], $a['v'], $a['p'], $b['v'], $b['p'], $b['b'], $c['v'], $c['p'], $c['b'], $c['bh']]);
            }
        }
    }

    /**
     * Builds a complete fake server under $root: config.json, cPanel user
     * files, /etc/userdomains, data dir with domain DBs and the registry.
     * Returns ['config' => path, 'root' => $root, 'totals' => [...]].
     *
     * Accounts: alice (owner root; example.com, shop.example.com),
     * bob (owner reseller1; blog.net), carol (owner reseller1; no data yet:
     * carol.org registered only), dave (owner root, disabled; dave.io no data).
     *
     * @return array{config:string,root:string,totals:array<string,array<string,int>>,now:int}
     */
    public static function fakeServer(string $root, ?int $now = null): array
    {
        $now ??= time();
        foreach (['etc', 'var/cpanel/users', 'var/webpulse/data', 'var/webpulse/log', 'var/webpulse/run', 'var/webpulse/geoip', 'home'] as $d) {
            if (!is_dir("$root/$d")) {
                mkdir("$root/$d", 0755, true);
            }
        }
        $accounts = [
            ['user' => 'alice', 'owner' => 'root', 'domains' => ['example.com' => 'main', 'shop.example.com' => 'sub']],
            ['user' => 'bob', 'owner' => 'reseller1', 'domains' => ['blog.net' => 'main']],
            ['user' => 'carol', 'owner' => 'reseller1', 'domains' => ['carol.org' => 'main']],
            ['user' => 'dave', 'owner' => 'root', 'enabled' => 0, 'domains' => ['dave.io' => 'main']],
        ];
        $ud = '';
        foreach ($accounts as $acc) {
            $lines = ['OWNER=' . $acc['owner'], 'USER=' . $acc['user']];
            $n = 0;
            foreach ($acc['domains'] as $dom => $_) {
                $lines[] = ($n === 0 ? 'DNS=' : 'DNS' . $n . '=') . $dom;
                $ud .= "$dom: {$acc['user']}\n";
                $n++;
            }
            file_put_contents("$root/var/cpanel/users/{$acc['user']}", implode("\n", $lines) . "\n");
        }
        file_put_contents("$root/etc/userdomains", $ud);

        $data = "$root/var/webpulse/data";
        $dbs = [
            'alice' => ['example.com' => "$data/alice/example.com.db", 'shop.example.com' => "$data/alice/shop.example.com.db"],
            'bob' => ['blog.net' => "$data/bob/blog.net.db"],
        ];
        $totals = [];
        $scales = ['example.com' => 1.0, 'shop.example.com' => 0.35, 'blog.net' => 0.6];
        $schema = class_exists(\WebPulse\Store\Schema::class) && defined(\WebPulse\Store\Schema::class . '::VERSION')
            ? (int) constant(\WebPulse\Store\Schema::class . '::VERSION') : 1;
        foreach ($dbs as $user => $doms) {
            foreach ($doms as $dom => $file) {
                $totals[$dom] = self::domain($file, $dom, ['user' => $user, 'scale' => $scales[$dom], 'now' => $now, 'schema_version' => $schema, 'days' => 75]);
            }
            @mkdir("$data/$user/user", 0700, true);
        }
        @mkdir("$data/carol", 0750, true);
        @mkdir("$data/carol/user", 0700, true);
        self::registry("$root/var/webpulse/webpulse.db", $accounts, $now);
        self::summaries("$root/var/webpulse/webpulse.db", $dbs, $now);

        $config = [
            'version' => 1, 'default_enabled' => true, 'timezone' => 'Europe/Rome', 'raw_retention_days' => 30,
            'max_rows_per_dim' => 1000, 'session_timeout_min' => 30, 'store_ip' => false, 'exclude_ips' => [],
            'exclude_paths' => ['/.well-known/'], 'count_head_requests' => false,
            'geoip' => ['enabled' => false, 'country_db' => "$root/var/webpulse/geoip/dbip-country-lite.mmdb", 'city_db' => null],
            'php_bin' => PHP_BINARY,
            'paths' => [
                'data_dir' => $data, 'registry_db' => "$root/var/webpulse/webpulse.db", 'log_dir' => "$root/var/webpulse/log",
                'run_dir' => "$root/var/webpulse/run", 'domlogs_dirs' => ["$root/etc/apache2/logs/domlogs/{user}"],
                'archive_dir' => "$root/home/{user}/logs", 'userdomains_file' => "$root/etc/userdomains",
                'cpanel_users_dir' => "$root/var/cpanel/users", 'home_root' => "$root/home",
                'geoip_dir' => "$root/var/webpulse/geoip",
            ],
        ];
        file_put_contents("$root/config.json", json_encode($config, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES));
        return ['config' => "$root/config.json", 'root' => $root, 'totals' => $totals, 'now' => $now];
    }

    /** Fixed clock of the shared fixture: 2026-10-16 12:00 Europe/Rome. */
    public const NOW = 1792144800;

    /** @var array{config:string,root:string,totals:array<string,array<string,int>>,now:int}|null */
    private static ?array $shared = null;

    /**
     * The shared seeded server under tests/tmp/ui-seed (built once per run).
     *
     * @return array{config:string,root:string,totals:array<string,array<string,int>>,now:int}
     */
    public static function shared(): array
    {
        putenv('WEBPULSE_NOW=' . self::NOW);
        if (self::$shared === null) {
            $root = dirname(__DIR__) . '/tmp/ui-seed';
            self::rmTree($root);
            self::$shared = self::fakeServer($root, self::NOW);
        }
        return self::$shared;
    }

    /** Fresh copy of the seeded server (for tests that mutate it). */
    public static function fresh(string $name): array
    {
        $src = self::shared();
        $dst = dirname(__DIR__) . '/tmp/' . $name;
        self::rmTree($dst);
        self::copyTree($src['root'], $dst);
        $cfg = (string) file_get_contents($src['config']);
        file_put_contents($dst . '/config.json', str_replace($src['root'], $dst, $cfg));
        return ['config' => $dst . '/config.json', 'root' => $dst, 'totals' => $src['totals'], 'now' => $src['now']];
    }

    /** Principal from `user:alice`, `admin`, `reseller:reseller1` (owned users from the fake cPanel files). */
    public static function principal(string $spec, string $root): \WebPulse\Http\Principal
    {
        [$type, $name] = array_pad(explode(':', $spec, 2), 2, '');
        if ($type === 'admin') {
            return new \WebPulse\Http\Principal('admin', 'root');
        }
        if ($type === 'reseller') {
            $owned = [];
            foreach (glob($root . '/var/cpanel/users/*') ?: [] as $f) {
                if (str_contains((string) file_get_contents($f), "OWNER=$name\n")) {
                    $owned[] = basename($f);
                }
            }
            return new \WebPulse\Http\Principal('reseller', $name, $owned);
        }
        return new \WebPulse\Http\Principal('user', $name);
    }

    /**
     * Calls the API in-process.
     *
     * @param array<string,mixed>      $query
     * @param array<string,mixed>|null $body  JSON body (POST)
     * @param array<string,string>     $headers
     * @return array{status:int,json:mixed,body:string,response:\WebPulse\Http\Response}
     */
    public static function api(array $env, string $principal, array $query, ?array $body = null, ?string $method = null, ?array $headers = null): array
    {
        putenv('WEBPULSE_NOW=' . $env['now']);
        $app = \WebPulse\App::boot($env['config']);
        $method ??= $body === null ? 'GET' : 'POST';
        $headers ??= $method === 'POST' ? ['X-WebPulse-Request' => '1'] : [];
        $req = new \WebPulse\Http\Request($method, $query, $body === null ? null : json_encode($body), $headers, '203.0.113.9');
        $res = \WebPulse\Http\Api::dispatch($app, $req, self::principal($principal, $env['root']));
        return ['status' => $res->status, 'json' => json_decode($res->body, true), 'body' => $res->body, 'response' => $res];
    }

    public static function rmTree(string $dir): void
    {
        if (!file_exists($dir) && !is_link($dir)) {
            return;
        }
        if (is_link($dir) || is_file($dir)) {
            @unlink($dir);
            return;
        }
        foreach (scandir($dir) ?: [] as $e) {
            if ($e !== '.' && $e !== '..') {
                self::rmTree($dir . '/' . $e);
            }
        }
        @rmdir($dir);
    }

    private static function copyTree(string $src, string $dst): void
    {
        @mkdir($dst, 0755, true);
        foreach (scandir($src) ?: [] as $e) {
            if ($e === '.' || $e === '..') {
                continue;
            }
            is_dir("$src/$e") ? self::copyTree("$src/$e", "$dst/$e") : copy("$src/$e", "$dst/$e");
        }
    }
}
