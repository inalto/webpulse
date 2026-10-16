# Development

Nothing here needs cPanel. The engine, the API and the dashboard run against a fake server root
under `tests/fixtures/fakeroot/`. WebPulse is a single Rust crate (see `docs/RUST-PORT.md` for the
module layout). The only PHP in the repository is what cPanel requires, the two LiveAPI shims
(`cpanel/webpulse/*.live.php` and their `lib/`), plus `tests/parity/php_api.php`, which drives the 0.1.0
release in the parity harness. Every test is a Rust integration test run by `cargo test`.

You need a stable Rust toolchain (`rustup`) and a C compiler (bundled SQLite). Optional: `php` (the
shim tests run the cPanel shims with it and `php -l` checks them; they are skipped without it), `php`
>= 8.1 with `pdo_sqlite` plus `sqlite3` and `jq` for the parity harness, `shellcheck`, `git` and
`python3` (the tests and `make` targets skip what is missing).

```bash
make build         # cargo build --release          -> target/release/webpulse
make test          # cargo test --release (engine, web, packaging) + tests/parity/run.sh
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
tests/                       Rust integration tests (tests/*.rs: engine_*, http_*, pkg_*, config_default), tests/parity/, fixtures/
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

Everything runs with `cargo test --release` (or `cargo test`); a test that needs a missing tool prints
`SKIP: …` and passes.

* **Engine, web, CLI**: unit tests in `src/**`, integration tests `tests/engine_*.rs` (helpers in
  `tests/common/`, a fake server root) and `tests/http_*.rs` (helpers in `tests/http_common/`).
  `engine_parser` and `engine_gen_logs` also compare with the 0.1.0 PHP code when `php` and the
  `v0.1.0` checkout of the parity harness (`tests/tmp/legacy`) exist.
* **Packaging** (`tests/pkg_*.rs`, helpers in `tests/pkg_common/`): `pkg_static` (bash -n, shellcheck,
  `php -l` of the shims, `install.json` and `webpulse.conf` against CPANEL-INTEGRATION.md, icons, cron
  file, Makefile, workflows), `pkg_hooks` (`--describe`, the RunUser flag, RunAll), `pkg_installer`
  (install, update, uninstall and purge end to end in a temporary prefix with a fake cPanel root whose
  tools are logging stubs, checksums, release resolution with a fake `curl`/`uname`, `--binary`,
  `--build` with a fake `cargo`, and an install of the real binary under test), `pkg_frontends` (the
  shims) and `pkg_dist` (`build_dist.sh`, `build_release.sh`). They use a stub binary in the release
  layout and `file://` URLs; nothing outside `tests/tmp` is touched and no network is used. Each test
  works in its own directory below `tests/tmp/`, removed at the end (`WEBPULSE_KEEP_TMP=1` keeps it).
  Run one file with `cargo test --test pkg_installer`, or add a substring of a test name.
* **Shim tests and PHP**: `pkg_frontends` runs `cpanel/webpulse/api.live.php` and `index.live.php`
  with the `php` CLI. A stub `CPANEL` class (what `/usr/local/cpanel/php/cpanel.php` provides on a
  server) is loaded with `-d auto_prepend_file`, `WEBPULSE_BIN` points at a stub binary that records
  its arguments, environment and stdin and prints a canned CGI response, and the test checks the
  arguments, the forwarded CGI variables, the request body, the status and headers (through `php -S`
  for a real HTTP request) and the body. `LiveApiUser` and `CgiBridge` are exercised with small PHP
  snippets. The WHM CGIs are bash and run without PHP. Without `php` these tests are skipped.
* **Configuration**: `config_default` checks that `etc/config.json.default` equals the fallback
  `webpulse setup` writes and agrees with the built-in defaults (`src/config.rs`).
* **Parity harness** (`tests/parity/run.sh`, needs `php` >= 8.1): checks out tag `v0.1.0` into
  `tests/tmp/legacy` (a git worktree), runs the 0.1.0 PHP engine and the Rust binary on the same
  generated logs and compares the tables, the counters and the JSON of every API action (see
  `docs/RUST-PORT.md`). `make test-parity` builds first and skips quietly when `php` or the script is
  missing.

## Continuous integration and releases

`.github/workflows/ci.yml` (push to `main`/`rust`, pull requests): `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test --release` (with PHP 8.3 installed so the
shim tests run), the parity harness as a separate job, and shellcheck.

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
