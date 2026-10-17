# Roadmap

This page lists ideas that the project documents as **non-goals for now** or as deferred. **None of
them is implemented** in version 0.2.0. They are not promises and have no dates. If one matters to you,
open an issue and say why.

| Idea | Status | Where it is mentioned |
|------|--------|-----------------------|
| E-mail reports | Not implemented | Listed as a non-goal and roadmap item in the architecture document |
| Goals and funnels | Not implemented | Same |
| JavaScript tracker | Not implemented | Same. WebPulse is deliberately log-based. |
| Multi-server | Not implemented | Same. One installation reports on one server only. |
| World map of visitors | Not implemented | The dashboard document says a map is deferred; countries are shown as a ranked list with bars |
| Standalone mode (without cPanel) | Done in 0.6.0 (Apache/nginx discovery, Cockpit module) | See below |

## E-mail reports

A periodic summary mailed to account owners or the administrator. Today you can read the numbers in the
dashboard, print them with `webpulse report` and export CSV, but nothing is sent by e-mail.

## Goals and funnels

Counting conversions (a visit that reached a given page) and multi-step funnels. WebPulse reports pages,
entry and exit pages, and downloads, but it has no goal definitions and no funnel report. Log analysis
makes funnels hard without a tracker.

## JavaScript tracker

An optional script that sends events from the visitor's browser. It would give real time-on-page and
events such as clicks. The project goal is the opposite: nothing on customers' sites. It is a listed
non-goal.

## Multi-server

Aggregating several servers into one view. Each WebPulse installation has its own registry and
databases; there is no shared view.

## World map

The Audience tab and the Countries card show a ranked list with flags and bars. A map is deferred.

## Standalone mode

Done in 0.6.0: `install.sh --standalone` installs WebPulse on a plain AlmaLinux/RHEL/Debian/Ubuntu web
server (sites from the Apache and nginx configuration or an explicit `sites` list, logrotate-aware
tailing, cron), with the dashboard in a **Cockpit module** or behind a reverse proxy with
`webpulse serve` ([[Standalone Install]], [[Cockpit Module]]). Still planned: RPM and DEB packages
built by the release workflow, and splitting a `vhost_combined` log shared by several virtual hosts
by its `%v` field.

## Things that need verification

These are not features, but known gaps in what has been checked:

- The cPanel registration steps (`install_plugin`, `register_appconfig`, `manage_hooks`) and the icon
  placement have not been run on a real cPanel server by the project.
- Support for themes other than Jupiter.
- The Cockpit module has been tested against a mock of Cockpit's API in a headless browser; log in to
  a real Cockpit to check the menu entry, administrative access and printing.
- Behaviour on very large servers (hundreds of domains); the WHM bot-share figure is marked partial
  above 300 domains.

## See also

- [[Home]]
- [[How It Works]]
- [[Development]]
- [Open issues](https://github.com/inalto/webpulse/issues)
