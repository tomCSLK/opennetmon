// Shared formatting for charts and tables (German: decimal comma, 24-hour clock, local time zone).
// Mirrors sensors::format::format_value on the server.
(function () {
  'use strict';

  function num(v, decimals) {
    var s = v.toFixed(decimals);
    if (s.indexOf('.') >= 0) { s = s.replace(/0+$/, '').replace(/\.$/, ''); }
    return s.replace('.', ',');
  }

  var OPER_STATUS = { 1: 'up', 2: 'down', 3: 'testing', 4: 'unknown', 5: 'dormant', 6: 'notPresent', 7: 'lowerLayerDown' };

  function value(unit, v, key) {
    if (v === null || v === undefined) { return '–'; }
    var a = Math.abs(v);
    switch (unit) {
      case 'bps':
        if (a >= 1e9) { return num(v / 1e9, 2) + ' Gbit/s'; }
        if (a >= 1e6) { return num(v / 1e6, 2) + ' Mbit/s'; }
        if (a >= 1e3) { return num(v / 1e3, 1) + ' kbit/s'; }
        return num(v, 0) + ' bit/s';
      case 'ms': return num(v, 2) + ' ms';
      case 's':
        if (a >= 86400) { return num(v / 86400, 1) + ' Tage'; }
        if (a >= 3600) { return num(v / 3600, 1) + ' Std.'; }
        if (a >= 60) { return num(v / 60, 1) + ' Min.'; }
        return num(v, 0) + ' s';
      case 'percent': return num(v, 1) + ' %';
      case 'per_second': return num(v, 3) + ' /s';
      case 'state':
        if (key === 'oper_status' && OPER_STATUS[Math.round(v)]) { return OPER_STATUS[Math.round(v)]; }
        return num(v, 0);
      default: return num(v, 3);
    }
  }

  // Data volume in decimal units, as PRTG shows it (1 MB = 1 000 000 bytes).
  function bytes(b) {
    if (b === null || b === undefined) { return '–'; }
    var a = Math.abs(b);
    if (a >= 1e12) { return num(b / 1e12, 2) + ' TB'; }
    if (a >= 1e9) { return num(b / 1e9, 2) + ' GB'; }
    if (a >= 1e6) { return num(b / 1e6, 2) + ' MB'; }
    if (a >= 1e3) { return num(b / 1e3, 1) + ' KB'; }
    return num(b, 0) + ' B';
  }

  function two(n) { return (n < 10 ? '0' : '') + n; }
  function clock(ts, withSeconds) {
    var d = new Date(ts * 1000);
    return two(d.getHours()) + ':' + two(d.getMinutes()) + (withSeconds ? ':' + two(d.getSeconds()) : '');
  }
  function day(ts) {
    var d = new Date(ts * 1000);
    return two(d.getDate()) + '.' + two(d.getMonth() + 1) + '.';
  }
  function dateTime(ts, withSeconds) {
    var d = new Date(ts * 1000);
    return two(d.getDate()) + '.' + two(d.getMonth() + 1) + '.' + d.getFullYear() + ' ' + clock(ts, withSeconds);
  }

  // Formatter for one series of the chart JSON: MIB enumeration labels ("up (1)") and the unit
  // label of custom OID sensors ("41,5 °C") on top of the channel unit.
  function forSeries(s) {
    var labels = {};
    (s.labels || []).forEach(function (pair) { labels[pair[0]] = pair[1]; });
    var hasLabels = (s.labels || []).length > 0;
    return function (v) {
      if (v === null || v === undefined) { return '–'; }
      if (hasLabels && Math.round(v) === v && labels[v] !== undefined) { return labels[v] + ' (' + v + ')'; }
      if (s.unit_label) {
        if (s.unit === 'per_second') { return num(v, 3) + ' ' + s.unit_label + '/s'; }
        if (s.unit === 'count') { return num(v, 3) + ' ' + s.unit_label; }
      }
      return value(s.unit, v, s.key);
    };
  }

  window.opennetmonFormat = { num: num, value: value, bytes: bytes, clock: clock, day: day, dateTime: dateTime, forSeries: forSeries };
})();
