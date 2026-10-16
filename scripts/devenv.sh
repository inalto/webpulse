#!/usr/bin/env bash
# Build a fake cPanel server under tests/tmp/<name> (default "dev") with generated
# access logs, then run setup + collect + backfill + rollup with the Rust binary.
# Prints the config path on stdout. Used by `make fixtures` / `make serve`.
#
#   scripts/devenv.sh [name] [--lines N] [--seed N] [--days N]
#
# Environment: WEBPULSE_BIN (default target/release/webpulse).
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bin=${WEBPULSE_BIN:-$root/target/release/webpulse}
name=dev
lines=60000
seed=42
days=45

while [ "$#" -gt 0 ]; do
    case "$1" in
        --lines) lines=$2; shift ;;
        --seed) seed=$2; shift ;;
        --days) days=$2; shift ;;
        -*) echo "devenv: unknown option $1" >&2; exit 2 ;;
        *) name=$1 ;;
    esac
    shift
done
case "$name" in ''|*[!A-Za-z0-9_.-]*) echo "devenv: bad name '$name'" >&2; exit 2 ;; esac
[ -x "$bin" ] || { echo "devenv: $bin not found, run 'cargo build --release' first" >&2; exit 1; }

dir="$root/tests/tmp/$name"
fake="$dir/root"
rm -rf "$dir"
mkdir -p "$fake"
cp -a "$root/tests/fixtures/fakeroot/." "$fake/"
for d in etc/apache2/logs/domlogs/alice etc/apache2/logs/domlogs/bob home/alice/logs home/bob/logs \
    var/webpulse var/log/nginx/domains etc/cron.d; do
    mkdir -p "$fake/$d"
done

geo_enabled=false
geo="$root/tests/fixtures/geoip/dbip-country-lite.mmdb"
if [ -f "$geo" ]; then
    mkdir -p "$fake/var/webpulse/geoip"
    cp "$geo" "$fake/var/webpulse/geoip/dbip-country-lite.mmdb"
    geo_enabled=true
fi

cat >"$dir/config.json" <<JSON
{
  "version": 1,
  "default_enabled": true,
  "timezone": "Europe/Rome",
  "raw_retention_days": 30,
  "max_rows_per_dim": 1000,
  "session_timeout_min": 30,
  "store_ip": false,
  "exclude_ips": [],
  "exclude_paths": ["/.well-known/", "/cpanel", "/whm", "/webmail", "/cgi-sys/"],
  "count_head_requests": false,
  "geoip": { "enabled": $geo_enabled, "country_db": "$fake/var/webpulse/geoip/dbip-country-lite.mmdb", "city_db": null },
  "php_bin": null,
  "paths": {
    "data_dir": "$fake/var/webpulse/data",
    "registry_db": "$fake/var/webpulse/webpulse.db",
    "log_dir": "$fake/var/webpulse/log",
    "run_dir": "$fake/var/webpulse/run",
    "domlogs_dirs": ["$fake/etc/apache2/logs/domlogs/{user}", "$fake/etc/apache2/logs/domlogs", "$fake/usr/local/apache/domlogs/{user}", "$fake/var/log/nginx/domains"],
    "archive_dir": "$fake/home/{user}/logs",
    "userdomains_file": "$fake/etc/userdomains",
    "cpanel_users_dir": "$fake/var/cpanel/users",
    "cpanel_userdata_dir": "$fake/var/cpanel/userdata",
    "home_root": "$fake/home",
    "cron_file": "$fake/etc/cron.d/webpulse"
  }
}
JSON

export WEBPULSE_CONFIG="$dir/config.json"
end=$(date +%Y-%m-%d)
"$bin" dev gen-logs --seed "$seed" --days "$days" --lines "$lines" --end "$end" --max-ts "$(date +%s)" --out "$fake" >/dev/null
for cmd in "setup --quiet" "collect --quiet" "backfill --user alice --quiet" "rollup --quiet"; do
    # shellcheck disable=SC2086  # the commands are fixed words
    "$bin" $cmd || { echo "devenv: webpulse $cmd failed" >&2; exit 1; }
done
echo "generated $lines lines (seed $seed, $days days to $end) in $fake" >&2
echo "$dir/config.json"
