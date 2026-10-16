# Using the Dashboard

The same dashboard is used by account owners in cPanel and by administrators in WHM. This page walks
through it from top to bottom and says which question each card answers. Names in **bold** are the
labels you see on screen.

![Overview tab](https://raw.githubusercontent.com/inalto/webpulse/main/docs/screenshots/overview-1280-light.png)

## Opening it

- **Account owners:** cPanel » Metrics » **WebPulse Analytics**.
- **Administrators and resellers:** WHM » Plugins » **WebPulse Analytics**. It opens on the server
  view; the **Server** button next to the domain picker brings you back to it (see
  [[WHM Administration]]). Pick a domain and the tabs work as described here.

## Header controls

| Control | What it does |
|---------|--------------|
| Domain picker | Choose the domain. It has a search box. A domain without data yet is marked *no data yet*. |
| Date range | **Today**, **Yesterday**, **7 days**, **30 days**, **This month**, **Last month**, **12 months**, **Custom** (a From and To date, then **Apply**). On narrow screens the buttons become a drop-down that also offers **90 days**, **This year** and **All time**. |
| **Compare** | Compares with the previous period of the same length. KPI tiles show the change; the main chart draws the previous period as a dashed line. |
| **Server** (WHM only) | Next to the domain picker. Shows the server view instead of the domain tabs; pick a domain (or click one in the server view) to come back. On a phone only its icon is shown. |
| **Download PDF** (document icon) | Opens a printable report of the domain in a new browser tab; see [PDF report](#pdf-report). |
| Theme button | Switches between light and dark. The default follows your system. |

The domain, range and tab are kept in the address (after `#`), so a view can be bookmarked.

Under the header, a line shows **N active visits** (last 30 minutes) and **last collection 3 min ago**.
It is only as fresh as the last collection run.

## KPI tiles

Eight tiles sit above the chart. Each has the value, the change against the previous period (green or
red arrow, grey when the change is under 1 %) and a small trend line. Hover a tile for its definition.
Click a tile to show that metric in the main chart.

| Tile | Meaning |
|------|---------|
| **Visits** | Series of hits from one visitor with gaps under 30 minutes. Bots are never visits. |
| **Unique visitors** | Distinct visitors per day. Over a range the daily numbers are added up. |
| **Pageviews** | HTML page requests answered successfully. |
| **Pages / visit** | Pageviews divided by visits. |
| **Avg. duration** | First to last pageview of a visit; single-page visits count as 0 s. |
| **Bounce rate** | Share of visits with exactly one pageview. |
| **Bandwidth** | Bytes sent to human visitors. |
| **Bot share** | Share of all requests made by crawlers, monitors and scripts. |

Definitions in detail: [[Metrics and Definitions]].

## Main chart

An area and line chart. The buttons above it choose the metrics: **Visits**, **Pageviews**,
**Visitors**, **Bandwidth**, **Bot hits**, **Errors**. Metrics with the same unit can be combined.
Granularity is automatic: hourly for ranges up to two days, daily up to 120 days, then weekly and
monthly. Shaded columns are weekends. Hover for exact values.

## Tabs

### Overview

| Card | Question it answers |
|------|---------------------|
| **When do people visit?** | At which weekday and hour is traffic highest? A heatmap for ranges of 7 days or more, hour-of-day bars for shorter ones. |
| **Top pages** | What do people read? |
| **Channels** | Where does traffic come from: direct, organic search, social, referral, paid, email, campaign? |
| **Top referrers** | Which sites link to me? |
| **Countries** | Where are my visitors? Needs GeoIP. |
| **Devices** | Desktop, mobile or tablet? |
| **Engagement** | How long do visits last and how many pages do they view? |

### Pages

**Top pages** (pageviews, visits, bandwidth, exit rate), **Entry pages** (entries and bounce rate),
**Exit pages** (exits and exit rate) and **Downloads**. The search box (**Filter pages…**) filters all
four tables.

### Acquisition

**Channels over time**, **Referrer hosts**, **Referrer URLs**, **Search engines**, **Keywords**,
**Campaigns** (UTM campaign) and **Source / medium**. Most search engines no longer send the search
terms, so **Keywords** is usually short.

### Audience

**Countries**, **Cities** (only with a GeoIP city database), **Browsers** and **Operating systems**
(click a row for the versions), **Device types**, and the **Visit duration** and **Pages per visit**
histograms.

### Technical

| Card | Question it answers |
|------|---------------------|
| **HTTP status** | How many requests succeed, redirect or fail? |
| **Errors over time** | When did 4xx and 5xx errors rise? |
| **Not found (404)** | Which missing URLs are requested? Expand a row to see **Linked from**: those referrers are broken links you can fix. |
| **Server errors (5xx)** | Which pages crash? |
| **Bandwidth by file type** | What uses the bandwidth? |
| **Heaviest static files** | Which image or script is too big? |
| **HTTP vs HTTPS**, **HTTP versions**, **Methods** | How is the site requested? |
| **Peak hour** | When was the busiest hour? |

### Bots

**Bot traffic over time** (bot hits against human hits) and **Top crawlers** with hits, bandwidth and
last seen. AI crawlers such as GPTBot, ClaudeBot, CCBot, Bytespider and Amazonbot carry an **AI** badge.

### Visits log

Newest visits first: time, country, device and browser, channel, referrer, duration and the pages
viewed, with status and size. A visit that is still open shows **Active now**. Press **Load more
visits** for older ones. Individual visits are kept only for a limited number of days (30 by default),
and the tab says so; older periods have aggregated reports only.

### Settings (cPanel only)

Per domain, the account owner can set:

- **Timezone.** Days and hours use it. A change affects new data only; the dashboard shows a notice
  with the date of the change.
- **Excluded IP addresses.** One IP or CIDR range per line, at most 50. **Add my current IP** fills in
  your own address.
- **Excluded path prefixes.** One per line, starting with `/`, at most 50.
- **Dashboard preferences:** the default date range and the theme.

Changes apply to data collected from then on; existing reports are not recalculated. In WHM this tab
is not shown: the settings are managed by each account owner. Server-wide settings are in the Server
tab ([[WHM Administration]]).

## PDF report

**Download PDF** (the document icon in the header) opens a report of the selected domain, date range
and comparison in a new browser tab. It contains a cover (domain, period, date and time it was made,
WebPulse version) and then every tab one after another, each starting on a new page: Overview with
the KPI tiles and the main chart, Pages, Acquisition, Audience, Technical, Bots and the 50 most recent
visits of the Visits log. Top lists show 25 rows. Settings are not included.

Once everything has loaded, the browser's print dialog opens by itself: choose **Save as PDF** (or
*Microsoft Print to PDF*) as the printer. If you close the dialog, the **Save as PDF** button at the top
of the report opens it again; **Back to dashboard** returns to the normal view. The report is always
printed in the light theme, A4 and Letter both fit, and charts stay sharp because they are vector
graphics. Nothing is sent anywhere: the PDF is made by your own browser.

In WHM, the same button in the server view prints a server report instead (see
[[WHM Administration]]).

## CSV export

Every table has an **Export CSV** button. It downloads the same rows as the table without the row
limit (at most 10 000 rows), for the selected domain and range. The file is named like
`example.com-page-2026-09-17_2026-10-16.csv`.

## Empty and error states

| Message | Meaning |
|---------|---------|
| **Collecting…** | The domain has no data yet. The first data appears within 5 minutes after the next collection run. |
| **No domains yet** | Domains appear once WebPulse has discovered them and collected their first log lines. |
| **No traffic in this period. Try a longer range.** | The domain has data, but not for this range. |
| **Nothing recorded in this period.** | A single card is empty. |
| GeoIP notice on the Countries card | No GeoIP database is installed. Ask the administrator to enable GeoIP. |
| **Database is being upgraded. Reload in a minute.** | A schema migration is running after an update. |
| **Could not load this report** with **Try again** | A request failed. Retry; see [[Troubleshooting and FAQ]] if it persists. |

## See also

- [[Metrics and Definitions]]
- [[WHM Administration]]
- [[Privacy and GDPR]]
- [Dashboard design notes](https://github.com/inalto/webpulse/blob/main/docs/DASHBOARD.md)
