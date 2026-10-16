#!/usr/bin/env bash
# Parity of the databases: the legacy PHP implementation (0.1.0) and the Rust
# binary must write identical domain databases for the same logs and clock.
#
#   tests/parity/db.sh
#
# Builds two identical fake cPanel roots (tests/tmp/parity/php and
# tests/tmp/parity/rust, each with its config.json and a `now` file for
# api.sh), generates the logs once with the Rust generator (`webpulse dev
# gen-logs`, byte-identical to tests/gen_logs.php), pre-seeds the registry
# salts (so visitor hashes match), then runs the same scenario with both
# implementations under the same WEBPULSE_NOW:
#   collect → backfill → more (out-of-order) lines → collect → collect → rollup
# with GeoIP on (tests/fixtures/geoip) and a small open-visit cap and
# max_rows_per_dim so that eviction and folding are exercised.
#
# Every table of every domain database is dumped with a stable ORDER BY (root
# paths normalised, checkpoint inodes left out) and diffed, as well as the
# registry (accounts, domains, jobs, domain_summary without db_bytes, salts,
# settings). Diffs go to tests/tmp/parity/db-diff/.
#
# Legacy code: $WEBPULSE_LEGACY (a directory with lib/ and bin/webpulse), else
# tests/tmp/legacy, created with `git worktree add --detach tests/tmp/legacy
# v0.1.0` when missing. Binary: $WEBPULSE_BIN, else target/release/webpulse,
# else target/debug/webpulse.
# Exit status: 0 identical, 1 different, 2 setup error.

set -uo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
TMP=$REPO/tests/tmp/parity
OUT=$TMP/db-diff
PHP=${PHP:-php}

die() { echo "db.sh: $*" >&2; exit 2; }

command -v "$PHP" >/dev/null || die "php is required"
"$PHP" -r 'exit(PHP_VERSION_ID >= 80100 && extension_loaded("pdo_sqlite") ? 0 : 1);' || die "php >= 8.1 with pdo_sqlite is required"

LEGACY=${WEBPULSE_LEGACY:-$REPO/tests/tmp/legacy}
if [ ! -f "$LEGACY/lib/autoload.php" ]; then
    [ -z "${WEBPULSE_LEGACY:-}" ] || die "no legacy PHP code in $LEGACY"
    git -C "$REPO" worktree add --detach "$LEGACY" v0.1.0 >/dev/null 2>&1 || die "cannot check out v0.1.0 into $LEGACY"
fi
[ -f "$LEGACY/bin/webpulse" ] || die "no legacy bin/webpulse in $LEGACY"

BIN=${WEBPULSE_BIN:-}
if [ -z "$BIN" ]; then
    if [ -x "$REPO/target/release/webpulse" ]; then BIN=$REPO/target/release/webpulse; else BIN=$REPO/target/debug/webpulse; fi
fi
[ -x "$BIN" ] || die "webpulse binary not found ($BIN); run cargo build"

export TZ=UTC
unset WEBPULSE_CONFIG WEBPULSE_NOW

ts() { "$PHP" -r 'echo (new DateTimeImmutable($argv[1], new DateTimeZone("Europe/Rome")))->getTimestamp();' "$1"; }

rm -rf "$TMP"
mkdir -p "$TMP/template/root" "$OUT"

# --- one template root ------------------------------------------------------------------------
cp -a "$REPO/tests/fixtures/fakeroot/." "$TMP/template/root/"
for d in etc/apache2/logs/domlogs/alice etc/apache2/logs/domlogs/bob home/alice/logs home/bob/logs var/webpulse var/log/nginx/domains etc/cron.d; do
    mkdir -p "$TMP/template/root/$d"
done

write_config() { # <dir>
    local r=$1/root
    cat > "$1/config.json" <<JSON
{
    "version": 1,
    "default_enabled": true,
    "timezone": "Europe/Rome",
    "raw_retention_days": 30,
    "max_rows_per_dim": 10,
    "session_timeout_min": 30,
    "store_ip": false,
    "exclude_ips": ["198.51.100.250"],
    "exclude_paths": ["/.well-known/", "/cpanel", "/whm", "/webmail", "/cgi-sys/"],
    "count_head_requests": false,
    "max_open_visits": 60,
    "geoip": {"enabled": true, "country_db": "$REPO/tests/fixtures/geoip/dbip-country-lite.mmdb", "city_db": null},
    "php_bin": null,
    "paths": {
        "data_dir": "$r/var/webpulse/data",
        "registry_db": "$r/var/webpulse/webpulse.db",
        "log_dir": "$r/var/webpulse/log",
        "run_dir": "$r/var/webpulse/run",
        "domlogs_dirs": ["$r/etc/apache2/logs/domlogs/{user}", "$r/etc/apache2/logs/domlogs", "$r/usr/local/apache/domlogs/{user}", "$r/var/log/nginx/domains"],
        "archive_dir": "$r/home/{user}/logs",
        "userdomains_file": "$r/etc/userdomains",
        "cpanel_users_dir": "$r/var/cpanel/users",
        "cpanel_userdata_dir": "$r/var/cpanel/userdata",
        "home_root": "$r/home",
        "cron_file": "$r/etc/cron.d/webpulse"
    }
}
JSON
}

NOW0=$(ts '2026-10-10 15:00:00')
NOW1=$(ts '2026-10-11 12:00:00')
NOW2=$(ts '2026-10-12 03:00:00')
NOW3=$(ts '2026-10-12 04:00:00')

write_config "$TMP/template"
"$BIN" dev gen-logs --seed 7 --days 25 --lines 40000 --end 2026-10-10 --max-ts "$NOW0" --out "$TMP/template/root" > "$TMP/gen1.json" \
    || die "gen-logs failed"
"$BIN" dev gen-logs --seed 8 --days 2 --lines 6000 --end 2026-10-11 --no-archive --out "$TMP/extra" > "$TMP/gen2.json" \
    || die "gen-logs failed"
WEBPULSE_NOW=$NOW0 "$BIN" --config "$TMP/template/config.json" setup --quiet || die "setup failed"
# deterministic visitor hashes: the same salts in both registries
days=$("$PHP" -r 'for ($t = strtotime("2026-09-10 UTC"); $t <= strtotime("2026-10-14 UTC"); $t += 86400) echo gmdate("Y-m-d", $t), " ";')
"$PHP" -r '
    $pdo = new PDO("sqlite:" . $argv[1]);
    $st = $pdo->prepare("INSERT OR REPLACE INTO salts (day, salt) VALUES (?, ?)");
    foreach (array_slice($argv, 2) as $d) { $st->bindValue(1, $d); $st->bindValue(2, hash("sha256", "parity-$d", true), PDO::PARAM_LOB); $st->execute(); }
' "$TMP/template/root/var/webpulse/webpulse.db" $days || die "cannot seed salts"

for impl in php rust; do
    cp -a "$TMP/template" "$TMP/$impl"
    write_config "$TMP/$impl"
    echo "$NOW3" > "$TMP/$impl/now"
done
rm -rf "$TMP/template"

run() { # <impl> <now> <args…>
    local impl=$1 now=$2
    shift 2
    if [ "$impl" = php ]; then
        WEBPULSE_NOW=$now WEBPULSE_CONFIG="$TMP/php/config.json" "$PHP" -d date.timezone=UTC -d memory_limit=1G "$LEGACY/bin/webpulse" "$@"
    else
        WEBPULSE_NOW=$now WEBPULSE_CONFIG="$TMP/rust/config.json" "$BIN" "$@"
    fi
}

for impl in php rust; do
    r=$TMP/$impl/root
    run "$impl" "$NOW0" collect --quiet || die "$impl collect failed"
    run "$impl" "$NOW0" backfill --user alice --quiet || die "$impl backfill failed"
    for f in "$TMP"/extra/etc/apache2/logs/domlogs/*/*; do
        rel=${f#"$TMP"/extra/}
        cat "$f" >> "$r/$rel"
    done
    run "$impl" "$NOW1" collect --quiet || die "$impl collect failed"
    run "$impl" "$NOW2" collect --quiet || die "$impl collect failed"
    run "$impl" "$NOW3" rollup --quiet || die "$impl rollup failed"
done

# --- dump and compare ---------------------------------------------------------------------------
dump() { # <impl>
    "$PHP" -r '
        [$_, $root] = $argv;
        $q = static function (PDO $pdo, string $sql): void {
            echo "## $sql\n";
            foreach ($pdo->query($sql)->fetchAll(PDO::FETCH_NUM) as $r) {
                echo str_replace($GLOBALS["root"], "ROOT", json_encode($r, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE | JSON_PRESERVE_ZERO_FRACTION)), "\n";
            }
        };
        $GLOBALS["root"] = $root;
        foreach (glob("$root/var/webpulse/data/*/*.db") as $db) {
            echo "# " . substr($db, strlen($root)) . "\n";
            $pdo = new PDO("sqlite:$db");
            foreach (["SELECT * FROM daily ORDER BY day", "SELECT * FROM hourly ORDER BY day, hour",
                "SELECT * FROM dim_daily ORDER BY day, dim, key", "SELECT * FROM visits ORDER BY visit_id",
                "SELECT * FROM events ORDER BY id", "SELECT * FROM open_visits ORDER BY visitor",
                "SELECT * FROM visitor_day ORDER BY day, visitor",
                "SELECT path, offset, size, first_line, last_ts, lines, bad_lines, ignored_lines, updated_at FROM checkpoints ORDER BY path",
                "SELECT * FROM imported_archives ORDER BY path", "SELECT * FROM meta ORDER BY key",
                "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE name NOT LIKE \"sqlite_stat%\" ORDER BY name"] as $sql) {
                $q($pdo, $sql);
            }
        }
        echo "# registry\n";
        $pdo = new PDO("sqlite:$root/var/webpulse/webpulse.db");
        foreach (["SELECT * FROM accounts ORDER BY user", "SELECT * FROM domains ORDER BY user, domain",
            "SELECT id, type, scope, started_at, finished_at, status, lines, files, domains, message FROM jobs ORDER BY id",
            "SELECT user, domain, last_collect_at, last_line_ts, lines_total, visits_today, pageviews_today, visits_7d, pageviews_7d, bytes_7d, visits_30d, pageviews_30d, bytes_30d, bot_hits_30d FROM domain_summary ORDER BY user, domain",
            "SELECT day, hex(salt) FROM salts ORDER BY day", "SELECT * FROM settings ORDER BY key",
            "SELECT key, value FROM meta WHERE key <> \"version\" ORDER BY key",
            "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY name"] as $sql) {
            $q($pdo, $sql);
        }
    ' "$TMP/$1/root"
}
dump php > "$TMP/php.dump" || die "dump failed"
dump rust > "$TMP/rust.dump" || die "dump failed"

status=0
if diff -u "$TMP/php.dump" "$TMP/rust.dump" > "$OUT/db.diff"; then
    rm -f "$OUT/db.diff"
    echo "db parity: identical ($(grep -c '^\[' "$TMP/php.dump") rows in $(grep -c '^# /' "$TMP/php.dump") domain databases + registry)"
else
    status=1
    echo "db parity: DIFFERENT — $(grep -c '^[-+]\[' "$OUT/db.diff") row(s) differ; see ${OUT#"$REPO"/}/db.diff"
    grep '^## \|^# ' "$TMP/php.dump" > /dev/null
    awk '/^ ##|^ #|^-##|^\+##/ {sec=$0} /^[-+]\[/ {print sec; n++} n>=20 {exit}' "$OUT/db.diff" | uniq | head -20
fi
# the database sizes may differ (page allocation of two SQLite builds): informational only
for f in "$TMP"/php/root/var/webpulse/data/*/*.db; do
    g=$TMP/rust/root/${f#"$TMP"/php/root/}
    [ -f "$g" ] && echo "  size $(basename "$f"): php $(stat -c %s "$f") / rust $(stat -c %s "$g") bytes"
done
exit $status
