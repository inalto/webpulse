#!/usr/bin/env bash
# WebPulse installer for cPanel & WHM.
#
#   curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --geoip
#   bash install.sh [options]            (from a checkout or an unpacked release)
#
# Run `install.sh --help` for the options. The heavy lifting lives in
# installer/lib.sh; this file only parses flags, obtains the sources
# (current checkout or GitHub tarball) and calls the library.
# shellcheck disable=SC2034  # WP_* variables are consumed by installer/lib.sh
set -euo pipefail

usage() {
    cat <<'USAGE'
WebPulse installer for cPanel & WHM (run as root)

Usage: install.sh [options]

  --geoip                 enable GeoIP and download the free DB-IP country database
  --ref <branch|tag>      install this git ref              (env WEBPULSE_REF, default: main)
  --repo <owner/name>     install from this GitHub repo     (env WEBPULSE_REPO, default: inalto/webpulse)
  --no-cron               do not install /etc/cron.d/webpulse
  --update                update an existing installation in place (idempotent)
  --uninstall             remove plugins, hooks, cron and code; keeps /var/webpulse and /etc/webpulse
  --purge                 with --uninstall: also delete /var/webpulse (all statistics) and /etc/webpulse
  --standalone            no cPanel: install only the CLI and the dev server
  --all-themes            also install the cPanel plugin into every theme under base/frontend/
  --theme <name>          also install the cPanel plugin into this theme (repeatable)
  -y, --yes               do not ask for confirmation (also implied when stdin is not a terminal)
  -h, --help              show this help

Environment overrides (testing / unusual layouts):
  WEBPULSE_PREFIX, WEBPULSE_FAKE_CPANEL=1, WEBPULSE_CPANEL_ROOT, WEBPULSE_ETC,
  WEBPULSE_VAR, WEBPULSE_CRON_DIR, WEBPULSE_APPS_DIR, WEBPULSE_DOWNLOAD_URL,
  WEBPULSE_SKIP_FIRST_COLLECT=1, WEBPULSE_VERBOSE=1
USAGE
}

die_usage() {
    printf 'install.sh: %s\n\nTry: install.sh --help\n' "$*" >&2
    exit 2
}

main() {
    local t
    WP_REPO="${WEBPULSE_REPO:-inalto/webpulse}"
    WP_REF="${WEBPULSE_REF:-main}"
    WP_GEOIP="${WEBPULSE_GEOIP:-0}"
    WP_NO_CRON="${WEBPULSE_NO_CRON:-0}"
    WP_YES="${WEBPULSE_YES:-0}"
    WP_UPDATE=0
    WP_UNINSTALL=0
    WP_PURGE=0
    WP_STANDALONE=0
    WP_ALL_THEMES=0
    WP_EXTRA_THEMES=()

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --geoip) WP_GEOIP=1 ;;
            --no-cron) WP_NO_CRON=1 ;;
            --update) WP_UPDATE=1 ;;
            --uninstall) WP_UNINSTALL=1 ;;
            --purge) WP_PURGE=1 ;;
            --standalone) WP_STANDALONE=1 ;;
            --all-themes) WP_ALL_THEMES=1 ;;
            -y|--yes) WP_YES=1 ;;
            -h|--help) usage; exit 0 ;;
            --ref) [ "$#" -ge 2 ] || die_usage "--ref needs a value"; WP_REF=$2; shift ;;
            --ref=*) WP_REF=${1#--ref=} ;;
            --repo) [ "$#" -ge 2 ] || die_usage "--repo needs a value"; WP_REPO=$2; shift ;;
            --repo=*) WP_REPO=${1#--repo=} ;;
            --theme) [ "$#" -ge 2 ] || die_usage "--theme needs a value"; WP_EXTRA_THEMES+=("$2"); shift ;;
            --theme=*) WP_EXTRA_THEMES+=("${1#--theme=}") ;;
            *) die_usage "unknown option: $1" ;;
        esac
        shift
    done

    case "$WP_REPO" in
        */*/*|*[!A-Za-z0-9._/-]*|/*|*/|'') die_usage "invalid --repo '$WP_REPO' (expected owner/name)" ;;
        */*) ;;
        *) die_usage "invalid --repo '$WP_REPO' (expected owner/name)" ;;
    esac
    case "$WP_REF" in
        ''|-*|*[!A-Za-z0-9._/+-]*|*..*) die_usage "invalid --ref '$WP_REF'" ;;
    esac
    for t in ${WP_EXTRA_THEMES[@]+"${WP_EXTRA_THEMES[@]}"}; do
        case "$t" in ''|*[!A-Za-z0-9._-]*|.*) die_usage "invalid --theme '$t'" ;; esac
    done
    if [ "$WP_PURGE" = 1 ] && [ "$WP_UNINSTALL" = 0 ]; then
        die_usage "--purge only makes sense together with --uninstall"
    fi

    # ---- where do we run from? ------------------------------------------
    local self_dir='' prefix="${WEBPULSE_PREFIX:-}"
    prefix="${prefix%/}"
    local installed="$prefix/usr/local/webpulse"
    if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
        self_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd -P) || self_dir=''
    fi

    SRC=''
    WP_SOURCE_DESC=''
    TMP_DIR=''
    trap 'wp_cleanup' EXIT

    local use_checkout=0
    if [ -n "$self_dir" ] && [ -f "$self_dir/installer/lib.sh" ] \
        && { [ -f "$self_dir/bin/webpulse" ] || [ "$WP_UNINSTALL" = 1 ]; }; then
        use_checkout=1
        # an installed copy updating "from itself" would reinstall the same code: fetch instead
        if [ "$WP_UNINSTALL" = 0 ]; then
            local real_installed='' real_releases=''
            real_installed=$(cd "$installed" 2>/dev/null && pwd -P) || real_installed=''
            real_releases=$(cd "$prefix/usr/local/webpulse-releases" 2>/dev/null && pwd -P) || real_releases=''
            if { [ -n "$real_installed" ] && [ "$self_dir" = "$real_installed" ]; } \
                || { [ -n "$real_releases" ] && [ "${self_dir#"$real_releases"/}" != "$self_dir" ]; }; then
                use_checkout=0
            fi
        fi
    fi

    if [ "$use_checkout" = 1 ]; then
        SRC=$self_dir
        WP_SOURCE_DESC="local checkout $SRC"
    elif [ "$WP_UNINSTALL" = 1 ] && [ -f "$installed/installer/lib.sh" ]; then
        SRC=$(cd "$installed" && pwd -P)
        WP_SOURCE_DESC="installed copy $SRC"
    else
        fetch_source
    fi

    # shellcheck disable=SC1091
    . "$SRC/installer/lib.sh"

    if [ "$WP_UNINSTALL" = 1 ]; then
        wp_uninstall
    else
        trap 'wp_on_exit $?' EXIT
        wp_install
    fi
}

# Download the GitHub tarball of ${WP_REPO}@${WP_REF} into a temp dir and set SRC.
fetch_source() {
    local url="${WEBPULSE_DOWNLOAD_URL:-https://codeload.github.com/${WP_REPO}/tar.gz/${WP_REF}}"
    TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/webpulse-install.XXXXXX")
    printf 'Downloading %s\n' "$url"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 3 --connect-timeout 15 -o "$TMP_DIR/webpulse.tar.gz" "$url" \
            || { printf 'Download failed: %s\n' "$url" >&2; exit 1; }
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$TMP_DIR/webpulse.tar.gz" "$url" \
            || { printf 'Download failed: %s\n' "$url" >&2; exit 1; }
    else
        printf 'Neither curl nor wget is available.\n' >&2
        exit 1
    fi
    mkdir -p "$TMP_DIR/src"
    tar -xzf "$TMP_DIR/webpulse.tar.gz" -C "$TMP_DIR/src" --strip-components=1 --no-same-owner \
        || { printf 'The downloaded file is not a valid tar.gz archive.\n' >&2; exit 1; }
    if [ ! -f "$TMP_DIR/src/bin/webpulse" ] || [ ! -f "$TMP_DIR/src/installer/lib.sh" ]; then
        printf 'The archive does not look like WebPulse (bin/webpulse or installer/lib.sh missing).\n' >&2
        exit 1
    fi
    SRC="$TMP_DIR/src"
    WP_SOURCE_DESC="$url"
}

wp_cleanup() {
    if [ -n "${TMP_DIR:-}" ] && [ -d "$TMP_DIR" ]; then
        rm -rf "$TMP_DIR"
    fi
}

# EXIT hook for installs: any unexpected failure rolls back what this run did.
wp_on_exit() {
    local rc=$1
    if [ "$rc" -ne 0 ]; then
        wp_rollback
    fi
    wp_cleanup
    return 0
}

# Everything is wrapped so that a truncated `curl | bash` never runs half a script.
main "$@"
