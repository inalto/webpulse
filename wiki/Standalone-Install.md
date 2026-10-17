# Standalone Install (without cPanel)

Since 0.6.0 WebPulse runs on any Linux web server with Apache or nginx: AlmaLinux, Rocky Linux,
RHEL, CentOS Stream (EL 8, 9, 10), Debian and Ubuntu. It finds the sites in the web server's
configuration, tails their access logs from cron and shows the dashboard in **Cockpit** (the
recommended way, see [[Cockpit Module]]) or through `webpulse serve` behind a reverse proxy.
Everything else is the same as on cPanel: one static binary, one SQLite file per site, the same
metrics and the same dashboard.

## Requirements

- **Root** access on Linux x86_64 or aarch64, `curl` (or `wget`), `tar`, `sha256sum`, cron
  (`cronie` on EL, `cron` on Debian).
- **Apache** (`httpd` on EL, `apache2` on Debian/Ubuntu) and/or **nginx**, with one access log per site
  (or at least one per server) in a **combined-compatible** format:
  - Apache: `combined`, `combinedio`, `vhost_combined`, or any `LogFormat` that starts with
    `%h %l %u %t "%r" %>s %b` (optionally preceded by `%v` or `%v:%p`, `%a` for `%h` and `%O`/`%B`
    for `%b` are fine) followed by `"%{Referer}i" "%{User-Agent}i"`. The `common` format is read but
    has no referrer and no user agent (no sources, no browsers, every hit looks human).
  - nginx: `combined` (the default) or a `log_format` that starts like it, such as EL's `main`
    (combined plus `"$http_x_forwarded_for"`).
- Optional: **Cockpit** (`dnf install cockpit` / `apt install cockpit`) for the dashboard.

## Install

```bash
# AlmaLinux / Rocky / RHEL
dnf install -y cockpit && systemctl enable --now cockpit.socket     # optional, recommended
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --standalone

# Debian / Ubuntu
apt install -y cockpit                                              # optional, recommended
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --standalone
```

`--standalone` installs:

- the binary in `/usr/local/webpulse/bin/webpulse` (releases in `/usr/local/webpulse-releases/`);
- `/etc/webpulse/config.json` with `"mode": "standalone"`, `/var/webpulse/` (`webpulse setup`);
- `/etc/cron.d/webpulse`: `collect` every 5 minutes, `rollup` at 02:10, `geoip-update` monthly
  (no cPanel hook line);
- the Cockpit module in `/usr/share/cockpit/webpulse/` when Cockpit is installed (`--cockpit` forces it
  and warns when Cockpit is missing);
- with `--serve`: `webpulse-serve.service` (see below).

It ends with the output of `webpulse sites`: the sites it found and those it will not collect, with
the reason. Add `--geoip` for countries (DB-IP database). Run `install.sh --standalone --update` (or
just `--update`: an installation with `"mode": "standalone"` stays standalone) to update.

Import the logs that logrotate already rotated (once, per owner):

```bash
webpulse sites                        # owners are the "owner" column
webpulse backfill --user local        # sites without an owner
webpulse backfill --user alice
```

## How sites are found

`webpulse sites` shows it. In order of precedence (a domain found by an earlier source hides the
same domain in a later one):

1. **`sites` in `/etc/webpulse/config.json`**, always collected:

   ```json
   {
     "mode": "standalone",
     "sites": [
       {
         "domain": "example.com",
         "owner": "alice",
         "aliases": ["www.example.com"],
         "logs": ["/var/log/httpd/example.com-access.log", "/var/log/httpd/example.com-ssl-access.log"],
         "archives": "/var/log/httpd/example.com-*access.log-*"
       }
     ]
   }
   ```

   `owner` is optional (default `local`); a log whose file name contains `ssl` counts as https;
   `archives` is optional (default: the rotated files next to each log, see below).
2. **Apache**: `/etc/httpd/conf/httpd.conf` with `conf.modules.d/*.conf` and `conf.d/*.conf` (EL),
   `/etc/apache2/apache2.conf` with `ports.conf`, `conf-enabled/`, `sites-enabled/` and the variables
   of `envvars` (Debian). Every `<VirtualHost>` is a site: `ServerName` (port and case dropped),
   `ServerAlias` as aliases, `CustomLog`/`TransferLog` as logs (the 80 and 443 vhosts of a name are
   merged; `SSLEngine on` or port 443 marks https). The owner is the `SuexecUserGroup` (or mpm-itk
   `AssignUserID`) user, else the owner of the `DocumentRoot` when it is a regular user (uid ≥ 1000),
   else `local`. Logs declared outside any vhost (EL's `logs/access_log`, Debian's
   `other_vhosts_access.log`) belong to the **default host**, named after the machine's host name;
   a vhost without a `CustomLog` of its own writes there and is listed as "not collected".
3. **nginx**: `/etc/nginx/nginx.conf` with its `include`s. Every `server` is a site (`server_name`:
   the first name is the domain, the others aliases; a catch-all `_` is the default host); its
   `access_log`s, else the `http`-level ones, which belong to the default host. `access_log off`,
   `syslog:` targets and paths with variables (`$host.log`) cannot be collected.

A log file is read for one site only (the first that declares it). Piped logs (`|rotatelogs …`) are
not supported: log to a file and rotate with logrotate. Configuration roots can be moved with
`paths.apache_conf_dirs` and `paths.nginx_conf` (see [[Configuration]]).

Sites are grouped into **accounts** by owner, exactly like cPanel accounts: the statistics of a site
are in `/var/webpulse/data/<owner>/<domain>.db` (mode `0640 root:<owner>`, so the owner, and only
the owner, can read them, which is what the per-user view in Cockpit relies on). Collection can be
switched per account and per site as on cPanel (`webpulse accounts`, `webpulse domains`).

## Log rotation

logrotate renames the live log (`access_log` → `access_log-20261017` with `dateext`, or
`access.log.1`) and the server reopens a new file. When the collector sees that the log's inode
changed, it first reads the rest of the previous file (found next to it by inode under the rotated
name, uncompressed), then the new one from the start: the lines written between the last run and the
rotation are not lost. Keep `delaycompress` (the default on EL and Debian) so the previous file stays
uncompressed until the next rotation.

`webpulse backfill` imports the rotated files of each log: `<log>-*` and `<log>.N`, plain or `.gz`
(or the `archives` pattern of a configured site). An archive is never imported twice, even when
logrotate renames it (`.1` → `.2.gz`), and lines that live collection already counted are skipped.

## The dashboard without Cockpit: `webpulse serve`

`install.sh --standalone --serve` installs and starts `webpulse-serve.service`:

```ini
[Service]
ExecStart=/usr/local/webpulse/bin/webpulse serve --listen 127.0.0.1:8099 --token-file /etc/webpulse/serve-token --allow-root
```

It serves the administrator dashboard on `http://127.0.0.1:8099/` only. Every request needs the token
of `/etc/webpulse/serve-token` (created with mode 0600): open `http://127.0.0.1:8099/?token=<token>`
once, the page then keeps it in a cookie. `--allow-root` is needed because only root can read every
site's statistics; use `--user <owner>` (and run it as that user) for one owner's sites. Options:

| Option | Meaning |
|--------|---------|
| `--listen ADDR:PORT` | Address and port (default `127.0.0.1:8099`; `--port P` means `127.0.0.1:P`). |
| `--token-file PATH` | Persistent token, created when missing; refused when readable by others. |
| `--allow-host HOST` | Also accept this `Host` header (the public name of a reverse proxy). |
| `--admin`, `--user U` | Principal: the administrator (default) or one owner's sites. |

`serve` speaks plain HTTP. To reach it from elsewhere, put a TLS reverse proxy in front, at the root
of a dedicated host name, or use an SSH tunnel (`ssh -L 8099:127.0.0.1:8099 server`).

nginx:

```nginx
server {
    listen 443 ssl;
    server_name stats.example.com;
    ssl_certificate     /etc/letsencrypt/live/stats.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/stats.example.com/privkey.pem;
    location / {
        proxy_pass http://127.0.0.1:8099;     # Host becomes 127.0.0.1:8099, which serve accepts
        proxy_set_header X-Forwarded-Proto https;
    }
}
```

Apache (`mod_proxy`, `mod_proxy_http`):

```apache
<VirtualHost *:443>
    ServerName stats.example.com
    SSLEngine on
    SSLCertificateFile /etc/letsencrypt/live/stats.example.com/fullchain.pem
    SSLCertificateKeyFile /etc/letsencrypt/live/stats.example.com/privkey.pem
    ProxyPass        / http://127.0.0.1:8099/
    ProxyPassReverse / http://127.0.0.1:8099/
</VirtualHost>
```

With `ProxyPreserveHost On` (or `proxy_set_header Host $host`) add `--allow-host stats.example.com` to
the service (`systemctl edit webpulse-serve`). The token is the only authentication: add HTTP basic
authentication or an IP allow list in the proxy if the host name is public. Change the token by
deleting the file and restarting the service.

## Uninstall

`install.sh --uninstall` (or `/usr/local/webpulse/uninstall.sh`) removes the binary, the cron file, the
Cockpit module and the service; `--purge` also deletes `/var/webpulse` and `/etc/webpulse`.

## See also

- [[Cockpit Module]]
- [[Configuration]]
- [[Command Line]]
- [[Troubleshooting and FAQ]]
