<?php

declare(strict_types=1);

namespace WebPulse\Cli\Command;

use WebPulse\Cli\Args;
use WebPulse\Cli\Cli;
use WebPulse\Cli\CommandInterface;

/**
 * webpulse hooked — alias of `webpulse collect --hooked`.
 */
final class HookedCommand implements CommandInterface
{
    /** {@inheritDoc} */
    public static function summary(): string
    {
        return 'drain the accounts flagged by the Stats::RunUser hook (= collect --hooked)';
    }

    /** {@inheritDoc} */
    public static function usage(): string
    {
        return "Usage: webpulse hooked [--job [ID]]\n\nSame as: webpulse collect --hooked";
    }

    /** {@inheritDoc} */
    public static function options(): array
    {
        return ['job' => Args::OPTIONAL];
    }

    /** {@inheritDoc} */
    public function run(Cli $cli, Args $args): int
    {
        return CollectCommand::hooked($cli, $args);
    }
}
