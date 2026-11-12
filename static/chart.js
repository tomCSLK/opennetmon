// Draws the chart of the sensor detail page with uPlot, as in the draft: the unit once above each axis,
// horizontal grid lines only, the first channel with a light area and the others dashed, outages tinted
// red, a legend below and a tooltip at the cursor. Channels of a second unit get an axis on the right
// (ping: packet loss as red bars). A click on a legend entry hides or shows its channel; the choice holds
// over the reloads of the tab (sessionStorage, per sensor). Sensors with status values only
// (#charts[data-band]) get a band of their status over the range instead. Data comes from the JSON
// endpoint named in #charts[data-src].
(function () {
  'use strict';
  var root = document.getElementById('charts');
  if (!root || !window.opennetmonFormat) { return; }
  var bandMode = root.hasAttribute('data-band');
  if (!bandMode && !window.uPlot) { return; }

  var REFRESH_MS = 30000;
  var HEIGHT = 240;
  var SVG = 'http://www.w3.org/2000/svg';
  var charts = [];
  var lastData = null;
  // Keys of the channels hidden by a click on the legend.
  var HIDDEN_KEY = 'opennetmon-chart-hidden:' + (root.dataset.src || '').split('?')[0];
  var hidden = [];
  try { hidden = JSON.parse(sessionStorage.getItem(HIDDEN_KEY) || '[]') || []; } catch (e) { hidden = []; }
  function remember() {
    try { sessionStorage.setItem(HIDDEN_KEY, JSON.stringify(hidden)); } catch (e) { /* not remembered */ }
  }
  var F = window.opennetmonFormat;
  var T = window.opennetmonText;

  function css(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }
  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) { e.className = cls; }
    if (text !== undefined) { e.textContent = text; }
    return e;
  }

  function xValues(u, splits, axisIdx, foundSpace, foundIncr) {
    return splits.map(function (ts) {
      if (foundIncr >= 86400) { return F.day(ts); }
      var d = new Date(ts * 1000);
      return (d.getHours() === 0 && d.getMinutes() === 0) ? F.day(ts) : F.clock(ts, foundIncr < 60);
    });
  }

  // Four steps of a round size from 0, e.g. 0, 3, 6, 9, 12 for a largest value of 10.1. The sizes lie close
  // together (at most a third apart), so the largest value fills at least three quarters of the height.
  var STEPS = [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 7.5, 8, 10];
  function niceStep(max) {
    if (!(max > 0)) { return 0.25; }
    var raw = max / 4, e = Math.pow(10, Math.floor(Math.log10(raw))), m = raw / e;
    for (var k = 0; k < STEPS.length; k += 1) { if (m <= STEPS[k] * (1 + 1e-9)) { return STEPS[k] * e; } }
    return 10 * e;
  }

  function inPeriod(data, ts, status) {
    return (data.periods || []).some(function (p) { return p.status === status && ts >= p.from && ts < p.to; });
  }

  function destroyAll() {
    charts.forEach(function (c) { if (c.plot) { c.plot.destroy(); } });
    charts = [];
    root.textContent = '';
  }

  function empty() {
    root.appendChild(el('p', 'muted', root.dataset.empty || ''));
  }

  // ---------- band of status sensors ----------
  function buildBand(data) {
    var span = data.to - data.from;
    if (!(data.periods || []).length || span <= 0) { empty(); return; }
    var svg = document.createElementNS(SVG, 'svg');
    svg.setAttribute('class', 'bigband');
    svg.setAttribute('viewBox', '0 0 1000 36');
    svg.setAttribute('preserveAspectRatio', 'none');
    svg.setAttribute('role', 'img');
    var seen = {}, legend = [];
    data.periods.forEach(function (p) {
      var r = document.createElementNS(SVG, 'rect');
      r.setAttribute('class', p.status);
      r.setAttribute('x', ((p.from - data.from) / span * 1000).toFixed(2));
      r.setAttribute('width', ((p.to - p.from) / span * 1000).toFixed(2));
      r.setAttribute('height', '36');
      var title = document.createElementNS(SVG, 'title');
      title.textContent = p.label + ' · ' + F.dateTime(p.from) + ' – ' + F.dateTime(p.to);
      r.appendChild(title);
      svg.appendChild(r);
      if (!seen[p.status]) { seen[p.status] = true; legend.push(p); }
    });
    svg.setAttribute('aria-label', legend.map(function (p) { return p.label; }).join(', '));
    root.appendChild(svg);
    var ticks = el('div', 'band-axis');
    for (var k = 0; k <= 5; k += 1) {
      var ts = data.from + span * k / 5;
      ticks.appendChild(el('span', '', span > 2 * 86400 ? F.day(ts) : F.clock(ts)));
    }
    root.appendChild(ticks);
    var box = el('div', 'chart-legend');
    legend.forEach(function (p) {
      var item = el('span');
      item.appendChild(el('i', 'sw st-' + p.status));
      item.appendChild(document.createTextNode(p.label));
      box.appendChild(item);
    });
    root.appendChild(box);
  }

  // ---------- chart ----------
  // Raw samples have no rows for polls that failed: a row without values breaks the line there.
  function withGaps(data) {
    var ts = data.ts.slice(), cols = data.series.map(function (s) { return s.values.slice(); });
    if (!data.step_secs && data.interval_secs) {
      var limit = data.interval_secs * 2.5;
      for (var i = ts.length - 1; i > 0; i -= 1) {
        if (ts[i] - ts[i - 1] > limit) {
          ts.splice(i, 0, ts[i - 1] + 1);
          cols.forEach(function (c) { c.splice(i, 0, null); });
        }
      }
    }
    // The chart spans the whole range, also before the first value.
    if (data.from && (!ts.length || ts[0] > data.from)) { ts.unshift(data.from); cols.forEach(function (c) { c.unshift(null); }); }
    if (data.to && ts[ts.length - 1] < data.to) { ts.push(data.to); cols.forEach(function (c) { c.push(null); }); }
    return { ts: ts, cols: cols };
  }

  function maxOf(cols) {
    var m = -Infinity, lo = Infinity;
    cols.forEach(function (c) { c.forEach(function (v) { if (v !== null && isFinite(v)) { m = Math.max(m, v); lo = Math.min(lo, v); } }); });
    return { max: m === -Infinity ? 0 : m, min: lo === Infinity ? 0 : lo };
  }

  // One chart for up to two units: the first on the left axis, the second on the right.
  function buildChart(data, rows, units) {
    var palette = [css('--s1'), css('--s2'), css('--s3'), css('--s4')];
    var err = css('--err'), text3 = css('--text-3'), border = css('--border');
    var font = '11px ' + (css('--font-sans') || 'sans-serif');
    var box = el('div', 'chart');
    var head = el('div', 'chart-units');
    var area = el('div', 'chart-plot');
    var legend = el('div', 'chart-legend');
    box.appendChild(head);
    box.appendChild(area);
    box.appendChild(legend);
    root.appendChild(box);

    var series = [{}], columns = [rows.ts], scales = { x: { time: true } }, axes = [{
      stroke: text3, font: font, grid: { show: false }, ticks: { show: false }, values: xValues, gap: 6, size: 28,
      space: function (u, i, min, max, dim) { return Math.max(80, dim / 7); }
    }];
    var shown = [];
    units.forEach(function (unit, side) {
      var scale = side ? 'y2' : 'y';
      var list = data.series.map(function (s, i) { return { s: s, values: rows.cols[i] }; })
        .filter(function (x) { return x.s.unit === unit; });
      var loss = list.every(function (x) { return x.s.key === 'loss'; });
      var range = maxOf(list.map(function (x) { return x.values; }));
      var state = unit === 'state' || list.some(function (x) { return (x.s.labels || []).length; });
      var ax = F.axis(list[0].s, range.max);
      head.appendChild(el('span', side ? 'right' : '', state ? '' : ax.unit));
      // From 0 in four round steps in the unit shown (days, Mbit/s …), over the channels shown; loss 0–100 %.
      var fixed = !state && range.min >= 0;
      scales[scale] = !fixed ? {} : {
        range: loss ? function () { return [0, 100]; }
          : function (u, min, max) { return [0, 4 * niceStep((max || 0) / ax.div) * ax.div]; }
      };
      axes.push({
        scale: scale, side: side ? 1 : 3, stroke: text3, font: font, gap: 6, size: state ? 110 : 52,
        grid: side ? { show: false } : { stroke: border, width: 1 }, ticks: { show: false },
        splits: fixed ? function (u, i, min, max) { return [0, 1, 2, 3, 4].map(function (k) { return max * k / 4; }); } : undefined,
        values: state ? (function (f) { return function (u, vals) { return vals.map(f); }; })(F.forSeries(list[0].s))
          : function (u, vals) { return vals.map(function (v) { return F.num(v / ax.div, 3); }); }
      });
      list.forEach(function (x) {
        var n = shown.length, color = loss ? err : palette[n % palette.length];
        var opts = {
          label: x.s.label, scale: scale, stroke: color, width: 1.6, spanGaps: false,
          points: { show: false }, show: hidden.indexOf(x.s.key) < 0
        };
        if (loss) {
          opts.fill = err + '59';
          opts.paths = uPlot.paths.bars({ size: [0.7, 12] });
        } else if (state) {
          opts.paths = uPlot.paths.stepped({ align: 1 });
        } else if (n === 0) {
          opts.fill = color + '1a';
        } else {
          opts.dash = [5, 3];
        }
        series.push(opts);
        // Ping: no bar where nothing was lost.
        columns.push(loss ? x.values.map(function (v) { return v === 0 ? null : v; }) : x.values);
        shown.push({ s: x.s, format: F.forSeries(x.s), cls: loss ? 'loss' : 'lg-' + (n % 4 + 1) + (n && !state ? ' dash' : '') });
      });
    });
    var buttons = shown.map(function (x) {
      var item = el('button');
      item.type = 'button';
      item.title = T('chart-toggle');
      item.setAttribute('aria-pressed', String(hidden.indexOf(x.s.key) < 0));
      item.appendChild(el('i', 'sw ' + x.cls));
      item.appendChild(document.createTextNode(x.s.label));
      legend.appendChild(item);
      return item;
    });

    var tip = el('div', 'ctip');
    var plot = new uPlot({
      width: Math.max(area.clientWidth, 260),
      height: HEIGHT,
      legend: { show: false },
      cursor: { y: false, points: { size: 6 }, drag: { x: true, y: false } },
      scales: scales,
      series: series,
      axes: axes,
      hooks: {
        init: [function (u) { u.over.appendChild(tip); }],
        // Outages tinted red behind the grid and the lines.
        drawClear: [function (u) {
          var ctx = u.ctx, min = u.scales.x.min, max = u.scales.x.max;
          ctx.save();
          ctx.fillStyle = css('--err-bg');
          (data.periods || []).forEach(function (p) {
            if (p.status !== 'down' || p.to <= min || p.from >= max) { return; }
            var a = u.valToPos(Math.max(p.from, min), 'x', true), b = u.valToPos(Math.min(p.to, max), 'x', true);
            ctx.fillRect(a, u.bbox.top, Math.max(b - a, 1), u.bbox.height);
          });
          ctx.restore();
        }],
        setCursor: [function (u) {
          var i = u.cursor.idx;
          if (i === null || i === undefined || u.cursor.left < 0) { tip.classList.remove('on'); return; }
          var ts = u.data[0][i];
          tip.textContent = '';
          tip.appendChild(el('div', 't', F.dateTime(ts, !data.step_secs)));
          var any = false;
          shown.forEach(function (x, k) {
            var v = u.data[k + 1][i];
            if (v === null || v === undefined || !u.series[k + 1].show) { return; }
            any = true;
            var line = el('div');
            line.appendChild(document.createTextNode(x.s.label + ' '));
            line.appendChild(el('b', '', x.format(v)));
            tip.appendChild(line);
          });
          if (!any) { tip.appendChild(el('div', '', inPeriod(data, ts, 'down') ? T('chart-no-answer') : '–')); }
          tip.classList.add('on');
          var w = tip.offsetWidth, left = u.cursor.left + 12;
          if (left + w > u.over.clientWidth) { left = u.cursor.left - w - 12; }
          tip.style.left = Math.max(0, left) + 'px';
          tip.style.top = '8px';
        }]
      }
    }, columns, area);
    buttons.forEach(function (b, k) {
      b.addEventListener('click', function () {
        var key = shown[k].s.key, show = !plot.series[k + 1].show;
        plot.setSeries(k + 1, { show: show });
        b.setAttribute('aria-pressed', String(show));
        hidden = hidden.filter(function (h) { return h !== key; });
        if (!show) { hidden.push(key); }
        remember();
      });
    });
    charts.push({ plot: plot, area: area });
  }

  function build(data) {
    lastData = data;
    destroyAll();
    if (bandMode) { buildBand(data); return; }
    if (!data.ts.length) { empty(); return; }
    var rows = withGaps(data);
    var units = [];
    data.series.forEach(function (s) { if (units.indexOf(s.unit) < 0) { units.push(s.unit); } });
    for (var k = 0; k < units.length; k += 2) { buildChart(data, rows, units.slice(k, k + 2)); }
  }

  function load() {
    fetch(root.dataset.src, { credentials: 'same-origin', headers: { Accept: 'application/json' } })
      .then(function (r) { if (!r.ok) { throw new Error(r.status); } return r.json(); })
      .then(function (data) {
        build(data);
        // The table of values (static/samples.js) reloads with the chart.
        window.dispatchEvent(new CustomEvent('opennetmon:data', { detail: data }));
      })
      .catch(function () { /* keep the last picture; the next refresh retries */ });
  }

  window.addEventListener('resize', function () {
    charts.forEach(function (c) { c.plot.setSize({ width: Math.max(c.area.clientWidth, 260), height: HEIGHT }); });
  });
  // Colors come from CSS variables: redraw when the color scheme changes.
  window.addEventListener('opennetmon:theme', function () { if (lastData) { build(lastData); } });
  load();
  setInterval(load, REFRESH_MS);
})();
