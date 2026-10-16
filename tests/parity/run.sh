#!/usr/bin/env bash
# Parity harness (docs/RUST-PORT.md): databases first (db.sh builds the two fake
# roots), then the JSON API on those roots (api.sh) when present.
set -uo pipefail
DIR=$(cd "$(dirname "$0")" && pwd)
"$DIR/db.sh"
db=$?
[ $db -eq 2 ] && exit 2
api=0
if [ -x "$DIR/api.sh" ] || [ -f "$DIR/api.sh" ]; then
    bash "$DIR/api.sh"
    api=$?
fi
[ $db -eq 0 ] && [ $api -eq 0 ]
