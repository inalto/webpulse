# Manual Installation

Use this page when you cannot run `curl | bash` on the server, for example because of an audit policy.
It lists every command the installer runs, in the order it runs them. Run them as **root** on a
cPanel & WHM server with the Jupiter theme.

> The installer is tested against a fake cPanel root; the cPanel tools used below have not yet been
> verified on a live server by the project. Read each command's output.

You need no PHP, Rust or SQLite: WebPulse is one static binary (see [[Installation]]).
is the output of `uname -m`, `x86_64` or `aarch64`.

## 1. Get the release and stage it

Download the tarball on any machine, check its checksum, review the contents, and copy it to the server,
or fetch it directly:

```bash
VER=0.2.0                                  # a published release
T=$(uname -m)-unknown-linux-musl           # x86_64-unknown-linux-musl or aarch64-unknown-linux-musl
cd /root
curl -fsSLO https://github.com/inalto/webpulse/releases/download/v$VER/webpulse-$VER-$T.tar.gz
curl -fsSLO https://github.com/inalto/webpulse/releases/download/v$VER/webpulse-$VER-$T.tar.gz.sha256
sha256sum -c webpulse-$VER-$T.tar.gz.sha256        # must print: ...tar.gz: OK

REL=/usr/local/webpulse-releases/$VER
mkdir -p "$REL"
tar -xzf webpulse-$VER-$T.tar.gz --strip-components=1 --no-same-owner -C "$REL"
```

The tarball contains `bin/webpulse`, `cpanel/`, `whm/`, `hooks/`, `etc/`, `installer/`, `install.sh`,
`uninstall.sh`, `VERSION`, `LICENSE`, `README.md` and `CHANGELOG.md` (the layout is described in the
[technical installation notes](https://github.com/inalto/webpulse/blob/main/docs/INSTALL.md)).

No release for your platform? Build the binary yourself (`cargo build --release` in a checkout, see
[[Development]]) and put it at `$REL/bin/webpulse` together with the other directories of a checkout
(`cpanel whm hooks etc installer install.sh uninstall.sh VERSION LICENSE README.md`).

Fix the permissions and check that the binary runs:

```bash
find "$REL" -type d -exec chmod 755 {} +
find "$REL" -type f -exec chmod go+r,go-w {} +
chmod 755 "$REL/bin/webpulse" "$REL"/hooks/*.sh "$REL"/whm/cgi/*.cgi "$REL/install.sh" "$REL/uninstall.sh"
chown -R root:root "$REL"
"$REL/bin/webpulse" --version
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
mkdir -p $T
cp -a /usr/local/webpulse/cpanel/webpulse/. $T/
cp /usr/local/webpulse/VERSION $T/VERSION
find $T -type d -exec chmod 755 {} +
find $T -type f -exec chmod 644 {} +
chown -R root:root $T
/usr/local/cpanel/scripts/install_plugin /usr/local/webpulse/cpanel/plugin --theme jupiter
```

The plugin consists of two small PHP shims (`index.live.php`, `api.live.php`, plus `lib/LiveApiUser.php`
and `lib/CgiBridge.php`) that run as the account user on cPanel's PHP and start
`/usr/local/webpulse/bin/webpulse cgi`. Repeat the block with another theme name if you use one.

## 4. WHM plugin

The CGI files are bash shims that `exec /usr/local/webpulse/bin/webpulse cgi --whm ...`.

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

`config set` stores the setting in the registry, where it wins over `config.json`; the installer's `--geoip`
does exactly the same.

## 8. First collection and check

```bash
nohup /usr/local/webpulse/bin/webpulse collect --quiet </dev/null >/dev/null 2>&1 &
/usr/local/webpulse/bin/webpulse status
```

Then open **WHM » Plugins » WebPulse Analytics** and **cPanel » Metrics » WebPulse Analytics**.

## Differences from the installer

The installer additionally does the following, which you do not need to repeat by hand: it keeps only
the three newest releases, verifies the download, undoes its own work if a step fails, and replaces an existing plain
`/usr/local/webpulse` directory by moving it to `/usr/local/webpulse-releases/legacy-<timestamp>`.

## See also

- [[Installation]]
- [[Updating and Uninstalling]]
- [[Troubleshooting and FAQ]]
- [Technical installation notes](https://github.com/inalto/webpulse/blob/main/docs/INSTALL.md)
