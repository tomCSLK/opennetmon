// Table of measured values below the chart (like PRTG's "Live data" table): one row per point,
// newest first, with average/minimum/maximum (and the summed volume for traffic) on top. The viewer
// picks how many of the newest rows are listed (50/100/500); the summary covers exactly those rows.
// Renders from the data static/chart.js loads (event "opennetmon:data"), so both refresh together.
(function () {
  'use strict';
  var box = document.getElementById('samples');
  var info = document.getElementById('samples-info');
  if (!box || !window.opennetmonFormat) { return; }
  var F = window.opennetmonFormat;
  var T = window.opennetmonText;
  var LIMITS = [50, 100, 500];
  var select = document.getElementById('samples-limit');
  var shown = LIMITS[0];
  var data = null;

  // The chosen number of rows is a per-viewer convenience; storage may be blocked.
  try {
    var saved = parseInt(localStorage.getItem('opennetmon-samples-limit'), 10);
    if (LIMITS.indexOf(saved) >= 0) { shown = saved; }
  } catch (e) { /* default */ }
  if (select) {
    select.value = String(shown);
    select.addEventListener('change', function () {
      var n = parseInt(select.value, 10);
      if (LIMITS.indexOf(n) < 0) { return; }
      shown = n;
      try { localStorage.setItem('opennetmon-samples-limit', String(n)); } catch (e) { /* not remembered */ }
      render();
    });
  }

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) { e.className = cls; }
    if (text !== undefined) { e.textContent = text; }
    return e;
  }

  // Seconds a point stands for: the bucket length for averages, otherwise the gap to the previous
  // sample, at most one polling interval. A rate always describes the last interval; after a gap
  // (restart, lost poll) the longer distance must not inflate the volume.
  function durations(d) {
    var out = [];
    for (var i = 0; i < d.ts.length; i++) {
      if (d.step_secs) { out.push(d.step_secs); continue; }
      var gap = i > 0 ? d.ts[i] - d.ts[i - 1] : d.interval_secs;
      out.push(Math.min(gap, d.interval_secs));
    }
    return out;
  }

  // Bucket length as whole hours and minutes, e.g. "1 Std. 24 Min.".
  function stepText(secs) {
    var h = Math.floor(secs / 3600), m = Math.round((secs % 3600) / 60), parts = [];
    if (h) { parts.push(h + ' ' + T('unit-hours')); }
    if (m || !h) { parts.push(m + ' ' + T('unit-minutes')); }
    return parts.join(' ');
  }

  // Column definitions: traffic channels get a volume and a speed column, like PRTG.
  function columns(d, dur) {
    var cols = [];
    d.series.forEach(function (s) {
      if (s.unit === 'bps') {
        cols.push({
          title: T('samples-volume', s.label),
          values: s.values.map(function (v, i) { return v === null ? null : v * dur[i] / 8; }),
          fmt: F.bytes, sum: true
        });
        cols.push({ title: T('samples-speed', s.label), values: s.values,
          fmt: function (v) { return F.value('bps', v); }, sum: false });
      } else {
        cols.push({ title: s.label + (s.unit_label ? ' (' + s.unit_label + ')' : ''), values: s.values,
          fmt: F.forSeries(s), sum: false, state: s.unit === 'state' || (s.labels || []).length > 0 });
      }
    });
    return cols;
  }

  function stats(values) {
    var n = 0, sum = 0, min = null, max = null;
    for (var i = 0; i < values.length; i++) {
      var v = values[i];
      if (v === null || v === undefined) { continue; }
      n++; sum += v;
      if (min === null || v < min) { min = v; }
      if (max === null || v > max) { max = v; }
    }
    return { n: n, sum: sum, avg: n ? sum / n : null, min: min, max: max };
  }

  function render() {
    box.textContent = '';
    if (!data || !data.ts.length) {
      box.appendChild(el('p', 'empty small', box.getAttribute('data-empty') || ''));
      if (info) { info.textContent = ''; }
      return;
    }
    var dur = durations(data);
    var cols = columns(data, dur);
    var count = data.ts.length;
    var limit = Math.min(shown, count);
    if (info) {
      var text;
      if (count > limit) { text = T('samples-newest', limit, count); }
      else { text = count === 1 ? T('samples-one') : T('samples-many', count); }
      if (data.step_secs) {
        text += ' · ' + T('samples-averages', stepText(data.step_secs));
      }
      info.textContent = text;
    }

    var table = el('table', 'samples');
    var head = el('thead');
    var hr = el('tr');
    hr.appendChild(el('th', null, data.step_secs ? T('samples-period-from') : T('samples-date-time')));
    cols.forEach(function (c) { hr.appendChild(el('th', 'n', c.title)); });
    head.appendChild(hr);
    table.appendChild(head);

    var body = el('tbody');
    var st = cols.map(function (c) { return stats(c.values.slice(count - limit)); });
    var summary = [
      { label: T('samples-sum'), pick: function (s, c) { return c.sum ? c.fmt(s.n ? s.sum : null) : ''; }, onlyIf: function () { return cols.some(function (c) { return c.sum; }); } },
      { label: T('samples-average'), pick: function (s, c) { return c.state ? '' : c.fmt(s.avg); } },
      { label: T('samples-minimum'), pick: function (s, c) { return c.fmt(s.min); } },
      { label: T('samples-maximum'), pick: function (s, c) { return c.fmt(s.max); } }
    ];
    var lastSummary = null;
    summary.forEach(function (row) {
      if (row.onlyIf && !row.onlyIf()) { return; }
      var tr = el('tr', 'summary');
      lastSummary = tr;
      tr.appendChild(el('td', null, limit === 1 ? T('samples-from-one', row.label) : T('samples-from-many', row.label, limit)));
      cols.forEach(function (c, i) { tr.appendChild(el('td', 'n', row.pick(st[i], c))); });
      body.appendChild(tr);
    });
    if (lastSummary) { lastSummary.className = 'summary summary-last'; }

    for (var k = 0; k < limit; k++) {
      var i = count - 1 - k; // newest first
      var tr = el('tr');
      tr.appendChild(el('td', 'nowrap', F.dateTime(data.ts[i], !data.step_secs)));
      cols.forEach(function (c) { tr.appendChild(el('td', 'n', c.fmt(c.values[i]))); });
      body.appendChild(tr);
    }
    table.appendChild(body);
    box.appendChild(table);
  }

  // Averaged ranges list empty buckets too (gaps in the chart); the table leaves them out.
  function withValues(d) {
    var keep = [];
    for (var i = 0; i < d.ts.length; i++) {
      if (d.series.some(function (s) { return s.values[i] !== null; })) { keep.push(i); }
    }
    if (keep.length === d.ts.length) { return d; }
    var pick = function (a) { return keep.map(function (i) { return a[i]; }); };
    return Object.assign({}, d, {
      ts: pick(d.ts),
      series: d.series.map(function (s) { return Object.assign({}, s, { values: pick(s.values) }); })
    });
  }

  window.addEventListener('opennetmon:data', function (e) { data = withValues(e.detail); render(); });
})();
