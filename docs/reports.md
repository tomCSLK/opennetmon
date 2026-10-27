# Reports: availability

Like the reports of PRTG: how reliable a device or sensor was over the last 24 hours, 7 or 30 days or a calendar month
(`src/availability.rs`, `storage::availability`, page `web::reports`). Available to every user, readers included.

## Counting

- Every poll (`storage::status::record_poll`, in the same transaction) adds the time since the poll before to the status the sensor
  had, split at full hours, in table `availability_1h` (milliseconds per status and hour).
- A **gap** of more than three polling intervals between two polls (service stopped, host off) is left out: it counts as no data,
  not as available.
- **Paused** time (by hand, through a dependency or a maintenance window) is recorded as paused and does not count; neither does
  *Unknown*.
- Availability = (OK + warning) / (OK + warning + down). Downtime = time down.
- A **device** is as available as its master sensor (see “Dependencies” in `docs/devices-and-groups.md`), else its ping sensor, else the
  mean of its sensors that have data (then without downtime). The report says which.
- Counting starts with the update to the version that has it; earlier times cannot be computed afterwards.

## Storage and retention

- `availability_1h` (`sensor_id`, `ts` = start of the hour in UTC, `up_ms`, `warning_ms`, `down_ms`, `unknown_ms`, `paused_ms`) and
  `availability_1d` (the same per UTC day), migration `0021`.
- The maintenance run of the poller sums the hours into days (`storage::availability::rollup`) and deletes hours after
  `retention_hourly_days` (default one year); days are kept forever. About 25 000 hourly rows per year for 70 sensors.
- Queries (`storage::availability::totals`) are exact to the hour: they use the hours that start in the range and, for days whose hours
  are gone, the daily sums.

## Pages

- **Reports** (`/reports`, sidebar): table per device (with its group and basis) or per sensor (with its device), for 7 days,
  30 days or one of the last twelve calendar months. Months are in the time zone of the server (like maintenance windows: the
  system's or config `time_zone`), the current month up to now. Sortable by name, availability (lowest first) and downtime (longest
  first); rows without data stay last. Colors: at least 99 % green, at least 95 % yellow, below red.
- **CSV** (`/reports.csv`, button “Download CSV”, same parameters): columns `device`/`sensor`, `group`/`device`,
  `availability_percent` (four decimals, decimal point), `downtime_seconds`, `counted_seconds`, `basis`, `from`, `to` (UTC, ISO 8601).
  Text fields are quoted; text that a spreadsheet would run as a formula (`=`, `+`, `-`, `@`) starts with `'`.
- **Sensor detail page:** tiles “Availability 24 h / 7 days / 30 days” with the downtime.
- Percentages are rounded down (99.999 % shows as 99.99 %), so a short outage never shows as 100 %.

## Not yet

- Monthly report by mail (planned later, `PLAN.md`, G).

## Tests

`availability::tests` (hours, gaps, calculation, devices), `storage::availability::tests` (counting, rollup, retention),
`storage::status::tests::every_poll_counts_…`, `web::reports::tests` (months in Europe/Berlin, CSV fields, rounding) and
`web::reports::handler_tests` (page, views, CSV, sensor tiles).
