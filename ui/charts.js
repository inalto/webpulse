/* WebPulse charts — hand-written SVG charts, no dependencies.
 * Every chart: responsive (ResizeObserver), colours from CSS custom
 * properties (light/dark), a single shared tooltip, keyboard focus, and a
 * visually hidden <table> twin for screen readers. Untrusted strings are
 * only ever inserted with textContent.
 */
(function () {
  'use strict';

  var NS = 'http://www.w3.org/2000/svg';
  var uid = 0;

  /* ---------------------------------------------------------------- format */
  var nf = new Intl.NumberFormat(undefined);
  var nf1 = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
  var nf2 = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
  var compactNf;
  try { compactNf = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }); } catch (e) { compactNf = null; }

  var fmt = {
    num: function (v) { return v == null || isNaN(v) ? '–' : nf.format(Math.round(v)); },
    dec: function (v) { return v == null || isNaN(v) ? '–' : nf2.format(v); },
    compact: function (v) {
      if (v == null || isNaN(v)) return '–';
      if (Math.abs(v) < 1000) return nf1.format(v);
      if (compactNf) return compactNf.format(v);
      var u = ['', 'K', 'M', 'B', 'T'], i = 0;
      while (Math.abs(v) >= 1000 && i < u.length - 1) { v /= 1000; i++; }
      return nf1.format(v) + u[i];
    },
    bytes: function (v) {
      if (v == null || isNaN(v)) return '–';
      var u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'], i = 0;
      while (Math.abs(v) >= 1024 && i < u.length - 1) { v /= 1024; i++; }
      return (i === 0 ? Math.round(v) : (v >= 100 ? Math.round(v) : nf1.format(v))) + ' ' + u[i];
    },
    duration: function (s) {
      if (s == null || isNaN(s)) return '–';
      s = Math.round(s);
      if (s < 60) return s + 's';
      if (s < 3600) return Math.floor(s / 60) + 'm ' + (s % 60) + 's';
      return Math.floor(s / 3600) + 'h ' + Math.floor((s % 3600) / 60) + 'm';
    },
    pct: function (v, digits) {
      if (v == null || isNaN(v)) return '–';
      if (v === 0) return '0%';
      var d = digits == null ? (Math.abs(v) < 0.1 ? 1 : 0) : digits;
      return (v * 100).toFixed(d) + '%';
    },
    signedPct: function (v) {
      if (v == null || isNaN(v)) return '–';
      var p = Math.abs(v * 100);
      var s = p >= 100 ? Math.round(p).toString() : p.toFixed(1);
      return (v > 0 ? '+' : v < 0 ? '−' : '') + s + '%';
    }
  };

  /** Format a value by unit name: count | bytes | duration | ratio | decimal. */
  function byUnit(unit, v, axis) {
    if (unit === 'bytes') return fmt.bytes(v);
    if (unit === 'duration') return fmt.duration(v);
    if (unit === 'ratio') return fmt.pct(v);
    if (unit === 'decimal') return fmt.dec(v);
    return axis ? fmt.compact(v) : fmt.num(v);
  }

  /* --------------------------------------------------------------- helpers */
  function s(tag, attrs, parent) {
    var e = document.createElementNS(NS, tag);
    if (attrs) {
      for (var k in attrs) {
        if (!Object.prototype.hasOwnProperty.call(attrs, k) || attrs[k] == null) continue;
        if (k === 'css') { for (var p in attrs.css) e.style.setProperty(p, attrs.css[p]); }
        else if (k === 'text') e.textContent = attrs.text;
        else e.setAttribute(k, attrs[k]);
      }
    }
    if (parent) parent.appendChild(e);
    return e;
  }
  function h(tag, cls, text, parent) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    if (parent) parent.appendChild(e);
    return e;
  }
  function clear(node) { while (node.firstChild) node.removeChild(node.firstChild); }

  /** "Nice" axis maximum and tick step. */
  function niceScale(max, ticks) {
    ticks = ticks || 4;
    if (!(max > 0)) return { max: 1, step: 0.25 };
    var raw = max / ticks;
    var mag = Math.pow(10, Math.floor(Math.log10(raw)));
    var norm = raw / mag;
    var step = (norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 2.5 ? 2.5 : norm <= 5 ? 5 : 10) * mag;
    return { max: Math.ceil(max / step - 1e-9) * step, step: step };
  }
  function bytesScale(max, ticks) {
    if (!(max > 0)) return { max: 1, step: 0.25 };
    var p = Math.pow(1024, Math.max(0, Math.floor(Math.log(max) / Math.log(1024))));
    var n = niceScale(max / p, ticks);
    return { max: n.max * p, step: n.step * p };
  }
  function textWidth(str, px) { return String(str).length * (px || 11) * 0.6; }

  /* --------------------------------------------------------------- tooltip */
  var tip = null;
  function tooltip() {
    if (tip) return tip;
    tip = h('div', 'wp-tip');
    tip.setAttribute('role', 'tooltip');
    tip.hidden = true;
    var root = document.querySelector('.wp-root') || document.body;
    root.appendChild(tip);
    return tip;
  }
  /**
   * rows: [{value, label, color, dashed}] — value first (strong), label secondary.
   */
  function showTip(clientX, clientY, title, rows) {
    var t = tooltip();
    clear(t);
    if (title) h('div', 'wp-tip-title', title, t);
    rows.forEach(function (r) {
      var row = h('div', 'wp-tip-row', null, t);
      if (r.color) {
        var k = h('span', 'wp-tip-key' + (r.dashed ? ' is-dashed' : ''), null, row);
        k.style.setProperty('--k', r.color);
      }
      h('strong', 'wp-tip-val', r.value, row);
      if (r.label) h('span', 'wp-tip-lab', r.label, row);
    });
    t.hidden = false;
    var w = t.offsetWidth, hh = t.offsetHeight;
    var vw = document.documentElement.clientWidth, vh = document.documentElement.clientHeight;
    var x = clientX + 14, y = clientY + 14;
    if (x + w > vw - 8) x = clientX - w - 14;
    if (x < 8) x = 8;
    if (y + hh > vh - 8) y = clientY - hh - 14;
    if (y < 8) y = 8;
    t.style.left = x + 'px';
    t.style.top = y + 'px';
  }
  function hideTip() { if (tip) tip.hidden = true; }

  /** Tooltip anchored on an element (keyboard focus). */
  function tipAt(elm, title, rows) {
    var r = elm.getBoundingClientRect();
    showTip(r.left + r.width / 2, r.top, title, rows);
  }

  /* ----------------------------------------------------- sr-only table twin */
  function srTable(container, caption, headers, rows) {
    var wrap = h('div', 'sr-only', null, container);
    var t = h('table', null, null, wrap);
    h('caption', null, caption, t);
    var tr = h('tr', null, null, h('thead', null, null, t));
    headers.forEach(function (x) { h('th', null, x, tr).setAttribute('scope', 'col'); });
    var tb = h('tbody', null, null, t);
    rows.forEach(function (r) {
      var row = h('tr', null, null, tb);
      r.forEach(function (c, i) { h(i === 0 ? 'th' : 'td', null, c, row); });
    });
    return t;
  }

  /* ------------------------------------------------------------ mounting */
  /**
   * Renders `draw(width)` into `container` now and on every resize.
   */
  function mount(container, draw) {
    if (container.__wpRO) container.__wpRO.disconnect();
    var lastW = -1, frame = 0;
    function run() {
      var w = Math.floor(container.clientWidth);
      if (w <= 0 || w === lastW) return;
      lastW = w;
      clear(container);
      draw(w);
    }
    run();
    if (lastW <= 0) {
      lastW = -1;
      clear(container);
      draw(600);
    }
    if (typeof ResizeObserver === 'function') {
      var ro = new ResizeObserver(function () {
        cancelAnimationFrame(frame);
        frame = requestAnimationFrame(run);
      });
      ro.observe(container);
      container.__wpRO = ro;
    }
  }

  function svgRoot(parent, w, hgt, label) {
    var root = s('svg', { width: w, height: hgt, viewBox: '0 0 ' + w + ' ' + hgt, class: 'wp-svg', role: 'img', 'aria-label': label || '' }, parent);
    return root;
  }

  /* ------------------------------------------------------------ line/area */
  /**
   * opts: {labels, series:[{name, values, color, dashed, area, unit}], unit,
   *        height, weekend (labels are YYYY-MM-DD), xTick(label,i), title(label,i),
   *        caption, stacked}
   */
  function line(container, opts) {
    var labels = opts.labels || [];
    var series = (opts.series || []).filter(function (x) { return x && x.values; });
    var unit = opts.unit || 'count';
    var H = opts.height || 240;
    var n = labels.length;

    mount(container, function (W) {
      var stacked = !!opts.stacked;
      var tops = [];
      if (stacked) {
        var acc = new Array(n).fill(0);
        series.forEach(function (sr) {
          var t = sr.values.map(function (v, i) { acc[i] += (v || 0); return acc[i]; });
          tops.push(t);
        });
      }
      var max = 0;
      series.forEach(function (sr, si) {
        (stacked ? tops[si] : sr.values).forEach(function (v) { if (v != null && v > max) max = v; });
      });
      var sc = unit === 'bytes' ? bytesScale(max, 4) : unit === 'ratio' ? niceScale(Math.max(max, 0.01), 4) : niceScale(max, 4);
      var tickVals = [];
      for (var tv = 0; tv <= sc.max + 1e-9; tv += sc.step) tickVals.push(tv);
      var mL = Math.max(28, Math.max.apply(null, tickVals.map(function (v) { return textWidth(byUnit(unit, v, true)); })) + 10);
      var mR = 10, mT = 10, mB = 24;
      var pw = Math.max(10, W - mL - mR), ph = H - mT - mB;
      var x = function (i) { return n <= 1 ? mL + pw / 2 : mL + (i * pw) / (n - 1); };
      var y = function (v) { return mT + ph - (v / sc.max) * ph; };
      var root = svgRoot(container, W, H, opts.caption);

      // weekend shading
      if (opts.weekend && n > 1) {
        var step = pw / (n - 1);
        labels.forEach(function (lab, i) {
          var d = new Date(lab + 'T12:00:00');
          var wd = d.getDay();
          if (wd === 0 || wd === 6) {
            s('rect', { x: Math.max(mL, x(i) - step / 2), y: mT, width: Math.min(step, pw), height: ph, class: 'wp-weekend' }, root);
          }
        });
      }
      // grid + y ticks
      tickVals.forEach(function (v) {
        s('line', { x1: mL, x2: mL + pw, y1: y(v), y2: y(v), class: v === 0 ? 'wp-axis' : 'wp-gridline' }, root);
        s('text', { x: mL - 8, y: y(v) + 4, 'text-anchor': 'end', class: 'wp-tick', text: byUnit(unit, v, true) }, root);
      });
      // x ticks
      var every = Math.max(1, Math.ceil(n / Math.max(2, Math.floor(pw / 74))));
      for (var i = 0; i < n; i += every) {
        var lx = x(i);
        var anchor = i === 0 && n > 1 ? 'start' : 'middle';
        s('text', { x: lx, y: H - 6, 'text-anchor': anchor, class: 'wp-tick', text: opts.xTick ? opts.xTick(labels[i], i) : labels[i] }, root);
      }

      // series (draw in reverse so the first series is on top)
      var paths = [];
      for (var si = series.length - 1; si >= 0; si--) {
        var sr = series[si];
        var vals = stacked ? tops[si] : sr.values;
        var d = '', started = false, pts = [];
        vals.forEach(function (v, j) {
          if (v == null) { started = false; return; }
          d += (started ? 'L' : 'M') + x(j).toFixed(1) + ' ' + y(v).toFixed(1);
          started = true;
          pts.push([x(j), y(v)]);
        });
        if (!d) continue;
        if ((sr.area || stacked) && pts.length > 1) {
          var base;
          if (stacked && si > 0) {
            var below = tops[si - 1];
            base = '';
            for (var b = n - 1; b >= 0; b--) base += 'L' + x(b).toFixed(1) + ' ' + y(below[b] || 0).toFixed(1);
          } else {
            base = 'L' + pts[pts.length - 1][0].toFixed(1) + ' ' + y(0) + 'L' + pts[0][0].toFixed(1) + ' ' + y(0);
          }
          s('path', { d: d + base + 'Z', class: stacked ? 'wp-stack' : 'wp-area', css: { fill: sr.color } }, root);
        }
        paths.push(s('path', { d: d, class: 'wp-line' + (sr.dashed ? ' is-dashed' : '') + (stacked ? ' is-stack' : ''), css: { stroke: sr.color } }, root));
      }

      // hover layer
      var cross = s('line', { x1: 0, x2: 0, y1: mT, y2: mT + ph, class: 'wp-cross', visibility: 'hidden' }, root);
      var dots = series.map(function (sr) { return s('circle', { r: 4, class: 'wp-dot', visibility: 'hidden', css: { fill: sr.color } }, root); });
      var hit = s('rect', { x: mL, y: mT, width: pw, height: ph, class: 'wp-hit', tabindex: 0, 'aria-label': (opts.caption || 'Chart') + ' — use arrow keys to read values' }, root);
      var cur = -1;
      function at(i, cx, cy) {
        if (i < 0 || i >= n) return;
        cur = i;
        var px = x(i);
        cross.setAttribute('x1', px); cross.setAttribute('x2', px); cross.setAttribute('visibility', 'visible');
        var rows = [];
        series.forEach(function (sr, k) {
          var v = sr.values[i];
          var top = stacked ? tops[k][i] : v;
          if (v == null) { dots[k].setAttribute('visibility', 'hidden'); return; }
          dots[k].setAttribute('cx', px); dots[k].setAttribute('cy', y(top)); dots[k].setAttribute('visibility', 'visible');
          rows.push({ value: byUnit(sr.unit || unit, v), label: sr.name, color: sr.color, dashed: sr.dashed });
        });
        if (stacked) rows.reverse();
        var title = opts.title ? opts.title(labels[i], i) : labels[i];
        if (cx != null) showTip(cx, cy, title, rows);
        else {
          var r = root.getBoundingClientRect();
          showTip(r.left + px, r.top + mT + 10, title, rows);
        }
      }
      function out() {
        cross.setAttribute('visibility', 'hidden');
        dots.forEach(function (d) { d.setAttribute('visibility', 'hidden'); });
        hideTip();
      }
      hit.addEventListener('pointermove', function (ev) {
        var r = root.getBoundingClientRect();
        var rel = ev.clientX - r.left - mL;
        var i = n <= 1 ? 0 : Math.round((rel / pw) * (n - 1));
        at(Math.max(0, Math.min(n - 1, i)), ev.clientX, ev.clientY);
      });
      hit.addEventListener('pointerleave', out);
      hit.addEventListener('blur', out);
      hit.addEventListener('focus', function () { at(cur < 0 ? n - 1 : cur); });
      hit.addEventListener('keydown', function (ev) {
        if (ev.key === 'ArrowLeft' || ev.key === 'ArrowRight' || ev.key === 'Home' || ev.key === 'End') {
          ev.preventDefault();
          var nx = ev.key === 'Home' ? 0 : ev.key === 'End' ? n - 1 : cur + (ev.key === 'ArrowLeft' ? -1 : 1);
          at(Math.max(0, Math.min(n - 1, nx)));
        } else if (ev.key === 'Escape') { out(); }
      });
    });

    // table twin (outside the redraw so it is built once)
    var twin = h('div', 'wp-twin', null, null);
    srTable(twin, opts.caption || 'Chart data', [opts.xName || 'Date'].concat(series.map(function (x) { return x.name; })),
      labels.map(function (lab, i) { return [opts.title ? opts.title(lab, i) : lab].concat(series.map(function (x) { return byUnit(x.unit || unit, x.values[i]); })); }));
    container.parentNode && container.parentNode.insertBefore(twin, container.nextSibling);
  }

  /* ----------------------------------------------------------------- bars */
  /**
   * Vertical columns. opts: {labels, values, color, unit, height, xTick(lab,i), title(lab,i),
   *                          caption, highlight: index, every}
   */
  function bars(container, opts) {
    var labels = opts.labels, values = opts.values, n = labels.length;
    var H = opts.height || 180, unit = opts.unit || 'count';
    mount(container, function (W) {
      var max = Math.max.apply(null, values.concat([0]));
      var sc = niceScale(max, 3);
      var ticks = [];
      for (var tv = 0; tv <= sc.max + 1e-9; tv += sc.step) ticks.push(tv);
      var mL = Math.max(24, Math.max.apply(null, ticks.map(function (v) { return textWidth(byUnit(unit, v, true)); })) + 8);
      var mR = 4, mT = 8, mB = 22;
      var pw = W - mL - mR, ph = H - mT - mB;
      var band = pw / n;
      var bw = Math.max(2, Math.min(24, band - 2));
      var y = function (v) { return mT + ph - (v / sc.max) * ph; };
      var root = svgRoot(container, W, H, opts.caption);
      ticks.forEach(function (v) {
        s('line', { x1: mL, x2: mL + pw, y1: y(v), y2: y(v), class: v === 0 ? 'wp-axis' : 'wp-gridline' }, root);
        s('text', { x: mL - 6, y: y(v) + 4, 'text-anchor': 'end', class: 'wp-tick', text: byUnit(unit, v, true) }, root);
      });
      var tickW = Math.max.apply(null, labels.map(function (l, i) { return textWidth(opts.xTick ? opts.xTick(l, i) : l); }).concat([10]));
      var every = opts.every || Math.max(1, Math.ceil(n / Math.max(2, Math.floor(pw / (tickW + 8)))));
      values.forEach(function (v, i) {
        var cx = mL + band * i + band / 2;
        var top = y(v), hgt = mT + ph - top;
        var r = Math.min(4, bw / 2, hgt);
        var x0 = cx - bw / 2, x1 = cx + bw / 2, yb = mT + ph;
        var d = hgt <= 0 ? '' : 'M' + x0 + ' ' + yb + 'L' + x0 + ' ' + (top + r) + 'Q' + x0 + ' ' + top + ' ' + (x0 + r) + ' ' + top +
          'L' + (x1 - r) + ' ' + top + 'Q' + x1 + ' ' + top + ' ' + x1 + ' ' + (top + r) + 'L' + x1 + ' ' + yb + 'Z';
        if (d) s('path', { d: d, class: 'wp-bar' + (opts.highlight === i ? ' is-hl' : ''), css: { fill: opts.color || 'var(--wp-s1)' } }, root);
        if (i % every === 0) s('text', { x: cx, y: H - 6, 'text-anchor': 'middle', class: 'wp-tick', text: opts.xTick ? opts.xTick(labels[i], i) : labels[i] }, root);
        var hit = s('rect', { x: mL + band * i, y: mT, width: band, height: ph, class: 'wp-hit', tabindex: i === 0 ? 0 : -1 }, root);
        var title = opts.title ? opts.title(labels[i], i) : labels[i];
        var rows = [{ value: byUnit(unit, v), label: opts.name || '' }];
        hit.addEventListener('pointermove', function (ev) { showTip(ev.clientX, ev.clientY, title, rows); });
        hit.addEventListener('pointerleave', hideTip);
        hit.addEventListener('focus', function () { tipAt(hit, title, rows); });
        hit.addEventListener('blur', hideTip);
        hit.addEventListener('keydown', function (ev) { rovingKeys(ev, root); });
      });
    });
    var twin = h('div', 'wp-twin');
    srTable(twin, opts.caption || 'Chart data', [opts.xName || 'Bucket', opts.name || 'Value'],
      labels.map(function (lab, i) { return [opts.title ? opts.title(lab, i) : lab, byUnit(unit, values[i])]; }));
    container.parentNode && container.parentNode.insertBefore(twin, container.nextSibling);
  }

  /** Arrow keys move focus between sibling hit targets. */
  function rovingKeys(ev, root) {
    var keys = { ArrowLeft: -1, ArrowUp: -1, ArrowRight: 1, ArrowDown: 1 };
    if (!(ev.key in keys)) { if (ev.key === 'Escape') hideTip(); return; }
    ev.preventDefault();
    var hits = Array.prototype.slice.call(root.querySelectorAll('.wp-hit'));
    var i = hits.indexOf(ev.target) + keys[ev.key];
    if (i >= 0 && i < hits.length) {
      ev.target.setAttribute('tabindex', '-1');
      hits[i].setAttribute('tabindex', '0');
      hits[i].focus();
    }
  }

  /* ---------------------------------------------------------------- donut */
  /**
   * opts: {items:[{label, value, color}], unit, size, center:{value, label}, caption}
   */
  function donut(container, opts) {
    var items = opts.items.filter(function (x) { return x.value > 0; });
    var total = items.reduce(function (a, b) { return a + b.value; }, 0);
    var size = opts.size || 168, unit = opts.unit || 'count';
    clear(container);
    var root = svgRoot(container, size, size, opts.caption);
    root.classList.add('wp-donut');
    var cx = size / 2, cy = size / 2, R = size / 2 - 2, r = R - Math.max(14, size * 0.13);
    var a0 = -Math.PI / 2;
    function pt(rad, a) { return [cx + rad * Math.cos(a), cy + rad * Math.sin(a)]; }
    if (!total) s('circle', { cx: cx, cy: cy, r: (R + r) / 2, class: 'wp-donut-empty', 'stroke-width': R - r }, root);
    items.forEach(function (it, i) {
      var frac = it.value / total;
      var a1 = a0 + frac * Math.PI * 2;
      var large = a1 - a0 > Math.PI ? 1 : 0;
      var d;
      if (frac >= 0.9999) {
        d = 'M' + (cx - R) + ' ' + cy + 'A' + R + ' ' + R + ' 0 1 1 ' + (cx + R) + ' ' + cy + 'A' + R + ' ' + R + ' 0 1 1 ' + (cx - R) + ' ' + cy +
          'M' + (cx - r) + ' ' + cy + 'A' + r + ' ' + r + ' 0 1 0 ' + (cx + r) + ' ' + cy + 'A' + r + ' ' + r + ' 0 1 0 ' + (cx - r) + ' ' + cy + 'Z';
      } else {
        var p0 = pt(R, a0), p1 = pt(R, a1), p2 = pt(r, a1), p3 = pt(r, a0);
        d = 'M' + p0[0] + ' ' + p0[1] + 'A' + R + ' ' + R + ' 0 ' + large + ' 1 ' + p1[0] + ' ' + p1[1] +
          'L' + p2[0] + ' ' + p2[1] + 'A' + r + ' ' + r + ' 0 ' + large + ' 0 ' + p3[0] + ' ' + p3[1] + 'Z';
      }
      var seg = s('path', { d: d, class: 'wp-seg wp-hit-seg', tabindex: i === 0 ? 0 : -1, css: { fill: it.color } }, root);
      seg.classList.add('wp-hit');
      var rows = [{ value: byUnit(unit, it.value) + ' · ' + fmt.pct(frac), label: '' }];
      seg.addEventListener('pointermove', function (ev) { showTip(ev.clientX, ev.clientY, it.label, rows); });
      seg.addEventListener('pointerleave', hideTip);
      seg.addEventListener('focus', function () { tipAt(seg, it.label, rows); });
      seg.addEventListener('blur', hideTip);
      seg.addEventListener('keydown', function (ev) { rovingKeys(ev, root); });
      a0 = a1;
    });
    if (opts.center) {
      s('text', { x: cx, y: cy + 2, 'text-anchor': 'middle', class: 'wp-donut-val', text: opts.center.value }, root);
      s('text', { x: cx, y: cy + 20, 'text-anchor': 'middle', class: 'wp-donut-lab', text: opts.center.label }, root);
    }
    srTable(container, opts.caption || 'Breakdown', ['Item', 'Value', 'Share'],
      items.map(function (it) { return [it.label, byUnit(unit, it.value), fmt.pct(it.value / total)]; }));
  }

  /* -------------------------------------------------------------- heatmap */
  /**
   * opts: {rows:[labels], cols:[labels], values:[[...]], caption, unit, colTick(c), title(r,c)}
   */
  function heatmap(container, opts) {
    var rows = opts.rows, cols = opts.cols, vals = opts.values, unit = opts.unit || 'count';
    var max = 0;
    vals.forEach(function (r) { r.forEach(function (v) { if (v > max) max = v; }); });
    mount(container, function (W) {
      var mL = 40, mT = 4, mB = 20, gap = 2;
      var cw = Math.max(6, (W - mL) / cols.length);
      var ch = Math.min(36, Math.max(14, cw * 0.9));
      var H = mT + rows.length * ch + mB;
      var root = svgRoot(container, W, H, opts.caption);
      rows.forEach(function (rl, ri) {
        s('text', { x: mL - 8, y: mT + ri * ch + ch / 2 + 4, 'text-anchor': 'end', class: 'wp-tick', text: rl }, root);
        cols.forEach(function (cl, ci) {
          var v = vals[ri][ci] || 0;
          var step = v <= 0 || !max ? 0 : Math.min(7, 1 + Math.floor((v / max) * 6.999));
          var cell = s('rect', { x: mL + ci * cw + gap / 2, y: mT + ri * ch + gap / 2, width: Math.max(1, cw - gap), height: ch - gap, rx: 2,
            class: 'wp-cell wp-hit', tabindex: ri === 0 && ci === 0 ? 0 : -1, css: { fill: step ? 'var(--wp-seq-' + step + ')' : 'var(--wp-seq-0)' } }, root);
          var title = opts.title ? opts.title(ri, ci) : rl + ' ' + cl;
          var trs = [{ value: byUnit(unit, v), label: opts.name || '' }];
          cell.addEventListener('pointermove', function (ev) { showTip(ev.clientX, ev.clientY, title, trs); });
          cell.addEventListener('pointerleave', hideTip);
          cell.addEventListener('focus', function () { tipAt(cell, title, trs); });
          cell.addEventListener('blur', hideTip);
          cell.addEventListener('keydown', function (ev) { rovingKeys(ev, root); });
        });
      });
      var every = Math.max(1, Math.ceil(cols.length / Math.max(2, Math.floor((W - mL) / 30))));
      cols.forEach(function (cl, ci) {
        if (ci % every === 0) s('text', { x: mL + ci * cw + cw / 2, y: H - 5, 'text-anchor': 'middle', class: 'wp-tick', text: opts.colTick ? opts.colTick(cl, ci) : cl }, root);
      });
    });
    var twin = h('div', 'wp-twin');
    srTable(twin, opts.caption || 'Heatmap', [''].concat(cols), rows.map(function (rl, ri) { return [rl].concat(vals[ri].map(function (v) { return byUnit(unit, v); })); }));
    container.parentNode && container.parentNode.insertBefore(twin, container.nextSibling);
  }

  /** Legend for the heatmap's sequential scale. */
  function seqLegend(parent, maxLabel) {
    var w = h('div', 'wp-seq-legend', null, parent);
    h('span', null, '0', w);
    for (var i = 0; i <= 7; i++) { var sw = h('i', null, null, w); sw.style.setProperty('background', 'var(--wp-seq-' + i + ')'); }
    h('span', null, maxLabel, w);
    return w;
  }

  /* ------------------------------------------------------------ sparkline */
  function sparkline(container, values, opts) {
    opts = opts || {};
    var H = opts.height || 28;
    mount(container, function (W) {
      var vals = values.filter(function (v) { return v != null; });
      var max = Math.max.apply(null, vals.concat([0])), min = opts.zero === false ? Math.min.apply(null, vals.concat([max])) : 0;
      var span = max - min || 1;
      var n = values.length;
      var root = svgRoot(container, W, H, opts.caption || '');
      root.setAttribute('aria-hidden', 'true');
      var x = function (i) { return n <= 1 ? W / 2 : 2 + (i * (W - 6)) / (n - 1); };
      var y = function (v) { return 3 + (H - 6) - ((v - min) / span) * (H - 6); };
      var d = '', started = false, last = null;
      values.forEach(function (v, i) {
        if (v == null) { started = false; return; }
        d += (started ? 'L' : 'M') + x(i).toFixed(1) + ' ' + y(v).toFixed(1);
        started = true;
        last = [x(i), y(v)];
      });
      if (!d) return;
      if (opts.area) s('path', { d: d + 'L' + last[0] + ' ' + (H - 1) + 'L' + x(0) + ' ' + (H - 1) + 'Z', class: 'wp-spark-area', css: { fill: opts.color || 'var(--wp-s1)' } }, root);
      s('path', { d: d, class: 'wp-spark', css: { stroke: opts.color || 'var(--wp-spark)' } }, root);
      if (last && opts.dot !== false) s('circle', { cx: last[0], cy: last[1], r: 2.5, class: 'wp-spark-dot', css: { fill: opts.dotColor || opts.color || 'var(--wp-s1)' } }, root);
    });
  }

  /* -------------------------------------------------- stacked single bar */
  /**
   * One horizontal 100% bar with 2px gaps. opts: {items:[{label,value,color}], unit, caption, height}
   */
  function stackBar(container, opts) {
    var items = opts.items.filter(function (x) { return x.value > 0; });
    var total = items.reduce(function (a, b) { return a + b.value; }, 0);
    var H = opts.height || 14, unit = opts.unit || 'count';
    mount(container, function (W) {
      var root = svgRoot(container, W, H, opts.caption);
      if (!total) { s('rect', { x: 0, y: 0, width: W, height: H, rx: 4, class: 'wp-donut-empty-fill' }, root); return; }
      var gap = 2, avail = W - gap * (items.length - 1), x0 = 0;
      items.forEach(function (it, i) {
        var w = Math.max(1, (it.value / total) * avail);
        var seg = s('rect', { x: x0, y: 0, width: w, height: H, rx: Math.min(4, w / 2), class: 'wp-hit wp-seg', tabindex: i === 0 ? 0 : -1, css: { fill: it.color } }, root);
        var rows = [{ value: byUnit(unit, it.value) + ' · ' + fmt.pct(it.value / total), label: '' }];
        seg.addEventListener('pointermove', function (ev) { showTip(ev.clientX, ev.clientY, it.label, rows); });
        seg.addEventListener('pointerleave', hideTip);
        seg.addEventListener('focus', function () { tipAt(seg, it.label, rows); });
        seg.addEventListener('blur', hideTip);
        seg.addEventListener('keydown', function (ev) { rovingKeys(ev, root); });
        x0 += w + gap;
      });
    });
    var twin = h('div', 'wp-twin');
    srTable(twin, opts.caption || 'Breakdown', ['Item', 'Value', 'Share'],
      items.map(function (it) { return [it.label, byUnit(unit, it.value), fmt.pct(it.value / total)]; }));
    container.parentNode && container.parentNode.insertBefore(twin, container.nextSibling);
  }

  /** Categorical colour for slot i (0-based), never cycled past 8. */
  function color(i) { return 'var(--wp-s' + (Math.min(i, 7) + 1) + ')'; }

  window.WebPulseCharts = {
    fmt: fmt, byUnit: byUnit, line: line, bars: bars, donut: donut, heatmap: heatmap, seqLegend: seqLegend,
    sparkline: sparkline, stackBar: stackBar, color: color, showTip: showTip, hideTip: hideTip, tipAt: tipAt,
    srTable: srTable, h: h, clear: clear
  };
})();
