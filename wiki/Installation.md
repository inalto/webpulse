# Installation

This page explains how to install WebPulse on a cPanel & WHM server with the installer script. If you
cannot pipe a script into a shell, see [[Manual Installation]].

> WebPulse 0.2.0 (the Rust rewrite) was developed without a real cPanel server. The installer is tested against a fake
> cPanel root, but the calls to cPanel's own tools (`install_plugin`, `register_appconfig`,
> `manage_hooks`) have not yet been verified on a live server. If a step fails, the installer undoes
> what it did and prints the failing command. Please report what you see.

## Requirements

| Requirement | Detail |
|-------------|--------|
| Control panel | cPanel & WHM with the **Jupiter** theme. The documentation targets cPanel 110 or later. The installer does not check the cPanel version. |
| Operating system | AlmaLinux, Rocky Linux, CloudLinux or Ubuntu (the systems cPanel supports). |
| Web server | Apache (EasyApache 4) or ea-nginx, writing one access log per domain in the **combined** format (the cPanel default). |
| CPU | Linux **x86_64** or **aarch64**. The prebuilt binary is statically linked and runs on every distribution. |
| Runtime | Nothing to install. WebPulse is a single binary; there is no PHP library, no interpreter and no SQLite package to add. cPanel's own PHP (`/usr/local/cpanel/3rdparty/bin/php`) runs the two small shims of the cPanel page, and the installer only warns when it is missing. |
| Privileges | You must run the installer as **root**. |
| Tools | `curl` or `wget`, `tar`, `sha256sum`, `cp`, `mv`, `ln`, `rm`, `mkdir`, `chmod`, `find`, `sed`, `grep`, `awk`, `diff`. |
| Network | Outbound HTTPS to `github.com` and `api.github.com`, and to `download.db-ip.com` for GeoIP. Not needed for [air-gapped installs](#air-gapped-servers-and-your-own-build). |

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

The installer downloads the release for your CPU (`webpulse-<version>-x86_64-unknown-linux-musl.tar.gz` or
`...-aarch64-...`), verifies its **sha256 checksum** and only then changes anything. From an unpacked
release tarball, `bash install.sh` installs the files next to it.
When stdin is not a terminal, as in `curl | bash`, the installer does not ask for confirmation.

### Air-gapped servers and your own build

* **No internet on the server.** On another machine download the release tarball for the server's CPU
  and its `.sha256` from the [releases page](https://github.com/inalto/webpulse/releases), check it with
  `sha256sum -c`, copy it over, then `tar -xzf` it and run `./install.sh` inside. It finds `bin/webpulse`
  next to itself and downloads nothing.
* **Your own binary.** `./install.sh --binary /path/to/webpulse` installs that binary instead of the
  release's.
* **Build on the server.** `install.sh --build` downloads the source of `--ref` and runs
  `cargo build --release --locked` (needs the Rust toolchain, <https://rustup.rs>). The installer does the
  same on its own when a release has no binary for your architecture and `cargo` is present; without
  `cargo` it stops with a message that names the three options.

## Flags

| Flag | Meaning |
|------|---------|
| `--geoip` | Set `geoip.enabled` and download the free DB-IP country database. A failed download is only a warning. |
| `--ref <tag>` | Install this release, `v0.2.0` or `0.2.0`. Default `main`, which means **the latest release** (env `WEBPULSE_REF`). A branch name is built from source (needs `cargo`). |
| `--repo <owner/name>` | Install from this GitHub repository. Default `inalto/webpulse` (env `WEBPULSE_REPO`). |
| `--build` | Build the binary from source with `cargo` instead of downloading it. Fails clearly when `cargo` is missing. |
| `--binary <file>` | Install this binary instead of the release's. |
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

1. **Checks.** Root, `/usr/local/cpanel/cpanel`, the required tools.
2. **Download.** The installer maps `uname -m` to `x86_64-unknown-linux-musl` or
   `aarch64-unknown-linux-musl`, finds the latest release (or the `--ref` tag), downloads
   `webpulse-<version>-<target>.tar.gz` and its `.sha256` from GitHub, **verifies the checksum** and
   unpacks the archive in a temporary directory. A mismatch, or an archive that does not look like
   WebPulse, stops the installer before anything is changed.
3. **Staging.** The release goes to `/usr/local/webpulse-releases/<version>/`; `bin/webpulse` (mode 0755) is
   started once with `--version` to check that it runs on this machine.
4. **Setup.** `webpulse setup` creates `/etc/webpulse/config.json` (mode 0600), `/var/webpulse/`
   (`data/`, `log/`, `run/`, `geoip/`), the registry database, and a data directory per cPanel account.
5. **Activation.** `/usr/local/webpulse` becomes a symlink to the release, swapped atomically. The
   three newest releases are kept.
6. **cPanel plugin.** The two small PHP shims are copied to `/usr/local/cpanel/base/frontend/jupiter/webpulse/` and
   registered with `install_plugin`. The icon appears in the *Metrics* group.
7. **WHM plugin.** The two bash CGI shims go to `/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/`, the icon to
   `.../addon_plugins/webpulse.png`, and `register_appconfig` creates `/var/cpanel/apps/webpulse.conf`.
   `cpsrvd` is restarted only if the registration says it must be.
8. **Hooks.** `manage_hooks add script` registers `stats_runall_pre.sh` (`Stats::RunAll`) and
   `stats_runuser_pre.sh` (`Stats::RunUser`), stage `pre`. A hook that is already registered is
   skipped. `/var/webpulse/run/hook` is created with mode 1777.
9. **Cron.** `/etc/cron.d/webpulse` is written (see [[Command Line]] for the schedule).
10. **GeoIP** (only with `--geoip`: `webpulse config set geoip.enabled true` and `webpulse geoip-update`) and a
    **first collection** started in the background.
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
