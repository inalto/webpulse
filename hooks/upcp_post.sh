#!/bin/bash
# WebPulse - cPanel Standardized Hook: System::upcp (stage: post), runs as root.
#
# cPanel updates regenerate the Meridian theme's hub registry
# (_assets/_registry.js), which drops the WebPulse side-menu entry. This hook
# puts it back after every update. Never fails the update: always exits 0.
#
# Register:   /usr/local/cpanel/bin/manage_hooks add script <this file> --manual \
#                 --category System --event upcp --stage post

self_dir() {
    local src=${BASH_SOURCE[0]:-$0}
    (cd "$(dirname -- "$src")" 2>/dev/null && pwd) || dirname -- "$src"
}

HOOK_DIR=$(self_dir)
HOOK_PATH="$HOOK_DIR/$(basename -- "${BASH_SOURCE[0]:-$0}")"

if [ "${1:-}" = "--describe" ]; then
    esc=${HOOK_PATH//\\/\\\\}
    esc=${esc//\"/\\\"}
    printf '[{"category":"System","event":"upcp","stage":"post","hook":"%s","exectype":"script"}]\n' "$esc"
    exit 0
fi

trusted() {
    local st mode
    st=$(stat -L -c '%u %a' -- "$1" 2>/dev/null) || return 1
    [ "${st%% *}" = "$EUID" ] || return 1
    mode=${st##* }
    (( (8#$mode & 8#022) == 0 ))
}

DEFAULT_HOME=$(dirname -- "$HOOK_DIR")
if [ -n "${WEBPULSE_HOME:-}" ] && trusted "$WEBPULSE_HOME" && trusted "$WEBPULSE_HOME/bin" \
    && trusted "$WEBPULSE_HOME/bin/webpulse"; then
    :
else
    WEBPULSE_HOME=$DEFAULT_HOME
fi
BIN="$WEBPULSE_HOME/bin/webpulse"
THEME_ROOT="${WEBPULSE_THEME_ROOT:-/usr/local/cpanel/base/frontend}"
HUB_DIR="$THEME_ROOT/meridian/webpulse"
REGISTRY="$THEME_ROOT/meridian/_assets/_registry.js"

# drain stdin (hook payload) so cPanel never blocks on a full pipe
cat >/dev/null 2>&1 || true

if [ -x "$BIN" ] && [ -d "$THEME_ROOT/meridian" ] && [ -f "$REGISTRY" ]; then
    # the theme update may have replaced our hub files: refresh them from the install
    if [ -d "$WEBPULSE_HOME/cpanel/meridian" ]; then
        mkdir -p "$HUB_DIR" 2>/dev/null || true
        cp -f "$WEBPULSE_HOME"/cpanel/meridian/manifest.json "$WEBPULSE_HOME"/cpanel/meridian/remoteEntry.js "$HUB_DIR"/ 2>/dev/null || true
        chmod 644 "$HUB_DIR"/manifest.json "$HUB_DIR"/remoteEntry.js 2>/dev/null || true
    fi
    if [ -f "$HUB_DIR/manifest.json" ]; then
        "$BIN" meridian-registry add --registry "$REGISTRY" --manifest "$HUB_DIR/manifest.json" \
            --entry "$HUB_DIR/remoteEntry.js" --quiet </dev/null >/dev/null 2>&1 || true
    fi
fi

exit 0
