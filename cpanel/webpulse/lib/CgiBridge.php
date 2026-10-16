<?php

declare(strict_types=1);

namespace WebPulse\Cpanel;

/**
 * Runs the WebPulse binary as a CGI program on behalf of a LiveAPI page.
 *
 * All WebPulse logic lives in the binary (`webpulse cgi`). This class only:
 *   - locates the binary,
 *   - forwards the CGI variables through the environment and the request body
 *     through stdin (proc_open, no shell),
 *   - turns the `Status:` line and the headers the binary prints into header()
 *     calls and streams the body to the client.
 *
 * It never puts paths or process output into an error it reports to the client.
 */
final class CgiBridge
{
    public const DEFAULT_BIN = '/usr/local/webpulse/bin/webpulse';

    /** Largest request body forwarded (the binary enforces its own, smaller cap). */
    private const MAX_BODY = 2097152;

    /** Largest response header block accepted from the binary. */
    private const MAX_HEAD = 65536;

    /** CGI variables copied from the request environment to the binary. */
    private const FORWARD = ['REQUEST_METHOD', 'QUERY_STRING', 'CONTENT_TYPE', 'CONTENT_LENGTH',
        'HTTP_X_WEBPULSE_REQUEST', 'REMOTE_ADDR'];

    /**
     * Binary resolution: $WEBPULSE_BIN, /usr/local/webpulse/bin/webpulse, then a
     * `webpulse` binary next to the shim (bin/webpulse; used by tests).
     *
     * @return string|null path of an executable file, null when none is found
     */
    public static function binary(?string $shimDir = null): ?string
    {
        $candidates = [];
        $env = getenv('WEBPULSE_BIN');
        if (is_string($env) && $env !== '') {
            $candidates[] = $env;
        }
        $candidates[] = self::DEFAULT_BIN;
        if ($shimDir !== null) {
            $candidates[] = rtrim($shimDir, '/') . '/bin/webpulse';
        }
        foreach ($candidates as $path) {
            if (is_file($path) && is_executable($path)) {
                return $path;
            }
        }
        return null;
    }

    /** A CGI variable from the environment or, failing that, $_SERVER. */
    private static function var(string $name): ?string
    {
        $v = getenv($name);
        if (is_string($v) && $v !== '') {
            return $v;
        }
        $v = $_SERVER[$name] ?? null;
        return is_string($v) && $v !== '' ? $v : null;
    }

    /**
     * Environment of the child process: only the CGI variables, PATH and the
     * optional WEBPULSE_CONFIG passthrough.
     *
     * @return array<string,string>
     */
    public static function environment(string $user): array
    {
        $env = ['PATH' => '/usr/local/bin:/usr/bin:/bin', 'REQUEST_METHOD' => 'GET', 'REMOTE_USER' => $user];
        foreach (self::FORWARD as $name) {
            $v = self::var($name);
            if ($v !== null) {
                $env[$name] = $v;
            }
        }
        $cfg = getenv('WEBPULSE_CONFIG');
        if (is_string($cfg) && $cfg !== '') {
            $env['WEBPULSE_CONFIG'] = $cfg;
        }
        return $env;
    }

    /** The request body (POST/PUT…), capped; '' when there is none. */
    public static function requestBody(): string
    {
        $method = strtoupper(self::var('REQUEST_METHOD') ?? 'GET');
        if ($method === 'GET' || $method === 'HEAD') {
            return '';
        }
        $body = '';
        $in = @fopen('php://input', 'rb');
        if ($in !== false) {
            $body = (string) stream_get_contents($in, self::MAX_BODY);
            fclose($in);
        }
        if ($body === '' && (int) (self::var('CONTENT_LENGTH') ?? 0) > 0) {
            $body = (string) stream_get_contents(STDIN, min((int) self::var('CONTENT_LENGTH'), self::MAX_BODY));
        }
        return $body;
    }

    /**
     * Split a CGI header block into a status code and a list of header lines.
     * Anything that is not a plain `Name: value` line is dropped (no header
     * injection from a misbehaving child).
     *
     * @return array{0:int,1:list<string>}
     */
    public static function parseHead(string $head): array
    {
        $status = 200;
        $lines = [];
        foreach (preg_split('/\r?\n/', $head) ?: [] as $line) {
            if ($line === '') {
                continue;
            }
            if (preg_match('/^Status:\s*(\d{3})\b/i', $line, $m) === 1) {
                $status = (int) $m[1];
                continue;
            }
            if (preg_match('/^[A-Za-z0-9-]{1,64}:[ \t]*[^\x00-\x08\x0a-\x1f\x7f]*$/', $line) === 1) {
                $lines[] = $line;
            }
        }
        return [$status, $lines];
    }

    /**
     * Run the binary and send its response.
     *
     * @param list<string> $args       arguments after the binary path (e.g. ['cgi', '--principal', 'user:alice', '--api'])
     * @param bool         $sendHeaders false = body only (the cPanel chrome has already started the page)
     * @return bool true when a response was produced; false when nothing was sent
     *              (binary missing, cannot start, empty output) so the caller prints its error
     */
    public static function run(array $args, string $user, bool $sendHeaders, ?string $shimDir = null): bool
    {
        if (!function_exists('proc_open')) {
            error_log('webpulse shim: proc_open is disabled in this PHP');
            return false;
        }
        $bin = self::binary($shimDir);
        if ($bin === null) {
            error_log('webpulse shim: binary not found');
            return false;
        }
        $body = self::requestBody();
        $env = self::environment($user);
        if ($body !== '') {
            $env['CONTENT_LENGTH'] = (string) strlen($body);
        }

        $proc = @proc_open(array_merge([$bin], $args), [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['file', '/dev/null', 'a']], $pipes, null, $env);
        if (!is_resource($proc)) {
            error_log('webpulse shim: cannot start the binary');
            return false;
        }
        stream_set_blocking($pipes[0], false);
        stream_set_blocking($pipes[1], false);

        $pending = $body;
        if ($pending === '') {
            fclose($pipes[0]);
            $pipes[0] = null;
        }
        $buffer = '';
        $inBody = false;
        $sent = false;
        $status = 200;
        $deadline = time() + 300;

        while (true) {
            $read = [$pipes[1]];
            $write = $pipes[0] !== null ? [$pipes[0]] : [];
            $except = null;
            if (@stream_select($read, $write, $except, 1) === false) {
                break;
            }
            if ($write !== [] && $pipes[0] !== null) {
                $n = @fwrite($pipes[0], substr($pending, 0, 8192));
                if ($n === false || $n === 0) {
                    // the child closed stdin (or is full): stop feeding it
                    if ($n === false) {
                        fclose($pipes[0]);
                        $pipes[0] = null;
                    }
                } else {
                    $pending = (string) substr($pending, $n);
                    if ($pending === '') {
                        fclose($pipes[0]);
                        $pipes[0] = null;
                    }
                }
            }
            if ($read !== []) {
                $chunk = fread($pipes[1], 65536);
                if ($chunk !== false && $chunk !== '') {
                    if ($inBody) {
                        print $chunk;
                        $sent = true;
                    } else {
                        $buffer .= $chunk;
                        $split = self::headEnd($buffer);
                        if ($split !== null) {
                            [$headEnd, $bodyStart] = $split;
                            [$status, $headers] = self::parseHead(substr($buffer, 0, $headEnd));
                            if ($sendHeaders) {
                                http_response_code($status);
                                foreach ($headers as $h) {
                                    header($h, true);
                                }
                            }
                            $inBody = true;
                            $first = (string) substr($buffer, $bodyStart);
                            $buffer = '';
                            if ($first !== '') {
                                print $first;
                                $sent = true;
                            }
                        } elseif (strlen($buffer) > self::MAX_HEAD) {
                            break;
                        }
                    }
                }
            }
            if (feof($pipes[1])) {
                break;
            }
            if (time() > $deadline) {
                break;
            }
        }

        if ($pipes[0] !== null) {
            fclose($pipes[0]);
        }
        fclose($pipes[1]);
        $code = proc_close($proc);

        if (!$inBody) {
            // no header terminator at all: nothing usable was produced
            if ($buffer !== '') {
                error_log('webpulse shim: malformed response from the binary');
            }
            return false;
        }
        if ($code !== 0 && !$sent) {
            error_log('webpulse shim: the binary exited with status ' . $code);
        }
        return true;
    }

    /** @return array{0:int,1:int}|null [end of the header block, start of the body] */
    private static function headEnd(string $buffer): ?array
    {
        $crlf = strpos($buffer, "\r\n\r\n");
        $lf = strpos($buffer, "\n\n");
        if ($crlf !== false && ($lf === false || $crlf <= $lf)) {
            return [$crlf, $crlf + 4];
        }
        if ($lf !== false) {
            return [$lf, $lf + 2];
        }
        return null;
    }
}
