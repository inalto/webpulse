/* WebPulse — Cockpit adapter (only in the Cockpit module's index.html).
 *
 * The dashboard talks to `webpulse cgi --cockpit --api` through Cockpit's
 * process bridge instead of HTTP: `WebPulseCockpit.fetch(url, init)` has the
 * shape of window.fetch (status, headers.get, text) and runs
 *   cockpit.spawn([bin, 'cgi', '--cockpit', '--api'], {superuser: 'try', ...})
 * with the CGI environment, the request body on stdin, and parses the CGI reply
 * (`Status:` line, headers, blank line, body). With "Administrative access" on,
 * Cockpit runs it as root (the administrator view); otherwise as the logged-in
 * user, who sees the sites it owns. No network listener, no token.
 *
 * Start-up: whoami (principal, version), domains, the user's settings, then
 * WebPulseApp.start. The theme follows the Cockpit shell (`shell:style`, the
 * pf-v5/pf-v6 dark class, the `cockpit-style` event), else the OS preference.
 */
(function () {
  'use strict';

  var BOOT = null;
  var ENC = new TextEncoder();

  /** CONTENT_LENGTH counts UTF-8 bytes, not UTF-16 code units. */
  function byteLength(s) {
    return s ? ENC.encode(s).length : 0;
  }

  /** Splits a CGI reply into status, lower-case headers and body. */
  function parseCgi(out) {
    var text = String(out == null ? '' : out);
    var sep = text.indexOf('\r\n\r\n');
    var skip = 4;
    var alt = text.indexOf('\n\n');
    if (sep < 0 || (alt >= 0 && alt < sep)) { sep = alt; skip = 2; }
    if (sep < 0) return { status: 502, headers: {}, body: text };
    var headers = {};
    var status = 200;
    text.slice(0, sep).split(/\r?\n/).forEach(function (line) {
      var i = line.indexOf(':');
      if (i <= 0) return;
      var k = line.slice(0, i).trim().toLowerCase();
      var v = line.slice(i + 1).trim();
      if (k === 'status') status = parseInt(v, 10) || 500;
      else headers[k] = v;
    });
    return { status: status, headers: headers, body: text.slice(sep + skip) };
  }

  function response(r) {
    return {
      ok: r.status >= 200 && r.status < 300,
      status: r.status,
      headers: { get: function (name) { var v = r.headers[String(name).toLowerCase()]; return v === undefined ? null : v; } },
      text: function () { return Promise.resolve(r.body); },
      json: function () { return Promise.resolve(JSON.parse(r.body)); }
    };
  }

  /** A JSON error reply in the API's envelope (spawn failures). */
  function failure(status, code, message) {
    return response({
      status: status,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ ok: false, error: { code: code, message: message } })
    });
  }

  function headerOf(init, name) {
    var hs = (init && init.headers) || {};
    for (var k in hs) if (k.toLowerCase() === name.toLowerCase()) return hs[k];
    return null;
  }

  /** window.fetch over cockpit.spawn: `url` is the API URL with its query string. */
  function cockpitFetch(url, init) {
    init = init || {};
    var u = String(url);
    var qi = u.indexOf('?');
    var query = qi >= 0 ? u.slice(qi + 1).split('#')[0] : '';
    var method = (init.method || 'GET').toUpperCase();
    var body = init.body == null ? '' : String(init.body);
    var environ = [
      'GATEWAY_INTERFACE=CGI/1.1',
      'REQUEST_METHOD=' + method,
      'QUERY_STRING=' + query,
      'CONTENT_LENGTH=' + byteLength(body)
    ];
    var ct = headerOf(init, 'Content-Type');
    if (ct) environ.push('CONTENT_TYPE=' + ct);
    var xr = headerOf(init, 'X-WebPulse-Request');
    if (xr) environ.push('HTTP_X_WEBPULSE_REQUEST=' + xr);
    return new Promise(function (resolve) {
      var proc;
      try {
        proc = window.cockpit.spawn([BOOT.bin, 'cgi', '--cockpit', '--api'], {
          superuser: 'try', err: 'message', environ: environ
        });
      } catch (e) {
        resolve(failure(502, 'spawn', 'Cannot run ' + BOOT.bin + ': ' + (e && e.message ? e.message : e)));
        return;
      }
      proc.then(function (out) {
        resolve(response(parseCgi(out)));
      }, function (ex, out) {
        // a refused request still prints a CGI reply (exit status 1)
        if (out && /^Status:/i.test(String(out))) { resolve(response(parseCgi(out))); return; }
        var msg = ex && ex.message ? ex.message : String(ex || 'failed');
        if (ex && ex.problem === 'not-found') msg = BOOT.bin + ' was not found: is WebPulse installed?';
        resolve(failure(502, 'spawn', msg));
      });
      proc.input(body);
    });
  }

  function call(params) {
    var q = Object.keys(params).map(function (k) { return encodeURIComponent(k) + '=' + encodeURIComponent(params[k]); }).join('&');
    return cockpitFetch('?' + q, { headers: { Accept: 'application/json' } }).then(function (res) {
      return res.text().then(function (txt) {
        var j = null;
        try { j = JSON.parse(txt); } catch (e) { j = null; }
        if (!j || !j.ok) {
          var err = new Error(j && j.error ? j.error.message : 'HTTP ' + res.status);
          err.code = j && j.error ? j.error.code : 'http_' + res.status;
          throw err;
        }
        return j.data;
      });
    });
  }

  /* ------------------------------------------------------------ theme */
  function shellStyle() {
    try { return window.localStorage.getItem('shell:style') || 'auto'; } catch (e) { return 'auto'; }
  }
  function shellTheme(style) {
    var cl = document.documentElement.classList;
    if (cl.contains('pf-v6-theme-dark') || cl.contains('pf-v5-theme-dark')) return 'dark';
    var s = style || shellStyle();
    return s === 'dark' || s === 'light' ? s : 'auto';
  }
  function followShell() {
    var push = function (style) {
      if (window.WebPulseApp && window.WebPulseApp.setTheme) window.WebPulseApp.setTheme(shellTheme(style));
    };
    window.addEventListener('storage', function (ev) { if (ev.key === 'shell:style') push(ev.newValue); });
    window.addEventListener('cockpit-style', function (ev) { push(ev && ev.detail ? ev.detail.style : null); });
    if (window.MutationObserver) {
      new MutationObserver(function () { push(null); })
        .observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
    }
  }

  /* ------------------------------------------------------------ start */
  function fail(root, err) {
    while (root.firstChild) root.removeChild(root.firstChild);
    var box = document.createElement('div');
    box.className = 'wp-state';
    var h2 = document.createElement('h2');
    h2.textContent = 'Web analytics could not start';
    var p = document.createElement('p');
    p.textContent = err && err.message ? err.message : String(err);
    var hint = document.createElement('p');
    hint.textContent = 'The module runs ' + BOOT.bin + ' cgi --cockpit --api. Check that WebPulse is installed (install.sh --cockpit) and see /var/webpulse/log/webpulse.log.';
    box.appendChild(h2); box.appendChild(p); box.appendChild(hint);
    root.appendChild(box);
    root.setAttribute('data-cockpit-error', '1');
  }

  function start(root, boot) {
    if (!root || !boot) return;
    BOOT = boot;
    if (!window.cockpit || typeof window.cockpit.spawn !== 'function') {
      fail(root, new Error('cockpit.js is not available: open this page from Cockpit.'));
      return;
    }
    boot.defaults = boot.defaults || {};
    boot.defaults.dark = shellTheme(null);
    var user = typeof window.cockpit.user === 'function'
      ? Promise.resolve(window.cockpit.user()).catch(function () { return null; })
      : Promise.resolve(null);
    Promise.all([user, call({ a: 'whoami' })]).then(function (r) {
      var u = r[0], who = r[1];
      boot.principal = who.principal;
      if (who.version) boot.version = who.version;
      if (who.hostname) boot.hostname = who.hostname;
      if (who.geoip_credit) boot.geoip_credit = who.geoip_credit;
      boot.mode = who.mode;
      var name = u && (u.full_name || u.name);
      if (name) {
        boot.userLabel = name + (who.principal.type === 'admin' ? ' (administrative access)' : '');
      }
      var steps = [call({ a: 'domains' }).then(function (d) { boot.domains = d.domains || []; })];
      if (who.principal.type === 'user') {
        steps.push(call({ a: 'user_settings' }).then(function (s) {
          var ui = (s && s.settings && s.settings.ui) || (s && s.ui) || {};
          if (ui.default_range) boot.defaults.range = ui.default_range;
        }, function () { /* defaults */ }));
      }
      return Promise.all(steps);
    }).then(function () {
      root.setAttribute('data-principal', boot.principal.type);
      window.WebPulseApp.start(root, boot);
      followShell();
    }).catch(function (err) { fail(root, err); });
  }

  window.WebPulseCockpit = { start: start, fetch: cockpitFetch, parseCgi: parseCgi };
})();
