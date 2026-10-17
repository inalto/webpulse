# Updating and Uninstalling

How to move to a newer version, go back to an older one, and remove WebPulse. Everything is done by
the same `install.sh` that installed it.

## Updating

**One click from WHM (0.4.0 and later).** The server looks for a new release every night. When one is
out, the bell in the WebPulse header in WHM shows a dot: open it (or the **Updates** card of the server
view) and choose **Upgrade now**. This runs the same `install.sh --update` described below, for the
release shown, in the background; the output is kept in `/var/webpulse/log/upgrade.log`. See
[[WHM Administration]] for the details and for turning the nightly check off.

**From the shell.** Run as root:

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --update
```

`--update` and a plain run do the same thing when WebPulse is already installed: the installer
detects `/usr/local/webpulse/bin/webpulse` and updates in place. If WebPulse is **not** installed,
`--update` prints a warning and performs a fresh installation.

### What an update does

1. Downloads the requested release (default: the latest; `webpulse-<version>-<target>.tar.gz` for your CPU),
   **verifies its sha256 checksum** and stages it as `/usr/local/webpulse-releases/<version>/`. If a directory with that version already exists and is
   identical, it is reused; if it differs, the new one gets a timestamp suffix.
2. Runs `webpulse setup`, which creates what is missing and migrates the database schema.
3. Swaps the `/usr/local/webpulse` symlink to the new release.
4. Refreshes the cPanel shim files in the theme directories, the WHM CGIs, page template and icon, and
   re-runs the registrations. A registration that is already in place is reported as such.
5. Rewrites `/etc/cron.d/webpulse` only if its content changed.
6. Keeps the three newest releases and removes older ones. The release that is active, and the one
   you came from, are never removed.
7. Starts a collection in the background.

Your configuration (`/etc/webpulse/config.json`) and statistics (`/var/webpulse/`) are not modified.
Running `--update` twice is harmless.

If the update fails half-way, the installer rolls back: the symlink returns to the previous release and
the plugin files are restored from it.

### Updating from 0.1.0 (PHP) to 0.2.0 (Rust)

The same command. The installer replaces the PHP launcher `bin/webpulse` by the binary at the same path,
removes the PHP library copies from the cPanel plugin directory and keeps `/etc/webpulse` and
`/var/webpulse`: the 0.2.0 binary opens 0.1.0 databases as they are, and 0.1.0 can open the databases
again if you roll back. `php_bin` in `config.json` is no longer used.

### Choosing a version

```bash
... | bash -s -- --update --ref v0.2.0      # a release tag
... | bash -s -- --update --build           # compile the source with cargo instead of downloading a binary
... | bash -s -- --update --binary /root/webpulse   # a binary you provide
... | bash -s -- --update --repo yourname/webpulse   # a fork's releases
```

A branch name (`--ref some-branch`) has no prebuilt binary: it is built from source and needs `cargo`.

### Themes other than Jupiter

An update refreshes the `jupiter` theme and the themes named on the command line. If you installed
the plugin in other themes, repeat `--theme <name>` or `--all-themes` when you update.

### Updating from the installed copy

`bash /usr/local/webpulse/install.sh --update` does not reinstall the code it sits in. It detects that
it is running from an installed release and downloads a fresh copy instead.

### Check the result

```bash
/usr/local/webpulse/bin/webpulse --version
/usr/local/webpulse/bin/webpulse status
ls -l /usr/local/webpulse
```

## Rolling back by hand

Old releases stay in `/usr/local/webpulse-releases/` (the last three). To go back, point the symlink at
the old release and refresh the plugin copies from it:

```bash
ls /usr/local/webpulse-releases/
ln -sfn /usr/local/webpulse-releases/<old-version> /usr/local/webpulse
```

Then repeat steps 3 and 4 of [[Manual Installation]] (copy the cPanel shim files and the WHM CGI
shims). The hooks and the cron file refer to `/usr/local/webpulse`, so they keep working. Running
`install.sh` from inside `/usr/local/webpulse-releases/` does not reinstall that release: it downloads
a fresh copy. The alternative is `install.sh --update --ref <tag>` with the tag of the old version.

Be careful when going back across a schema change: an older release may refuse a database written by a
newer one.

## Uninstalling

```bash
/usr/local/webpulse/uninstall.sh              # keep the statistics and the configuration
/usr/local/webpulse/uninstall.sh --purge      # also delete /var/webpulse and /etc/webpulse
```

Without a local copy:

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --uninstall
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --uninstall --purge
```

`uninstall.sh` is a thin wrapper for `install.sh --uninstall`.

### What is removed

| Step | Command or action |
|------|-------------------|
| Hooks | `manage_hooks delete script ... --category Stats --event RunAll` and `RunUser`, stage `pre` |
| WHM plugin | `unregister_appconfig webpulse`, then the CGI directory and the icon are deleted; `cpsrvd` is restarted if an unregistration was done |
| cPanel plugin | `uninstall_plugin /usr/local/webpulse/cpanel/plugin --theme <theme>` for every theme that contains a `webpulse` directory, then that directory is deleted |
| Cron | `/etc/cron.d/webpulse` |
| Code | `/usr/local/webpulse` and `/usr/local/webpulse-releases` |

### What is kept

Without `--purge`, `/var/webpulse` (all statistics) and `/etc/webpulse` (configuration) stay. Only the
hook flag directory `/var/webpulse/run/hook` is removed. Installing again later picks the data up.

With `--purge` both directories are deleted. From a terminal the installer first asks you to type
`delete`; with `-y`, or when stdin is not a terminal, it does not ask.

### Uninstalling by hand

If the installer is not available:

```bash
/usr/local/cpanel/bin/manage_hooks delete script /usr/local/webpulse/hooks/stats_runall_pre.sh  --manual --category Stats --event RunAll  --stage pre
/usr/local/cpanel/bin/manage_hooks delete script /usr/local/webpulse/hooks/stats_runuser_pre.sh --manual --category Stats --event RunUser --stage pre
/usr/local/cpanel/bin/unregister_appconfig webpulse
/usr/local/cpanel/scripts/uninstall_plugin /usr/local/webpulse/cpanel/plugin --theme jupiter
rm -rf /usr/local/cpanel/base/frontend/jupiter/webpulse \
       /usr/local/cpanel/whostmgr/docroot/cgi/webpulse \
       /usr/local/cpanel/whostmgr/docroot/addon_plugins/webpulse.png
rm -f /etc/cron.d/webpulse
/usr/local/cpanel/scripts/restartsrv_cpsrvd
rm -rf /usr/local/webpulse /usr/local/webpulse-releases
# only if you want to lose the statistics:
# rm -rf /var/webpulse /etc/webpulse
```

Do not delete `/usr/local/webpulse` before the `uninstall_plugin` line has run: it reads `cpanel/plugin` from there.

## Removing the data of one account

You do not have to uninstall to delete the statistics of a single account or domain:

```bash
/usr/local/webpulse/bin/webpulse purge --user <user> --yes
/usr/local/webpulse/bin/webpulse purge --user <user> --domain example.com --yes
```

See [[Privacy and GDPR]].

## See also

- [[Installation]]
- [[Manual Installation]]
- [[Command Line]]
- [[Troubleshooting and FAQ]]
