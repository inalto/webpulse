# WebPulse

WebPulse is a lightweight web-analytics plugin for **cPanel & WHM**. It reads the access logs that
Apache or Nginx already write for every domain and turns them into a dashboard: visits, pageviews,
where visitors come from, which pages they read, which links are broken, how much traffic is bots.

There is nothing to add to your customers' websites. No JavaScript tracker, no cookie, no external
service. Since 0.2.0 WebPulse is a **single static Rust binary** with SQLite built in: nothing else has to
be installed on the server.

![The WebPulse overview](https://raw.githubusercontent.com/inalto/webpulse/main/docs/screenshots/overview-1280-light.png)

> **Project status.** Version 0.2.0 (a rewrite of the 0.1.0 PHP code in Rust, same data and same
> dashboard) is not released yet. WebPulse was developed without access to a real cPanel server. The engine, API and dashboard are covered by automated tests that run against a
> fake server root; the cPanel-specific registration steps follow the cPanel developer documentation
> but must be verified on a real server. Test on a non-production machine first.

## Who is it for?

- **Hosting providers and server administrators** who want a built-in statistics tool for all accounts,
  with an on/off switch per account and per domain, and no third-party service.
- **Resellers**, who get the same administration view limited to the accounts they own.
- **Account owners**, who get a dashboard for each of their domains under *cPanel » Metrics*.
- **Anyone who cares about privacy.** IP addresses are not stored; visitors are not tracked across days.

## Feature overview

| Area | What you get |
|------|--------------|
| Collection | Reads domain logs every 5 minutes from a checkpoint; a cPanel hook drains the logs before `cpanellogd` truncates them. Archived `.gz` logs can be imported (backfill). |
| Storage | One SQLite database per domain. Daily aggregates are kept forever; individual visits for 30 days by default. |
| Traffic | Visits, unique visitors, pageviews, pages per visit, average duration, bounce rate, bandwidth, with comparison to the previous period. |
| Acquisition | Channels (direct, organic, social, referral, paid, email, campaign), referrers, search engines, UTM campaigns. |
| Audience | Countries and cities (with GeoIP), browsers, operating systems, devices. |
| Content | Top pages, entry pages with bounce rate, exit pages with exit rate, downloads. |
| Technical | HTTP status, 404s with their referrers, 5xx errors, bandwidth by file type, heaviest static files, HTTP vs HTTPS. |
| Bots | Crawler share over time, top crawlers, AI crawlers highlighted. |
| Administration | Server overview, accounts with Default/On/Off switches, settings, collect/backfill/rollup jobs. |
| Privacy | Daily-salted visitor hashes, no cookies, optional IP storage (off by default), retention control. |
| Interface | Light and dark theme, phone friendly, CSV export of every table. |

![The WHM server view](https://raw.githubusercontent.com/inalto/webpulse/main/docs/screenshots/server-1280-dark.png)

## Install in one line

On a cPanel server, as root:

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash
```

Then open **WHM » Plugins » WebPulse Analytics**. See [[Installation]] for requirements, flags and
what the installer does.

## Page index

**Getting started**

- [[Installation]] - requirements, the one-line installer, flags, air-gapped and source builds, first collection
- [[Updating and Uninstalling]] - update in place, roll back, remove
- [[Manual Installation]] - every command the installer runs
- [[Troubleshooting and FAQ]] - no data, missing icons, permission errors

**Using WebPulse**

- [[Using the Dashboard]] - tabs, cards, date ranges, CSV export
- [[WHM Administration]] - accounts, switches, settings, jobs, resellers
- [[Metrics and Definitions]] - what a visit, a bounce or a bot is
- [[Privacy and GDPR]] - what is stored and what is not

**Reference**

- [[Command Line]] - every command with options and exit codes
- [[Configuration]] - `config.json`, run-time settings, per-user settings, paths
- [[How It Works]] - architecture for curious administrators

**Project**

- [[Development]] - build with cargo, run the tests and the dashboard without cPanel, cut a release
- [[Roadmap]] - ideas that are not implemented
- [Changelog](https://github.com/inalto/webpulse/blob/main/CHANGELOG.md) and
  [source code](https://github.com/inalto/webpulse)

## See also

- [README](https://github.com/inalto/webpulse#readme)
- [Technical documents in the repository](https://github.com/inalto/webpulse/tree/main/docs)
- License: MIT
