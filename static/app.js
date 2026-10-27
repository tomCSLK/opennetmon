// Shell behaviour: theme toggle (system -> light -> dark), active navigation entry, mobile menu.
(function () {
  'use strict';
  var root = document.documentElement;
  var T = window.opennetmonText;
  var LABELS = { system: T('theme-system'), light: T('theme-light'), dark: T('theme-dark') };

  function stored() {
    try { return localStorage.getItem('opennetmon-theme') || 'system'; } catch (e) { return 'system'; }
  }
  function apply(mode) {
    if (mode === 'light' || mode === 'dark') { root.setAttribute('data-theme', mode); } else { root.removeAttribute('data-theme'); }
    try {
      if (mode === 'system') { localStorage.removeItem('opennetmon-theme'); } else { localStorage.setItem('opennetmon-theme', mode); }
    } catch (e) { /* not remembered, still applied */ }
    var buttons = document.querySelectorAll('[data-theme-toggle]');
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].setAttribute('data-mode', mode);
      var label = buttons[i].querySelector('.label');
      if (label) { label.textContent = LABELS[mode]; }
      buttons[i].setAttribute('aria-label', T('theme-toggle', LABELS[mode]));
    }
    window.dispatchEvent(new Event('opennetmon:theme'));
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
    var onChange = function () { if (stored() === 'system') { window.dispatchEvent(new Event('opennetmon:theme')); } };
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
  var FOLDED = 'opennetmon-folded-groups';
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
  // A link to a group (#group-<id>, from the breadcrumbs of device pages) unfolds it and the groups around it.
  var target = /^#group-\d+$/.test(location.hash) ? document.getElementById(location.hash.slice(1)) : null;
  if (target) {
    for (var d = target; d; d = d.parentElement) {
      if (d.tagName === 'DETAILS') { d.open = true; }
    }
    target.scrollIntoView();
  }

  // "Back" on error pages: the browser's history keeps what was typed into the form; the link
  // (previous page or overview) is the fallback without history.
  var back = document.querySelector('[data-back]');
  if (back && window.history.length > 1) {
    back.addEventListener('click', function (e) {
      e.preventDefault();
      window.history.back();
    });
  }

  // "More actions" in page heads (`details.more-actions`, rendered open): a menu on small screens,
  // all buttons in one row on wide ones. The setup actions inside (`.setup`) are the other way round:
  // a menu on wide screens, entries of the outer menu on small ones. Without JavaScript all buttons
  // simply stay visible.
  var mores = document.querySelectorAll('details.more-actions');
  if (mores.length && window.matchMedia) {
    var narrow = window.matchMedia('(max-width: 640px)');
    var isMenu = function (m) { return m.classList.contains('setup') ? !narrow.matches : narrow.matches; };
    var fit = function () {
      for (var m = 0; m < mores.length; m++) { mores[m].open = !isMenu(mores[m]); mores[m].setAttribute('data-ready', ''); }
    };
    fit();
    if (narrow.addEventListener) { narrow.addEventListener('change', fit); }
    // Closes every open menu that does not contain `target` (all of them for `null`).
    var closeMenus = function (target) {
      for (var m = 0; m < mores.length; m++) {
        if (isMenu(mores[m]) && !(target && mores[m].contains(target))) { mores[m].open = false; }
      }
    };
    document.addEventListener('click', function (e) {
      // A jump within the page (e.g. to "Delete sensor") leaves the page as it is, so close the menu as well.
      var jump = e.target.closest && e.target.closest('.more-items a[href^="#"]');
      closeMenus(jump ? null : e.target);
    });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape') { closeMenus(null); } });
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
