# Development

You do not need cPanel to work on WebPulse. The engine, the API and the dashboard run against a fake
server root, and the installer can be exercised in a temporary directory. This page covers how to run
things, where the code lives and the rules for contributions.

## Requirements

PHP 8.1 or later with `pdo_sqlite` (plus `json`; `mbstring`, `zlib` and `posix` are optional), and `make`.
Node.js is optional: if present, `make lint` also checks the JavaScript syntax. `shellcheck` is used by CI
for the shell scripts.

```bash
git clone https://github.com/inalto/webpulse.git
cd webpulse
```

## Make targets

| Command | What it does |
|---------|--------------|
| `make test` | `php -l` on every PHP file (and `node --check` on the JavaScript if Node is installed), then the test suite `php tests/run.php`. |
| `make lint` | Only the syntax checks. |
| `make fixtures` | Build `tests/tmp/dev/`: 45 days of generated sample logs (seed 42), a development config, then setup, collect, backfill and rollup. |
| `make serve` | Build the sample environment if needed and serve the dashboard on `127.0.0.1:8099` as the account `alice`. |
| `make serve-admin` | Same, with the WHM administrator view (Server tab). |
| `make geoip-fixture` | Download the free DB-IP country database into `tests/fixtures/geoip/` (needs internet). The dev environment then shows countries; GeoIP tests are skipped without it. |
| `make dist` | Build `dist/webpulse-<version>.tar.gz` from a clean `git archive` (what `install.sh` downloads for a tag). Needs git. |
| `make clean` | Remove `tests/tmp` and `dist`. |

Variables: `PHP` (the interpreter, default `php`) and `PORT` (default `8099`).

## The dev server

```bash
make serve
```

It prints a URL with a random token, for example `http://127.0.0.1:8099/?token=...`. Open that exact
URL; requests without the token are refused. The server listens on `127.0.0.1` only. You can also run
it yourself against your own config:

```bash
WEBPULSE_CONFIG=tests/tmp/dev/config.json php bin/webpulse serve --port 8099 --user alice
php bin/webpulse serve --admin        # the WHM administrator principal
php bin/webpulse serve --reseller reseller1
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
WEBPULSE_CONFIG=tests/tmp/dev/config.json php bin/webpulse collect
WEBPULSE_CONFIG=tests/tmp/dev/config.json php bin/webpulse report --user alice --domain example.com
```

The sample logs come from `tests/gen_logs.php`, a generator with a seed (diurnal traffic, bots, referrers,
404s, downloads, IPv6, malformed lines, an SSL log and an archive).

## Tests

`tests/run.php` is a dependency-free runner. Every `tests/cases/*.php` file returns an array of test
names and callables; a test fails when it throws.

```bash
php tests/run.php                 # everything
php tests/run.php engine_         # files whose name contains "engine_"
php tests/run.php "sessionizer"   # a filter also matches the test name
WEBPULSE_SKIP_PERF=1 php tests/run.php
```

Case files are grouped by prefix: `engine_*` (parser, classifier, sessionizer, collector, rollup,
registry, GeoIP, performance), `ui_*` (API, authorization, ranges, page), `pkg_*` (installer, packaging,
hooks, front-ends, dist), `sec_*` (file handling, tenant isolation, web hardening). The installer test
runs `install.sh` in a temporary directory with `WEBPULSE_PREFIX` and `WEBPULSE_FAKE_CPANEL=1`; this is
the only coverage of the installer, so it cannot show how the real cPanel tools behave.

## Code layout

```
bin/webpulse            CLI entry point; dispatches WebPulse\Cli\Command\*
lib/autoload.php        PSR-4 autoloader for WebPulse\ (no Composer)
lib/WebPulse/           App, Config, Paths, Log, Util, Registry, Discovery, Log (parser), Classify,
                        Collect, Store, Rollup, Reports, Http (API, pages, principals), Dev, Cli
ui/                     app.js, admin.js, charts.js, i18n.js, app.css (inlined into the page)
cpanel/                 cPanel plugin: install.json, icon, index.live.php, api.live.php
whm/                    WHM plugin: AppConfig, CGI scripts, icon
hooks/                  Standardized Hook scripts
etc/                    config.json.default, cron.d/webpulse
install.sh, uninstall.sh, installer/lib.sh   the installer
scripts/                build_dist.sh, make_icons.py, publish_wiki.sh
tests/                  run.php, cases/, fixtures/, support/, gen_logs.php
docs/                   technical documentation
wiki/                   the pages of this wiki
```

## Contributing rules

- PHP 8.1 or later, `declare(strict_types=1)`, typed properties, no dynamic properties, `final` classes
  by default, no global state except `WebPulse\App`.
- **No external PHP or JavaScript dependencies, ever.** Allowed extensions: `pdo_sqlite`, `json`,
  `mbstring` (optional), `zlib` (optional), `posix` (optional).
- Errors are exceptions inside and exit codes or JSON errors at the edge. The collector never dies on a
  bad line or a bad file: it logs and continues.
- Every user-visible UI string goes through `t()` and lives in `ui/i18n.js` (English only for now).
- Values from logs and requests are untrusted: use `textContent`, prepared statements and whitelisted
  names.
- Add tests with your change, and run `make test` before you open a pull request. CI runs it on PHP 8.1
  and 8.3 and runs shellcheck on `install.sh`, `uninstall.sh`, `installer/lib.sh`, `hooks/*.sh` and
  `scripts/*.sh`.
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
