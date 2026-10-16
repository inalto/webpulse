# cPanel & WHM integration

Verified against the cPanel developer documentation (api.docs.cpanel.net,
docs.cpanel.net) on 2026-10-16. Where the docs were silent, the behaviour is
marked *assumed* and the installer verifies it at run time and prints a clear
error instead of guessing.

## Access logs

* Location: `/etc/apache2/logs/domlogs/<user>/<domain>` and
  `<domain>-ssl_log` (cPanel 11.60+ with EA4; `/usr/local/apache/domlogs` is a
  symlink on older systems; `/var/log/apache2/domlogs` is another symlink).
  With ea-nginx in standalone mode logs are in `/var/log/nginx/domains/`.
  Discovery searches all configured `domlogs_dirs`.
* Format: Apache `combined`
  (`%h %l %u %t "%r" %>s %b "%{Referer}i" "%{User-Agent}i"`), local time with
  numeric offset. `%b` is `-` for zero bytes.
* Permissions: files are `0640 root:<user>` ("Apache log file chmod value",
  default 0640). The collector runs as root.
* Lifecycle (WHM » Tweak Settings » Stats and Logs):
  * "Delete each domain's access logs after statistics are gathered" (default
    **On**): `cpanellogd` truncates the domlog after each stats run (every 24 h
    by default, "Stats cycle"). That is why the collector runs every 5 minutes
    *and* hooks `Stats::RunUser` (pre) to drain the file right before the run.
  * "Archive logs in the user's home directory at the end of each stats run"
    (default On): compressed copies land in `/home/<user>/logs/` as
    `<domain>-<Mon>-<YYYY>.gz` and `<domain>-ssl_log-<Mon>-<YYYY>.gz`
    (*assumed naming; the backfill command accepts any `*.gz` in that directory
    whose name starts with a known domain and never imports the same file twice*).
  * "Remove the previous month's archived logs" (default On).
* Rotation: cPanel does **not** use logrotate for domlogs; truncation is the
  only event. The checkpoint logic (inode + offset + size + first-line hash)
  handles truncation, replacement and manual `> file`.

## cPanel plugin (account dashboard)

Files are copied by our installer (the `install_plugin` script "does not
extract the plugin from the archive"; it only reads `install.json`):

```
/usr/local/cpanel/base/frontend/jupiter/webpulse/
    index.live.php      dashboard page: thin shim (LiveAPI PHP)
    api.live.php        JSON API: thin shim (LiveAPI PHP)
    lib/LiveApiUser.php resolves the account name from the cPanel session
    lib/CgiBridge.php   runs `webpulse cgi` and relays status/headers/body
    webpulse.svg        48×48 icon (also referenced by install.json)
    VERSION             copied by the installer (not symlinks, for robustness;
                        `install.sh --update` re-copies them)
```
Since 0.2.0 there is no PHP library any more: all logic is in the binary
`/usr/local/webpulse/bin/webpulse`, and the two `*.live.php` files only exist
because cPanel requires LiveAPI pages to be PHP (or Perl) to use the `CPANEL`
class (which is also how the account is identified).

`install.json` (in `/usr/local/webpulse/cpanel/plugin/`, together with the icon):
```json
[{"id":"webpulse","name":"WebPulse Analytics","description":"Visitor statistics generated from your access logs",
  "uri":"webpulse/index.live.php","icon":"webpulse.svg","group_id":"metrics","order":1,
  "type":"link","featuremanager":1,"feature":"webpulse"}]
```
Register / unregister:
```
/usr/local/cpanel/scripts/install_plugin   /usr/local/webpulse/cpanel/plugin --theme jupiter
/usr/local/cpanel/scripts/uninstall_plugin /usr/local/webpulse/cpanel/plugin --theme jupiter
```
`featuremanager: 1` adds a "WebPulse Analytics" feature to WHM » Feature
Manager, so hosts can hide the icon per package. Collection is governed
separately by the WHM plugin switches.

The shims (`cpanel/webpulse/*.live.php`):
```php
require_once '/usr/local/cpanel/php/cpanel.php';
$cpanel = new CPANEL();                                  // connects to cpsrvd; one per request
$user   = LiveApiUser::resolve($cpanel);                 // uapi Variables::get_user_information, REMOTE_USER, euid
// api.live.php:
CgiBridge::run(['cgi', '--principal', "user:$user", '--api'], $user, true);
// index.live.php (between header() and footer(); the binary's headers are not applied):
print $cpanel->header('WebPulse Analytics');
CgiBridge::run(['cgi', '--principal', "user:$user", '--page', '--embedded', '--api-url', 'api.live.php'], $user, false);
print $cpanel->footer();
$cpanel->end();
```
`CgiBridge::run()` starts the binary with `proc_open` (argument list, no shell) and:
* an environment that contains only `REQUEST_METHOD`, `QUERY_STRING`, `CONTENT_TYPE`,
  `CONTENT_LENGTH`, `HTTP_X_WEBPULSE_REQUEST`, `REMOTE_ADDR`, `REMOTE_USER` (the resolved account),
  `PATH`, and `WEBPULSE_CONFIG` when it is set;
* the request body (read from `php://input`, at most 2 MiB) piped to the binary's stdin;
* the CGI response parsed from its stdout: the `Status:` line becomes `http_response_code()`, each
  well-formed `Name: value` header a `header()` call, the rest is the body, streamed as it arrives;
* stderr discarded. On any failure (binary missing, cannot start, no response) nothing from the
  child is shown: `api.live.php` answers a generic JSON 500 and `index.live.php` a generic alert,
  and the details go to cPanel's error log without paths.

The binary is found at `$WEBPULSE_BIN`, else `/usr/local/webpulse/bin/webpulse`, else
`bin/webpulse` next to the shim (the packaging tests use the first and last).

The LiveAPI PHP runs **as the account user**, so the binary does too, and the OS permissions are the real
boundary: it may only read `/var/webpulse/data/<user>/` (group-readable) and write
`/var/webpulse/data/<user>/user/` (owned by the user). The binary additionally refuses
`--principal user:X` when its effective uid is neither root nor X's. `uapi()` return shapes
differ slightly between versions (array vs object); `LiveApiUser::resolve()`
accepts both and falls back to `getenv('REMOTE_USER')`, then to
`posix_getpwuid(posix_geteuid())['name']`. The shim needs `proc_open` to be allowed in cPanel's PHP
(`disable_functions`) — to be confirmed on a real server.

## WHM plugin (administrator)

```
/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/index.cgi    #!/bin/bash shim
/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/api.cgi      #!/bin/bash shim
/usr/local/cpanel/whostmgr/docroot/addon_plugins/webpulse.png  48×48 transparent PNG
/var/cpanel/apps/webpulse.conf
```
`index.cgi` keeps the WHM header comments right after the shebang:
```bash
#!/bin/bash
#WHMADDON:webpulse:WebPulse Analytics:webpulse.png
#ACLS:all
exec /usr/local/webpulse/bin/webpulse cgi --whm --page --api-url api.cgi
```
`api.cgi` is the same with `exec … cgi --whm --api`. Both locate the binary through `$WEBPULSE_BIN`
(honoured only when the file and its directory belong to the user running the shim, root under
cpsrvd, and are not group/other-writable), `/usr/local/webpulse/bin/webpulse`, then `bin/webpulse`
next to the script, and print a generic
500 (HTML or JSON) when none exists. cpsrvd's CGI environment (`REMOTE_USER`, `REQUEST_METHOD`,
`QUERY_STRING`, `CONTENT_TYPE`, `CONTENT_LENGTH`, `HTTP_X_WEBPULSE_REQUEST`, `REMOTE_ADDR`) and the
request body on stdin are inherited by `exec`.
The AppConfig `acls=all,list-accts` is what actually gates access in modern WHM;
the `#ACLS` comment is the legacy marker. Whether a reseller with only
`list-accts` sees the menu entry must be confirmed on a real server.
AppConfig `webpulse.conf`:
```
name=webpulse
service=whostmgr
url=/cgi/webpulse/index.cgi
url2=/cgi/webpulse/api.cgi
user=root
acls=all,list-accts
displayname=WebPulse Analytics
entryurl=webpulse/index.cgi
icon=webpulse.png
target=_self
searchtext=analytics statistics visitors logs awstats webalizer
```
Register / unregister (cpsrvd picks up the change on restart; the installer
runs `/scripts/restartsrv_cpsrvd` only if registration reports it is needed):
```
/usr/local/cpanel/bin/register_appconfig   /usr/local/webpulse/whm/webpulse.conf
/usr/local/cpanel/bin/unregister_appconfig webpulse
```
The CGI runs as root and `webpulse cgi --whm` refuses to run when its euid is not 0. It prints
`Content-Type: text/html\r\n\r\n` then the SPA; WHM shows it inside its own frame so no WHM
master template is needed. Mutating API actions need `POST` and `X-WebPulse-Request: 1` (the binary
answers 405 otherwise).
Caller identity: `REMOTE_USER`; `root` → admin principal; otherwise
the user is a reseller → `reseller` principal whose visible accounts are those
with `OWNER=<reseller>` in `/var/cpanel/users/<account>` (root is also the
owner of its own accounts). A reseller that owns no account, or any other
REMOTE_USER value, gets 403.

## Standardized hook — drain logs before stats

`Stats::RunUser` (stage `pre`) runs **as the cPanel user** right before
`cpanellogd` processes (and then truncates/archives) that user's domlogs.
Because it runs unprivileged, the hook script cannot write our root-owned
databases; it therefore only *requests* a run: it writes a flag file
`/var/webpulse/run/hook/<user>` (dir is 1777) and the collector's cron, which
runs every minute in "hook mode" (`webpulse collect --hooked`, cheap no-op when
no flags exist), drains those users within ≤ 60 s. `cpanellogd` itself takes
longer than that to process a user's logs, so the window is covered in
practice; additionally `Stats::RunAll` (stage `pre`, runs as **root**) triggers
a full `webpulse collect` synchronously, which drains everything before the
whole stats cycle starts. Both hooks are registered:
```
/usr/local/cpanel/bin/manage_hooks add script /usr/local/webpulse/hooks/stats_runall_pre.sh  --manual --category Stats --event RunAll  --stage pre
/usr/local/cpanel/bin/manage_hooks add script /usr/local/webpulse/hooks/stats_runuser_pre.sh --manual --category Stats --event RunUser --stage pre
/usr/local/cpanel/bin/manage_hooks delete script <path> --manual --category … --event … --stage pre
```
Hook scripts must answer `--describe` with a JSON array describing
themselves (category/event/stage/hook/exectype) and read the JSON payload on
STDIN (`{"context":{…},"data":{"user":"…", …}}`). `stats_runuser_pre.sh`
extracts `data.user` with plain bash (no PHP, no binary, no `jq`: it runs as the
account), validates `^[a-z0-9_][a-z0-9_-]{0,31}$`, and touches the flag.
`stats_runall_pre.sh` runs `/usr/local/webpulse/bin/webpulse collect --quiet` with a
20 minute timeout. Both scripts exit 0 always (never block stats).

## Cron — `/etc/cron.d/webpulse`
```
*/5 * * * * root /usr/local/webpulse/bin/webpulse collect --quiet
*   * * * * root /usr/local/webpulse/bin/webpulse collect --hooked --quiet
10  2 * * * root /usr/local/webpulse/bin/webpulse rollup --quiet
30  4 1 * * root /usr/local/webpulse/bin/webpulse geoip-update --quiet
```

## Installer checks
`install.sh` refuses to run when not root, when `/usr/local/cpanel/cpanel` is
missing (unless `--standalone`, which installs only CLI + dev server), when no
binary can be obtained (see `docs/INSTALL.md`: release download with sha256
verification, `--build`, `--binary`) or when the binary does not run on the
machine. It looks for cPanel's PHP (`/usr/local/cpanel/3rdparty/bin/php`, then
`php` in PATH) only to warn that the cPanel page needs it. It prints the exact
command that failed when any cPanel registration step fails, leaving the system
consistent (it uninstalls what it registered).
