# Configuration

WebPulse is configured in three layers. Each layer overrides the one before:

1. **Built-in defaults** in the code.
2. **`/etc/webpulse/config.json`**, owned by root, mode 0600. Written by `webpulse setup`; edit it by hand
   when you need to change paths.
3. **Run-time settings** stored in the registry database. They are changed in **WHM » WebPulse
   Analytics » Server » Settings** or with `webpulse config set`, and only for the keys marked
   *editable* below.

Account owners add a fourth, narrower layer for their own domains (see
[Per-user settings](#per-user-settings)). The collector reads the configuration each time it starts, so
changes apply from the next run; there is nothing to restart.

To use another file, pass `--config <file>` or set the `WEBPULSE_CONFIG` environment variable. Relative
paths inside a config file are resolved against the directory of that file.

## config.json

The file shipped as `etc/config.json.default`:

```json
{
  "version": 1,
  "default_enabled": true,
  "timezone": null,
  "raw_retention_days": 30,
  "max_rows_per_dim": 1000,
  "session_timeout_min": 30,
  "store_ip": false,
  "exclude_ips": [],
  "exclude_paths": ["/.well-known/", "/cpanel", "/whm", "/webmail", "/cgi-sys/"],
  "count_head_requests": false,
  "geoip": { "enabled": false, "country_db": "/var/webpulse/geoip/dbip-country-lite.mmdb", "city_db": null },
  "php_bin": null,
  "paths": { "...": "see below" }
}
```

### Collection and retention

| Key | Default | Editable | Meaning |
|-----|---------|----------|---------|
| `default_enabled` | `true` | yes | Collect for accounts that have no explicit On/Off switch. |
| `timezone` | `null` | yes | Timezone identifier such as `Europe/Rome`. `null` means the server's timezone. |
| `raw_retention_days` | `30` | yes | Days to keep the visits log and raw events. Range 1 to 3650. Aggregates are kept forever. |
| `max_rows_per_dim` | `1000` | yes | Rows kept per report per day. The rest is folded into `(other)` by the nightly rollup. Range 10 to 100000. |
| `session_timeout_min` | `30` | yes | Minutes without a hit that end a visit. Range 5 to 240. |
| `store_ip` | `false` | yes | Write the visitor IP address to the visits log. See [[Privacy and GDPR]]. |
| `exclude_ips` | `[]` | yes | IPs and CIDR ranges that are not counted (at most 500). |
| `exclude_paths` | `/.well-known/`, `/cpanel`, `/whm`, `/webmail`, `/cgi-sys/` | yes | Path prefixes that are not counted (at most 200, each starting with `/`). |
| `count_head_requests` | `false` | yes | Count `HEAD` and `OPTIONS` requests. |
| `static_prefixes` | `/wp-content/uploads/`, `/wp-includes/`, `/wp-json/`, `/wp-admin/admin-ajax.php`, `/wp-cron.php`, `/xmlrpc.php` | yes | Paths treated as static files, not pageviews. Built-in default; not in the shipped file. |
| `max_open_visits` | `50000` | no | Safety cap on visits kept open in memory per domain. Built-in default; not in the shipped file. When exceeded, the oldest idle visits are closed early and a warning is logged. |
| `php_bin` | `null` | no | PHP binary to run the CLI and WHM jobs with, if it differs from the interpreter that started `webpulse`. |

### GeoIP

| Key | Default | Editable | Meaning |
|-----|---------|----------|---------|
| `geoip.enabled` | `false` | yes | Look up the country of each visit. |
| `geoip.country_db` | `/var/webpulse/geoip/dbip-country-lite.mmdb` | no | Country database in MaxMind DB (`.mmdb`) format. |
| `geoip.city_db` | `null` | no | Optional city database. Without one the **Cities** card stays empty. |
| `geoip.url` | `https://download.db-ip.com/free/dbip-country-lite-{YYYY}-{MM}.mmdb.gz` | no | Download address used by `geoip-update`. Built-in default. |

### Paths

| Key | Default | Meaning |
|-----|---------|---------|
| `paths.data_dir` | `/var/webpulse/data` | One directory per account, one database per domain. |
| `paths.registry_db` | `/var/webpulse/webpulse.db` | Registry: accounts, switches, settings, salts, jobs. |
| `paths.log_dir` | `/var/webpulse/log` | `webpulse.log`, rotated at 5 MiB, five files kept. |
| `paths.run_dir` | `/var/webpulse/run` | Locks and the hook flag directory `hook/`. |
| `paths.domlogs_dirs` | `/etc/apache2/logs/domlogs/{user}`, `/etc/apache2/logs/domlogs`, `/usr/local/apache/domlogs/{user}`, `/var/log/nginx/domains` | Where to look for `<domain>` and `<domain>-ssl_log`. `{user}` is replaced by the account name. |
| `paths.archive_dir` | `/home/{user}/logs` | Archived `.gz` logs read by `backfill`. |
| `paths.userdomains_file` | `/etc/userdomains` | Domain to account map. |
| `paths.cpanel_users_dir` | `/var/cpanel/users` | Account files (domains, owner). |
| `paths.home_root` | `/home` | Home directories. |
| `paths.cpanel_userdata_dir` | `/var/cpanel/userdata` | Built-in default; not in the shipped file. |
| `paths.cron_file` | `/etc/cron.d/webpulse` | Used by the cron-installed check. Built-in default. |
| `paths.hooks_file` | `/var/cpanel/hooks.yaml` | Built-in default. |

Changing `paths.*` is the only reason to edit `config.json` by hand; the file is not touched when you
change run-time settings.

## Run-time settings in the registry

Run-time settings override the file for the editable keys only. Inspect and change them:

```bash
webpulse config get                      # the effective configuration
webpulse config get raw_retention_days
webpulse config set raw_retention_days 90
webpulse config unset raw_retention_days # back to the file or default value
```

Values are validated. An out-of-range number, a bad CIDR or an unknown timezone is refused. The WHM
settings form writes the same registry entries.

## Per-user settings

Each account owner can adjust, per domain, in the cPanel **Settings** tab:

- timezone,
- excluded IPs and CIDR ranges,
- excluded path prefixes,
- dashboard preferences (default date range and theme).

These are stored in `/var/webpulse/data/<user>/user/settings.json`, written by the cPanel plugin
(running as the account) and read by the collector:

```json
{ "version": 1,
  "domains": {
    "example.com": { "timezone": "Europe/Rome", "exclude_ips": ["203.0.113.7"], "exclude_paths": ["/staging/"] }
  },
  "ui": { "default_range": "30d", "dark": "auto" }
}
```

Because the account owner writes this file, the collector treats it as untrusted. It must be a regular
file (not a symbolic link), at most 64 KiB, valid JSON, with at most 50 IPs or CIDRs and 50 path
prefixes per domain and a valid timezone. Anything else is ignored.

**Which timezone wins?** The account owner's setting for the domain, then the timezone set for the
account (`webpulse accounts enable <user> --timezone TZ`), then the server-wide setting, then the
server's own timezone. Excluded IPs and paths from the server and from the owner both apply.

## Where everything lives

| What | Path | Owner and mode |
|------|------|----------------|
| Code | `/usr/local/webpulse` (symlink to a release) | root 0755 |
| Configuration | `/etc/webpulse/config.json` | root 0600 |
| Registry | `/var/webpulse/webpulse.db` | root 0600 |
| Domain databases | `/var/webpulse/data/<user>/<domain>.db` | `root:<user>` 0640, directory 0750 |
| User settings | `/var/webpulse/data/<user>/user/settings.json` | owned by the user |
| GeoIP | `/var/webpulse/geoip/*.mmdb` | root 0644 |
| Log | `/var/webpulse/log/webpulse.log` | root 0600 |
| Locks and hook flags | `/var/webpulse/run/`, `run/hook/` | `run` 0711, `hook` 1777 |
| cPanel plugin | `/usr/local/cpanel/base/frontend/jupiter/webpulse/` | root 0755 |
| WHM plugin | `/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/` | root 0755 |
| AppConfig | `/var/cpanel/apps/webpulse.conf` | root 0600 |
| Cron | `/etc/cron.d/webpulse` | root 0644 |

## Installer environment variables

For testing or unusual layouts, the installer reads `WEBPULSE_PREFIX`, `WEBPULSE_FAKE_CPANEL=1`,
`WEBPULSE_CPANEL_ROOT`, `WEBPULSE_ETC`, `WEBPULSE_VAR`, `WEBPULSE_CRON_DIR`, `WEBPULSE_APPS_DIR`,
`WEBPULSE_DOWNLOAD_URL`, `WEBPULSE_SKIP_FIRST_COLLECT=1` and `WEBPULSE_VERBOSE=1`. Also `WEBPULSE_REPO`
and `WEBPULSE_REF` (same as `--repo` and `--ref`). See [[Development]] for how the tests use them.

## See also

- [[Command Line]]
- [[WHM Administration]]
- [[Privacy and GDPR]]
- [[How It Works]]
