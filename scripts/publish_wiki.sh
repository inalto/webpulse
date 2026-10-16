#!/usr/bin/env bash
# Publish the pages in wiki/ to the GitHub wiki of the repository.
#
#   scripts/publish_wiki.sh [commit message]
#
# A GitHub wiki is a separate git repository: https://github.com/<owner>/<name>.wiki.git
# GitHub creates it only when the first page is saved from the web interface, so
# do this ONCE before the first run: open the "Wiki" tab of the repository on
# GitHub and click "Create the first page" > "Save page". After that this script
# can replace the wiki content from the files in wiki/.
#
# Authentication: git uses your normal credentials. With the GitHub CLI run
# `gh auth login` and `gh auth setup-git` once, or use an SSH remote by setting
# WEBPULSE_WIKI_URL.
#
# Environment:
#   WEBPULSE_REPO       owner/name of the repository          (default: inalto/webpulse)
#   WEBPULSE_WIKI_URL   full clone URL of the wiki, overrides the one derived from WEBPULSE_REPO
#   WEBPULSE_WIKI_BRANCH  branch to push                      (default: master, GitHub's wiki default)
#   WEBPULSE_WIKI_DRY_RUN=1   commit locally, show the result, do not push
#
# Pages that exist in the wiki but not in wiki/ are removed, so wiki/ is the
# single source of truth. Edit the files in wiki/, not on the GitHub web page.
set -euo pipefail

repo="${WEBPULSE_REPO:-inalto/webpulse}"
url="${WEBPULSE_WIKI_URL:-https://github.com/${repo}.wiki.git}"
branch="${WEBPULSE_WIKI_BRANCH:-master}"
dry_run="${WEBPULSE_WIKI_DRY_RUN:-0}"

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
src="$here/../wiki"
msg="${1:-Update wiki from wiki/ ($(date -u +%Y-%m-%d))}"

die() {
    printf 'publish_wiki: %s\n' "$*" >&2
    exit 1
}

command -v git >/dev/null 2>&1 || die "git is required"
[ -d "$src" ] || die "directory not found: $src"

shopt -s nullglob
pages=("$src"/*.md)
shopt -u nullglob
[ "${#pages[@]}" -gt 0 ] || die "no .md files in $src"

work=$(mktemp -d "${TMPDIR:-/tmp}/webpulse-wiki.XXXXXX")
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

printf 'Cloning %s\n' "$url"
if git clone --quiet "$url" "$work/wiki" 2>"$work/clone.err"; then
    cd "$work/wiki"
    # push to the branch the wiki already uses
    current=$(git symbolic-ref --short -q HEAD || true)
    if [ -n "$current" ]; then
        branch=$current
    fi
else
    # An empty (never saved) wiki and a wiki that does not exist both fail here.
    # Tell them apart from an authentication problem as far as git lets us.
    err=$(cat "$work/clone.err")
    case "$err" in
        *"Authentication failed"*|*"could not read Username"*|*"Permission denied"*|*"terminal prompts disabled"*)
            printf '%s\n' "$err" >&2
            die "authentication failed. Run 'gh auth login' and 'gh auth setup-git', then try again."
            ;;
    esac
    printf '\nCould not clone %s\n' "$url" >&2
    printf '%s\n' "$err" | sed 's/^/    /' >&2
    cat >&2 <<MSG

The wiki probably does not exist yet. GitHub creates the wiki repository only
when the first page is saved from the web interface. Do this once:

  1. Open https://github.com/${repo}/wiki
     (enable "Wikis" under Settings > Features if the tab is missing).
  2. Click "Create the first page" and then "Save page".
  3. Run this script again.

Continuing with a fresh repository (git init). The push below will fail if the
wiki really has not been created yet.

MSG
    mkdir -p "$work/wiki"
    cd "$work/wiki"
    git init --quiet
    git checkout --quiet -b "$branch" 2>/dev/null || git symbolic-ref HEAD "refs/heads/$branch"
    git remote add origin "$url"
fi

# make the wiki an exact copy of wiki/*.md (keep .git)
find . -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
cp "${pages[@]}" .

git add -A
if git diff --cached --quiet; then
    printf 'The wiki is already up to date.\n'
    exit 0
fi

# use the user's identity when set, otherwise a neutral one for this commit only
name=$(git config user.name || true)
email=$(git config user.email || true)
[ -n "$name" ] || name="WebPulse wiki publisher"
[ -n "$email" ] || email="noreply@users.noreply.github.com"
git -c "user.name=$name" -c "user.email=$email" commit --quiet -m "$msg"
git --no-pager log --stat --oneline -n 1

if [ "$dry_run" = 1 ]; then
    printf '\nDry run: not pushing. Work copy: %s\n' "$work/wiki"
    trap - EXIT
    exit 0
fi

printf '\nPushing to %s (branch %s)\n' "$url" "$branch"
if ! git push origin "HEAD:refs/heads/$branch" 2>"$work/push.err"; then
    sed 's/^/    /' "$work/push.err" >&2
    cat >&2 <<MSG

Push failed. Common causes:
  - the wiki has not been created yet: open https://github.com/${repo}/wiki,
    click "Create the first page", save it, and run this script again;
  - no credentials: run 'gh auth login' and 'gh auth setup-git';
  - you do not have write access to ${repo}.
MSG
    exit 1
fi

printf 'Done. https://github.com/%s/wiki\n' "$repo"
