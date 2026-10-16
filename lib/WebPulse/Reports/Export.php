<?php

declare(strict_types=1);

namespace WebPulse\Reports;

/**
 * `a=export&format=csv`: the rows of a report as RFC 4180 CSV. Cells that a
 * spreadsheet would interpret as formulas are prefixed with `'`.
 */
final class Export
{
    /**
     * @param array<string,mixed> $report output of Report::build()
     */
    public static function csv(array $report): string
    {
        $dim = (string) $report['name'];
        $cols = ['key'];
        if (Pretty::hasLabels($dim)) {
            $cols[] = 'label';
        }
        $metrics = Dimension::metrics($dim);
        if (Dimension::hasBounces($dim)) {
            $metrics = array_values(array_unique(array_merge($metrics, ['bounces', 'duration'])));
        }
        $cols = array_merge($cols, $metrics, ['share']);

        $fh = fopen('php://temp', 'w+');
        fputcsv($fh, $cols, ',', '"', '');
        foreach ($report['rows'] as $row) {
            $line = [];
            foreach ($cols as $c) {
                $v = $row[$c] ?? '';
                $line[] = is_string($v) ? self::safe($v) : (is_float($v) ? rtrim(rtrim(sprintf('%.4f', $v), '0'), '.') : (string) $v);
            }
            fputcsv($fh, $line, ',', '"', '');
        }
        rewind($fh);
        $csv = (string) stream_get_contents($fh);
        fclose($fh);
        return $csv;
    }

    /** `example.com-page-2026-09-17_2026-10-16.csv` */
    public static function filename(string $domain, string $dim, Range $r): string
    {
        $safe = preg_replace('/[^a-z0-9.-]/', '_', strtolower($domain));
        return $safe . '-' . $dim . '-' . $r->from . '_' . $r->to . '.csv';
    }

    /** Neutralises spreadsheet formula injection. */
    public static function safe(string $v): string
    {
        return $v !== '' && strpbrk($v[0], "=+-@\t\r") !== false ? "'" . $v : $v;
    }
}
