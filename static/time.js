// Shows the timestamps the server renders as <time class="ts" datetime="...Z"> in the browser's time zone,
// 24-hour clock, date order of the page's language. The UTC text from the server stays as tooltip (and as
// the fallback without JS).
(function () {
  'use strict';
  var T = window.opennetmonText;
  function two(n) { return (n < 10 ? '0' : '') + n; }
  function local(d) {
    return T('date').replace('DD', two(d.getDate())).replace('MM', two(d.getMonth() + 1)).replace('YYYY', d.getFullYear()) +
      ' ' + two(d.getHours()) + ':' + two(d.getMinutes()) + ':' + two(d.getSeconds());
  }
  var times = document.querySelectorAll('time.ts');
  for (var i = 0; i < times.length; i++) {
    var el = times[i];
    var d = new Date(el.getAttribute('datetime'));
    if (isNaN(d.getTime())) { continue; }
    el.title = el.textContent;
    el.textContent = local(d);
  }
})();
