#!/usr/bin/env bash
# WebPulse installer for cPanel & WHM.
#
#   curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --geoip
#   bash install.sh [options]            (from a checkout or an unpacked release)
#
# Run `install.sh --help` for the options. WebPulse is one static binary: the
# installer downloads the release tarball for this machine's architecture
# (bin/webpulse + cpanel/ whm/ hooks/ etc/ installer/ ...), verifies its sha256,
# and hands over to installer/lib.sh, which does the real work. Without a
# prebuilt binary it can build from source (--build, needs cargo) or install a
# binary you provide (--binary FILE).
# shellcheck disable=SC2034  # WP_* variables are consumed by installer/lib.sh
set -euo pipefail

usage() {
    cat <<'USAGE'
WebPulse installer for cPanel & WHM (run as root)

Usage: install.sh [options]

  --geoip                 enable GeoIP and download the free DB-IP country database
  --ref <tag|branch>      release tag (v0.2.0) to install; main/unset = latest release
                          (env WEBPULSE_REF; a branch name builds from source, needs cargo)
  --repo <owner/name>     install from this GitHub repo     (env WEBPULSE_REPO, default: inalto/webpulse)
  --build                 build the binary from source with cargo instead of downloading it
  --binary <file>         install this WebPulse binary (air-gapped servers, custom builds)
  --no-cron               do not install /etc/cron.d/webpulse
  --update                update an existing installation in place (idempotent)
  --uninstall             remove plugins, hooks, cron and code; keeps /var/webpulse and /etc/webpulse
  --purge                 with --uninstall: also delete /var/webpulse (all statistics) and /etc/webpulse
  --standalone            no cPanel: install only the CLI and the dev server
  --all-themes            also install the cPanel plugin into every theme under base/frontend/
  --theme <name>          also install the cPanel plugin into this theme (repeatable)
  -y, --yes               do not ask for confirmation (also implied when stdin is not a terminal)
  -h, --help              show this help

Supported platforms for the prebuilt binary: Linux x86_64 and aarch64 (static musl builds).

Environment overrides (testing / unusual layouts):
  WEBPULSE_PREFIX, WEBPULSE_FAKE_CPANEL=1, WEBPULSE_CPANEL_ROOT, WEBPULSE_ETC,
  WEBPULSE_VAR, WEBPULSE_CRON_DIR, WEBPULSE_APPS_DIR, WEBPULSE_SKIP_FIRST_COLLECT=1,
  WEBPULSE_VERBOSE=1, WEBPULSE_BUILD=1, WEBPULSE_BINARY=<file>,
  WEBPULSE_DOWNLOAD_URL=<release tarball URL; its .sha256 is fetched next to it>,
  WEBPULSE_DOWNLOAD_SHA256=<expected hash>, WEBPULSE_SOURCE_URL=<source tarball URL>,
  WEBPULSE_TARGET=<rust target triple>, WEBPULSE_CARGO=<cargo binary>
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
    WP_BUILD="${WEBPULSE_BUILD:-0}"
    WP_BINARY="${WEBPULSE_BINARY:-}"
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
            --build) WP_BUILD=1 ;;
            -y|--yes) WP_YES=1 ;;
            -h|--help) usage; exit 0 ;;
            --ref) [ "$#" -ge 2 ] || die_usage "--ref needs a value"; WP_REF=$2; shift ;;
            --ref=*) WP_REF=${1#--ref=} ;;
            --repo) [ "$#" -ge 2 ] || die_usage "--repo needs a value"; WP_REPO=$2; shift ;;
            --repo=*) WP_REPO=${1#--repo=} ;;
            --binary) [ "$#" -ge 2 ] || die_usage "--binary needs a value"; WP_BINARY=$2; shift ;;
            --binary=*) WP_BINARY=${1#--binary=} ;;
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
    case "$WP_BUILD" in 0|1) ;; *) die_usage "invalid WEBPULSE_BUILD '$WP_BUILD' (0 or 1)" ;; esac
    if [ -n "$WP_BINARY" ]; then
        if [ "$WP_BUILD" = 1 ]; then
            die_usage "--build and --binary exclude each other"
        fi
        [ -f "$WP_BINARY" ] || die_usage "--binary: '$WP_BINARY' is not a file"
        [ -s "$WP_BINARY" ] || die_usage "--binary: '$WP_BINARY' is empty"
        WP_BINARY=$(cd "$(dirname -- "$WP_BINARY")" && pwd -P)/$(basename -- "$WP_BINARY")
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
    WP_BIN_SRC=''
    TMP_DIR=''
    trap 'wp_cleanup' EXIT

    # A usable local source is an unpacked release tarball (bin/webpulse next to
    # installer/lib.sh) or a source checkout (Cargo.toml + src/).
    local use_checkout=0 is_source=0
    if [ -n "$self_dir" ] && [ -f "$self_dir/installer/lib.sh" ] && [ -f "$self_dir/VERSION" ]; then
        if [ -f "$self_dir/Cargo.toml" ] && [ -d "$self_dir/src" ]; then
            is_source=1
        fi
        if [ "$WP_UNINSTALL" = 1 ] || [ "$is_source" = 1 ] || [ -f "$self_dir/bin/webpulse" ]; then
            use_checkout=1
        fi
        # --build of an unpacked release means "build the source of that ref"
        if [ "$WP_BUILD" = 1 ] && [ "$is_source" = 0 ] && [ "$WP_UNINSTALL" = 0 ]; then
            use_checkout=0
        fi
        # an installed copy updating "from itself" would reinstall the same code: fetch instead
        if [ "$use_checkout" = 1 ] && [ "$WP_UNINSTALL" = 0 ]; then
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
        if [ "$is_source" = 1 ]; then
            WP_SOURCE_DESC="source checkout $SRC"
        else
            WP_SOURCE_DESC="local release $SRC"
        fi
    elif [ "$WP_UNINSTALL" = 1 ] && [ -f "$installed/installer/lib.sh" ]; then
        SRC=$(cd "$installed" && pwd -P)
        WP_SOURCE_DESC="installed copy $SRC"
    elif [ "$WP_UNINSTALL" = 1 ]; then
        # nothing installed and no local copy to run from: the uninstaller library comes from the source tarball
        TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/webpulse-install.XXXXXX")
        fetch_source_tarball
    else
        fetch_source
    fi

    if [ "$WP_UNINSTALL" = 0 ]; then
        select_binary "$is_source"
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

# ---------------------------------------------------------------- helpers ---

fail() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

have() { command -v "$1" >/dev/null 2>&1; }

# http_get URL DEST : sets HTTP_CODE (curl only; file:// is copied). Returns 0 on success.
HTTP_CODE=''
http_get() {
    local url=$1 dest=$2
    HTTP_CODE=''
    case "$url" in
        file://*)
            if cp -- "${url#file://}" "$dest" 2>/dev/null; then HTTP_CODE=200; return 0; fi
            HTTP_CODE=404
            return 1
            ;;
    esac
    if have curl; then
        HTTP_CODE=$(curl -sSL --retry 3 --connect-timeout 15 -o "$dest" -w '%{http_code}' "$url" 2>/dev/null) || HTTP_CODE=${HTTP_CODE:-000}
        case "$HTTP_CODE" in 2??) return 0 ;; esac
        rm -f "$dest"
        return 1
    elif have wget; then
        if wget -q -O "$dest" "$url" 2>/dev/null; then HTTP_CODE=200; return 0; fi
        rm -f "$dest"
        HTTP_CODE=404
        return 1
    fi
    fail "Neither curl nor wget is available."
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    elif have openssl; then
        openssl dgst -sha256 "$1" | sed 's/^.*= *//'
    else
        fail "sha256sum (coreutils) is required to verify the download."
    fi
}

# Target triple of the prebuilt binary for this machine.
detect_target() {
    WP_TARGET=${WEBPULSE_TARGET:-}
    if [ -n "$WP_TARGET" ]; then
        case "$WP_TARGET" in *[!A-Za-z0-9._-]*) die_usage "invalid WEBPULSE_TARGET '$WP_TARGET'" ;; esac
        return 0
    fi
    local os arch
    os=$(uname -s 2>/dev/null || echo unknown)
    arch=$(uname -m 2>/dev/null || echo unknown)
    [ "$os" = Linux ] || fail "Unsupported operating system '$os': WebPulse needs Linux (cPanel & WHM)."
    case "$arch" in
        x86_64|amd64) WP_TARGET=x86_64-unknown-linux-musl ;;
        aarch64|arm64) WP_TARGET=aarch64-unknown-linux-musl ;;
        *) fail "Unsupported CPU architecture '$arch' (prebuilt binaries: x86_64, aarch64). Use --build with cargo, or --binary FILE." ;;
    esac
}

# Latest release tag of $WP_REPO (prints it, e.g. v0.2.0), empty on failure.
latest_tag() {
    local tmp="$TMP_DIR/latest.json" tag=''
    if http_get "${WEBPULSE_API_URL:-https://api.github.com/repos/${WP_REPO}/releases/latest}" "$tmp"; then
        tag=$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp" | head -n 1)
    fi
    if [ -z "$tag" ] && have curl; then
        # no API access (rate limit, proxy): the web redirect of /releases/latest names the tag
        tag=$(curl -sIL --connect-timeout 15 -o /dev/null -w '%{url_effective}' \
            "https://github.com/${WP_REPO}/releases/latest" 2>/dev/null || true)
        tag=${tag##*/tag/}
        case "$tag" in */*|*[!A-Za-z0-9._+-]*) tag='' ;; esac
    fi
    case "$tag" in v[0-9]*) printf '%s' "$tag" ;; esac
}

# Extract tarball $1 into $2 (stripping the top-level directory) and refuse odd archives.
extract_tarball() {
    local tarball=$1 dest=$2
    if tar -tzf "$tarball" 2>/dev/null | grep -Eq '(^|/)\.\.(/|$)|^/'; then
        fail "The archive contains unsafe paths; refusing to unpack it."
    fi
    mkdir -p "$dest"
    tar -xzf "$tarball" -C "$dest" --strip-components=1 --no-same-owner 2>/dev/null \
        || fail "The downloaded file is not a valid tar.gz archive."
    if [ ! -f "$dest/installer/lib.sh" ] || [ ! -f "$dest/VERSION" ]; then
        fail "The archive does not look like WebPulse (installer/lib.sh or VERSION missing)."
    fi
}

find_cargo() {
    WP_CARGO=''
    if [ -n "${WEBPULSE_CARGO:-}" ]; then
        if [ -x "$WEBPULSE_CARGO" ] || have "$WEBPULSE_CARGO"; then WP_CARGO=$WEBPULSE_CARGO; fi
    elif have cargo; then
        WP_CARGO=cargo
    elif [ -x "${HOME:-/root}/.cargo/bin/cargo" ]; then
        WP_CARGO="${HOME:-/root}/.cargo/bin/cargo"
    elif [ -x /root/.cargo/bin/cargo ]; then
        WP_CARGO=/root/.cargo/bin/cargo
    fi
    [ -n "$WP_CARGO" ]
}

# cargo build --release in directory $1; sets WP_BIN_SRC.
cargo_build() {
    local dir=$1 locked=()
    find_cargo || fail "cargo (the Rust toolchain) was not found, so WebPulse cannot be built from source.
  Install Rust (https://rustup.rs), or install a prebuilt binary:
    - normal install:  run the installer without --build (Linux x86_64 / aarch64 releases)
    - air-gapped:      install.sh --binary /path/to/webpulse"
    [ -f "$dir/Cargo.lock" ] && locked=(--locked)
    printf 'Building WebPulse from source with %s (this takes a few minutes)...\n' "$WP_CARGO"
    ( cd "$dir" && "$WP_CARGO" build --release ${locked[@]+"${locked[@]}"} ) \
        || fail "cargo build failed (see the output above)."
    [ -f "$dir/target/release/webpulse" ] || fail "cargo build did not produce target/release/webpulse."
    WP_BIN_SRC="$dir/target/release/webpulse"
}

# Source tarball of ${WP_REPO}@${WP_REF} -> $TMP_DIR/src (SRC). No binary yet.
fetch_source_tarball() {
    local url="${WEBPULSE_SOURCE_URL:-https://codeload.github.com/${WP_REPO}/tar.gz/${WP_REF}}"
    printf 'Downloading source %s\n' "$url"
    http_get "$url" "$TMP_DIR/source.tar.gz" || fail "Download failed: $url"
    extract_tarball "$TMP_DIR/source.tar.gz" "$TMP_DIR/src"
    SRC="$TMP_DIR/src"
    WP_SOURCE_DESC="$url"
}

# Prebuilt release tarball -> $TMP_DIR/src (SRC); returns 1 when the release or its
# asset does not exist (WP_FETCH_WHY says why). Checksum problems are fatal.
fetch_release() {
    local tag='' ver url sha_url expected actual
    WP_FETCH_WHY=''
    if [ -n "${WEBPULSE_DOWNLOAD_URL:-}" ]; then
        url=$WEBPULSE_DOWNLOAD_URL
        sha_url="$url.sha256"
    else
        detect_target
        case "$WP_REF" in
            main|master) tag=$(latest_tag) ;;
            v[0-9]*) tag=$WP_REF ;;
            [0-9]*.*) tag=v$WP_REF ;;
            *) WP_FETCH_WHY="'$WP_REF' is a branch, there is no prebuilt binary for it"; return 1 ;;
        esac
        if [ -z "$tag" ]; then
            WP_FETCH_WHY="no release of ${WP_REPO} could be found"
            return 1
        fi
        ver=${tag#v}
        url="https://github.com/${WP_REPO}/releases/download/${tag}/webpulse-${ver}-${WP_TARGET}.tar.gz"
        sha_url="$url.sha256"
    fi

    printf 'Downloading %s\n' "$url"
    if ! http_get "$url" "$TMP_DIR/webpulse.tar.gz"; then
        case "$HTTP_CODE" in
            404|403)
                WP_FETCH_WHY="no release asset at $url"
                return 1
                ;;
        esac
        fail "Download failed: $url (HTTP ${HTTP_CODE:-error})"
    fi

    expected=${WEBPULSE_DOWNLOAD_SHA256:-}
    if [ -z "$expected" ]; then
        http_get "$sha_url" "$TMP_DIR/webpulse.tar.gz.sha256" \
            || fail "Cannot verify the download: $sha_url could not be fetched."
        expected=$(head -n 1 "$TMP_DIR/webpulse.tar.gz.sha256" | cut -d' ' -f1)
    fi
    case "$expected" in
        *[!0-9A-Fa-f]*|'') fail "Invalid checksum file ($sha_url)." ;;
    esac
    [ "${#expected}" = 64 ] || fail "Invalid checksum file ($sha_url)."
    actual=$(sha256_of "$TMP_DIR/webpulse.tar.gz")
    expected=$(printf '%s' "$expected" | tr 'A-F' 'a-f')
    if [ "$actual" != "$expected" ]; then
        fail "Checksum mismatch for $url
  expected $expected
  actual   $actual
Nothing was installed."
    fi
    printf 'Checksum OK (sha256 %s)\n' "$actual"

    extract_tarball "$TMP_DIR/webpulse.tar.gz" "$TMP_DIR/src"
    SRC="$TMP_DIR/src"
    WP_SOURCE_DESC="$url"
    return 0
}

# Remote source: release binary, or source + cargo when asked for / when there is no release.
fetch_source() {
    TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/webpulse-install.XXXXXX")
    if [ -n "$WP_BINARY" ]; then
        # the binary is provided: only the support files (cpanel/, whm/, hooks/, ...) are needed
        fetch_source_tarball
        return 0
    fi
    if [ "$WP_BUILD" = 1 ]; then
        fetch_source_tarball
        cargo_build "$SRC"
        return 0
    fi
    if fetch_release; then
        return 0
    fi
    printf 'No prebuilt binary: %s.\n' "$WP_FETCH_WHY"
    if find_cargo; then
        printf 'cargo found: falling back to a build from source (ref %s).\n' "$WP_REF"
        rm -rf "$TMP_DIR/src"
        fetch_source_tarball
        cargo_build "$SRC"
        return 0
    fi
    fail "$WP_FETCH_WHY, and cargo is not installed, so there is nothing to install.
  - Install a released version:  install.sh --ref v<version>   (see https://github.com/${WP_REPO}/releases)
  - Build from source:           install Rust (https://rustup.rs), then run install.sh --build
  - Air-gapped:                  install.sh --binary /path/to/webpulse"
}

# Decide which binary gets installed (sets WP_BIN_SRC) for a local source.
select_binary() {
    local is_source=$1
    if [ -n "$WP_BINARY" ]; then
        WP_BIN_SRC=$WP_BINARY
        printf 'Using the provided binary %s\n' "$WP_BIN_SRC"
    elif [ -n "$WP_BIN_SRC" ]; then
        : # built or downloaded by fetch_source
    elif [ "$is_source" = 1 ]; then
        if [ "$WP_BUILD" = 1 ]; then
            cargo_build "$SRC"
        elif [ -f "$SRC/target/release/webpulse" ]; then
            WP_BIN_SRC="$SRC/target/release/webpulse"
            printf 'Using the existing build %s\n' "$WP_BIN_SRC"
        else
            fail "$SRC is a source checkout without a built binary.
  Build it first:  cargo build --release     (or run: install.sh --build)
  or point to a binary:  install.sh --binary /path/to/webpulse
  or install a release:  curl -fsSL https://raw.githubusercontent.com/${WP_REPO}/main/install.sh | bash"
        fi
    elif [ -f "$SRC/bin/webpulse" ]; then
        WP_BIN_SRC="$SRC/bin/webpulse"
    else
        fail "No WebPulse binary in $SRC/bin."
    fi
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
