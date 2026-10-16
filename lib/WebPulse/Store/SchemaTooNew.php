<?php

declare(strict_types=1);

namespace WebPulse\Store;

/**
 * Thrown by a read-only open when the database was written by a newer
 * WebPulse (schema_version greater than {@see Schema::VERSION}).
 */
final class SchemaTooNew extends \RuntimeException
{
}
