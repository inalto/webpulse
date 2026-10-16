#!/bin/bash
#WHMADDON:webpulse:WebPulse Analytics:webpulse.png
#ACLS:all
#
# WebPulse Analytics - WHM dashboard (runs as root under cpsrvd).
# Thin shim: everything is done by `webpulse cgi --whm --page`, which derives the
# principal (administrator or reseller) from REMOTE_USER.
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
    printf 'Status: 500 Internal Server Error\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\r\n'
    printf '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>WebPulse Analytics is not available</title></head>'
    printf '<body><h2>WebPulse Analytics is not available</h2><p>The dashboard could not be loaded. Is WebPulse installed?</p></body></html>'
    exit 0
fi

exec "$bin" cgi --whm --page --api-url api.cgi
