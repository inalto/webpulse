<?php

declare(strict_types=1);

namespace WebPulse\Cli;

/**
 * Bookkeeping of `registry.jobs` for long-running commands.
 *
 * `--job`        create a job row now (status running) and finish it at the end;
 * `--job <id>`   adopt a row created beforehand by the WHM UI (Registry::startJob)
 *                and finish it at the end;
 * no `--job`     the row is written at the end only when $record says so.
 */
final class Job
{
    private ?int $id = null;
    private int $startedAt;

    /** Creates or adopts the job row when --job is present. */
    public function __construct(private Cli $cli, private string $type, private ?string $scope, Args $args)
    {
        $this->startedAt = $cli->app()->now();
        if (!$args->has('job')) {
            return;
        }
        $reg = $cli->app()->registry();
        $v = $args->opt('job');
        if ($v !== null && ctype_digit($v) && $reg->job((int) $v) !== null) {
            $this->id = (int) $v;
            $reg->touchJob($this->id);
        } else {
            $this->id = $reg->startJob($type, $scope);
        }
    }

    /** Job id (null when no row was written yet). */
    public function id(): ?int
    {
        return $this->id;
    }

    /**
     * @param array{lines?:int,files?:int,domains?:int} $counters
     * @param bool $record write a row even without --job
     */
    public function finish(string $status, array $counters, ?string $message = null, bool $record = false): void
    {
        try {
            $reg = $this->cli->app()->registry();
            if ($this->id === null) {
                if (!$record) {
                    return;
                }
                $this->id = $reg->startJob($this->type, $this->scope, $this->startedAt);
            }
            $reg->finishJob($this->id, $status, $counters, $message);
        } catch (\Throwable $e) {
            $this->cli->app()->log->warn('cannot record job: ' . $e->getMessage());
        }
    }
}
