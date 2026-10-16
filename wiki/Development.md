# Development

You do not need cPanel to work on WebPulse. The engine, the API and the dashboard run against a fake
server root, and the installer can be exercised in a temporary directory. This page covers how to run
things, where the code lives and the rules for contributions.

## Requirements

A stable Rust toolchain ([rustup](https://rustup.rs)) and a C compiler (SQLite is bundled), plus `make`.
Optional: `php` (the tests run the two cPanel shims with it and skip them without it; PHP is not used by
the program), `php` 8.1 or later with `pdo_sqlite` plus `sqlite3` and `jq` for the parity harness,
`shellcheck` and Node.js (JavaScript syntax check).

```bash
git clone https://github.com/inalto/webpulse.git
cd webpulse
export PATH=$HOME/.cargo/bin:$PATH
cargo build --release          # -> target/release/webpulse
```

## Make targets

| Command | What it does |
|---------|--------------|
| `make build` | `cargo build --release`. |
| `make test` | `cargo test --release` (engine, web and packaging tests), then the parity harness (`tests/parity/run.sh`) when `php` is installed. |
| `make lint` | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, shellcheck and `php -l` of the cPanel shims. |
| `make fmt` | `cargo fmt`. |
| `make fixtures` | Build `tests/tmp/dev/`: 45 days of generated sample logs (seed 42), a development config, then setup, collect, backfill and rollup, all with the Rust binary. |
| `make serve` | Build the sample environment if needed and serve the dashboard on `127.0.0.1:8099` as the account `alice`. |
| `make serve-admin` | Same, with the WHM administrator view (server view). |
| `make geoip-fixture` | Download the free DB-IP country database into `tests/fixtures/geoip/` (needs internet). The dev environment then shows countries; GeoIP tests are skipped without it. |
| `make dist` | Build the source tarball `dist/webpulse-<version>.tar.gz` from a clean `git archive`. Needs git. |
| `make release` | Build the binary tarball `dist/webpulse-<version>-<target>.tar.gz` and its `.sha256` (`TARGET=aarch64-unknown-linux-musl`, `ZIG=1` for `cargo zigbuild`). |
| `make clean` | Remove `tests/tmp` and `dist`. |

Variables: `CARGO`, `PHP` (default `php`), `PORT` (default `8099`), `TARGET`, `ZIG`.

## The dev server

```bash
make serve
```

It prints a URL with a random token, for example `http://127.0.0.1:8099/?token=...`. Open that exact
URL; requests without the token are refused. The server listens on `127.0.0.1` only. You can also run
it yourself against your own config:

```bash
WEBPULSE_CONFIG=tests/tmp/dev/config.json target/release/webpulse serve --port 8099 --user alice
target/release/webpulse serve --admin        # the WHM administrator principal
target/release/webpulse serve --reseller reseller1
```

## Running the engine without cPanel

The test runner and `make fixtures` write a config whose `paths` point into a fake root:

```
etc/userdomains                      example.com: alice / shop.example.com: alice / blog.net: bob
var/cpanel/users/alice               DNS=example.com DNS1=shop.example.com OWNER=root
var/cpanel/users/bob                 DNS=blog.net OWNER=reseller1
etc/apache2/logs/domlogs/alice/...   example.com, example.com-ssl_log, shop.example.com
etc/apache2/logs/domlogs/bob/blog.net
home/alice/logs/example.com-Sep-2026.gz
```

```bash
WEBPULSE_CONFIG=tests/tmp/dev/config.json target/release/webpulse collect
WEBPULSE_CONFIG=tests/tmp/dev/config.json target/release/webpulse report --user alice --domain example.com
```

The sample logs come from `webpulse dev gen-logs`, a generator with a seed (diurnal traffic, bots, referrers,
404s, downloads, IPv6, malformed lines, an SSL log and an archive).

## Tests

```bash
cargo test --release                      # every test: engine, web, CLI and packaging
cargo test --release --test pkg_installer  # one file (tests/pkg_installer.rs); add a name filter to narrow
tests/parity/run.sh                       # Rust vs the 0.1.0 PHP release (needs php >= 8.1)
```

The packaging tests (`tests/pkg_*.rs`) are Rust integration tests like the others. They run `install.sh`
in a temporary directory with `WEBPULSE_PREFIX` and `WEBPULSE_FAKE_CPANEL=1`, stub cPanel tools that log
their invocations, a stub binary in the release layout and `file://` downloads (checksum verification,
missing release asset, `--binary`, `--build` with a fake `cargo`, and one install of the real binary). They
run the cPanel shims with the `php` CLI (a stub `CPANEL` class is prepended, `WEBPULSE_BIN` points at a
stub binary that prints a CGI response; skipped when `php` is missing) and the bash WHM CGIs, and check the
hooks, the tarballs and the static files. They cannot show how the real cPanel tools behave. The parity
harness checks out tag `v0.1.0`, runs the 0.1.0 PHP engine and the Rust binary on the same generated logs
and compares tables, counters and the JSON of every API action.

## Releases

Bump `VERSION` and the `version` in `Cargo.toml`, update `CHANGELOG.md`, commit and push a tag `vX.Y.Z`.
`.github/workflows/release.yml` builds static `x86_64` and `aarch64` musl binaries with `cargo-zigbuild`,
packs them with `scripts/build_release.sh` (tarball + `.sha256`), builds the source tarball and uploads
everything to the GitHub release; `install.sh` installs from there. CI (`ci.yml`) runs `cargo fmt --check`,
`cargo clippy -D warnings`, `cargo test` (packaging tests included), the parity harness and shellcheck.

## Code layout

```
Cargo.toml, src/        the program (one crate, one binary): engine, reports, http, cli
ui/                     app.js, admin.js, charts.js, i18n.js, app.css (embedded in the binary)
cpanel/                 cPanel plugin: install.json, icon, index.live.php + api.live.php (PHP shims), lib/
whm/                    WHM plugin: AppConfig, bash CGI shims, icon
hooks/                  Standardized Hook scripts (bash)
etc/                    config.json.default, cron.d/webpulse
install.sh, uninstall.sh, installer/lib.sh   the installer
scripts/                build_release.sh, build_dist.sh, devenv.sh, geoip_fixture.sh, make_icons.py, publish_wiki.sh
tests/                  Rust integration tests (engine_*, http_*, pkg_*), parity/, fixtures/
docs/                   technical documentation
wiki/                   the pages of this wiki
```

## Contributing rules

- Rust 2024; `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must be clean. Keep the dependency
  list short and the binary static.
- Errors are `Result`s inside and exit codes or JSON errors at the edge. The collector never dies on a
  bad line or a bad file: it logs and continues.
- Every user-visible UI string goes through `t()` and lives in `ui/i18n.js` (English only for now).
- Values from logs and requests are untrusted: use `textContent`, prepared statements and whitelisted
  names.
- The PHP and bash shims stay thin; logic belongs in the binary, where `cargo test` covers it.
- Add tests with your change, and run `make test` before you open a pull request.
- Metric definitions in `docs/METRICS.md` are normative: the engine, tests and dashboard tooltips must
  agree with them. Update the document when behaviour changes.

## Publishing this wiki

The wiki pages live in the `wiki/` directory of the main repository, and `scripts/publish_wiki.sh`
copies them to the GitHub wiki repository. Edit the files in `wiki/`, not on the GitHub web page. See the
header of the script for the one-time step of creating the wiki.

## See also

- [[How It Works]]
- [[Command Line]]
- [[Roadmap]]
- [Technical development notes](https://github.com/inalto/webpulse/blob/main/docs/DEVELOPMENT.md)
