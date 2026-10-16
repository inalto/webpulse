#!/bin/bash
# WebPulse - cPanel Standardized Hook: Stats::RunAll (stage: pre), runs as root.
#
# Drains every access log into the WebPulse databases right before cpanellogd
# starts its stats cycle (and truncates/archives the domlogs). Synchronous, with
# a 20 minute cap. Never blocks the stats run: always exits 0.
#
# Register:   /usr/local/cpanel/bin/manage_hooks add script <this file> --manual \
#                 --category Stats --event RunAll --stage pre

self_dir() {
    local src=${BASH_SOURCE[0]:-$0}
    # logical (non-resolved) path, so a registration through the
    # /usr/local/webpulse symlink survives updates
    (cd "$(dirname -- "$src")" 2>/dev/null && pwd) || dirname -- "$src"
}

HOOK_DIR=$(self_dir)
HOOK_PATH="$HOOK_DIR/$(basename -- "${BASH_SOURCE[0]:-$0}")"

if [ "${1:-}" = "--describe" ]; then
    esc=${HOOK_PATH//\\/\\\\}
    esc=${esc//\"/\\\"}
    printf '[{"category":"Stats","event":"RunAll","stage":"pre","hook":"%s","exectype":"script"}]\n' "$esc"
    exit 0
fi

# trusted PATH: owned by the user running the hook (root under cPanel) and not
# group/other-writable (symlinks followed).
trusted() {
    local st mode
    st=$(stat -L -c '%u %a' -- "$1" 2>/dev/null) || return 1
    [ "${st%% *}" = "$EUID" ] || return 1
    mode=${st##* }
    (( (8#$mode & 8#022) == 0 ))
}

# $WEBPULSE_HOME (tests, relocated installs) is honoured only when the directory, its
# bin/ and bin/webpulse all pass `trusted`; otherwise the install next to this hook is used.
DEFAULT_HOME=$(dirname -- "$HOOK_DIR")
if [ -n "${WEBPULSE_HOME:-}" ] && trusted "$WEBPULSE_HOME" && trusted "$WEBPULSE_HOME/bin" \
    && trusted "$WEBPULSE_HOME/bin/webpulse"; then
    :
else
    WEBPULSE_HOME=$DEFAULT_HOME
fi
BIN="$WEBPULSE_HOME/bin/webpulse"

if [ -x "$BIN" ]; then
    if command -v timeout >/dev/null 2>&1; then
        timeout 1200 "$BIN" collect --quiet </dev/null >/dev/null 2>&1 || true
    else
        "$BIN" collect --quiet </dev/null >/dev/null 2>&1 || true
    fi
fi

exit 0
