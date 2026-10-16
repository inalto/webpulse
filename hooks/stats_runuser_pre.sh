#!/bin/bash
# WebPulse - cPanel Standardized Hook: Stats::RunUser (stage: pre), runs as the cPanel user.
#
# Requests a drain of this user's logs before cpanellogd truncates them. The
# hook runs unprivileged and cannot write the root-owned databases, so it only
# touches a flag file <run_dir>/hook/<user> (a world-writable sticky directory
# created by the installer); the every-minute cron job "collect --hooked" picks
# it up. Never blocks the stats run: always exits 0.
#
# Payload on STDIN: {"context":{...},"data":{"user":"alice", ...}}
#
# Register:   /usr/local/cpanel/bin/manage_hooks add script <this file> --manual \
#                 --category Stats --event RunUser --stage pre

self_dir() {
    local src=${BASH_SOURCE[0]:-$0}
    (cd "$(dirname -- "$src")" 2>/dev/null && pwd) || dirname -- "$src"
}

HOOK_DIR=$(self_dir)
HOOK_PATH="$HOOK_DIR/$(basename -- "${BASH_SOURCE[0]:-$0}")"

if [ "${1:-}" = "--describe" ]; then
    esc=${HOOK_PATH//\\/\\\\}
    esc=${esc//\"/\\\"}
    printf '[{"category":"Stats","event":"RunUser","stage":"pre","hook":"%s","exectype":"script"}]\n' "$esc"
    exit 0
fi

FLAG_DIR=${WEBPULSE_HOOK_DIR:-/var/webpulse/run/hook}

PHP=${WEBPULSE_PHP:-/usr/local/cpanel/3rdparty/bin/php}
if [ ! -x "$PHP" ]; then
    PHP=$(command -v php 2>/dev/null || true)
fi
[ -n "$PHP" ] && [ -x "$PHP" ] || exit 0

# shellcheck disable=SC2016  # the PHP code is deliberately single-quoted
"$PHP" -r '
$flagDir = $argv[1] ?? "";
$raw = @file_get_contents("php://stdin");
$json = is_string($raw) ? json_decode($raw, true) : null;
$user = is_array($json) && isset($json["data"]) && is_array($json["data"]) ? ($json["data"]["user"] ?? null) : null;
if ($flagDir === "" || !is_string($user) || preg_match("/^[a-z0-9_][a-z0-9_-]{0,31}\\z/", $user) !== 1) {
    exit(0);
}
$file = rtrim($flagDir, "/") . "/" . $user;
if (is_dir($flagDir) && !is_link($file)) {
    @touch($file);
}
' -- "$FLAG_DIR" >/dev/null 2>&1 || true

exit 0
