# Development

Nothing here needs cPanel. The engine, the API and the dashboard run against a fake server root
under `tests/fixtures/fakeroot/`. WebPulse is a single Rust crate (see `docs/RUST-PORT.md` for the
module layout); PHP survives only in the two cPanel shims and as the runner of the packaging tests
and the parity harness.

You need a stable Rust toolchain (`rustup`), a C compiler (bundled SQLite) and, for the packaging
tests, `php` >= 8.1 with `pdo_sqlite`, `sqlite3`, `jq` and `shellcheck` (all optional: the
`make` targets skip what is missing).

```bash
make build         # cargo build --release          -> target/release/webpulse
make test          # cargo test --release + php tests/run.php pkg_ + tests/parity/run.sh
make lint          # cargo fmt --check, cargo clippy --all-targets -D warnings, shellcheck, php -l of the shims
make fixtures      # fake server root with 45 days of generated logs in tests/tmp/dev (seed 42)
make serve         # dev dashboard on http://127.0.0.1:8099/?token=… as account "alice" (URL printed)
make serve-admin   # same, with the WHM administrator principal
make dist          # source tarball dist/webpulse-<version>.tar.gz   (git archive of HEAD)
make release       # binary tarball dist/webpulse-<version>-<target>.tar.gz + .sha256 (TARGET=…, ZIG=1)
make geoip-fixture # download the free DB-IP country database into tests/fixtures/geoip/
make fmt           # cargo fmt
```

## Layout

```
Cargo.toml, Cargo.lock       single binary crate "webpulse"
src/                         the program: engine, reports, http, cli (docs/RUST-PORT.md)
ui/                          dashboard sources (app.js, charts.js, app.css, admin.js), embedded in the binary
cpanel/                      plugin descriptor + the two LiveAPI shims (PHP) + lib/LiveApiUser.php, lib/CgiBridge.php
whm/                         AppConfig, icon, bash CGI shims
hooks/                       Standardized-hook scripts (bash)
etc/                         config.json.default, cron file
install.sh, uninstall.sh, installer/lib.sh    installer (bash)
scripts/                     build_release.sh, build_dist.sh, devenv.sh, geoip_fixture.sh, publish_wiki.sh, make_icons.py
tests/                       Rust integration tests (tests/*.rs), tests/parity/, packaging tests (tests/cases/pkg_*.php, tests/run.php), fixtures/
docs/                        this documentation
```

## Cargo workflow

```bash
export PATH=$HOME/.cargo/bin:$PATH
cargo build --release                 # release binary (LTO, stripped)
cargo test --release                  # unit + integration tests
cargo fmt --check && cargo clippy --all-targets -- -D warnings    # what CI enforces
target/release/webpulse --help
```

`make serve` is the quickest way to look at the dashboard: it builds the binary, builds the fake
server (`scripts/devenv.sh`: copies `tests/fixtures/fakeroot`, writes a `config.json` whose `paths`
point into `tests/tmp/dev/root`, runs `webpulse dev gen-logs`, `setup`, `collect`, `backfill`,
`rollup`) and starts `webpulse serve`. To use the fake root by hand:

```bash
WEBPULSE_CONFIG=tests/tmp/dev/config.json target/release/webpulse collect --verbose
WEBPULSE_CONFIG=tests/tmp/dev/config.json target/release/webpulse status
```

The fake root contains:

```
etc/userdomains                      example.com: alice / shop.example.com: alice / blog.net: bob
var/cpanel/users/alice               DNS=example.com DNS1=shop.example.com OWNER=root
var/cpanel/users/bob                 DNS=blog.net OWNER=reseller1
etc/apache2/logs/domlogs/alice/example.com, example.com-ssl_log, shop.example.com
etc/apache2/logs/domlogs/bob/blog.net
home/alice/logs/example.com-Sep-2026.gz
```

`webpulse serve --user alice` serves the page at `/` and the API at `/api` with a `user` principal;
`--admin` gives the root principal; `--reseller reseller1` a reseller. The dev server is not open to
anyone who can reach 127.0.0.1: at start it prints a URL with a random access token, the first
page load trades `?token=` for an `HttpOnly`, `SameSite=Strict` cookie, every later request must carry the
cookie or an `X-WebPulse-Dev-Token` header, requests whose `Host` is not `127.0.0.1:PORT` or
`localhost:PORT` are refused, and `serve` refuses to run as root unless `--allow-root` is given.

## Tests

* **Rust** (`cargo test --release`): unit tests in `src/**`, integration tests in `tests/*.rs`
  (engine, HTTP/API, CLI).
* **Packaging tests** (`php tests/run.php pkg_`): the installer end to end in a temporary prefix with a
  fake cPanel root, the PHP and bash shims, the hooks, the release tarballs and static checks. PHP is
  only the runner here (`tests/run.php` is a small dependency-free TAP-like runner, every
  `tests/cases/pkg_*.php` returns a `name => callable` array and an assertion throws). They use a
  stub binary in the release layout, a fake `curl`/`uname`/`cargo` and `file://` URLs; nothing
  outside `tests/tmp` is touched and no network is used. `pkg_installer` additionally installs the real
  `target/release/webpulse` when it exists. Run a subset with `php tests/run.php pkg_hooks` or a
  substring of a test name.
* **Parity harness** (`tests/parity/run.sh`, needs `php` >= 8.1): checks out tag `v0.1.0`, runs the PHP
  engine and the Rust binary on the same generated logs and compares the tables, the counters and the
  JSON of every API action (see `docs/RUST-PORT.md`). `make test-parity` builds first and skips quietly when
  `php` or the script is missing.

## Continuous integration and releases

`.github/workflows/ci.yml` (push to `main`/`rust`, pull requests): `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test --release`, `php tests/run.php pkg_`
(PHP 8.3, `sqlite3`), the parity harness as a separate job, and shellcheck.

`.github/workflows/release.yml` runs on tags `v*` (the tag must equal `VERSION`, which must equal the
`version` in `Cargo.toml`): it builds `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`
with `cargo-zigbuild` and zig, packs them with `scripts/build_release.sh`, builds the source tarball with
`scripts/build_dist.sh` and uploads tarballs and `.sha256` files to the GitHub release.
To cut a release: bump `VERSION` and `Cargo.toml`, update `CHANGELOG.md`, commit, `git tag v0.2.0 && git push --tags`.
The tarball layout is documented in `docs/INSTALL.md` ("Release layout"). To try the release path locally:

```bash
make release                    # host architecture, musl target must be installed: rustup target add x86_64-unknown-linux-musl
make release TARGET=aarch64-unknown-linux-musl ZIG=1    # needs cargo-zigbuild + zig
scripts/build_release.sh --binary target/release/webpulse --out /tmp/dist    # package an existing binary
```

## Trying the installer

`install.sh` can be exercised without privileges: `WEBPULSE_PREFIX=/tmp/x WEBPULSE_FAKE_CPANEL=1 WEBPULSE_CPANEL_ROOT=…`
and the other `WEBPULSE_*` overrides listed in `install.sh --help` redirect every path (this is how the
packaging tests work). From a source checkout `install.sh` installs `target/release/webpulse`
(`--build` runs cargo, `--binary FILE` takes another binary).

## Coding conventions

* Rust 2024, `cargo fmt` and `clippy -D warnings` clean. Errors are `anyhow`/`thiserror` values mapped
  to exit codes (CLI) or JSON errors (API) at the edge. The collector never dies on a bad line or a bad
  file; it logs and continues.
* Dependencies are few and pinned by `Cargo.lock`; add one only when the standard library cannot do it,
  and keep the binary static (musl) and a few megabytes.
* Logging: `crate::log` → file in `log_dir`, levels debug/info/warn/error, `--verbose` mirrors to
  stderr, `--quiet` suppresses info on stdout. Messages are sanitised centrally.
* The shims (PHP and bash) stay thin: no logic that is not "find the binary, forward the CGI variables,
  relay the response". Everything else belongs in the binary where `cargo test` covers it.
* Everything user-visible in the UI goes through `t()` (English only for now, strings are centralised
  in `ui/i18n.js`).
