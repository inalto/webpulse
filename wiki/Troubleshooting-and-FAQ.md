# Troubleshooting and FAQ

Start with these three commands. They answer most questions:

```bash
/usr/local/webpulse/bin/webpulse status
tail -n 50 /var/webpulse/log/webpulse.log
/usr/local/webpulse/bin/webpulse collect --verbose
```

> WebPulse was developed without a real cPanel server. Several items below say "location may differ
> between cPanel versions". If you find a behaviour that differs, please open an issue.

## The plugin is not in Meridian's side menu

On cPanel 138 the Meridian sidebar is a fixed list of seven built-in hubs (Dashboard, Websites, Email, Files, Databases, Security, Performance); no plugin can add an entry there, and the classic Metrics icons are not shown either. WebPulse 0.2.3 and later register a Meridian *hub* instead, which gives two ways in: press **Ctrl+K** (or click Search) and type "WebPulse", or open `…/frontend/meridian/index.html#/webpulse`. The dashboard then renders inside the Meridian shell. The hub is re-applied after every cPanel update; if the search entry is missing, run the installer again with `--update`, check that `grep -c '"slug": "webpulse"' /usr/local/cpanel/base/frontend/meridian/_assets/_registry.js` prints 1 and that the account has the `webpulse` feature, then reload the page. Should a future Meridian open its sidebar to registered hubs, WebPulse will appear there automatically.

## Reading `webpulse status`

`status` prints one line per topic:

| Line | What to look for |
|------|------------------|
| `config` | The file in use. `[missing]` or `[unreadable]` means defaults are used. |
| `data_dir`, `registry_db`, `log_dir`, `run_dir` | `[missing]` means `webpulse setup` has not run or a path is wrong. |
| `registry` | Must be `ok`. Otherwise the command exits with 1. |
| `accounts`, `domains` | How many exist, how many are enabled, how many have data and the total size. |
| `last collect`, `newest line` | When a collection last ran and the age of the newest log line (the lag). A lag of a few minutes is normal. |
| `cron` | `installed` or `NOT installed`, and the number of pending hook flags. |
| `geoip` | `enabled` or `disabled`, and the database date or `no database`. |
| `last jobs` | The latest runs with status and line counts. |

`webpulse status --json` gives the same data for scripts.

## The log file

`/var/webpulse/log/webpulse.log` (root only, mode 0600) has one line per event:

```
2026-10-16T10:05:02+02:00 [info] 4821 message {"optional":"json context"}
```

Fields: ISO time with offset, level (`debug`, `info`, `warn`, `error`), process id, message. The file
rotates at 5 MiB and five files are kept. Useful searches:

```bash
grep -E '\[(warn|error)\]' /var/webpulse/log/webpulse.log | tail -n 30
grep example.com /var/webpulse/log/webpulse.log | tail
```

`--verbose` on any command also prints the log to the terminal.

## There is no data

1. **Wait.** A domain shows *Collecting…* until its first lines are processed. The next collection runs
   within five minutes.
2. **Is cron running?** `cat /etc/cron.d/webpulse` and `grep webpulse /var/log/cron`. `status` shows
   `NOT installed` if the file is missing (did you install with `--no-cron`?).
3. **Run it by hand:** `webpulse collect --verbose`, and read the output and the log.
4. **Is the account or domain switched off?** `webpulse accounts list` and
   `webpulse domains list <user>`, or WHM » WebPulse Analytics » Server » Accounts.
5. **Can the collector find the logs?** The collector runs as root and looks in the directories of
   `paths.domlogs_dirs`. Check `ls -l /etc/apache2/logs/domlogs/<user>/`. Files named `<domain>` and
   `<domain>-ssl_log` are expected. On an ea-nginx-only server they are in `/var/log/nginx/domains/`.
   If your layout is different, add the directory to `paths.domlogs_dirs` in `/etc/webpulse/config.json`
   ([[Configuration]]).
6. **Is the log format *combined*?** Lines that cannot be parsed are counted and skipped. If every line
   is skipped, the format is different.
7. **Are the hits excluded?** Check the excluded IPs and paths, server-wide and per domain. Traffic from
   bots, and `HEAD` requests, does not create visits.
8. **History from before the installation.** Import archived logs with
   `webpulse backfill --user <user>`.

## The cPanel icon is missing

- Re-run the registration and read its output:
  `/usr/local/cpanel/scripts/install_plugin /usr/local/webpulse/cpanel/plugin --theme jupiter`.
- The icon shows only for accounts whose **feature list** includes `webpulse`
  (WHM » Feature Manager » "WebPulse Analytics"). Custom feature lists need it ticked.
- Look for `webpulse` in `/usr/local/cpanel/base/frontend/jupiter/dynamicui/` (the location may differ
  between cPanel versions). If absent, the registration did not run or failed. If present, log out of
  cPanel and in again.
- Check that the files exist: `ls /usr/local/cpanel/base/frontend/jupiter/webpulse/`.
- Another theme? Install with `--theme <name>` or `--all-themes` ([[Installation]]).

## The WHM entry is missing

- `/var/cpanel/apps/webpulse.conf` must exist. If not, run
  `/usr/local/cpanel/bin/register_appconfig /usr/local/webpulse/whm/webpulse.conf` and read the output.
- Restart the WHM service: `/usr/local/cpanel/scripts/restartsrv_cpsrvd`, then reload WHM.
- Try the direct URL `https://<server>:2087/cgi/webpulse/index.cgi`. WHM redirects to the session URL.
  A blank page or an error is explained in `/usr/local/cpanel/logs/error_log`.
- A reseller sees only the accounts it owns. Someone who is neither root nor a reseller with accounts is
  refused.

## Permission errors

- The collector must run as **root**. A cron entry or a manual run as another user cannot read the
  domlogs (`0640 root:<user>`).
- Domain databases are `root:<user>` mode 0640 in a directory `0750`. If an account sees "not found" for
  its own domain, check `ls -ld /var/webpulse /var/webpulse/data /var/webpulse/data/<user>`: the first
  two must be traversable by others (`0711`), the third `0750 root:<user>`. `webpulse setup` repairs them.
- The hook flag directory must be `1777`: `ls -ld /var/webpulse/run/hook`.
- Settings cannot be saved in cPanel: the `user/` directory under the account's data directory must be
  owned by the account. `webpulse setup` creates it.
- The config file is root-only (0600). The cPanel plugin cannot read it and uses defaults for display.

## Hooks, and logs truncated by cpanellogd

By default cPanel empties each domain log after its statistics run. WebPulse's hooks read the log first.
Check them:

```bash
/usr/local/cpanel/bin/manage_hooks list | grep webpulse     # two entries
ls -ld /var/webpulse/run/hook                                # drwxrwxrwt
```

If one is missing, register it again (commands in [[Manual Installation]], step 5). If you see gaps in the
data around the daily statistics time, check that both hooks are listed and that the cron job
`collect --hooked` exists. The *Integration* tile in the WHM server view shows **Stats hook**.

If you turn off "Delete each domain's access logs after statistics are gathered" in WHM » Tweak
Settings » Stats and Logs, the logs are no longer truncated and the hooks matter less; WebPulse handles
truncation and replacement either way.

## nginx

With ea-nginx in standalone mode, logs are in `/var/log/nginx/domains/`, which is in the default
`paths.domlogs_dirs`. The log format must still be *combined*. If you use a custom format, WebPulse
cannot parse it.

## "Unknown" countries

Countries need GeoIP: `webpulse config set geoip.enabled true`, then `webpulse geoip-update`
([[Installation]]). Only hits collected **after** the database is in place get a country; earlier data
stays unknown. `geoip-update` needs outbound HTTPS to `download.db-ip.com`. `status` shows the database
date. Cities need a separate city database (`geoip.city_db`).

## I changed the timezone

Days and hours use the timezone at the time of collection. Earlier days are **not** recalculated, and the
dashboard shows a notice with the date of the change. This is expected.

## The installer rolled back

Read the `Step failed` block above the rollback message. It shows the command, the exit status and
the output. Typical causes: no Jupiter theme, a checksum mismatch, GitHub not reachable, a hook
registration refused by cPanel. Fix the cause and run the installer again. Hooks that are already
registered are detected and skipped.

## FAQ

**Does it slow down my customers' sites?** No. It reads log files after the requests are served.

**How much disk space does it use?** `status` and the *Data size* tile show it. Aggregates are small;
the visits log is limited by `raw_retention_days`.

**Can I keep the data when I uninstall?** Yes. Without `--purge`, `/var/webpulse` stays.

**Can I run it without cPanel?** Yes, since 0.6.0: `install.sh --standalone` on any Apache or nginx
server, with the dashboard in Cockpit ([[Standalone Install]], [[Cockpit Module]]). `webpulse sites`
explains which sites it found and why one is not collected.

**Numbers differ from another tool?** See [[Metrics and Definitions]].

**Where do I report a bug?** <https://github.com/inalto/webpulse/issues>

## See also

- [[Installation]]
- [[Command Line]]
- [[How It Works]]
- [Technical troubleshooting notes](https://github.com/inalto/webpulse/blob/main/docs/INSTALL.md)
