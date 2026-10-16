# Privacy and GDPR

WebPulse is built to collect as little personal data as possible. This page lists what is stored, what is
not, and how to delete it. It describes the software's behaviour; whether your use satisfies GDPR or
another law is for you and your legal adviser to decide, and this page is not legal advice.

## In short

- No JavaScript, cookie, local storage or fingerprinting on your customers' sites.
- IP addresses are **not stored** by default. They are used in memory to build a visitor hash.
- The hash includes a random salt that **changes every day** and is deleted after two days, so a visitor
  cannot be recognised from one day to the next.
- Individual visits are kept for **30 days** by default; only aggregated numbers are kept longer.
- The dashboards make no requests to third parties. GeoIP lookups use a local file.

## What is stored

Everything is in SQLite files under `/var/webpulse/`.

| Data | Where | How long | Personal data? |
|------|-------|----------|----------------|
| Daily and hourly totals (visits, pageviews, bytes, errors, bot hits) | `daily`, `hourly` | Forever | No |
| Report rows: page, referrer host, channel, country, browser, OS, device, status, UTM values, bot name | `dim_daily` | Forever | No (counts only) |
| Visits log: visit start and end, pages, entry and exit page, channel, referrer host, search engine, keyword, campaign, country, city, browser, OS, device | `visits` | `raw_retention_days` (30) | A pseudonymous visitor hash only |
| Raw events: time, path, status, size, referrer host | `events` | `raw_retention_days` (30) | Paths may identify a person if the site puts names or tokens in URL paths |
| Visitor hash of a visit | `visits`, open visits | With the visit | Pseudonymous; cannot be linked across days |
| Unique-visitor bookkeeping | `visitor_day` | 2 days | Pseudonymous |
| Daily salts | registry `salts` | 2 days | No |
| Log checkpoints (file, inode, offset, counters) | domain database | While the domain exists | No |
| Visitor IP address | `visits.ip` | `raw_retention_days` | **Yes, only if `store_ip` is on** |

## What is not stored

- The IP address (unless `store_ip` is on). It is hashed in memory and dropped.
- The user agent string. Only the derived browser, version, OS and device class are kept.
- The query string of a URL. It is removed from paths. UTM parameters are read first and kept as
  campaign values.
- Cookies or any identifier set by WebPulse. There is none.
- Language, screen size, or anything a script could collect.

## How the visitor hash works

For each hit, WebPulse computes `sha256(daily_salt + IP + user agent)` and keeps the first 16
hexadecimal characters. The salt is a random 32-byte value created for each local day. Salts older than
two days are deleted, and after that nobody, including the administrator, can recompute the hash of a
past day from an IP address. The same person visiting on two days is counted as two visitors. This
is why *Unique visitors* over a range are daily sums.

## Optional IP storage

The administrator can turn on **Store visitor IP addresses in the visits log** (`store_ip`, off by
default). Then the IP address is written to the visits log, and only there, and is deleted with the visit
after `raw_retention_days`. Switching it on makes the data personal data in the strict sense. Do it only
if you have a reason and have told the site owners. Switching it off again stops new storage; addresses
already stored disappear at the end of the retention period, or earlier with a purge.

## Retention

| Setting | Effect |
|---------|--------|
| `raw_retention_days` (1 to 3650, default 30) | Age at which the visits log and raw events are deleted by the nightly rollup. |
| Aggregates | Not deleted. They contain no personal data. |
| Data of a removed cPanel account | Kept until purged. |

Reduce the retention in **WHM » WebPulse Analytics » Server » Settings** or with
`webpulse config set raw_retention_days 7`.

## Exclusions

You can leave traffic out entirely:

- **Server-wide** (administrator): excluded IPs and CIDR ranges, excluded path prefixes. By default
  `/.well-known/`, `/cpanel`, `/whm`, `/webmail` and `/cgi-sys/` are excluded.
- **Per domain** (account owner, Settings tab): excluded IPs and path prefixes, for example your own
  office address or a staging path.
- **Per account or domain** (administrator): switch collection **Off**. See [[WHM Administration]].

Excluded hits are not counted; they appear only in an "ignored lines" counter.

## Deleting data

To delete the statistics of an account or one domain (as root):

```bash
webpulse purge --user <user> --yes
webpulse purge --user <user> --domain example.com --yes
```

This deletes the domain databases, including visits, events and checkpoints. It cannot be undone. If
the account no longer exists in cPanel, its data directory and registry rows are removed as well. To
make sure nothing is collected again for an account that stays on the server, disable it first:

```bash
webpulse accounts disable <user>
```

Because the checkpoints live in the domain database, a domain that is still enabled will be read again
from the beginning of its current log file at the next collection.

To remove everything WebPulse ever stored, uninstall with `--purge` ([[Updating and Uninstalling]]).

## What WebPulse does not control

- **The web server's own access logs.** They contain full IP addresses and user agents and are managed
  by cPanel (see WHM » Tweak Settings » Stats and Logs). WebPulse only reads them.
- **Archived logs** in `/home/<user>/logs/`, used by backfill.
- **Backups** of `/var/webpulse`.
- **Your privacy notice.** Telling visitors that you analyse server logs is up to the site owner.

## GeoIP

GeoIP is optional. The country is looked up locally in a downloaded database file; no IP address is sent
anywhere. The only network access is the download of the database file from DB-IP (`geoip-update`). Check
the licence terms of the DB-IP free database before you use it.

## See also

- [[Metrics and Definitions]]
- [[Configuration]]
- [[How It Works]]
- [[Command Line]]
