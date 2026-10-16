<?php

declare(strict_types=1);

namespace WebPulse\Collect;

/**
 * Mutable state of one open visit (see {@see Sessionizer}).
 */
final class Visit
{
    /** @var array<string,bool> distinct page keys seen (capped) */
    public array $pages = [];
    /** @var array<int,array{0:string,1:string}> visit-level dim rows */
    public array $vdims;
    public ?int $firstPv = null;
    public int $lastPv;
    public int $pv = 0;
    public int $hits = 0;
    public int $bytes = 0;
    public ?string $entry = null;
    public ?string $exit = null;
    public bool $dirty = true;
    /** pageviews / bounce / duration already written to the visit-level dims */
    public int $cPv = 0;
    public int $cBounce = 0;
    public int $cDuration = 0;

    /**
     * @param array<string,mixed> $attrs
     */
    public function __construct(
        public string $key,
        public int $id,
        public string $day,
        public int $start,
        public int $last,
        public array $attrs,
    ) {
        $this->lastPv = $start;
        $this->vdims = Sessionizer::visitDims($attrs);
    }

    /** Duration in seconds between the first and the last pageview (0 for bounces). */
    public function duration(): int
    {
        return $this->pv > 0 && $this->firstPv !== null ? $this->lastPv - $this->firstPv : 0;
    }
}
