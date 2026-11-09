// Page frame: light/dark switch, active navigation entry, menus of the header, keyboard shortcuts.
(function () {
  'use strict';
  var root = document.documentElement;
  var KEY = 'opennetmon-theme';
  var systemDark = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;

  function isDark() {
    var t = root.getAttribute('data-theme');
    if (t) { return t === 'dark'; }
    return !!(systemDark && systemDark.matches);
  }
  // Flips between light and dark. Landing on the system's own choice follows the system again.
  function flipTheme() {
    var next = isDark() ? 'light' : 'dark';
    var system = systemDark && systemDark.matches ? 'dark' : 'light';
    try {
      if (next === system) { localStorage.removeItem(KEY); } else { localStorage.setItem(KEY, next); }
    } catch (e) { /* not remembered, still applied */ }
    if (next === system) { root.removeAttribute('data-theme'); } else { root.setAttribute('data-theme', next); }
    window.dispatchEvent(new Event('opennetmon:theme'));
  }
  var toggles = document.querySelectorAll('[data-theme-toggle]');
  for (var i = 0; i < toggles.length; i++) {
    toggles[i].addEventListener('click', function () { flipTheme(); closeMenus(null); });
  }
  if (systemDark && systemDark.addEventListener) {
    systemDark.addEventListener('change', function () {
      if (!root.getAttribute('data-theme')) { window.dispatchEvent(new Event('opennetmon:theme')); }
    });
  }

  // Highlight the navigation entry of the current section (longest matching prefix), in the header and at the bottom.
  var links = document.querySelectorAll('.nav a[data-match], .bottomnav a[data-match]');
  var path = window.location.pathname, bestLen = -1, best = [];
  for (var j = 0; j < links.length; j++) {
    var prefixes = links[j].getAttribute('data-match').split(' ');
    for (var k = 0; k < prefixes.length; k++) {
      var p = prefixes[k];
      var hit = p === '/' ? path === '/' : path === p || path.indexOf(p + '/') === 0;
      if (!hit) { continue; }
      if (p.length > bestLen) { best = [links[j]]; bestLen = p.length; } else if (p.length === bestLen) { best.push(links[j]); }
    }
  }
  for (var b = 0; b < best.length; b++) { best[b].setAttribute('aria-current', 'page'); }

  // On small screens the settings tabs scroll sideways: bring the current one into view.
  var subnav = document.querySelector('.subnav');
  var currentTab = subnav && subnav.querySelector('a[aria-current="page"]');
  if (currentTab && subnav.scrollWidth > subnav.clientWidth) {
    subnav.scrollLeft = Math.max(0, currentTab.offsetLeft - subnav.offsetLeft - 16);
  }

  // Menus of the header (`details.menu`): one open at a time, closed by a click elsewhere or Escape.
  var menus = document.querySelectorAll('details.menu');
  function closeMenus(except) {
    for (var m = 0; m < menus.length; m++) {
      if (menus[m] !== except && menus[m].open) { menus[m].open = false; }
    }
  }
  for (var n = 0; n < menus.length; n++) {
    menus[n].addEventListener('toggle', function (e) { if (e.target.open) { closeMenus(e.target); } });
  }
  document.addEventListener('click', function (e) {
    var inside = e.target.closest && e.target.closest('details.menu');
    closeMenus(inside || null);
  });

  // Search in the header: the text of the current search stays in the field; a shorter hint on narrow screens.
  var search = document.getElementById('header-search');
  if (search) {
    var q = new URLSearchParams(location.search).get('q');
    if (q && path.indexOf('/status/') === 0) { search.value = q; }
    var short = search.getAttribute('data-short-placeholder');
    if (short && window.matchMedia && window.matchMedia('(max-width: 899px)').matches) { search.placeholder = short; }
  }

  // Keyboard shortcuts and their help (`dialog#keys-help`).
  var help = document.getElementById('keys-help');
  function openHelp() {
    closeMenus(null);
    if (help && !help.open && help.showModal) { help.showModal(); }
  }
  var helpButtons = document.querySelectorAll('[data-keys-help]');
  for (var h = 0; h < helpButtons.length; h++) { helpButtons[h].addEventListener('click', openHelp); }
  if (help) {
    var closers = help.querySelectorAll('[data-close]');
    for (var c = 0; c < closers.length; c++) { closers[c].addEventListener('click', function () { help.close(); }); }
    help.addEventListener('click', function (e) { if (e.target === help) { help.close(); } });
  }
  var GO = {};
  var targets = help ? help.querySelectorAll('[data-go]') : [];
  for (var g = 0; g < targets.length; g++) { GO[targets[g].getAttribute('data-go')] = targets[g].getAttribute('data-href'); }
  var goPending = false, goTimer = null;
  function typing(e) {
    var t = e.target;
    return (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) ||
      (help && help.open) || e.ctrlKey || e.metaKey || e.altKey;
  }
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') { closeMenus(null); }
    if (typing(e)) { return; }
    if (goPending) {
      goPending = false; clearTimeout(goTimer);
      var dest = GO[e.key.toLowerCase()];
      if (dest) { e.preventDefault(); location.href = dest; }
      return;
    }
    if (e.key === '/' && search) { e.preventDefault(); search.focus(); search.select(); return; }
    if (e.key === '?') { e.preventDefault(); openHelp(); return; }
    if (e.key === 't') { flipTheme(); return; }
    if (e.key === 'g') { goPending = true; clearTimeout(goTimer); goTimer = setTimeout(function () { goPending = false; }, 1500); }
  });

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
    var closeMoreMenus = function (target) {
      for (var m = 0; m < mores.length; m++) {
        if (isMenu(mores[m]) && !(target && mores[m].contains(target))) { mores[m].open = false; }
      }
    };
    document.addEventListener('click', function (e) {
      // A jump within the page (e.g. to "Delete sensor") leaves the page as it is, so close the menu as well.
      var jump = e.target.closest && e.target.closest('.more-items a[href^="#"]');
      closeMoreMenus(jump ? null : e.target);
    });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape') { closeMoreMenus(null); } });
  }

})();
