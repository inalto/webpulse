<?php

declare(strict_types=1);

namespace WebPulse\Http;

use WebPulse\App;
use WebPulse\Reports\DomainData;
use WebPulse\Reports\Export;
use WebPulse\Reports\Health;
use WebPulse\Reports\Overview;
use WebPulse\Reports\Range;
use WebPulse\Reports\Realtime;
use WebPulse\Reports\Report;
use WebPulse\Reports\VisitsLog;

/**
 * The JSON API (docs/API.md). One entry point shared by the cPanel plugin,
 * the WHM plugin and the dev server; the caller supplies the Principal.
 */
final class Api
{
    /** Actions that change state: POST + `X-WebPulse-Request: 1` required. */
    public const MUTATING = [
        'save_user_settings', 'admin.set_account', 'admin.set_domain', 'admin.save_settings', 'admin.run', 'admin.purge',
    ];

    private const DOMAIN_ACTIONS = ['overview', 'report', 'realtime', 'visits_log', 'health', 'export'];

    /**
     * Handles one API request and never throws.
     */
    public static function dispatch(App $app, Request $r, Principal $p): Response
    {
        $t0 = microtime(true);
        $action = $r->get('a', '') ?? '';
        try {
            if (in_array($action, self::MUTATING, true)) {
                if ($r->method !== 'POST') {
                    throw new ApiError(405, 'method_not_allowed', 'This action requires POST');
                }
                if ($r->header('X-WebPulse-Request') !== '1') {
                    throw ApiError::forbidden('Missing X-WebPulse-Request header');
                }
            } elseif ($r->method !== 'GET' && $r->method !== 'HEAD' && $action !== '') {
                throw new ApiError(405, 'method_not_allowed', 'This action requires GET');
            }

            if (in_array($action, self::DOMAIN_ACTIONS, true)) {
                $data = self::domainAction($app, $r, $p, $action);
            } elseif (str_starts_with($action, 'admin.')) {
                if (!$p->hasAdminScope()) {
                    throw ApiError::forbidden('Administrator access required');
                }
                $data = AdminApi::handle($app, $r, $p, substr($action, 6));
            } else {
                $data = match ($action) {
                    'domains' => ['domains' => self::domains($app, $p)],
                    'user_settings' => UserSettingsApi::get($app, $r, $p),
                    'save_user_settings' => UserSettingsApi::save($app, $r, $p),
                    'ping' => ['version' => $app->version(), 'principal' => $p->toArray()],
                    default => throw ApiError::badRequest($action === '' ? 'Parameter "a" is required' : 'Unknown action "' . substr($action, 0, 40) . '"'),
                };
            }
            return $data instanceof Response ? $data : Response::json($data, $t0);
        } catch (ApiError $e) {
            return Response::error($e->status, $e->errorCode, $e->getMessage());
        } catch (\InvalidArgumentException $e) {
            return Response::error(400, 'bad_request', $e->getMessage());
        } catch (\WebPulse\Store\NotFound) {
            return Response::error(404, 'no_data', 'No data has been collected for this domain yet');
        } catch (\WebPulse\Store\SchemaTooNew) {
            return Response::error(503, 'schema_too_new', 'The database is being upgraded; try again in a few minutes');
        } catch (\Throwable $e) {
            try {
                $app->log->error('api ' . $action . ': ' . get_class($e) . ': ' . $e->getMessage());
            } catch (\Throwable) {
                // logging must never break the response
            }
            return Response::error(500, 'internal_error', 'Internal error');
        }
    }

    /**
     * The registry, or null when it is not readable (cPanel accounts).
     */
    public static function registry(App $app, Principal $p): ?object
    {
        if ($p->isUser()) {
            $file = $app->paths->registryDb();
            if (!is_file($file) || !is_readable($file)) {
                return null;
            }
        }
        try {
            return $app->registry();
        } catch (\Throwable) {
            return null;
        }
    }

    /**
     * Opens a visible domain read-only.
     *
     * @throws ApiError 404 when not visible or without data
     */
    public static function open(App $app, Request $r, Principal $p): DomainData
    {
        $ref = DomainResolver::resolve($p, self::registry($app, $p), $app->paths, $r->get('domain'), $r->get('user'));
        if (!$ref->hasData) {
            throw new ApiError(404, 'no_data', 'No data has been collected for this domain yet');
        }
        $store = $app->openDomain($ref->user, $ref->domain, true);
        $now = (new \DateTimeImmutable('@' . App::clock()));
        return new DomainData(
            $store->pdo(),
            $store->meta(),
            $ref->domain,
            $app->serverTimezone(),
            (int) $app->config->get('raw_retention_days', 30),
            $now
        );
    }

    /**
     * @return array<string,mixed>|Response
     */
    private static function domainAction(App $app, Request $r, Principal $p, string $action): array|Response
    {
        $d = self::open($app, $r, $p);
        if ($action === 'realtime') {
            return Realtime::build($d);
        }
        $range = Range::fromQuery($r->query, $d->tz, $d->now(), $d->firstDay());
        $compare = in_array($r->get('compare'), ['1', 'true'], true);
        switch ($action) {
            case 'overview':
                return Overview::build($d, $range, $compare);
            case 'report':
                return Report::build($d, $range, Report::params($r->query));
            case 'visits_log':
                return VisitsLog::build($d, $range, $r->query);
            case 'health':
                return Health::build($d, $range);
            case 'export':
                $format = $r->get('format', 'csv');
                if ($format !== 'csv') {
                    throw ApiError::badRequest('Unsupported format "' . substr((string) $format, 0, 10) . '"');
                }
                $params = Report::params($r->query, true);
                $params['compare'] = false;
                $params['series'] = false;
                $rep = Report::build($d, $range, $params);
                $name = Export::filename($d->domain, $params['dim'], $range);
                return new Response(200, 'text/csv; charset=utf-8', Export::csv($rep), [
                    'Content-Disposition' => 'attachment; filename="' . $name . '"',
                    'Cache-Control' => 'no-store',
                    'X-Content-Type-Options' => 'nosniff',
                ]);
        }
        throw ApiError::badRequest('Unknown action');
    }

    /**
     * `a=domains`: visible domains with a tiny summary.
     *
     * @return list<array<string,mixed>>
     */
    public static function domains(App $app, Principal $p): array
    {
        $registry = self::registry($app, $p);
        $refs = DomainResolver::visibleDomains($p, $registry, $app->paths);
        $summaries = [];
        if ($registry !== null && !$p->isUser()) {
            try {
                foreach ($registry->summaries() as $s) {
                    $summaries[$s['user'] . '/' . $s['domain']] = $s;
                }
            } catch (\Throwable) {
                $summaries = [];
            }
        }
        $now = App::clock();
        $out = [];
        foreach ($refs as $ref) {
            $row = [
                'domain' => $ref->domain, 'user' => $ref->user, 'kind' => $ref->kind, 'enabled' => $ref->enabled ?? true,
                'has_data' => $ref->hasData, 'first_day' => null, 'last_day' => null, 'last_collect_at' => null,
                'visits_today' => 0, 'pageviews_today' => 0, 'timezone' => null, 'db_bytes' => $ref->dbBytes(),
            ];
            $s = $summaries[$ref->user . '/' . $ref->domain] ?? null;
            if ($s !== null) {
                $row['visits_today'] = (int) $s['visits_today'];
                $row['pageviews_today'] = (int) $s['pageviews_today'];
                $row['last_collect_at'] = $s['last_collect_at'] ? date('c', (int) $s['last_collect_at']) : null;
            } elseif ($ref->hasData && (count($refs) <= 200 || $p->isUser())) {
                try {
                    $store = $app->openDomain($ref->user, $ref->domain, true);
                    $d = new DomainData($store->pdo(), $store->meta(), $ref->domain, $app->serverTimezone(), 30, new \DateTimeImmutable('@' . $now));
                    $t = $d->row('SELECT visits, pageviews FROM daily WHERE day = ?', [$d->today()]);
                    $row['visits_today'] = (int) ($t['visits'] ?? 0);
                    $row['pageviews_today'] = (int) ($t['pageviews'] ?? 0);
                    $row['first_day'] = $d->firstDay();
                    $row['last_day'] = $d->lastDay();
                    $row['last_collect_at'] = $d->iso($d->metaString('last_collect_at'));
                    $row['timezone'] = $d->tz->getName();
                } catch (\Throwable) {
                    $row['has_data'] = false;
                }
            }
            $out[] = $row;
        }
        return $out;
    }
}
