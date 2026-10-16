# Architecture

## Goals and non-goals

* **Lightweight.** One language (PHP ≥ 8.1, the `/usr/local/cpanel/3rdparty/bin/php`
  that every cPanel server ships), SQLite via the `pdo_sqlite` extension, no
  Composer, no daemon, no external service, no JavaScript tracker on customer
  sites. Vanilla JS + inline SVG charts in the dashboard (no chart library).
* **Log based.** Everything is derived from the per-domain Apache/Nginx access
  logs (`domlogs`) that cPanel already writes in *combined* format.
* **Rotation proof.** State lives in SQLite; logs are tailed with
  inode/offset checkpoints and flushed by a cPanel hook before `cpanellogd`
  truncates or archives them.
* **Per-user isolation.** One SQLite file per domain, readable only by root and
  the owning cPanel account. The cPanel dashboard runs as the account and opens
  its own databases read-only. No privilege escalation path.
* Non-goals (roadmap): e-mail reports, funnels/goals, JS tracker, multi-server.

## Components

```
                    ┌──────────────────────────────────────────────┐
                    │ /etc/apache2/logs/domlogs/<user>/<domain>    │
                    │ /etc/apache2/logs/domlogs/<user>/<domain>-ssl_log │
   Apache/Nginx ───▶│ /var/log/nginx/domains/<domain>[-ssl_log]    │
                    │ /home/<user>/logs/<domain>-<Mon>-<YYYY>.gz   │ (archives, backfill)
                    └───────────────┬──────────────────────────────┘
                                    │ tail from checkpoint (cron */5, Stats::RunUser pre-hook)
                                    ▼
                          ┌──────────────────┐
                          │  bin/webpulse    │  collect / backfill / rollup / status / accounts …
                          │  (PHP CLI, root) │
                          └───────┬──────────┘
            Discovery ◀───────────┤──────────▶ Registry  /var/webpulse/webpulse.db
   /etc/userdomains               │             (accounts, domain switches, settings, jobs, salts)
   /var/cpanel/users/<u>          │
                                  ▼
                 LogParser → Classifier (UA, referrer, path, GeoIP) → Sessionizer → DomainStore
                                  │
                                  ▼
                 /var/webpulse/data/<user>/<domain>.db   (0640 root:<user>, dir 0750 root:<user>)
                                  │
          ┌───────────────────────┼─────────────────────────────┐
          ▼                       ▼                             ▼
 cPanel plugin (LiveAPI PHP)   WHM plugin (PHP CGI, root)     bin/webpulse serve (dev)
 runs as the account,          sees every domain, toggles     php -S router, fake principal
 opens own DBs read-only       accounts, settings, jobs
          └───────────────────────┴─────────────────────────────┘
                              WebPulse\Http\Api  +  WebPulse\Http\Page (shared SPA, inline JS/CSS)
```

### Collector pipeline (`webpulse collect`)

1. **Discovery** builds the list of `(user, domain, [logfiles])` triples:
   * users: directory names in `/var/cpanel/users/` (skip `system`, dotfiles);
     `OWNER=` line gives the reseller, `DNS=`/`DNS<n>=` the domains;
   * domains: `/etc/userdomains` (`domain: user` lines, skip `*`), merged with
     the files that exist in the domlog directories;
   * for each domain the candidate log files are `<domain>` and
     `<domain>-ssl_log` in every configured `domlogs_dirs` entry
     (default `/etc/apache2/logs/domlogs/<user>`, `/etc/apache2/logs/domlogs`,
     `/usr/local/apache/domlogs/<user>`, `/var/log/nginx/domains`);
     files named `*-bytes_log`, `*-ftp_log`, `ftpxferlog*`, `*.bkup*`,
     `*.offset*` are ignored;
   * the Registry decides whether collection is enabled
     (`accounts.enabled` override → global `default_enabled`; then
     `domains.enabled` override).
2. **LogSource** opens each file at its checkpoint `(inode, offset)`.
   Rotation/truncation rules: inode changed → start at 0 of the new file;
   size < offset → file truncated, start at 0; otherwise seek to offset.
   Reads in 1 MiB chunks, never loads a file in memory, stops at the last
   complete line, stores the new offset only after the DB transaction commits.
   A `.gz` source (backfill) is streamed with `gzopen`.
3. **LogParser** parses *combined* lines with a tolerant regex (optional
   leading `%v` vhost, optional trailing fields, `-` for empty size, IPv6,
   malformed request lines such as `"-"` or `"\x16\x03..."`). Unparseable
   lines are counted in `checkpoints.bad_lines`, never fatal.
4. **Classifier** enriches a hit (pure functions, heavily unit-tested):
   * `UserAgent` → browser, browser_version, os, device
     (desktop/mobile/tablet/bot/other), bot name;
   * `Referrer` → referrer_host, referrer_url, channel, search_engine, keyword;
   * `Path` → clean path (query stripped except UTM), file_type, is_static,
     is_download, utm_source/medium/campaign;
   * `GeoIp` → ISO country (and city when a city db is installed), via a pure
     PHP MaxMind DB (`.mmdb`) reader; returns `null` without a database.
5. **Sessionizer** turns hits into visits (see METRICS.md): visitor hash =
   `substr(sha256(daily_salt . ip . ua), 0, 16)`; a new visit starts when
   the visitor has no open visit or the gap since its last hit is ≥ 30 min.
   Open visits are kept in the `open_visits` table of the domain DB so a
   visit survives across cron runs; visits whose last hit is older than
   30 min are closed at the end of each run (engagement buckets are written
   then, closed visits are copied to `visits`).
6. **DomainStore** accumulates all counters in memory during a run and flushes
   them in a single transaction per log file (batched `INSERT … ON CONFLICT DO
   UPDATE`), so a 1 M-line log costs a few seconds and the DB is locked for
   milliseconds at a time. Target throughput ≥ 20 000 lines/s on commodity
   hardware.
7. **Rollup** (nightly 02:10): closes stale visits, caps high-cardinality
   dimensions per day to `max_rows_per_dim` (rest folded into `(other)`),
   purges raw tables older than `raw_retention_days`, prunes `visitor_day`
   and `salts`, refreshes `registry.domain_summary`, runs `PRAGMA optimize`,
   `VACUUM` when free pages > 25 %.

### Concurrency
* `collect` takes a lock file `/var/webpulse/run/collect.lock` (flock, non
  blocking; a second instance exits 0 with a notice). A per-user lock allows
  the Stats hook (one user) to run while cron (all users) is running: cron
  skips users whose lock is held.
* Domain DBs use `journal_mode=DELETE` (not WAL) so that the account can read
  them with **only read permission on the file** (WAL needs a writable `-shm`).
  `busy_timeout=5000` on both sides. The dashboard opens with
  `PDO::SQLITE_ATTR_OPEN_FLAGS => PDO::SQLITE_OPEN_READONLY`.

## Paths

| Purpose                 | Path                                              | Owner / mode          |
|-------------------------|---------------------------------------------------|-----------------------|
| Code                    | `/usr/local/webpulse`                             | root 0755             |
| Config                  | `/etc/webpulse/config.json`                       | root 0600             |
| Registry DB             | `/var/webpulse/webpulse.db`                       | root 0600             |
| Domain DBs              | `/var/webpulse/data/<user>/<domain>.db`           | root:<user> 0640, dir 0750 |
| User settings           | `/var/webpulse/data/<user>/user/settings.json`    | <user> dir 0700 (written by the cPanel UI, read by the collector with symlink and size checks) |
| GeoIP                   | `/var/webpulse/geoip/*.mmdb`                      | root 0644             |
| Logs                    | `/var/webpulse/log/webpulse.log` (rotated by size, 5 × 5 MiB) | root 0600 |
| Locks                   | `/var/webpulse/run/`                              | root 0700             |
| cPanel plugin files     | `/usr/local/cpanel/base/frontend/jupiter/webpulse/` | root 0755           |
| WHM plugin CGI          | `/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/` | root 0755            |
| WHM icon                | `/usr/local/cpanel/whostmgr/docroot/addon_plugins/webpulse.png` | root 0644 |
| AppConfig               | `/var/cpanel/apps/webpulse.conf`                  | root 0600             |
| Cron                    | `/etc/cron.d/webpulse`                            | root 0644             |

Every path is overridable in `config.json` (`paths` section) and by the
`WEBPULSE_CONFIG` environment variable, which is how the test-suite and the dev
server run on a machine without cPanel (see DEVELOPMENT.md).

## Configuration (`/etc/webpulse/config.json`)

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
  "paths": {
    "data_dir": "/var/webpulse/data",
    "registry_db": "/var/webpulse/webpulse.db",
    "log_dir": "/var/webpulse/log",
    "run_dir": "/var/webpulse/run",
    "domlogs_dirs": ["/etc/apache2/logs/domlogs/{user}", "/etc/apache2/logs/domlogs", "/usr/local/apache/domlogs/{user}", "/var/log/nginx/domains"],
    "archive_dir": "/home/{user}/logs",
    "userdomains_file": "/etc/userdomains",
    "cpanel_users_dir": "/var/cpanel/users",
    "home_root": "/home"
  }
}
```
`timezone: null` means the server's `date_default_timezone_get()`. Values the
WHM UI can edit are stored in `registry.settings` and override the file, so the
file is only touched by the installer.

## Security model

* Collector and WHM plugin run as root; the cPanel plugin runs as the account.
* Account → domain authorization in the cPanel plugin is **not** derived from
  user input: the API lists `*.db` files in `/var/webpulse/data/<user>/` where
  `<user>` comes from the LiveAPI session (`uapi Variables::get_user_information`,
  fallback `REMOTE_USER` env). A domain parameter that does not match one of
  those files is a 404.
* The WHM plugin accepts root and resellers. Root sees all accounts; a reseller
  sees accounts whose `/var/cpanel/users/<u>` has `OWNER=<reseller>`. Account
  toggles are allowed for both within that scope; global settings only for root.
* Mutating API calls require `POST` + header `X-WebPulse-Request: 1`; cPanel/WHM
  already enforce their security token in the URL path, which kills CSRF.
* Every value rendered by the SPA is inserted with `textContent`, never
  `innerHTML` with untrusted data (paths, referrers and user agents come from
  attackers).
* SQL only via prepared statements; dimension names are whitelisted.
* The collector reads user-writable `settings.json` with `lstat` (no symlinks),
  a 64 KiB size cap, strict JSON schema, and never writes into user space.
