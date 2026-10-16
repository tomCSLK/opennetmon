// Name suggestions for the "custom OID" field: asks /mibs/search.json while typing and fills the
// <datalist>; for an exact name it shows module, type, unit and description below the field.
(function () {
  'use strict';
  var input = document.getElementById('oid');
  var list = document.getElementById('mib-names');
  var info = document.getElementById('oid-info');
  if (!input || !list || !info) { return; }
  var initialInfo = info.innerHTML;
  var timer = null;
  var lastQuery = null;

  function nameOf(text) {
    var t = text.trim();
    var i = t.lastIndexOf('::');
    if (i >= 0) { t = t.slice(i + 2); }
    return t.split('.')[0];
  }

  function show(items, typed) {
    list.textContent = '';
    items.forEach(function (o) {
      var opt = document.createElement('option');
      opt.value = o.suggestion;
      opt.label = o.module + ' · ' + o.kind + (o.units ? ' · ' + o.units : '');
      list.appendChild(opt);
    });
    var exact = items.filter(function (o) { return o.name.toLowerCase() === nameOf(typed).toLowerCase(); })[0];
    if (!exact) { info.innerHTML = initialInfo; return; }
    info.textContent = exact.name + ' (' + exact.module + ') · ' + exact.kind + (exact.type ? ' · ' + exact.type : '') +
      (exact.units ? ' · ' + exact.units : '') + ' · OID ' + exact.oid +
      (exact.index ? ' · Index anhängen (' + exact.index + '), z. B. ' + exact.name + '.1' : '') +
      (exact.description ? ' – ' + exact.description : '') +
      (exact.pollable ? '' : ' (kein abfragbarer Zahlenwert)');
  }

  function lookup() {
    var typed = input.value;
    var q = nameOf(typed);
    if (q.length < 2 || /^[0-9]/.test(q)) { list.textContent = ''; info.innerHTML = initialInfo; return; }
    if (q === lastQuery) { return; }
    lastQuery = q;
    fetch('/mibs/search.json?q=' + encodeURIComponent(q), { credentials: 'same-origin', headers: { Accept: 'application/json' } })
      .then(function (r) { if (!r.ok) { throw new Error(r.status); } return r.json(); })
      .then(function (items) { if (q === lastQuery) { show(items, input.value); } })
      .catch(function () { /* suggestions are optional */ });
  }

  var kind = document.getElementById('kind');
  input.addEventListener('input', function () {
    // Typing an OID means a custom OID sensor.
    if (kind && input.value.trim() && kind.value !== 'oid') { kind.value = 'oid'; }
    clearTimeout(timer);
    timer = setTimeout(lookup, 200);
  });
})();
