<?php

declare(strict_types=1);

namespace WebPulse\Collect;

use WebPulse\Classify\GeoIp;
use WebPulse\Classify\Path;
use WebPulse\Classify\Referrer;
use WebPulse\Classify\UserAgent;
use WebPulse\Log\Hit;
use WebPulse\Store\DomainStore;
use WebPulse\Store\Dims;
use WebPulse\Util\DayClock;
use WebPulse\Util\Ip;

/**
 * Processing state of one domain during one collector run: classifies hits
 * (METRICS.md "Hit classification"), feeds the counters and the sessionizer,
 * and flushes everything — including log checkpoints — in one transaction.
 */
final class DomainRun
{
    public const CLASS_IGNORED = 'ignored';
    public const CLASS_BOT = 'bot';
    public const CLASS_ERROR = 'error';
    public const CLASS_REDIRECT = 'redirect';
    public const CLASS_DOWNLOAD = 'download';
    public const CLASS_STATIC = 'static';
    public const CLASS_PAGEVIEW = 'pageview';
    public const CLASS_OTHER = 'other';

    private const REDIRECTS = [301 => true, 302 => true, 303 => true, 307 => true, 308 => true];
    private const PV_STATUS = [200 => true, 203 => true, 206 => true, 304 => true];

    public Counters $counters;
    public Sessionizer $sessions;
    private DayClock $clock;
    /** @var array<int,array{0:string,1:int}> */
    private array $excludeIps;
    /** @var string[] */
    private array $excludePaths;
    /** @var array<string,bool> */
    private array $ipExcludedCache = [];
    /** @var array<string,bool> */
    private array $pathExcludedCache = [];
    /** @var array{0:Hit,1:array<string,mixed>,2:array<string,?string>,3:bool}|null hit being processed */
    private ?array $current = null;
    private \Closure $attrsFn;
    /** @var array<string,?string> referrer => external host */
    private array $refHostCache = [];
    private bool $countHead;
    private bool $storeIp;
    private ?string $minDay = null;
    private ?string $maxDay = null;
    private ?int $minTs = null;
    private int $lines = 0;
    /** @var array<string,int> class => hits (run statistics) */
    public array $classes = [];

    /**
     * @param array{timezone:?string,exclude_ips:string[],exclude_paths:string[]} $domainSettings
     */
    public function __construct(
        private DomainStore $store,
        private string $domain,
        string $timezone,
        array $domainSettings,
        array $config,
        private UserAgent $ua,
        private Referrer $referrer,
        private Path $path,
        private GeoIp $geo,
        \Closure $salt,
        bool $persistent,
    ) {
        $this->clock = new DayClock($timezone);
        $this->counters = new Counters();
        $timeout = max(60, (int) ($config['session_timeout_min'] ?? 30) * 60);
        $next = (int) ($store->getMeta('next_visit_id') ?? 1);
        $maxId = (int) $store->pdo()->query('SELECT MAX(id) FROM (SELECT MAX(visit_id) AS id FROM visits UNION ALL SELECT MAX(visit_id) FROM open_visits)')->fetchColumn();
        $this->sessions = new Sessionizer($this->counters, $this->clock, $salt, $timeout, max($next, $maxId + 1), $store->pdo());
        if ($persistent) {
            $this->sessions->load($store->pdo());
        }
        $this->excludeIps = Ip::compile(array_merge((array) ($config['exclude_ips'] ?? []), $domainSettings['exclude_ips']));
        $this->excludePaths = array_values(array_filter(array_merge((array) ($config['exclude_paths'] ?? []), $domainSettings['exclude_paths']), 'is_string'));
        $this->countHead = (bool) ($config['count_head_requests'] ?? false);
        $this->storeIp = (bool) ($config['store_ip'] ?? false);
        $this->attrsFn = fn (): array => $this->visitAttrs(...$this->current);
    }

    public function clock(): DayClock
    {
        return $this->clock;
    }

    /** Lines processed (parsed hits) so far. */
    public function lines(): int
    {
        return $this->lines;
    }

    /** Smallest hit timestamp processed in this run. */
    public function minTs(): ?int
    {
        return $this->minTs;
    }

    /**
     * Classifies and accounts one hit.
     *
     * @return string the hit class (see CLASS_* constants)
     */
    public function process(Hit $h, bool $https): string
    {
        $this->lines++;
        $class = $this->doProcess($h, $https);
        $this->classes[$class] = ($this->classes[$class] ?? 0) + 1;
        return $class;
    }

    private function doProcess(Hit $h, bool $https): string
    {
        $method = $h->method;
        if (!$this->countHead && ($method === 'HEAD' || $method === 'OPTIONS')) {
            return self::CLASS_IGNORED;
        }
        if ($this->excludeIps !== []) {
            $ex = $this->ipExcludedCache[$h->ip] ??= Ip::matches($h->ip, $this->excludeIps);
            if (count($this->ipExcludedCache) > 50000) {
                $this->ipExcludedCache = [];
            }
            if ($ex) {
                return self::CLASS_IGNORED;
            }
        }
        $p = $this->path->analyze($h->target);
        $path = $p['path'];
        if ($this->excludePaths !== []) {
            $ex = $this->pathExcludedCache[$path] ?? null;
            if ($ex === null) {
                if (count($this->pathExcludedCache) > 50000) {
                    $this->pathExcludedCache = [];
                }
                $ex = false;
                foreach ($this->excludePaths as $prefix) {
                    if (str_starts_with($path, $prefix)) {
                        $ex = true;
                        break;
                    }
                }
                $this->pathExcludedCache[$path] = $ex;
            }
            if ($ex) {
                return self::CLASS_IGNORED;
            }
        }
        $ts = $h->ts;
        [$day, $hour] = $this->clock->at($ts);
        if ($this->minDay === null || $day < $this->minDay) {
            $this->minDay = $day;
        }
        if ($this->maxDay === null || $day > $this->maxDay) {
            $this->maxDay = $day;
        }
        if ($this->minTs === null || $ts < $this->minTs) {
            $this->minTs = $ts;
        }
        $c = $this->counters;
        $bytes = $h->bytes;
        $ua = $this->ua->parse($h->ua);
        if ($ua['bot'] !== null) {
            $c->day($day, 'bot_hits');
            $c->day($day, 'bot_bytes', $bytes);
            $c->hour($day, $hour, 'bot_hits');
            $c->dimHit($day, 'bot', $ua['bot'], $bytes);
            return self::CLASS_BOT;
        }
        $status = $h->status;
        $key = $p['key'];
        if ($status >= 400) {
            $class = self::CLASS_ERROR;
        } elseif (isset(self::REDIRECTS[$status])) {
            $class = self::CLASS_REDIRECT;
        } elseif ($p['is_download'] && ($status === 200 || $status === 206)) {
            $class = self::CLASS_DOWNLOAD;
        } elseif ($p['is_static']) {
            $class = self::CLASS_STATIC;
        } elseif (($method === 'GET' || $method === 'POST') && isset(self::PV_STATUS[$status])) {
            $class = self::CLASS_PAGEVIEW;
        } else {
            $class = self::CLASS_OTHER;
        }

        $d = &$c->daily[$day];
        $d['hits'] = ($d['hits'] ?? 0) + 1;
        $d['bytes'] = ($d['bytes'] ?? 0) + $bytes;
        $hr = &$c->hourly[$day][$hour];
        $hr['hits'] = ($hr['hits'] ?? 0) + 1;
        $hr['bytes'] = ($hr['bytes'] ?? 0) + $bytes;
        unset($d, $hr);
        $dd = &$c->dims[$day];
        foreach ([
            'status' => $status === 0 ? '-' : (string) $status,
            'method' => $method === '' ? '(invalid)' : $method,
            'protocol' => $p['scheme'] ?? ($https ? 'https' : 'http'),
            'http_version' => $h->protocol === '' ? '(none)' : $h->protocol,
            'file_type' => $p['file_type'],
        ] as $dim => $k) {
            $row = &$dd[$dim][$k];
            if ($row === null) {
                $row = [0, 0, 0, 0, 0, 0];
            }
            $row[2]++;
            $row[3] += $bytes;
        }
        unset($dd, $row);

        $evType = null;
        switch ($class) {
            case self::CLASS_ERROR:
                $c->day($day, $status >= 500 ? 'errors_5xx' : 'errors_4xx');
                if ($status === 404) {
                    $c->dimHit($day, 'not_found', $key, $bytes);
                    $rh = $this->referrer->anyHost($h->referrer);
                    if ($rh !== null) {
                        $c->dimHit($day, 'not_found_referrer', \WebPulse\Util\Str::key($key . Dims::NF_REF_SEP . $rh), $bytes);
                    }
                } elseif ($status >= 500) {
                    $c->dimHit($day, 'server_error', $key, $bytes);
                }
                $evType = 'error';
                break;
            case self::CLASS_REDIRECT:
                $evType = 'redirect';
                break;
            case self::CLASS_DOWNLOAD:
                $c->day($day, 'downloads');
                $c->dimHit($day, 'download', $key, $bytes);
                $evType = 'download';
                break;
            case self::CLASS_STATIC:
                $c->dimHit($day, 'static_file', $key, $bytes);
                break;
            case self::CLASS_PAGEVIEW:
                $c->day($day, 'pageviews');
                $c->hour($day, $hour, 'pageviews');
                $c->dim($day, 'page', $key, Counters::PV);
                $c->dimHit($day, 'page', $key, $bytes);
                $evType = 'pageview';
                break;
        }

        $isPv = $class === self::CLASS_PAGEVIEW;
        $this->current = [$h, $p, $ua, $https];
        $visitId = $this->sessions->hit(
            $ts,
            $h->ip,
            $h->ua,
            $isPv,
            $isPv || $class === self::CLASS_DOWNLOAD,
            $key,
            $bytes,
            $this->attrsFn
        );
        if ($evType !== null) {
            $ref = $h->referrer;
            if ($ref === null) {
                $rh = null;
            } else {
                if (!array_key_exists($ref, $this->refHostCache)) {
                    if (count($this->refHostCache) > 20000) {
                        $this->refHostCache = [];
                    }
                    $this->refHostCache[$ref] = $this->referrer->externalHost($ref, $this->domain);
                }
                $rh = $this->refHostCache[$ref];
            }
            $c->events[] = [$ts, $visitId, $evType, $key, $status, $bytes, $rh];
        }
        return $class;
    }

    /**
     * Attributes of a visit, from its first hit.
     *
     * @param array<string,mixed> $p
     * @param array<string,?string> $ua
     * @return array<string,mixed>
     */
    private function visitAttrs(Hit $h, array $p, array $ua, bool $https): array
    {
        $ref = $this->referrer->classify($h->referrer, $this->domain, $p);
        $city = $this->geo->city($h->ip);
        $country = $this->geo->country($h->ip);
        $cityKey = null;
        if ($city !== null) {
            $cityKey = \WebPulse\Util\Str::key($city['name'] . ', ' . strtoupper((string) ($city['country'] ?? $country ?? '')));
            $country ??= $city['country'];
        }
        $a = [
            'channel' => $ref['channel'],
            'referrer_host' => $ref['referrer_host'],
            'referrer_url' => $ref['referrer_url'],
            'search_engine' => $ref['search_engine'],
            'keyword' => $ref['keyword'],
            'campaign' => $p['utm_campaign'],
            'utm_source' => $p['utm_source'],
            'utm_medium' => $p['utm_medium'],
            'country' => $country ?? '(unknown)',
            'city' => $cityKey,
            'browser' => $ua['browser'],
            'browser_version' => $ua['browser_version'],
            'os' => $ua['os'],
            'os_version' => $ua['os_version'],
            'device' => $ua['device'],
            'protocol' => $p['scheme'] ?? ($https ? 'https' : 'http'),
        ];
        if ($this->storeIp) {
            $a['ip'] = \WebPulse\Util\Str::clean($h->ip);
        }
        return array_filter($a, static fn ($v) => $v !== null);
    }

    /**
     * Writes counters, sessions, meta and the given checkpoint/archive rows
     * in ONE transaction.
     *
     * @param array<int,array<string,mixed>> $checkpoints rows for `checkpoints` (absolute values)
     * @param array<int,array<string,mixed>> $archives    rows for `imported_archives`
     * @param array<string,?string> $meta extra meta keys
     */
    public function flush(array $checkpoints = [], array $archives = [], array $meta = []): void
    {
        $pdo = $this->store->pdo();
        $pdo->exec('BEGIN IMMEDIATE');
        try {
            $this->sessions->commitAll();
            $this->counters->flush($pdo);
            $this->sessions->flush($pdo);
            if ($checkpoints !== []) {
                $st = $pdo->prepare('INSERT INTO checkpoints (path, inode, offset, size, first_line, last_ts, lines, bad_lines, ignored_lines, updated_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(path) DO UPDATE SET inode = excluded.inode, offset = excluded.offset, size = excluded.size,
                    first_line = excluded.first_line, last_ts = COALESCE(excluded.last_ts, checkpoints.last_ts),
                    lines = checkpoints.lines + excluded.lines, bad_lines = checkpoints.bad_lines + excluded.bad_lines,
                    ignored_lines = checkpoints.ignored_lines + excluded.ignored_lines, updated_at = excluded.updated_at');
                foreach ($checkpoints as $cp) {
                    $st->execute([$cp['path'], $cp['inode'], $cp['offset'], $cp['size'], $cp['first_line'], $cp['last_ts'],
                        $cp['lines'], $cp['bad_lines'], $cp['ignored_lines'], $cp['updated_at']]);
                }
            }
            if ($archives !== []) {
                $st = $pdo->prepare('INSERT INTO imported_archives (path, size, mtime, lines, imported_at, last_ts) VALUES (?, ?, ?, ?, ?, ?)
                    ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, lines = imported_archives.lines + excluded.lines,
                    imported_at = excluded.imported_at, last_ts = MAX(COALESCE(imported_archives.last_ts, 0), COALESCE(excluded.last_ts, 0))');
                foreach ($archives as $a) {
                    $st->execute([$a['path'], $a['size'], $a['mtime'], $a['lines'], $a['imported_at'], $a['last_ts']]);
                }
            }
            $m = $this->store->meta();
            $meta['next_visit_id'] = (string) $this->sessions->nextVisitId();
            if ($this->minDay !== null && (($m['first_day'] ?? null) === null || $this->minDay < $m['first_day'])) {
                $meta['first_day'] = $this->minDay;
            }
            if ($this->maxDay !== null && (($m['last_day'] ?? null) === null || $this->maxDay > $m['last_day'])) {
                $meta['last_day'] = $this->maxDay;
            }
            $meta['total_lines'] = (string) ((int) ($m['total_lines'] ?? 0) + $this->pendingLines);
            $this->pendingLines = 0;
            $this->store->setMetaMany($meta);
            $pdo->exec('COMMIT');
        } catch (\Throwable $e) {
            $pdo->exec('ROLLBACK');
            throw $e;
        }
    }

    /** Lines read since the last flush (parsed or not), added to meta.total_lines. */
    public int $pendingLines = 0;
}
