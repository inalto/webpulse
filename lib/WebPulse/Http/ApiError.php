<?php

declare(strict_types=1);

namespace WebPulse\Http;

/**
 * An error that maps to a JSON error response
 * (`{"ok":false,"error":{"code":…,"message":…}}`).
 */
final class ApiError extends \RuntimeException
{
    public function __construct(public readonly int $status, public readonly string $errorCode, string $message)
    {
        parent::__construct($message, $status);
    }

    public static function notFound(string $message = 'Not found'): self
    {
        return new self(404, 'not_found', $message);
    }

    public static function forbidden(string $message = 'You are not allowed to do this'): self
    {
        return new self(403, 'forbidden', $message);
    }

    public static function badRequest(string $message): self
    {
        return new self(400, 'bad_request', $message);
    }
}
