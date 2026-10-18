# JSON API

One entry point in the binary, `http::api::dispatch(&Ctx, &Request, &Principal) -> Response`
(`src/http/api.rs`), is reached through three thin front-ends:

| Front-end                                  | URL (relative to the page)      | Principal                 |
|--------------------------------------------|---------------------------------|---------------------------|
| cPanel plugin `cpanel/webpulse/api.live.php` (PHP shim → `webpulse cgi --principal user:U --api`) | `api.live.php?a=…` | `user` (from LiveAPI) |
| WHM plugin `whm/cgi/api.cgi` (bash → `webpulse cgi --whm --api`) | `api.cgi?a=…`        | `admin` (root) or `reseller` |
| Dev server `webpulse serve`                | `/api?a=…`                      | from `--user` / `--admin` |
| Cockpit module (`ui/cockpit.js`: `cockpit.spawn` → `webpulse cgi --cockpit --api`, query string in `QUERY_STRING`) | `?a=…` | effective uid: root → `admin`, else `user` (the sites it owns) |

All responses are `application/json; charset=utf-8`, shape
`{"ok":true,"data":…,"meta":{"took_ms":12,"generated_at":"…"}}` or
`{"ok":false,"error":{"code":"not_found","message":"…"}}` with HTTP 400/403/404/
500. Dates are `YYYY-MM-DD` in the domain's timezone; timestamps are ISO-8601
with offset. Numbers are numbers, never strings.

Mutating actions require `POST` with `X-WebPulse-Request: 1` and a JSON body.

## Common query parameters

| Param    | Meaning |
|----------|---------|
| `domain` | required for domain reports; must be visible to the principal |
| `from`, `to` | inclusive local dates. Shortcut `range`: `today`, `yesterday`, `7d`, `30d`, `90d`, `12m`, `month` (this month), `lastmonth`, `year`, `all`. Default `30d`. |
| `compare` | `1` → also return the immediately preceding period of the same length (`prev` keys) |
| `limit`, `offset` | for table reports (default 10 / 0, max 500) |
| `sort`   | column name, prefix `-` for descending (default `-visits` or `-pageviews` depending on the dim) |
| `q`      | substring filter on `key` (case-insensitive) |

## Actions (domain scope)

### `a=domains`
Domains visible to the principal with a tiny summary.
```json
{"domains":[{"domain":"example.com","user":"alice","kind":"main","enabled":true,
  "has_data":true,"first_day":"2026-09-01","last_day":"2026-10-16",
  "last_collect_at":"2026-10-16T10:05:02+02:00","visits_today":42,"pageviews_today":97,
  "timezone":"Europe/Rome","db_bytes":1843200}]}
```

### `a=overview`
Everything the top of the dashboard needs in one round-trip.
```json
{"range":{"from":"2026-09-17","to":"2026-10-16","days":30,"granularity":"day",
          "prev":{"from":"2026-08-18","to":"2026-09-16"}},
 "kpi":{
   "visits":   {"value":1234,"prev":1100,"change":0.1218},
   "visitors": {"value":980,"prev":900,"change":0.0889},
   "pageviews":{"value":4321,"prev":4000,"change":0.08},
   "pages_per_visit":{"value":3.5,"prev":3.6,"change":-0.0278},
   "avg_duration":{"value":145,"prev":150,"change":-0.0333},
   "bounce_rate":{"value":0.42,"prev":0.45,"change":-0.0667},
   "bytes":    {"value":123456789,"prev":100000000,"change":0.2346},
   "bot_share":{"value":0.31,"prev":0.28,"change":0.1071},
   "downloads":{"value":12,"prev":9,"change":0.3333},
   "errors_4xx":{"value":87,"prev":60,"change":0.45},
   "errors_5xx":{"value":2,"prev":0,"change":null}
 },
 "series":{"labels":["2026-09-17","…"],
   "visits":[…],"visitors":[…],"pageviews":[…],"bytes":[…],"bot_hits":[…],
   "bounces":[…],"duration":[…],"errors_4xx":[…],"errors_5xx":[…],
   "prev":{"visits":[…],"pageviews":[…]}},
 "hours":{"visits":[24 ints],"pageviews":[24 ints]},
 "weekdays":{"visits":[7 ints, Monday first],"pageviews":[7 ints]},
 "heatmap":[[24 ints] × 7],
 "meta":{"domain":"example.com","timezone":"Europe/Rome","first_day":"…","last_day":"…",
         "last_collect_at":"…","tz_changed_at":null,"raw_retention_days":30}}
```
`granularity` is `hour` when the range is ≤ 2 days (series built from
`hourly`), `day` up to 120 days, `week` up to 400, else `month`.
`change` is `(value - prev) / prev`, `null` when prev is 0.

### `a=report&name=<dim>`
Table for one dimension (`dim` must be in `Dimension::ALL`; `visits_log` and
`realtime` are separate actions).
```json
{"name":"page","total":{"visits":1234,"pageviews":4321,"hits":5000,"bytes":1,"rows":87},
 "rows":[{"key":"/","visits":600,"pageviews":900,"hits":950,"bytes":12345,
          "bounces":0,"duration":0,
          "share":0.208,            // pageviews / total pageviews (or visits for visit-level dims)
          "exit_rate":0.21,         // page dim only: exits/pageviews via exit_page join
          "bounce_rate":0.4,        // entry_page & visit-level dims: bounces/visits
          "avg_duration":120,       // visit-level dims: duration/visits
          "prev":{"visits":550,"pageviews":800}}],   // when compare=1
 "has_more":true}
```
Keys the UI must prettify: country codes (flag emoji + name from a built-in
table), `status` (`404 Not Found`), `channel`, `device`, bucket names.

### `a=realtime`
Last 30 minutes, built from `open_visits` + `events`.
```json
{"active_visits":7,"pageviews_30m":45,"per_minute":[30 ints],
 "recent":[{"ts":"…","path":"/pricing","status":200,"type":"pageview",
            "country":"it","device":"mobile","browser":"Safari 17","referrer":"google.com","visit_pageviews":3}]}
```

### `a=visits_log`
Matomo-style visitor log; paginated by `before` (timestamp cursor), `limit` ≤ 100.
```json
{"visits":[{"visit_id":991,"start":"…","end":"…","duration":240,"pageviews":4,"bytes":1,
  "entry_page":"/","exit_page":"/contact","channel":"organic","referrer_host":"google.com",
  "search_engine":"google","keyword":null,"campaign":null,"country":"it","city":null,
  "browser":"Chrome 129","os":"Windows 11","device":"desktop",
  "actions":[{"ts":"…","type":"pageview","path":"/","status":200,"bytes":1234}]}],
 "next_before":1760600000}
```

### `a=health`
Technical panel data: `status` breakdown, `errors` series, top `not_found`
with referrers, `server_error` pages, bandwidth by `file_type`, top
`static_file`, `download`, `protocol`, `http_version`, `method`, peak hour.
```json
{"status":[{"key":"200","hits":1},…],"errors_series":{"labels":[…],"errors_4xx":[…],"errors_5xx":[…]},
 "not_found":[{"key":"/old","hits":9,"referrers":[{"key":"bing.com","hits":4}]}],
 "server_error":[…],"file_type":[…],"static_file":[…],"download":[…],
 "protocol":[…],"http_version":[…],"method":[…],
 "peak":{"day":"2026-10-03","hour":14,"hits":1234}}
```

### `a=export&name=<dim>&format=csv`
Same rows as `report` without limit (max 10 000 rows), `text/csv`,
`Content-Disposition: attachment; filename="example.com-page-2026-09-17_2026-10-16.csv"`.

### `a=user_settings` (GET) / `a=save_user_settings` (POST, cPanel principal only)
Read/write `/var/webpulse/data/<user>/user/settings.json` (see DATA-MODEL.md).
Only `user` principals may save (the dev server's `--user` principal included,
so the Settings tab is testable); admin and reseller get 403, an empty body 400.

### `a=whoami`
Every principal. Cheap: no database is opened. The Cockpit module asks it before it
starts the dashboard (its page is generated once and holds no principal).
```json
{"principal": {"type": "user", "name": "alice"}, "version": "0.6.0", "mode": "standalone"}
```
`mode` is the discovery mode (`cpanel` or `standalone`). Admin and reseller
principals also get `hostname`; `geoip_credit` (`{text, url}`) is present while
a DB-IP database is in use. (`a=ping` remains: `{version, principal}`.)

## Actions (admin scope — `admin` and `reseller` principals)

* `a=admin.overview` – totals across visible accounts for today/7d/30d,
  top domains by visits_30d (`top`, 1–500, default 10; the server report asks for 500), bot share, disk usage of data dir, last jobs,
  collector health (`last_collect_at`, `lag_seconds` = now − newest
  `last_line_ts`, `cron_installed`, `hook_installed`, `geoip`:{enabled, db_date}).
* `a=admin.accounts` – list of accounts with `user, owner, enabled (effective
  + override), domains:[{domain, kind, enabled, has_data, db_bytes, visits_30d,
  last_collect_at}]`, `q` filter and paging.
* `a=admin.set_account` (POST `{user, enabled: true|false|null, timezone?}`)
* `a=admin.set_domain` (POST `{user, domain, enabled: true|false|null}`) –
  `domain` must belong to `user` (live registry row, cPanel user files /
  `/etc/userdomains`, or an existing database), else 404.
* `a=admin.settings` (GET, root only) / `a=admin.save_settings` (POST, root only) –
  `default_enabled, raw_retention_days, max_rows_per_dim, session_timeout_min,
  store_ip, exclude_ips[], exclude_paths[], geoip.enabled, timezone`. `admin.save_settings` also
  accepts `geoip.url` (an `https://` URL ending in `.mmdb` or `.mmdb.gz`, `{YYYY}`/`{MM}` allowed;
  empty or null = the built-in DB-IP URL) and `geoip.city_db` (an existing regular `.mmdb` file in
  the directory of `geoip.country_db`; empty or null = none); `admin.settings` does not list them
  (its shape is the one of 0.1.0) — `admin.geoip` reports them. Invalid values: 400.
* `a=admin.run` (POST `{type: "collect"|"backfill"|"rollup", user?, domain?}`)
  – spawns `bin/webpulse <type> … --job` detached (`nohup … &`), returns
  `{job_id}` only; the UI polls `admin.jobs`. `domain` must be a domain of
  `user` (404 otherwise). 409 `conflict` when a job of the same type and scope
  is running, or when 3 jobs are running (rows "running" for more than 6 hours
  are presumed dead and ignored).
* `a=admin.run` (POST `{type: "upgrade"}`, root only) – one-click upgrade to the release found
  by the last update check: spawns `bin/webpulse dev run-upgrade --job <id> --tag <tag>` detached
  like the other jobs, which runs `bash <install_root>/install.sh --update --ref <tag> -y` (plus
  `--repo <update.repo>` when it is not the default) with a clean environment, appends its output
  to `<log_dir>/upgrade.log` and finishes the job `ok` ("Upgraded to <tag>") or `error` ("install.sh
  exited with status N; …") from its exit status (killed after 30 minutes). The job row has type
  `upgrade` and scope `<tag>`. Returns `{job_id, tag}`. 400 when no newer release is known or the
  stored tag is not `^v?\d+\.\d+\.\d+(?:[-.][0-9A-Za-z.]+)?$`; 409 `conflict` when an upgrade job
  is running (started less than an hour ago) or when the binary does not run from an installation
  (`<root>/bin/webpulse` with a root-owned, not group/other-writable `<root>/install.sh`); 403 for
  resellers.
* `a=admin.run` (POST `{type: "geoip-update"}`, root only) – spawns `bin/webpulse geoip-update
  --force --job <id>` detached: downloads `geoip.url` for the current month (else the previous one),
  validates it and replaces `geoip.country_db`. Works with `geoip.enabled` false (download only).
  The job row has type `geoip-update` and scope `all`; it finishes `ok` ("installed
  DBIP-Country-Lite built 2026-10-01 (N bytes)") or `error` (the download / validation message).
  Returns `{job_id, enabled}`; 409 `conflict` while one is running; 403 for resellers.
* `a=admin.geoip` (GET; admin and reseller) – status for the WHM GeoIP card:
  `{enabled, dir (directory of country_db), country: DB, city: DB|null (null when geoip.city_db is
  unset), url (template in effect), url_preview ({YYYY}/{MM} = current UTC month), url_default,
  url_overridden, cron: {file, present, scheduled (a non-comment line runs geoip-update), line},
  last_job (newest job of type geoip-update, with started_at_iso / finished_at_iso, or null),
  known_country_30d: {share (0–1 or null), visits, known, domains, partial}, attribution
  ("IP Geolocation by DB-IP" when enabled and a DB-IP database is installed, else null),
  provider: {name, url, license, attribution, attribution_url}, can_edit (root)}` where DB =
  `{path, present, bytes, mtime (ISO), metadata: {database_type, build_epoch, build_date, ip_version,
  node_count, record_size, languages, description, dbip} | null, error}` (the metadata is read from
  the end of the file with the maxminddb crate, without loading the database).
  `known_country_30d` sums the `country` dimension (`dim_daily`, last 30 days) of the 25 busiest
  domains in scope (`partial` when there are more, or one could not be read): the share of visits
  whose country is not `(unknown)`.
* `a=admin.jobs` – last 50 rows of `jobs`.
* `a=admin.update` (GET; admin and reseller) – the last release check, re-evaluated against the
  running version: `{current, latest, tag, is_newer, html_url, published_at, notes (plain text,
  ≤ 2 KiB), checked_at (unix s or null), error, checked_at_iso, check_enabled (update.check),
  install_sh (root only: an installer is present), can_upgrade (root && is_newer && install_sh),
  upgrade_job (root only: the newest job of type upgrade, or null)}`. The check itself runs on the
  server (`webpulse update-check`, the nightly rollup); the browser never contacts GitHub.
* `a=admin.update_check` (POST, root only) – asks GitHub again unless the last check is less than
  five minutes old; returns the `admin.update` object plus `throttled` (true = cached result returned).
* `a=admin.purge` (POST `{user, domain?, confirm: "DELETE"}`, root only) –
  deletes the domain DB(s).
* Domain reports (`overview`, `report`, …) work for admins with any
  `domain` (plus `user` param when the same domain exists under two users).

## Principal

```rust
// src/http/principal.rs
pub enum Kind { User, Admin, Reseller }
pub struct Principal {
    pub kind: Kind,
    pub name: String,                      // account / reseller name, "root" for the admin
    pub owned_users: Option<Vec<String>>,  // reseller's accounts (None = look them up in the registry)
}
impl Principal {
    pub fn is_admin(&self) -> bool;
    pub fn can_see_user(&self, user: &str) -> bool;
}
```
Mutations are allowed to the admin, to a reseller within its accounts and to an
account for its own settings only.
`http::resolver::visible_domains(&Principal, Option<&Registry>, &Paths) -> Vec<DomainRef>`
is the only place that decides visibility: for `user` it lists
`<data_dir>/<user>/*.db` (plus Registry rows without data for the "no data
yet" state); for `reseller` all accounts with `owner = name`; for `admin` all.

## Fields beyond the examples above

The implementation returns a few extra fields that the UI relies on:
`overview.series.hits`; per-hour series that `hourly` cannot provide are `null`
at `hour` granularity; `report` rows carry `label` (pretty name) and, for
`page`, `exits`, for `bot`, `last_day`; with `series=1` a `series` object
`{labels, granularity, metric, keys, key_labels, values}`; `visits_log` adds
`next_before_id` (cursor tie-breaker), `live` (open visits included) and
`retention_days`; `realtime` adds `last_event_at` and `last_collect_at`;
`health` adds `peak_hour` and `errors_series.hits/bot_hits`. The page bootstrap
(`window.WEBPULSE`) carries `geoip_credit: {text, url}` while a DB-IP database is installed and
GeoIP is on (always checked when the registry is not readable, as in cPanel): the dashboard footer
shows the attribution its CC BY 4.0 licence requires. Admin
`bot_share_30d` is computed from each domain's `daily` table (flag `partial`
above 300 domains).
