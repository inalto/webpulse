<?php

declare(strict_types=1);

namespace WebPulse\Util;

/**
 * Fast unix timestamp -> local (day, hour) conversion for one timezone.
 *
 * Results are cached per 15-minute bucket, which is exact for every real
 * timezone (all UTC offsets and DST transitions are multiples of 15 min).
 */
final class DayClock
{
    private \DateTimeZone $tz;
    /** @var array<int,array{0:string,1:int,2:int}> bucket => [day, hour, dayStartTs] */
    private array $cache = [];

    /** Invalid identifiers fall back to the PHP default timezone. */
    public function __construct(string $timezone)
    {
        try {
            $this->tz = new \DateTimeZone($timezone);
        } catch (\Exception) {
            $this->tz = new \DateTimeZone(date_default_timezone_get());
        }
    }

    /** Name of the timezone in use. */
    public function timezone(): string
    {
        return $this->tz->getName();
    }

    /**
     * @return array{0:string,1:int,2:int} [Y-m-d, hour 0-23, unix ts of local midnight]
     */
    public function at(int $ts): array
    {
        $b = intdiv($ts, 900) - ($ts < 0 && $ts % 900 !== 0 ? 1 : 0);
        if (isset($this->cache[$b])) {
            return $this->cache[$b];
        }
        if (count($this->cache) > 20000) {
            $this->cache = [];
        }
        $d = (new \DateTimeImmutable('@' . $ts))->setTimezone($this->tz);
        $day = $d->format('Y-m-d');
        $mid = (new \DateTimeImmutable($day . ' 00:00:00', $this->tz))->getTimestamp();
        return $this->cache[$b] = [$day, (int) $d->format('G'), $mid];
    }

    /** Local day of $ts. */
    public function day(int $ts): string
    {
        return $this->at($ts)[0];
    }

    /** The local day before $day (Y-m-d). */
    public static function prevDay(string $day): string
    {
        return gmdate('Y-m-d', (int) strtotime($day . ' 00:00:00 UTC') - 86400);
    }

    /** $day shifted by $n days (Y-m-d). */
    public static function addDays(string $day, int $n): string
    {
        return gmdate('Y-m-d', (int) strtotime($day . ' 00:00:00 UTC') + 86400 * $n);
    }
}
