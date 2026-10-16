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

# Extract data.user from the JSON payload on STDIN with plain bash (no interpreter, no
# binary, no jq: this runs as the account on every cPanel box). Only a string that
# matches the cPanel user name grammar is accepted; anything else is ignored.
payload=$(head -c 65536 2>/dev/null | tr -d '\r\n') || payload=''

user=''
case $payload in
    *'"data"'*)
        rest=${payload#*'"data"'}
        if [[ $rest =~ ^[[:space:]]*:[[:space:]]*\{ ]]; then
            if [[ $rest == *'"user"'* ]]; then
                rest=${rest#*'"user"'}
                if [[ $rest =~ ^[[:space:]]*:[[:space:]]*\"([a-z0-9_][a-z0-9_-]{0,31})\" ]]; then
                    user=${BASH_REMATCH[1]}
                fi
            fi
        fi
        ;;
esac

if [ -n "$user" ] && [ -d "$FLAG_DIR" ]; then
    flag="${FLAG_DIR%/}/$user"
    if [ ! -L "$flag" ]; then
        touch -- "$flag" >/dev/null 2>&1 || true
    fi
fi

exit 0
