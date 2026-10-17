# Dashboard — reports, graphs and the reasoning behind them

The same single-page app (vanilla JS, inline SVG charts, inline CSS, ≈ 190 KB
unminified (budget 200 KB, checked by `tests/http_page.rs`), no external requests, light/dark via `prefers-color-scheme` with a
manual toggle) is served to cPanel users, WHM admins and the dev server. It is
rendered by the binary (`http::page::render(&Ctx, &Principal, &PageOptions)`,
`webpulse cgi --page`) as one HTML document with `<script>` and `<style>`
inlined (`ui/*.js` and `ui/app.css` are embedded in the binary), so it works
behind the cPanel shim and the WHM CGI without caring about static-asset paths. The page receives a JSON
bootstrap (`window.WEBPULSE = {apiUrl, principal, domains, defaults, version, embedded, dims, ranges}`,
plus `hostname` for admin-scope principals — it appears on the cover of the WHM reports).

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
│ ● WebPulse   [example.com ▾] [▤ Server]   [Today|Yesterday|7d|30d|Month|Last month|12m|Custom]  [Compare □]  [⤓ PDF]  [🔔]  [☾] │
│   ([▤ Server] and [🔔] only for admin/reseller; icon-only below 640 px; [⤓ PDF] = "Download PDF", opens the report view) │
│ Live: 7 active visits · last collection 3 min ago   (90 days / This year / All time live in the narrow-screen drop-down)                        │
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
Same SPA with `principal.type = admin|reseller`. There is no Server tab: a
**Server** button (stacked-servers icon, same height, radius and border as
the domain switcher, `aria-pressed` = server view shown) sits right after the
domain switcher. It shows the **server view** in the main area — the content
below, rendered by `ui/admin.js` — and hides the domain tabs, the KPI row and
main chart, the range buttons, **Compare** and the live line; the domain
switcher stays. Choosing a domain (switcher, Top domains, "View dashboard" in
Accounts) returns to the domain view with that domain selected; pressing the
pressed button does the same for the current domain. Hash state: `#server`
(bookmarkable, Back works; the pre-0.3 `#t=server` still opens it); an empty
hash lands admins on the server view; the per-domain state
(`#t=…&d=…&u=…&r=…&f=…&to=…&c=…&m=…`) is unchanged and the arrow-key tab
navigation cycles through the domain tabs only. Below 640 px the button
collapses to its icon (`aria-label` "Server overview: all accounts and
domains"). The server view contains:

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
* **Updates** (full width, last): the same panel as the bell popover below.

### Update bell (admin and reseller)
An icon button (bell) right of **Download PDF**, `aria-haspopup="dialog"`. Its data comes from
`admin.update`, fetched once at load (never for cPanel users; the browser never calls GitHub).
When `is_newer`, a small accent dot sits on the bell and its tooltip/`aria-label` reads
"WebPulse X.Y.Z available" (else "WebPulse updates"). Clicking opens a popover (`role="dialog"`,
Escape or a click outside closes it) with: a heading (Update available / WebPulse is up to date /
Update check failed), *Installed* → *Latest* versions, release date and "Checked … ago", a
**Release notes** link (`html_url`, new tab, `rel="noopener noreferrer"`), the last error if any.
Root also gets **Upgrade now** (only when `can_upgrade`) and **Check again** (`admin.update_check`;
"Checked less than an hour ago" when throttled); resellers read "The server administrator can
upgrade WebPulse." **Upgrade now** first asks inline ("The WHM pages stay available during the
upgrade, which takes about a minute"), then calls `admin.run {type: "upgrade"}` and polls
`admin.jobs` every 3 s for up to 10 minutes, showing an indeterminate bar, the job number and the
elapsed time (the bell's dot blinks meanwhile). On `ok`: "Upgraded to X.Y.Z" with **Reload**, and
the page reloads by itself after 5 s; on `error`: the job message and "Details are in
/var/webpulse/log/upgrade.log on the server", with **Try again**. A job still running when the page
loads (`upgrade_job.status = running`) is picked up again. All text is set with `textContent`.

## Report view and PDF (`#report`)
The header's **Download PDF** icon button (every principal, domain view with
data; in the server view it prints the server report) is a link opening the
same document in a new tab with `#report&d=…&u=…&r=…&f=…&to=…&c=…&m=…` (the
current domain/range/compare state) or `#report=server`. No server-side PDF
code: the browser prints the page (Save as PDF), so the same API, auth and
URLs work for cPanel (`index.live.php#report…` inside the Jupiter chrome),
WHM (`index.cgi#report…`) and the dev server.

* **Domain report:** on-screen toolbar (not printed): **Save as PDF**
  (`window.print()`), **Back to dashboard**, a status line. Cover: domain,
  period (and compared period), generated timestamp, account (WHM), server
  hostname (WHM, from the bootstrap), "WebPulse <version>", contents. Then
  every tab in sequence — Overview (KPI tiles + main chart + cards), Pages,
  Acquisition, Audience, Technical, Bots, the first 50 visits of the Visits
  log — Settings excluded. Top lists use limit 25 (no "Show more" on paper);
  cards with three or more columns take the full width. Each section ends
  with a footer line (domain, period, version).
* **Server report** (`WebPulseAdmin.report`): server KPI tiles, every top
  domain (`admin.overview&top=500`), accounts summary table (state and
  Default/On/Off as text, no switches), the 20 most recent jobs; no settings.
* **Auto print:** every API call of the report is counted; when the last one
  has settled (and the sections exist) the status says "Report ready.",
  `#webpulse-app` gets `data-report-ready="1"` and `window.print()` runs once
  (guarded). `noprint=1` in the query (`?noprint=1`, kept by the dev server's
  token redirect) or in the hash (`#report&noprint=1&…`) disables it for
  headless rendering and screenshots.
* **Print stylesheet** (`@media print`, also applies when printing the
  normal dashboard): dark tokens are screen-only, so paper is always light;
  header, tabs, toolbar, buttons, "Show more", CSV links, info buttons and
  the live badge are hidden; content max 190 mm wide (A4 and Letter);
  slightly smaller type; links undecorated; `break-before: page` per section,
  `break-inside: avoid` for cards, tiles and visits; charts are the SVGs
  (vectors, `max-width: 100%`); `@page` margin box with "page / pages".
* **cPanel chrome:** the embedded fragment sits deep inside Jupiter's layout,
  so `body > :not(#webpulse-app)` is not enough: the report marks every
  sibling of each ancestor of `#webpulse-app` with `wp-print-hide` and the
  ancestors with `wp-print-host` (margins, padding, positioning and overflow
  reset in print), which hides the top bar, sidebar, page title and footer.

## Charts (ui/charts.js)
Hand-written, ≈ 500 lines: `lineChart`, `areaChart` (stacked option),
`barChart` (vertical, horizontal), `donut`, `heatmap`, `sparkline`,
`histogram`. All responsive (ResizeObserver, redraw on resize), tooltips via a
single absolutely positioned `<div>`, colours from CSS custom properties so
dark mode works, categorical palette of 8 colours validated for colour-vision
deficiency on both light and dark surfaces, numbers formatted with `Intl.NumberFormat` (compact for axes),
bytes formatted as KB/MB/GB (1024), durations as `1m 32s`.
