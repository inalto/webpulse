# Changelog

All notable changes to WebPulse are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.4] - 2026-10-16

### Fixed
- Meridian hub: the browser tab now reads "cPanel - WebPulse Analytics"
  instead of the untranslated key `nav.webpulse`.
- Documentation now states precisely what Meridian allows (fixed sidebar;
  search palette and `#/webpulse` route).

## [0.2.3] - 2026-10-16

### Added
- cPanel Meridian: WebPulse is now reachable inside the Meridian shell as a
  registered hub — via the command palette (Ctrl+K → "WebPulse") and the
  `#/webpulse` route — rendering the dashboard natively. The Meridian sidebar
  itself is a fixed list of built-in hubs on cPanel 138 and cannot be extended
  by plugins; the hub will show up there automatically if a future version
  allows it. `webpulse meridian-registry add|remove` maintains the registry
  entry and a `System::upcp` post hook re-applies it after cPanel updates.
- `frame.live.php`: the dashboard as a standalone document for embedding.

## [0.2.2] - 2026-10-16

### Fixed
- cPanel Meridian: the plugin now appears in the side menu. The plugin
  descriptor is staged at `/var/cpanel/plugins/webpulse/` with a
  `menu/LeftMenu.yaml` entry, which cPanel's menu builder merges into the
  Meridian navigation; uninstall removes it and rebuilds the menu cache.

## [0.2.1] - 2026-10-16

### Fixed
- cPanel Jupiter: the Metrics icon was invisible because Jupiter flattens all
  application icons into one sprite SVG and drops `<defs>`; the icon now uses
  solid fills only (no gradient).
- cPanel Meridian: the plugin was only registered in Jupiter; the installer now
  installs into every theme that ships a `dynamicui.conf` (Jupiter, Meridian).
  Existing installations pick this up with `install.sh --update`.

## [0.2.0] - 2026-10-16

### Rewritten in Rust

WebPulse is now **one statically linked Rust binary**, `webpulse`, that replaces the whole PHP library:
collector, registry, rollup, CLI, reports, JSON API, page rendering, dev server and the new CGI entry
point. The dashboard (`ui/*.js`, `ui/app.css`) is unchanged and embedded in the binary. Command line,
databases, metrics, API and `config.json` keys are compatible with 0.1.0: a 0.2.0 binary opens a 0.1.0
database without migration and the other way round, so an update can be rolled back
(`docs/RUST-PORT.md` has the compatibility contract; `tests/parity/run.sh` compares both engines).

### Changed
- **No PHP on the server any more** except cPanel's own, which runs the two LiveAPI shims. The installer no
  longer looks for a PHP with `pdo_sqlite`, no longer rewrites a shebang and no longer copies `lib/` and `ui/`
  into the cPanel plugin directory. `php_bin` in `config.json` is ignored.
- `cpanel/webpulse/index.live.php` and `api.live.php` are thin shims: they resolve the account with
  `LiveApiUser` and run `webpulse cgi --principal user:<account> --page --embedded` / `--api` as that account
  (`proc_open`, CGI variables in the environment, request body on stdin, `Status:` and headers relayed).
  `lib/bootstrap.php` is gone, `lib/CgiBridge.php` is new. On failure only a generic message is shown.
- `whm/cgi/index.cgi` and `api.cgi` are bash scripts that `exec webpulse cgi --whm ...` (the `#WHMADDON` and
  `#ACLS` markers are kept). `whm/lib/Edge.php` is gone: its checks (principal from `REMOTE_USER`, POST plus
  `X-WebPulse-Request` for mutations) live in the binary.
- `hooks/stats_runuser_pre.sh` extracts `data.user` with plain bash instead of cPanel's PHP.
- `install.sh` installs a release binary: it detects the architecture (x86_64, aarch64), resolves the latest
  release or `--ref <tag>`, downloads `webpulse-<version>-<target>.tar.gz` and its `.sha256`, **verifies the
  checksum** and installs `bin/webpulse` (0755, root). `--ref` now means a release tag; `main` means the
  latest release.
- Faster and lighter: no interpreter start-up, no `memory_limit` or `max_execution_time`.
- Continuous integration runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --release` (which
  includes the packaging tests), the parity harness and shellcheck.
- `Makefile`: `build`, `test` (`cargo test` + parity), `lint`, `fixtures`, `serve`, `serve-admin`, `dist`,
  `release`, `geoip-fixture`.
- Every test is a Rust integration test (`cargo test`), the packaging tests included: `tests/pkg_static.rs`,
  `pkg_hooks.rs`, `pkg_installer.rs`, `pkg_frontends.rs` and `pkg_dist.rs` replace `tests/run.php` and
  `tests/cases/pkg_*.php`. PHP is needed only to run the cPanel shims under test (skipped without `php`) and by
  the parity harness, which checks out the 0.1.0 release.

### Fixed
- `scripts/build_release.sh` and `scripts/build_dist.sh` could reject a complete archive ("... is missing from
  the archive") on a busy machine: `printf | grep -q` under `pipefail` failed when `grep` exited first. The
  installer's checks of cPanel tool output had the same pattern.

### Added
- `webpulse cgi (--principal user:NAME | --whm) (--api | --page) [--embedded] [--api-url URL]`, the CGI entry
  point used by the shims.
- `install.sh --build` (build from source with `cargo`; also the fallback when a release has no binary for the
  architecture and `cargo` is present, otherwise a clear error), `--binary <file>` (air-gapped installs, custom
  builds) and the environment overrides `WEBPULSE_BUILD`, `WEBPULSE_BINARY`, `WEBPULSE_DOWNLOAD_URL`,
  `WEBPULSE_DOWNLOAD_SHA256`, `WEBPULSE_SOURCE_URL`, `WEBPULSE_TARGET`, `WEBPULSE_CARGO`.
- `scripts/build_release.sh` (binary tarball with `bin/webpulse`, `cpanel/`, `whm/`, `hooks/`, `etc/`,
  `installer/`, `install.sh`, `uninstall.sh`, `VERSION`, `LICENSE`, `README.md`, `CHANGELOG.md` plus `.sha256`),
  `scripts/devenv.sh`, `scripts/geoip_fixture.sh` and `.github/workflows/release.yml`, which builds static
  `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` binaries with `cargo-zigbuild` on `v*` tags and
  publishes the tarballs, checksums and the source tarball.
- Packaging tests for the binary flow: checksum verification, missing release asset, release resolution against
  GitHub URLs, `--binary`, `--build` with and without `cargo`, shim passthrough of status, headers and body,
  request-body piping, WHM shim `exec` arguments, release tarball layout.

### Removed
- **The PHP implementation**: `lib/` (the whole `WebPulse\` library), the PHP launcher `bin/webpulse`, the PHP
  test runner and suites (`tests/run.php`, `tests/cases/`, `tests/support/`) and `tests/gen_logs.php` (replaced
  by `webpulse dev gen-logs`). The only PHP left is what cPanel requires, the two LiveAPI shims
  (`cpanel/webpulse/{index,api}.live.php` and `cpanel/webpulse/lib/{LiveApiUser,CgiBridge}.php`), plus the
  parity driver `tests/parity/php_api.php`, which drives the 0.1.0 release checked out from tag `v0.1.0`.
- `whm/lib/Edge.php`, `cpanel/webpulse/lib/bootstrap.php`, the installer's PHP detection and `wp_json_patch`.

## [0.1.0] - 2026-10-16

First public version.

### Added
- Log-based collector (`bin/webpulse collect`) with inode/offset checkpoints, sessionizer, bot, referrer,
  user-agent and optional GeoIP classification, one SQLite database per domain, nightly rollup.
- cPanel plugin (Jupiter, *Metrics* group) with a per-domain dashboard, and WHM plugin with the server-wide
  overview, per-account and per-domain collection switches, settings, jobs and CSV export.
- JSON API shared by both front-ends (`WebPulse\Http\Api`) with user, reseller and administrator principals.
- Packaging for cPanel & WHM: `install.json`, AppConfig (`/var/cpanel/apps/webpulse.conf`), WHM CGI front-ends,
  48x48 icons (generated by `scripts/make_icons.py` with the Python standard library only).
- Standardized hooks `Stats::RunAll` and `Stats::RunUser` (stage `pre`) that drain the access logs before
  `cpanellogd` truncates or archives them, and `/etc/cron.d/webpulse` (collect every 5 minutes, hooked
  collect every minute, nightly rollup, monthly GeoIP refresh).
- `install.sh` (`curl | bash` or from a checkout): versioned releases with an atomic symlink swap, rollback when a
  cPanel registration step fails, idempotent `--update`, `--uninstall` / `--purge`, `--standalone`,
  `--geoip`, `--no-cron`, `--ref`, `--repo`, `--all-themes`, and environment overrides for testing in a
  temporary prefix with a fake cPanel root.
- `scripts/build_dist.sh` builds `dist/webpulse-<version>.tar.gz` from a clean `git archive`.
- Documentation: architecture, data model, metrics, API, dashboard, cPanel integration, installation guide.
- Continuous integration on PHP 8.1 and 8.3 (`make test`, shellcheck).

### Security
- WHM `admin.set_domain`, `admin.run` and `admin.purge` only accept a domain of the account; the collector reads
  shared domlog directories (no `{user}`) and a shared archive directory only for domains cPanel lists for the
  account, and the global collect re-syncs each account right before collecting it.
- Open visits are capped per domain (`max_open_visits`, default 50 000; the oldest are closed early with their
  counters); stored visits beyond the cap or already stale are closed in batches instead of being loaded; the
  global collect runs every account in its own child process.
- `settings.json` and archives are opened non-blocking and checked with `fstat` (a FIFO cannot hang the
  collector); gzip archives are inflated in small slices with an output budget.
- `webpulse serve` requires a random token (cookie after the first load), checks the Host header and refuses to
  run as root without `--allow-root`.
- MaxMind DB decoding is bounded (node budget, no pointer-to-pointer, metadata validated first); `geoip-update`
  streams to disk with a 64 MiB cap and follows at most one redirect, to https.
- `Fs::removeTree` never follows symlinks and leaves account-owned directories to `rm -rf --one-file-system`.
- `admin.run` refuses duplicate jobs and more than 3 running jobs (409) and no longer returns the command line;
  resellers no longer read global settings; log messages are sanitised; `\z` anchors in name validation and a
  single account-name rule.

[0.2.4]: https://github.com/inalto/webpulse/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/inalto/webpulse/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/inalto/webpulse/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/inalto/webpulse/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/inalto/webpulse/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/inalto/webpulse/releases/tag/v0.1.0
