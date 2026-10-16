# Metrics and Definitions

WebPulse counts what is in the web server's access log. This page explains, without code, what the
numbers mean and why they may differ from other tools such as Google Analytics, Matomo or AWStats.
The normative text for developers is
[docs/METRICS.md](https://github.com/inalto/webpulse/blob/main/docs/METRICS.md).

## What a log line becomes

Each line of the access log is one **hit**. WebPulse sorts every hit into the first class that matches:

| Class | What it is | Counted as |
|-------|-----------|------------|
| Ignored | Excluded IP, excluded path, `HEAD` or `OPTIONS` request (unless `count_head_requests` is on), or a line that cannot be parsed | Not counted at all, only in an "ignored lines" counter |
| Bot | The user agent looks like a crawler, monitor or script, or is empty | Bot hits and bot bandwidth only. Never a visit. |
| Error | Status 400 or higher | Hits, bandwidth, 4xx or 5xx error counters; 404s are listed with their referrers |
| Redirect | Status 301, 302, 303, 307 or 308 | Hits and bandwidth. Part of the visit, not a pageview. |
| Download | Successful (200 or 206) request for a file such as `pdf`, `zip`, `docx`, `mp3`, `mp4`, `iso` | Hits, bandwidth, downloads. Part of the visit, not a pageview. |
| Static | Stylesheets, scripts, images, fonts, `json`, `xml`, `txt`, and paths such as `/wp-content/uploads/` | Hits, bandwidth, file type and heaviest-files reports |
| Pageview | A `GET` or `POST` answered with 200, 203, 206 or 304 that is none of the above | Pageview, hit and bandwidth |
| Other | Anything left | Hits and bandwidth |

Paths are cleaned before counting: the query string is removed (UTM parameters are read first),
`index.php` and `index.html` collapse to the directory, repeated slashes collapse, and paths are cut at
512 bytes.

## Visitor and visit

- **Visitor.** The log has no cookie, so a visitor is a hash of IP address and user agent. The hash is
  salted with a random value that is new every day and deleted after two days. The same person on two
  different days is two visitors. The IP address itself is not stored (unless the administrator turns
  `store_ip` on).
- **Visit.** A series of hits from the same visitor in which no gap is longer than **30 minutes** (the
  server setting `session_timeout_min`). A visit belongs to the day it started, even if it runs past
  midnight. Bots never create visits.
- **Unique visitors.** The number of different visitors per day. For a range of several days the daily
  numbers are added up, so someone who comes back on three days counts three times. Matomo does the same
  for ranges.
- **Pageviews.** Hits classed as pageview.
- **Hits.** Every counted request from a human: pages, files, downloads, redirects and errors.
- **Bandwidth.** The size column of the log, summed over human hits. Bots are counted separately.
  Sizes are shown with 1024-based units (KB, MB, GB).

## Engagement

- **Bounce.** A visit with exactly one pageview. Other hits (images, downloads) do not save a bounce.
- **Bounce rate.** Bounces divided by visits.
- **Visit duration.** Time between the first and the last **pageview** of the visit. A one-page visit
  lasts 0 seconds, because the log cannot know how long the visitor stayed on the last page. This makes
  the average duration lower than what a JavaScript tool reports.
- **Pages per visit.** Pageviews divided by visits.
- **Entry page.** The first pageview of a visit. **Bounce rate of an entry page** is bounces divided by
  entries for that page.
- **Exit page.** The last pageview of a visit. **Exit rate of a page** is the number of visits that
  ended on it divided by its pageviews.
- **Engagement histograms.** Visit duration falls into `0-10s`, `11-30s`, `31-60s`, `1-2m`, `2-5m`,
  `5-10m`, `10-30m`, `30m+`. Pages per visit fall into `1`, `2`, `3`, `4`, `5`, `6-10`, `11-20`, `21+`.
  They are written when a visit ends.

## Where visitors come from

The channel is decided on the **first hit of the visit**. The first referrer of the visit wins.

| Channel | Rule |
|---------|------|
| Direct | No referrer, or the referrer is the site itself (including its `www.` variant) |
| Organic search | The referrer is a search engine (Google, Bing, DuckDuckGo, Yahoo, Yandex, Baidu, Ecosia, Qwant, Startpage, Brave, Naver, Seznam, Ask, AOL) |
| Social | The referrer is a social network (Facebook, Instagram, X/Twitter, LinkedIn, Pinterest, Reddit, TikTok, YouTube, Threads, Mastodon, Bluesky, Snapchat, WhatsApp, Telegram and their short links) |
| Paid | `utm_medium` is `cpc`, `ppc`, `paid`, `paidsearch`, `display`, `cpm` or `banner`, or the URL carries `gclid`, `fbclid` or `msclkid` |
| Email | `utm_medium` is `email`, or the referrer is a webmail site |
| Campaign | Any other visit tagged with UTM parameters |
| Referral | Any other external referrer |

Search keywords are shown only for engines that still send them, which is few. Visits from your own
domain are not listed as referrers.

## Audience

Country, city, browser, operating system and device are read from the first hit of the visit.
The country needs the GeoIP database; without it countries are `(unknown)`. Cities need a city
database. Browser versions appear as, for example, `Chrome 129`. Brave cannot be told apart from Chrome
in logs. Language is not in the combined log format, so it is not reported.

## Bots

A user agent counts as a bot when it matches the built-in list (for example `bot`, `crawl`, `spider`,
`curl`, `wget`, `python-requests`, `HeadlessChrome`, `Lighthouse`, `UptimeRobot`, Googlebot, bingbot,
AhrefsBot, SemrushBot, GPTBot, ClaudeBot, CCBot, Amazonbot, Bytespider) or is empty. A crawler that
pretends to be a normal browser cannot be detected from a log, so the bot share is a lower bound.

## Time zones

Days and hours use the domain's timezone, falling back to the account's, then the server's. Changing
the timezone does not rewrite history: earlier days keep the old one, and the dashboard shows a notice
with the date of the change.

## Why numbers differ from other tools

- **No JavaScript.** Visitors who block trackers are still counted; bots that execute no script are
  filtered by user agent only.
- **Caches and CDNs.** A page served by a CDN never reaches the origin log, so it is not counted.
- **Visit duration** is measured between pageviews, so it is shorter than in script-based tools.
- **Unique visitors** are per day, and reset daily by design.
- **Open visits.** A visit still in progress is already counted as a visit at its first hit.

## See also

- [[Using the Dashboard]]
- [[Privacy and GDPR]]
- [[How It Works]]
- [[Configuration]]
