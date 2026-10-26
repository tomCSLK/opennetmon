// Applies the color scheme chosen with the toggle before the page paints (no flash).
// Without a choice the system preference applies (CSS media query).
// Also provides window.opennetmonText for the other scripts: texts in the page's language from the
// data block #i18n (catalog locales/<lang>/js.ftl), with %1, %2 … filled in.
(function () {
  'use strict';
  var texts = null;
  window.opennetmonText = function (key) {
    if (texts === null) {
      try { texts = JSON.parse(document.getElementById('i18n').textContent); } catch (e) { texts = {}; }
    }
    var text = texts[key] !== undefined ? texts[key] : key;
    for (var i = 1; i < arguments.length; i++) { text = text.split('%' + i).join(String(arguments[i])); }
    return text;
  };
  // Lets the CSS hide what scripts rearrange before they run (e.g. the "more actions" menu).
  document.documentElement.classList.add('js');
  try {
    var t = localStorage.getItem('opennetmon-theme');
    if (t === 'light' || t === 'dark') { document.documentElement.setAttribute('data-theme', t); }
  } catch (e) { /* storage blocked: follow the system */ }
})();
