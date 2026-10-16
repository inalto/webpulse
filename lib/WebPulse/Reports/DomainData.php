<?php

declare(strict_types=1);

namespace WebPulse\Reports;

use DateTimeImmutable;
use DateTimeZone;
use PDO;

/**
 * Read-side handle on one domain database: the PDO, its `meta` table, the
 * effective timezone and a clock. Every report class works on this so that
 * the reports do not depend on how the database was opened.
 */
final class DomainData
{
    public readonly DateTimeZone $tz;
    private readonly DateTimeImmutable $now;

    /**
     * @param array<string,mixed> $meta      contents of the `meta` table
     * @param string|null         $defaultTz timezone used when meta has none
     */
    public function __construct(
        public readonly PDO $pdo,
        public readonly array $meta,
        public readonly string $domain,
        ?string $defaultTz = null,
        public readonly int $rawRetentionDays = 30,
        ?DateTimeImmutable $now = null,
    ) {
        $this->tz = self::zone(is_string($meta['timezone'] ?? null) && $meta['timezone'] !== '' ? $meta['timezone'] : $defaultTz);
        $this->now = ($now ?? new DateTimeImmutable('now'))->setTimezone($this->tz);
    }

    /** Reads the meta table of a domain DB as an associative array. */
    public static function readMeta(PDO $pdo): array
    {
        $out = [];
        foreach ($pdo->query('SELECT key, value FROM meta')->fetchAll(PDO::FETCH_NUM) as [$k, $v]) {
            $out[(string) $k] = $v;
        }
        return $out;
    }

    private static function zone(?string $name): DateTimeZone
    {
        if ($name !== null && $name !== '') {
            try {
                return new DateTimeZone($name);
            } catch (\Exception) {
                // fall through to the server zone
            }
        }
        return new DateTimeZone(date_default_timezone_get());
    }

    public function now(): DateTimeImmutable
    {
        return $this->now;
    }

    public function today(): string
    {
        return $this->now->format('Y-m-d');
    }

    public function metaString(string $key): ?string
    {
        $v = $this->meta[$key] ?? null;
        return $v === null || $v === '' ? null : (string) $v;
    }

    /** First day with data: meta.first_day, else MIN(daily.day). */
    public function firstDay(): ?string
    {
        $v = $this->metaString('first_day');
        if ($v === null) {
            $v = $this->pdo->query('SELECT MIN(day) FROM daily')->fetchColumn();
            $v = is_string($v) ? $v : null;
        }
        return $v;
    }

    public function lastDay(): ?string
    {
        $v = $this->metaString('last_day');
        if ($v === null) {
            $v = $this->pdo->query('SELECT MAX(day) FROM daily')->fetchColumn();
            $v = is_string($v) ? $v : null;
        }
        return $v;
    }

    /** ISO-8601 with offset in the domain zone, from a unix ts or an ISO string. */
    public function iso(int|string|null $v): ?string
    {
        if ($v === null || $v === '') {
            return null;
        }
        if (is_int($v) || ctype_digit((string) $v)) {
            return (new DateTimeImmutable('@' . (int) $v))->setTimezone($this->tz)->format('c');
        }
        try {
            return (new DateTimeImmutable((string) $v))->setTimezone($this->tz)->format('c');
        } catch (\Exception) {
            return null;
        }
    }

    /** Runs a prepared statement and returns all rows (assoc). */
    public function rows(string $sql, array $params = []): array
    {
        $st = $this->pdo->prepare($sql);
        $st->execute($params);
        return $st->fetchAll(PDO::FETCH_ASSOC);
    }

    /** Runs a prepared statement and returns the first row (assoc) or []. */
    public function row(string $sql, array $params = []): array
    {
        $st = $this->pdo->prepare($sql);
        $st->execute($params);
        $r = $st->fetch(PDO::FETCH_ASSOC);
        return is_array($r) ? $r : [];
    }

    /** True when the table exists (old/partial DBs). */
    public function hasTable(string $name): bool
    {
        $st = $this->pdo->prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?");
        $st->execute([$name]);
        return $st->fetchColumn() !== false;
    }

    /** Meta block shared by overview and other reports. */
    public function metaBlock(): array
    {
        return [
            'domain' => $this->domain,
            'timezone' => $this->tz->getName(),
            'first_day' => $this->firstDay(),
            'last_day' => $this->lastDay(),
            'last_collect_at' => $this->iso($this->metaString('last_collect_at')),
            'tz_changed_at' => $this->iso($this->metaString('tz_changed_at')),
            'raw_retention_days' => $this->rawRetentionDays,
        ];
    }
}
