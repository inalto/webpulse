# Rust port (0.2.0)

Everything that can be Rust becomes Rust. One statically linked binary,
`webpulse`, replaces the whole PHP library: collector, registry, rollup, CLI,
reports, JSON API, page rendering, dev server and the CGI entry points. PHP
survives only as the two LiveAPI shims that cPanel requires (`*.live.php` must
be PHP or Perl to use the `CPANEL` class); the WHM entry points become bash
shims that `exec` the binary. The dashboard (`ui/*.js`, `ui/app.css`) is
unchanged and embedded in the binary with `include_str!`.

Why: a 2–5 MB static binary with no interpreter dependency, ~10× the
collector throughput, ~5× less memory, no `memory_limit`/`max_execution_time`
surprises, the same binary for every cPanel OS, and one language for the
server side.

## Layout

```
Cargo.toml                     single binary crate "webpulse", edition 2021, MSRV 1.75
src/main.rs                    clap: every subcommand's Args struct lives in the owning module
src/app.rs                     App { config, paths, log, registry (lazy), clock }  — shared
src/config.rs                  Config: defaults < /etc/webpulse/config.json < registry.settings — shared
src/paths.rs                   Paths (+ validation of user/domain names)           — shared
src/log.rs                     file logger with central sanitising                 — shared
src/error.rs                   WpError / Result                                     — shared
src/util/{fs,ip,lock,strs,time}.rs
src/engine/                    R1
   discovery.rs  cpanel users, userdomains, userdata kinds, log file candidates
   logsource.rs  checkpointed tailing, gz streaming with output caps
   parser.rs     combined log parser → Hit
   classify/{mod,ua,bots,referrer,path,geoip}.rs   (+ data/*.rs tables)
   session.rs    Sessionizer, Visit, open-visit cap
   counters.rs   in-memory accumulation + batched upserts
   collector.rs  per-user/per-domain runs, ownership checks, child processes
   user_settings.rs
   store/{schema,migrate,domain_store,dims}.rs
   registry.rs
   rollup.rs
src/cli/                       R1 owns collect, backfill, rollup, status, accounts, domains,
                               config_cmd, setup, geoip_update, purge, hooked, dev (gen-logs);
                               R2 owns report, serve, cgi
src/reports/                   R2: range, dimension, pretty (+ data/countries.rs), overview,
                               report, realtime, visits_log, health, export
src/http/                      R2: request, response, principal, resolver, api, admin_api,
                               user_settings_api, job_runner, page, dev_server
ui/                            unchanged (embedded)
cpanel/webpulse/*.live.php     R3: thin shims → exec binary
whm/cgi/*.cgi                  R3: bash shims → exec binary
install.sh, installer/lib.sh   R3: download release binary per arch, fallback cargo build
.github/workflows/{ci,release}.yml   R3
tests/                         Rust integration tests (tests/*.rs) + tests/parity/ + legacy pkg tests
```

## Compatibility contract

* **CLI**: same commands, flags, exit codes, text and `--json` output shapes as
  the PHP CLI documented in `wiki/Command-Line.md` and `bin/webpulse --help`
  (0.1.0). The installer, cron file, hooks and the packaging tests depend on it.
  The PHP launcher `bin/webpulse` is replaced by the binary at the same path.
* **Databases**: byte-for-byte the schemas in `docs/DATA-MODEL.md` (0.1.0 plus
  the `imported_archives.last_ts` column and the meta keys the PHP engine
  added: `next_visit_id`, `live_since`, `backfill_until`). A Rust binary must
  open and continue a database written by 0.1.0 without migration, and vice
  versa, so operators can roll back.
* **Metrics**: `docs/METRICS.md` is normative; the parity harness enforces it.
* **API**: `docs/API.md` including the "Fields beyond the examples" section;
  the UI is not modified, so every field it reads must be present.
* **Config**: same `config.json` keys. `php_bin` becomes ignored (kept for
  compatibility, warned about once).

## New entry point: `webpulse cgi`

```
webpulse cgi (--principal user:NAME | --whm) (--api | --page) [--embedded] [--api-url URL]
```
Reads the CGI environment (`REQUEST_METHOD`, `QUERY_STRING`, `CONTENT_TYPE`,
`CONTENT_LENGTH`, `HTTP_X_WEBPULSE_REQUEST`, `REMOTE_ADDR`, `REMOTE_USER`) and
the body from stdin, prints `Status: NNN`, headers, blank line, body.
* `--principal user:NAME` is for the cPanel shims; the binary runs as that
  account (inherited uid) and the OS permissions are the real boundary. The
  binary additionally refuses `--principal user:X` when the effective uid is
  neither root nor X's uid.
* `--whm` derives the principal from `REMOTE_USER`: `root` → admin, otherwise
  reseller with the accounts it owns (403 when it owns none). Refuses to run
  with `--whm` when euid ≠ 0.
* `--page` renders the dashboard HTML (`--embedded` → fragment for the cPanel
  chrome), `--api` dispatches the JSON API.

cPanel shim (`api.live.php`, sketch):
```php
require_once '/usr/local/cpanel/php/cpanel.php';
$cpanel = new CPANEL(); $user = LiveApiUser::resolve($cpanel); $cpanel->end();
passthru_cgi('/usr/local/webpulse/bin/webpulse', ['cgi', '--principal', "user:$user", '--api']);
```
`passthru_cgi` runs the binary with `proc_open`, forwards the CGI variables and
the request body, and copies stdout to the client, translating the `Status:`
line to `header()` calls. WHM shim (`index.cgi`):
```bash
#!/bin/bash
#WHMADDON:webpulse:WebPulse Analytics:webpulse.png
#ACLS:all
exec /usr/local/webpulse/bin/webpulse cgi --whm --page --api-url api.cgi
```

## Parity harness — `tests/parity/run.sh`

Runs only when `php` ≥ 8.1 is available. It checks out tag `v0.1.0` into
`tests/tmp/legacy` (git worktree), generates identical logs with the **Rust**
generator (`webpulse dev gen-logs`, a port of `tests/gen_logs.php` producing
byte-identical output for the same seed), runs `collect`, `backfill` and
`rollup` with both implementations against two copies of the same fake root
with the same `WEBPULSE_NOW`, then compares:
1. `daily`, `hourly`, `dim_daily` dumped as sorted CSV — must be identical;
2. counts and sums of `visits` and `events`; `open_visits` count;
3. the JSON of every API action (`domains`, `overview`, `report` for every
   dimension, `realtime`, `visits_log`, `health`, `admin.overview`,
   `admin.accounts`) with `meta.took_ms`/`generated_at` removed — must be
   identical after `jq -S`.
Intentional differences are listed at the end of this file (none expected).

## Releases and installation

* `.github/workflows/release.yml` on `v*` tags: `cargo-zigbuild` for
  `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, `strip`,
  `webpulse-<ver>-<arch>-linux-musl.tar.gz` (+ `.sha256`) uploaded to the
  GitHub release together with the source tarball from `make dist`.
* `install.sh` detects `uname -m`, downloads the binary for the requested
  release (`WEBPULSE_REF` tag; `main` means latest release), verifies the
  checksum, installs it as `/usr/local/webpulse/bin/webpulse` (0755 root).
  When no binary exists for the ref/arch it builds from source if `cargo` is
  present (`--build`), otherwise fails with a clear message. `--binary <file>`
  installs a local binary (air-gapped servers).
* Nothing on the server needs PHP any more except the cPanel shims, which use
  cPanel's own PHP exactly as before.

## Work packages

* **R1 — engine + CLI** (`src/engine/**`, `src/util/**`, `src/cli/{collect,
  backfill,rollup,status,accounts,domains,config_cmd,setup,geoip_update,purge,
  hooked,dev}.rs`, `tests/engine_*.rs`, `tests/common/mod.rs` (FakeRoot port),
  `tests/parity/db.sh`). Also owns the shared files after the scaffold
  (`app.rs`, `config.rs`, `paths.rs`, `log.rs`, `error.rs`, `main.rs`): R2 asks
  R1 (via the orchestrator) for additions rather than editing them.
* **R2 — reports + HTTP** (`src/reports/**`, `src/http/**`,
  `src/cli/{report,serve,cgi}.rs`, `tests/http_*.rs`, `tests/parity/api.sh`).
  Reads databases through `engine::store::DomainStore::open_read_only` and the
  registry through `engine::registry::Registry` — until R1 lands those, R2
  tests seed databases from the DDL in `docs/DATA-MODEL.md` directly.
* **R3 — packaging** (`install.sh`, `installer/**`, `cpanel/**`, `whm/**`,
  `hooks/**`, `.github/**`, `Makefile`, `scripts/**`, `tests/cases/pkg_*.php`,
  `tests/run.php`, docs and wiki pages touched by the change, `CHANGELOG.md`).

## Removal of the PHP implementation

When the parity harness passes: delete `lib/`, `bin/webpulse` (PHP),
`tests/cases/{engine,ui,sec}_*.php`, `tests/support/*` except what the pkg
tests need, `tests/gen_logs.php`; keep `tests/run.php` + `tests/cases/pkg_*.php`
(they test shell scripts and shims and need PHP only on the dev machine/CI).
Version 0.2.0.

## Intentional differences from 0.1.0

(none yet)
