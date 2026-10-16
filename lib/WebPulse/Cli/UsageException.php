<?php

declare(strict_types=1);

namespace WebPulse\Cli;

/** Invalid command line; the dispatcher prints usage and exits with 2. */
final class UsageException extends \RuntimeException
{
}
