// Draws the charts of the sensor detail page with uPlot. Data comes from the JSON endpoint
// named in #charts[data-src]; one chart per unit (e.g. "in" and "out" share one).
(function () {
  'use strict';
  var root = document.getElementById('charts');
  if (!root || !window.uPlot) { return; }

  var REFRESH_MS = 30000;
  var charts = [];

  function css(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  function num(v, decimals) {
    var s = v.toFixed(decimals);
    if (s.indexOf('.') >= 0) { s = s.replace(/0+$/, '').replace(/\.$/, ''); }
    return s.replace('.', ',');
  }

  // Mirrors sensors::format::format_value on the server.
  function fmt(unit, v) {
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
      case 'state': return num(v, 0);
      default: return num(v, 3);
    }
  }

  function two(n) { return (n < 10 ? '0' : '') + n; }

  // Local time, 24-hour clock, German date order.
  function clock(ts, withSeconds) {
    var d = new Date(ts * 1000);
    return two(d.getHours()) + ':' + two(d.getMinutes()) + (withSeconds ? ':' + two(d.getSeconds()) : '');
  }
  function day(ts) {
    var d = new Date(ts * 1000);
    return two(d.getDate()) + '.' + two(d.getMonth() + 1) + '.';
  }
  function xValues(u, splits, axisIdx, foundSpace, foundIncr) {
    return splits.map(function (ts) {
      if (foundIncr >= 86400) { return day(ts); }
      var d = new Date(ts * 1000);
      return (d.getHours() === 0 && d.getMinutes() === 0 && d.getSeconds() === 0) ? day(ts) : clock(ts, foundIncr < 60);
    });
  }

  function destroyAll() {
    charts.forEach(function (c) { c.plot.destroy(); c.el.remove(); });
    charts = [];
  }

  function build(data) {
    destroyAll();
    if (!data.ts.length) {
      var p = document.createElement('p');
      p.className = 'muted';
      p.textContent = root.dataset.empty || '';
      root.appendChild(p);
      charts.push({ plot: { destroy: function () {} }, el: p });
      return;
    }
    var palette = [css('--accent'), '#3b82f6', '#f59e0b', '#ef4444'];
    var groups = {};
    data.series.forEach(function (s) { (groups[s.unit] = groups[s.unit] || []).push(s); });
    Object.keys(groups).forEach(function (unit) {
      var el = document.createElement('div');
      el.className = 'chart';
      root.appendChild(el);
      var series = [{ value: function (u, ts) { return ts === null ? '--' : day(ts) + ' ' + clock(ts, true); } }];
      var columns = [data.ts];
      groups[unit].forEach(function (s, i) {
        series.push({
          label: s.label,
          stroke: palette[i % palette.length],
          width: 1.5,
          spanGaps: false,
          value: function (u, v) { return fmt(unit, v); },
          paths: unit === 'state' ? uPlot.paths.stepped({ align: 1 }) : undefined
        });
        columns.push(s.values);
      });
      var fg = css('--fg'), grid = css('--border');
      var plot = new uPlot({
        width: Math.max(root.clientWidth, 280),
        height: 240,
        series: series,
        axes: [
          { stroke: fg, grid: { stroke: grid }, ticks: { stroke: grid }, values: xValues },
          { stroke: fg, grid: { stroke: grid }, ticks: { stroke: grid }, size: 80,
            values: function (u, vals) { return vals.map(function (v) { return fmt(unit, v); }); } }
        ]
      }, columns, el);
      charts.push({ plot: plot, el: el });
    });
  }

  function load() {
    fetch(root.dataset.src, { credentials: 'same-origin', headers: { Accept: 'application/json' } })
      .then(function (r) { if (!r.ok) { throw new Error(r.status); } return r.json(); })
      .then(build)
      .catch(function () { /* keep the last picture; the next refresh retries */ });
  }

  window.addEventListener('resize', function () {
    charts.forEach(function (c) {
      if (c.plot.setSize) { c.plot.setSize({ width: Math.max(root.clientWidth, 280), height: 240 }); }
    });
  });
  load();
  setInterval(load, REFRESH_MS);
})();
