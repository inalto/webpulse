#!/usr/bin/env bash
# Download the free DB-IP country lite database (current month, else the previous
# one) into tests/fixtures/geoip/dbip-country-lite.mmdb. Used by `make geoip-fixture`;
# tests that need it skip without it.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
dest="$root/tests/fixtures/geoip/dbip-country-lite.mmdb"
mkdir -p "$(dirname "$dest")"
command -v curl >/dev/null 2>&1 || { echo "geoip_fixture: curl is required" >&2; exit 1; }

this=$(date -u +%Y-%m)
prev=$(date -u -d "$(date -u +%Y-%m-01) -1 month" +%Y-%m 2>/dev/null || date -u -v-1m +%Y-%m)
for ym in "$this" "$prev"; do
    url="https://download.db-ip.com/free/dbip-country-lite-$ym.mmdb.gz"
    tmp=$(mktemp)
    if curl -fsSL --retry 2 --connect-timeout 15 -o "$tmp" "$url" && gzip -dc "$tmp" >"$dest.new" 2>/dev/null; then
        rm -f "$tmp"
        mv -f "$dest.new" "$dest"
        echo "downloaded $url -> $dest ($(wc -c <"$dest") bytes)"
        exit 0
    fi
    rm -f "$tmp" "$dest.new"
    echo "geoip_fixture: $url not available" >&2
done
exit 1
