# How It Works

This page explains WebPulse for administrators who want to know what runs on their server, and why. It
is a summary; the full text is in
[docs/ARCHITECTURE.md](https://github.com/inalto/webpulse/blob/main/docs/ARCHITECTURE.md),
[docs/DATA-MODEL.md](https://github.com/inalto/webpulse/blob/main/docs/DATA-MODEL.md) and
[docs/CPANEL-INTEGRATION.md](https://github.com/inalto/webpulse/blob/main/docs/CPANEL-INTEGRATION.md).

## Design goals

- **Lightweight.** One static Rust binary with SQLite built in (since 0.2.0; 0.1.0 was PHP). No
  interpreter, no daemon, no external service, no tracker on customer sites. The dashboard is plain
  JavaScript with inline SVG charts and no chart library, embedded in the binary.
- **Log based.** Everything comes from the per-domain access logs that cPanel already writes.
- **Rotation proof.** State is kept in SQLite. A cPanel hook reads the logs before they are truncated.
- **Isolated per account.** One database per domain, readable only by root and the owning account.

## The pieces

```
access logs  ->  webpulse collect (root)  ->  /var/webpulse/data/<user>/<domain>.db
                      |                              |
                registry database          cPanel plugin (as the account, read-only)
                                           WHM plugin   (as root)
```

| Piece | Role |
|-------|------|
| `bin/webpulse` | The program: command line, collector, JSON API, page renderer and `webpulse cgi`. Cron and the hooks run it as root; the cPanel shim runs it as the account. |
| Registry (`/var/webpulse/webpulse.db`) | Accounts, domains, On/Off switches, run-time settings, daily salts, job history, a per-domain summary for the WHM overview. |
| Domain databases | Daily and hourly aggregates, one table for all report dimensions, open and closed visits, raw events, log checkpoints. |
| cPanel plugin | Two thin PHP shims (cPanel requires LiveAPI pages to be PHP) run by cPanel as the account user. They identify the account from the cPanel session and run `webpulse cgi --principal user:<account>`, which opens that account's databases read-only. |
| WHM plugin | Two bash CGI shims run by `cpsrvd` as root; they `exec webpulse cgi --whm`. Sees all accounts (or a reseller's). |
| Hooks | Two cPanel Standardized Hooks that make sure logs are read before `cpanellogd` truncates them. |

## What `collect` does

1. **Discovery.** Accounts come from `/var/cpanel/users/`, domains from `/etc/userdomains`, merged with
   the log files found in the configured directories. For each domain, `<domain>` and
   `<domain>-ssl_log` are the candidate files. Files such as `*-bytes_log`, `*-ftp_log` and `*.bkup*` are
   ignored. The registry decides whether the account and domain are enabled.
2. **Tailing with checkpoints.** For each file WebPulse remembers the inode, the offset, the size and a
   hash of the first 256 bytes. On the next run: a different inode means a new file, so start from
   the beginning; a size smaller than the offset means truncation, so start from the beginning;
   otherwise continue at the offset. The file is read in 1 MiB chunks up to the last complete line, and
   the new offset is saved only after the database transaction has committed. A crash leaves the
   checkpoint where it was.
3. **Parsing.** A tolerant parser reads *combined* lines, with an optional leading virtual host,
   IPv6 addresses and malformed request lines. Lines that cannot be parsed are counted, never fatal.
4. **Classifying.** The user agent gives browser, operating system, device and bot name. The referrer
   gives channel, search engine and keyword. The path gives file type, download and UTM parameters.
   GeoIP gives the country, using a built-in reader for `.mmdb` files.
5. **Sessionizing.** Hits become visits (see [[Metrics and Definitions]]). Open visits are stored in
   the database, so a visit continues across runs.
6. **Writing.** Counters are accumulated in memory and written in one transaction per log file, as
   upserts. The database is locked for milliseconds, not seconds.

### Concurrency

`collect` takes a lock file (`/var/webpulse/run/collect.lock`); a second run exits immediately with
success. A global run processes each account in its own child process, so a failure in one account
cannot stop the others. Each account also has a lock, so the hook can drain one account while cron works
on the others. Domain databases use SQLite's `journal_mode=DELETE`, not WAL, so an account can read
them with read permission only (WAL needs a writable side file).

## The cPanel hooks

By default cPanel deletes each domain's access log after its statistics run (WHM » Tweak Settings »
Stats and Logs, "Delete each domain's access logs after statistics are gathered"). Lines written since
WebPulse's last 5-minute run would be lost. Two hooks prevent that:

- `Stats::RunAll`, stage `pre`, runs as **root** before the whole cycle and runs `webpulse collect`
  synchronously (with a 20-minute cap).
- `Stats::RunUser`, stage `pre`, runs as the **account user**. It cannot write root-owned databases, so
  it only drops a flag file in `/var/webpulse/run/hook/` (mode 1777). The cron job
  `collect --hooked`, which runs every minute, sees the flag and drains that account within about a
  minute. cPanel needs longer than that to process an account, so the window is covered in practice.

Both hook scripts always exit 0, so they can never block cPanel's statistics.

## Permissions model

| Who | Can |
|-----|-----|
| root (collector, WHM plugin as administrator) | Read the logs, write all databases. |
| Account (cPanel plugin) | Read only its own `/var/webpulse/data/<user>/*.db`; write only its own `user/settings.json`. |
| Reseller (WHM plugin) | See and switch the accounts it owns. No global settings. |

The account's identity comes from cPanel's session, never from request parameters. The list of domains
an account can open is the list of database files in its own directory; any other domain name is
answered with "not found". Changes through the API require `POST` and a custom header, on top of the
security token that cPanel and WHM put in the URL. Values from logs (paths, referrers, user agents)
are attacker-controlled and are inserted into the page as text, never as HTML. All SQL uses prepared
statements. The collector reads the user-writable settings file with strict checks (see
[[Configuration]]).

## Rollup and retention

The nightly `webpulse rollup` (02:10):

- closes visits that timed out,
- folds the tail of large reports into `(other)` (`max_rows_per_dim` rows per report per day),
- deletes the visits log and raw events older than `raw_retention_days`,
- removes the unique-visitor bookkeeping and daily salts older than two days,
- refreshes the summary used by the WHM overview,
- runs `PRAGMA optimize` and `VACUUM` when more than a quarter of the file is free space.

Daily and hourly aggregates and the report tables are **never deleted**, which is why history survives
log rotation and cPanel's log deletion. A database grows with the number of distinct paths, referrers
and so on, not with traffic.

## Performance

The 0.1.0 PHP engine read about **30 000 log lines per second** in **26 MiB** on the development machine
with generated logs. The Rust engine is designed to be several times faster and lighter (no interpreter,
no `memory_limit` or `max_execution_time`); the numbers are to be re-measured on the release build. Real
servers vary with disk speed and log content. Because of the checkpoints, the routine 5-minute run only
reads the lines written since the previous one.

Aggregated reports grow with the number of distinct values (pages, referrers, countries), not with
traffic, and are capped per day by `max_rows_per_dim`. The visits log does grow with traffic, which is
why it is purged after `raw_retention_days`.

## Log

`/var/webpulse/log/webpulse.log` is rotated by size (5 files of 5 MiB). See [[Troubleshooting and FAQ]]
for how to read it.

## See also

- [[Metrics and Definitions]]
- [[Configuration]]
- [[Privacy and GDPR]]
- [[Development]]
