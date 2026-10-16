<?php

declare(strict_types=1);

use WebPulse\Reports\Range;
use WebPulse\Tests\T;

/*
 * WP2 — Reports\Range: shortcuts, month boundaries, timezone edges,
 * previous period and granularity.
 */

$tz = new DateTimeZone('Europe/Rome');
$at = static fn (string $s, string $zone = 'Europe/Rome') => new DateTimeImmutable($s, new DateTimeZone($zone));
$r = static fn (string $name, string $now, string $zone = 'Europe/Rome', ?string $first = null) =>
    Range::shortcut($name, new DateTimeZone($zone), new DateTimeImmutable($now, new DateTimeZone($zone)), $first);

return [
    'shortcuts on 2026-10-16' => function () use ($r) {
        $now = '2026-10-16 12:00';
        $exp = [
            'today' => ['2026-10-16', '2026-10-16', 1, 'hour'],
            'yesterday' => ['2026-10-15', '2026-10-15', 1, 'hour'],
            '7d' => ['2026-10-10', '2026-10-16', 7, 'day'],
            '30d' => ['2026-09-17', '2026-10-16', 30, 'day'],
            '90d' => ['2026-07-19', '2026-10-16', 90, 'day'],
            '12m' => ['2025-11-01', '2026-10-16', 350, 'week'],
            'month' => ['2026-10-01', '2026-10-16', 16, 'day'],
            'lastmonth' => ['2026-09-01', '2026-09-30', 30, 'day'],
            'year' => ['2026-01-01', '2026-10-16', 289, 'week'],
        ];
        foreach ($exp as $name => [$from, $to, $days, $g]) {
            $x = $r($name, $now);
            T::eq([$from, $to, $days, $g], [$x->from, $x->to, $x->days, $x->granularity], "range $name");
        }
    },
    'API.md example: 30d previous period' => function () use ($r) {
        $x = $r('30d', '2026-10-16 09:00');
        T::eq(['2026-08-18', '2026-09-16'], [$x->prevFrom, $x->prevTo]);
        T::eq(['from' => '2026-09-17', 'to' => '2026-10-16', 'days' => 30, 'granularity' => 'day',
            'prev' => ['from' => '2026-08-18', 'to' => '2026-09-16']], $x->toArray());
    },
    'all uses first day with data, today when none' => function () use ($r) {
        T::eq('2026-09-02', $r('all', '2026-10-16 10:00', 'Europe/Rome', '2026-09-02')->from);
        T::eq('2026-10-16', $r('all', '2026-10-16 10:00')->from);
        T::eq('2026-10-16', $r('all', '2026-10-16 10:00', 'Europe/Rome', 'garbage')->from);
    },
    'month boundaries' => function () use ($r) {
        $x = $r('lastmonth', '2026-03-01 00:30');
        T::eq(['2026-02-01', '2026-02-28', 28], [$x->from, $x->to, $x->days]);
        $x = $r('lastmonth', '2024-03-31 23:00');
        T::eq(['2024-02-01', '2024-02-29'], [$x->from, $x->to], 'leap year');
        $x = $r('lastmonth', '2026-01-15 12:00');
        T::eq(['2025-12-01', '2025-12-31'], [$x->from, $x->to], 'year rollover');
        $x = $r('month', '2026-01-31 12:00');
        T::eq(['2026-01-01', '2026-01-31', 31], [$x->from, $x->to, $x->days]);
        $x = $r('month', '2026-05-01 08:00');
        T::eq(['2026-05-01', '2026-05-01', 'hour'], [$x->from, $x->to, $x->granularity]);
        T::eq(['2026-04-30', '2026-04-30'], [$x->prevFrom, $x->prevTo]);
        $x = $r('12m', '2026-03-31 12:00');
        T::eq('2025-04-01', $x->from);
    },
    'timezone decides "today"' => function () {
        $utc = new DateTimeImmutable('2026-10-16 23:30', new DateTimeZone('UTC'));
        T::eq('2026-10-17', Range::shortcut('today', new DateTimeZone('Europe/Rome'), $utc)->from, 'Rome is already tomorrow');
        T::eq('2026-10-16', Range::shortcut('today', new DateTimeZone('America/New_York'), $utc)->from);
        T::eq('2026-10-17', Range::shortcut('today', new DateTimeZone('Pacific/Kiritimati'), $utc)->from);
        $early = new DateTimeImmutable('2026-10-16 00:30', new DateTimeZone('UTC'));
        T::eq('2026-10-15', Range::shortcut('yesterday', new DateTimeZone('UTC'), $early)->from);
        T::eq('2026-10-15', Range::shortcut('today', new DateTimeZone('America/Los_Angeles'), $early)->from);
    },
    'DST change keeps whole days' => function () {
        $tz = new DateTimeZone('Europe/Rome');
        $x = new Range('2026-10-20', '2026-10-30', $tz); // DST ends on 2026-10-25
        T::eq(11, $x->days);
        T::eq(11, count($x->dayList()));
        T::eq(['2026-10-09', '2026-10-19'], [$x->prevFrom, $x->prevTo]);
        $y = new Range('2026-03-28', '2026-03-30', $tz); // DST starts on 2026-03-29
        T::eq(['2026-03-28', '2026-03-29', '2026-03-30'], $y->dayList());
    },
    'granularity rule' => function () {
        T::eq('hour', Range::granularityFor(1));
        T::eq('hour', Range::granularityFor(2));
        T::eq('day', Range::granularityFor(3));
        T::eq('day', Range::granularityFor(120));
        T::eq('week', Range::granularityFor(121));
        T::eq('week', Range::granularityFor(400));
        T::eq('month', Range::granularityFor(401));
    },
    'labels and bucket index per granularity' => function () use ($tz) {
        $x = new Range('2026-10-15', '2026-10-16', $tz);
        $l = $x->labels();
        T::eq(48, count($l));
        T::eq('2026-10-15 00:00', $l[0]);
        T::eq('2026-10-16 23:00', $l[47]);
        $w = new Range('2026-06-03', '2026-10-16', $tz); // Wednesday start, 136 days → week
        T::eq('week', $w->granularity);
        $wl = $w->labels();
        T::eq('2026-06-03', $wl[0], 'first week clipped to the range start');
        T::eq('2026-06-08', $wl[1], 'then Mondays');
        $idx = $w->dayIndex();
        T::eq(0, $idx['2026-06-07']);
        T::eq(1, $idx['2026-06-08']);
        T::eq(count($wl) - 1, $idx['2026-10-16']);
        $m = new Range('2025-01-15', '2026-10-16', $tz);
        T::eq('month', $m->granularity);
        T::eq('2025-01', $m->labels()[0]);
        T::eq('2026-10', $m->labels()[21]);
    },
    'fromQuery: custom, default and validation' => function () use ($tz, $at) {
        $now = $at('2026-10-16 12:00');
        T::eq('30d', Range::fromQuery([], $tz, $now)->shortcut);
        $c = Range::fromQuery(['from' => '2026-02-01', 'to' => '2026-02-28', 'range' => '7d'], $tz, $now);
        T::eq(['2026-02-01', '2026-02-28', 'custom'], [$c->from, $c->to, $c->shortcut], 'from/to win over range');
        T::throws(InvalidArgumentException::class, fn () => Range::fromQuery(['range' => 'fortnight'], $tz, $now), 'Unknown range');
        T::throws(InvalidArgumentException::class, fn () => Range::fromQuery(['from' => '2026-02-01'], $tz, $now));
        T::throws(InvalidArgumentException::class, fn () => Range::fromQuery(['from' => '2026-02-30', 'to' => '2026-03-01'], $tz, $now));
        T::throws(InvalidArgumentException::class, fn () => Range::fromQuery(['from' => '2026-03-02', 'to' => '2026-03-01'], $tz, $now), 'after');
        T::throws(InvalidArgumentException::class, fn () => Range::fromQuery(['from' => "2026-03-01'--", 'to' => '2026-03-02'], $tz, $now));
        T::throws(InvalidArgumentException::class, fn () => Range::fromQuery(['from' => '1990-01-01', 'to' => '2026-03-02'], $tz, $now), 'too long');
    },
];
