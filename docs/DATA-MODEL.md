# Data model

All databases are SQLite 3 (`journal_mode=DELETE`, `synchronous=NORMAL`,
`foreign_keys=ON`, `busy_timeout=5000`). Schema migrations are numbered in
`meta.schema_version` and applied by `engine::store::migrate` on open (writer
side only; the read-only dashboard refuses to open a DB with a newer schema
than it knows and shows a friendly message).

## Domain database — `/var/webpulse/data/<user>/<domain>.db`

```sql
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT
);
-- keys: schema_version, user, domain, created_at, timezone, tz_changed_at,
--       first_day, last_day, last_collect_at, last_rollup_at, total_lines

CREATE TABLE daily (
  day        TEXT PRIMARY KEY,      -- 'YYYY-MM-DD' local
  visits     INTEGER NOT NULL DEFAULT 0,
  visitors   INTEGER NOT NULL DEFAULT 0,
  pageviews  INTEGER NOT NULL DEFAULT 0,
  hits       INTEGER NOT NULL DEFAULT 0,
  bytes      INTEGER NOT NULL DEFAULT 0,
  bounces    INTEGER NOT NULL DEFAULT 0,
  duration   INTEGER NOT NULL DEFAULT 0,  -- seconds, sum over visits
  downloads  INTEGER NOT NULL DEFAULT 0,
  errors_4xx INTEGER NOT NULL DEFAULT 0,
  errors_5xx INTEGER NOT NULL DEFAULT 0,
  bot_hits   INTEGER NOT NULL DEFAULT 0,
  bot_bytes  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE hourly (
  day       TEXT NOT NULL,
  hour      INTEGER NOT NULL,          -- 0..23 local
  visits    INTEGER NOT NULL DEFAULT 0,
  pageviews INTEGER NOT NULL DEFAULT 0,
  hits      INTEGER NOT NULL DEFAULT 0,
  bytes     INTEGER NOT NULL DEFAULT 0,
  bot_hits  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, hour)
);

-- One generic table for every dimension; `dim` is one of the whitelisted names
-- in METRICS.md / Dimension::ALL. Columns that do not apply to a dim stay 0.
CREATE TABLE dim_daily (
  day       TEXT NOT NULL,
  dim       TEXT NOT NULL,
  key       TEXT NOT NULL,             -- e.g. '/pricing', 'google', 'it', 'Chrome 129'
  visits    INTEGER NOT NULL DEFAULT 0, -- visits in which the key occurred (page) / visits attributed (channel, country…)
  pageviews INTEGER NOT NULL DEFAULT 0,
  hits      INTEGER NOT NULL DEFAULT 0,
  bytes     INTEGER NOT NULL DEFAULT 0,
  bounces   INTEGER NOT NULL DEFAULT 0, -- entry_page, channel, country, device, browser, os, referrer_host, search_engine, campaign
  duration  INTEGER NOT NULL DEFAULT 0, -- same dims as bounces (sum of visit durations, written at close)
  PRIMARY KEY (day, dim, key)
) WITHOUT ROWID;
CREATE INDEX dim_daily_dim_day ON dim_daily (dim, day);

-- Open (not yet timed-out) visits; survive between collector runs.
CREATE TABLE open_visits (
  visitor     TEXT PRIMARY KEY,         -- visitor hash (one open visit per visitor)
  visit_id    INTEGER NOT NULL,
  day         TEXT NOT NULL,            -- attribution day
  start_ts    INTEGER NOT NULL,
  last_ts     INTEGER NOT NULL,         -- last hit of any class
  last_pv_ts  INTEGER NOT NULL,         -- last pageview (for duration)
  pageviews   INTEGER NOT NULL,
  hits        INTEGER NOT NULL,
  bytes       INTEGER NOT NULL,
  entry_page  TEXT,
  exit_page   TEXT,
  pages_seen  TEXT NOT NULL,            -- JSON array of distinct paths, capped at 200
  attrs       TEXT NOT NULL             -- JSON: channel, referrer_host, referrer_url, search_engine, keyword,
                                        --       campaign, utm_source, utm_medium, country, city, browser,
                                        --       browser_version, os, os_version, device, protocol, ip (if store_ip)
);
CREATE INDEX open_visits_last ON open_visits (last_ts);

-- Closed visits (Matomo "visits log"), kept raw_retention_days.
CREATE TABLE visits (
  visit_id    INTEGER PRIMARY KEY,
  visitor     TEXT NOT NULL,
  day         TEXT NOT NULL,
  start_ts    INTEGER NOT NULL,
  end_ts      INTEGER NOT NULL,
  duration    INTEGER NOT NULL,
  pageviews   INTEGER NOT NULL,
  hits        INTEGER NOT NULL,
  bytes       INTEGER NOT NULL,
  entry_page  TEXT,
  exit_page   TEXT,
  channel     TEXT,
  referrer_host TEXT,
  search_engine TEXT,
  keyword     TEXT,
  campaign    TEXT,
  country     TEXT,
  city        TEXT,
  browser     TEXT,
  browser_version TEXT,
  os          TEXT,
  os_version  TEXT,
  device      TEXT,
  ip          TEXT                      -- NULL unless store_ip
);
CREATE INDEX visits_day ON visits (day, start_ts);

-- Raw events (pageviews, downloads, errors) for the visits log and the
-- real-time panel; kept raw_retention_days.
CREATE TABLE events (
  id        INTEGER PRIMARY KEY,
  ts        INTEGER NOT NULL,
  visit_id  INTEGER,                    -- NULL for bots
  type      TEXT NOT NULL,              -- pageview | download | error | redirect | bot
  path      TEXT NOT NULL,
  status    INTEGER NOT NULL,
  bytes     INTEGER NOT NULL,
  referrer  TEXT                        -- external referrer host for the hit, if any
);
CREATE INDEX events_ts ON events (ts);
CREATE INDEX events_visit ON events (visit_id);

-- Unique-visitor bookkeeping, pruned after 2 days.
CREATE TABLE visitor_day (
  day     TEXT NOT NULL,
  visitor TEXT NOT NULL,
  PRIMARY KEY (day, visitor)
) WITHOUT ROWID;

-- Tail checkpoints, one per physical log file.
CREATE TABLE checkpoints (
  path          TEXT PRIMARY KEY,       -- absolute path as discovered
  inode         INTEGER NOT NULL,
  offset        INTEGER NOT NULL,
  size          INTEGER NOT NULL,
  first_line    TEXT,                   -- hash of the first 256 bytes, extra rotation guard
  last_ts       INTEGER,                -- timestamp of last processed line
  lines         INTEGER NOT NULL DEFAULT 0,
  bad_lines     INTEGER NOT NULL DEFAULT 0,
  ignored_lines INTEGER NOT NULL DEFAULT 0,
  updated_at    INTEGER NOT NULL
);

-- Backfilled archives, so a .gz is never imported twice.
CREATE TABLE imported_archives (
  path      TEXT PRIMARY KEY,
  size      INTEGER NOT NULL,
  mtime     INTEGER NOT NULL,
  lines     INTEGER NOT NULL,
  imported_at INTEGER NOT NULL
);

CREATE TABLE sqlite_sequence_dummy (x);  -- (not used; visit_id comes from meta.next_visit_id)
```

Notes
* `visit_id` is allocated from `meta.next_visit_id` (incremented in the same
  transaction) so open and closed visits share an id space.
* Keys in `dim_daily` are limited to 255 bytes; longer keys are truncated with
  `…`. Dimension names (`dim`) are the exact list in `Dimension::ALL`:
  `page entry_page exit_page referrer_host referrer_url channel search_engine
  keyword campaign utm_source utm_medium source_medium country city browser
  browser_version os os_version device status method protocol http_version
  file_type static_file download not_found not_found_referrer server_error bot
  visit_duration pages_per_visit`.
* The engine must never write to the same `(day, dim, key)` row twice in one
  flush; counters are accumulated in memory per run and upserted once.

## Registry database — `/var/webpulse/webpulse.db`

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);  -- schema_version, installed_at, version

CREATE TABLE settings (                 -- overrides of config.json, edited from WHM
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL                   -- JSON encoded
);

CREATE TABLE accounts (
  user        TEXT PRIMARY KEY,
  owner       TEXT,                     -- reseller (OWNER= in /var/cpanel/users/<user>)
  enabled     INTEGER,                  -- NULL = follow default_enabled, 1 = on, 0 = off
  timezone    TEXT,                     -- NULL = server
  first_seen  INTEGER NOT NULL,
  last_seen   INTEGER NOT NULL,         -- last time discovery saw this account
  removed_at  INTEGER                   -- account no longer exists; data kept until purge
);

CREATE TABLE domains (
  user        TEXT NOT NULL,
  domain      TEXT NOT NULL,
  enabled     INTEGER,                  -- NULL = follow account
  kind        TEXT,                     -- main | addon | parked | sub | unknown
  first_seen  INTEGER NOT NULL,
  last_seen   INTEGER NOT NULL,
  removed_at  INTEGER,
  PRIMARY KEY (user, domain)
);

CREATE TABLE domain_summary (           -- refreshed after every collect run, for the WHM overview
  user        TEXT NOT NULL,
  domain      TEXT NOT NULL,
  db_bytes    INTEGER NOT NULL DEFAULT 0,
  last_collect_at INTEGER,
  last_line_ts INTEGER,
  lines_total INTEGER NOT NULL DEFAULT 0,
  visits_today INTEGER NOT NULL DEFAULT 0,
  pageviews_today INTEGER NOT NULL DEFAULT 0,
  visits_7d   INTEGER NOT NULL DEFAULT 0,
  pageviews_7d INTEGER NOT NULL DEFAULT 0,
  bytes_7d    INTEGER NOT NULL DEFAULT 0,
  visits_30d  INTEGER NOT NULL DEFAULT 0,
  pageviews_30d INTEGER NOT NULL DEFAULT 0,
  bytes_30d   INTEGER NOT NULL DEFAULT 0,
  bot_hits_30d INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (user, domain)
);

CREATE TABLE salts (
  day   TEXT PRIMARY KEY,
  salt  BLOB NOT NULL
);

CREATE TABLE jobs (                     -- collector / rollup / backfill history (last 500 kept)
  id          INTEGER PRIMARY KEY,
  type        TEXT NOT NULL,            -- collect | backfill | rollup | hook
  scope       TEXT,                     -- 'all' or user[/domain]
  started_at  INTEGER NOT NULL,
  finished_at INTEGER,
  status      TEXT NOT NULL,            -- running | ok | error | skipped
  lines       INTEGER NOT NULL DEFAULT 0,
  files       INTEGER NOT NULL DEFAULT 0,
  domains     INTEGER NOT NULL DEFAULT 0,
  message     TEXT
);
```

## User settings — `/var/webpulse/data/<user>/user/settings.json`

Written by the cPanel plugin (runs as the account), read by the collector.

```json
{ "version": 1,
  "domains": {
    "example.com": { "timezone": "Europe/Rome", "exclude_ips": ["203.0.113.7"], "exclude_paths": ["/staging/"] }
  },
  "ui": { "default_range": "30d", "dark": "auto" }
}
```
Validation on read: regular file, ≤ 64 KiB, valid JSON, ≤ 50 IPs/CIDRs and
≤ 50 path prefixes per domain, timezone must be in `DateTimeZone::listIdentifiers()`.
