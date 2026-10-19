// Applies the color scheme chosen with the toggle before the page paints (no flash).
// Without a choice the system preference applies (CSS media query).
(function () {
  'use strict';
  try {
    var t = localStorage.getItem('opennetmon-theme');
    if (t === 'light' || t === 'dark') { document.documentElement.setAttribute('data-theme', t); }
  } catch (e) { /* storage blocked: follow the system */ }
})();
