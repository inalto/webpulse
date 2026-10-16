#!/bin/bash
#WHMADDON:webpulse:WebPulse Analytics:webpulse.png
#ACLS:all
#
# WebPulse Analytics - JSON API for the WHM dashboard (runs as root under cpsrvd).
# Thin shim: everything is done by `webpulse cgi --whm --api` (principal from REMOTE_USER,
# POST + X-WebPulse-Request guard for mutations).
#
# Binary: $WEBPULSE_BIN, else /usr/local/webpulse/bin/webpulse, else bin/webpulse
# next to this script (tests).

bin=${WEBPULSE_BIN:-}
if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    bin=/usr/local/webpulse/bin/webpulse
    if [ ! -x "$bin" ]; then
        bin="$(dirname -- "${BASH_SOURCE[0]:-$0}")/bin/webpulse"
    fi
fi

if [ ! -x "$bin" ]; then
    printf 'Status: 500 Internal Server Error\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\n\r\n'
    printf '{"ok":false,"error":{"code":"internal_error","message":"Internal error"}}'
    exit 0
fi

exec "$bin" cgi --whm --api
