<?php

declare(strict_types=1);

namespace WebPulse\Http;

/**
 * An HTTP request as seen by the API: method, query parameters, raw body,
 * lower-cased headers and the client address.
 */
final class Request
{
    /** @var array<string,string> lower-case header name => value */
    public readonly array $headers;

    /**
     * @param array<string,mixed>  $query
     * @param array<string,string> $headers any case; normalised to lower case
     */
    public function __construct(
        public readonly string $method,
        public readonly array $query,
        public readonly ?string $body,
        array $headers,
        public readonly string $remoteAddr,
    ) {
        $h = [];
        foreach ($headers as $k => $v) {
            $h[strtolower(str_replace('_', '-', (string) $k))] = (string) $v;
        }
        $this->headers = $h;
    }

    /** Builds the request from PHP's superglobals (CGI, LiveAPI, php -S). */
    public static function fromGlobals(): self
    {
        $headers = [];
        foreach ($_SERVER as $k => $v) {
            if (is_string($k) && str_starts_with($k, 'HTTP_') && is_scalar($v)) {
                $headers[substr($k, 5)] = (string) $v;
            }
        }
        if (isset($_SERVER['CONTENT_TYPE'])) {
            $headers['CONTENT-TYPE'] = (string) $_SERVER['CONTENT_TYPE'];
        }
        $method = strtoupper((string) ($_SERVER['REQUEST_METHOD'] ?? getenv('REQUEST_METHOD') ?: 'GET'));
        $body = null;
        if ($method !== 'GET' && $method !== 'HEAD') {
            $raw = file_get_contents('php://input', false, null, 0, 1048576);
            $body = $raw === false ? null : $raw;
        }
        $query = $_GET;
        if ($query === [] && is_string($qs = $_SERVER['QUERY_STRING'] ?? getenv('QUERY_STRING')) && $qs !== '') {
            parse_str($qs, $query);
        }
        return new self($method, $query, $body, $headers, (string) ($_SERVER['REMOTE_ADDR'] ?? getenv('REMOTE_ADDR') ?: ''));
    }

    /** A scalar query parameter as string, or $default. */
    public function get(string $name, ?string $default = null): ?string
    {
        $v = $this->query[$name] ?? null;
        return is_scalar($v) ? (string) $v : $default;
    }

    public function header(string $name): ?string
    {
        return $this->headers[strtolower($name)] ?? null;
    }

    /**
     * The JSON body as an object/array.
     *
     * @return array<string,mixed>
     * @throws ApiError 400 on invalid JSON
     */
    public function json(): array
    {
        if ($this->body === null || trim($this->body) === '') {
            return [];
        }
        if (strlen($this->body) > 1048576) {
            throw new ApiError(400, 'bad_request', 'Request body too large');
        }
        try {
            $v = json_decode($this->body, true, 32, JSON_THROW_ON_ERROR);
        } catch (\JsonException) {
            throw new ApiError(400, 'bad_request', 'Request body is not valid JSON');
        }
        if (!is_array($v)) {
            throw new ApiError(400, 'bad_request', 'Request body must be a JSON object');
        }
        return $v;
    }
}
