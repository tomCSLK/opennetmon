// Draws the charts of the sensor detail page with uPlot. Data comes from the JSON endpoint
// named in #charts[data-src]; one chart per unit (e.g. "in" and "out" share one).
(function () {
  'use strict';
  var root = document.getElementById('charts');
  if (!root || !window.uPlot || !window.netmonFormat) { return; }

  var REFRESH_MS = 30000;
  var charts = [];
  var lastData = null;

  function css(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  var F = window.netmonFormat;
  var day = F.day, clock = F.clock;

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
    lastData = data;
    destroyAll();
    if (!data.ts.length) {
      var p = document.createElement('p');
      p.className = 'muted';
      p.textContent = root.dataset.empty || '';
      root.appendChild(p);
      charts.push({ plot: { destroy: function () {} }, el: p });
      return;
    }
    var palette = [css('--accent'), css('--ok'), '#a855f7', css('--warn')];
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
          width: 2,
          fill: unit === 'state' ? undefined : palette[i % palette.length] + '1f',
          spanGaps: false,
          value: (function (f) { return function (u, v) { return f(v); }; })(F.forSeries(s)),
          paths: unit === 'state' ? uPlot.paths.stepped({ align: 1 }) : undefined
        });
        columns.push(s.values);
      });
      var fg = css('--muted'), grid = css('--border');
      var plot = new uPlot({
        width: Math.max(root.clientWidth, 280),
        height: 240,
        series: series,
        axes: [
          { stroke: fg, grid: { stroke: grid }, ticks: { stroke: grid }, values: xValues },
          { stroke: fg, grid: { stroke: grid }, ticks: { stroke: grid }, size: (groups[unit][0].labels || []).length ? 130 : 80,
            values: (function (f) { return function (u, vals) { return vals.map(f); }; })(F.forSeries(groups[unit][0])) }
        ]
      }, columns, el);
      charts.push({ plot: plot, el: el });
    });
  }

  function load() {
    fetch(root.dataset.src, { credentials: 'same-origin', headers: { Accept: 'application/json' } })
      .then(function (r) { if (!r.ok) { throw new Error(r.status); } return r.json(); })
      .then(function (data) {
        build(data);
        // The table of values (static/samples.js) renders from the same data.
        window.dispatchEvent(new CustomEvent('netmon:data', { detail: data }));
      })
      .catch(function () { /* keep the last picture; the next refresh retries */ });
  }

  window.addEventListener('resize', function () {
    charts.forEach(function (c) {
      if (c.plot.setSize) { c.plot.setSize({ width: Math.max(root.clientWidth, 280), height: 240 }); }
    });
  });
  // Colors come from CSS variables: redraw when the color scheme changes.
  window.addEventListener('netmon:theme', function () { if (lastData) { build(lastData); } });
  load();
  setInterval(load, REFRESH_MS);
})();
