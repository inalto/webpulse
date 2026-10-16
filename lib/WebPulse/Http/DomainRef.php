<?php

declare(strict_types=1);

namespace WebPulse\Http;

/**
 * A domain visible to a principal: owner account, kind, collection switch
 * and whether a database exists yet.
 */
final class DomainRef
{
    public function __construct(
        public readonly string $user,
        public readonly string $domain,
        public readonly string $kind,
        public readonly ?bool $enabled,
        public readonly bool $hasData,
        public readonly string $dbPath,
    ) {
    }

    public function dbBytes(): int
    {
        return $this->hasData ? (int) @filesize($this->dbPath) : 0;
    }
}
