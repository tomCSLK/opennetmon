// Page controls shared by the tables with pages (static/samples.js, static/events.js):
// "Rows 101–150 of 1440" on the left and « ‹ Page 3 of 29 › » on the right, hidden while everything
// fits on one page. The caller keeps the position and draws the rows; this only draws the controls.
//
//   window.opennetmonPager(nav, { count: 1440, start: 100, size: 50, go: function (first) { … } })
//
// `start` is the index of the first row shown (0 = the first page), `go(first)` is called with the
// index of the first row of the page to show.
(function () {
  'use strict';
  var T = window.opennetmonText;

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) { e.className = cls; }
    if (text !== undefined) { e.textContent = text; }
    return e;
  }

  function button(label, title, disabled, onClick) {
    var b = el('button', 'secondary small', label);
    b.type = 'button';
    b.title = title;
    b.setAttribute('aria-label', title);
    b.disabled = disabled;
    b.addEventListener('click', onClick);
    return b;
  }

  window.opennetmonPager = function (nav, o) {
    if (!nav) { return; }
    nav.textContent = '';
    if (o.count <= o.size) { nav.hidden = true; return; }
    var pages = Math.ceil(o.count / o.size);
    var page = Math.min(pages, Math.floor(o.start / o.size) + 1);
    var last = (pages - 1) * o.size;
    var end = Math.min(o.count, o.start + o.size);
    var atStart = o.start === 0, atEnd = end >= o.count;
    function to(first) { return function () { o.go(Math.max(0, first)); }; }
    nav.appendChild(el('span', 'muted small pager-rows', T('samples-rows', o.start + 1, end, o.count)));
    var buttons = el('span', 'pager-buttons');
    buttons.appendChild(button('«', T('samples-first'), atStart, to(0)));
    buttons.appendChild(button('‹', T('samples-previous'), atStart, to(o.start - o.size)));
    buttons.appendChild(el('span', 'pager-page', T('samples-page', page, pages)));
    buttons.appendChild(button('›', T('samples-next'), atEnd, to(o.start + o.size)));
    buttons.appendChild(button('»', T('samples-last'), atEnd || o.start >= last, to(last)));
    nav.appendChild(buttons);
    nav.hidden = false;
  };
})();
