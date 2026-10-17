# Installation

WebPulse installs on a cPanel & WHM server (Jupiter theme, EasyApache 4) as `root`, on Linux
x86_64 or aarch64. Since 0.2.0 it is **one static binary**: nothing else has to be installed. No PHP
(except cPanel's own, which runs the two LiveAPI shims of the cPanel page), no Rust toolchain, no
SQLite. You need `curl` or `wget`, `tar` and `sha256sum`.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash
```

Pass flags after `bash -s --`:

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --geoip --ref v0.2.0
```

The script is only a loader: it works out which release to install (`--ref`, default *the latest
release*), downloads the tarball for this CPU, verifies its checksum and runs the installer library
inside it. GitHub downloads are https-only, redirects included (`curl --proto =https --proto-redir =https`),
and an archive containing symbolic or hard links is refused before it is unpacked. From an unpacked release tarball the same script installs the files it sits next to:
`sudo ./install.sh`.

| Flag                  | Meaning |
|-----------------------|---------|
| `--geoip`             | set `geoip.enabled`, download the free DB-IP country database (a failed download is only a warning; retry with `webpulse geoip-update`) |
| `--ref <tag>`         | release to install: `v0.2.0` or `0.2.0`; default `main` = the latest release (env `WEBPULSE_REF`). A branch name is built from source and needs `cargo` |
| `--repo <owner/name>` | GitHub repository, default `inalto/webpulse` (env `WEBPULSE_REPO`) |
| `--build`             | build the binary from source (`cargo build --release --locked`) instead of downloading it; also the automatic fallback when a release has no asset for this architecture and `cargo` exists. Fails with a clear message without `cargo` |
| `--binary <file>`     | install this binary instead of the release's (air-gapped servers, custom builds); the other files still come from the release/source tree |
| `--no-cron`           | do not install `/etc/cron.d/webpulse` (then schedule `webpulse collect` yourself) |
| `--update`            | update an existing installation; safe to repeat |
| `--uninstall`         | remove everything WebPulse registered; statistics and config are kept |
| `--purge`             | with `--uninstall`: also delete `/var/webpulse` and `/etc/webpulse` |
| `--standalone`        | no cPanel: install only the CLI and the dev server (no plugins, hooks or cron) |
| `--all-themes`        | install the cPanel plugin into every directory under `/usr/local/cpanel/base/frontend/`; by default every theme with a `dynamicui.conf` (Jupiter, Meridian) is installed |
| `--theme <name>`      | additionally install into this theme (repeatable) |
| `-y`, `--yes`         | do not ask for confirmation (also implied when stdin is not a terminal, as with `curl \| bash`) |
| `-h`, `--help`        | print the options |

### What the installer does

1. Checks it runs as root and that `/usr/local/cpanel/cpanel` exists (unless `--standalone`).
2. **Obtains the release.** Running from an unpacked release (or a source checkout with a build), it uses
   that. Otherwise it maps `uname -m` to the target (`x86_64` -> `x86_64-unknown-linux-musl`, `aarch64` ->
   `aarch64-unknown-linux-musl`), resolves the tag (`--ref`, or the latest release from
   `https://api.github.com/repos/<repo>/releases/latest`), downloads
   `https://github.com/<repo>/releases/download/<tag>/webpulse-<version>-<target>.tar.gz` and the
   matching `.sha256`, **verifies the sha256** (a mismatch aborts before anything is changed) and unpacks it.
   If the release has no asset for this ref/architecture: with `cargo` it downloads the source tarball of
   **that same release tag** (`codeload.github.com/<repo>/tar.gz/refs/tags/<tag>`, never the development
   branch) and builds it (as `--build` does), without `cargo` it stops and tells you the three ways out (a
   released `--ref`, `--build`, `--binary`).
3. Copies the release to `/usr/local/webpulse-releases/<version>/` (`bin/webpulse` mode 0755 root),
   checks that the binary runs on this machine (`webpulse --version`) and runs `bin/webpulse setup`
   (creates `/etc/webpulse/config.json`, `/var/webpulse/...`; idempotent).
4. Switches `/usr/local/webpulse` to the new release with an atomic symlink swap. The last three releases
   are kept.
5. Copies the shims into `/usr/local/cpanel/base/frontend/jupiter/webpulse/` (`index.live.php`,
   `api.live.php`, `lib/LiveApiUser.php`, `lib/CgiBridge.php`, icon, `VERSION`; no PHP library) and runs
   ```
   cp -a /usr/local/webpulse/cpanel/plugin/. /var/cpanel/plugins/webpulse/   # install.json, icon, menu/LeftMenu.yaml
   /usr/local/cpanel/scripts/install_plugin /var/cpanel/plugins/webpulse --theme jupiter   # repeat with --theme meridian
   ```
   It also looks for cPanel's PHP (`/usr/local/cpanel/3rdparty/bin/php`, then `php`) and only warns when
   there is none: the cPanel page runs on it.
6. Copies the WHM CGIs to `/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/` (`index.cgi`, Perl, and
   `api.cgi`, bash), the page template to `/usr/local/cpanel/whostmgr/docroot/templates/webpulse/index.tmpl`
   (it puts the dashboard inside WHM's top bar and left navigation), the icon to
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
8. Writes `/etc/cron.d/webpulse`, with `--geoip` runs `webpulse config set geoip.enabled true` and
   `webpulse geoip-update`, starts a first `webpulse collect` in the background and prints the URLs.

If any registration step fails the installer prints the exact command and its output, then undoes what
the run had done (an update goes back to the previous release). The system is left as it was.

## Release layout

`scripts/build_release.sh` (run by the release workflow for each target) produces
`webpulse-<version>-<target>.tar.gz` and `webpulse-<version>-<target>.tar.gz.sha256`
(`<hash>  <file name>`, as written by `sha256sum`). The tarball has one top-level directory,
`webpulse-<version>-<target>/`:

```
bin/webpulse                 the static binary (mode 0755)
cpanel/plugin/               install.json + icon, for install_plugin
cpanel/webpulse/             index.live.php, api.live.php, lib/LiveApiUser.php, lib/CgiBridge.php, icon
whm/webpulse.conf            AppConfig
whm/cgi/index.cgi api.cgi    bash shims
whm/addon_plugins/           icon
hooks/                       stats_runall_pre.sh, stats_runuser_pre.sh
etc/cron.d/webpulse          cron file
etc/config.json.default      default configuration
install.sh uninstall.sh installer/lib.sh
VERSION LICENSE README.md CHANGELOG.md
```

Each release also carries the source tarball `webpulse-<version>.tar.gz` (`scripts/build_dist.sh`, a
`git archive`) that `--build` compiles. The installer copies the same list of files into
`/usr/local/webpulse-releases/<version>/`, so `/usr/local/webpulse/install.sh` and `uninstall.sh` are
always available.

### Verifying build provenance

The release workflow attests every tarball it publishes (the two binary tarballs and the source
tarball) with [GitHub artifact attestations](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations)
(SLSA build provenance, signed through Sigstore with the workflow's OIDC identity). The checksum only
proves the download is intact; the attestation proves it was built by this repository's `Release`
workflow from the tagged commit. With the GitHub CLI (2.49 or newer):

```bash
gh attestation verify webpulse-<version>-x86_64-unknown-linux-musl.tar.gz --repo inalto/webpulse
# stricter: also pin the workflow that must have produced it
gh attestation verify webpulse-<version>-x86_64-unknown-linux-musl.tar.gz --repo inalto/webpulse \
    --signer-workflow inalto/webpulse/.github/workflows/release.yml
```

A tarball that fails verification (or has no attestation) did not come from the release workflow: do
not install it. The release workflow itself has a read-only token except for the final publish job, pins
every third-party action to a commit SHA and uses no build cache.

## Building from source

Needs the Rust toolchain (`cargo`, [rustup.rs](https://rustup.rs); stable, edition 2024) and a C compiler
for the bundled SQLite.

```bash
git clone https://github.com/inalto/webpulse && cd webpulse
cargo build --release                 # -> target/release/webpulse (dynamically linked against glibc)
sudo ./install.sh --binary target/release/webpulse     # or just: sudo ./install.sh (uses target/release/webpulse)
```

For a static, distro-independent binary like the released ones:

```bash
rustup target add x86_64-unknown-linux-musl           # aarch64-unknown-linux-musl on ARM
cargo install --locked cargo-zigbuild                 # and install zig (https://ziglang.org)
scripts/build_release.sh --zigbuild --target x86_64-unknown-linux-musl   # -> dist/webpulse-<ver>-<target>.tar.gz
tar -xzf dist/webpulse-*-x86_64-unknown-linux-musl.tar.gz && sudo ./webpulse-*/install.sh
```

`sudo bash install.sh --build` does the `cargo build --release --locked` for you (in the checkout it sits
in, or on the downloaded source of `--ref`; if `/tmp` is mounted `noexec`, as on hardened cPanel servers,
set `TMPDIR` to a directory that is not, e.g. `TMPDIR=/root/tmp`). A binary built on one distribution may not start on an older
glibc: use the musl build for other servers.

## Air-gapped installation

On a machine with internet access download `webpulse-<version>-<arch>-unknown-linux-musl.tar.gz` and its
`.sha256` from the GitHub release, check `sha256sum -c` (and, optionally, `gh attestation verify`, see
[Verifying build provenance](#verifying-build-provenance)), copy the tarball to the server and:

```bash
tar -xzf webpulse-<version>-x86_64-unknown-linux-musl.tar.gz
cd webpulse-<version>-x86_64-unknown-linux-musl && sudo ./install.sh
```

The installer finds `bin/webpulse` next to itself and downloads nothing. To use your own binary add
`--binary /path/to/webpulse`.

## Where things live

| What | Path |
|------|------|
| Code (symlink) | `/usr/local/webpulse` -> `/usr/local/webpulse-releases/<version>`; the program is `bin/webpulse` |
| Config | `/etc/webpulse/config.json` (0600) |
| Statistics | `/var/webpulse/` (`webpulse.db`, `data/<user>/<domain>.db`, `geoip/`, `log/`, `run/`) |
| cPanel plugin | `/usr/local/cpanel/base/frontend/jupiter/webpulse/` (PHP shims) |
| WHM plugin | `/usr/local/cpanel/whostmgr/docroot/cgi/webpulse/` (`index.cgi` Perl, `api.cgi` bash), `/usr/local/cpanel/whostmgr/docroot/templates/webpulse/index.tmpl`, `.../addon_plugins/webpulse.png`, `/var/cpanel/apps/webpulse.conf` |
| Cron | `/etc/cron.d/webpulse` |

## Manual installation

Equivalent to the installer, for locked-down servers. Run as root. `$ARCH` is `x86_64` or `aarch64`.

```bash
# 1. the release (binary + support files), checksum verified
VER=0.2.0; ARCH=$(uname -m); T=$ARCH-unknown-linux-musl
cd /root
curl -fsSLO https://github.com/inalto/webpulse/releases/download/v$VER/webpulse-$VER-$T.tar.gz
curl -fsSLO https://github.com/inalto/webpulse/releases/download/v$VER/webpulse-$VER-$T.tar.gz.sha256
sha256sum -c webpulse-$VER-$T.tar.gz.sha256

mkdir -p /usr/local/webpulse-releases/$VER
tar -xzf webpulse-$VER-$T.tar.gz --strip-components=1 --no-same-owner -C /usr/local/webpulse-releases/$VER
chown -R root:root /usr/local/webpulse-releases/$VER
chmod 755 /usr/local/webpulse-releases/$VER/bin/webpulse /usr/local/webpulse-releases/$VER/hooks/*.sh \
          /usr/local/webpulse-releases/$VER/whm/cgi/*.cgi
/usr/local/webpulse-releases/$VER/bin/webpulse --version
/usr/local/webpulse-releases/$VER/bin/webpulse setup
ln -sfn /usr/local/webpulse-releases/$VER /usr/local/webpulse

# 2. cPanel plugin (shims only)
D=/usr/local/cpanel/base/frontend/jupiter/webpulse
mkdir -p $D
cp -a /usr/local/webpulse/cpanel/webpulse/. $D/
cp /usr/local/webpulse/VERSION $D/VERSION
find $D -type d -exec chmod 755 {} + ; find $D -type f -exec chmod 644 {} +
cp -a /usr/local/webpulse/cpanel/plugin/. /var/cpanel/plugins/webpulse/   # install.json, icon, menu/LeftMenu.yaml
   /usr/local/cpanel/scripts/install_plugin /var/cpanel/plugins/webpulse --theme jupiter   # repeat with --theme meridian

# 3. WHM plugin
mkdir -p /usr/local/cpanel/whostmgr/docroot/cgi/webpulse
install -m 755 /usr/local/webpulse/whm/cgi/*.cgi /usr/local/cpanel/whostmgr/docroot/cgi/webpulse/
install -m 644 /usr/local/webpulse/whm/addon_plugins/webpulse.png /usr/local/cpanel/whostmgr/docroot/addon_plugins/
install -d -m 755 /usr/local/cpanel/whostmgr/docroot/templates/webpulse
install -m 644 /usr/local/webpulse/whm/templates/webpulse/index.tmpl /usr/local/cpanel/whostmgr/docroot/templates/webpulse/
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

**From WHM (one click).** Every night the rollup asks the GitHub API for the latest release (at most
once every 20 hours; `webpulse update-check` does it by hand). When a newer version exists, the bell in
the WebPulse header shows a dot; open it (or the **Updates** card in the Server view) and choose
**Upgrade now**. The server runs `/usr/local/webpulse/install.sh --update --ref <tag> -y` in the
background as a job of type `upgrade` (its output goes to `/var/webpulse/log/upgrade.log`); the WHM
pages stay available, and the page reloads by itself when the job is done. Only root can upgrade;
resellers see the notice only. Turn the nightly check off with
`webpulse config set update.check false` (`update.repo` in `config.json` points it at a fork).

**From the shell:**

```bash
curl -fsSL https://raw.githubusercontent.com/inalto/webpulse/main/install.sh | bash -s -- --update
```

The newest release is downloaded and checksum-verified, a new release directory is staged,
`webpulse setup` migrates the schema, the symlink is swapped and the shim files are refreshed. Configuration and statistics are never modified, the previous release stays on
disk (last three are kept) and is restored automatically if the update fails half-way. Running `--update`
twice is harmless.

Roll back by hand: `ln -sfn /usr/local/webpulse-releases/<old> /usr/local/webpulse`, then refresh the
shim copies with `bash install.sh --update --ref v<old version>` (the installer always downloads the
requested release; running it from inside an installed release directory does not reinstall that directory).

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
  `cp -a /usr/local/webpulse/cpanel/plugin/. /var/cpanel/plugins/webpulse/   # install.json, icon, menu/LeftMenu.yaml
   /usr/local/cpanel/scripts/install_plugin /var/cpanel/plugins/webpulse --theme jupiter   # repeat with --theme meridian`
* The plugin only shows for accounts whose feature list includes `webpulse`
  (WHM » Feature Manager » WebPulse Analytics). The `default` feature list gets it automatically on
  most servers; custom lists need it ticked.
* `install_plugin` registers the icon in cPanel's dynamic UI definitions; look for `webpulse` in
  `/usr/local/cpanel/base/frontend/jupiter/dynamicui/` (location may differ between cPanel versions). If
  it is absent the registration did not run or failed; if present, log out of cPanel and back in so the
  page is rebuilt. Also check that the files exist: `ls /usr/local/cpanel/base/frontend/jupiter/webpulse/`.
* Using another theme? Jupiter and Meridian are detected automatically (any theme directory with a `dynamicui.conf`); for anything else install with `--theme <name>` or `--all-themes`.
* No WebPulse entry in Meridian's side menu? That is expected on cPanel 138: the Meridian sidebar is a fixed list of seven built-in hubs and no plugin can add to it. WebPulse registers a *hub* instead (`webpulse meridian-registry add …`, re-applied after cPanel updates by the `System::upcp` hook): account owners open it with **Ctrl+K → "WebPulse"** or at `…/frontend/meridian/index.html#/webpulse`. Check the registration with `grep -c '"slug": "webpulse"' /usr/local/cpanel/base/frontend/meridian/_assets/_registry.js` (must print 1; run `install.sh --update` otherwise) and make sure the account has the `webpulse` feature.
* Icon missing in Jupiter's Metrics group? Jupiter compiles every application icon into one sprite SVG and drops `<defs>`, so icons must use solid fills only (no gradients, `<title>`, `<style>` or ids). The shipped icon follows that rule since 0.2.1; after replacing an icon run `/usr/local/cpanel/bin/sprite_generator` or wait for the scheduled rebuild.

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
  reason (missing Jupiter theme, unreachable GitHub, a binary that does not run on this CPU) and run it again.

**"No prebuilt binary" / "Checksum mismatch" / "does not run on this machine"**
* *No prebuilt binary*: the ref has no release asset for this architecture. Use a released `--ref v<version>`,
  install Rust and run `install.sh --build`, or pass `--binary /path/to/webpulse`.
* *Checksum mismatch*: the download was corrupted or truncated, or was replaced. Nothing was installed;
  retry, and compare with the `.sha256` on the release page.
* *The binary does not run on this machine*: wrong architecture (the released binaries are Linux x86_64 and
  aarch64, statically linked) or a `noexec` temporary directory (`TMPDIR`/`/usr/local`).

**The cPanel page shows "not available right now" or the icon opens an empty page**
* The shim could not run the binary. Check `ls -l /usr/local/webpulse/bin/webpulse` (0755, root) and run
  `/usr/local/webpulse/bin/webpulse status` as root; the shim's cPanel PHP is
  `/usr/local/cpanel/3rdparty/bin/php`. PHP errors go to `/usr/local/cpanel/logs/error_log`.
* cPanel's PHP must allow `proc_open` (the shim starts the binary with it).

**GeoIP is empty**
* `webpulse geoip-update` needs outbound HTTPS to `download.db-ip.com`; `geoip.enabled` must be true
  (`--geoip` sets it with `webpulse config set geoip.enabled true`, stored in the registry). Countries appear for hits collected after the database is in place.

**Moving the data or testing the installer**
* `WEBPULSE_PREFIX`, `WEBPULSE_FAKE_CPANEL=1`, `WEBPULSE_CPANEL_ROOT`, `WEBPULSE_ETC`, `WEBPULSE_VAR`,
  `WEBPULSE_CRON_DIR`, `WEBPULSE_APPS_DIR` redirect every path so the installer can be exercised in a
  temporary directory without privileges (this is how `tests/pkg_installer.rs` works).
  `WEBPULSE_DOWNLOAD_URL` (release tarball; its `.sha256` is fetched next to it, or give
  `WEBPULSE_DOWNLOAD_SHA256`), `WEBPULSE_SOURCE_URL`, `WEBPULSE_TARGET`, `WEBPULSE_CARGO`,
  `WEBPULSE_BUILD=1` and `WEBPULSE_BINARY=<file>` steer where the binary comes from.
