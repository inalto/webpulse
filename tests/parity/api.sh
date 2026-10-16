#!/usr/bin/env bash
# Parity of the JSON API: the legacy PHP implementation (0.1.0) and the Rust
# binary must answer every action identically for the same databases.
#
#   tests/parity/api.sh [PHP_ROOT [RUST_ROOT]]
#
# PHP_ROOT / RUST_ROOT are fake server roots containing config.json (default
# tests/tmp/parity/php and tests/tmp/parity/rust, as left by db.sh). They may
# be the same directory (e.g. a SeedDb fixture). The clock is WEBPULSE_NOW,
# else the content of <PHP_ROOT>/now. The legacy code is taken from
# $WEBPULSE_LEGACY_LIB, else tests/tmp/legacy/lib (the v0.1.0 checkout of db.sh).
# The binary is $WEBPULSE_BIN, else target/release/webpulse, else
# target/debug/webpulse.
#
# Every JSON response is compared after `jq -S 'del(.meta.took_ms,
# .meta.generated_at)'` (the official criterion; database file sizes are
# ignored when the two roots differ, and the product version string that
# `ping` and `admin.overview` report is blanked: 0.1.0 against the current
# release) and, more strictly, byte for byte with those values blanked (reported, not fatal: `jq` 1.6 hides the
# int/float distinction). CSV exports are compared byte for byte, with their
# Content-Disposition. Diffs are written to tests/tmp/parity/api-diff/.
# Exit status: 0 when everything matches, 1 otherwise, 2 on setup errors.

set -uo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
PHP_ROOT=${1:-$REPO/tests/tmp/parity/php}
RUST_ROOT=${2:-${1:-$REPO/tests/tmp/parity/rust}}
OUT=$REPO/tests/tmp/parity/api-diff
PHP=${PHP:-php}

die() { echo "api.sh: $*" >&2; exit 2; }

command -v jq >/dev/null || die "jq is required"
command -v "$PHP" >/dev/null || die "php is required"
[ -f "$PHP_ROOT/config.json" ] || die "no config.json in $PHP_ROOT"
[ -f "$RUST_ROOT/config.json" ] || die "no config.json in $RUST_ROOT"

LIB=${WEBPULSE_LEGACY_LIB:-}
if [ -z "$LIB" ]; then
    LIB=$REPO/tests/tmp/legacy/lib # the v0.1.0 checkout made by db.sh
fi
[ -f "$LIB/autoload.php" ] || die "legacy PHP library not found ($LIB); run tests/parity/run.sh"

BIN=${WEBPULSE_BIN:-}
if [ -z "$BIN" ]; then
    if [ -x "$REPO/target/release/webpulse" ]; then BIN=$REPO/target/release/webpulse; else BIN=$REPO/target/debug/webpulse; fi
fi
[ -x "$BIN" ] || die "webpulse binary not found ($BIN); run cargo build"

NOW=${WEBPULSE_NOW:-}
[ -z "$NOW" ] && [ -f "$PHP_ROOT/now" ] && NOW=$(tr -d ' \n' < "$PHP_ROOT/now")
[ -n "$NOW" ] || die "set WEBPULSE_NOW (or write it to $PHP_ROOT/now)"
export WEBPULSE_NOW=$NOW
# PHP's date() uses date.timezone (default UTC); the binary uses TZ.
export TZ=UTC

rm -rf "$OUT"
mkdir -p "$OUT"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/webpulse-api-parity.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

# php_batch <requests.tsv> <out.jsonl>
php_batch() {
    WEBPULSE_CONFIG="$PHP_ROOT/config.json" "$PHP" -d date.timezone=UTC -d memory_limit=1G \
        "$REPO/tests/parity/php_api.php" "$LIB" < "$1" > "$2" || die "PHP driver failed"
}

# rust_one <principal> <method> <query> [body]: one JSON line like php_api.php
rust_one() {
    local who=$1 method=$2 qs=$3 body=${4:-} args
    if [[ $who == user:* ]]; then args=(cgi --principal "$who" --api); else args=(cgi --whm --api); fi
    local hdr=$WORK/r.hdr raw=$WORK/r.raw bodyf=$WORK/r.body
    printf '%s' "$body" | env WEBPULSE_CONFIG="$RUST_ROOT/config.json" WEBPULSE_CGI_TESTING=1 \
        REQUEST_METHOD="$method" QUERY_STRING="$qs" CONTENT_LENGTH="${#body}" REMOTE_ADDR=203.0.113.9 \
        REMOTE_USER="${who#whm:}" HTTP_X_WEBPULSE_REQUEST="$([ "$method" = POST ] && echo 1)" \
        "$BIN" "${args[@]}" > "$raw" 2>/dev/null
    sed -n '1,/^\r$/p' "$raw" | tr -d '\r' > "$hdr"
    sed '1,/^\r$/d' "$raw" > "$bodyf"
    local status type disp
    status=$(sed -n 's/^Status: \([0-9]*\).*/\1/p' "$hdr")
    type=$(sed -n 's/^Content-Type: //p' "$hdr")
    disp=$(sed -n 's/^Content-Disposition: //p' "$hdr")
    jq -Rsc --argjson status "${status:-0}" --arg type "$type" --arg disp "$disp" \
        '{status: $status, type: $type, headers: (if $disp == "" then {} else {"Content-Disposition": $disp} end), body: .}' < "$bodyf"
}

# rust_batch <requests.tsv> <out.jsonl>
rust_batch() {
    : > "$2"
    while IFS=$'\t' read -r who method qs body; do
        [ -z "$who" ] && continue
        rust_one "$who" "$method" "$qs" "${body:-}" >> "$2"
    done < "$1"
}

# --- discover the domains with data -----------------------------------------------------------
printf 'whm:root\tGET\ta=domains\n' > "$WORK/d.tsv"
php_batch "$WORK/d.tsv" "$WORK/d.jsonl"
mapfile -t DOMS < <(jq -r '.body | fromjson | .data.domains[] | select(.has_data) | "\(.user) \(.domain)"' "$WORK/d.jsonl")
[ ${#DOMS[@]} -gt 0 ] || die "no domain with data in $PHP_ROOT"
mapfile -t USERS < <(printf '%s\n' "${DOMS[@]}" | cut -d' ' -f1 | sort -u)
CPUSERS=$(jq -r '.paths.cpanel_users_dir // empty' "$PHP_ROOT/config.json")
mapfile -t RESELLERS < <(cat "${CPUSERS:-$PHP_ROOT/var/cpanel/users}"/* 2>/dev/null | sed -n 's/^OWNER=//p' | grep -v '^root$' | sort -u)

DIMS="page entry_page exit_page referrer_host referrer_url channel search_engine keyword campaign utm_source utm_medium
source_medium country city browser browser_version os os_version device status method protocol http_version file_type
static_file download not_found not_found_referrer server_error bot visit_duration pages_per_visit"

REQ=$WORK/req.tsv
: > "$REQ"
add() { printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "${4:-}" >> "$REQ"; }

add whm:root GET 'a=domains'
add whm:root GET 'a=admin.overview'
add whm:root GET 'a=admin.accounts'
add whm:root GET 'a=admin.accounts&q=a&limit=2'
add whm:root GET 'a=admin.settings'
add whm:root GET 'a=admin.jobs'
add whm:root GET 'a=ping'
for r in "${RESELLERS[@]}"; do
    add "whm:$r" GET 'a=domains'
    add "whm:$r" GET 'a=admin.overview'
    add "whm:$r" GET 'a=admin.accounts'
    add "whm:$r" GET 'a=admin.jobs'
    add "whm:$r" GET 'a=admin.settings'
done
add whm:nobody-here GET 'a=domains'
for u in "${USERS[@]}"; do
    add "user:$u" GET 'a=domains'
    add "user:$u" GET 'a=user_settings'
    add "user:$u" GET 'a=admin.overview'
done
for ud in "${DOMS[@]}"; do
    u=${ud% *}
    d=${ud#* }
    p="user:$u"
    q="domain=$d"
    for range in today yesterday 7d 30d 90d 12m month lastmonth year all; do
        add "$p" GET "a=overview&$q&range=$range"
    done
    add "$p" GET "a=overview&$q&range=30d&compare=1"
    add "$p" GET "a=overview&$q&range=today&compare=1"
    add "$p" GET "a=overview&$q&range=12m&compare=1"
    add "$p" GET "a=overview&$q&from=2026-01-01&to=2026-12-31"
    for dim in $DIMS; do
        add "$p" GET "a=report&$q&name=$dim&limit=50&compare=1"
    done
    add "$p" GET "a=report&$q&name=channel&series=1"
    add "$p" GET "a=report&$q&name=country&series=1&range=today"
    add "$p" GET "a=report&$q&name=page&series=1&range=12m"
    add "$p" GET "a=report&$q&name=page&q=BLOG"
    add "$p" GET "a=report&$q&name=page&q=%25"
    add "$p" GET "a=report&$q&name=page&limit=2&offset=2"
    add "$p" GET "a=report&$q&name=country&sort=-bounce_rate&limit=20"
    add "$p" GET "a=report&$q&name=entry_page&sort=avg_duration"
    add "$p" GET "a=report&$q&name=status&sort=key&limit=50"
    add "$p" GET "a=report&$q&name=visit_duration&limit=20"
    add "$p" GET "a=report&$q&name=page&sort=share"
    add "$p" GET "a=realtime&$q"
    add "$p" GET "a=visits_log&$q&range=7d&limit=10"
    add "$p" GET "a=visits_log&$q&range=all&limit=100"
    add "$p" GET "a=health&$q"
    add "$p" GET "a=health&$q&range=today"
    add "$p" GET "a=health&$q&range=12m"
    add "$p" GET "a=export&$q&name=page&format=csv"
    add "$p" GET "a=export&$q&name=country"
    add "$p" GET "a=export&$q&name=not_found&range=90d"
    add whm:root GET "a=overview&$q&user=$u&compare=1"
    add whm:root GET "a=report&$q&user=$u&name=bot&limit=50"
    # error paths
    add "$p" GET "a=report&$q&name=nope"
    add "$p" GET "a=report&$q&name=page&sort=key%20DESC"
    add "$p" GET "a=report&$q&name=page&limit=ten"
    add "$p" GET "a=overview&$q&range=forever"
    add "$p" GET "a=overview&$q&from=2026-13-01&to=2026-12-01"
    add "$p" GET "a=visits_log&$q&before=-5"
    add "$p" GET "a=export&$q&name=page&format=xlsx"
    add "$p" POST "a=overview&$q"
done
add "user:${USERS[0]}" GET 'a=overview'
add "user:${USERS[0]}" GET 'a=overview&domain=..%2Fx'
add "user:${USERS[0]}" GET 'a=overview&domain=nosuch.example'
add "user:${USERS[0]}" GET 'a=teleport'
add "user:${USERS[0]}" GET ''
add "user:${USERS[0]}" GET 'a=admin.jobs'
add whm:root GET 'a=admin.set_account'
add whm:root GET 'a=admin.nope'

php_batch "$REQ" "$WORK/php.jsonl"
rust_batch "$REQ" "$WORK/rust.jsonl"

# second page of every visits_log request, with the cursor PHP returned
REQ2=$WORK/req2.tsv
: > "$REQ2"
i=0
while IFS=$'\t' read -r who method qs body; do
    i=$((i + 1))
    [[ $qs == a=visits_log* ]] || continue
    cur=$(sed -n "${i}p" "$WORK/php.jsonl" | jq -r '.body | fromjson | .data | select(.next_before != null) | "before=\(.next_before)&before_id=\(.next_before_id)"' 2>/dev/null)
    [ -n "$cur" ] && printf '%s\t%s\t%s\t%s\n' "$who" "$method" "$qs&$cur" "${body:-}" >> "$REQ2"
done < "$REQ"
if [ -s "$REQ2" ]; then
    php_batch "$REQ2" "$WORK/php2.jsonl"
    rust_batch "$REQ2" "$WORK/rust2.jsonl"
    cat "$REQ2" >> "$REQ"
    cat "$WORK/php2.jsonl" >> "$WORK/php.jsonl"
    cat "$WORK/rust2.jsonl" >> "$WORK/rust.jsonl"
fi

# --- compare ----------------------------------------------------------------------------------
total=0 same=0 bytes=0 failed=0
norm='del(.meta.took_ms, .meta.generated_at) | walk(if type == "object" and (.version | type) == "string" then .version = "" else . end)'
# Two engines write the same rows into SQLite files of different sizes: when the
# roots differ, file sizes (db_bytes, data_bytes, registry_bytes) are not compared.
if [ "$(cd "$PHP_ROOT" && pwd -P)" != "$(cd "$RUST_ROOT" && pwd -P)" ]; then
    norm="$norm | walk(if type == \"object\" then del(.db_bytes, .data_bytes, .registry_bytes) else . end)"
    SIZES_IGNORED=1
fi
blank() { sed -E 's/"took_ms":[0-9]+/"took_ms":0/; s/"generated_at":"[^"]*"/"generated_at":""/; s/"version":"[^"]*"/"version":""/g'; }
n=$(wc -l < "$REQ")
for i in $(seq 1 "$n"); do
    total=$((total + 1))
    req=$(sed -n "${i}p" "$REQ" | cut -f1-3 | tr '\t' ' ')
    a=$(sed -n "${i}p" "$WORK/php.jsonl")
    b=$(sed -n "${i}p" "$WORK/rust.jsonl")
    sa=$(jq -r .status <<<"$a"); sb=$(jq -r .status <<<"$b")
    ta=$(jq -r .type <<<"$a"); tb=$(jq -r .type <<<"$b")
    jq -r .body <<<"$a" > "$WORK/a.body"
    jq -r .body <<<"$b" > "$WORK/b.body"
    ok=1
    if [ "$sa" != "$sb" ] || [ "$ta" != "$tb" ]; then
        ok=0
    elif [[ $ta == application/json* ]]; then
        if ! diff <(jq -S "$norm" "$WORK/a.body") <(jq -S "$norm" "$WORK/b.body") > "$WORK/d.txt"; then ok=0; fi
        if [ $ok = 1 ] && cmp -s <(blank < "$WORK/a.body") <(blank < "$WORK/b.body"); then bytes=$((bytes + 1)); fi
    else
        if ! cmp -s "$WORK/a.body" "$WORK/b.body" || [ "$(jq -c '.headers["Content-Disposition"]' <<<"$a")" != "$(jq -c '.headers["Content-Disposition"]' <<<"$b")" ]; then
            ok=0
            diff "$WORK/a.body" "$WORK/b.body" > "$WORK/d.txt"
        else
            bytes=$((bytes + 1))
        fi
    fi
    if [ $ok = 1 ]; then
        same=$((same + 1))
    else
        failed=$((failed + 1))
        f=$OUT/$(printf '%03d' "$i").txt
        {
            echo "request: $req"
            echo "php:  status $sa  $ta  $(jq -c .headers <<<"$a")"
            echo "rust: status $sb  $tb  $(jq -c .headers <<<"$b")"
            [ -f "$WORK/d.txt" ] && head -c 20000 "$WORK/d.txt"
        } > "$f"
        echo "DIFF $req  (see ${f#"$REPO"/})"
    fi
    rm -f "$WORK/d.txt"
done

# --- CLI: `webpulse report` (text and --json) -------------------------------------------------
LEGACY_BIN=$(dirname "$LIB")/bin/webpulse
cli_total=0 cli_same=0
if [ -f "$LEGACY_BIN" ] && head -c 64 "$LEGACY_BIN" | grep -q php; then
    for ud in "${DOMS[@]}"; do
        u=${ud% *}
        d=${ud#* }
        for args in "" "--json" "--range 7d --compare" "--name page --limit 5" "--name country --json --compare" \
            "--name status --sort key --limit 50" "--name visit_duration" "--name bot --range all --limit 3" "--range today --json"; do
            cli_total=$((cli_total + 1))
            # shellcheck disable=SC2086
            WEBPULSE_CONFIG="$PHP_ROOT/config.json" "$PHP" -d date.timezone=UTC "$LEGACY_BIN" report --user "$u" --domain "$d" $args \
                > "$WORK/c.php" 2>&1; ca=$?
            # shellcheck disable=SC2086
            WEBPULSE_CONFIG="$RUST_ROOT/config.json" "$BIN" report --user "$u" --domain "$d" $args > "$WORK/c.rust" 2>&1; cb=$?
            if [ $ca = $cb ] && cmp -s "$WORK/c.php" "$WORK/c.rust"; then
                cli_same=$((cli_same + 1))
            else
                failed=$((failed + 1))
                f=$OUT/cli-$cli_total.txt
                { echo "report --user $u --domain $d $args (exit php $ca, rust $cb)"; diff "$WORK/c.php" "$WORK/c.rust" | head -100; } > "$f"
                echo "DIFF report --user $u --domain $d $args  (see ${f#"$REPO"/})"
            fi
        done
    done
    echo "cli parity: $cli_same/$cli_total identical"
else
    echo "cli parity: skipped (no legacy PHP bin/webpulse)"
fi

[ -n "${SIZES_IGNORED:-}" ] && echo "note: database file sizes not compared (different roots)"
echo "api parity: $same/$total identical after jq -S ($bytes byte-identical); $failed different"
[ "$failed" -eq 0 ]
