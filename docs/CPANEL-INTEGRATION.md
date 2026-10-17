# cPanel & WHM integration

Verified against the cPanel developer documentation (api.docs.cpanel.net,
docs.cpanel.net) on 2026-10-16. Where the docs were silent, the behaviour is
marked *assumed* and the installer verifies it at run time and prints a clear
error instead of guessing.

Servers without cPanel (standalone mode, the Cockpit module): see
[STANDALONE-COCKPIT.md](STANDALONE-COCKPIT.md).

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
/usr/local/cpanel/scripts/install_plugin   /var/cpanel/plugins/webpulse --theme jupiter   # staged copy of cpanel/plugin/
/usr/local/cpanel/scripts/uninstall_plugin /var/cpanel/plugins/webpulse --theme jupiter
```
The installer registers the plugin in every theme that has a `dynamicui.conf`
under `/usr/local/cpanel/base/frontend/` (Jupiter and Meridian on current
servers). Icon rules learned on a real server: Jupiter merges all application
icons into `assets/application_icons/sprites/icon_spritemap.svg` with
`sprite_generator` (scheduled by `install_plugin`) and drops `<defs>`, so an
icon using a gradient or any `url(#id)` reference renders as an empty square.
Icons must use solid fills only, `viewBox` without `width`/`height`, no
`<title>`, `<style>` or ids. Meridian reads `assets/application_icons/<id>.svg`
directly (formats svg/png, see the theme's `config.json`).

**Meridian (hub registry).** Verified on cPanel 138: Meridian's side
navigation is a *fixed list of seven built-in hubs* inside the shell bundle
(`Dashboard, Websites, Email, Files, Databases, Security, Performance`), only
filtered by which of them are available. No plugin — not WP Toolkit, not
cPanel's own Site Quality Monitoring — can add a sidebar entry in this version.
What a plugin *can* do is register a hub: it gets a route (`#/<slug>`), an
entry in the command palette (Ctrl+K / the Search box) through `searchIndex`,
and will appear in the sidebar automatically should a future Meridian open it
to registered hubs. Hubs are *hub manifests* listed in
`/usr/local/cpanel/base/frontend/meridian/_assets/_registry.js` (owned by the
`cpanel-meridian-plugin` RPM and regenerated on theme updates). Legacy
dynamicui apps are not rendered there at all (`applications` is empty in the
shell data), so WebPulse ships a hub: `cpanel/meridian/manifest.json` (slug
`webpulse`, `nav.path /webpulse`, `nav.section monitor`, `dependencies.required
["webpulse"]` so it hides when the feature is off) and
`cpanel/meridian/remoteEntry.js`, a hand-written ES-module Module-Federation
remote: the shell calls `init(shareScope)` (React 19 singleton) and
`get("./module")`, then renders the exported `WebPulseHub` component (first
export ending in `Hub`) at `nav.path`. The component embeds
`webpulse/frame.live.php` (the dashboard as a full document, no cPanel chrome)
in a same-origin iframe sized to its content and sets the document title (the
shell would otherwise show the untranslated key `nav.webpulse`). Account
owners reach it with **Ctrl+K → "WebPulse"** or at `…/frontend/meridian/index.html#/webpulse`;
the classic Metrics icon still exists for Jupiter. The installer copies both files
into `meridian/webpulse/`, runs `webpulse meridian-registry add --registry
… --manifest … --entry …` (idempotent, atomic, other entries untouched) and
registers a `System::upcp` post hook (`hooks/upcp_post.sh`) that re-applies
the entry after cPanel updates; uninstall runs `meridian-registry remove`.
Contract version 1.0.0 is checked by the shell: an incompatible major is
skipped with a console warning, never an error.

**Jupiter side menu (LeftMenu.yaml).** Jupiter's left navigation uses menu files: `Cpanel::Plugins::MenuBuilder` merges
`/usr/local/cpanel/Cpanel/Plugins/menus/LeftMenu.yaml` with every
`/var/cpanel/plugins/*/**/LeftMenu.yaml` into the cache
`/var/cpanel/menus/LeftMenu.yaml`; `install_plugin`/`uninstall_plugin` trigger
that rebuild when the staging directory they are given contains a `menu/`
directory. The convention (used by cPanel's own plugins) is therefore to keep
the plugin descriptor at `/var/cpanel/plugins/<id>/` with `install.json`, the
icon and `menu/LeftMenu.yaml`, and to run `install_plugin /var/cpanel/plugins/<id>
--theme <theme>`. WebPulse ships `cpanel/plugin/menu/LeftMenu.yaml`:
```yaml
- key: webpulse        # must equal the dynamicui id (install.json "id")
  order: 35            # wp-toolkit 20, team_manager 30, koality 40
  icon: <svg …24×24… stroke="var(--cp-tertiary-contrast)" …/>
```
The icon is an inline 24×24 SVG coloured through the theme's CSS variable; the
same sprite rules apply (no `<defs>`, ids or `url(#)`). On uninstall the
installer removes `/var/cpanel/plugins/webpulse` and rebuilds the menu with
`/usr/local/cpanel/3rdparty/bin/perl -MCpanel::Plugins::MenuBuilder -e 'Cpanel::Plugins::MenuBuilder::build_menu("LeftMenu")'`.

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
/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/index.cgi    Perl CGI (cPanel's perl): the page, in WHM's chrome
/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/api.cgi      #!/bin/bash shim: the JSON API
/usr/local/cpanel/whostmgr/docroot/templates/webpulse/index.tmpl  Template Toolkit wrapper (0644, dir 0755)
/usr/local/cpanel/whostmgr/docroot/addon_plugins/webpulse.png  48×48 transparent PNG
/var/cpanel/apps/webpulse.conf
```
Both CGIs keep the WHM header comments right after the shebang:
```perl
#!/usr/local/cpanel/3rdparty/bin/perl
#WHMADDON:webpulse:WebPulse Analytics:webpulse.png
#ACLS:all
```
**The page inside WHM's chrome.** WHM CGIs get the top bar, the left navigation and the breadcrumbs
by printing `Content-type: text/html\r\n\r\n` and rendering a template that wraps itself in WHM's
master template (this is how cPanel's own CGIs do it, e.g. the changelog page). `index.cgi`
(`use strict; use warnings; use lib '/usr/local/cpanel'`):
1. calls `Whostmgr::ACLS::init_acls()` (the navigation is drawn from the ACLs);
2. runs `webpulse cgi --whm --page --embedded --api-url api.cgi` with a list-form
   `open(my $fh, '-|', $bin, @args)` (no shell; the CGI environment is inherited, stdin is
   `/dev/null`) and splits its CGI head from the body;
3. if the `Status:` is not 2xx (403 for a user that is neither root nor a reseller owning accounts,
   500) prints the binary's output unchanged;
4. otherwise prints `Content-type: text/html\r\n\r\n` and calls
   `Cpanel::Template::process_template('whostmgr', { template_file => 'webpulse/index.tmpl', print => 1, data => { body => $body } })`
   (if the template dies, the fragment is printed as is);
5. if `Cpanel::Template` cannot be loaded (not cPanel's perl) it runs the binary again without
   `--embedded` and prints the standalone page.

`index.tmpl` (under `/usr/local/cpanel/whostmgr/docroot/templates/webpulse/`):
```
[% WRAPPER 'master_templates/master.tmpl' theme="bootstrap" header="WebPulse Analytics" app_key='webpulse' include_legacy_stylesheets=0 -%]
[% data.body %]
[%- END %]
```
`app_key='webpulse'` (the AppConfig `name`) highlights the entry in the navigation and breadcrumbs.
TT does not escape `data.body`: it is our own fragment — `<div id="webpulse-app" class="wp-root"
data-embedded="1">` followed by nonce'd `<style>`/`<script>` tags, no CSP meta (WHM's page is the
document), and pure ASCII (non-ASCII characters are written as HTML, JS or CSS escapes) so that the
template engine's byte/character handling cannot garble it. The dashboard CSS rolls every element
under `.wp-root` back to the browser defaults (`:where(.wp-root) *:not(svg):not(svg *) { all: revert }`,
specificity 0,0,3) so WHM's bootstrap and legacy rules on bare elements (`td`, `label`, `a`,
`input[type=search]`…) cannot restyle cards and tables; the dark theme only applies inside `.wp-root`.
Printing (the PDF report) hides every sibling of each ancestor of `#webpulse-app`, so WHM's chrome
never reaches paper.

Both CGIs locate the binary through `$WEBPULSE_BIN` (honoured only when the file and its directory
belong to the user running the CGI, root under cpsrvd, and are not group/other-writable),
`/usr/local/webpulse/bin/webpulse`, then `bin/webpulse` next to the script, and print a generic
500 (HTML or JSON) when none exists. `api.cgi` `exec`s `webpulse cgi --whm --api`: cpsrvd's CGI
environment (`REMOTE_USER`, `REQUEST_METHOD`, `QUERY_STRING`, `CONTENT_TYPE`, `CONTENT_LENGTH`,
`HTTP_X_WEBPULSE_REQUEST`, `REMOTE_ADDR`) and the request body on stdin are inherited.
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
The CGIs run as root and `webpulse cgi --whm` refuses to run when its euid is not 0. Mutating API
actions need `POST` and `X-WebPulse-Request: 1` (the binary answers 405 otherwise).
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
The nightly rollup also runs the update check (`update.check`, at most once every 20 h): a GET of
`https://api.github.com/repos/<update.repo>/releases/latest` (10 s timeout, `User-Agent:
webpulse/<version>`, 256 KiB cap), stored in the registry `settings` row `update_status` and shown by
the WHM bell. **Upgrade now** (root) spawns `webpulse dev run-upgrade --job <id> --tag <tag>`
detached (`setsid`, `nohup`), which runs `bash /usr/local/webpulse/install.sh --update --ref <tag> -y`
with a clean environment (no CGI variables), appends to `/var/webpulse/log/upgrade.log` and finishes
the `upgrade` job from the exit status. The installer replaces the binary, the CGIs and the
template while WHM keeps serving the old ones until the symlink swap.

## Installer checks
`install.sh` refuses to run when not root, when `/usr/local/cpanel/cpanel` is
missing (unless `--standalone`, see [STANDALONE-COCKPIT.md](STANDALONE-COCKPIT.md)), when no
binary can be obtained (see `docs/INSTALL.md`: release download with sha256
verification, `--build`, `--binary`) or when the binary does not run on the
machine. It looks for cPanel's PHP (`/usr/local/cpanel/3rdparty/bin/php`, then
`php` in PATH) only to warn that the cPanel page needs it. It prints the exact
command that failed when any cPanel registration step fails, leaving the system
consistent (it uninstalls what it registered).
