<?php

declare(strict_types=1);

namespace WebPulse\Cli;

/**
 * A `webpulse <name>` sub-command. The dispatcher maps <name> to the class
 * WebPulse\Cli\Command\<Studly>Command (e.g. geoip-update ->
 * GeoipUpdateCommand), instantiates it without arguments and calls run().
 */
interface CommandInterface
{
    /** One-line description for `webpulse --help`. */
    public static function summary(): string;

    /** Usage text for `webpulse <name> --help` (may span several lines). */
    public static function usage(): string;

    /**
     * Options accepted besides the global ones: name => Args::VALUE (requires
     * a value), Args::FLAG (boolean) or Args::OPTIONAL (optional value).
     *
     * @return array<string,string>
     */
    public static function options(): array;

    /**
     * Runs the command.
     *
     * @return int exit code: 0 ok, 1 error, 2 usage error
     * @throws UsageException for invalid arguments (exit code 2)
     */
    public function run(Cli $cli, Args $args): int;
}
