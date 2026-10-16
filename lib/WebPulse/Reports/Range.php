<?php

declare(strict_types=1);

namespace WebPulse\Reports;

use DateInterval;
use DateTimeImmutable;
use DateTimeZone;
use InvalidArgumentException;

/**
 * An inclusive range of local days in a domain's timezone, plus the
 * immediately preceding period of the same length and the bucket
 * granularity used for time series (see API.md "a=overview").
 */
final class Range
{
    /** Shortcuts accepted by the `range` query parameter. */
    public const SHORTCUTS = ['today', 'yesterday', '7d', '30d', '90d', '12m', 'month', 'lastmonth', 'year', 'all'];

    public const DEFAULT = '30d';

    /** Longest custom range accepted (days). */
    public const MAX_DAYS = 3700;

    public readonly int $days;
    public readonly string $granularity;
    public readonly string $prevFrom;
    public readonly string $prevTo;

    /**
     * @param string $from     first local day, YYYY-MM-DD
     * @param string $to       last local day (inclusive), YYYY-MM-DD
     * @param string $shortcut the shortcut that produced the range, or 'custom'
     */
    public function __construct(
        public readonly string $from,
        public readonly string $to,
        public readonly DateTimeZone $tz,
        public readonly string $shortcut = 'custom',
    ) {
        $a = self::date($from, $tz);
        $b = self::date($to, $tz);
        if ($a > $b) {
            throw new InvalidArgumentException('Range start is after its end');
        }
        $days = (int) $a->diff($b)->days + 1;
        if ($days > self::MAX_DAYS) {
            throw new InvalidArgumentException('Range is too long (max ' . self::MAX_DAYS . ' days)');
        }
        $this->days = $days;
        $this->granularity = self::granularityFor($days);
        $prevTo = $a->sub(new DateInterval('P1D'));
        $this->prevTo = $prevTo->format('Y-m-d');
        $this->prevFrom = $prevTo->sub(new DateInterval('P' . ($days - 1) . 'D'))->format('Y-m-d');
    }

    /**
     * Builds a range from request parameters (`range` shortcut or `from`/`to`).
     *
     * @param array<string,mixed> $q        query parameters
     * @param string|null         $firstDay first day with data (for `all`)
     * @throws InvalidArgumentException on malformed input
     */
    public static function fromQuery(array $q, DateTimeZone $tz, ?DateTimeImmutable $now = null, ?string $firstDay = null): self
    {
        $from = isset($q['from']) && is_string($q['from']) && $q['from'] !== '' ? $q['from'] : null;
        $to = isset($q['to']) && is_string($q['to']) && $q['to'] !== '' ? $q['to'] : null;
        if ($from !== null || $to !== null) {
            if ($from === null || $to === null) {
                throw new InvalidArgumentException('Both "from" and "to" are required for a custom range');
            }
            return new self($from, $to, $tz, 'custom');
        }
        $range = $q['range'] ?? self::DEFAULT;
        if (!is_string($range) || $range === '') {
            $range = self::DEFAULT;
        }
        return self::shortcut($range, $tz, $now, $firstDay);
    }

    /**
     * @throws InvalidArgumentException for unknown shortcuts
     */
    public static function shortcut(string $name, DateTimeZone $tz, ?DateTimeImmutable $now = null, ?string $firstDay = null): self
    {
        $now = ($now ?? new DateTimeImmutable('now'))->setTimezone($tz);
        $today = $now->setTime(0, 0);
        $d = static fn (DateTimeImmutable $x): string => $x->format('Y-m-d');
        $back = static fn (int $n): DateTimeImmutable => $today->sub(new DateInterval('P' . $n . 'D'));

        switch ($name) {
            case 'today':
                return new self($d($today), $d($today), $tz, $name);
            case 'yesterday':
                return new self($d($back(1)), $d($back(1)), $tz, $name);
            case '7d':
                return new self($d($back(6)), $d($today), $tz, $name);
            case '30d':
                return new self($d($back(29)), $d($today), $tz, $name);
            case '90d':
                return new self($d($back(89)), $d($today), $tz, $name);
            case '12m':
                $start = $today->modify('first day of this month')->sub(new DateInterval('P11M'));
                return new self($d($start), $d($today), $tz, $name);
            case 'month':
                return new self($d($today->modify('first day of this month')), $d($today), $tz, $name);
            case 'lastmonth':
                $first = $today->modify('first day of last month');
                return new self($d($first), $d($first->modify('last day of this month')), $tz, $name);
            case 'year':
                return new self($today->format('Y') . '-01-01', $d($today), $tz, $name);
            case 'all':
                $start = $firstDay !== null && self::isDate($firstDay) && $firstDay <= $d($today) ? $firstDay : $d($today);
                $minStart = $d($back(self::MAX_DAYS - 1));
                return new self(max($start, $minStart), $d($today), $tz, $name);
        }
        throw new InvalidArgumentException('Unknown range "' . $name . '"');
    }

    /** Granularity rule from API.md. */
    public static function granularityFor(int $days): string
    {
        if ($days <= 2) {
            return 'hour';
        }
        if ($days <= 120) {
            return 'day';
        }
        return $days <= 400 ? 'week' : 'month';
    }

    /** True for a syntactically and calendrically valid YYYY-MM-DD. */
    public static function isDate(string $s): bool
    {
        if (!preg_match('/^(\d{4})-(\d{2})-(\d{2})\z/', $s, $m)) {
            return false;
        }
        return checkdate((int) $m[2], (int) $m[3], (int) $m[1]);
    }

    /** The preceding period as its own Range. */
    public function previous(): self
    {
        return new self($this->prevFrom, $this->prevTo, $this->tz, 'custom');
    }

    /**
     * Every local day in the range, in order.
     *
     * @return list<string>
     */
    public function dayList(): array
    {
        return self::daysBetween($this->from, $this->to, $this->tz);
    }

    /**
     * @return list<string>
     */
    public static function daysBetween(string $from, string $to, DateTimeZone $tz): array
    {
        $out = [];
        $cur = self::date($from, $tz);
        $end = self::date($to, $tz);
        $one = new DateInterval('P1D');
        while ($cur <= $end) {
            $out[] = $cur->format('Y-m-d');
            $cur = $cur->add($one);
        }
        return $out;
    }

    /**
     * Bucket labels for a granularity: `YYYY-MM-DD HH:00` (hour), `YYYY-MM-DD`
     * (day), the first day of the ISO week clipped to the range (week),
     * `YYYY-MM` (month).
     *
     * @return list<string>
     */
    public function labels(?string $granularity = null): array
    {
        $g = $granularity ?? $this->granularity;
        if ($g === 'hour') {
            $out = [];
            foreach ($this->dayList() as $day) {
                for ($h = 0; $h < 24; $h++) {
                    $out[] = sprintf('%s %02d:00', $day, $h);
                }
            }
            return $out;
        }
        $labels = [];
        foreach ($this->dayList() as $day) {
            $labels[$this->bucketKey($day, $g)] = true;
        }
        return array_keys($labels);
    }

    /**
     * Map of local day → bucket index for day/week/month granularity.
     *
     * @return array<string,int>
     */
    public function dayIndex(?string $granularity = null): array
    {
        $g = $granularity ?? $this->granularity;
        if ($g === 'hour') {
            $g = 'day';
        }
        $map = [];
        $idx = [];
        foreach ($this->dayList() as $day) {
            $k = $this->bucketKey($day, $g);
            if (!isset($idx[$k])) {
                $idx[$k] = count($idx);
            }
            $map[$day] = $idx[$k];
        }
        return $map;
    }

    private function bucketKey(string $day, string $g): string
    {
        if ($g === 'month') {
            return substr($day, 0, 7);
        }
        if ($g === 'week') {
            $dt = self::date($day, $this->tz);
            $dow = (int) $dt->format('N');
            $monday = $dt->sub(new DateInterval('P' . ($dow - 1) . 'D'))->format('Y-m-d');
            return max($monday, $this->from);
        }
        return $day;
    }

    /**
     * @return array{from:string,to:string,days:int,granularity:string,prev:array{from:string,to:string}}
     */
    public function toArray(): array
    {
        return [
            'from' => $this->from,
            'to' => $this->to,
            'days' => $this->days,
            'granularity' => $this->granularity,
            'prev' => ['from' => $this->prevFrom, 'to' => $this->prevTo],
        ];
    }

    private static function date(string $s, DateTimeZone $tz): DateTimeImmutable
    {
        if (!self::isDate($s)) {
            throw new InvalidArgumentException('Invalid date "' . substr($s, 0, 20) . '" (expected YYYY-MM-DD)');
        }
        $dt = DateTimeImmutable::createFromFormat('!Y-m-d', $s, $tz);
        if ($dt === false) {
            throw new InvalidArgumentException('Invalid date "' . $s . '"');
        }
        return $dt;
    }
}
