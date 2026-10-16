// Shell behaviour: theme toggle (system -> light -> dark), active navigation entry, mobile menu.
(function () {
  'use strict';
  var root = document.documentElement;
  var LABELS = { system: 'Design: System', light: 'Design: Hell', dark: 'Design: Dunkel' };

  function stored() {
    try { return localStorage.getItem('netmon-theme') || 'system'; } catch (e) { return 'system'; }
  }
  function apply(mode) {
    if (mode === 'light' || mode === 'dark') { root.setAttribute('data-theme', mode); } else { root.removeAttribute('data-theme'); }
    try {
      if (mode === 'system') { localStorage.removeItem('netmon-theme'); } else { localStorage.setItem('netmon-theme', mode); }
    } catch (e) { /* not remembered, still applied */ }
    var buttons = document.querySelectorAll('[data-theme-toggle]');
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].setAttribute('data-mode', mode);
      var label = buttons[i].querySelector('.label');
      if (label) { label.textContent = LABELS[mode]; }
      buttons[i].setAttribute('aria-label', LABELS[mode] + ' (umschalten)');
    }
    window.dispatchEvent(new Event('netmon:theme'));
  }

  var toggles = document.querySelectorAll('[data-theme-toggle]');
  for (var i = 0; i < toggles.length; i++) {
    toggles[i].addEventListener('click', function () {
      var next = { system: 'light', light: 'dark', dark: 'system' }[stored()] || 'system';
      apply(next);
    });
  }
  apply(stored());
  if (window.matchMedia) {
    var mq = window.matchMedia('(prefers-color-scheme: dark)');
    var onChange = function () { if (stored() === 'system') { window.dispatchEvent(new Event('netmon:theme')); } };
    if (mq.addEventListener) { mq.addEventListener('change', onChange); }
  }

  // Highlight the navigation entry of the current section (longest matching prefix).
  var links = document.querySelectorAll('.nav a[data-match]');
  var path = window.location.pathname, best = null, bestLen = -1;
  for (var j = 0; j < links.length; j++) {
    var prefixes = links[j].getAttribute('data-match').split(' ');
    for (var k = 0; k < prefixes.length; k++) {
      var p = prefixes[k];
      var hit = p === '/' ? path === '/' : path === p || path.indexOf(p + '/') === 0 || path.indexOf(p) === 0;
      if (hit && p.length > bestLen) { best = links[j]; bestLen = p.length; }
    }
  }
  if (best) { best.setAttribute('aria-current', 'page'); }

  // Overview: folded groups are remembered per browser (only a convenience; without storage all stay open).
  var FOLDED = 'netmon-folded-groups';
  function foldedGroups() {
    try { return JSON.parse(localStorage.getItem(FOLDED) || '[]'); } catch (e) { return []; }
  }
  var groups = document.querySelectorAll('details[data-group]');
  var folded = foldedGroups();
  for (var g = 0; g < groups.length; g++) {
    (function (details) {
      var id = details.getAttribute('data-group');
      if (folded.indexOf(id) >= 0) { details.open = false; }
      details.addEventListener('toggle', function () {
        var list = foldedGroups().filter(function (x) { return x !== id; });
        if (!details.open) { list.push(id); }
        try { localStorage.setItem(FOLDED, JSON.stringify(list)); } catch (e) { /* not remembered */ }
      });
    })(groups[g]);
  }

  var menu = document.querySelector('.menu-toggle');
  if (menu) {
    menu.addEventListener('click', function () {
      var sidebar = document.querySelector('.sidebar');
      var open = sidebar.classList.toggle('open');
      menu.setAttribute('aria-expanded', open ? 'true' : 'false');
    });
  }
})();
