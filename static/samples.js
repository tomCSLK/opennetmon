// Table of measured values below the chart (like PRTG's "Live data" table): one row per poll,
// newest first, with average/minimum/maximum (and the summed volume for traffic) on top. Polls that
// warned or failed are tinted like in PRTG; a failed poll has no values and shows its message. Always raw
// values, also where the chart shows averages; the viewer picks how far back
// the table reaches (#samples-span, minutes) and the summary covers exactly those rows. The rows are
// shown in pages (#samples-rows rows each, #samples-pager); the summary stays that of the whole span.
// Loads from #samples[data-src] whenever static/chart.js has loaded new data (event "opennetmon:data"),
// so both refresh together.
(function () {
  'use strict';
  var box = document.getElementById('samples');
  var info = document.getElementById('samples-info');
  if (!box || !window.opennetmonFormat) { return; }
  var F = window.opennetmonFormat;
  var T = window.opennetmonText;
  var select = document.getElementById('samples-span');
  var rowsSelect = document.getElementById('samples-rows');
  var pager = document.getElementById('samples-pager');
  var data = null;
  // First row shown (0 = newest) and the time of that row: a refresh keeps the same row on top
  // instead of letting every new poll shift the page by one. On page 1 new polls come in on top.
  var start = 0, anchor = null;

  // The chosen span is remembered per range, a per-viewer convenience; storage may be blocked.
  if (select) {
    var key = 'opennetmon-samples-span-' + select.getAttribute('data-range');
    try {
      var saved = localStorage.getItem(key);
      if (saved && select.querySelector('option[value="' + saved + '"]')) { select.value = saved; }
    } catch (e) { /* default */ }
    select.addEventListener('change', function () {
      try { localStorage.setItem(key, select.value); } catch (e) { /* not remembered */ }
      firstPage();
      load();
    });
  }

  function pageSize() {
    var n = rowsSelect ? parseInt(rowsSelect.value, 10) : 0;
    return n > 0 ? n : 50;
  }
  if (rowsSelect) {
    try {
      var rows = localStorage.getItem('opennetmon-samples-rows');
      if (rows && rowsSelect.querySelector('option[value="' + rows + '"]')) { rowsSelect.value = rows; }
    } catch (e) { /* default */ }
    rowsSelect.addEventListener('change', function () {
      try { localStorage.setItem('opennetmon-samples-rows', rowsSelect.value); } catch (e) { /* not remembered */ }
      firstPage();
      render();
    });
  }

  // "Automatisch" (static/refresh.js) reloads the whole page: the page shown survives a reload of
  // this tab, but a new visit starts on page 1.
  var placeKey = 'opennetmon-samples-at-' + location.pathname + '-' + (box.getAttribute('data-src') || '');
  function reloaded() {
    try {
      var nav = performance.getEntriesByType('navigation')[0];
      return nav ? nav.type === 'reload' : false;
    } catch (e) { return false; }
  }
  try {
    var place = reloaded() ? parseInt(sessionStorage.getItem(placeKey), 10) : NaN;
    if (place > 0) { anchor = place; start = 1; }
  } catch (e) { /* page 1 */ }
  function remember() {
    try {
      if (start > 0 && anchor !== null) { sessionStorage.setItem(placeKey, String(anchor)); }
      else { sessionStorage.removeItem(placeKey); }
    } catch (e) { /* not remembered */ }
  }
  function firstPage() { start = 0; anchor = null; remember(); }

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
      if (pager) { pager.hidden = true; }
      return;
    }
    var dur = durations(data);
    var cols = columns(data, dur);
    var count = data.ts.length;
    var size = pageSize();
    var pages = Math.ceil(count / size);
    if (start > 0 && anchor !== null) {
      // The row shown on top before, or the next older one if it is gone.
      var k = 0;
      while (k < count && data.ts[count - 1 - k] > anchor) { k++; }
      start = k;
    }
    if (start > count - 1) { start = (pages - 1) * size; }
    if (start < 0) { start = 0; }
    anchor = start > 0 ? data.ts[count - 1 - start] : null;
    remember();
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

    var end = Math.min(count, start + size);
    for (var k = start; k < end; k++) {
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
    renderPager(count, size, end);
  }

  function goTo(first) {
    start = Math.max(0, first);
    anchor = null;
    if (start > 0 && data) { anchor = data.ts[data.ts.length - 1 - start]; }
    render();
  }

  function pagerButton(label, title, first, disabled) {
    var b = el('button', 'secondary small', label);
    b.type = 'button';
    b.title = title;
    b.setAttribute('aria-label', title);
    b.disabled = disabled;
    b.addEventListener('click', function () { goTo(first); });
    return b;
  }

  // "Rows 101–150 of 1440" and « ‹ Page 3 of 29 › »; hidden while everything fits on one page.
  function renderPager(count, size, end) {
    if (!pager) { return; }
    pager.textContent = '';
    if (count <= size) { pager.hidden = true; return; }
    var pages = Math.ceil(count / size);
    var page = Math.min(pages, Math.floor(start / size) + 1);
    var last = (pages - 1) * size;
    var atStart = start === 0, atEnd = end >= count;
    pager.appendChild(el('span', 'muted small pager-rows', T('samples-rows', start + 1, end, count)));
    var nav = el('span', 'pager-buttons');
    nav.appendChild(pagerButton('«', T('samples-first'), 0, atStart));
    nav.appendChild(pagerButton('‹', T('samples-previous'), Math.max(0, start - size), atStart));
    nav.appendChild(el('span', 'pager-page', T('samples-page', page, pages)));
    nav.appendChild(pagerButton('›', T('samples-next'), start + size, atEnd));
    nav.appendChild(pagerButton('»', T('samples-last'), last, atEnd || start >= last));
    pager.appendChild(nav);
    pager.hidden = false;
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
