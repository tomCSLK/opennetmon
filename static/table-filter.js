// Filter field above a table (templates/mibs.html, log.html, audit.html, status_list.html): `data-table-filter` names
// the table. Rows stay visible when their `data-filter` text (lower case) contains every word typed; a counter shows how
// many are left. Heading rows (`data-filter-group`) stay visible while one of the rows below them does. The text
// survives reloads in this tab (e.g. after deleting a row).
//
// With a `select[data-table-rows]` in the same card the matching rows are shown in pages of that many rows, with the
// controls of static/pager.js in `nav[data-table-pager]` (log and changes); the choice is remembered in this browser.
// Without JavaScript the field and the page controls stay hidden and the table complete.
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
    var rowsSelect = card.querySelector('select[data-table-rows]');
    var pager = card.querySelector('[data-table-pager]');
    var wrap = table.closest('.table-wrap') || table;
    var all = Array.prototype.slice.call(table.tBodies[0].rows);
    var rows = all.filter(function (row) { return !row.hasAttribute('data-filter-group'); });
    // Each scope (e.g. the log of one device) remembers its own text.
    var KEY = 'opennetmon-filter-' + (input.getAttribute('data-table-filter-key') || id);
    var ROWS_KEY = 'opennetmon-rows-' + id;
    var start = 0;

    function pageSize() { return rowsSelect ? (parseInt(rowsSelect.value, 10) || 0) : 0; }

    function apply(keepPage) {
      var words = input.value.toLowerCase().split(/\s+/).filter(Boolean);
      var matched = rows.filter(function (row) {
        var text = row.getAttribute('data-filter') || row.textContent.toLowerCase();
        return words.every(function (w) { return text.indexOf(w) !== -1; });
      });
      var size = pageSize();
      if (!keepPage) { start = 0; }
      if (size && start >= matched.length) { start = Math.max(0, Math.floor((matched.length - 1) / size) * size); }
      var end = size ? start + size : matched.length;
      rows.forEach(function (row) { row.opennetmonShow = false; });
      matched.forEach(function (row, i) { row.opennetmonShow = i >= start && i < end; });

      var head = null, headShown = false;
      function closeGroup() { if (head) { head.hidden = !headShown; } }
      all.forEach(function (row) {
        if (row.hasAttribute('data-filter-group')) {
          closeGroup();
          head = row;
          headShown = false;
          return;
        }
        row.hidden = !row.opennetmonShow;
        if (row.opennetmonShow) { headShown = true; }
      });
      closeGroup();

      if (count) { count.textContent = words.length ? T('filter-count', matched.length, rows.length) : ''; }
      wrap.hidden = matched.length === 0;
      if (empty) { empty.hidden = matched.length > 0; }
      if (pager && size && window.opennetmonPager) {
        window.opennetmonPager(pager, { count: matched.length, start: start, size: size, go: function (first) {
          start = first;
          apply(true);
          if (card.getBoundingClientRect && card.getBoundingClientRect().top < 0) { card.scrollIntoView(); }
        } });
      }
      try { sessionStorage.setItem(KEY, input.value); } catch (e) { /* not remembered */ }
    }

    try { input.value = sessionStorage.getItem(KEY) || ''; } catch (e) { /* storage blocked */ }
    input.hidden = false;
    input.addEventListener('input', function () { apply(false); });
    // Escape empties the field.
    input.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && input.value) { input.value = ''; apply(false); }
    });
    if (rowsSelect) {
      try {
        var saved = localStorage.getItem(ROWS_KEY);
        if (saved && rowsSelect.querySelector('option[value="' + saved + '"]')) { rowsSelect.value = saved; }
      } catch (e) { /* storage blocked */ }
      rowsSelect.addEventListener('change', function () {
        try { localStorage.setItem(ROWS_KEY, rowsSelect.value); } catch (e) { /* not remembered */ }
        apply(false);
      });
      var label = rowsSelect.closest('label');
      // Only worth choosing when there is more than the smallest page.
      if (label) { label.hidden = rows.length <= (parseInt(rowsSelect.options[0].value, 10) || 0); }
    }
    apply(false);
  });
})();
