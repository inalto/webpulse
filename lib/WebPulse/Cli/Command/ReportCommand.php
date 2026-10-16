<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\App;
use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;
use WebPulse\Cli\UsageException;
use WebPulse\Http\DomainResolver;
use WebPulse\Reports\Dimension;
use WebPulse\Reports\DomainData;
use WebPulse\Reports\Overview;
use WebPulse\Reports\Pretty;
use WebPulse\Reports\Range;
use WebPulse\Reports\Report;

/**
 * webpulse report --user U --domain D [--range 30d | --from X --to Y] [--name DIM] [--limit N] [--sort COL] [--json]
 */
final class ReportCommand implements CommandInterface
{
    public static function summary(): string
    {
        return 'print the overview or one report of a domain (table or --json)';
    }

    public static function usage(): string
    {
        return <<<TXT
        Usage: webpulse report --user USER --domain DOMAIN [--range 30d | --from YYYY-MM-DD --to YYYY-MM-DD]
                               [--name DIMENSION] [--limit 10] [--sort -visits] [--compare] [--json]

        Without --name prints the KPIs of the period (with the change versus the
        previous period). With --name prints the ranked table of a dimension:
        TXT . "\n  " . wordwrap(implode(' ', Dimension::ALL), 76, "\n  ") . "\n"
        . '  Ranges: ' . implode(' ', Range::SHORTCUTS) . "\n";
    }

    public static function options(): array
    {
        return [
            'user' => Args::VALUE, 'domain' => Args::VALUE, 'range' => Args::VALUE, 'from' => Args::VALUE,
            'to' => Args::VALUE, 'name' => Args::VALUE, 'limit' => Args::VALUE, 'sort' => Args::VALUE, 'compare' => Args::FLAG,
        ];
    }

    public function run(Cli $cli, Args $args): int
    {
        $user = (string) ($args->opt('user') ?? '');
        $domain = strtolower((string) ($args->opt('domain') ?? ''));
        if (!DomainResolver::validUser($user) || !DomainResolver::validDomain($domain)) {
            throw new UsageException('--user and --domain are required and must be valid names');
        }
        $app = $cli->app();
        $store = $app->openDomain($user, $domain, true);
        $d = new DomainData($store->pdo(), $store->meta(), $domain, $app->serverTimezone(),
            (int) $app->config->get('raw_retention_days', 30), new \DateTimeImmutable('@' . App::clock()));
        $q = array_filter([
            'range' => $args->opt('range'), 'from' => $args->opt('from'), 'to' => $args->opt('to'),
        ], static fn ($v) => is_string($v) && $v !== '');
        try {
            $range = Range::fromQuery($q, $d->tz, $d->now(), $d->firstDay());
        } catch (\InvalidArgumentException $e) {
            throw new UsageException($e->getMessage());
        }

        $name = $args->opt('name');
        if (!is_string($name) || $name === '') {
            $ov = Overview::build($d, $range, $args->has('compare'));
            $cli->result($ov, static fn () => self::renderOverview($domain, $range, $ov));
            return Cli::EXIT_OK;
        }
        try {
            $p = Report::params(array_filter([
                'name' => $name, 'limit' => $args->opt('limit'), 'sort' => $args->opt('sort'), 'compare' => $args->has('compare') ? '1' : null,
            ], static fn ($v) => $v !== null));
        } catch (\InvalidArgumentException $e) {
            throw new UsageException($e->getMessage());
        }
        $rep = Report::build($d, $range, $p);
        $cli->result($rep, static fn () => self::renderReport($domain, $range, $rep));
        return Cli::EXIT_OK;
    }

    /** @param array<string,mixed> $ov */
    public static function renderOverview(string $domain, Range $r, array $ov): string
    {
        $rows = [];
        $units = [
            'visits' => 'n', 'visitors' => 'n', 'pageviews' => 'n', 'pages_per_visit' => 'd', 'avg_duration' => 't',
            'bounce_rate' => '%', 'bytes' => 'b', 'bot_share' => '%', 'downloads' => 'n', 'errors_4xx' => 'n', 'errors_5xx' => 'n',
        ];
        foreach ($units as $k => $u) {
            $kv = $ov['kpi'][$k];
            $f = static fn ($v) => match ($u) {
                'n' => number_format((float) $v),
                'd' => number_format((float) $v, 2),
                't' => Pretty::duration((float) $v),
                '%' => number_format((float) $v * 100, 1) . '%',
                'b' => Pretty::bytes((float) $v),
            };
            $chg = $kv['change'] === null ? '-' : sprintf('%+.1f%%', $kv['change'] * 100);
            $rows[] = [$k, $f($kv['value']), $f($kv['prev']), $chg];
        }
        return "$domain  {$r->from} → {$r->to} ({$r->days} days, previous {$r->prevFrom} → {$r->prevTo})\n\n"
            . Cli::table(['metric', 'value', 'previous', 'change'], $rows);
    }

    /** @param array<string,mixed> $rep */
    public static function renderReport(string $domain, Range $r, array $rep): string
    {
        $dim = (string) $rep['name'];
        $metrics = Dimension::metrics($dim);
        $rows = [];
        foreach ($rep['rows'] as $row) {
            $line = [mb_strimwidth((string) ($row['label'] ?? $row['key']), 0, 60, '…')];
            foreach ($metrics as $m) {
                $v = $row[$m] ?? null;
                $line[] = match (true) {
                    $v === null => '-',
                    $m === 'bytes' => Pretty::bytes((int) $v),
                    $m === 'avg_duration' => Pretty::duration((int) $v),
                    str_ends_with($m, '_rate') => number_format((float) $v * 100, 1) . '%',
                    default => number_format((float) $v),
                };
            }
            $line[] = number_format((float) $row['share'] * 100, 1) . '%';
            $rows[] = $line;
        }
        $head = "$domain  " . Dimension::label($dim) . "  {$r->from} → {$r->to}\n\n";
        if ($rows === []) {
            return $head . '(no rows)';
        }
        return $head . Cli::table(array_merge([$dim], $metrics, ['share']), $rows)
            . ($rep['has_more'] ? "\n… " . ($rep['total']['rows'] - count($rows)) . ' more rows (use --limit)' : '');
    }
}
