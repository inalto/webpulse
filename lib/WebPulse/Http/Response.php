<?php

declare(strict_types=1);

namespace WebPulse\Http;

/**
 * A complete HTTP response. In CGI mode (WHM) `send()` prints the
 * `Status:` header line and all headers itself instead of using header().
 */
final class Response
{
    private const REASONS = [
        200 => 'OK', 204 => 'No Content', 303 => 'See Other', 400 => 'Bad Request', 403 => 'Forbidden', 404 => 'Not Found',
        405 => 'Method Not Allowed', 409 => 'Conflict', 413 => 'Content Too Large', 500 => 'Internal Server Error',
        503 => 'Service Unavailable',
    ];

    private bool $cgi = false;

    /**
     * @param array<string,string> $headers extra headers
     */
    public function __construct(
        public readonly int $status,
        public readonly string $contentType,
        public readonly string $body,
        public readonly array $headers = [],
    ) {
    }

    /**
     * `{"ok":true,"data":…,"meta":{…}}`.
     */
    public static function json(mixed $data, float $startedAt, int $status = 200): self
    {
        $payload = [
            'ok' => true,
            'data' => $data,
            'meta' => ['took_ms' => (int) round((microtime(true) - $startedAt) * 1000), 'generated_at' => date('c')],
        ];
        return new self($status, 'application/json; charset=utf-8', self::encode($payload), self::noStore());
    }

    /** `{"ok":false,"error":{"code":…,"message":…}}`. */
    public static function error(int $status, string $code, string $message): self
    {
        return new self(
            $status,
            'application/json; charset=utf-8',
            self::encode(['ok' => false, 'error' => ['code' => $code, 'message' => $message]]),
            self::noStore()
        );
    }

    /** @return array<string,string> */
    private static function noStore(): array
    {
        return ['Cache-Control' => 'no-store', 'X-Content-Type-Options' => 'nosniff'];
    }

    private static function encode(mixed $v): string
    {
        return json_encode($v, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE | JSON_PRESERVE_ZERO_FRACTION | JSON_THROW_ON_ERROR);
    }

    /** Enables CGI output (prints the `Status:` line). */
    public function cgi(bool $on): self
    {
        $this->cgi = $on;
        return $this;
    }

    public function isCgi(): bool
    {
        return $this->cgi;
    }

    /** All header lines, including Content-Type and Content-Length. */
    public function headerLines(): array
    {
        $lines = [];
        if ($this->cgi) {
            $lines[] = 'Status: ' . $this->status . ' ' . (self::REASONS[$this->status] ?? 'Unknown');
        }
        $lines[] = 'Content-Type: ' . $this->contentType;
        foreach ($this->headers as $k => $v) {
            $lines[] = $k . ': ' . str_replace(["\r", "\n"], '', $v);
        }
        $lines[] = 'Content-Length: ' . strlen($this->body);
        return $lines;
    }

    /** Emits headers and body. */
    public function send(): void
    {
        if ($this->cgi) {
            echo implode("\r\n", $this->headerLines()), "\r\n\r\n";
        } elseif (!headers_sent()) {
            http_response_code($this->status);
            foreach ($this->headerLines() as $line) {
                header($line);
            }
        }
        echo $this->body;
    }
}
