# Metric definitions

These definitions are normative: the engine, the tests and the dashboard
tooltips must agree with them. They follow Matomo's log-analytics semantics
where a choice had to be made.

## Hit classification (per log line)

Order matters; the first rule that matches wins.

| Class        | Rule                                                                                                   | Counted in |
|--------------|--------------------------------------------------------------------------------------------------------|------------|
| `ignored`    | IP in `exclude_ips` (server, or per-domain user settings), or path matches `exclude_paths`, or method is `HEAD`/`OPTIONS` and `count_head_requests` is false, or line unparseable | `checkpoints.ignored_lines` only |
| `bot`        | user agent matches the bot list (`bot`, `crawl`, `spider`, `slurp`, `curl`, `wget`, `python-requests`, `Go-http-client`, `HeadlessChrome`, `Lighthouse`, `GTmetrix`, `UptimeRobot`, `facebookexternalhit`, `Googlebot`, `bingbot`, `AhrefsBot`, `SemrushBot`, `YandexBot`, `Baiduspider`, `DuckDuckBot`, `PetalBot`, `Applebot`, `Bytespider`, `GPTBot`, `ClaudeBot`, `CCBot`, `Amazonbot`, …) or is empty/`-`          | `daily.bot_hits/bot_bytes`, `hourly.bot_hits`, dim `bot` (name) |
| `error`      | status ≥ 400                                                                                           | hits/bytes; `daily.errors_4xx` or `errors_5xx`; dim `status`; 404 → dim `not_found` (path) and `not_found_referrer` (`path ← referrer_host`); 5xx → dim `server_error` (path) |
| `redirect`   | status 301/302/303/307/308                                                                             | hits/bytes; dim `status`; belongs to the visit but is not a pageview |
| `download`   | status 200/206 and extension in download list (`pdf zip rar 7z gz tgz bz2 xz dmg exe msi apk deb rpm iso doc docx xls xlsx ppt pptx odt ods csv mp3 mp4 m4a mov avi mkv wav flac epub mobi`) | hits/bytes; `daily.downloads`; dim `download` (path); counts toward the visit's hits, not pageviews |
| `static`     | extension in static list (`css js mjs map png jpg jpeg gif webp avif svg ico bmp tif tiff woff woff2 ttf otf eot json xml txt webmanifest`) or path starts with one of the `static_prefixes` (default `/wp-content/uploads/`, `/wp-includes/`, `/wp-json/`, `/wp-admin/admin-ajax.php`, `/wp-cron.php`, `/xmlrpc.php`) | hits/bytes; dim `file_type`; dim `static_file` (top assets by bandwidth) |
| `pageview`   | method GET or POST, status 200, 203, 206 or 304, everything else                                      | pageviews + hits/bytes; all page dims |
| `other`      | anything left (e.g. 1xx, 2xx with unknown method)                                                      | hits/bytes; dim `status`, `method` |

`file_type` is the lowercase extension (max 8 chars, `[a-z0-9]`), `html`/`htm`/
`php` → `html`, no extension → `html`. Every non-bot, non-ignored class also
feeds the dims `method`, `protocol` (`http`/`https` from the log file name
`-ssl_log` or scheme in the request line) and `http_version`.

**Path normalisation:** request path is URL-decoded once, truncated to 512
bytes, query string removed (UTM parameters are extracted first), trailing
`index.php`/`index.html` collapsed to the directory, repeated slashes
collapsed. Invalid UTF-8 is replaced with `?`. Paths are shown URL-encoded
again in the UI only where needed for links.

## Visitor and visit

* **Visitor hash** = first 16 hex chars of `sha256(salt(day) . ip . "\n" . ua)`.
  `salt(day)` is a random 32-byte value stored in `registry.salts` for the log
  line's local day; salts older than 2 days are deleted, which makes
  re-identification across days impossible (Plausible model). The raw IP is
  never written unless `store_ip` is true (then only in `visits.ip`).
* **Visit (session)** = sequence of non-bot hits of one visitor hash with gaps
  < `session_timeout_min` (30). A visit's **day** is the local day of its first
  hit; everything about the visit (bounce, duration, engagement buckets, all
  visit-level dimensions) is attributed to that day even if it crosses
  midnight. Visits are closed when the gap since their last hit exceeds the
  timeout *at the time the collector runs*; a visit still open is counted as a
  visit already (visit-level counters are incremented at the first hit).
  Safety cap: at most `max_open_visits` (50 000) visits of a domain are open at
  once; beyond it the visits idle the longest are closed early (normally, with
  all their counters), so a visitor returning to one of them starts a new
  visit. Only floods of distinct visitors within one timeout reach the cap.
* **Unique visitors** (`daily.visitors`) = number of distinct visitor hashes
  with at least one visit that day (`visitor_day` table, pruned after 2 days).
  Over a range the dashboard sums the daily values (same approximation Matomo
  makes for ranges); the tooltip says so.
* **Pageviews** = hits classified `pageview`. **Hits** = every non-ignored,
  non-bot line. **Bandwidth** (`bytes`) = sum of `%b` for the hits of that row
  (bots have their own `bot_bytes`).
* **Bounce** = visit with exactly one pageview (hits of other classes do not
  rescue a bounce). Implemented as: +1 bounce at the visit's first pageview,
  −1 at its second pageview.
* **Visit duration** = time between the first and the last pageview of the
  visit (0 for bounces, Matomo semantics). Stored as the running sum
  `daily.duration` so that the average = `duration / visits`.
* **Pages per visit** = `pageviews / visits`.
* **Entry page** = first pageview's path, written at visit start.
  **Exit page** = last pageview's path; the dim row is moved on every new
  pageview (−1 old, +1 new), so it is always right even for open visits.
  **Exit rate** of a page = exits / pageviews of that page (computed in the
  API). **Bounce rate of an entry page** = bounces / entries for that row.
* **Engagement buckets** (written when the visit is closed, on its start day):
  dim `visit_duration` with keys `0-10s`, `11-30s`, `31-60s`, `1-2m`, `2-5m`,
  `5-10m`, `10-30m`, `30m+`; dim `pages_per_visit` with keys `1`, `2`, `3`,
  `4`, `5`, `6-10`, `11-20`, `21+`.

## Acquisition

Evaluated on the **first hit of the visit** only (Matomo attribution:
first referrer of the visit wins).

* `channel` (dim, one of): `direct` (no referrer or referrer host equals the
  domain or one of its `www.` variants), `organic` (referrer host matches a
  search engine), `social` (referrer host matches a social network list:
  facebook, instagram, twitter/x, linkedin, pinterest, reddit, tiktok, youtube,
  threads, mastodon instances list, bsky, snapchat, whatsapp, telegram,
  t.co, lnkd.in, fb.me…), `paid` (utm_medium in `cpc ppc paid paidsearch
  display cpm banner` or `gclid`/`fbclid`/`msclkid` present in the query),
  `email` (utm_medium `email` or referrer host is a webmail: mail.google.com,
  outlook.live.com, mail.yahoo.com…), `campaign` (any other UTM-tagged visit),
  `referral` (anything else with an external referrer).
* `referrer_host` (external hosts only, `www.` stripped), `referrer_url`
  (host+path, no query, max 255 chars; only for referral/social).
* `search_engine` (google, bing, duckduckgo, yahoo, yandex, baidu, ecosia,
  qwant, startpage, brave, naver, seznam, ask, aol) and `keyword` (from the
  referrer query `q`, `query`, `p`, `text`, `wd`; most engines send none → key
  `(not provided)` is **not** stored, the engine row alone suffices).
* `campaign` = `utm_campaign`; `utm_source`, `utm_medium` as their own dims;
  `source_medium` = `utm_source / utm_medium` (Matomo/GA-style combined row).
* Internal referrers (same domain) are not stored.

## Audience

Evaluated on the first hit of the visit:

* `country` ISO-3166 alpha-2 lowercase, `(unknown)` when no GeoIP DB or lookup
  miss; `city` = `City, CC` only with a city database.
* `browser` (Chrome, Firefox, Safari, Edge, Opera, Samsung Internet, IE,
  Brave*, Vivaldi, UC Browser, Yandex Browser, In-app (Facebook/Instagram/
  WhatsApp/…), Other), `browser_version` = `Browser Major` (e.g. `Chrome 129`),
  `os` (Windows, macOS, iOS, Android, Linux, ChromeOS, Other) and `os_version`
  (`Windows 10/11`, `Android 14`, `iOS 17`, `macOS 14`…), `device`
  (desktop, mobile, tablet, other). *Brave is indistinguishable from Chrome
  in logs; it is only listed for completeness.*
* `language` is not available in combined logs and is therefore not shown.

## Time

* `daily.day` is the local date (domain timezone, else server timezone) of the
  hit. Changing the timezone later does not re-aggregate history; the UI tells
  the user when the timezone was changed (`meta.tz_changed_at`).
* `hourly` rows (day, hour 0–23) carry visits (by first hit), pageviews,
  hits, bytes, bot_hits. Weekday is derived from `day` at query time.

## Bots

Bots never create visits. Dim `bot` key = bot family name taken from the UA
(`Googlebot`, `bingbot`, `AhrefsBot`, `GPTBot`, …, `Other bot`), with hits and
bytes. `daily.bot_hits` lets the dashboard show the bot share over time.
