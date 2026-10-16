#!/usr/bin/env bash
# Shim: uninstall.sh [--purge] [-y]  ==  install.sh --uninstall [--purge] [-y]
set -euo pipefail

here=''
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
fi

if [ -n "$here" ] && [ -f "$here/install.sh" ]; then
    exec bash "$here/install.sh" --uninstall "$@"
fi

repo="${WEBPULSE_REPO:-inalto/webpulse}"
ref="${WEBPULSE_REF:-main}"
curl -fsSL "https://raw.githubusercontent.com/${repo}/${ref}/install.sh" | bash -s -- --uninstall "$@"
