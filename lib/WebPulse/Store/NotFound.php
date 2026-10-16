<?php

declare(strict_types=1);

namespace WebPulse\Store;

/** Thrown when a domain database does not exist (read-only open). */
final class NotFound extends \RuntimeException
{
}
