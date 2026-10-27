// Table of measured values below the chart (like PRTG's "Live data" table): one row per poll,
// newest first, with average/minimum/maximum (and the summed volume for traffic) on top. Polls that
// warned or failed are tinted like in PRTG; a failed poll has no values and shows its message. Always raw
// values, also where the chart shows averages; the viewer picks how far back
// the table reaches (#samples-span, minutes) and the summary covers exactly those rows. Loads from
// #samples[data-src] whenever static/chart.js has loaded new data (event "opennetmon:data"), so both
// refresh together.
(function () {
  'use strict';
  var box = document.getElementById('samples');
  var info = document.getElementById('samples-info');
  if (!box || !window.opennetmonFormat) { return; }
  var F = window.opennetmonFormat;
  var T = window.opennetmonText;
  var select = document.getElementById('samples-span');
  var data = null;

  // The chosen span is remembered per range, a per-viewer convenience; storage may be blocked.
  if (select) {
    var key = 'opennetmon-samples-span-' + select.getAttribute('data-range');
    try {
      var saved = localStorage.getItem(key);
      if (saved && select.querySelector('option[value="' + saved + '"]')) { select.value = saved; }
    } catch (e) { /* default */ }
    select.addEventListener('change', function () {
      try { localStorage.setItem(key, select.value); } catch (e) { /* not remembered */ }
      load();
    });
  }

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) { e.className = cls; }
    if (text !== undefined) { e.textContent = text; }
    return e;
  }

  // Seconds a sample stands for: the gap to the previous sample, at most one polling interval.
  // A rate always describes the last interval; after a gap (restart, lost poll) the longer distance
  // must not inflate the volume.
  function durations(d) {
    var out = [];
    for (var i = 0; i < d.ts.length; i++) {
      var gap = i > 0 ? d.ts[i] - d.ts[i - 1] : d.interval_secs;
      out.push(Math.min(gap, d.interval_secs));
    }
    return out;
  }

  // Column definitions: traffic channels get a volume and a speed column, like PRTG.
  function columns(d, dur) {
    var cols = [];
    d.series.forEach(function (s) {
      if (s.unit === 'bps') {
        cols.push({
          title: T('samples-volume', s.label),
          values: s.values.map(function (v, i) { return v === null ? null : v * dur[i] / 8; }),
          fmt: F.bytes, sum: true, cls: 'n vol'
        });
        cols.push({ title: T('samples-speed', s.label), short: s.label, values: s.values,
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
    var states = data.states || [];
    var messages = data.messages || [];
    // Rows with at least one value: what the average, minimum and maximum are made of.
    var valued = 0;
    for (var r = 0; r < count; r++) {
      if (cols.some(function (c) { return c.values[r] !== null && c.values[r] !== undefined; })) { valued++; }
    }
    if (info) {
      if (data.total > count) { info.textContent = T('samples-newest', count, data.total); }
      else { info.textContent = count === 1 ? T('samples-one') : T('samples-many', count); }
    }

    var table = el('table', 'samples');
    var head = el('thead');
    var hr = el('tr');
    hr.appendChild(el('th', null, T('samples-date-time')));
    // Volume columns (class "vol") and the sum row give way to the speeds on phones (app.css).
    // A speed column is headed by just the channel name there ("short"), the volume being hidden.
    cols.forEach(function (c) {
      var th = el('th', c.cls || 'n');
      if (c.short) {
        th.appendChild(el('span', 'head-long', c.title));
        th.appendChild(el('span', 'head-short', c.short));
      } else {
        th.textContent = c.title;
      }
      hr.appendChild(th);
    });
    head.appendChild(hr);
    table.appendChild(head);

    var body = el('tbody');
    var st = cols.map(function (c) { return stats(c.values); });
    var summary = [
      { label: T('samples-sum'), pick: function (s, c) { return c.sum ? c.fmt(s.n ? s.sum : null) : ''; }, onlyIf: function () { return cols.some(function (c) { return c.sum; }); } },
      { label: T('samples-average'), pick: function (s, c) { return c.state ? '' : c.fmt(s.avg); } },
      { label: T('samples-minimum'), pick: function (s, c) { return c.fmt(s.min); } },
      { label: T('samples-maximum'), pick: function (s, c) { return c.fmt(s.max); } }
    ];
    var lastSummary = null;
    summary.forEach(function (row) {
      if (row.onlyIf && !row.onlyIf()) { return; }
      var tr = el('tr', row.onlyIf ? 'summary sum' : 'summary');
      lastSummary = tr;
      tr.appendChild(el('td', null, valued === 1 ? T('samples-from-one', row.label) : T('samples-from-many', row.label, valued)));
      cols.forEach(function (c, i) { tr.appendChild(el('td', c.cls || 'n', row.pick(st[i], c))); });
      body.appendChild(tr);
    });
    if (lastSummary) { lastSummary.className += ' summary-last'; }

    for (var k = 0; k < count; k++) {
      var i = count - 1 - k; // newest first
      var tr = el('tr', states[i] ? 'state-' + states[i] : null);
      tr.appendChild(el('td', 'nowrap', F.dateTime(data.ts[i], true)));
      var empty = cols.every(function (c) { return c.values[i] === null || c.values[i] === undefined; });
      if (states[i] && empty) {
        var td = el('td', 'problem', messages[i] || '');
        td.colSpan = cols.length;
        tr.appendChild(td);
      } else {
        if (messages[i]) { tr.title = messages[i]; }
        cols.forEach(function (c) { tr.appendChild(el('td', c.cls || 'n', c.fmt(c.values[i]))); });
      }
      body.appendChild(tr);
    }
    table.appendChild(body);
    box.appendChild(table);
  }

  // The newest request wins; an older reply arriving late is dropped.
  var pending = 0;
  function load() {
    var src = box.getAttribute('data-src');
    if (!src) { return; }
    var mine = ++pending;
    if (select) { src += '&minutes=' + encodeURIComponent(select.value); }
    fetch(src, { credentials: 'same-origin', headers: { Accept: 'application/json' } })
      .then(function (r) { if (!r.ok) { throw new Error(r.status); } return r.json(); })
      .then(function (d) { if (mine === pending) { data = d; render(); } })
      .catch(function () { /* keep the last table; the next refresh retries */ });
  }

  window.addEventListener('opennetmon:data', load);
})();
