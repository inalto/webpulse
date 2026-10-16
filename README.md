# WebPulse — lightweight web analytics for cPanel & WHM

WebPulse turns the Apache/Nginx access logs that cPanel already writes for every
domain into a modern, privacy-friendly analytics dashboard, with **zero JavaScript
tracker, zero external services and zero dependencies** beyond the PHP and SQLite
that ship with cPanel.

* **cPanel plugin** – every account owner gets a "WebPulse Analytics" icon in the
  *Metrics* group and sees a dashboard for each of their domains.
* **WHM plugin** – the administrator enables or disables collection per account
  (and per domain), sees a server-wide overview, tunes retention and GeoIP and
  can trigger collection or a backfill from archived logs.
* **One SQLite database per domain** – aggregates are kept forever, raw visit
  detail for a configurable number of days, so log rotation and cPanel's
  "delete access logs after stats" setting never lose history.
* **Near real-time** – a cron job tails the domlogs every 5 minutes with
  inode/offset checkpoints; a cPanel `Stats::RunUser` pre-hook flushes the logs
  right before `cpanellogd` truncates them, so nothing is lost.
* **Meaningful graphs** inspired by Matomo, Plausible, GoAccess and AWStats:
  visits/pageviews over time with period comparison, hour-of-day × weekday
  heatmap, acquisition channels, search engines & keywords, campaigns (UTM),
  countries, devices/browsers/OS, top/entry/exit pages with bounce & exit
  rates, 404s with referrers, 5xx trend, bandwidth by file type, downloads,
  crawler share, and a Matomo-style visits log.
* **Privacy by default** – IP addresses are hashed with a daily rotating salt
  and never stored; no cookies; GDPR friendly.

## Install (cPanel/WHM server, as root)

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash
```

Options are passed through environment variables or flags, e.g.

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --geoip
```

| Flag / env                 | Meaning                                                   |
|----------------------------|-----------------------------------------------------------|
| `--geoip`                  | download the free DB-IP country database for geolocation |
| `--ref <branch\|tag>`      | install a specific git ref (`WEBPULSE_REF`)               |
| `--repo <owner/name>`      | install from a fork (`WEBPULSE_REPO`)                     |
| `--no-cron`                | do not install `/etc/cron.d/webpulse`                     |
| `--theme <name>`           | also install the cPanel plugin into another theme (repeatable) |
| `--all-themes`             | install the cPanel plugin into every theme under `base/frontend/` |
| `--standalone`             | no cPanel: install only the CLI and the dev dashboard     |
| `-y`                       | skip the confirmation (implied when stdin is not a terminal) |
| `--update`                 | update an existing installation in place                  |
| `--uninstall`              | remove plugins, cron, hooks and code (data is kept unless `--purge`) |

After installation open **WHM » Plugins » WebPulse Analytics** to review which
accounts are enabled (all accounts are enabled by default) and run the first
collection. Account owners find **cPanel » Metrics » WebPulse Analytics**.

## Command line

```
webpulse collect [--user U] [--domain D]   # tail logs, update stats (cron does this)
webpulse backfill --user U [--domain D]    # import /home/U/logs/*.gz archives
webpulse rollup                            # nightly compaction + retention
webpulse status [--json]                   # collection health
webpulse accounts list|enable|disable|reset <user>
webpulse domains  enable|disable <user> <domain>
webpulse report --user U --domain D --range 30d
webpulse serve --port 8099 --user U        # local dev dashboard (no cPanel needed)
webpulse config get|set <key> [value]      # run-time settings (also editable in WHM)
webpulse purge --user U [--domain D] --yes # delete statistics (irreversible)
```

Every command accepts `--config <file>`, `--verbose`, `--quiet`, `--json` and `--help`.

## How it works in one paragraph

Every 5 minutes (and right before cPanel's own `cpanellogd` statistics run,
through a `Stats::RunAll`/`Stats::RunUser` hook) the collector tails each
enabled domain's `domlogs` from an inode/offset checkpoint, parses the
*combined* log lines, classifies them (pageview, static asset, download,
redirect, error, bot), groups human hits into 30-minute visits using a
visitor hash salted per day (no IP is stored), and upserts daily aggregates
plus a short-lived raw visits log into the domain's SQLite file. The
dashboards only read those files. Measured throughput: ≈ 30 000 lines/s,
26 MiB peak memory.

## Screenshots

![Overview](docs/screenshots/overview-1280-light.png)
![WHM server view](docs/screenshots/server-1280-dark.png)

## Requirements

* cPanel & WHM 110 or later (Jupiter theme) on AlmaLinux / Rocky / CloudLinux / Ubuntu
* PHP ≥ 8.1 with `pdo_sqlite` — the bundled `/usr/local/cpanel/3rdparty/bin/php` qualifies
* Apache (EA4) or ea-nginx writing per-domain access logs in *combined* format


## Documentation

* [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) – components, data flow, paths, security model
* [docs/DATA-MODEL.md](docs/DATA-MODEL.md) – SQLite schemas
* [docs/METRICS.md](docs/METRICS.md) – exact definitions of visit, pageview, bounce, bot…
* [docs/API.md](docs/API.md) – JSON API used by the dashboards
* [docs/DASHBOARD.md](docs/DASHBOARD.md) – every report and graph, and why it is there
* [docs/CPANEL-INTEGRATION.md](docs/CPANEL-INTEGRATION.md) – plugin registration, hooks, log locations
* [docs/INSTALL.md](docs/INSTALL.md) – manual installation, update, uninstall, troubleshooting
* [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) – running the test-suite and the dev dashboard without cPanel

## License

MIT — see [LICENSE](LICENSE).
