# Development

Nothing here needs cPanel. The whole engine and dashboard run against a fake
server root under `tests/fixtures/fakeroot/`.

```bash
make test          # php -l on every file + unit/integration tests (tests/run.php)
make fixtures      # regenerate deterministic sample logs (tests/gen_logs.php, seed 42)
make serve         # collect fixtures into tests/tmp and serve the dashboard on http://127.0.0.1:8099/?token=… (URL printed)
make serve-admin   # same, with the WHM admin principal
make dist          # build dist/webpulse-<version>.tar.gz (what install.sh downloads for a tag)
make lint          # php -l + basic JS syntax check with node --check
```

## Layout

```
bin/webpulse                 CLI entry point (php), dispatches WebPulse\Cli\Command\*
lib/autoload.php             PSR-4 autoloader for the WebPulse\ namespace (no Composer)
lib/WebPulse/…               engine, store, http, reports (see IMPLEMENTATION-PLAN.md)
ui/                          dashboard sources (app.js, charts.js, app.css, admin.js) inlined by Page::render
cpanel/                      cPanel plugin files + install.json + icon
whm/                         WHM plugin files + AppConfig
hooks/                       Standardized-hook scripts
etc/                         config.json.default, cron file
install.sh / uninstall.sh    curl | bash installer (repo root)
tests/                       run.php test runner, cases/*.php, fixtures/, gen_logs.php
docs/                        this documentation
```

## Running without cPanel

`WEBPULSE_CONFIG=tests/tmp/dev/config.json bin/webpulse collect` — `make fixtures`
writes a config whose `paths` point into `tests/tmp/dev/root`. The
fake root contains:

```
etc/userdomains                      example.com: alice / shop.example.com: alice / blog.net: bob
var/cpanel/users/alice               DNS=example.com DNS1=shop.example.com OWNER=root
var/cpanel/users/bob                 DNS=blog.net OWNER=reseller1
etc/apache2/logs/domlogs/alice/example.com, example.com-ssl_log, shop.example.com
etc/apache2/logs/domlogs/bob/blog.net
home/alice/logs/example.com-Sep-2026.gz
```

`bin/webpulse serve --user alice` starts `php -S` with a router that serves
`Page::render()` at `/` and `Api::dispatch()` at `/api`, with a `user`
principal; `--admin` gives the root principal; `--reseller reseller1` a
reseller.

The dev server is not open to anyone who can reach 127.0.0.1. At start it
prints a URL with a random access token:

```
WebPulse dev server: http://127.0.0.1:8099/?token=3f9c…  (principal user:alice, config …)
```

Open that URL: the first page load trades `?token=` for an `HttpOnly`,
`SameSite=Strict` cookie and redirects to `/`; every later request (page and
`/api`) must carry the cookie or an `X-WebPulse-Dev-Token: <token>` header
(handy for `curl`). Requests whose `Host` is not `127.0.0.1:PORT` or
`localhost:PORT` are refused (DNS rebinding). A new token is generated at
every start. `serve` refuses to run as root unless `--allow-root` is given.

## Test runner

`tests/run.php` is a 100-line dependency-free runner: every `tests/cases/*.php`
returns a `callable[]` keyed by test name, assertions throw. Exit code ≠ 0 on
failure, output is TAP-like. Integration cases build a fresh temp root,
generate logs, run the CLI via `proc_open` and assert on the databases and on
the JSON API.

## Coding conventions

* PHP ≥ 8.1, `declare(strict_types=1)`, typed properties, no dynamic
  properties, `final` classes by default, no global state except
  `WebPulse\App` (config + paths + registry, built once in the entry point).
* No external PHP or JS dependencies, ever. Extensions allowed: `pdo_sqlite`,
  `json`, `mbstring` (optional, fall back to byte functions), `zlib`
  (optional, fall back to `gzip -dc`), `posix` (optional).
* Errors: exceptions inside, mapped to exit codes / JSON errors at the edge.
  The collector never dies on a bad line or a bad file; it logs and continues.
* Logging: `WebPulse\Log` → file in `log_dir`, levels debug/info/warn/error,
  `--verbose` mirrors to stderr, `--quiet` suppresses info on stdout.
* Everything user-visible in the UI goes through `t()` (English only for now,
  but strings are centralised in `ui/i18n.js`).
