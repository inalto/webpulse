# Installation

This page explains how to install WebPulse on a cPanel & WHM server with the installer script. If you
cannot pipe a script into a shell, see [[Manual Installation]].

> WebPulse 0.1.0 was developed without a real cPanel server. The installer is tested against a fake
> cPanel root, but the calls to cPanel's own tools (`install_plugin`, `register_appconfig`,
> `manage_hooks`) have not yet been verified on a live server. If a step fails, the installer undoes
> what it did and prints the failing command. Please report what you see.

## Requirements

| Requirement | Detail |
|-------------|--------|
| Control panel | cPanel & WHM with the **Jupiter** theme. The documentation targets cPanel 110 or later. The installer does not check the cPanel version. |
| Operating system | AlmaLinux, Rocky Linux, CloudLinux or Ubuntu (the systems cPanel supports). |
| Web server | Apache (EasyApache 4) or ea-nginx, writing one access log per domain in the **combined** format (the cPanel default). |
| PHP | 8.1 or later with `pdo_sqlite`. cPanel's `/usr/local/cpanel/3rdparty/bin/php` qualifies; otherwise the installer uses `php` from `PATH`. |
| Privileges | You must run the installer as **root**. |
| Tools | `curl` or `wget`, `tar`, `cp`, `mv`, `ln`, `rm`, `mkdir`, `chmod`, `find`, `sed`, `grep`, `diff`. |
| Network | Outbound HTTPS to `codeload.github.com`, and to `download.db-ip.com` for GeoIP. |

## One-line install

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash
```

With flags, put them after `bash -s --`:

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --geoip
```

To read the script first, download it, read it, then run it:

```bash
curl -fsSLO https://raw.githubusercontent.com/inalto/webpulse/main/install.sh
less install.sh
bash install.sh --geoip
```

From a git checkout or an unpacked release, `bash install.sh` installs the code next to it.
When stdin is not a terminal, as in `curl | bash`, the installer does not ask for confirmation.

## Flags

| Flag | Meaning |
|------|---------|
| `--geoip` | Set `geoip.enabled` and download the free DB-IP country database. A failed download is only a warning. |
| `--ref <branch\|tag>` | Install this git ref. Default `main` (env `WEBPULSE_REF`). Use a tag only after it has been published. |
| `--repo <owner/name>` | Install from this GitHub repository. Default `inalto/webpulse` (env `WEBPULSE_REPO`). |
| `--no-cron` | Do not write `/etc/cron.d/webpulse`. Nothing collects data until you schedule `webpulse collect`. |
| `--update` | Update an existing installation. See [[Updating and Uninstalling]]. |
| `--uninstall` | Remove WebPulse; statistics and configuration are kept. |
| `--purge` | With `--uninstall`: also delete `/var/webpulse` and `/etc/webpulse`. |
| `--standalone` | No cPanel: install only the CLI and the dev server. No plugins, hooks or cron. |
| `--all-themes` | Install the cPanel plugin into every theme under `/usr/local/cpanel/base/frontend/`. |
| `--theme <name>` | Also install into this theme. Repeatable. |
| `-y`, `--yes` | Do not ask for confirmation. |
| `-h`, `--help` | Print the options. |

`--repo` accepts `owner/name` only; `--ref` accepts letters, digits and `. _ / + -`.

## What happens, step by step

1. **Checks.** Root, `/usr/local/cpanel/cpanel`, a suitable PHP, the required tools.
2. **Download.** `https://codeload.github.com/<repo>/tar.gz/<ref>` is unpacked in a temporary
   directory. An archive without `bin/webpulse` and `installer/lib.sh` is refused.
3. **Staging.** The release goes to `/usr/local/webpulse-releases/<version>/`. The shebang of
   `bin/webpulse` is rewritten to the PHP the installer found.
4. **Setup.** `webpulse setup` creates `/etc/webpulse/config.json` (mode 0600), `/var/webpulse/`
   (`data/`, `log/`, `run/`, `geoip/`), the registry database, and a data directory per cPanel account.
5. **Activation.** `/usr/local/webpulse` becomes a symlink to the release, swapped atomically. The
   three newest releases are kept.
6. **cPanel plugin.** Files are copied to `/usr/local/cpanel/base/frontend/jupiter/webpulse/` and
   registered with `install_plugin`. The icon appears in the *Metrics* group.
7. **WHM plugin.** CGI scripts go to `/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/`, the icon to
   `.../addon_plugins/webpulse.png`, and `register_appconfig` creates `/var/cpanel/apps/webpulse.conf`.
   `cpsrvd` is restarted only if the registration says it must be.
8. **Hooks.** `manage_hooks add script` registers `stats_runall_pre.sh` (`Stats::RunAll`) and
   `stats_runuser_pre.sh` (`Stats::RunUser`), stage `pre`. A hook that is already registered is
   skipped. `/var/webpulse/run/hook` is created with mode 1777.
9. **Cron.** `/etc/cron.d/webpulse` is written (see [[Command Line]] for the schedule).
10. **GeoIP** (only with `--geoip`) and a **first collection** started in the background.
11. **Summary** with the WHM and cPanel URLs, the config path and the uninstall command.

If a step fails, the installer prints `Step failed`, the command, its exit status and its output, then
rolls back. Fix the cause and run it again.

## Verify the install

```bash
/usr/local/webpulse/bin/webpulse status
/usr/local/cpanel/bin/manage_hooks list | grep webpulse     # two entries
cat /etc/cron.d/webpulse
ls -l /var/cpanel/apps/webpulse.conf
```

In WHM, open **Plugins » WebPulse Analytics**. The **Server** tab has an *Integration* tile that shows
whether the cron job, the Stats hook and GeoIP are in place.

## First collection

The installer starts one `collect` in the background. After that, cron runs it every 5 minutes. A
domain has no database until its first log line has been processed, and until then the dashboard says
*Collecting…*. To see progress or force a run:

```bash
/usr/local/webpulse/bin/webpulse status
/usr/local/webpulse/bin/webpulse collect --verbose
```

To import history that already sits in `/home/<user>/logs/*.gz`:

```bash
/usr/local/webpulse/bin/webpulse backfill --user <user>
```

By default collection is **on for every account**. Switch accounts off in WHM or with
`webpulse accounts disable <user>`. See [[WHM Administration]].

## Access for account owners

Open **cPanel » Metrics » WebPulse Analytics**. If an account does not see the icon, tick the
`webpulse` feature ("WebPulse Analytics") in **WHM » Feature Manager** for its feature list. See
[[Troubleshooting and FAQ]].

## Enabling GeoIP later

Without GeoIP, countries show as unknown. To enable it after installation:

```bash
/usr/local/webpulse/bin/webpulse config set geoip.enabled true
/usr/local/webpulse/bin/webpulse geoip-update
```

Or run the installer again with `--update --geoip`. The database is a free DB-IP country file; the
cron job refreshes it on the 1st of each month. Countries appear only for hits collected after the
database is in place. Check the terms of the DB-IP free database for your use.

## Installing into other themes

The plugin is installed into the `jupiter` theme by default. For another theme, add it:

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --update --theme paper_lantern
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --update --all-themes
```

A theme that does not exist under `/usr/local/cpanel/base/frontend/` is skipped with a warning.
`jupiter` is the only theme the project was written for.

## See also

- [[Manual Installation]]
- [[Updating and Uninstalling]]
- [[Troubleshooting and FAQ]]
- [[Configuration]]
