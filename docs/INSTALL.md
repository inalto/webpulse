# Installation

WebPulse installs on a cPanel & WHM server (Jupiter theme, EasyApache 4) as
`root`. It needs PHP >= 8.1 with `pdo_sqlite`: cPanel's own
`/usr/local/cpanel/3rdparty/bin/php` qualifies, otherwise the installer falls
back to `php` from `PATH`.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash
```

Pass flags after `bash -s --`:

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --geoip --ref v0.1.0
```

From a checkout or an unpacked release the same script installs the code it
sits next to: `sudo bash install.sh`.

| Flag                  | Meaning |
|-----------------------|---------|
| `--geoip`             | set `geoip.enabled`, download the free DB-IP country database (a failed download is only a warning; retry with `webpulse geoip-update`) |
| `--ref <branch\|tag>` | git ref to install, default `main` (env `WEBPULSE_REF`) |
| `--repo <owner/name>` | GitHub repository, default `inalto/webpulse` (env `WEBPULSE_REPO`) |
| `--no-cron`           | do not install `/etc/cron.d/webpulse` (then schedule `webpulse collect` yourself) |
| `--update`            | update an existing installation; safe to repeat |
| `--uninstall`         | remove everything WebPulse registered; statistics and config are kept |
| `--purge`             | with `--uninstall`: also delete `/var/webpulse` and `/etc/webpulse` |
| `--standalone`        | no cPanel: install only the CLI and the dev server (no plugins, hooks or cron) |
| `--all-themes`        | install the cPanel plugin into every theme under `/usr/local/cpanel/base/frontend/`, not only `jupiter` |
| `--theme <name>`      | additionally install into this theme (repeatable) |
| `-y`, `--yes`         | do not ask for confirmation (also implied when stdin is not a terminal, as with `curl \| bash`) |
| `-h`, `--help`        | print the options |

### What the installer does

1. Checks it runs as root, that `/usr/local/cpanel/cpanel` exists (unless `--standalone`) and that a
   suitable PHP is available.
2. Downloads `https://codeload.github.com/<repo>/tar.gz/<ref>` (or uses the current checkout), refuses
   archives without `bin/webpulse`.
3. Copies the release to `/usr/local/webpulse-releases/<version>/`, rewrites the shebang of
   `bin/webpulse` to the detected PHP and runs `bin/webpulse setup` (creates `/etc/webpulse/config.json`,
   `/var/webpulse/...`; idempotent).
4. Switches `/usr/local/webpulse` to the new release with an atomic symlink swap. The last three releases
   are kept.
5. Copies the plugin files into `/usr/local/cpanel/base/frontend/jupiter/webpulse/` and runs
   ```
   /usr/local/cpanel/scripts/install_plugin /usr/local/webpulse/cpanel/plugin --theme jupiter
   ```
6. Copies the CGI scripts to `/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/`, the icon to
   `.../addon_plugins/webpulse.png` and registers the WHM entry:
   ```
   /usr/local/cpanel/bin/register_appconfig /usr/local/webpulse/whm/webpulse.conf
   ```
   (`restartsrv_cpsrvd` runs only if `register_appconfig` says a restart is needed).
7. Registers the two standardized hooks (skipped when already listed by `manage_hooks list`):
   ```
   /usr/local/cpanel/bin/manage_hooks add script /usr/local/webpulse/hooks/stats_runall_pre.sh  --manual --category Stats --event RunAll  --stage pre
   /usr/local/cpanel/bin/manage_hooks add script /usr/local/webpulse/hooks/stats_runuser_pre.sh --manual --category Stats --event RunUser --stage pre
   ```
   and creates `/var/webpulse/run/hook` (mode 1777), where the unprivileged `RunUser` hook leaves its flag files.
8. Writes `/etc/cron.d/webpulse`, optionally sets up GeoIP, starts a first `webpulse collect` in the
   background and prints the URLs.

If any registration step fails the installer prints the exact command and its output, then undoes what
the run had done (an update goes back to the previous release). The system is left as it was.

## Where things live

| What | Path |
|------|------|
| Code (symlink) | `/usr/local/webpulse` -> `/usr/local/webpulse-releases/<version>` |
| Config | `/etc/webpulse/config.json` (0600) |
| Statistics | `/var/webpulse/` (`webpulse.db`, `data/<user>/<domain>.db`, `geoip/`, `log/`, `run/`) |
| cPanel plugin | `/usr/local/cpanel/base/frontend/jupiter/webpulse/` |
| WHM plugin | `/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/`, `.../addon_plugins/webpulse.png`, `/var/cpanel/apps/webpulse.conf` |
| Cron | `/etc/cron.d/webpulse` |

## Manual installation

Equivalent to the installer, for locked-down servers. Run as root.

```bash
# 1. code
VER=0.1.0
mkdir -p /usr/local/webpulse-releases
curl -fsSL https://codeload.github.com/inalto/webpulse/tar.gz/v$VER | tar -xz -C /usr/local/webpulse-releases
mv /usr/local/webpulse-releases/webpulse-$VER /usr/local/webpulse-releases/$VER   # directory name inside the tarball
chown -R root:root /usr/local/webpulse-releases/$VER
sed -i '1s|^#!.*|#!/usr/local/cpanel/3rdparty/bin/php|' /usr/local/webpulse-releases/$VER/bin/webpulse
chmod 755 /usr/local/webpulse-releases/$VER/bin/webpulse /usr/local/webpulse-releases/$VER/hooks/*.sh \
          /usr/local/webpulse-releases/$VER/whm/cgi/*.cgi
/usr/local/webpulse-releases/$VER/bin/webpulse setup
ln -sfn /usr/local/webpulse-releases/$VER /usr/local/webpulse

# 2. cPanel plugin
T=/usr/local/cpanel/base/frontend/jupiter/webpulse
mkdir -p $T/lib
cp -a /usr/local/webpulse/cpanel/webpulse/. $T/
cp -a /usr/local/webpulse/lib/. $T/lib/
cp -a /usr/local/webpulse/ui $T/ui
cp /usr/local/webpulse/VERSION $T/VERSION
/usr/local/cpanel/scripts/install_plugin /usr/local/webpulse/cpanel/plugin --theme jupiter

# 3. WHM plugin
mkdir -p /usr/local/cpanel/whostmgr/docroot/cgi/webpulse
install -m 755 /usr/local/webpulse/whm/cgi/*.cgi /usr/local/cpanel/whostmgr/docroot/cgi/webpulse/
install -m 644 /usr/local/webpulse/whm/addon_plugins/webpulse.png /usr/local/cpanel/whostmgr/docroot/addon_plugins/
/usr/local/cpanel/bin/register_appconfig /usr/local/webpulse/whm/webpulse.conf

# 4. hooks and cron
/usr/local/cpanel/bin/manage_hooks add script /usr/local/webpulse/hooks/stats_runall_pre.sh  --manual --category Stats --event RunAll  --stage pre
/usr/local/cpanel/bin/manage_hooks add script /usr/local/webpulse/hooks/stats_runuser_pre.sh --manual --category Stats --event RunUser --stage pre
mkdir -p /var/webpulse/run/hook && chmod 1777 /var/webpulse/run/hook && chmod 711 /var/webpulse/run
install -m 644 /usr/local/webpulse/etc/cron.d/webpulse /etc/cron.d/webpulse

# 5. first run
/usr/local/webpulse/bin/webpulse collect
```

## Update

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --update
```

A new release directory is staged, `webpulse setup` migrates the schema, the symlink is swapped and the
plugin files are refreshed. Configuration and statistics are never modified, the previous release stays on
disk (last three are kept) and is restored automatically if the update fails half-way. Running `--update`
twice is harmless.

Roll back by hand: `ln -sfn /usr/local/webpulse-releases/<old> /usr/local/webpulse` (and rerun the install
of that release to refresh the plugin copies).

## Uninstall

```bash
/usr/local/webpulse/uninstall.sh            # remove plugins, hooks, cron, code; keep the statistics
/usr/local/webpulse/uninstall.sh --purge    # ... and delete /var/webpulse and /etc/webpulse
# or, without a local copy:
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --uninstall
```

This runs `manage_hooks delete` for both hooks, `unregister_appconfig webpulse`,
`uninstall_plugin /usr/local/webpulse/cpanel/plugin --theme <theme>` for every theme that has the plugin,
removes the CGI directory, icon, cron file and the code, and restarts cpsrvd if the unregistration
asks for it. Re-installing later picks up the kept data.

## Troubleshooting

**The cPanel icon does not appear**
* Re-run the registration and read its output:
  `/usr/local/cpanel/scripts/install_plugin /usr/local/webpulse/cpanel/plugin --theme jupiter`
* The plugin only shows for accounts whose feature list includes `webpulse`
  (WHM » Feature Manager » WebPulse Analytics). The `default` feature list gets it automatically on
  most servers; custom lists need it ticked.
* `install_plugin` registers the icon in cPanel's dynamic UI definitions; look for `webpulse` in
  `/usr/local/cpanel/base/frontend/jupiter/dynamicui/` (location may differ between cPanel versions). If
  it is absent the registration did not run or failed; if present, log out of cPanel and back in so the
  page is rebuilt. Also check that the files exist: `ls /usr/local/cpanel/base/frontend/jupiter/webpulse/`.
* Using another theme? Install with `--theme <name>` or `--all-themes`.

**The WHM entry is missing (WHM » Plugins » WebPulse Analytics)**
* `ls /var/cpanel/apps/webpulse.conf` must exist; if not, run
  `/usr/local/cpanel/bin/register_appconfig /usr/local/webpulse/whm/webpulse.conf` and read the output.
* Restart the WHM service: `/usr/local/cpanel/scripts/restartsrv_cpsrvd`, then reload WHM.
* Direct URL for a quick check: `https://<server>:2087/cgi/webpulse/index.cgi` (WHM redirects to the
  session-token URL). A blank page or error: look at `/usr/local/cpanel/logs/error_log`.
* Resellers only see accounts they own; accounts of other owners return 403/404 on purpose.

**There is no data**
* `webpulse status` shows the collector health, last run, lag and per-domain counters.
* Is cron running? `cat /etc/cron.d/webpulse`, `grep webpulse /var/log/cron`,
  `tail /var/webpulse/log/webpulse.log`.
* Run it by hand and watch: `/usr/local/webpulse/bin/webpulse collect --verbose`.
* Permissions: the collector runs as root and must read the domlogs. Check
  `ls -l /etc/apache2/logs/domlogs/<user>/` (files are `0640 root:<user>`), and that
  `paths.domlogs_dirs` in `/etc/webpulse/config.json` matches your layout (nginx-only servers use
  `/var/log/nginx/domains`).
* The account or domain may be disabled: WHM » Plugins » WebPulse Analytics » Accounts, or
  `webpulse accounts list`.
* Domains only get a database after the first hit is collected; a brand-new domain shows "no data yet".
* History from before the installation: `webpulse backfill --user <u>` imports `/home/<u>/logs/*.gz`.
* cPanel truncates domlogs after its stats run. The hooks cover that; verify with
  `/usr/local/cpanel/bin/manage_hooks list | grep webpulse` (two entries) and that
  `/var/webpulse/run/hook` has mode `1777`.

**Hook registration failed during install**
* The installer printed the failing command; run it yourself to see the full message. A hook that is
  already registered is detected and skipped, so re-running the installer is safe.

**The installer rolled back**
* Read the `Step failed` block above the rollback message. Nothing is left half-installed; fix the
  reason (missing Jupiter theme, no `pdo_sqlite`, unreachable GitHub) and run it again.

**GeoIP is empty**
* `webpulse geoip-update` needs outbound HTTPS to `download.db-ip.com`; `geoip.enabled` must be true
  (`--geoip` sets it). Countries appear for hits collected after the database is in place.

**Moving the data or testing the installer**
* `WEBPULSE_PREFIX`, `WEBPULSE_FAKE_CPANEL=1`, `WEBPULSE_CPANEL_ROOT`, `WEBPULSE_ETC`, `WEBPULSE_VAR`,
  `WEBPULSE_CRON_DIR`, `WEBPULSE_APPS_DIR` redirect every path so the installer can be exercised in a
  temporary directory without privileges (this is how `tests/cases/pkg_installer.php` works).
