# Standalone servers and the Cockpit module — plan

Goal: run WebPulse on a plain AlmaLinux/Rocky/RHEL (and Debian/Ubuntu) web
server without cPanel, and offer the same dashboard as a **Cockpit** module,
keeping the same basics: one static binary, one SQLite file per site, the
same metrics, the same UI, the same `webpulse cgi` entry point.

Decision: **same repository.** The engine, reports, UI and installer are
shared; standalone mode is a second *discovery provider* plus packaging, and
the Cockpit module is our existing UI with a 150-line transport adapter that
calls the existing `webpulse cgi` through Cockpit's process bridge. A separate
repo would only duplicate the release pipeline.

## What already works without cPanel

* The binary itself: collect, backfill, rollup, reports, `serve` (dev server),
  `cgi`. Nothing in the engine needs cPanel except *discovery* (which domains
  exist, who owns them, where their logs are) and the hooks.
* `install.sh --standalone` installs the binary and the CLI only.

## Phase A — standalone collection (target 0.6.0)

### Mode and discovery provider
* New config key `mode`: `auto` (default: cPanel when `/usr/local/cpanel/cpanel`
  exists, else standalone), `cpanel`, `standalone`.
* `engine::discovery` gets a `Provider` trait with two implementations:
  `CpanelProvider` (today's code, unchanged) and `StandaloneProvider`.
* `StandaloneProvider` builds the `(account, domain, logs[], archives glob)`
  list from, in order of precedence:
  1. **Explicit sites** in `config.json`:
     ```json
     "sites": [
       { "domain": "example.com", "owner": "alice",
         "logs": ["/var/log/httpd/example.com-access.log", "/var/log/httpd/example.com-ssl-access.log"],
         "archives": "/var/log/httpd/example.com-access.log-*" }
     ]
     ```
  2. **Apache** auto-detection: `/etc/httpd/conf/httpd.conf`, `/etc/httpd/conf.d/*.conf`
     (EL) and `/etc/apache2/apache2.conf`, `/etc/apache2/sites-enabled/*`
     (Debian): `<VirtualHost>` blocks → `ServerName` (+ `ServerAlias` as
     aliases, same site), `CustomLog <path> <format>` where the format is
     `combined`/`combinedio`/`vhost_combined` or a `LogFormat` nickname that
     expands to a combined-compatible string; `SuexecUserGroup` or the
     `DocumentRoot` owner gives the `owner`. The default host's log
     (`/var/log/httpd/access_log`) is attributed to the machine hostname.
  3. **Nginx** auto-detection: `/etc/nginx/nginx.conf` with `include`s →
     `server { server_name …; access_log <path> [format]; }`; formats
     `combined` (default) or `main` (EL's default: combined plus a trailing
     `"$http_x_forwarded_for"`, which the parser already tolerates).
  * Accounts: the `owner` (a Linux user) groups sites, exactly as cPanel
    accounts do today; sites without an owner go to the `local` account.
    Databases keep the `/var/webpulse/data/<owner>/<domain>.db` layout and the
    `0640 root:<owner>` mode, so a Linux user can read their own sites (the
    group named after the user exists on EL/Debian by default).
  * `webpulse sites [--json]` prints what discovery found (source: config,
    apache, nginx; log files; parse-format check on the first lines) so an
    admin can see why a site is missing before writing `sites` by hand.
  * The registry keeps `accounts`/`domains` exactly as today (kind = `site`).

### Log rotation without cpanellogd
* logrotate renames `access_log` → `access_log-YYYYMMDD` (dateext) or `.1`,
  compresses later (`delaycompress`). Today an inode change restarts at 0 of
  the new file and loses the lines appended between the last run and the
  rotation. New: when the inode changed, `LogSource` first looks for the
  previous inode among the directory's files (`access_log-*`, `access_log.1`),
  finishes reading it from the stored offset, then switches. Zero loss with
  the 5-minute cron.
* Backfill: archives are the rotated files (`<log>-*`, `<log>.N`, `.gz`),
  from the `archives` glob when given, else derived from the live log path.
  The same "never twice" bookkeeping applies.
* Permissions: standalone logs are root-owned; the collector runs as root
  from cron, as today. No hooks are registered.

### Viewing the dashboard without cPanel
* **Cockpit module** (Phase B) is the recommended UI.
* `webpulse serve --listen 127.0.0.1:8099` gets a production-ish mode: a
  persistent token in `/etc/webpulse/serve-token` (0600), `--listen` on any
  address (TLS is left to a reverse proxy, documented for nginx/Apache),
  optional systemd unit `webpulse-serve.service` installed by
  `install.sh --standalone --serve`. Principals: `--admin` (default) or
  `--user <linux user>`.
* The PDF report, CSV export and all tabs work unchanged.

### Installer
* `install.sh --standalone` becomes a full install: binary, config with
  `mode: standalone`, `webpulse setup`, cron (collect every 5 min, rollup,
  geoip-update, no hooks), first `webpulse sites` output in the summary, and
  `--cockpit` when `/usr/share/cockpit` exists (Phase B). Works on EL8/9/10
  and Debian/Ubuntu (paths detected by the provider, not by the installer).
* Later: RPM (`webpulse.spec`, EL9/10) and `.deb` built in the release
  workflow, so `dnf install` works and updates come from the package manager
  instead of the one-click upgrade (which stays for tarball installs).

## Phase B — Cockpit module (target 0.7.0)

* Files in `cockpit/` installed to `/usr/share/cockpit/webpulse/`:
  * `manifest.json`: `{"version": "<ver>", "requires": {"cockpit": "266"},
    "menu": {"index": {"label": "Web analytics", "order": 65}}}` — a left-menu
    entry in Cockpit.
  * `index.html`: generated at install time by `webpulse page --cockpit`
    (the same `Page::render` output as the WHM page, plus `../base1/cockpit.js`
    and the Cockpit adapter; `window.WEBPULSE.transport = "cockpit"`).
  * `cockpit.js` adapter (in `ui/`, embedded like the other assets): the SPA's
    `api()` helper, instead of `fetch`, runs
    `cockpit.spawn([bin, "cgi", "--cockpit", "--api"], {superuser: "try",
    environ: ["REQUEST_METHOD=…", "QUERY_STRING=…", "CONTENT_LENGTH=…",
    "HTTP_X_WEBPULSE_REQUEST=1"], err: "message"}).input(body)` and parses
    the CGI response (`Status:` line, headers, body). Cockpit handles login,
    the "Administrative access" toggle and the transport; no network listener,
    no token.
  * `webpulse cgi --cockpit`: the principal comes from the **real effective
    uid** of the spawned process (Cockpit runs it as the logged-in user, or as
    root when administrative access is on): uid 0 → admin; another user →
    `user:<name>` seeing the sites whose `owner` is that user (DB files are
    group-readable by them). `REMOTE_USER` is ignored in this mode.
  * Report/PDF inside Cockpit: open the report in the same frame
    (`#report…`) and call `window.print()` from the frame, which prints the
    module's document only; "Back to dashboard" returns.
  * Dark mode follows Cockpit's `pf-v5-theme-dark` class on the shell when
    available, else the OS preference.
* Installer: `install.sh --cockpit` (implied by `--standalone` when Cockpit is
  installed) copies the files, generates `index.html`, and `--update`
  regenerates it; uninstall removes the directory. Cockpit picks modules up
  without a restart.
* Also usable on cPanel servers that run Cockpit (same binary, `--cockpit`
  mode maps cPanel accounts to Linux users one-to-one).

## Phase C — packages and polish
* RPM and DEB packaging in the release workflow; `dnf`/`apt` repositories
  are out of scope for now.
* Debian/Ubuntu Apache log formats (`vhost_combined` with `%v:%p`) are
  handled by the existing optional-vhost prefix in the parser; verify on a
  real box.
* Per-user dashboards on standalone servers through Cockpit only (no PAM
  login of our own).

## Non-goals
* No JavaScript tracker, no own web server with TLS, no own user database:
  authentication is always delegated (cPanel, WHM, Cockpit, or an SSH tunnel
  to `serve`).
