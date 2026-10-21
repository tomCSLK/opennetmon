// Refresh control (templates/refresh.html): "Aktualisieren" reloads the page, "Automatisch" does so
// every 60 s. It waits while a form field has the focus or a form was changed, so nothing entered
// gets lost, and counts only while the page is visible.
(function () {
  'use strict';
  var box = document.querySelector('[data-refresh]');
  if (!box) { return; }
  var T = window.opennetmonText;
  var PERIOD = 60, KEY = 'opennetmon-auto-refresh';
  var auto = box.querySelector('[data-refresh-auto]');
  var left = box.querySelector('[data-refresh-left]');
  var loadedAt = Date.now(), remaining = PERIOD, dirty = false;

  // A flash message ("?msg=…") belongs to the action before, not to every reload.
  function reload() {
    var url = new URL(location.href);
    if (url.searchParams.has('msg')) {
      url.searchParams.delete('msg');
      location.replace(url.toString());
    } else {
      location.reload();
    }
  }
  box.querySelector('[data-refresh-now]').addEventListener('click', function (e) {
    e.preventDefault();
    reload();
  });

  function mark(e) { if (e.target !== auto && e.target.form) { dirty = true; } }
  document.addEventListener('input', mark, true);
  document.addEventListener('change', mark, true);
  function busy() {
    var a = document.activeElement;
    return dirty || (a !== auto && a !== null && /^(INPUT|SELECT|TEXTAREA)$/.test(a.tagName));
  }

  function show() {
    left.textContent = auto.checked ? ' · ' + (busy() ? T('refresh-paused') : T('refresh-left', remaining)) : '';
  }

  try { auto.checked = localStorage.getItem(KEY) !== 'off'; } catch (e) { auto.checked = true; }
  auto.addEventListener('change', function () {
    try { localStorage.setItem(KEY, auto.checked ? 'on' : 'off'); } catch (e) { /* not remembered */ }
    remaining = PERIOD;
    show();
  });
  auto.parentElement.hidden = false;
  show();

  setInterval(function () {
    if (auto.checked && !document.hidden && !busy()) {
      remaining -= 1;
      if (remaining <= 0) { reload(); return; }
    }
    show();
  }, 1000);
  // Back to a tab that was hidden for longer than a period: reload right away.
  document.addEventListener('visibilitychange', function () {
    if (!document.hidden && auto.checked && !busy() && Date.now() - loadedAt > PERIOD * 1000) { reload(); }
  });
})();
