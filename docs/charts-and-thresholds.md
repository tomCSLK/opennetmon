# Detail page, charts, thresholds, rollups

## Detail page (`/sensors/<id>`, all signed-in users)

A header with type, device, status, message and last poll; the time range switch **1 hour / 24 hours / 7 days / 30 days** (`?range=1h|24h|7d|30d`,
invalid values result in 24 hours); the chart; a table with the latest value per channel; the last 20 status changes of this sensor. The sensor list of
a device (`/devices/<id>/sensors`) links every sensor there. Name, interval, what the sensor checks, thresholds and ratings are changed on the settings page.

## Settings page (`/sensors/<id>/edit`, administrators only)

“Settings” in the menu “⋯ Actions” of the detail page (`web::sensor_settings`). One form with one “Save”, like the settings tab of a PRTG sensor:

- **General:** name (at most 64 characters, unique per device) and the sensor's own polling interval (empty = the device's; `docs/devices-and-groups.md`).
- **What is checked** (custom OID and service sensors only): the fields the sensor was added with (`docs/sensors.md`, “User interface”).
- **Thresholds** per channel (below), **status values** for sensors with named values (below).

The whole form is checked before anything is written: one wrong field changes nothing, the page shows the message with all input. A section only changes
when its fields are sent (the page always sends all of them). Saving the page unchanged writes nothing and adds no audit entry; saved changes are one
audit entry. Values, chart and events are always kept. Readers do not see the page; the detail page shows them the ratings of status values.

## Chart (uPlot)

- **uPlot 1.6.32** (MIT) is kept unchanged under `static/vendor/uplot/` (with license, source and checksum in the `README.md` there) and shipped with the
  binary – no CDN, no npm. Own code: `static/chart.js` (no inline script, the CSP stays `default-src 'self'`).
- The data come as JSON from `/sensors/<id>/data?range=…`: `{"range","step_secs","interval_secs","ts":[seconds…],"series":[{"key","label","unit","values":[…|null]}]}`.
  All channels share the time axis; a missing value is `null` (a gap in the chart, no line).
- One chart per unit (inbound and outbound share one). Shown in the **browser's time zone** (`static/chart.js`), 24-hour clock, numbers and units in the
  user's language (bit/s, kbit/s, Mbit/s, Gbit/s). The chart reloads every 30 s; header and value tiles are updated by “Auto” (every 30 s, see
  `docs/devices-and-groups.md`).
- Where the points come from: 1 h raw values (every poll); 24 h, 7 d and 30 d **120 averages** each in blocks of equal length – 12 minutes, 1 h 24 min
  and 6 hours. 24 h and 7 d average the raw values, 30 d the hourly values from `samples_1h` (as long as no rollups exist yet, the raw data directly).
  The blocks start at multiples of their length (UTC), so that they do not move on reload; the newest block is still filling.
  For status channels an average is an intermediate value (e.g. 1.5 between “up” and “down”). At most 20,000 raw values per channel in the 1-hour view.
- Static files have `Cache-Control: no-cache` and an ETag (content hash): the browser asks briefly on every load and gets `304 Not Modified` as long as
  nothing has changed. After an update, new files therefore apply immediately. (The `?v=2` on the addresses bypasses the old one-hour caches once.)

## The “Values” table

Below the chart – like PRTG's “live data” table – there is one row per **poll** (raw values), **newest at the top**, within the chosen time range.
This holds for every range, also where the chart shows block averages. How far back the table reaches is chosen next to its title
(“Time range”; the first entry is the default, the browser remembers the choice per range):

| Chart range | Table reaches back |
|---|---|
| 1 hour | last 15 minutes, last 30 minutes, last hour |
| 24 hours | last hour, last 6 hours, last 24 hours |
| 7 days | last 6 hours, last 24 hours, last 7 days |
| 30 days | last 24 hours, last 7 days |

Raw values exist for `retention_raw_days` (7 days by default), so no choice reaches further. An hour has 60 polls at a 60-second interval and 360 at
10 seconds; seven days are about 10,000 rows at 60 seconds and 60,000 at 10 seconds. At most 20,000 rows are sent (the newest; the table then says
“newest 20000 of …”). The table loads from its own endpoint `/sensors/<id>/samples?range=…&minutes=…` (the raw values of the span, the status and message of each row that warned or failed, plus their total).
The volume of a traffic row uses the sensor's interval (its own or the device's).

Polls that did not end “up” are tinted like in PRTG: red for down, amber for a warning. A failed poll (timeout, no reply) writes no values,
so its row shows the message instead (e.g. the timeout); a warning from a limit keeps its values and shows the message on hover. Average,
minimum and maximum count only the rows with values. These polls are stored in `poll_problems` (status and message, migration `0023`) and
kept like the raw values (`retention_raw_days`). Paused times have no rows: a paused sensor is not polled.

- Traffic channels have two columns per direction: **volume** (the amount of data in the section, decimal: 1 MB = 1,000,000 bytes) and **speed**.
  The volume is rate × duration; the duration is the gap to the previous poll, at most one polling interval (after a restart or a lost poll
  a longer gap would otherwise inflate the volume). It is therefore an approximation; PRTG calculates with the counter readings
  themselves.
- Above the rows are **sum** (volumes only), **average**, **minimum** and **maximum**, each “of N values”. Status channels show the name (`up`, `down` …).
- Sum, average, minimum and maximum cover all rows of the chosen span, on every page. The table reloads together with the chart every 30 s
  (`static/samples.js`, shared formatting in `static/format.js`).
- **Pages:** the rows are shown in pages of 50, 100 or 250 (“Rows”, remembered per browser), with « ‹ Page 3 of 29 › » and “Rows 101–150 of 1440”
  below the table; the bar is hidden while everything fits on one page. On page 1 new polls come in on top; on a later page a refresh keeps the
  same row on top (the newer polls push the page number up instead of shifting the rows). The page shown survives a reload of the tab, also the
  automatic one; a new visit and a change of span or page size start on page 1. All rows are still loaded; only the page shown is drawn.
- After a restart of opennetmon a traffic sensor misses one value, because a rate needs two counter readings.
- Not taken over from PRTG: columns for downtime and coverage (PRTG shows them for averaged historic data; with one row per poll they would only be 0 or 100 %).

## Times in tables

The server writes every point in time as `<time class="ts" datetime="2026-10-15T19:32:10Z">2026-10-15 19:32:10 UTC</time>` (`web::time_html`). `static/time.js`
(loaded on every page) replaces the text in the browser with the local time in the format of the user's language (e.g. `10/15/2026 21:32:10` or
`15.10.2026 21:32:10`) and puts the UTC time into the tooltip. Without JavaScript, the UTC time stays. This applies to the last poll and the messages of the
sensors, status changes and delivery attempts. **E-mails** keep stating the time explicitly in UTC.

## Thresholds

Per channel (not for status channels) four optional limits: **warning above / below** and **down above / below**. Entered in the display unit
(traffic in Mbit/s, amounts of data in GB, temperatures in °C, a decimal comma is allowed), stored in the base unit (`channels.warn_above` etc., migration `0007`).

- A value triggers when it is **strictly above or below** the limit; down takes precedence over warning.
- Warning → sensor status `Warning`, down limit → `Down`, each **immediately without debouncing** (unlike poll errors, see `docs/poller.md`);
  the message names channel, value and limit (“Inbound: 90 Mbit/s is above the down limit 80 Mbit/s”).
- Validation: down limits must not be milder than warning limits, the upper limit must not be below the lower one, everything finite. Errors appear on the
  settings page, the input stays. Only administrators can save (CSRF-protected).
- Effective from the next cycle (the poller reads the limits anew every time).

## Rating status values

Sensors with named status values – custom OID sensors whose MIB object is an enumeration (e.g. `tpPoePowerStatus`: off, on, overload, short,
hardware-fault …, `entPhySensorOperStatus`, `ifOperStatus`), and the interface status sensor – have the section **“Status values”** on the settings page:
per value **OK**, **Warning** or **Down** (stored in `value_states`, migration `0012`).

- **Rated:** the measured value determines the status immediately (like thresholds, without debouncing); the message names the value
  (“overload (3) counts as down”, normally “on (2)”). A value that is not in the list is a warning (“Unknown status value 42”).
  Thresholds apply as well; the worse finding wins.
- **Suggestion when creating:** new custom OID sensors (form, “Find values”) get the rating from the names of the values if these contain clear error or
  warning words: `fail`, `fault`, `critical`, `error`, `overload`, `short`, `overtemp`, `nonoperational`, `notPresent`, `lowerLayerDown`, `down` … → down;
  `warn`, `degraded`, `unavailable`, `testing`, `nonstandard`, `voltage-high/-low`, `unknown` → warning; everything else (`ok`, `on`, `off`, `enabled`,
  `disable` …) → OK. Negations such as `noError` stay OK. If all values are harmless, the sensor stays unrated.
- **Existing sensors** stay unrated (the value does not change the status) until the rating is saved; the form shows the suggestion for this.
  Saving the settings unchanged keeps them unrated. “Switch rating off” removes the rating again (`POST /sensors/<id>/states`, `action=reset`).
- **Interface status:** without a rating of its own, the rule “only up is OK, everything else is down” applies as before; with a rating, e.g. “down” can be
  only a warning. “Restore default” returns to the built-in rule.
- Device templates take over the ratings (`docs/device-templates.md`). Viewers see the rating, only administrators change it.
- Measured on the SG2008P: “PoE status port 1” (`tpPoePowerStatus.1`) got overload/short/hardware-fault/overtemperature = down and
  nonstandard-pd/voltage-high/voltage-low = warning when created; port 1 reports “on (2)”, port 2 “off (0)”, both OK.

## Rollups and retention (`src/maintenance.rs`)

The poller also starts a maintenance task (30 s after the start, then every 10 minutes):

1. **Rollup:** `samples_1h` (hour, min/avg/max/count) from the raw values, `samples_1d` (day, weighted average) from the hourly values. The last 2 hours or
   1 day before the newest rollup entry are recalculated (unfinished hour, late values); running it again changes nothing.
2. **Retention:** raw values `retention_raw_days` (default 7), hourly values and events `retention_hourly_days` (default 365; events additionally at most
   the newest 500, see `docs/poller.md`); **daily values are kept forever**. Aggregation always happens before deletion.

Config: `retention_raw_days` (1–3650), `retention_hourly_days` (at least `retention_raw_days`, at most 36500). If the server does not run for longer than the
raw data retention, the hourly values of that time are missing. Without a running poller (`poller_enabled = false`) there is no maintenance either.

## Tests

Status logic and thresholds (`src/status.rs`), formatting (`src/sensors/format.rs`), rollups and retention (`src/maintenance.rs`), time series
(`src/storage/measurements.rs`), handlers (`src/web/sensor_detail.rs`: display, JSON, saving, permissions, CSRF, assets), poller (thresholds change the status).
The chart itself was checked in the browser against the real switch (rendering, no console errors/CSP violations).
