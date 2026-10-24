// Filter field above a table (templates/mibs.html, log.html): `data-table-filter` names the table. Rows stay visible
// when their `data-filter` text (lower case) contains every word typed; a counter shows how many are left. Heading rows
// (`data-filter-group`) stay visible while one of the rows below them does. The text survives
// reloads in this tab (e.g. after deleting a row). Without JavaScript the field stays hidden and the table complete.
(function () {
  'use strict';
  var T = window.opennetmonText;
  document.querySelectorAll('[data-table-filter]').forEach(function (input) {
    var id = input.getAttribute('data-table-filter');
    var table = document.getElementById(id);
    if (!table) { return; }
    var card = input.closest('.card') || document;
    var count = card.querySelector('[data-table-filter-count]');
    var empty = card.querySelector('[data-table-filter-empty]');
    var wrap = table.closest('.table-wrap') || table;
    var all = Array.prototype.slice.call(table.tBodies[0].rows);
    var rows = all.filter(function (row) { return !row.hasAttribute('data-filter-group'); });
    var KEY = 'opennetmon-filter-' + id;

    function apply() {
      var words = input.value.toLowerCase().split(/\s+/).filter(Boolean);
      var shown = 0;
      var head = null, headShown = false;
      function closeGroup() { if (head) { head.hidden = !headShown; } }
      all.forEach(function (row) {
        if (row.hasAttribute('data-filter-group')) {
          closeGroup();
          head = row;
          headShown = false;
          return;
        }
        var text = row.getAttribute('data-filter') || row.textContent.toLowerCase();
        var match = words.every(function (w) { return text.indexOf(w) !== -1; });
        row.hidden = !match;
        if (match) { shown += 1; headShown = true; }
      });
      closeGroup();
      if (count) { count.textContent = words.length ? T('filter-count', shown, rows.length) : ''; }
      wrap.hidden = shown === 0;
      if (empty) { empty.hidden = shown > 0; }
      try { sessionStorage.setItem(KEY, input.value); } catch (e) { /* not remembered */ }
    }

    try { input.value = sessionStorage.getItem(KEY) || ''; } catch (e) { /* storage blocked */ }
    input.hidden = false;
    input.addEventListener('input', apply);
    // Escape empties the field.
    input.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && input.value) { input.value = ''; apply(); }
    });
    apply();
  });
})();
