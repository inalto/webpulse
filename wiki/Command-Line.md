# Command Line

Everything the dashboards do can be done from the shell with `webpulse`. The program is
`/usr/local/webpulse/bin/webpulse`; run it as root on a cPanel server. The examples below write it as
`webpulse` for short. Add `/usr/local/webpulse/bin` to your `PATH` or type the full path.

```
webpulse <command> [options]
webpulse <command> --help
```

## Global options

Accepted by every command, anywhere on the line.

| Option | Meaning |
|--------|---------|
| `--config <file>` | Use this configuration file instead of `/etc/webpulse/config.json` (also the `WEBPULSE_CONFIG` environment variable). |
| `--verbose`, `-v` | Print more detail and mirror the log to stderr. |
| `--quiet`, `-q` | Suppress informational output. |
| `--json` | Print the result as JSON. |
| `--help`, `-h` | Show the help of the command. |
| `--version` | Print the version (`webpulse 0.2.0`). |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success. A second `collect` that finds another one running also exits 0. |
| 1 | Error (for example `status` cannot open the registry, a download failed, a key is unknown). |
| 2 | Usage error: unknown command, missing or invalid option. |

## Commands

### collect

Reads the access logs from their checkpoints and updates the statistics. Cron runs it every 5 minutes.

```
webpulse collect [--user USER [--domain DOMAIN]] [--hooked] [--job [ID]]
```

| Option | Meaning |
|--------|---------|
| `--user U` | Only this account. Waits for the account's lock. |
| `--domain D` | Only this domain of `--user`. |
| `--hooked` | Process the accounts flagged by the `Stats::RunUser` hook and remove the flags. Cannot be combined with `--user`. |
| `--job [ID]` | Record the run in the job history (WHM uses this). |

Without `--user`, every enabled account is processed, each in its own child process. A second concurrent
run exits immediately, and accounts whose lock is held are skipped.

```bash
webpulse collect --verbose
webpulse collect --user alice --domain example.com
```

### hooked

Same as `webpulse collect --hooked`. Cron runs it every minute; it does nothing when no flag exists.

### backfill

Imports archived logs, `/home/<user>/logs/*.gz`, that cPanel keeps after its own statistics run.

```
webpulse backfill --user USER [--domain DOMAIN] [--archive-dir DIR] [--job [ID]]
```

An archive that was imported (same path, size and modification time) is skipped, and lines newer than
the start of live collection are not imported again, so it is safe to repeat.

```bash
webpulse backfill --user alice
webpulse backfill --user alice --archive-dir /backup/alice/logs
```

### rollup

Nightly maintenance: closes stale visits, folds the tail of large reports into `(other)`
(`max_rows_per_dim`), deletes visits and events older than `raw_retention_days`, prunes visitor and
salt tables, refreshes the WHM summary, optimises and vacuums the database when more than 25 % is free.

```
webpulse rollup [--user USER [--domain DOMAIN]] [--job [ID]]
```

### status

Collection health: version, config path, the main paths, registry, accounts and domains, last
collection and the lag of the newest log line, cron, pending hook flags, GeoIP and the last jobs.

```bash
webpulse status
webpulse status --json
```

### accounts

```
webpulse accounts list [--owner RESELLER] [--q TEXT]
webpulse accounts enable|disable|reset USER [--timezone TZ]
```

`enable` and `disable` set an explicit override; `reset` returns to the server default.
`--timezone TZ` also sets the account's timezone (`""` for the server's).

### domains

```
webpulse domains list [USER]
webpulse domains enable|disable|reset USER DOMAIN
```

A domain override wins over the account setting, except when the account is explicitly disabled.

### report

Prints a report in the terminal.

```
webpulse report --user USER --domain DOMAIN [--range 30d | --from YYYY-MM-DD --to YYYY-MM-DD]
                [--name DIMENSION] [--limit 10] [--sort -visits] [--compare] [--json]
```

Without `--name` it prints the KPIs of the period (with the change against the previous period). With
`--name` it prints a ranked table. Ranges: `today yesterday 7d 30d 90d 12m month lastmonth year all`.
Dimensions: `page entry_page exit_page referrer_host referrer_url channel search_engine keyword campaign
utm_source utm_medium source_medium country city browser browser_version os os_version device status
method protocol http_version file_type static_file download not_found not_found_referrer server_error
bot visit_duration pages_per_visit`.

```bash
webpulse report --user alice --domain example.com --range 7d --compare
webpulse report --user alice --domain example.com --name not_found --limit 20
webpulse report --user alice --domain example.com --name country --json
```

### config

Shows or changes run-time settings.

```
webpulse config get [KEY]
webpulse config set KEY VALUE        (VALUE is JSON, or a plain string)
webpulse config unset KEY
```

Settings changed here are stored in the registry and win over `config.json`. Editable keys:
`default_enabled`, `timezone`, `raw_retention_days`, `max_rows_per_dim`, `session_timeout_min`,
`store_ip`, `exclude_ips`, `exclude_paths`, `count_head_requests`, `geoip.enabled`, `static_prefixes`.
Other keys (`paths.*`, `geoip.country_db`, `php_bin`) are edited in `config.json`. An invalid value
exits with code 2.

```bash
webpulse config get
webpulse config set raw_retention_days 90
webpulse config set exclude_ips '["203.0.113.7","198.51.100.0/24"]'
webpulse config unset raw_retention_days
```

### purge

Deletes statistics. **Irreversible.**

```
webpulse purge --user USER [--domain DOMAIN] --yes
```

Without `--domain`, every database of the account is deleted. If the account no longer exists in cPanel,
its data directory and registry rows are removed too. `--yes` is mandatory.

### geoip-update

Downloads the free DB-IP country database for the current month (the previous month if that fails),
validates it and replaces `geoip.country_db` atomically. It does nothing unless `geoip.enabled` is true,
or `--force` (download only; WHM's **Download database now** uses it so the file can be installed
before GeoIP is switched on).

```
webpulse geoip-update [--force] [--url URL] [--out FILE] [--job [ID]]
```

`--url` may contain `{YYYY}` and `{MM}` (default `geoip.url`); `file://` URLs and local paths work.
`--out` writes elsewhere. Each run that downloads is recorded in the job history (WHM's GeoIP card
shows the last one) when the registry exists; `--job ID` adopts a row created by WHM.

### update-check

Asks the GitHub API for the latest release of `update.repo` (default `inalto/webpulse`), compares it
with the installed version and stores the answer for the bell in WHM. The network is used at most
once every 20 hours unless `--force`; with `update.check` set to false it does nothing unless
`--force`. The nightly `webpulse rollup` runs the same check. Exit status 1 when the request failed.

```
webpulse update-check [--force] [--json]
```

To upgrade, use the bell in WHM or run `install.sh --update`.

### setup

Creates or repairs the configuration file, the data, log, run and geoip directories, the hook flag
directory (mode 1777), the registry (mode 0600) and one data directory per cPanel account. Idempotent.
The installer runs it; you rarely need to.

```
webpulse setup [--no-sync]
```

`--no-sync` skips creating the per-account directories.

### serve

A local development dashboard that needs no cPanel. See [[Development]].

```
webpulse serve [--port 8099] (--user USER | --admin | --reseller NAME) [--config FILE] [--allow-root]
```

It listens only on `127.0.0.1` and answers only requests that carry the random token in the printed
URL. It refuses to run as root unless `--allow-root` is given.

## Cron schedule

The installer writes `/etc/cron.d/webpulse`:

| Schedule | Command | Purpose |
|----------|---------|---------|
| `*/5 * * * *` | `webpulse collect --quiet` | Read new log lines for all enabled accounts |
| `* * * * *` | `webpulse collect --hooked --quiet` | Drain accounts flagged by the `Stats::RunUser` hook (cheap when there is nothing to do) |
| `10 2 * * *` | `webpulse rollup --quiet` | Nightly maintenance and retention |
| `30 4 1 * *` | `webpulse geoip-update --quiet` | Monthly GeoIP refresh (does nothing if GeoIP is off) |

All run as root. The file is managed by the installer; edits are overwritten on update. Use
`--no-cron` at install time if you want your own schedule.

## See also

- [[Configuration]]
- [[WHM Administration]]
- [[How It Works]]
- [[Troubleshooting and FAQ]]
