<?php

declare(strict_types=1);

namespace WebPulse\Log;

/**
 * One parsed access-log line (raw values, not yet classified).
 */
final class Hit
{
    /**
     * @param string      $ip       client address (%h), IPv4 or IPv6
     * @param int         $ts       unix timestamp of the request
     * @param int         $offset   UTC offset of the logged time, seconds
     * @param string      $method   upper-case method, '' when the request line is malformed
     * @param string      $target   raw request target (path?query or absolute URI), '' when malformed
     * @param string      $protocol "HTTP/1.1", "HTTP/2.0"… ('' when absent)
     * @param int         $status   HTTP status (0 when logged as "-")
     * @param int         $bytes    response size (%b, "-" = 0)
     * @param string|null $referrer Referer header (null when "-" or empty)
     * @param string      $ua       User-Agent ('' when "-")
     * @param string|null $vhost    leading %v field when present
     */
    public function __construct(
        public readonly string $ip,
        public readonly int $ts,
        public readonly int $offset,
        public readonly string $method,
        public readonly string $target,
        public readonly string $protocol,
        public readonly int $status,
        public readonly int $bytes,
        public readonly ?string $referrer,
        public readonly string $ua,
        public readonly ?string $vhost = null,
    ) {
    }

    /** False when the request line could not be split into method/target/protocol. */
    public function requestValid(): bool
    {
        return $this->method !== '';
    }
}
