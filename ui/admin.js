/* WebPulse — WHM "Server" tab (admin and reseller principals). */
(function () {
  'use strict';

  var A = {};

  function render(root, ctx) {
    var h = ctx.h, t = ctx.t, C = ctx.C, fmt = ctx.fmt;
    var isRoot = ctx.principal.type === 'admin';
    var token = ctx.seq();
    var alive = function () { return token === ctx.seq(); };
    if (A.poll) { clearTimeout(A.poll); A.poll = 0; }

    /* ---------------------------------------------------- server KPIs */
    var hero = h('section', 'wp-hero', null, root);
    hero.setAttribute('aria-label', 'Server overview');
    var kp = h('div', 'wp-health', null, hero);
    h('div', 'wp-empty', t('common.loading'), kp).style.padding = '16px';
    var grid = h('div', 'wp-grid', null, root);
    grid.style.marginTop = '16px';

    var top = ctx.card(grid, { title: 'Top domains', sub: 'Last 30 days', span: 'wide' });
    var acc = ctx.card(grid, { title: 'Accounts', sub: 'Collection is on for every account unless switched off here. "Default" follows the server setting.', span: 'wide' });
    var jobs = ctx.card(grid, { title: 'Jobs', span: isRoot ? 'two-thirds' : 'wide' });
    var set = isRoot ? ctx.card(grid, { title: 'Settings', sub: 'Server-wide; stored in the WebPulse registry.', span: 'third' }) : null;

    ctx.load({ el: hero, body: kp }, function () { return ctx.api({ a: 'admin.overview' }); }, function (o) {
      var cell = function (label, value, sub, cls) {
        var d = h('div', null, null, kp);
        h('div', 'wp-kpi-label', label, d);
        var v = h('div', 'wp-kpi-value', value, d);
        if (cls) v.classList.add(cls);
        if (sub) h('div', 'wp-kpi-sub', sub, d);
        return d;
      };
      cell('Visits today', fmt.num(o.totals.today.visits), fmt.num(o.totals.today.pageviews) + ' pageviews');
      cell('Visits, 7 days', fmt.num(o.totals['7d'].visits), fmt.num(o.totals['7d'].pageviews) + ' pageviews');
      cell('Visits, 30 days', fmt.num(o.totals['30d'].visits), fmt.num(o.totals['30d'].pageviews) + ' pageviews');
      cell('Bandwidth, 30 days', fmt.bytes(o.totals['30d'].bytes), fmt.bytes(o.totals['7d'].bytes) + ' in 7 days');
      cell('Bot share, 30 days', o.bot_share_30d == null ? '–' : fmt.pct(o.bot_share_30d), fmt.num(o.totals['30d'].bot_hits) + ' bot hits' + (o.bot_share_partial ? ' (partial)' : ''));
      cell('Data size', fmt.bytes(o.disk.data_bytes), o.disk.databases + ' databases · ' + o.accounts.enabled + '/' + o.accounts.total + ' accounts on');
      var lag = o.collector.lag_seconds;
      cell('Collector', o.collector.last_collect_at ? ctx.ago(o.collector.last_collect_at) : 'never',
        lag == null ? 'no log lines yet' : 'newest log line ' + (lag < 120 ? lag + ' s' : Math.round(lag / 60) + ' min') + ' old');
      var ok = function (b) { return b ? '✓' : '✗'; };
      var st = cell('Integration', null, null);
      var line = function (label, good) {
        var p = h('div', 'wp-kpi-sub', null, st);
        h('span', good ? 'wp-ok-icon' : 'wp-bad-icon', ok(good) + ' ', p);
        h('span', null, label, p);
      };
      st.querySelector('.wp-kpi-value').remove();
      line('Cron job', o.collector.cron_installed);
      line('Stats hook', o.collector.hook_installed);
      line('GeoIP' + (o.collector.geoip.db_date ? ' (' + o.collector.geoip.db_date + ')' : ''), o.collector.geoip.enabled && !!o.collector.geoip.db_date);

      // top domains
      ctx.clear(top.body);
      if (!o.top_domains.length) { h('div', 'wp-empty', 'No data collected yet. Run a collection below.', top.body); return; }
      var wrap = h('div', 'wp-scroll', null, top.body);
      var tbl = h('table', 'wp-table is-admin', null, wrap);
      var hr = h('tr', null, null, h('thead', null, null, tbl));
      ['Domain', 'Visits', 'Pageviews', 'Bandwidth', 'Bot hits', 'Today', 'Last collection'].forEach(function (x, i) {
        var th = h('th', i >= 2 ? 'is-optional' : null, x, hr); th.scope = 'col';
      });
      var tb = h('tbody', null, null, tbl);
      var max = Math.max.apply(null, o.top_domains.map(function (d) { return d.visits_30d; }).concat([1]));
      o.top_domains.forEach(function (d) {
        var tr = h('tr', null, null, tb);
        var key = h('div', 'wp-key', null, h('td', null, null, tr));
        h('span', 'wp-key-bar', null, key).style.width = Math.max(0.5, d.visits_30d / max * 100) + '%';
        var a = h('a', 'wp-key-text', d.domain, key);
        a.href = '#';
        a.addEventListener('click', function (ev) { ev.preventDefault(); ctx.openDomain(d.user, d.domain); });
        h('span', 'wp-badge', d.user, key);
        h('td', null, fmt.num(d.visits_30d), tr);
        h('td', 'is-optional', fmt.num(d.pageviews_30d), tr);
        h('td', 'is-optional', fmt.bytes(d.bytes_30d), tr);
        h('td', 'is-optional', fmt.num(d.bot_hits_30d), tr);
        h('td', 'is-optional', fmt.num(d.visits_today), tr);
        h('td', 'is-optional', d.last_collect_at ? ctx.ago(d.last_collect_at) : '–', tr);
      });
    });

    /* ------------------------------------------------------- accounts */
    var accState = { q: '', limit: 50, list: [] };
    var search = h('div', 'wp-search', null, acc.body.parentNode);
    acc.body.parentNode.insertBefore(search, acc.body);
    var qIn = h('input', 'wp-input', null, search);
    qIn.type = 'search';
    qIn.placeholder = 'Search accounts or domains…';
    qIn.setAttribute('aria-label', 'Search accounts or domains');
    var qTimer = 0;
    qIn.addEventListener('input', function () { clearTimeout(qTimer); qTimer = setTimeout(function () { accState.q = qIn.value.trim(); loadAccounts(); }, 300); });

    function tri(current, onSet, label) {
      var g = h('div', 'wp-tri', null, null);
      g.setAttribute('role', 'radiogroup');
      g.setAttribute('aria-label', label);
      [['Default', null, ''], ['On', true, 'is-on'], ['Off', false, 'is-off']].forEach(function (o) {
        var b = h('button', o[2], o[0], g);
        b.type = 'button';
        b.setAttribute('role', 'radio');
        b.setAttribute('aria-checked', current === o[1] ? 'true' : 'false');
        b.addEventListener('click', function () {
          if (b.getAttribute('aria-checked') === 'true') return;
          var all = g.querySelectorAll('button');
          for (var i = 0; i < all.length; i++) all[i].disabled = true;
          onSet(o[1]).then(function () {
            for (var i = 0; i < all.length; i++) all[i].setAttribute('aria-checked', all[i] === b ? 'true' : 'false');
          }).catch(function (e) { window.alert(e.message); }).then(function () {
            for (var i = 0; i < all.length; i++) all[i].disabled = false;
          });
        });
      });
      return g;
    }
    function stateBadge(on) {
      var b = h('span', 'wp-badge ' + (on ? 'is-on' : 'is-off'), on ? 'Collecting' : 'Off', null);
      return b;
    }

    function loadAccounts() {
      ctx.load(acc, function () { return ctx.api({ a: 'admin.accounts', q: accState.q || null, limit: accState.limit }); }, function (d) {
        accState.list = d.accounts;
        fillJobAccounts();
        if (!d.accounts.length) { h('div', 'wp-empty', accState.q ? 'No matching accounts.' : 'No accounts discovered yet. Run a collection to discover accounts.', acc.body); return; }
        var wrap = h('div', 'wp-scroll', null, acc.body);
        var tbl = h('table', 'wp-table is-admin', null, wrap);
        var hr = h('tr', null, null, h('thead', null, null, tbl));
        [['Account', 'is-left'], ['Owner', 'is-left is-optional'], ['State', 'is-left'], ['Collection', 'is-left'], ['Visits 30d', 'is-optional'], ['Data', 'is-optional']].forEach(function (x) {
          var th = h('th', x[1], x[0], hr); th.scope = 'col';
        });
        var tb = h('tbody', null, null, tbl);
        d.accounts.forEach(function (a) {
          var tr = h('tr', 'wp-acc-row', null, tb);
          var td0 = h('td', null, null, tr);
          var kb = h('button', 'wp-key-btn', null, td0);
          kb.type = 'button';
          kb.setAttribute('aria-expanded', 'false');
          h('span', 'wp-caret', '▶', kb).setAttribute('aria-hidden', 'true');
          h('strong', null, a.user, kb);
          h('span', 'wp-card-sub', ' ' + a.domains.length + (a.domains.length === 1 ? ' domain' : ' domains'), kb);
          if (a.removed) h('span', 'wp-badge', 'removed', td0);
          h('td', 'is-left is-optional', a.owner || '–', tr);
          var stTd = h('td', 'is-left', null, tr);
          stTd.appendChild(stateBadge(a.enabled));
          var swTd = h('td', 'is-left', null, tr);
          swTd.appendChild(tri(a.enabled_override, function (val) {
            return ctx.api({ a: 'admin.set_account' }, { user: a.user, enabled: val }).then(function (res) {
              a.enabled = res.account ? res.account.enabled : a.enabled;
              ctx.clear(stTd); stTd.appendChild(stateBadge(a.enabled));
            });
          }, 'Collection for ' + a.user));
          h('td', 'is-optional', fmt.num(a.visits_30d), tr);
          h('td', 'is-optional', fmt.bytes(a.db_bytes), tr);

          var sub = [];
          kb.addEventListener('click', function () {
            var open = kb.getAttribute('aria-expanded') === 'true';
            kb.setAttribute('aria-expanded', open ? 'false' : 'true');
            if (sub.length) { sub.forEach(function (r) { r.hidden = open; }); return; }
            var anchor = tr;
            a.domains.forEach(function (dm) {
              var r = document.createElement('tr');
              r.className = 'wp-acc-domains';
              anchor.parentNode.insertBefore(r, anchor.nextSibling);
              anchor = r;
              sub.push(r);
              var c0 = h('td', null, null, r);
              var kk = h('div', 'wp-key', null, c0);
              kk.style.paddingLeft = '26px';
              h('span', 'wp-key-text', dm.domain, kk);
              h('span', 'wp-badge', dm.kind, kk);
              if (dm.has_data) {
                var link = h('a', null, 'View dashboard', h('td', 'is-left is-optional', null, r));
                link.href = '#';
                link.addEventListener('click', function (ev) { ev.preventDefault(); ctx.openDomain(a.user, dm.domain); });
              } else {
                h('td', 'is-left is-optional', 'no data yet', r);
              }
              var s2 = h('td', 'is-left', null, r);
              s2.appendChild(stateBadge(dm.enabled));
              var sw2 = h('td', 'is-left', null, r);
              sw2.appendChild(tri(dm.enabled_override, function (val) {
                return ctx.api({ a: 'admin.set_domain' }, { user: a.user, domain: dm.domain, enabled: val }).then(function (res) {
                  dm.enabled = res.domain ? res.domain.enabled : dm.enabled;
                  ctx.clear(s2); s2.appendChild(stateBadge(dm.enabled));
                });
              }, 'Collection for ' + dm.domain));
              h('td', 'is-optional', fmt.num(dm.visits_30d), r);
              h('td', 'is-optional', fmt.bytes(dm.db_bytes), r);
            });
            if (!a.domains.length) {
              var r0 = document.createElement('tr');
              r0.className = 'wp-acc-domains';
              tr.parentNode.insertBefore(r0, tr.nextSibling);
              var cc = h('td', 'is-left', 'No domains known for this account.', r0);
              cc.colSpan = 6;
              sub.push(r0);
            }
          });
        });
        ctx.clear(acc.foot);
        acc.foot.hidden = false;
        h('span', null, d.total + (d.total === 1 ? ' account' : ' accounts') + ' · default: collection ' + (d.default_enabled ? 'on' : 'off'), acc.foot);
        if (d.has_more) ctx.btn('is-small', 'Show more', function () { accState.limit = Math.min(500, accState.limit * 2); loadAccounts(); }, acc.foot);
      });
    }
    loadAccounts();

    /* ----------------------------------------------------------- jobs */
    var jobCtl = h('div', 'wp-form-row', null, jobs.body.parentNode);
    jobs.body.parentNode.insertBefore(jobCtl, jobs.body);
    jobCtl.style.marginBottom = '12px';
    var accSel = h('select', 'wp-select', null, jobCtl);
    accSel.setAttribute('aria-label', 'Account');
    var runStatus = h('span', 'wp-status', null, null);
    runStatus.setAttribute('role', 'status');
    function fillJobAccounts() {
      var cur = accSel.value;
      ctx.clear(accSel);
      if (isRoot) h('option', null, 'All accounts', accSel).value = '';
      accState.list.forEach(function (a) { h('option', null, a.user, accSel).value = a.user; });
      if (cur) accSel.value = cur;
    }
    function run(type) {
      var user = accSel.value || null;
      if (type === 'backfill' && !user) { runStatus.className = 'wp-status is-err'; runStatus.textContent = 'Pick an account to backfill.'; return; }
      runStatus.className = 'wp-status';
      runStatus.textContent = 'Starting…';
      ctx.api({ a: 'admin.run' }, { type: type, user: type === 'rollup' ? null : user }).then(function (r) {
        runStatus.className = 'wp-status is-ok';
        runStatus.textContent = 'Started job #' + r.job_id + '.';
        loadJobs();
      }).catch(function (e) { runStatus.className = 'wp-status is-err'; runStatus.textContent = e.message; });
    }
    ctx.btn('is-primary', 'Run collection now', function () { run('collect'); }, jobCtl);
    ctx.btn('', 'Backfill from archives', function () { run('backfill'); }, jobCtl);
    if (isRoot) ctx.btn('', 'Run rollup', function () { run('rollup'); }, jobCtl);
    jobCtl.appendChild(runStatus);

    function loadJobs() {
      if (A.poll) { clearTimeout(A.poll); A.poll = 0; }
      ctx.api({ a: 'admin.jobs' }).then(function (d) {
        if (!alive()) return;
        ctx.clear(jobs.body);
        if (!d.jobs.length) { h('div', 'wp-empty', 'No jobs have run yet.', jobs.body); return; }
        var wrap = h('div', 'wp-scroll', null, jobs.body);
        var tbl = h('table', 'wp-table is-admin', null, wrap);
        var hr = h('tr', null, null, h('thead', null, null, tbl));
        [['#', 'is-left'], ['Type', 'is-left'], ['Scope', 'is-left is-optional'], ['Started', 'is-left'], ['Took', ''], ['Lines', ''], ['Status', 'is-left']].forEach(function (x) {
          var th = h('th', x[1], x[0], hr); th.scope = 'col';
        });
        var tb = h('tbody', null, null, tbl);
        var running = false;
        d.jobs.slice(0, 25).forEach(function (j) {
          var tr = h('tr', null, null, tb);
          h('td', 'is-left', String(j.id), tr);
          h('td', 'is-left', j.type, tr);
          h('td', 'is-left is-optional', j.scope || '–', tr);
          h('td', 'is-left', j.started_at_iso ? ctx.ago(j.started_at_iso) : '–', tr);
          h('td', null, j.finished_at ? fmt.duration(j.finished_at - j.started_at) : '…', tr);
          h('td', null, fmt.num(j.lines), tr);
          var st = h('td', 'is-left', null, tr);
          var cls = j.status === 'ok' ? 'is-on' : j.status === 'running' ? 'is-live' : j.status === 'error' ? 'is-off' : '';
          var b = h('span', 'wp-badge ' + cls, j.status, st);
          if (j.message) { b.title = j.message; h('span', 'wp-card-sub', ' ' + j.message.slice(0, 60), st); }
          if (j.status === 'running') running = true;
        });
        if (running) A.poll = setTimeout(function () { if (alive()) loadJobs(); }, 3000);
      }).catch(function (e) {
        if (!alive()) return;
        ctx.clear(jobs.body);
        h('div', 'wp-card-error', e.message, jobs.body);
      });
    }
    loadJobs();

    /* -------------------------------------------------------- settings */
    if (set) {
      ctx.load(set, function () { return ctx.api({ a: 'admin.settings' }); }, function (d) {
        var s = d.settings;
        var form = h('form', 'wp-form', null, set.body);
        form.noValidate = true;
        var fields = {};
        function check(key, label) {
          var f = h('div', 'wp-field', null, form);
          var l = h('label', 'wp-checkbox', null, f);
          var i = h('input', null, null, l);
          i.type = 'checkbox';
          i.checked = !!s[key];
          h('span', null, label, l);
          fields[key] = function () { return i.checked; };
        }
        function num(key, label, min, max, help) {
          var f = h('div', 'wp-field', null, form);
          var l = h('label', null, label, f);
          var i = h('input', 'wp-input', null, f);
          i.type = 'number'; i.min = min; i.max = max; i.value = s[key];
          i.id = 'wp-set-' + key.replace(/\./g, '-'); l.htmlFor = i.id;
          if (help) h('div', 'wp-help', help, f);
          fields[key] = function () { return parseInt(i.value, 10); };
        }
        function list(key, label, help) {
          var f = h('div', 'wp-field', null, form);
          var l = h('label', null, label, f);
          var i = h('textarea', 'wp-textarea', null, f);
          i.id = 'wp-set-' + key; l.htmlFor = i.id;
          i.value = (s[key] || []).join('\n');
          i.spellcheck = false;
          i.style.minHeight = '80px';
          if (help) h('div', 'wp-help', help, f);
          fields[key] = function () { return i.value.split(/\s+/).filter(Boolean); };
        }
        check('default_enabled', 'Collect for new and "Default" accounts');
        num('raw_retention_days', 'Keep individual visits for (days)', 1, 3650, 'Aggregated reports are kept forever.');
        num('session_timeout_min', 'Visit timeout (minutes)', 5, 240);
        num('max_rows_per_dim', 'Max rows per report per day', 10, 100000, 'Longer tails are folded into "(other)" by the nightly rollup.');
        check('store_ip', 'Store visitor IP addresses in the visits log');
        check('geoip.enabled', 'Geolocate visitors (needs the GeoIP database)');
        list('exclude_ips', 'Excluded IPs / CIDRs (server-wide)');
        list('exclude_paths', 'Excluded path prefixes (server-wide)');
        var tf = h('div', 'wp-field', null, form);
        var tl = h('label', null, 'Default timezone', tf);
        var tz = h('select', 'wp-select', null, tf);
        tz.id = 'wp-set-tz'; tl.htmlFor = tz.id;
        h('option', null, 'Server (' + d.server_timezone + ')', tz).value = '';
        var zones = [];
        try { zones = Intl.supportedValuesOf('timeZone'); } catch (e) { zones = []; }
        if (s.timezone && zones.indexOf(s.timezone) < 0) zones.unshift(s.timezone);
        zones.forEach(function (z) { h('option', null, z, tz).value = z; });
        tz.value = s.timezone || '';
        fields.timezone = function () { return tz.value || null; };

        var row = h('div', 'wp-form-row', null, form);
        var save = h('button', 'wp-btn is-primary', 'Save settings', row);
        save.type = 'submit';
        var status = h('span', 'wp-status', null, row);
        status.setAttribute('role', 'status');
        form.addEventListener('submit', function (ev) {
          ev.preventDefault();
          var out = {};
          Object.keys(fields).forEach(function (k) { out[k] = fields[k](); });
          save.disabled = true;
          status.className = 'wp-status';
          status.textContent = 'Saving…';
          ctx.api({ a: 'admin.save_settings' }, { settings: out }).then(function () {
            status.className = 'wp-status is-ok';
            status.textContent = 'Saved.';
          }).catch(function (e) {
            status.className = 'wp-status is-err';
            status.textContent = e.message;
          }).then(function () { save.disabled = false; });
        });
      });
    }
  }

  window.WebPulseAdmin = { render: render };
})();
