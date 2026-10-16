#!/usr/bin/env bash
# Build dist/webpulse-<VERSION>.tar.gz (the SOURCE tarball) from a clean `git archive` of HEAD.
# It is what `install.sh --build` compiles with cargo; the prebuilt binary tarball
# (webpulse-<VERSION>-<target>.tar.gz) comes from scripts/build_release.sh.
#
#   scripts/build_dist.sh [output-dir]        (default: dist/)
#
# The archive has a single top-level directory webpulse-<VERSION>/, exactly like
# the tarballs GitHub's codeload serves, so install.sh can consume either.
# Uncommitted changes are NOT included (warning printed); set
# WEBPULSE_DIST_REF to archive a tag or commit other than HEAD.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

command -v git >/dev/null 2>&1 || { echo "build_dist: git is required" >&2; exit 1; }
git rev-parse --git-dir >/dev/null 2>&1 || { echo "build_dist: $root is not a git checkout" >&2; exit 1; }

ref="${WEBPULSE_DIST_REF:-HEAD}"
# the version of what is archived, not of the (possibly modified) working copy
version=$(git show "$ref:VERSION" | tr -d ' \t\r\n')
case "$version" in
    ''|*[!0-9A-Za-z._+-]*) echo "build_dist: invalid version '$version'" >&2; exit 1 ;;
esac

outdir="${1:-dist}"
mkdir -p "$outdir"
out="$outdir/webpulse-$version.tar.gz"

if [ "$ref" = HEAD ] && [ -n "$(git status --porcelain 2>/dev/null)" ]; then
    echo "build_dist: warning: uncommitted changes are not part of the archive" >&2
fi

git archive --format=tar --prefix="webpulse-$version/" "$ref" | gzip -9n >"$out"

# sanity: the installer needs these (a real checkout also has Cargo.toml and src/)
listing=$(tar -tzf "$out")
for required in "webpulse-$version/install.sh" "webpulse-$version/installer/lib.sh" "webpulse-$version/VERSION"; do
    if ! grep -qxF "$required" <<<"$listing"; then
        echo "build_dist: $required is missing from the archive (commit it first)" >&2
        rm -f "$out"
        exit 1
    fi
done

if command -v sha256sum >/dev/null 2>&1; then
    (cd "$outdir" && sha256sum "webpulse-$version.tar.gz" >"webpulse-$version.tar.gz.sha256")
fi
echo "$out ($(wc -c <"$out") bytes)"
