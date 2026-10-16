# Implementation plan & contracts

Three work packages are implemented in parallel by independent agents. Each
package owns a disjoint set of paths (listed below) and must not create or
edit files outside of them. Cross-package calls go through the contracts in
this file; if a contract turns out to be insufficient, extend it in the
*callee's* package in a backward-compatible way and document it here.

All packages: read `docs/*.md` first (ARCHITECTURE, METRICS, DATA-MODEL, API,
DASHBOARD, CPANEL-INTEGRATION, DEVELOPMENT). PHP ≥ 8.1, no dependencies.
Every package ships its own tests under `tests/cases/<package>_*.php`.

## WP1 — Engine & CLI (owner: engine agent)

Owns: `bin/webpulse`, `lib/autoload.php`, `lib/WebPulse/App.php`, `Config.php`,
`Paths.php`, `Log.php`, `Util/*`, `Registry/*`, `Discovery/*`, `Log/*`
(LogSource, LogParser), `Classify/*` (UserAgent, Referrer, Path, GeoIp, Bots),
`Collect/*` (Collector, Sessionizer, Counters), `Store/*` (DomainStore,
Migrator, Schema), `Rollup/*`, `Cli/Cli.php`, `Cli/Command/{Collect,Backfill,
Rollup,Status,Accounts,Domains,ConfigCmd,Setup,GeoipUpdate,Purge,Hooked}Command.php`,
`etc/config.json.default`, `tests/run.php`, `tests/gen_logs.php`,
`tests/fixtures/**`, `tests/cases/engine_*.php`, `Makefile`.

Deliverables
* Everything in ARCHITECTURE.md "Collector pipeline", METRICS.md and
  DATA-MODEL.md, including the pure-PHP MMDB reader (test it against the free
  DB-IP country lite file downloaded into `tests/fixtures/geoip/` by
  `make geoip-fixture`; skip the test gracefully offline).
* `bin/webpulse` subcommands listed in README.md plus `--job` (records a row
  in `registry.jobs`), `--config <file>`, `--verbose`, `--quiet`, `--json`.
  `setup` creates dirs/config/registry with correct ownership and modes,
  idempotent. `geoip-update` downloads
  `https://download.db-ip.com/free/dbip-country-lite-YYYY-MM.mmdb.gz`
  (current month, fall back to previous) when `geoip.enabled`.
* `tests/gen_logs.php --seed 42 --days 45 --lines 60000 --out <dir>` writes
  realistic combined logs for the fixture domains (diurnal curve, weekend
  dip, ~30 % bots incl. AI crawlers, search/social/direct/UTM referrers,
  mobile/desktop UA mix, 404s, a few 5xx, downloads, static assets, IPv6,
  malformed lines, an SSL log, and a September `.gz` archive).
* Performance test: 200 k lines in < 10 s on the dev box, memory < 128 MB.
* Correctness tests: parser edge cases, classifier tables, sessionizer
  (timeout, midnight crossing, bounce −1/+1, exit page move, open visits
  surviving a second run, truncation and inode change, idempotent re-run of
  an already-imported archive), MMDB lookup, discovery, enable/disable logic.

## WP2 — Reports, HTTP API and dashboard UI (owner: ui agent)

Owns: `lib/WebPulse/Reports/*` (Range, Dimension (ALL list + per-dim
metadata: default sort, which metrics apply, pretty-name hints), Overview,
Report, Realtime, VisitsLog, Health, Export), `lib/WebPulse/Http/*`
(Request, Response, Api, Principal, DomainResolver, Page, PageOptions, Admin*
handlers calling WP1's Registry and spawning CLI jobs), `lib/WebPulse/Dev/*`
(dev router), `Cli/Command/{Serve,Report}Command.php`, `ui/**`,
`tests/cases/ui_*.php`.

Deliverables
* API.md in full. `Dimension::ALL` is the single source of truth for dim
  names and is also used by WP1 (WP1 may write a temporary constant list; the
  values are fixed in DATA-MODEL.md so both agree).
* DASHBOARD.md in full, including the WHM Server tab (`ui/admin.js`), the
  Settings tab and CSV export. Load the `dataviz` and `frontend-design`
  skills before writing charts and CSS. Verify visually with the Playwright
  MCP tools (`browser_navigate` to the dev server, `browser_take_screenshot`
  at 1280 and 390 px, light and dark) and fix what looks wrong; save
  screenshots to `docs/screenshots/`.
* `Page::render()` inlines `ui/app.css`, `ui/charts.js`, `ui/i18n.js`,
  `ui/app.js` and (for admin principals) `ui/admin.js`, escapes the bootstrap
  JSON for `<script>` (`JSON_HEX_TAG|JSON_HEX_AMP|JSON_UNESCAPED_SLASHES`),
  sets a strict CSP meta (`default-src 'none'; script-src 'nonce-…';
  style-src 'nonce-…'; img-src data:; connect-src 'self'`).
* Tests: Range parsing (all shortcuts, month boundaries, tz), every API
  action against a DB produced by WP1's engine (the test builds it by running
  `bin/webpulse collect` on fixtures — if WP1 is not finished yet, write a
  small `tests/support/seed_db.php` that fills the schema from DATA-MODEL.md
  directly, and keep it as a fast unit fixture), authorization (user cannot
  read another user's domain → 404, reseller scope, admin), CSV output,
  input validation (bad dim → 400).

## WP3 — cPanel/WHM packaging, installer, docs (owner: packaging agent)

Owns: `cpanel/**`, `whm/**`, `hooks/**`, `etc/cron.d/webpulse`,
`install.sh`, `uninstall.sh`, `installer/**` (shared bash lib), `scripts/**`
(`make_icons.py` generating the PNG/SVG icons with the Python stdlib only,
`build_dist.sh`), `docs/INSTALL.md`, `CHANGELOG.md`, `tests/cases/pkg_*.php`,
`.github/workflows/ci.yml`.

Deliverables
* Everything in CPANEL-INTEGRATION.md: `cpanel/plugin/install.json`,
  `cpanel/webpulse/index.live.php`, `api.live.php` (LiveApiUser resolution
  cascade, then `Api::dispatch`), `whm/webpulse.conf`, `whm/cgi/index.cgi`,
  `api.cgi` (REMOTE_USER → Principal via `/var/cpanel/users` OWNER parsing —
  implement that parsing in `installer/`? No: it belongs to WP1's
  `Discovery\CpanelUsers::owner(string $user): ?string`; call it), hook
  scripts with `--describe`, cron file.
* `install.sh`: POSIX-ish bash, `set -euo pipefail`, works via
  `curl … | bash` and `bash install.sh` from a checkout; downloads
  `https://codeload.github.com/${WEBPULSE_REPO}/tar.gz/${WEBPULSE_REF}` into
  a temp dir, verifies the tarball contains `bin/webpulse`, installs to
  `/usr/local/webpulse` (rsync-free: `cp -a` to a versioned dir + atomic
  symlink swap `/usr/local/webpulse -> /usr/local/webpulse-releases/<ver>`),
  runs `bin/webpulse setup`, copies plugin files, registers everything,
  installs cron, optionally GeoIP, runs a first `collect`, prints a summary
  with the WHM and cPanel URLs. Flags per README.md. `--update` re-runs the
  same steps idempotently. `--uninstall` reverses registrations (and `--purge`
  deletes `/var/webpulse`). Every cPanel command is wrapped so that a failure
  prints the command and its output and triggers rollback.
* `uninstall.sh` = `install.sh --uninstall` shim.
* `scripts/make_icons.py`: writes `cpanel/webpulse/webpulse.svg` (48×48) and
  `whm/addon_plugins/webpulse.png` (48×48 RGBA, transparent) — a simple pulse
  line in a rounded square — using only `zlib`/`struct`.
* `tests/cases/pkg_*.php`: `bash -n` on all shell scripts, `php -l` on the
  live.php/cgi files, `install.json` and `webpulse.conf` validation against
  CPANEL-INTEGRATION.md, hook `--describe` output is valid JSON, PNG header
  check, `install.sh --help` works, and a **dry-run** install into a fake root
  (`WEBPULSE_PREFIX=/tmp/x WEBPULSE_FAKE_CPANEL=1`) that exercises every
  branch without touching the real system.
* `docs/INSTALL.md`: manual steps, update, uninstall, troubleshooting
  (plugin not showing → `/usr/local/cpanel/scripts/install_plugin` output,
  `dynamicui`; WHM link missing → `register_appconfig`, cpsrvd restart;
  no data → check cron, permissions on domlogs, `webpulse status`).
* `.github/workflows/ci.yml`: `make test` on ubuntu-latest with PHP 8.1 and
  8.3 (`shivammathur/setup-php`).

## Contracts between packages

```php
namespace WebPulse;

final class App {                                   // WP1
    public static function boot(?string $configPath = null): App;   // reads WEBPULSE_CONFIG env when null
    public Config $config; public Paths $paths; public Log $log;
    public function registry(): Registry\Registry;                   // lazy
    public function openDomain(string $user, string $domain, bool $readOnly): Store\DomainStore; // throws Store\NotFound
    public function version(): string;                               // from VERSION file at repo root (WP1 creates it: "0.1.0")
}

final class Config {                                // WP1
    public function get(string $dottedKey, mixed $default = null): mixed;  // merged: defaults < file < registry.settings
    public function effective(): array;                                     // for admin.settings
    public function setOverride(string $dottedKey, mixed $value): void;     // writes registry.settings
}

final class Paths {                                 // WP1
    public function dataDir(): string; public function userDir(string $user): string;
    public function domainDb(string $user, string $domain): string;
    public function userSettingsFile(string $user): string;
    public function registryDb(): string; public function logDir(): string; public function runDir(): string;
    public function root(): string;                                        // install dir (repo root)
}

namespace WebPulse\Registry;
final class Registry {                              // WP1
    public function accounts(?string $owner = null, ?string $q = null): array;   // AccountRow[] with effective enabled
    public function domains(?string $user = null): array;                        // DomainRow[]
    public function setAccount(string $user, ?bool $enabled, ?string $timezone = null): void;
    public function setDomain(string $user, string $domain, ?bool $enabled): void;
    public function isEnabled(string $user, string $domain): bool;
    public function summaries(?array $users = null): array;                      // domain_summary rows
    public function jobs(int $limit = 50): array;  public function startJob(string $type, ?string $scope): int;
    public function finishJob(int $id, string $status, array $counters = [], ?string $message = null): void;
    public function salt(string $day): string;     public function settings(): array; public function setSetting(string $k, mixed $v): void;
}

namespace WebPulse\Discovery;
final class CpanelUsers {                           // WP1
    public function all(): array;                                   // ['alice' => ['owner'=>'root','domains'=>[…],'home'=>'/home/alice'], …]
    public function owner(string $user): ?string;
    public function domainsOf(string $user): array;                 // ['example.com' => 'main', 'shop.example.com' => 'sub', …]
    public function exists(string $user): bool;
}

namespace WebPulse\Store;
final class DomainStore {                           // WP1 — read side used by WP2
    public function pdo(): \PDO;                                    // read-only PDO when opened read-only
    public function meta(): array;                                  // meta table as assoc
    public function readOnly(): bool;
}

namespace WebPulse\Collect;
final class UserSettings {                          // WP1
    public static function read(Paths $p, string $user): array;    // validated, defaults applied
    public static function validate(array $raw): array;            // throws \InvalidArgumentException with messages
}

namespace WebPulse\Http;                            // WP2 — used by WP3 front-ends
final class Request {
    public static function fromGlobals(): Request;
    public function __construct(string $method, array $query, ?string $body, array $headers, string $remoteAddr);
}
final class Response {
    public function send(): void;                                   // headers + body (CGI: prints "Status:" line too when $cgi = true)
    public function __construct(int $status, string $contentType, string $body, array $headers = []);
    public function cgi(bool $on): self;
}
final class Api { public static function dispatch(App $app, Request $r, Principal $p): Response; }
final class Page {
    public static function render(App $app, Principal $p, PageOptions $o): string;  // full HTML document
}
final class PageOptions {
    public function __construct(public string $apiUrl, public bool $embedded = false /* inside cPanel chrome: no own <html>, only a fragment */, public ?string $title = null) {}
}
final class Principal { /* see API.md */ }
```

Front-end wiring (WP3):
```php
// cpanel/webpulse/api.live.php
require '/usr/local/webpulse/lib/autoload.php';
$app = WebPulse\App::boot();                 // /etc/webpulse/config.json
$user = WebPulse\Cpanel\LiveApiUser::resolve($cpanel);   // WP3 class, in cpanel/webpulse/lib/
WebPulse\Http\Api::dispatch($app, WebPulse\Http\Request::fromGlobals(),
    new WebPulse\Http\Principal(Principal::USER, $user))->send();
// whm/cgi/api.cgi
$who = getenv('REMOTE_USER'); $p = $who === 'root' ? new Principal(Principal::ADMIN, 'root')
     : new Principal(Principal::RESELLER, $who, array_keys(array_filter($app->... CpanelUsers->all(), fn($a) => $a['owner'] === $who)));
Api::dispatch($app, Request::fromGlobals(), $p)->cgi(true)->send();
```
`embedded = true` makes `Page::render()` return a `<div id="webpulse-app">…`
fragment with its `<style>`/`<script>` tags (cPanel's `header()`/`footer()`
supply the document); `false` returns a full document (WHM, dev).

## Integration & review (orchestrator)

After the three packages land: `make test`, `make serve` + screenshots,
a code review pass for security (path traversal in `domain`, SQL whitelists,
XSS in the SPA, symlink handling, privilege boundaries), then tag `v0.1.0`.
