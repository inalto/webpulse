# Manual Installation

Use this page when you cannot run `curl | bash` on the server, for example because of an audit policy.
It lists every command the installer runs, in the order it runs them. Run them as **root** on a
cPanel & WHM server with the Jupiter theme.

> The installer is tested against a fake cPanel root; the cPanel tools used below have not yet been
> verified on a live server by the project. Read each command's output.

You still need PHP 8.1 or later with `pdo_sqlite` (see [[Installation]]).

## 1. Get the code and stage the release

Download a tarball on any machine, review it, and copy it to the server, or fetch it directly:

```bash
REF=main                                   # a branch or, once published, a tag such as v0.1.0
PHP=/usr/local/cpanel/3rdparty/bin/php     # or the path of another PHP >= 8.1 with pdo_sqlite
"$PHP" -r 'exit(PHP_VERSION_ID >= 80100 && extension_loaded("pdo_sqlite") ? 0 : 1);' && echo "PHP ok"

SRC=$(mktemp -d)
curl -fsSL "https://codeload.github.com/inalto/webpulse/tar.gz/$REF" \
  | tar -xz -C "$SRC" --strip-components=1 --no-same-owner
test -f "$SRC/bin/webpulse" && VER=$(tr -d ' \t\r\n' < "$SRC/VERSION") && echo "version $VER"

REL=/usr/local/webpulse-releases/$VER
mkdir -p "$REL"
cp -a "$SRC"/. "$REL"/
```

The installer copies only `bin lib ui cpanel whm hooks etc installer scripts docs VERSION LICENSE
README.md CHANGELOG.md install.sh uninstall.sh Makefile`. You may delete `tests/` and other files
from `$REL`.

Fix the PHP in the shebang and the permissions:

```bash
sed -i "1s|^#!.*|#!$PHP|" "$REL/bin/webpulse"
find "$REL" -type d -exec chmod 755 {} +
find "$REL" -type f -exec chmod go+r,go-w {} +
chmod 755 "$REL/bin/webpulse" "$REL"/hooks/*.sh "$REL"/whm/cgi/*.cgi "$REL/install.sh" "$REL/uninstall.sh"
chown -R root:root "$REL"
```

## 2. Create the configuration and data directories

```bash
export WEBPULSE_CONFIG=/etc/webpulse/config.json
"$REL/bin/webpulse" setup
ln -sfn "$REL" /usr/local/webpulse
```

`setup` is idempotent. It creates `/etc/webpulse/config.json` from `etc/config.json.default` (mode
0600), the directories under `/var/webpulse/`, the registry database and one data directory per cPanel
account. The symlink makes `/usr/local/webpulse` point at the release; use `ln -sfn` again to switch
releases later.

## 3. cPanel plugin

```bash
T=/usr/local/cpanel/base/frontend/jupiter/webpulse
mkdir -p $T/lib
cp -a /usr/local/webpulse/cpanel/webpulse/. $T/
cp -a /usr/local/webpulse/lib/. $T/lib/
cp -a /usr/local/webpulse/ui $T/ui
cp /usr/local/webpulse/VERSION $T/VERSION
find $T -type d -exec chmod 755 {} +
find $T -type f -exec chmod 644 {} +
chown -R root:root $T
/usr/local/cpanel/scripts/install_plugin /usr/local/webpulse/cpanel/plugin --theme jupiter
```

The plugin runs as the account user, so it gets its own copy of `lib/` and `ui/`. Repeat the block
with another theme name if you use one.

## 4. WHM plugin

```bash
mkdir -p /usr/local/cpanel/whostmgr/docroot/cgi/webpulse
install -m 755 /usr/local/webpulse/whm/cgi/*.cgi /usr/local/cpanel/whostmgr/docroot/cgi/webpulse/
install -m 644 /usr/local/webpulse/whm/addon_plugins/webpulse.png /usr/local/cpanel/whostmgr/docroot/addon_plugins/
/usr/local/cpanel/bin/register_appconfig /usr/local/webpulse/whm/webpulse.conf
```

If the output of `register_appconfig` mentions a restart, run
`/usr/local/cpanel/scripts/restartsrv_cpsrvd`.

## 5. Hooks

```bash
/usr/local/cpanel/bin/manage_hooks add script /usr/local/webpulse/hooks/stats_runall_pre.sh  --manual --category Stats --event RunAll  --stage pre
/usr/local/cpanel/bin/manage_hooks add script /usr/local/webpulse/hooks/stats_runuser_pre.sh --manual --category Stats --event RunUser --stage pre
mkdir -p /var/webpulse/run/hook
chmod 1777 /var/webpulse/run/hook
chmod 0711 /var/webpulse/run
chmod o+x /var/webpulse
```

Run `manage_hooks list | grep webpulse` first; if the hooks are already listed, do not add them again.
The flag directory must be world-writable with the sticky bit because the `Stats::RunUser` hook runs as
the account user, not as root.

## 6. Cron

```bash
install -m 644 /usr/local/webpulse/etc/cron.d/webpulse /etc/cron.d/webpulse
```

If you installed the code somewhere other than `/usr/local/webpulse`, replace that path inside the file.
The schedule is in [[Command Line]].

## 7. GeoIP (optional)

```bash
/usr/local/webpulse/bin/webpulse config set geoip.enabled true
/usr/local/webpulse/bin/webpulse geoip-update
```

The installer writes `geoip.enabled` into `config.json`; `config set` stores it in the registry
instead, with the same effect.

## 8. First collection and check

```bash
nohup /usr/local/webpulse/bin/webpulse collect --quiet </dev/null >/dev/null 2>&1 &
/usr/local/webpulse/bin/webpulse status
```

Then open **WHM » Plugins » WebPulse Analytics** and **cPanel » Metrics » WebPulse Analytics**.

## Differences from the installer

The installer additionally does the following, which you do not need to repeat by hand: it keeps only
the three newest releases, undoes its own work if a step fails, and replaces an existing plain
`/usr/local/webpulse` directory by moving it to `/usr/local/webpulse-releases/legacy-<timestamp>`.

## See also

- [[Installation]]
- [[Updating and Uninstalling]]
- [[Troubleshooting and FAQ]]
- [Technical installation notes](https://github.com/inalto/webpulse/blob/main/docs/INSTALL.md)
