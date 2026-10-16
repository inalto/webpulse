<?php

declare(strict_types=1);

namespace WebPulse\Tests;

/** Thrown by a test to mark itself skipped (e.g. offline, missing fixture). */
final class Skip extends \RuntimeException
{
}
