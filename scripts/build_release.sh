#!/usr/bin/env bash
# Build the release tarball of the prebuilt binary:
#
#   dist/webpulse-<VERSION>-<TARGET>.tar.gz   (+ .sha256)
#
#   scripts/build_release.sh [options]
#     --target <triple>   Rust target (default: <host arch>-unknown-linux-musl)
#     --zigbuild          build with `cargo zigbuild` (cross compiling musl targets in CI)
#     --binary <file>     package this already built binary instead of building
#     --out <dir>         output directory (default: dist)
#
# Tarball layout (single top-level directory webpulse-<VERSION>-<TARGET>/):
#
#   bin/webpulse            the static binary, mode 0755
#   cpanel/                 plugin descriptor + LiveAPI shims (index.live.php, api.live.php, lib/)
#   whm/                    AppConfig, CGIs (index.cgi Perl, api.cgi bash), page template, icon
#   hooks/                  Standardized Hook scripts (Stats::RunAll, Stats::RunUser)
#   cockpit/                Cockpit module (manifest.json; index.html is generated at install)
#   etc/                    cron.d/webpulse, config.json.default
#   install.sh uninstall.sh installer/lib.sh
#   VERSION LICENSE README.md CHANGELOG.md
#
# install.sh consumes exactly this layout (unpacked locally or downloaded from the
# GitHub release). The source tarball for `install.sh --build` is a different
# artifact: scripts/build_dist.sh.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

target=''
zig=0
binary=''
outdir=dist

while [ "$#" -gt 0 ]; do
    case "$1" in
        --target) [ "$#" -ge 2 ] || { echo "build_release: --target needs a value" >&2; exit 2; }; target=$2; shift ;;
        --zigbuild) zig=1 ;;
        --binary) [ "$#" -ge 2 ] || { echo "build_release: --binary needs a value" >&2; exit 2; }; binary=$2; shift ;;
        --out) [ "$#" -ge 2 ] || { echo "build_release: --out needs a value" >&2; exit 2; }; outdir=$2; shift ;;
        -h|--help) sed -n '2,28p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "build_release: unknown option $1" >&2; exit 2 ;;
    esac
    shift
done
[ "${WEBPULSE_ZIGBUILD:-0}" = 1 ] && zig=1

if [ -z "$target" ]; then
    case "$(uname -m)" in
        x86_64|amd64) target=x86_64-unknown-linux-musl ;;
        aarch64|arm64) target=aarch64-unknown-linux-musl ;;
        *) echo "build_release: unknown host architecture, pass --target" >&2; exit 2 ;;
    esac
fi
case "$target" in *[!A-Za-z0-9._-]*|'') echo "build_release: invalid target '$target'" >&2; exit 2 ;; esac

version=$(tr -d ' \t\r\n' <VERSION)
case "$version" in
    ''|*[!0-9A-Za-z._+-]*) echo "build_release: invalid version '$version'" >&2; exit 1 ;;
esac
if [ -f Cargo.toml ]; then
    crate=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)
    if [ -n "$crate" ] && [ "$crate" != "$version" ]; then
        echo "build_release: Cargo.toml version ($crate) differs from VERSION ($version)" >&2
        exit 1
    fi
fi
if [ -n "${WEBPULSE_RELEASE_TAG:-}" ] && [ "${WEBPULSE_RELEASE_TAG#v}" != "$version" ]; then
    echo "build_release: tag $WEBPULSE_RELEASE_TAG does not match VERSION $version" >&2
    exit 1
fi

if [ -z "$binary" ]; then
    command -v cargo >/dev/null 2>&1 || { echo "build_release: cargo not found (or pass --binary)" >&2; exit 1; }
    if [ "$zig" = 1 ]; then
        cargo zigbuild --release --locked --target "$target"
    else
        cargo build --release --locked --target "$target"
    fi
    binary="target/$target/release/webpulse"
fi
[ -f "$binary" ] || { echo "build_release: binary $binary not found" >&2; exit 1; }
[ -s "$binary" ] || { echo "build_release: binary $binary is empty" >&2; exit 1; }

name="webpulse-$version-$target"
work=$(mktemp -d "${TMPDIR:-/tmp}/webpulse-release.XXXXXX")
trap 'rm -rf "$work"' EXIT
stage="$work/$name"
mkdir -p "$stage/bin" "$stage/etc"

install -m 0755 "$binary" "$stage/bin/webpulse"
for item in cpanel whm hooks cockpit installer; do
    [ -d "$item" ] || { echo "build_release: $item/ is missing" >&2; exit 1; }
    cp -a "$item" "$stage/$item"
done
cp -a etc/cron.d "$stage/etc/cron.d"
[ -f etc/config.json.default ] || { echo "build_release: etc/config.json.default is missing" >&2; exit 1; }
cp -a etc/config.json.default "$stage/etc/config.json.default"
for item in install.sh uninstall.sh VERSION LICENSE README.md CHANGELOG.md; do
    [ -f "$item" ] || { echo "build_release: $item is missing" >&2; exit 1; }
    cp -a "$item" "$stage/$item"
done
chmod 0755 "$stage/install.sh" "$stage/uninstall.sh" "$stage"/hooks/*.sh "$stage"/whm/cgi/*.cgi
find "$stage" -type d -exec chmod 0755 {} +

mkdir -p "$outdir"
out="$outdir/$name.tar.gz"
mtime="@${SOURCE_DATE_EPOCH:-$(date +%s)}"
if tar --version 2>/dev/null | grep -q 'GNU tar'; then
    tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="$mtime" -C "$work" -cf - "$name" | gzip -9n >"$out"
else
    tar -C "$work" -cf - "$name" | gzip -9n >"$out"
fi

listing=$(tar -tzf "$out")
for required in bin/webpulse installer/lib.sh install.sh uninstall.sh VERSION etc/config.json.default etc/cron.d/webpulse \
    cpanel/plugin/install.json cpanel/webpulse/index.live.php cpanel/webpulse/api.live.php whm/cgi/index.cgi whm/cgi/api.cgi \
    whm/webpulse.conf whm/templates/webpulse/index.tmpl hooks/stats_runall_pre.sh hooks/stats_runuser_pre.sh \
    cockpit/manifest.json; do
    if ! grep -qxF "$name/$required" <<<"$listing"; then
        echo "build_release: $name/$required is missing from the archive" >&2
        rm -f "$out"
        exit 1
    fi
done

if command -v sha256sum >/dev/null 2>&1; then
    (cd "$outdir" && sha256sum "$name.tar.gz" >"$name.tar.gz.sha256")
elif command -v shasum >/dev/null 2>&1; then
    (cd "$outdir" && shasum -a 256 "$name.tar.gz" >"$name.tar.gz.sha256")
fi
echo "$out ($(wc -c <"$out") bytes)"
