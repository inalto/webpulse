# Dashboard — reports, graphs and the reasoning behind them

The same single-page app (vanilla JS, inline SVG charts, inline CSS, ≈ 120 KB
unminified, no external requests, light/dark via `prefers-color-scheme` with a
manual toggle) is served to cPanel users, WHM admins and the dev server. It is
rendered by `WebPulse\Http\Page::render(Principal, PageOptions)` as one HTML
document with `<script>` and `<style>` inlined, so it works wherever cpsrvd
serves PHP without caring about static-asset paths. The page receives a JSON
bootstrap (`window.WEBPULSE = {apiUrl, principal, domains, defaults, version}`).

Design principles (from Plausible's simplicity, Matomo's depth, GoAccess'
technical panels, AWStats' completeness):

* **One screen tells the story.** KPI tiles with sparklines and deltas, one
  main chart, then cards in a responsive grid. Nothing requires configuration.
* **Every number has a tooltip explaining its definition** (text from
  METRICS.md). Hover a bar → exact value and date.
* **Click to drill.** Clicking a row filters nothing (log analytics cannot
  segment cheaply) but opens the row's details where they exist: a 404 shows
  its referrers, a page shows exit rate, entry bounce rate, bandwidth.
* **Period comparison** everywhere a number is shown (`compare` toggle).
* **Keyboard and screen-reader friendly:** every chart has a hidden table
  twin (`<table class="sr-only">`) and all tables are real `<table>`s.
* **Empty and error states are explicit:** "Collecting… first data appears
  within 5 minutes", "No GeoIP database installed — ask your administrator",
  "Database is being upgraded".

## Layout

```
┌ Header ─────────────────────────────────────────────────────────────────────┐
│ ● WebPulse   [example.com ▾]   [Today|Yesterday|7d|30d|Month|Last month|12m|Custom]  [Compare □]  [☾] │
│ Live: 7 active visitors · last collection 3 min ago                        │
├ KPI row (8 tiles, 4 per row on tablet, 2 on phone) ─────────────────────────┤
│ Visits  Unique visitors  Pageviews  Pages/visit  Avg. duration  Bounce rate  Bandwidth  Bot share │
│ each: big number, Δ vs previous period (green/red, muted when |Δ|<1 %), 30-point sparkline │
├ Main chart ─────────────────────────────────────────────────────────────────┤
│ Area/line: visits + pageviews (toggle visitors, bandwidth, bot hits, errors); dashed = previous period │
│ granularity auto (hour/day/week/month), weekend shading on day granularity │
├ Tabs ───────────────────────────────────────────────────────────────────────┤
│ Overview · Pages · Acquisition · Audience · Technical · Bots · Visits log · Settings │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Overview tab
| Card | Graph | Why |
|------|-------|-----|
| When do people visit? | 7×24 heatmap (weekday × hour) for ranges ≥ 7 days; otherwise hour-of-day bars | capacity planning, publishing time, support staffing |
| Top pages | horizontal bar table (pageviews, share) | the question everybody asks first |
| Channels | donut (direct / organic / social / referral / paid / email / campaign) + legend with visits & bounce rate | where traffic comes from, at a glance |
| Top referrers | table with favicon-less host chips | who links to me |
| Countries | horizontal bars with flag emoji | audience geography (needs GeoIP, otherwise card explains how to enable) |
| Devices | three-segment stacked bar desktop/mobile/tablet | responsive design decisions |
| Engagement | two small histograms: visit duration buckets, pages per visit buckets | depth of interest (Matomo "Engagement") |

### Pages tab
Top pages (pageviews, visits, bandwidth, exit rate), Entry pages (entries,
bounce rate, bounce-rate bar), Exit pages (exits, exit rate), Downloads
(hits, bandwidth), search box filters all four tables server-side (`q`).
Export CSV button on every table.

### Acquisition tab
Channel trend (stacked area by channel over time, built client-side from
`report&name=channel` with `series=1` — the API returns per-day rows when
`series=1`, limited to the top 7 keys + other), Referrer hosts, Referrer
URLs, Search engines (+ share donut), Keywords (with the "(most engines hide
keywords)" explanation), Campaigns (utm_campaign), Source / Medium.

### Audience tab
Countries (with map-less ranked list — flags + bars; a world map is deferred),
Cities (if city DB), Browsers (+ version drill), Operating systems (+ version),
Device types, Visit duration and Pages per visit histograms.

### Technical tab (GoAccess inspired)
HTTP status donut, 4xx/5xx trend lines, Top 404 pages with their referrers
expandable (fix broken links), 5xx pages, Bandwidth by file type (stacked
bar), Top static files by bandwidth (find the 8 MB hero image), HTTP vs
HTTPS share, HTTP versions, Methods, Peak hour.

### Bots tab
Bot share trend (line: bot hits vs human hits), Top crawlers (hits,
bandwidth, last seen), AI crawlers highlighted (GPTBot, ClaudeBot, CCBot,
Bytespider, Amazonbot…) because owners increasingly want to know.

### Visits log tab
Reverse-chronological list of visits: time, country flag, device/browser/OS
icons (inline SVG glyphs), channel badge, referrer, duration, and the list of
actions (page path, status, bytes) — Matomo's most loved feature. "Load
more" cursor pagination.

### Settings tab (cPanel user only)
Per-domain timezone, excluded IPs/CIDRs (with "add my current IP" button that
uses the IP the API reports from `REMOTE_ADDR`), excluded path prefixes; a
note that changes apply to new data only. Saved via `save_user_settings`.

## WHM admin page
Same SPA with `principal.type = admin|reseller` and an extra **Server** tab
first:

* Server KPIs (visits/pageviews/bandwidth today/7d/30d across visible
  accounts; bot share; data-dir size; collector lag; cron & hook status).
* Top domains table (visits 30d, pageviews, bandwidth, bots, last collect).
* **Accounts** table: user, owner, effective state badge, 3-state switch
  (Default/On/Off), expandable domains with their own 3-state switch,
  database size, "View dashboard" opens that domain in the normal tabs.
* **Settings** (root): default enabled, retention days, max rows/dim, session
  timeout, store IP, server-wide excluded IPs/paths, GeoIP toggle, timezone.
* **Jobs**: Run collection now / Backfill from archives (pick account) /
  Run rollup, job history with status and line counts.

## Charts (ui/charts.js)
Hand-written, ≈ 500 lines: `lineChart`, `areaChart` (stacked option),
`barChart` (vertical, horizontal), `donut`, `heatmap`, `sparkline`,
`histogram`. All responsive (ResizeObserver, redraw on resize), tooltips via a
single absolutely positioned `<div>`, colours from CSS custom properties so
dark mode works, categorical palette of 8 colours validated for colour-vision
deficiency on both light and dark surfaces, numbers formatted with `Intl.NumberFormat` (compact for axes),
bytes formatted as KB/MB/GB (1024), durations as `1m 32s`.
