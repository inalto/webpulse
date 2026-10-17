/* WebPulse dashboard SPA (vanilla JS, no dependencies).
 * State lives in the URL hash so back/forward and bookmarks work.
 * Every untrusted string (paths, referrers, user agents…) is inserted with
 * textContent; no innerHTML is used anywhere.
 */
(function () {
  'use strict';

  var C = window.WebPulseCharts;
  var I = window.WebPulseI18n;
  var t = I.t;
  var fmt = C.fmt;
  var h = C.h;
  var clear = C.clear;

  var RANGES = ['today', 'yesterday', '7d', '30d', 'month', 'lastmonth', '12m'];
  var ALL_RANGES = ['today', 'yesterday', '7d', '30d', '90d', 'month', 'lastmonth', '12m', 'year', 'all'];
  var TABS = ['overview', 'pages', 'acquisition', 'audience', 'technical', 'bots', 'visits', 'settings'];
  var CHANNELS = ['direct', 'organic', 'social', 'referral', 'paid', 'email', 'campaign'];
  var DEVICES = ['desktop', 'mobile', 'tablet', 'other'];
  var AI_BOTS = /^(GPTBot|ChatGPT-User|OAI-SearchBot|ClaudeBot|Claude-Web|anthropic-ai|CCBot|Bytespider|Amazonbot|PerplexityBot|Google-Extended|Applebot-Extended|cohere-ai|Diffbot|meta-externalagent|FacebookBot|YouBot|Timpibot|ImagesiftBot|Omgilibot)/i;

  /* Chart metrics: unit, colour (fixed per entity) and how to read a series. */
  var METRICS = {
    visits: { unit: 'count', color: 'var(--wp-s1)', chip: true },
    pageviews: { unit: 'count', color: 'var(--wp-s2)', chip: true },
    visitors: { unit: 'count', color: 'var(--wp-s3)', chip: true },
    bytes: { unit: 'bytes', color: 'var(--wp-s1)', chip: true },
    bot_hits: { unit: 'count', color: 'var(--wp-s7)', chip: true },
    errors: { unit: 'count', color: 'var(--wp-s8)', chip: true },
    pages_per_visit: { unit: 'decimal', color: 'var(--wp-s1)' },
    avg_duration: { unit: 'duration', color: 'var(--wp-s1)' },
    bounce_rate: { unit: 'ratio', color: 'var(--wp-s1)' },
    bot_share: { unit: 'ratio', color: 'var(--wp-s7)' }
  };
  var KPIS = [
    { key: 'visits', metric: 'visits', unit: 'count', def: 'visits' },
    { key: 'visitors', metric: 'visitors', unit: 'count', def: 'visitors' },
    { key: 'pageviews', metric: 'pageviews', unit: 'count', def: 'pageviews' },
    { key: 'pages_per_visit', metric: 'pages_per_visit', unit: 'decimal', def: 'pages_per_visit' },
    { key: 'avg_duration', metric: 'avg_duration', unit: 'duration', def: 'avg_duration' },
    { key: 'bounce_rate', metric: 'bounce_rate', unit: 'ratio', def: 'bounce_rate', inverse: true },
    { key: 'bytes', metric: 'bytes', unit: 'bytes', def: 'bytes' },
    { key: 'bot_share', metric: 'bot_share', unit: 'ratio', def: 'bot_share', inverse: true }
  ];

  var S = null; // app state

  /* ------------------------------------------------------------- helpers */
  function svgIcon(name) {
    var NS = 'http://www.w3.org/2000/svg';
    var paths = {
      chevron: 'M4 6l4 4 4-4',
      moon: 'M13.5 9.5A5.5 5.5 0 1 1 6.5 2.5a4.5 4.5 0 0 0 7 7z',
      sun: 'M8 5a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM8 1v1.5M8 13.5V15M1 8h1.5M13.5 8H15M3 3l1 1M12 12l1 1M3 13l1-1M12 4l1-1',
      download: 'M8 2v8M4.5 6.5L8 10l3.5-3.5M2.5 13.5h11',
      calendar: 'M2.5 4h11v9.5h-11zM2.5 7h11M5.5 2.5v3M10.5 2.5v3',
      desktop: 'M2 3h12v8H2zM6 14h4M8 11v3',
      mobile: 'M5 1.5h6v13H5zM7.5 12.5h1',
      tablet: 'M3 1.5h10v13H3zM7.5 12.5h1',
      other: 'M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2zM6.5 6.5a1.5 1.5 0 1 1 2 1.4c-.5.2-.5.6-.5 1.1M8 11v.5',
      refresh: 'M13 8a5 5 0 1 1-1.5-3.5M13 2.5v3h-3',
      server: 'M2.5 2.5h11v4.5h-11zM2.5 9h11v4.5h-11zM5 4.75h.01M5 11.25h.01M8 4.75h3M8 11.25h3',
      bell: 'M4 11.5V7a4 4 0 0 1 8 0v4.5l1.25 1.5H2.75zM6.5 13.5a1.5 1.5 0 0 0 3 0',
      report: 'M9.5 1.5H4.5a1 1 0 0 0-1 1v11a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1v-9zM9.5 1.5v3h3M8 7v4.5M6 9.5l2 2 2-2'
    };
    var svg = document.createElementNS(NS, 'svg');
    svg.setAttribute('viewBox', '0 0 16 16');
    svg.setAttribute('aria-hidden', 'true');
    svg.setAttribute('fill', 'none');
    svg.setAttribute('stroke', 'currentColor');
    svg.setAttribute('stroke-width', '1.5');
    svg.setAttribute('stroke-linecap', 'round');
    svg.setAttribute('stroke-linejoin', 'round');
    var p = document.createElementNS(NS, 'path');
    p.setAttribute('d', paths[name] || paths.other);
    svg.appendChild(p);
    return svg;
  }

  function logo() {
    var NS = 'http://www.w3.org/2000/svg';
    var svg = document.createElementNS(NS, 'svg');
    svg.setAttribute('viewBox', '0 0 24 24');
    svg.setAttribute('width', '24');
    svg.setAttribute('height', '24');
    svg.setAttribute('aria-hidden', 'true');
    var r = document.createElementNS(NS, 'rect');
    r.setAttribute('width', '24'); r.setAttribute('height', '24'); r.setAttribute('rx', '6');
    r.setAttribute('class', 'wp-logo-bg');
    var p = document.createElementNS(NS, 'path');
    p.setAttribute('d', 'M3.5 13h4l2-5 3 9 2.5-6 1.5 2h4');
    p.setAttribute('fill', 'none'); p.setAttribute('stroke-width', '2'); p.setAttribute('stroke-linecap', 'round'); p.setAttribute('stroke-linejoin', 'round');
    p.setAttribute('class', 'wp-logo-line');
    svg.appendChild(r); svg.appendChild(p);
    return svg;
  }

  function btn(cls, text, onClick, parent) {
    var b = h('button', 'wp-btn' + (cls ? ' ' + cls : ''), text, parent);
    b.type = 'button';
    if (onClick) b.addEventListener('click', onClick);
    return b;
  }

  /** "?" button whose tooltip explains a metric (METRICS.md). */
  function info(text, parent) {
    var b = h('button', 'wp-info', '?', parent);
    b.type = 'button';
    b.setAttribute('aria-label', text);
    function show() { C.tipAt(b, text, []); }
    b.addEventListener('pointerenter', show);
    b.addEventListener('focus', show);
    b.addEventListener('pointerleave', C.hideTip);
    b.addEventListener('blur', C.hideTip);
    b.addEventListener('click', function (ev) { ev.stopPropagation(); show(); });
    return b;
  }

  function flag(cc) {
    if (!cc || !/^[a-z]{2}$/i.test(cc)) return '\uD83C\uDF10';
    var u = cc.toUpperCase();
    return String.fromCodePoint(0x1F1E6 + u.charCodeAt(0) - 65, 0x1F1E6 + u.charCodeAt(1) - 65);
  }
  var flagSupport = null;
  /** True when the platform draws regional-indicator pairs as one flag glyph. */
  function canFlag() {
    if (flagSupport !== null) return flagSupport;
    try {
      var cv = document.createElement('canvas');
      cv.width = cv.height = 32;
      var cx = cv.getContext('2d');
      cx.font = '24px sans-serif';
      var pair = cx.measureText('\uD83C\uDDEE\uD83C\uDDF9').width;
      var one = cx.measureText('\uD83C\uDDEE').width;
      flagSupport = pair > 0 && pair < one * 1.5;
    } catch (e) { flagSupport = false; }
    return flagSupport;
  }
  /** Flag emoji, or a two-letter code chip where flags are not drawn. */
  function flagEl(cc, parent) {
    var valid = cc && /^[a-z]{2}$/i.test(cc);
    var el;
    if (canFlag() || !valid) el = h('span', 'wp-key-flag', valid ? flag(cc) : '\u2014', parent);
    else el = h('span', 'wp-key-cc', cc.toUpperCase(), parent);
    el.setAttribute('aria-hidden', 'true');
    return el;
  }

  var regionNames = null;
  try { regionNames = new Intl.DisplayNames(['en'], { type: 'region' }); } catch (e) { regionNames = null; }
  function countryName(cc, label) {
    if (label) return label;
    if (!cc || cc.charAt(0) === '(') return t('unknown');
    try { return regionNames ? regionNames.of(cc.toUpperCase()) : cc.toUpperCase(); } catch (e) { return cc.toUpperCase(); }
  }

  function ago(iso) {
    if (!iso) return null;
    var s = (Date.now() - new Date(iso).getTime()) / 1000;
    if (!(s >= 0)) s = 0;
    if (s < 60) return t('ago.now');
    if (s < 3600) return t('ago.min', { n: Math.floor(s / 60) });
    if (s < 86400) return t('ago.h', { n: Math.floor(s / 3600) });
    return t('ago.d', { n: Math.floor(s / 86400) });
  }

  var dateFmt = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short' });
  var dateFmtY = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
  var dateFmtLong = new Intl.DateTimeFormat(undefined, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' });
  var monthFmt = new Intl.DateTimeFormat(undefined, { month: 'short', year: 'numeric' });
  function parseDay(d) { return new Date(d.slice(0, 10) + 'T12:00:00'); }
  /** Short axis label for a bucket label of a granularity. */
  function bucketTick(label, g) {
    if (g === 'hour') return label.slice(11, 16);
    if (g === 'month') return monthFmt.format(parseDay(label + '-01'));
    return dateFmt.format(parseDay(label));
  }
  function bucketTitle(label, g) {
    if (g === 'hour') return dateFmtLong.format(parseDay(label)) + ' ' + label.slice(11, 16);
    if (g === 'month') return monthFmt.format(parseDay(label + '-01'));
    if (g === 'week') return t('chart.granularity.week') + ' · ' + dateFmtY.format(parseDay(label));
    return dateFmtLong.format(parseDay(label));
  }
  function timeOf(iso) {
    // ISO with offset: show the domain's local clock (the string's own hh:mm).
    var m = /T(\d\d:\d\d)/.exec(iso || '');
    return m ? m[1] : '';
  }
  function dayOf(iso) { return (iso || '').slice(0, 10); }

  function storage(key, val) {
    try {
      if (val === undefined) return window.localStorage.getItem(key);
      if (val === null) window.localStorage.removeItem(key); else window.localStorage.setItem(key, val);
    } catch (e) { /* private mode */ }
    return null;
  }

  /* ----------------------------------------------------------------- API */
  function qs(params) {
    var out = [];
    Object.keys(params).forEach(function (k) {
      var v = params[k];
      if (v === undefined || v === null || v === '' || v === false) return;
      out.push(encodeURIComponent(k) + '=' + encodeURIComponent(v === true ? '1' : v));
    });
    return out.join('&');
  }
  function apiHref(params) {
    return S.apiUrl + (S.apiUrl.indexOf('?') >= 0 ? '&' : '?') + qs(params);
  }
  function api(params, body) {
    var url = apiHref(params);
    var cacheable = !body && params.a !== 'realtime' && params.a !== 'admin.jobs';
    if (cacheable && S.cache[url]) return S.cache[url];
    var init = { credentials: 'same-origin', headers: { Accept: 'application/json' } };
    if (body) {
      init.method = 'POST';
      init.headers['Content-Type'] = 'application/json';
      init.headers['X-WebPulse-Request'] = '1';
      init.body = JSON.stringify(body);
    }
    // transport: fetch(), or the Cockpit adapter's cockpit.spawn bridge (same Response shape)
    var p = (S.fetchFn || window.fetch.bind(window))(url, init).then(function (res) {
      return res.text().then(function (txt) {
        var j = null;
        try { j = JSON.parse(txt); } catch (e) { j = null; }
        if (!j || !j.ok) {
          var err = new Error(j && j.error ? j.error.message : 'HTTP ' + res.status);
          err.code = j && j.error ? j.error.code : 'http_' + res.status;
          err.status = res.status;
          throw err;
        }
        // a mutation changes what GETs would return: drop the whole GET cache so that
        // reloads (GeoIP card, settings, accounts) show the saved state, not a stale copy
        if (body) S.cache = {};
        return j.data;
      });
    }, function () {
      var err = new Error(t('error.network'));
      err.code = 'network';
      throw err;
    });
    if (cacheable) {
      S.cache[url] = p;
      p.catch(function () { delete S.cache[url]; });
    }
    if (S.report) {
      S.pending++;
      p.then(settled, settled);
    }
    return p;
  }

  function domainParams() {
    var d = S.domain;
    if (!d) return {};
    var p = { domain: d.domain };
    if (S.boot.principal.type !== 'user') p.user = d.user;
    return p;
  }
  function rangeParams() {
    if (S.range === 'custom' && S.from && S.to) return { from: S.from, to: S.to };
    return { range: S.range };
  }
  function scoped(extra) {
    var p = {};
    var a = domainParams(), b = rangeParams(), k;
    for (k in a) p[k] = a[k];
    for (k in b) p[k] = b[k];
    for (k in extra) p[k] = extra[k];
    return p;
  }

  /* ------------------------------------------------------------ routing */
  function parseHash() {
    var out = {};
    var raw = (location.hash || '').replace(/^#\/?/, '');
    raw.split('&').forEach(function (kv) {
      if (!kv) return;
      var i = kv.indexOf('=');
      var k = decodeURIComponent(i < 0 ? kv : kv.slice(0, i));
      out[k] = i < 0 ? '' : decodeURIComponent(kv.slice(i + 1).replace(/\+/g, ' '));
    });
    return out;
  }
  function stateHash(patch) {
    var st = {
      t: S.tab, d: S.domain ? S.domain.domain : '', u: S.domain && S.boot.principal.type !== 'user' ? S.domain.user : '',
      r: S.range, f: S.range === 'custom' ? S.from : '', to: S.range === 'custom' ? S.to : '', c: S.compare ? '1' : '',
      m: S.metrics.join(',')
    };
    for (var k in patch) st[k] = patch[k];
    var parts = [];
    ['t', 'd', 'u', 'r', 'f', 'to', 'c', 'm'].forEach(function (k) {
      if (st[k]) parts.push(k + '=' + encodeURIComponent(st[k]));
    });
    return '#' + parts.join('&');
  }
  /** Hash of the printable report for the current view (opened in a new tab). */
  function reportHash() {
    if (S.view === 'server') return '#report=server';
    return stateHash({ t: '' }).replace(/^#/, '#report&');
  }
  function navigate(patch, replace) {
    var hsh = stateHash(patch);
    if (hsh === location.hash) { applyHash(); return; }
    if (replace && history.replaceState) { history.replaceState(null, '', hsh); applyHash(); }
    else location.hash = hsh;
  }
  function findDomain(name, user) {
    var list = S.domains;
    for (var i = 0; i < list.length; i++) {
      if (list[i].domain === name && (!user || list[i].user === user)) return list[i];
    }
    return null;
  }
  function firstDomain() {
    for (var i = 0; i < S.domains.length; i++) if (S.domains[i].has_data) return S.domains[i];
    return S.domains[0] || null;
  }
  /** Domain, range, compare and chart metrics from a parsed hash. */
  function readState(p) {
    S.domain = (p.d && findDomain(p.d, p.u)) || S.domain || firstDomain();
    S.range = p.r === 'custom' && /^\d{4}-\d\d-\d\d$/.test(p.f || '') && /^\d{4}-\d\d-\d\d$/.test(p.to || '') ? 'custom'
      : (ALL_RANGES.indexOf(p.r) >= 0 ? p.r : S.defaults.range);
    S.from = S.range === 'custom' ? p.f : null;
    S.to = S.range === 'custom' ? p.to : null;
    S.compare = p.c === '1';
    var ms = (p.m || '').split(',').filter(function (m) { return METRICS[m]; });
    S.metrics = ms.length ? ms : ['visits', 'pageviews'];
  }
  function applyHash() {
    var p = parseHash();
    var tabs = availableTabs();
    var prevKey = dataKey();
    // Admins land on the server view; `#t=server` is the pre-0.3 bookmark form.
    S.view = S.isAdmin && ('server' in p || p.t === 'server' || !location.hash.replace(/^#\/?/, '')) ? 'server' : 'domain';
    S.tab = tabs.indexOf(p.t) >= 0 ? p.t : (S.view === 'server' && S.tab) || tabs[0];
    readState(p);
    if (dataKey() !== prevKey) { S.cache = {}; S.overview = null; }
    render();
  }
  /** Hash change: the report view and the dashboard are two different shells. */
  function route() {
    var p = parseHash();
    if ('report' in p) {
      if (S.mode === 'report' && S.reportHash === location.hash) return;
      S.mode = 'report';
      S.reportHash = location.hash;
      renderReport(p);
      return;
    }
    if (S.mode !== 'app') { S.mode = 'app'; S.report = false; buildShell(); applyTheme(); }
    applyHash();
  }
  function dataKey() {
    return (S.domain ? S.domain.user + '/' + S.domain.domain : '') + '|' + S.range + '|' + (S.from || '') + '|' + (S.to || '');
  }
  function availableTabs() {
    // Admins and resellers reach the server view through the header button, not a tab.
    return S.isAdmin ? TABS.filter(function (x) { return x !== 'settings'; }) : TABS.slice();
  }

  /* --------------------------------------------------------------- theme */
  function applyTheme() {
    var th = S.mode === 'report' ? 'light' : S.theme;
    if (th === 'light' || th === 'dark') S.root.setAttribute('data-theme', th);
    else S.root.removeAttribute('data-theme');
    if (S.els && S.els.themeBtn) {
      clear(S.els.themeBtn);
      S.els.themeBtn.appendChild(svgIcon(isDark() ? 'sun' : 'moon'));
    }
  }
  function isDark() {
    if (S.theme === 'dark') return true;
    if (S.theme === 'light') return false;
    return !!(window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches);
  }
  function toggleTheme() {
    S.theme = isDark() ? 'light' : 'dark';
    storage('webpulse.theme', S.theme);
    applyTheme();
    rerenderCharts();
  }
  function rerenderCharts() { /* colours come from CSS variables: nothing to redraw */ }

  /* ---------------------------------------------------------------- shell */
  function buildShell() {
    var root = S.root;
    clear(root);
    var els = S.els = {};
    var header = h('header', 'wp-header', null, root);
    var hi = h('div', 'wp-header-inner', null, header);
    var brand = h('div', 'wp-brand', null, hi);
    brand.appendChild(logo());
    h('span', null, t('app.name'), brand);

    els.domainWrap = h('div', 'wp-domain', null, hi);
    els.domainBtn = btn('wp-domain-btn', null, function (ev) { ev.stopPropagation(); toggleDomainPop(); }, els.domainWrap);
    els.domainBtn.setAttribute('aria-haspopup', 'listbox');
    els.domainBtn.setAttribute('aria-expanded', 'false');
    els.domainBtn.setAttribute('aria-label', t('domain.label'));
    if (S.isAdmin) {
      els.serverBtn = btn('wp-server-btn', null, function () {
        if (S.view === 'server') navigate({}); else location.hash = '#server';
      }, hi);
      els.serverBtn.appendChild(svgIcon('server'));
      h('span', 'wp-server-text', t('server.button'), els.serverBtn);
      els.serverBtn.setAttribute('aria-label', t('server.label'));
      els.serverBtn.title = t('server.label');
    }

    h('div', 'wp-header-spacer', null, hi);
    var ctr = els.controls = h('div', 'wp-controls', null, hi);
    els.seg = h('div', 'wp-segmented', null, ctr);
    els.seg.setAttribute('role', 'group');
    els.seg.setAttribute('aria-label', t('range.label'));
    els.rangeBtns = {};
    RANGES.concat(['custom']).forEach(function (r) {
      var b = h('button', null, t('range.' + r), els.seg);
      b.type = 'button';
      b.addEventListener('click', function (ev) {
        if (r === 'custom') { ev.stopPropagation(); toggleCustomPop(b); return; }
        navigate({ r: r, f: '', to: '' });
      });
      els.rangeBtns[r] = b;
    });
    els.rangeSelect = h('select', 'wp-select wp-range-select', null, ctr);
    els.rangeSelect.setAttribute('aria-label', t('range.label'));
    ALL_RANGES.concat(['custom']).forEach(function (r) {
      var o = h('option', null, t('range.' + r), els.rangeSelect);
      o.value = r;
    });
    els.rangeSelect.addEventListener('change', function () {
      var v = els.rangeSelect.value;
      if (v === 'custom') { toggleCustomPop(els.rangeSelect); return; }
      navigate({ r: v, f: '', to: '' });
    });
    els.customWrap = h('div', 'wp-domain', null, ctr);

    els.compareBtn = btn('wp-toggle', null, function () { navigate({ c: S.compare ? '' : '1' }); }, ctr);
    h('span', 'wp-switch', null, els.compareBtn);
    h('span', 'wp-toggle-text', t('compare.label'), els.compareBtn);
    els.compareBtn.title = t('compare.hint');
    els.compareBtn.setAttribute('aria-label', t('compare.hint'));

    els.pdfBtn = h('a', 'wp-btn wp-icon-btn', null, ctr);
    // inside Cockpit the report opens in the module's own frame (and prints it)
    if (!S.fetchFn) {
      els.pdfBtn.target = '_blank';
      els.pdfBtn.rel = 'noopener';
    }
    els.pdfBtn.appendChild(svgIcon('report'));

    // admin/reseller: update bell (data from admin.update; never for cPanel users)
    if (S.isAdmin && window.WebPulseAdmin && window.WebPulseAdmin.bell) {
      els.bellWrap = h('div', 'wp-domain wp-bell-wrap', null, ctr);
      els.bellBtn = btn('wp-icon-btn wp-bell', null, function (ev) { ev.stopPropagation(); toggleUpdatePop(); }, els.bellWrap);
      els.bellBtn.appendChild(svgIcon('bell'));
      els.bellBtn.setAttribute('aria-haspopup', 'dialog');
      els.bellBtn.setAttribute('aria-expanded', 'false');
      window.WebPulseAdmin.bell(els.bellBtn, adminCtx());
    }

    els.themeBtn = btn('wp-icon-btn', null, toggleTheme, ctr);
    els.themeBtn.setAttribute('aria-label', t('theme.toggle'));
    els.themeBtn.title = t('theme.toggle');

    var shell = els.shell = h('main', 'wp-shell', null, root);
    els.live = h('div', 'wp-live', null, shell);
    els.live.setAttribute('aria-live', 'polite');
    els.notice = h('div', null, null, shell);
    els.hero = h('section', 'wp-hero', null, shell);
    els.hero.setAttribute('aria-label', t('chart.metrics'));
    els.kpis = h('div', 'wp-kpis', null, els.hero);
    els.chartHead = h('div', 'wp-chart-head', null, els.hero);
    els.chart = h('div', 'wp-main-chart', null, els.hero);
    els.tabs = h('nav', 'wp-tabs', null, shell);
    els.tabs.setAttribute('role', 'tablist');
    els.content = h('div', 'wp-content', null, shell);
    els.content.setAttribute('role', 'tabpanel');
    els.foot = h('footer', 'wp-foot', null, shell);
    h('span', null, 'WebPulse ' + (S.boot.version || ''), els.foot);
    h('span', null, 'Log-based, cookie-free analytics', els.foot);
    if (S.boot.userLabel) h('span', 'wp-foot-user', S.boot.userLabel, els.foot);
    var gc = S.boot.geoip_credit;
    if (gc) {
      var ga = h('a', 'wp-foot-credit', gc.text, els.foot);
      ga.href = gc.url; ga.target = '_blank'; ga.rel = 'noopener noreferrer';
    }

    if (S.listening) return;
    S.listening = true;
    document.addEventListener('click', closePops);
    document.addEventListener('keydown', function (ev) { if (ev.key === 'Escape') { closePops(); C.hideTip(); } });
    window.addEventListener('scroll', C.hideTip, { passive: true });
  }

  function closePops() {
    var pops = S.root.querySelectorAll('.wp-pop');
    for (var i = 0; i < pops.length; i++) pops[i].parentNode.removeChild(pops[i]);
    if (S.els && S.els.domainBtn) S.els.domainBtn.setAttribute('aria-expanded', 'false');
    if (S.els && S.els.bellBtn) S.els.bellBtn.setAttribute('aria-expanded', 'false');
  }

  function toggleUpdatePop() {
    var wrap = S.els.bellWrap;
    var open = wrap.querySelector('.wp-pop');
    closePops();
    if (open) { S.els.bellBtn.focus(); return; }
    S.els.bellBtn.setAttribute('aria-expanded', 'true');
    var pop = h('div', 'wp-pop is-right wp-upd-pop', null, wrap);
    pop.setAttribute('role', 'dialog');
    pop.setAttribute('aria-label', t('upd.title'));
    pop.addEventListener('click', function (ev) { ev.stopPropagation(); });
    if (window.innerWidth <= 640) pop.style.top = Math.round(S.els.bellBtn.getBoundingClientRect().bottom + 6) + 'px';
    window.WebPulseAdmin.updatePanel(h('div', 'wp-upd', null, pop), adminCtx());
    var f = pop.querySelector('button, a');
    if (f) f.focus();
  }

  function toggleDomainPop() {
    var wrap = S.els.domainWrap;
    var open = wrap.querySelector('.wp-pop');
    closePops();
    if (open) return;
    S.els.domainBtn.setAttribute('aria-expanded', 'true');
    var pop = h('div', 'wp-pop', null, wrap);
    pop.addEventListener('click', function (ev) { ev.stopPropagation(); });
    var input = null;
    if (S.domains.length > 7) {
      input = h('input', 'wp-input', null, pop);
      input.type = 'search';
      input.placeholder = t('domain.search');
      input.setAttribute('aria-label', t('domain.search'));
    }
    var list = h('div', 'wp-pop-list', null, pop);
    list.setAttribute('role', 'listbox');
    function fill() {
      clear(list);
      var q = input ? input.value.trim().toLowerCase() : '';
      var lastUser = null, n = 0;
      var multiUser = S.boot.principal.type !== 'user';
      S.domains.forEach(function (d) {
        if (q && d.domain.indexOf(q) < 0 && d.user.indexOf(q) < 0) return;
        if (n >= 300) return;
        n++;
        if (multiUser && d.user !== lastUser) { h('div', 'wp-pop-group', d.user, list); lastUser = d.user; }
        var it = h('button', 'wp-pop-item', null, list);
        it.type = 'button';
        it.setAttribute('role', 'option');
        var sel = S.domain && S.domain.domain === d.domain && S.domain.user === d.user;
        it.setAttribute('aria-selected', sel ? 'true' : 'false');
        h('span', 'wp-check', sel ? '✓' : '', it);
        h('span', 'wp-key-text', d.domain, it);
        h('span', 'wp-sub', d.has_data ? fmt.num(d.visits_today) + ' today' : t('domain.nodata'), it);
        it.addEventListener('click', function () {
          closePops();
          navigate({ d: d.domain, u: multiUser ? d.user : '' });
        });
      });
      if (!n) h('div', 'wp-pop-empty', t('domain.none'), list);
    }
    fill();
    if (input) { input.addEventListener('input', fill); input.focus(); }
    else { var f = list.querySelector('.wp-pop-item'); if (f) f.focus(); }
  }

  function toggleCustomPop(anchor) {
    var wrap = S.els.customWrap;
    var open = wrap.querySelector('.wp-pop');
    closePops();
    if (open) return;
    var pop = h('div', 'wp-pop is-right wp-custom-pop', null, wrap);
    pop.addEventListener('click', function (ev) { ev.stopPropagation(); });
    var today = new Date();
    var iso = function (d) { return d.getFullYear() + '-' + String(d.getMonth() + 1).padStart(2, '0') + '-' + String(d.getDate()).padStart(2, '0'); };
    var lf = h('label', null, t('range.from'), pop);
    var from = h('input', 'wp-input', null, pop);
    from.type = 'date'; from.id = 'wp-from'; lf.htmlFor = 'wp-from';
    var lt = h('label', null, t('range.to'), pop);
    var to = h('input', 'wp-input', null, pop);
    to.type = 'date'; to.id = 'wp-to'; lt.htmlFor = 'wp-to';
    var cur = S.overview && S.overview.range;
    from.value = S.from || (cur ? cur.from : iso(new Date(today.getTime() - 29 * 864e5)));
    to.value = S.to || (cur ? cur.to : iso(today));
    to.max = iso(new Date(today.getTime() + 864e5));
    var err = h('div', 'wp-err', null, pop);
    err.hidden = true;
    var row = h('div', 'wp-row', null, pop);
    btn('is-ghost', t('range.cancel'), function () { closePops(); syncHeader(); }, row);
    btn('is-primary', t('range.apply'), function () {
      if (!from.value || !to.value || from.value > to.value) { err.textContent = t('range.invalid'); err.hidden = false; return; }
      closePops();
      navigate({ r: 'custom', f: from.value, to: to.value });
    }, row);
    from.focus();
  }

  function syncHeader() {
    var els = S.els;
    clear(els.domainBtn);
    var d = S.domain;
    var multiUser = S.boot.principal.type !== 'user';
    if (d) {
      h('span', 'wp-domain-name', d.domain, els.domainBtn);
      if (multiUser) h('span', 'wp-domain-user', d.user, els.domainBtn);
    } else {
      h('span', 'wp-domain-name', t('state.nodomains.title'), els.domainBtn);
    }
    els.domainBtn.appendChild(svgIcon('chevron'));
    els.domainBtn.disabled = S.domains.length === 0;
    Object.keys(els.rangeBtns).forEach(function (r) { els.rangeBtns[r].setAttribute('aria-pressed', S.range === r ? 'true' : 'false'); });
    if (S.range === 'custom' && S.from) els.rangeBtns.custom.textContent = dateFmt.format(parseDay(S.from)) + ' – ' + dateFmt.format(parseDay(S.to));
    else els.rangeBtns.custom.textContent = t('range.custom');
    els.rangeSelect.value = S.range;
    els.compareBtn.setAttribute('aria-pressed', S.compare ? 'true' : 'false');
    var server = S.view === 'server';
    els.seg.hidden = server; els.rangeSelect.hidden = server; els.compareBtn.hidden = server; els.tabs.hidden = server;
    if (els.serverBtn) els.serverBtn.setAttribute('aria-pressed', server ? 'true' : 'false');
    els.pdfBtn.href = location.pathname + location.search + reportHash();
    var hint = t(server ? 'report.server.hint' : 'report.button.hint');
    els.pdfBtn.title = hint;
    els.pdfBtn.setAttribute('aria-label', t('report.button') + ' — ' + hint);
    els.pdfBtn.hidden = !server && !(S.domain && S.domain.has_data);
    // tabs
    clear(els.tabs);
    availableTabs().forEach(function (tab) {
      var b = h('button', 'wp-tab', t('tab.' + tab), els.tabs);
      b.type = 'button';
      b.setAttribute('role', 'tab');
      b.setAttribute('aria-selected', S.tab === tab ? 'true' : 'false');
      b.tabIndex = S.tab === tab ? 0 : -1;
      b.addEventListener('click', function () { navigate({ t: tab }); });
      b.addEventListener('keydown', function (ev) {
        var tabs = availableTabs(), i = tabs.indexOf(tab);
        if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft') {
          ev.preventDefault();
          var nx = tabs[(i + (ev.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length];
          navigate({ t: nx });
          setTimeout(function () { var sel = els.tabs.querySelector('[aria-selected="true"]'); if (sel) sel.focus(); }, 0);
        }
      });
    });
    var selTab = els.tabs.querySelector('[aria-selected="true"]');
    if (selTab && els.tabs.scrollWidth > els.tabs.clientWidth) {
      els.tabs.scrollLeft = Math.max(0, selTab.offsetLeft - els.tabs.offsetLeft - 24);
    }
  }

  /* -------------------------------------------------------------- live */
  function startLive() {
    if (S.liveTimer) clearInterval(S.liveTimer);
    loadLive();
    S.liveTimer = setInterval(function () { if (!document.hidden) loadLive(); }, 60000);
  }
  function loadLive() {
    var el = S.els.live;
    if (S.view === 'server' || !S.domain || !S.domain.has_data) { clear(el); el.hidden = S.view === 'server'; return; }
    el.hidden = false;
    var dom = S.domain;
    api({ a: 'realtime', domain: dom.domain, user: S.boot.principal.type !== 'user' ? dom.user : null }).then(function (rt) {
      if (S.domain !== dom) return;
      clear(el);
      var badge = h('span', 'wp-live-badge', null, el);
      var dot = h('span', 'wp-live-dot' + (rt.active_visits ? '' : ' is-idle'), null, badge);
      dot.setAttribute('aria-hidden', 'true');
      h('span', null, t('live.label'), badge);
      h('span', null, rt.active_visits === 1 ? t('live.active1') : rt.active_visits ? t('live.active', { n: fmt.num(rt.active_visits) }) : t('live.none'), el);
      var pulse = h('span', 'wp-pulse', null, el);
      pulse.title = t('live.pv', { n: fmt.num(rt.pageviews_30m) });
      C.sparkline(pulse, rt.per_minute, { height: 22, color: 'var(--wp-good)', dot: false });
      h('span', 'wp-live-sep', '·', el);
      h('span', null, rt.last_collect_at ? t('live.collected', { ago: ago(rt.last_collect_at) }) : t('live.never'), el);
      info(I.def('realtime'), el);
    }).catch(function () { clear(el); });
  }

  /* --------------------------------------------------------------- hero */
  function metricSeries(series, m) {
    if (!series) return null;
    var n = series.labels.length;
    var div = function (a, b, f) {
      if (!a || !b) return null;
      var out = new Array(n);
      for (var i = 0; i < n; i++) out[i] = a[i] == null || b[i] == null ? null : (b[i] > 0 ? f(a[i], b[i]) : null);
      return out;
    };
    switch (m) {
      case 'errors':
        if (!series.errors_4xx) return null;
        return series.errors_4xx.map(function (v, i) { return v == null ? null : v + (series.errors_5xx[i] || 0); });
      case 'pages_per_visit': return div(series.pageviews, series.visits, function (a, b) { return a / b; });
      case 'avg_duration': return div(series.duration, series.visits, function (a, b) { return a / b; });
      case 'bounce_rate': return div(series.bounces, series.visits, function (a, b) { return a / b; });
      case 'bot_share':
        if (!series.bot_hits || !series.hits) return null;
        return series.bot_hits.map(function (v, i) { var tot = v + (series.hits[i] || 0); return tot > 0 ? v / tot : null; });
      default: return series[m] || null;
    }
  }

  function renderKpis(ov) {
    var wrap = S.els.kpis;
    clear(wrap);
    KPIS.forEach(function (k) {
      var kv = ov.kpi[k.key] || { value: 0, prev: 0, change: null };
      var tile = h('button', 'wp-kpi', null, wrap);
      tile.type = 'button';
      var selected = S.metrics.length === 1 && S.metrics[0] === k.metric;
      tile.setAttribute('aria-pressed', selected ? 'true' : 'false');
      var lab = h('span', 'wp-kpi-label', t('kpi.' + k.key), tile);
      var val = C.byUnit(k.unit, kv.value);
      h('span', 'wp-kpi-value', val, tile);
      var d = h('span', 'wp-kpi-delta', null, tile);
      var ch = kv.change;
      var desc;
      if (ch == null) {
        d.classList.add('is-none');
        d.textContent = kv.prev ? '–' : t('kpi.noprev');
        desc = t('kpi.noprev');
      } else if (Math.abs(ch) < 0.01) {
        d.classList.add('is-flat');
        d.textContent = '± ' + fmt.signedPct(ch).replace(/^[+−]/, '');
        desc = fmt.signedPct(ch) + ' ' + t('kpi.vs');
      } else {
        var up = ch > 0;
        var good = k.inverse ? !up : up;
        d.classList.add(good ? 'is-good' : 'is-bad');
        d.textContent = (up ? '▲ ' : '▼ ') + fmt.signedPct(ch).replace(/^[+−]/, '');
        desc = fmt.signedPct(ch) + ' ' + t('kpi.vs');
      }
      d.title = t('kpi.vs') + ': ' + C.byUnit(k.unit, kv.prev);
      tile.setAttribute('aria-label', t('kpi.' + k.key) + ': ' + val + ', ' + desc + '. ' + I.def(k.def));
      tile.title = I.def(k.def);
      var sp = h('div', 'wp-kpi-spark', null, tile);
      var vals = metricSeries(ov.series, k.metric);
      if (vals && vals.some(function (v) { return v != null; })) {
        var pts = vals.length > 60 ? vals.slice(-60) : vals;
        C.sparkline(sp, pts, { height: 26, color: selected ? 'var(--wp-accent)' : 'var(--wp-spark)', dotColor: 'var(--wp-accent)', zero: k.unit === 'count' || k.unit === 'bytes' });
      }
      tile.addEventListener('click', function () { navigate({ m: k.metric }); });
      lab.setAttribute('aria-hidden', 'true');
    });
  }

  function renderMainChart(ov) {
    var head = S.els.chartHead, box = S.els.chart;
    clear(head);
    var g = ov.range.granularity;
    var chips = h('div', 'wp-chips', null, head);
    chips.setAttribute('role', 'group');
    chips.setAttribute('aria-label', t('chart.metrics'));
    var selUnit = METRICS[S.metrics[0]].unit;
    Object.keys(METRICS).forEach(function (m) {
      if (!METRICS[m].chip) return;
      var avail = metricSeries(ov.series, m) !== null;
      var on = S.metrics.indexOf(m) >= 0;
      var c = h('button', 'wp-chip', null, chips);
      c.type = 'button';
      c.setAttribute('aria-pressed', on ? 'true' : 'false');
      c.disabled = !avail;
      var key = h('span', 'wp-chip-key', null, c);
      key.style.setProperty('--k', METRICS[m].color);
      h('span', null, t('metric.' + m), c);
      c.addEventListener('click', function () {
        var cur = S.metrics.slice();
        var unit = METRICS[m].unit;
        if (unit !== selUnit || !METRICS[cur[0]].chip) cur = [m];
        else if (on) { if (cur.length > 1) cur.splice(cur.indexOf(m), 1); }
        else cur.push(m);
        navigate({ m: cur.join(',') });
      });
    });
    var meta = h('div', 'wp-chart-meta', null, head);
    h('span', null, t('chart.granularity.' + g), meta);
    if (S.compare && ov.series.prev) {
      var lp = h('span', 'wp-legend-inline', null, meta);
      var kl = h('span', 'wp-key-line is-dashed', null, lp);
      kl.style.setProperty('--k', 'var(--wp-prev)');
      h('span', null, dateFmt.format(parseDay(ov.range.prev.from)) + ' – ' + dateFmt.format(parseDay(ov.range.prev.to)), lp);
    }

    var series = [];
    S.metrics.forEach(function (m) {
      var vals = metricSeries(ov.series, m);
      if (!vals) return;
      var single = S.metrics.length === 1;
      series.push({ name: t('metric.' + m), values: vals, color: single ? 'var(--wp-s1)' : METRICS[m].color, area: series.length === 0, unit: METRICS[m].unit });
    });
    if (S.compare && ov.series.prev) {
      S.metrics.forEach(function (m, i) {
        var pv = metricSeries(ov.series.prev, m);
        if (!pv) return;
        var n = ov.series.labels.length;
        pv = pv.slice(0, n);
        while (pv.length < n) pv.push(null);
        series.push({ name: t('chart.prev', { name: t('metric.' + m) }), values: pv, color: S.metrics.length === 1 ? 'var(--wp-prev)' : METRICS[m].color, dashed: true, unit: METRICS[m].unit });
      });
    }
    var holder = h('div', null, null, null);
    clear(box);
    box.appendChild(holder);
    var w = box.clientWidth;
    C.line(holder, {
      labels: ov.series.labels, series: series, unit: METRICS[S.metrics[0]].unit, height: w < 500 ? 210 : 260,
      weekend: g === 'day', xTick: function (l) { return bucketTick(l, g); }, title: function (l) { return bucketTitle(l, g); },
      caption: S.metrics.map(function (m) { return t('metric.' + m); }).join(', ') + ' — ' + t('chart.granularity.' + g),
      xName: t('chart.granularity.' + g)
    });
  }

  /* --------------------------------------------------------- components */
  function card(grid, o) {
    var el = h('section', 'wp-card' + (o.span ? ' is-' + o.span : ''), null, grid);
    var head = h('div', 'wp-card-head', null, el);
    var tw = h('div', null, null, head);
    var title = h('h3', 'wp-card-title', null, tw);
    h('span', null, o.title, title);
    if (o.info) info(o.info, title);
    var sub = h('div', 'wp-card-sub', o.sub || '', tw);
    sub.hidden = !o.sub;
    var actions = h('div', 'wp-card-actions', null, head);
    if (o.exportDim) exportLink(actions, o.exportDim, o.title);
    var body = h('div', 'wp-card-body', null, el);
    var foot = h('div', 'wp-card-foot', null, el);
    foot.hidden = true;
    el.setAttribute('aria-label', o.title);
    return { el: el, body: body, foot: foot, actions: actions, sub: sub };
  }

  function exportLink(parent, dim, title) {
    var a = h('a', 'wp-btn is-small is-ghost', null, parent);
    a.href = apiHref(scoped({ a: 'export', name: dim, format: 'csv', q: S.q || null }));
    a.setAttribute('download', '');
    a.appendChild(svgIcon('download'));
    h('span', null, 'CSV', a);
    a.title = t('table.export.title');
    a.setAttribute('aria-label', t('table.export') + ' — ' + title);
    // a navigation would leave the Cockpit frame: fetch through the transport, save a Blob
    if (S.fetchFn) a.addEventListener('click', function (ev) { ev.preventDefault(); downloadVia(a.href); });
    return a;
  }
  function downloadVia(url) {
    S.fetchFn(url, { headers: { Accept: 'text/csv' } }).then(function (res) {
      return res.text().then(function (txt) {
        if (res.status !== 200) {
          var j = null;
          try { j = JSON.parse(txt); } catch (e) { j = null; }
          throw new Error(j && j.error ? j.error.message : 'HTTP ' + res.status);
        }
        var cd = res.headers.get('Content-Disposition') || '';
        var m = /filename="?([^";]+)"?/i.exec(cd);
        var blob = new Blob([txt], { type: res.headers.get('Content-Type') || 'text/csv' });
        var href = URL.createObjectURL(blob);
        var tmp = h('a', null, null, document.body);
        tmp.href = href;
        tmp.download = m ? m[1] : 'webpulse.csv';
        tmp.hidden = true;
        tmp.click();
        tmp.parentNode.removeChild(tmp);
        setTimeout(function () { URL.revokeObjectURL(href); }, 30000);
      });
    }).catch(function (e) {
      if (S.els && S.els.live) S.els.live.textContent = e.message;
    });
  }

  /** Runs a promise for a card: holds the frame while loading, renders or shows an error. */
  function load(c, promiseFn, renderFn) {
    var token = S.seq;
    c.el.classList.add('is-loading');
    if (!c.body.firstChild) h('div', 'wp-empty', t('common.loading'), c.body);
    promiseFn().then(function (data) {
      if (token !== S.seq) return;
      c.el.classList.remove('is-loading');
      clear(c.body);
      renderFn(data);
    }).catch(function (err) {
      if (token !== S.seq) return;
      c.el.classList.remove('is-loading');
      clear(c.body);
      var e = h('div', 'wp-card-error', null, c.body);
      h('span', null, err.message || t('state.error.title'), e);
      btn('is-small', t('state.retry'), function () { load(c, promiseFn, renderFn); }, e);
    });
  }

  function report(dim, extra) {
    var p = { a: 'report', name: dim };
    for (var k in extra) p[k] = extra[k];
    if (S.compare) p.compare = 1;
    return api(scoped(p));
  }

  var COLS = {
    visits: { label: 'metric.visits', unit: 'count' },
    pageviews: { label: 'metric.pageviews', unit: 'count' },
    hits: { label: 'metric.hits', unit: 'count' },
    bytes: { label: 'metric.bytes', unit: 'bytes' },
    share: { label: 'metric.share', unit: 'ratio', nosort: true, def: 'share' },
    exit_rate: { label: 'metric.exit_rate', unit: 'ratio', nosort: true, def: 'exit_rate' },
    bounce_rate: { label: 'metric.bounce_rate', unit: 'ratio', def: 'bounce_rate' },
    entry_bounce: { field: 'bounce_rate', label: 'metric.bounce_rate', unit: 'ratio', minibar: true, sort: 'bounce_rate', def: 'entry_bounce' },
    avg_duration: { label: 'metric.avg_duration', unit: 'duration' },
    entries: { field: 'visits', label: 'metric.entries', unit: 'count', sort: 'visits' },
    exits: { field: 'visits', label: 'metric.exits', unit: 'count', sort: 'visits' },
    last_day: { label: 'metric.last_seen', unit: 'day', nosort: true }
  };

  function cellValue(col, row) {
    var f = col.field || col.key;
    var v = row[f];
    if (col.unit === 'day') return v ? dateFmt.format(parseDay(v)) : '–';
    return C.byUnit(col.unit, v);
  }

  function keyContent(wrap, dim, row) {
    var meta = S.boot.dims[dim] || {};
    var pretty = meta.pretty;
    var k = row.key;
    var text = row.label || k;
    if (k === '(other)') { h('span', 'wp-key-text', t('other'), wrap); return; }
    if (pretty === 'country') {
      flagEl(k, wrap);
      text = countryName(k, row.label);
    } else if (pretty === 'status') {
      var cls = 's' + String(k).charAt(0) + 'xx';
      h('span', 'wp-badge ' + cls, k, wrap);
      text = (row.label || '').replace(/^\d+\s*/, '');
    } else if (pretty === 'channel') {
      var sw = h('span', 'wp-key-swatch', null, wrap);
      sw.style.setProperty('--k', channelColor(k));
    } else if (pretty === 'device') {
      var ic = svgIcon(DEVICES.indexOf(k) >= 0 ? k : 'other');
      ic.setAttribute('width', '15'); ic.setAttribute('height', '15');
      wrap.appendChild(ic);
    }
    var span = h('span', 'wp-key-text' + (pretty === 'path' || pretty === 'url' ? ' is-path' : ''), text, wrap);
    span.title = row.label && row.label !== k ? text + ' (' + k + ')' : k;
    if (dim === 'bot' && AI_BOTS.test(k)) h('span', 'wp-badge is-ai', t('ai.badge'), wrap);
  }

  function channelColor(k) {
    var i = CHANNELS.indexOf(k);
    return i >= 0 ? C.color(i) : 'var(--wp-other)';
  }
  function deviceColor(k) {
    var i = DEVICES.indexOf(k);
    return i >= 0 ? C.color(i) : 'var(--wp-other)';
  }

  /**
   * Ranked table with wash bars. o: {dim, cols:[colKey], barBy, sort, details(row, cell)}
   */
  function rankTable(parent, data, o) {
    var rows = data.rows || [];
    if (!rows.length) { h('div', 'wp-empty', o.empty || t('table.empty'), parent); return null; }
    var cols = o.cols.map(function (k) { var c = COLS[k]; var cc = {}; for (var x in c) cc[x] = c[x]; cc.key = k; return cc; });
    var barBy = o.barBy || (S.boot.dims[o.dim] || {}).share || 'visits';
    var max = 0;
    rows.forEach(function (r) { if ((r[barBy] || 0) > max) max = r[barBy]; });
    var tbl = h('table', 'wp-table', null, parent);
    var colgroup = h('colgroup', null, null, tbl);
    h('col', null, null, colgroup);
    cols.forEach(function (c) { var cg = h('col', c.optional ? 'is-optional' : null, null, colgroup); cg.style.width = c.unit === 'count' ? '72px' : '92px'; });
    var trh = h('tr', null, null, h('thead', null, null, tbl));
    var th0 = h('th', null, o.keyLabel || (S.boot.dims[o.dim] || {}).label || t('table.key'), trh);
    th0.scope = 'col';
    var curSort = o.sort || (S.boot.dims[o.dim] || {}).sort || '';
    cols.forEach(function (c, ci) {
      var th = h('th', ci >= 2 ? 'is-optional' : null, t(c.label), trh);
      th.scope = 'col';
      if (c.def) th.title = I.def(c.def);
      var sortKey = c.sort || c.key;
      if (o.onSort && !c.nosort) {
        th.classList.add('is-sortable');
        th.tabIndex = 0;
        if (curSort === '-' + sortKey) th.setAttribute('aria-sort', 'descending');
        else if (curSort === sortKey) th.setAttribute('aria-sort', 'ascending');
        var act = function () { o.onSort(curSort === '-' + sortKey ? sortKey : '-' + sortKey); };
        th.addEventListener('click', act);
        th.addEventListener('keydown', function (ev) { if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); act(); } });
      }
    });
    var tb = h('tbody', null, null, tbl);
    rows.forEach(function (r) {
      var tr = h('tr', null, null, tb);
      var td0 = h('td', null, null, tr);
      var key = h('div', 'wp-key', null, td0);
      var bar = h('span', 'wp-key-bar', null, key);
      bar.style.width = (max > 0 ? Math.max(0.5, (r[barBy] || 0) / max * 100) : 0) + '%';
      var holder = key;
      if (o.details) {
        var b = h('button', 'wp-key-btn', null, key);
        b.type = 'button';
        b.setAttribute('aria-expanded', 'false');
        h('span', 'wp-caret', '▶', b).setAttribute('aria-hidden', 'true');
        holder = b;
        var detailRow = null;
        b.addEventListener('click', function () {
          var open = b.getAttribute('aria-expanded') === 'true';
          b.setAttribute('aria-expanded', open ? 'false' : 'true');
          if (open) { if (detailRow) detailRow.hidden = true; return; }
          if (!detailRow) {
            detailRow = document.createElement('tr');
            detailRow.className = 'wp-detail';
            tr.parentNode.insertBefore(detailRow, tr.nextSibling);
            var td = h('td', null, null, detailRow);
            td.colSpan = cols.length + 1;
            o.details(r, td);
          }
          detailRow.hidden = false;
        });
      }
      keyContent(holder, o.dim, r);
      cols.forEach(function (c, ci) {
        var td = h('td', ci >= 2 ? 'is-optional' : null, cellValue(c, r), tr);
        if (c.minibar) {
          var mb = h('span', 'wp-minibar', null, td);
          mb.setAttribute('aria-hidden', 'true');
          h('i', null, null, mb).style.width = Math.round((r[c.field || c.key] || 0) * 100) + '%';
        }
        if (r.prev && (c.key === barBy || c.field === barBy || ci === 0)) {
          var f = c.field || c.key;
          var pv = r.prev[f];
          if (pv != null) {
            var chg = pv > 0 ? (r[f] - pv) / pv : null;
            td.title = t('kpi.vs') + ': ' + C.byUnit(c.unit, pv) + (chg != null ? ' (' + fmt.signedPct(chg) + ')' : '');
          }
        }
      });
    });
    return tbl;
  }

  /**
   * A card showing one dimension as a ranked table, with sort, "show more",
   * CSV export and optional extras rendered above the table.
   */
  function reportCard(grid, o) {
    // On paper every top list shows 25 rows, and tables with 3+ columns get the full width.
    var span = S.report && o.cols.length > 2 ? 'wide' : o.span;
    var c = card(grid, { title: o.title, sub: o.sub, info: o.info, span: span, exportDim: o.dim });
    var st = { limit: S.report ? Math.max(25, o.limit || 10) : o.limit || 10, sort: o.sort || '' };
    function fetchIt() {
      var p = { limit: st.limit };
      if (st.sort) p.sort = st.sort;
      if (o.filter && S.q) p.q = S.q;
      if (o.extra) for (var k in o.extra) p[k] = o.extra[k];
      return report(o.dim, p);
    }
    function draw(data) {
      if (o.before) o.before(c.body, data);
      rankTable(c.body, data, { dim: o.dim, cols: o.cols, barBy: o.barBy, sort: st.sort, details: o.details, empty: o.empty, keyLabel: o.keyLabel,
        onSort: o.nosort ? null : function (s) { st.sort = s; run(); } });
      if (o.after) o.after(c.body, data);
      clear(c.foot);
      c.foot.hidden = !data.has_more && !data.total.rows;
      if (data.total && data.total.rows) h('span', null, fmt.num(data.total.rows) + ' ' + (data.total.rows === 1 ? 'row' : 'rows'), c.foot);
      if (data.has_more && st.limit < 500) {
        btn('is-small', t('table.more'), function () { st.limit = Math.min(500, st.limit < 25 ? 25 : st.limit * 2); run(); }, c.foot);
      }
    }
    function run() { load(c, fetchIt, draw); }
    run();
    return c;
  }

  function legendList(parent, items, unit) {
    var ul = h('ul', 'wp-legend', null, parent);
    items.forEach(function (it) {
      var li = h('li', null, null, ul);
      var sw = h('span', 'wp-sw' + (it.line ? ' is-line' : '') + (it.dashed ? ' is-dashed' : ''), null, li);
      sw.style.setProperty('--k', it.color);
      h('span', null, it.label, li);
      if (it.value != null) h('strong', null, C.byUnit(unit || 'count', it.value), li);
    });
    return ul;
  }

  function weekdayNames() {
    var f = new Intl.DateTimeFormat(undefined, { weekday: 'short' });
    var out = [];
    for (var i = 0; i < 7; i++) out.push(f.format(new Date(2024, 0, 1 + i))); // 2024-01-01 is a Monday
    return out;
  }
  function hourLabel(hh) { return String(hh).padStart(2, '0') + ':00'; }

  /* ------------------------------------------------------------- tabs */
  function grid(parent) { return h('div', 'wp-grid', null, parent); }

  function tabOverview(root, ov) {
    var g = grid(root);
    var tz = ov.meta.timezone;
    var when = card(g, { title: t('card.when'), info: I.def('heatmap'), span: 'seven',
      sub: t(ov.range.days >= 7 ? 'card.when.sub' : 'card.hours.sub', { tz: tz }) });
    var hb = h('div', null, null, when.body);
    if (ov.range.days >= 7) {
      var max = 0;
      ov.heatmap.forEach(function (r) { r.forEach(function (v) { if (v > max) max = v; }); });
      var hours = [];
      for (var i = 0; i < 24; i++) hours.push(i);
      var wd = weekdayNames();
      C.heatmap(hb, { rows: wd, cols: hours, values: ov.heatmap, name: t('metric.visits'), caption: t('card.when'),
        colTick: function (c) { return String(c).padStart(2, '0'); },
        title: function (r, c) { return wd[r] + ' ' + hourLabel(c) + '–' + hourLabel((c + 1) % 24); } });
      C.seqLegend(when.body, fmt.compact(max));
    } else {
      var labs = [];
      for (var j = 0; j < 24; j++) labs.push(j);
      var peak = ov.hours.visits.indexOf(Math.max.apply(null, ov.hours.visits));
      C.bars(hb, { labels: labs, values: ov.hours.visits, name: t('metric.visits'), caption: t('card.when'), height: 190, highlight: peak,
        xTick: function (l) { return String(l).padStart(2, '0'); }, title: function (l) { return hourLabel(l) + '–' + hourLabel((l + 1) % 24); }, xName: 'Hour' });
    }

    var ch = card(g, { title: t('card.channels'), info: I.def('channel'), span: 'five', exportDim: 'channel' });
    load(ch, function () { return report('channel', { limit: 10 }); }, function (data) { channelBlock(ch.body, data); });

    reportCard(g, { dim: 'page', title: t('card.top_pages'), cols: ['pageviews', 'share'], limit: 8, nosort: true });
    reportCard(g, { dim: 'referrer_host', title: t('card.referrers'), cols: ['visits', 'share'], limit: 8, nosort: true });
    countriesCard(g, 'third', 8, ['visits', 'share']);

    var dev = card(g, { title: t('card.devices'), span: 'third', exportDim: 'device' });
    load(dev, function () { return report('device', { limit: 10 }); }, function (data) {
      if (!data.rows.length) { h('div', 'wp-empty', t('table.empty'), dev.body); return; }
      var items = data.rows.map(function (r) { return { label: r.label || r.key, value: r.visits, color: deviceColor(r.key) }; });
      var sb = h('div', null, null, dev.body);
      C.stackBar(sb, { items: items, caption: t('card.devices'), height: 16 });
      var tot = data.total.visits || 1;
      var ul = h('ul', 'wp-legend', null, dev.body);
      ul.style.flexDirection = 'column';
      items.forEach(function (it) {
        var li = h('li', null, null, ul);
        h('span', 'wp-sw', null, li).style.setProperty('--k', it.color);
        h('span', null, it.label, li);
        h('strong', null, fmt.pct(it.value / tot), li);
      });
      ul.style.flexDirection = 'row';
      ul.style.marginBottom = '14px';
      rankTable(dev.body, data, { dim: 'device', cols: ['visits', 'bounce_rate', 'avg_duration'] });
    });

    engagementCard(g, 'third', true);
  }

  function channelBlock(body, data) {
    if (!data.rows.length) { h('div', 'wp-empty', t('table.empty'), body); return; }
    var wrap = h('div', 'wp-donut-wrap', null, body);
    var dh = h('div', 'wp-donut-holder', null, wrap);
    C.donut(dh, { items: data.rows.map(function (r) { return { label: r.label || r.key, value: r.visits, color: channelColor(r.key) }; }),
      caption: t('card.channels'), size: 148, center: { value: fmt.compact(data.total.visits), label: t('metric.visits') } });
    var tbl = h('table', 'wp-table is-auto', null, wrap);
    var trh = h('tr', null, null, h('thead', null, null, tbl));
    h('th', null, t('card.channels'), trh).scope = 'col';
    h('th', null, t('metric.visits'), trh).scope = 'col';
    var bth = h('th', null, t('metric.bounce_rate'), trh);
    bth.scope = 'col'; bth.title = I.def('bounce_rate');
    var tb = h('tbody', null, null, tbl);
    data.rows.forEach(function (r) {
      var tr = h('tr', null, null, tb);
      var k = h('div', 'wp-key', null, h('td', null, null, tr));
      keyContent(k, 'channel', r);
      h('td', null, fmt.num(r.visits), tr);
      h('td', null, fmt.pct(r.bounce_rate), tr);
    });
  }

  function countriesCard(g, span, limit, cols) {
    return reportCard(g, { dim: 'country', title: t('card.countries'), info: I.def('country'), cols: cols, limit: limit, span: span, nosort: span === 'third',
      after: function (body, data) {
        var unknown = data.rows.length > 0 && data.rows.every(function (r) { return r.key.charAt(0) === '(' || r.key === ''; });
        if (unknown) h('p', 'wp-hint', t('geo.missing'), body);
      } });
  }

  function engagementCard(g, span, stacked) {
    var c = card(g, { title: t('card.engagement'), info: I.def('engagement'), span: span });
    load(c, function () {
      return Promise.all([report('visit_duration', { limit: 20 }), report('pages_per_visit', { limit: 20 })]);
    }, function (res) {
      var two = h('div', stacked ? null : 'wp-two', null, c.body);
      [['card.engagement.duration', res[0]], ['card.engagement.pages', res[1]]].forEach(function (p, idx) {
        var sec = h('div', null, null, two);
        if (stacked && idx) sec.style.marginTop = '12px';
        h('h4', null, t(p[0]), sec);
        var rows = p[1].rows;
        if (!rows.length) { h('div', 'wp-empty', t('table.empty'), sec); return; }
        var holder = h('div', null, null, sec);
        var labs = rows.map(function (r) { return r.label || r.key; });
        C.bars(holder, { labels: rows.map(function (r) { return r.key; }), values: rows.map(function (r) { return r.visits; }),
          name: t('metric.visits'), caption: t(p[0]), height: stacked ? 120 : 160,
          xTick: function (l) { return l.replace('-', '–'); }, title: function (l, i) { return labs[i]; }, xName: t(p[0]) });
      });
    });
  }

  function tabPages(root) {
    var srch = h('div', 'wp-search', null, root);
    var inp = h('input', 'wp-input', null, srch);
    inp.type = 'search';
    inp.placeholder = t('search.placeholder');
    inp.setAttribute('aria-label', t('search.label'));
    inp.value = S.q || '';
    var g = grid(root);
    function draw() {
      clear(g);
      reportCard(g, { dim: 'page', title: t('card.pages'), cols: ['pageviews', 'visits', 'bytes', 'exit_rate'], span: 'wide', filter: true });
      reportCard(g, { dim: 'entry_page', title: t('card.entry'), cols: ['entries', 'entry_bounce'], filter: true });
      reportCard(g, { dim: 'exit_page', title: t('card.exit'), cols: ['exits', 'exit_rate'], filter: true });
      reportCard(g, { dim: 'download', title: t('card.downloads'), cols: ['hits', 'bytes'], span: 'wide', filter: true });
    }
    var timer = 0;
    inp.addEventListener('input', function () {
      clearTimeout(timer);
      timer = setTimeout(function () { S.q = inp.value.trim(); S.seq++; draw(); }, 300);
    });
    draw();
  }

  function tabAcquisition(root) {
    var g = grid(root);
    var tr = card(g, { title: t('card.channel_trend'), info: I.def('channel'), span: 'wide', exportDim: 'channel' });
    load(tr, function () { return report('channel', { series: 1, limit: 10 }); }, function (data) {
      var se = data.series;
      if (!se || !se.keys.length) { h('div', 'wp-empty', t('table.empty'), tr.body); return; }
      var series = se.keys.map(function (k, i) { return { name: se.key_labels[i], values: se.values[i], color: channelColor(k) }; });
      var holder = h('div', null, null, tr.body);
      C.line(holder, { labels: se.labels, series: series, stacked: true, height: 240, weekend: se.granularity === 'day',
        xTick: function (l) { return bucketTick(l, se.granularity); }, title: function (l) { return bucketTitle(l, se.granularity); },
        caption: t('card.channel_trend'), xName: t('chart.granularity.' + se.granularity) });
      legendList(tr.body, series.map(function (s, i) { return { label: s.name, color: s.color, value: s.values.reduce(function (a, b) { return a + b; }, 0) }; }));
    });
    reportCard(g, { dim: 'referrer_host', title: t('card.ref_hosts'), cols: ['visits', 'bounce_rate', 'avg_duration'] });
    reportCard(g, { dim: 'referrer_url', title: t('card.ref_urls'), cols: ['visits', 'share'] });
    reportCard(g, { dim: 'search_engine', title: t('card.engines'), cols: ['visits', 'bounce_rate', 'share'],
      before: function (body, data) {
        if (!data.rows.length) return;
        var wrap = h('div', 'wp-donut-wrap', null, body);
        wrap.style.marginBottom = '12px';
        var top = data.rows.slice(0, 6);
        var rest = data.total.visits - top.reduce(function (a, r) { return a + r.visits; }, 0);
        var items = top.map(function (r, i) { return { label: r.label || r.key, value: r.visits, color: C.color(i) }; });
        if (rest > 0) items.push({ label: t('other'), value: rest, color: 'var(--wp-other)' });
        C.donut(h('div', 'wp-donut-holder', null, wrap), { items: items, size: 120, caption: t('card.engines'), center: { value: fmt.compact(data.total.visits), label: t('metric.visits') } });
        legendList(wrap, items);
      } });
    reportCard(g, { dim: 'keyword', title: t('card.keywords'), cols: ['visits', 'share'], empty: t('keywords.note'),
      after: function (body, data) { if (data.rows.length) h('p', 'wp-hint', t('keywords.note'), body); } });
    reportCard(g, { dim: 'campaign', title: t('card.campaigns'), cols: ['visits', 'bounce_rate', 'avg_duration'] });
    reportCard(g, { dim: 'source_medium', title: t('card.source_medium'), cols: ['visits', 'pageviews', 'share'] });
  }

  function versionDetails(dim) {
    return function (row, td) {
      var hold = h('div', null, t('common.loading'), td);
      report(dim, { q: row.key, limit: 10 }).then(function (data) {
        clear(hold);
        var rows = data.rows.filter(function (r) { return r.key.indexOf(row.key) === 0; });
        if (!rows.length) { h('div', null, t('table.empty'), hold); return; }
        var ul = h('ul', null, null, hold);
        rows.forEach(function (r) {
          var li = h('li', null, null, ul);
          h('span', null, r.key, li);
          h('span', null, fmt.num(r.visits) + ' · ' + fmt.pct(row.visits ? r.visits / row.visits : 0), li);
        });
      }).catch(function (e) { hold.textContent = e.message; });
    };
  }

  function tabAudience(root) {
    var g = grid(root);
    countriesCard(g, null, 15, ['visits', 'bounce_rate', 'avg_duration']);
    reportCard(g, { dim: 'city', title: t('card.cities'), cols: ['visits', 'share'], empty: t('cities.missing'), limit: 15 });
    reportCard(g, { dim: 'browser', title: t('card.browsers'), cols: ['visits', 'bounce_rate', 'share'], details: versionDetails('browser_version') });
    reportCard(g, { dim: 'os', title: t('card.os'), cols: ['visits', 'bounce_rate', 'share'], details: versionDetails('os_version') });
    var dc = card(g, { title: t('card.device_types'), exportDim: 'device' });
    load(dc, function () { return report('device', { limit: 10 }); }, function (data) {
      if (!data.rows.length) { h('div', 'wp-empty', t('table.empty'), dc.body); return; }
      var wrap = h('div', 'wp-donut-wrap', null, dc.body);
      C.donut(h('div', 'wp-donut-holder', null, wrap), { items: data.rows.map(function (r) { return { label: r.label || r.key, value: r.visits, color: deviceColor(r.key) }; }),
        size: 148, caption: t('card.device_types'), center: { value: fmt.compact(data.total.visits), label: t('metric.visits') } });
      rankTable(wrap, data, { dim: 'device', cols: ['visits', 'bounce_rate', 'avg_duration'] });
    });
    engagementCard(g, null, false);
  }

  function health() { return api(scoped({ a: 'health' })); }

  function rowsCard(g, o, getRows) {
    var c = card(g, { title: o.title, sub: o.sub, info: o.info, span: o.span, exportDim: o.dim });
    load(c, health, function (hd) {
      var rows = getRows(hd);
      if (o.before && rows.length) o.before(c.body, rows, hd);
      rankTable(c.body, { rows: rows }, { dim: o.dim, cols: o.cols, barBy: o.barBy, details: o.details, empty: o.empty });
    });
    return c;
  }

  var STATUS_CLASS = { '2': 'var(--wp-good)', '3': 'var(--wp-s1)', '4': 'var(--wp-warn)', '5': 'var(--wp-bad)' };

  function tabTechnical(root) {
    var g = grid(root);
    rowsCard(g, { title: t('card.status'), dim: 'status', cols: ['hits', 'share'], span: 'third', barBy: 'hits',
      before: function (body, rows) {
        var cls = {};
        rows.forEach(function (r) { var k = String(r.key).charAt(0); cls[k] = (cls[k] || 0) + r.hits; });
        var items = Object.keys(cls).sort().map(function (k) { return { label: k + 'xx', value: cls[k], color: STATUS_CLASS[k] || 'var(--wp-other)' }; });
        var wrap = h('div', 'wp-donut-wrap', null, body);
        wrap.style.marginBottom = '12px';
        var tot = items.reduce(function (a, b) { return a + b.value; }, 0);
        C.donut(h('div', 'wp-donut-holder', null, wrap), { items: items, size: 128, caption: t('card.status'), center: { value: fmt.compact(tot), label: t('metric.hits') } });
        legendList(wrap, items);
      } }, function (hd) { return hd.status.slice(0, 10); });

    var er = card(g, { title: t('card.errors_trend'), info: I.def('errors'), span: 'two-thirds' });
    load(er, health, function (hd) {
      var es = hd.errors_series;
      var g2 = hd.range.granularity === 'hour' ? 'day' : hd.range.granularity;
      var series = [
        { name: t('metric.errors_4xx'), values: es.errors_4xx, color: 'var(--wp-s2)' },
        { name: t('metric.errors_5xx'), values: es.errors_5xx, color: 'var(--wp-s8)' }
      ];
      C.line(h('div', null, null, er.body), { labels: es.labels, series: series, height: 220, weekend: g2 === 'day',
        xTick: function (l) { return bucketTick(l, g2); }, title: function (l) { return bucketTitle(l, g2); }, caption: t('card.errors_trend') });
      legendList(er.body, series.map(function (s) { return { label: s.name, color: s.color, line: true, value: s.values.reduce(function (a, b) { return a + (b || 0); }, 0) }; }));
    });

    rowsCard(g, { title: t('card.not_found'), info: I.def('not_found'), dim: 'not_found', cols: ['hits', 'share'],
      details: function (row, td) {
        if (!row.referrers || !row.referrers.length) { h('div', null, t('table.noreferrers'), td); return; }
        h('div', null, t('table.referrers'), td).style.fontWeight = '600';
        var ul = h('ul', null, null, td);
        row.referrers.forEach(function (r) { var li = h('li', null, null, ul); h('span', null, r.key, li); h('span', null, fmt.num(r.hits), li); });
      } }, function (hd) { return hd.not_found; });
    rowsCard(g, { title: t('card.server_error'), dim: 'server_error', cols: ['hits', 'share'] }, function (hd) { return hd.server_error; });

    rowsCard(g, { title: t('card.file_types'), dim: 'file_type', cols: ['bytes', 'hits', 'share'], barBy: 'bytes',
      before: function (body, rows) {
        var top = rows.slice(0, 6);
        var rest = rows.slice(6).reduce(function (a, r) { return a + r.bytes; }, 0);
        var items = top.map(function (r, i) { return { label: r.key, value: r.bytes, color: C.color(i) }; });
        if (rest > 0) items.push({ label: t('other'), value: rest, color: 'var(--wp-other)' });
        C.stackBar(h('div', null, null, body), { items: items, unit: 'bytes', caption: t('card.file_types'), height: 16 });
        var lg = legendList(body, items, 'bytes');
        lg.style.marginBottom = '12px';
      } }, function (hd) { return hd.file_type.slice(0, 12); });
    rowsCard(g, { title: t('card.static'), info: I.def('static'), dim: 'static_file', cols: ['bytes', 'hits'], barBy: 'bytes' }, function (hd) { return hd.static_file; });

    rowsCard(g, { title: t('card.protocol'), dim: 'protocol', cols: ['hits', 'share'], span: 'quarter',
      before: function (body, rows) {
        C.stackBar(h('div', null, null, body), { items: rows.map(function (r, i) { return { label: r.label || r.key, value: r.hits, color: C.color(i) }; }), caption: t('card.protocol'), height: 12 });
        h('div', null, null, body).style.height = '10px';
      } }, function (hd) { return hd.protocol; });
    rowsCard(g, { title: t('card.http_version'), dim: 'http_version', cols: ['hits', 'share'], span: 'quarter' }, function (hd) { return hd.http_version; });
    rowsCard(g, { title: t('card.methods'), dim: 'method', cols: ['hits', 'share'], span: 'quarter' }, function (hd) { return hd.method; });
    var pk = card(g, { title: t('card.peak'), span: 'quarter' });
    load(pk, health, function (hd) {
      if (!hd.peak) { h('div', 'wp-empty', t('table.empty'), pk.body); return; }
      h('div', 'wp-stat', hourLabel(hd.peak.hour), pk.body);
      h('div', 'wp-stat-sub', t('peak.at', { hits: fmt.num(hd.peak.hits), day: dateFmtLong.format(parseDay(hd.peak.day)), hour: hourLabel(hd.peak.hour) }), pk.body);
      if (hd.peak_hour) h('p', 'wp-hint', t('peak.hour', { hour: hourLabel(hd.peak_hour.hour) + '–' + hourLabel((hd.peak_hour.hour + 1) % 24) }), pk.body);
    });
  }

  function tabBots(root) {
    var g = grid(root);
    var tr = card(g, { title: t('card.bot_trend'), info: I.def('bot_share'), span: 'wide' });
    load(tr, health, function (hd) {
      var es = hd.errors_series;
      var g2 = hd.range.granularity === 'hour' ? 'day' : hd.range.granularity;
      var series = [
        { name: t('metric.human_hits'), values: es.hits, color: 'var(--wp-s1)', area: true },
        { name: t('metric.bot_hits'), values: es.bot_hits, color: 'var(--wp-s7)' }
      ];
      C.line(h('div', null, null, tr.body), { labels: es.labels, series: series, height: 230, weekend: g2 === 'day',
        xTick: function (l) { return bucketTick(l, g2); }, title: function (l) { return bucketTitle(l, g2); }, caption: t('card.bot_trend') });
      var hum = es.hits.reduce(function (a, b) { return a + b; }, 0), bot = es.bot_hits.reduce(function (a, b) { return a + b; }, 0);
      legendList(tr.body, [
        { label: t('metric.human_hits'), color: 'var(--wp-s1)', line: true, value: hum },
        { label: t('metric.bot_hits') + ' · ' + fmt.pct(hum + bot ? bot / (hum + bot) : 0), color: 'var(--wp-s7)', line: true, value: bot }
      ]);
    });
    reportCard(g, { dim: 'bot', title: t('card.crawlers'), cols: ['hits', 'bytes', 'share', 'last_day'], span: 'wide', limit: 20,
      after: function (body) { h('p', 'wp-hint', t('ai.note'), body); } });
  }

  function tabVisits(root, ov) {
    var note = h('p', 'wp-hint', (S.report ? t('report.visits') + ' ' : '') + t('visits.retention', { n: ov ? ov.meta.raw_retention_days : 30 }), root);
    note.style.margin = '0 0 12px';
    var list = h('div', 'wp-visits', null, root);
    var more = h('div', 'wp-more', null, root);
    var token = S.seq;
    function page(before, beforeId) {
      clear(more);
      var b = btn('', t('common.loading'), null, more);
      b.disabled = true;
      api(scoped({ a: 'visits_log', limit: S.report ? 50 : 25, before: before, before_id: beforeId })).then(function (d) {
        if (token !== S.seq) return;
        clear(more);
        if (!d.visits.length && !before) { h('div', 'wp-state', null, list).appendChild(document.createTextNode(t('visits.empty'))); return; }
        d.visits.forEach(function (v) { visitRow(list, v); });
        if (S.report) return;
        if (d.next_before) btn('', t('visits.load_more'), function () { page(d.next_before, d.next_before_id); }, more);
        else if (before) h('span', 'wp-hint', t('visits.end'), more);
      }).catch(function (e) {
        if (token !== S.seq) return;
        clear(more);
        h('span', 'wp-card-error', e.message, more);
        btn('is-small', t('state.retry'), function () { page(before, beforeId); }, more);
      });
    }
    page(null, null);
  }

  function visitRow(list, v) {
    var row = h('article', 'wp-visit', null, list);
    var meta = h('div', 'wp-visit-meta', null, row);
    var tm = h('div', 'wp-visit-time', null, meta);
    tm.textContent = dateFmt.format(parseDay(dayOf(v.start))) + ' · ' + timeOf(v.start);
    if (v.live) { tm.appendChild(document.createTextNode(' ')); h('span', 'wp-badge is-live', t('visits.live'), tm); }
    var who = h('div', 'wp-visit-who', null, meta);
    flagEl(v.country, who);
    h('span', null, countryName(v.country) + (v.city ? ' · ' + v.city : ''), who);
    var dev = h('div', 'wp-visit-who', null, meta);
    var ic = svgIcon(DEVICES.indexOf(v.device) >= 0 ? v.device : 'other');
    dev.appendChild(ic);
    var devText = [v.browser, v.os].filter(Boolean).join(' · ');
    h('span', null, devText || t('unknown'), dev);
    ic.setAttribute('aria-label', v.device || 'device');
    var src = h('div', 'wp-visit-who', null, meta);
    if (v.channel) {
      var sw = h('span', 'wp-key-swatch', null, src);
      sw.style.setProperty('--k', channelColor(v.channel));
      h('span', null, (S.boot.dims.channel ? '' : '') + ({ direct: 'Direct', organic: 'Organic search', social: 'Social', referral: 'Referral', paid: 'Paid', email: 'Email', campaign: 'Campaign' }[v.channel] || v.channel), src);
    }
    var ref = v.referrer_host || (v.campaign ? v.campaign : null);
    if (ref) h('span', 'wp-visit-src', '· ' + ref + (v.keyword ? ' “' + v.keyword + '”' : ''), src);
    var sum = h('div', 'wp-visit-sum', null, meta);
    h('span', null, v.pageviews === 1 ? t('visits.page') : t('visits.pages', { n: v.pageviews }), sum);
    h('span', null, fmt.duration(v.duration), sum);
    h('span', null, fmt.bytes(v.bytes), sum);

    var acts = h('div', 'wp-actions', null, row);
    acts.setAttribute('aria-label', t('visits.actions'));
    v.actions.forEach(function (a) {
      var ar = h('div', 'wp-action', null, acts);
      h('span', 'wp-action-time', timeOf(a.ts), ar);
      var p = h('span', 'wp-action-path', a.path, ar);
      p.title = a.path;
      var badges = h('span', null, null, ar);
      if (a.type !== 'pageview') h('span', 'wp-badge', t('type.' + a.type), badges);
      if (a.status !== 200) h('span', 'wp-badge s' + String(a.status).charAt(0) + 'xx', String(a.status), badges);
      h('span', 'wp-action-bytes', fmt.bytes(a.bytes), ar);
    });
    if (!v.actions.length) h('span', 'wp-hint', v.entry_page || '', acts);
  }

  function tabSettings(root) {
    if (S.boot.principal.type !== 'user') { h('div', 'wp-state', t('settings.readonly'), root); return; }
    var c = card(grid(root), { title: t('settings.title', { domain: S.domain.domain }), span: 'wide' });
    load(c, function () { return api({ a: 'user_settings' }); }, function (data) {
      var settings = data.settings || {};
      if (!settings.domains || Array.isArray(settings.domains)) settings.domains = {};
      var doms = settings.domains;
      var cur = doms[S.domain.domain] || {};
      var form = h('form', 'wp-form', null, c.body);
      form.noValidate = true;
      function field(label, help, id) {
        var f = h('div', 'wp-field', null, form);
        var l = h('label', null, label, f);
        l.htmlFor = id;
        var hp = help ? h('div', 'wp-help', help, null) : null;
        return { f: f, help: hp };
      }
      // timezone
      var tzF = field(t('settings.tz'), t('settings.tz.help'), 'wp-tz');
      var tz = h('select', 'wp-select', null, tzF.f);
      tz.id = 'wp-tz';
      h('option', null, t('settings.tz.default', { tz: data.server_timezone }), tz).value = '';
      var zones = [];
      try { zones = Intl.supportedValuesOf('timeZone'); } catch (e) { zones = []; }
      if (cur.timezone && zones.indexOf(cur.timezone) < 0) zones.unshift(cur.timezone);
      zones.forEach(function (z) { var o = h('option', null, z, tz); o.value = z; });
      tz.value = cur.timezone || '';
      tzF.f.appendChild(tzF.help);
      // IPs
      var ipF = field(t('settings.ips'), t('settings.ips.help'), 'wp-ips');
      var ips = h('textarea', 'wp-textarea', null, ipF.f);
      ips.id = 'wp-ips';
      ips.value = (cur.exclude_ips || []).join('\n');
      ips.spellcheck = false;
      if (data.remote_addr) {
        var addRow = h('div', 'wp-form-row', null, ipF.f);
        addRow.style.marginTop = '6px';
        btn('is-small', t('settings.addip', { ip: data.remote_addr }), function () {
          var lines = ips.value.split(/\s+/).filter(Boolean);
          if (lines.indexOf(data.remote_addr) < 0) lines.push(data.remote_addr);
          ips.value = lines.join('\n');
        }, addRow);
      }
      ipF.f.appendChild(ipF.help);
      // paths
      var pF = field(t('settings.paths'), t('settings.paths.help'), 'wp-paths');
      var paths = h('textarea', 'wp-textarea', null, pF.f);
      paths.id = 'wp-paths';
      paths.value = (cur.exclude_paths || []).join('\n');
      paths.spellcheck = false;
      pF.f.appendChild(pF.help);
      // UI prefs
      var ui = settings.ui || {};
      var rF = field(t('settings.default_range'), null, 'wp-defrange');
      var rs = h('select', 'wp-select', null, rF.f);
      rs.id = 'wp-defrange';
      ALL_RANGES.forEach(function (r) { var o = h('option', null, t('range.' + r), rs); o.value = r; });
      rs.value = ui.default_range || '30d';
      var thF = field(t('settings.theme'), null, 'wp-theme');
      var ths = h('select', 'wp-select', null, thF.f);
      ths.id = 'wp-theme';
      ['auto', 'light', 'dark'].forEach(function (x) { var o = h('option', null, t('settings.theme.' + x), ths); o.value = x; });
      ths.value = ui.dark === true ? 'dark' : ui.dark === false ? 'light' : (ui.dark || 'auto');

      h('p', 'wp-hint', t('settings.note'), form);
      var row = h('div', 'wp-form-row', null, form);
      var save = h('button', 'wp-btn is-primary', t('settings.save'), row);
      save.type = 'submit';
      var status = h('span', 'wp-status', null, row);
      status.setAttribute('role', 'status');
      form.addEventListener('submit', function (ev) {
        ev.preventDefault();
        var next = JSON.parse(JSON.stringify(settings));
        if (!next.domains || Array.isArray(next.domains)) next.domains = {};
        var entry = {};
        if (tz.value) entry.timezone = tz.value;
        entry.exclude_ips = ips.value.split(/[\s,]+/).filter(Boolean);
        entry.exclude_paths = paths.value.split(/\s+/).filter(Boolean);
        next.domains[S.domain.domain] = entry;
        next.ui = { default_range: rs.value, dark: ths.value };
        save.disabled = true;
        status.className = 'wp-status';
        status.textContent = t('common.loading');
        api({ a: 'save_user_settings' }, { settings: next }).then(function (res) {
          settings = res.settings || next;
          status.className = 'wp-status is-ok';
          status.textContent = t('settings.saved');
          S.theme = ths.value;
          storage('webpulse.theme', ths.value === 'auto' ? null : ths.value);
          applyTheme();
        }).catch(function (e) {
          status.className = 'wp-status is-err';
          status.textContent = e.message;
        }).then(function () { save.disabled = false; });
      });
    });
  }

  /* -------------------------------------------------------------- render */
  function stateBox(parent, title, body, retry) {
    var box = h('div', 'wp-state', null, parent);
    h('h2', null, title, box);
    h('p', null, body, box);
    if (retry) btn('', t('state.retry'), retry, box);
    return box;
  }

  function renderTab(ov) {
    var c = S.els.content;
    clear(c);
    switch (S.tab) {
      case 'overview': tabOverview(c, ov); break;
      case 'pages': tabPages(c); break;
      case 'acquisition': tabAcquisition(c); break;
      case 'audience': tabAudience(c); break;
      case 'technical': tabTechnical(c); break;
      case 'bots': tabBots(c); break;
      case 'visits': tabVisits(c, ov); break;
      case 'settings': tabSettings(c); break;
    }
  }

  function render() {
    S.seq++;
    var els = S.els;
    closePops();
    C.hideTip();
    syncHeader();
    clear(els.notice);
    var domKey = S.domain ? S.domain.user + '/' + S.domain.domain : '';
    if (domKey !== S.liveKey || S.view !== S.liveView) { S.liveKey = domKey; S.liveView = S.view; startLive(); }

    if (S.view === 'server') {
      els.hero.hidden = true;
      els.live.hidden = true;
      clear(els.content);
      if (window.WebPulseAdmin) window.WebPulseAdmin.render(els.content, adminCtx());
      return;
    }
    if (!S.domain) {
      els.hero.hidden = true;
      clear(els.content);
      stateBox(els.content, t('state.nodomains.title'), t('state.nodomains.body'));
      return;
    }
    if (S.tab === 'settings') {
      els.hero.hidden = true;
      renderTab(null);
      return;
    }
    if (!S.domain.has_data) {
      els.hero.hidden = true;
      clear(els.content);
      stateBox(els.content, t('state.nodata.title'), t('state.nodata.body'), function () { S.cache = {}; render(); });
      return;
    }
    els.hero.hidden = false;
    els.hero.classList.add('is-loading');
    els.chart.classList.add('is-loading');
    var token = S.seq;
    if (S.tab !== 'overview' && S.tab !== 'visits') renderTab(null);
    else if (!els.content.firstChild || S.lastTab !== S.tab) { clear(els.content); }
    S.lastTab = S.tab;
    api(scoped({ a: 'overview', compare: S.compare ? 1 : null })).then(function (ov) {
      if (token !== S.seq) return;
      S.overview = ov;
      els.hero.classList.remove('is-loading');
      els.chart.classList.remove('is-loading');
      renderKpis(ov);
      renderMainChart(ov);
      if (ov.meta.tz_changed_at) {
        var n = h('div', 'wp-notice', null, els.notice);
        h('span', null, t('tz.changed', { date: dateFmtY.format(parseDay(ov.meta.tz_changed_at)) }), n);
      }
      if (S.tab === 'overview' || S.tab === 'visits') renderTab(ov);
    }).catch(function (err) {
      if (token !== S.seq) return;
      els.hero.classList.remove('is-loading');
      els.hero.hidden = true;
      clear(els.content);
      if (err.code === 'no_data') stateBox(els.content, t('state.nodata.title'), t('state.nodata.body'));
      else if (err.code === 'schema_too_new') stateBox(els.content, t('state.error.title'), t('state.schema'));
      else stateBox(els.content, t('state.error.title'), err.message, function () { S.cache = {}; render(); });
    });
  }

  /* -------------------------------------------------------------- report */
  /* `#report&d=…&r=…` renders every tab in sequence for printing / Save as PDF;
   * `#report=server` the WHM server report. Prints once everything has loaded,
   * unless the URL carries `noprint=1` (headless rendering). */
  var REPORT_TABS = ['overview', 'pages', 'acquisition', 'audience', 'technical', 'bots', 'visits'];
  var stampFmt = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });

  function settled() {
    S.pending--;
    if (S.pending <= 0) setTimeout(reportCheck, 400);
  }
  function reportCheck() {
    if (!S.report || S.pending > 0 || !S.reportBuilt || S.reportDone) return;
    S.reportDone = true;
    S.els.status.textContent = t('report.ready');
    S.root.setAttribute('data-report-ready', '1');
    var noprint = /[?&]noprint=1(&|$)/.test(location.search) || parseHash().noprint === '1';
    if (!S.printed && !noprint) { S.printed = true; setTimeout(function () { window.print(); }, 200); }
  }
  /** Inside cPanel chrome the app is nested deep in Jupiter's layout: hide
   *  everything around it on paper and flatten its ancestors. */
  function printHost() {
    var n = S.root;
    while (n && n !== document.body && n.parentNode && n.parentNode.nodeType === 1) {
      var par = n.parentNode;
      for (var c = par.firstElementChild; c; c = c.nextElementSibling) if (c !== n) c.classList.add('wp-print-hide');
      par.classList.add('wp-print-host');
      n = par;
    }
  }
  function rangeText(r) {
    return dateFmtY.format(parseDay(r.from)) + ' – ' + dateFmtY.format(parseDay(r.to));
  }
  function reportSection(parent, title, foot) {
    var sec = h('section', 'wp-rpt-sec', null, parent);
    h('h2', 'wp-rpt-h', title, sec);
    var body = h('div', 'wp-rpt-body', null, sec);
    var f = h('p', 'wp-rpt-foot', null, sec);
    h('span', null, foot, f);
    h('span', null, 'WebPulse ' + (S.boot.version || ''), f);
    return { el: sec, body: body };
  }

  function renderReport(p) {
    S.report = true; S.reportBuilt = false; S.reportDone = false; S.pending = 0; S.seq++;
    var server = p.report === 'server' && S.isAdmin;
    if (!server) readState(p);
    if (S.liveTimer) { clearInterval(S.liveTimer); S.liveTimer = 0; }
    var root = S.root;
    clear(root);
    applyTheme();
    root.removeAttribute('data-report-ready');
    printHost();
    var els = S.els = {};
    var doc = h('div', 'wp-rpt', null, root);
    var bar = h('div', 'wp-rpt-bar', null, doc);
    var save = btn('is-primary', null, function () { S.printed = true; window.print(); }, bar);
    save.appendChild(svgIcon('report'));
    h('span', null, t('report.save'), save);
    var back = h('a', 'wp-btn', t('report.back'), bar);
    back.href = location.pathname + location.search + (server ? '#server' : stateHash({ t: 'overview' }));
    els.status = h('span', 'wp-rpt-status', t('report.loading'), bar);
    els.status.setAttribute('role', 'status');

    var cover = h('header', 'wp-rpt-cover', null, doc);
    var brand = h('div', 'wp-brand', null, cover);
    brand.appendChild(logo());
    h('span', null, t('app.name'), brand);
    h('p', 'wp-rpt-kind', t(server ? 'report.server.title' : 'report.title'), cover);
    var pr = S.boot.principal;
    var scope = pr.type === 'reseller' ? t('report.scope.reseller', { name: pr.name }) : t('report.scope.all');
    var title = server ? (S.boot.hostname || scope) : (S.domain ? S.domain.domain : t('state.nodomains.title'));
    h('h1', 'wp-rpt-title', title, cover);
    var meta = h('dl', 'wp-rpt-meta', null, cover);
    function row(k, v) { var d = h('div', null, null, meta); h('dt', null, k, d); return h('dd', null, v, d); }
    var rangeDd = row(t('report.range'), server ? t('report.server.range') : t('common.loading'));
    var prevDd = row(t('report.compare'), '');
    prevDd.parentNode.hidden = true;
    if (server && S.boot.hostname) row(t('report.accounts'), scope);
    if (!server && S.domain && S.isAdmin) row(t('report.account'), S.domain.user);
    if (S.boot.hostname && !server) row(t('report.server'), S.boot.hostname);
    row(t('report.generated'), stampFmt.format(new Date()));
    row(t('report.version'), 'WebPulse ' + (S.boot.version || ''));
    var toc = h('div', 'wp-rpt-toc', null, cover);
    h('h2', null, t('report.contents'), toc);
    var tocList = h('ol', null, null, toc);
    var names = server ? [t('server.button'), t('report.accounts'), t('report.jobs')] : REPORT_TABS.map(function (x) { return t('tab.' + x); });
    names.forEach(function (n) { h('li', null, n, tocList); });

    function built() { S.reportBuilt = true; if (!S.pending) setTimeout(reportCheck, 400); }
    function failed(msg) {
      S.reportDone = true;
      els.status.textContent = msg;
      stateBox(doc, t('state.error.title'), msg);
    }
    if (server) {
      if (window.WebPulseAdmin && window.WebPulseAdmin.report) {
        var ctx = adminCtx();
        var sfoot = (S.boot.hostname || scope) + ', ' + dateFmtY.format(new Date());
        ctx.section = function (name) { return reportSection(doc, name, sfoot); };
        window.WebPulseAdmin.report(doc, ctx);
      }
      built();
      return;
    }
    if (!S.domain || !S.domain.has_data) { failed(S.domain ? t('state.nodata.body') : t('state.nodomains.body')); return; }
    var token = S.seq;
    api(scoped({ a: 'overview', compare: S.compare ? 1 : null })).then(function (ov) {
      if (token !== S.seq) return;
      S.overview = ov;
      rangeDd.textContent = rangeText(ov.range) + ' (' + t('range.' + S.range) + ')';
      if (S.compare && ov.range.prev) { prevDd.textContent = rangeText(ov.range.prev); prevDd.parentNode.hidden = false; }
      var foot = S.domain.domain + ', ' + rangeText(ov.range);
      REPORT_TABS.forEach(function (tab) {
        var c = reportSection(doc, t('tab.' + tab), foot).body;
        switch (tab) {
          case 'overview':
            var hero = h('section', 'wp-hero', null, c);
            els.kpis = h('div', 'wp-kpis', null, hero);
            els.chartHead = h('div', 'wp-chart-head', null, hero);
            els.chart = h('div', 'wp-main-chart', null, hero);
            renderKpis(ov);
            renderMainChart(ov);
            tabOverview(h('div', 'wp-rpt-gap', null, c), ov);
            break;
          case 'pages': tabPages(c); break;
          case 'acquisition': tabAcquisition(c); break;
          case 'audience': tabAudience(c); break;
          case 'technical': tabTechnical(c); break;
          case 'bots': tabBots(c); break;
          case 'visits': tabVisits(c, ov); break;
        }
      });
      built();
    }).catch(function (err) {
      if (token !== S.seq) return;
      failed(err.code === 'no_data' ? t('state.nodata.body') : err.message);
    });
  }

  function adminCtx() {
    return {
      api: api, apiHref: apiHref, t: t, I: I, C: C, fmt: fmt, h: h, clear: clear, card: card, load: load, btn: btn, info: info,
      legendList: legendList, svgIcon: svgIcon, ago: ago, dateFmt: dateFmt, dateFmtY: dateFmtY, parseDay: parseDay, stateBox: stateBox,
      principal: S.boot.principal, mode: S.boot.mode || null, seq: function () { return S.seq; },
      openDomain: function (user, domain) {
        var d = findDomain(domain, user);
        if (!d) {
          d = { domain: domain, user: user, has_data: true, visits_today: 0, kind: 'unknown' };
          S.domains.push(d);
        }
        navigate({ t: 'overview', d: domain, u: user });
      }
    };
  }

  function start(root, boot) {
    if (!root || !boot) return;
    var cockpit = boot.transport === 'cockpit' && window.WebPulseCockpit;
    // inside Cockpit the shell's style wins over a theme saved in this browser
    var saved = cockpit ? null : storage('webpulse.theme');
    S = {
      root: root, boot: boot, apiUrl: boot.apiUrl || (cockpit ? '' : 'api'), domains: boot.domains || [], cache: {}, seq: 0, q: '',
      fetchFn: cockpit ? window.WebPulseCockpit.fetch : null,
      defaults: { range: (boot.defaults && boot.defaults.range) || '30d' },
      theme: saved || (boot.defaults && boot.defaults.dark) || 'auto',
      tab: 'overview', range: '30d', metrics: ['visits', 'pageviews'], compare: false, domain: null,
      isAdmin: boot.principal.type !== 'user', view: 'domain', mode: null, report: false, pending: 0
    };
    if (window.matchMedia) {
      var mq = window.matchMedia('(prefers-color-scheme: dark)');
      if (mq.addEventListener) mq.addEventListener('change', applyTheme);
    }
    window.addEventListener('hashchange', route);
    route();
  }

  /** Theme from the host (Cockpit shell style): 'auto' | 'light' | 'dark'. */
  function setTheme(th) {
    if (!S || (th !== 'auto' && th !== 'light' && th !== 'dark')) return;
    S.theme = th;
    applyTheme();
  }

  window.WebPulseApp = { start: start, setTheme: setTheme };
})();
