// Sign-out after a time without activity (config `session_idle_minutes`, templates/app.html): the server ends the
// session. This script reports clicks and key presses (at most once a minute) and opens the login page when the
// time is up, so that no data stays on the screen. Automatic reloads (static/refresh.js) do not count.
(function () {
  'use strict';
  var AUTO = 'opennetmon_auto';
  // The cookie of an automatic reload has done its job with the request of this page.
  var auto = document.cookie.split(';').some(function (c) { return c.trim().indexOf(AUTO + '=') === 0; });
  if (auto) { document.cookie = AUTO + '=; Path=/; Max-Age=0; SameSite=Strict'; }

  var form = document.querySelector('form[data-session-left]');
  if (!form) { return; }
  var csrf = form.querySelector('input[name="csrf_token"]').value;
  var REPORT_EVERY = 60000, RETRY = 30000;
  var deadline = Date.now() + Number(form.getAttribute('data-session-left')) * 1000;
  // Opening this page counted as activity unless it was an automatic reload.
  var reported = auto ? 0 : Date.now(), lastActive = 0;
  var timer = null;

  // Look again shortly before the end (activity not reported yet is reported then) and just after it.
  function schedule() {
    clearTimeout(timer);
    var wait = deadline - Date.now();
    timer = setTimeout(check, wait > 10000 ? wait - 5000 : Math.max(wait, 0) + 1000);
  }
  // The server answers with the time left; a session it ended redirects to the login page.
  function handle(res) {
    if (res.redirected) { location.replace(res.url); return null; }
    if (!res.ok) { throw new Error(String(res.status)); }
    return res.json().then(function (data) {
      if (data.left === null) { clearTimeout(timer); return; }
      deadline = Date.now() + data.left * 1000;
      schedule();
    });
  }
  function check() {
    if (lastActive > reported) { report(); return; }
    fetch('/session', { credentials: 'same-origin', headers: { Accept: 'application/json' } })
      .then(handle)
      .catch(function () { deadline = Date.now() + RETRY; schedule(); });
  }
  function report() {
    reported = Date.now();
    fetch('/session', {
      method: 'POST',
      credentials: 'same-origin',
      keepalive: true,
      headers: { Accept: 'application/json', 'Content-Type': 'application/x-www-form-urlencoded' },
      body: 'csrf_token=' + encodeURIComponent(csrf)
    }).then(handle).catch(function () { reported = 0; deadline = Date.now() + RETRY; schedule(); });
  }
  function active() {
    lastActive = Date.now();
    if (lastActive - reported >= REPORT_EVERY) { report(); }
  }
  ['pointerdown', 'keydown', 'wheel', 'touchstart'].forEach(function (name) {
    document.addEventListener(name, active, { capture: true, passive: true });
  });
  schedule();
})();
