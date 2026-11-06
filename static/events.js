// "Recent status changes" on the sensor page, shown in pages. The server draws the first page into
// the HTML (so the card works without a script); this loads the pages from #events[data-src]
// (newest first, #events-rows rows each, controls from static/pager.js) and replaces the rows.
// Warnings and outages are tinted like in the table of values.
//
// "Automatisch" (static/refresh.js) reloads the whole page: the page shown survives a reload of
// this tab (the id of its top row is kept in sessionStorage and sent as `anchor`, so new changes
// do not shift it), but a new visit starts on page 1.
(function () {
  'use strict';
  var box = document.getElementById('events');
  if (!box || !window.opennetmonFormat || !window.opennetmonPager) { return; }
  var F = window.opennetmonFormat;
  var T = window.opennetmonText;
  var info = document.getElementById('events-info');
  var rowsSelect = document.getElementById('events-rows');
  var pager = document.getElementById('events-pager');
  var src = box.getAttribute('data-src');
  var rowsKey = 'opennetmon-events-rows';
  var placeKey = 'opennetmon-events-at-' + location.pathname;
  var size = 20;
  var shown = null;   // last reply
  var pending = 0;    // the newest request wins; an older reply arriving late is dropped

  if (rowsSelect) {
    try {
      var saved = localStorage.getItem(rowsKey);
      if (saved && rowsSelect.querySelector('option[value="' + saved + '"]')) { rowsSelect.value = saved; }
    } catch (e) { /* default */ }
    size = parseInt(rowsSelect.value, 10) || 20;
    rowsSelect.addEventListener('change', function () {
      size = parseInt(rowsSelect.value, 10) || 20;
      try { localStorage.setItem(rowsKey, rowsSelect.value); } catch (e) { /* not remembered */ }
      remember(null);
      load({ offset: 0 });
    });
  }

  function reloaded() {
    try {
      var nav = performance.getEntriesByType('navigation')[0];
      return nav ? nav.type === 'reload' : false;
    } catch (e) { return false; }
  }

  function remember(id) {
    try {
      if (id) { sessionStorage.setItem(placeKey, String(id)); }
      else { sessionStorage.removeItem(placeKey); }
    } catch (e) { /* not remembered */ }
  }

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) { e.className = cls; }
    if (text !== undefined) { e.textContent = text; }
    return e;
  }

  function render(d) {
    shown = d;
    var body = box.querySelector('tbody');
    if (body && d.events.length) {
      body.textContent = '';
      d.events.forEach(function (ev) {
        var tr = el('tr', ev.status === 'warning' || ev.status === 'down' ? 'state-' + ev.status : null);
        tr.appendChild(el('td', 'num m-when', F.dateTime(ev.ts_ms / 1000, true)));
        tr.appendChild(el('td', 'm-name', ev.change));
        tr.appendChild(el('td', 'wrap muted m-msg', ev.message));
        body.appendChild(tr);
      });
    }
    if (info) {
      if (d.total === 0) { info.textContent = ''; }
      else if (d.oldest_ms === null) { info.textContent = T('events-count', d.total); }
      else if (d.total === 1) { info.textContent = T('events-since-one', F.dateTime(d.oldest_ms / 1000, false)); }
      else { info.textContent = T('events-since', d.total, F.dateTime(d.oldest_ms / 1000, false)); }
    }
    // A page after the first is found again after a reload by the id of its top row.
    remember(d.offset > 0 && d.events.length ? d.events[0].id : null);
    window.opennetmonPager(pager, { count: d.total, start: d.offset, size: size, go: function (first) { load({ offset: first }); } });
  }

  function load(where) {
    if (!src) { return; }
    var mine = ++pending;
    var url = src + '?limit=' + size + (where.anchor ? '&anchor=' + where.anchor : '&offset=' + where.offset);
    fetch(url, { credentials: 'same-origin', headers: { Accept: 'application/json' } })
      .then(function (r) { if (!r.ok) { throw new Error(r.status); } return r.json(); })
      .then(function (d) { if (mine === pending) { render(d); } })
      .catch(function () { /* keep the rows of the page; a reload retries */ });
  }

  var anchor = null;
  try { anchor = reloaded() ? parseInt(sessionStorage.getItem(placeKey), 10) : NaN; } catch (e) { /* page 1 */ }
  load(anchor > 0 ? { anchor: anchor } : { offset: 0 });
})();
