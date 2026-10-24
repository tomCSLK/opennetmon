# Poller, status and event log

## How it works (`src/poller.rs`)

When the server starts, a **supervisor** runs that reads the device list every 15 s and starts a task for each device.
Every device task reloads the device, the sensors that are not paused (including pauses of the device and its groups, `storage::pauses::load`) and the SNMP
profile from the database in **every cycle**. Changes (interval, address, credentials, new or paused sensors) therefore take effect without a restart; if the
device is deleted, the task ends.

- **Start times** are spread deterministically across the interval per device (at most 30 s offset): no burst at the full minute.
  The first values therefore appear up to 30 s after the start.
- **Concurrency:** a global `Semaphore` (`poll_concurrency`, default 16); a slow device never blocks others.
- **Interval per sensor:** a sensor is polled at the device's interval unless it has its own (sensor detail page, “Polling interval”; column
  `sensors.interval_secs`, migration `0017`; GitHub issue #1). The device task keeps the time each sensor is due next: a cycle polls only the due
  sensors (one SNMP session for them), then the task sleeps until the next one is due, but at most one device interval, so new or resumed sensors
  start soon. A shortened interval applies within one new interval.
- **Clock:** a fixed clock per sensor; if a cycle exceeds the interval, the next poll starts immediately instead of catching up in a burst.
- **The sensors of a device** run one after another (one GET each). Every sensor poll is additionally limited by `timeout × (retries + 1) + 2 s`.
- **Device-wide SNMP error** (timeout, authentication failed, not reachable): the remaining SNMP sensors of the same cycle are not tried any more but marked
  with the same error. Ping sensors run anyway.
- **The SNMP session** is set up anew in every cycle (one round of engine discovery for v3). This avoids stale engine times after a device reboot.
- **Counter base values** are kept across cycles as long as the sensor configuration does not change. After a change this costs one value.
- **Decryption** of the credentials only in the poller (`storage::credentials::load_credentials`); errors are logged without the secret.

Config: `poller_enabled` (default `true`) and `poll_concurrency` (1–256, default 16). On shutdown the server first stops the web server
(graceful shutdown), then the poller (waiting at most 10 s).

## Status (`src/status.rs`)

Sensor status: `Up`, `Warning`, `Down`, `Unknown`, `Paused`.

| Result of the poll | Status |
|---|---|
| Measurement succeeded, value OK | `Up` |
| Measurement succeeded, but interface not `up` (`ifOperStatus` ≠ 1) | `Down` (immediately) |
| Measurement failed, 1st failure in a row | `Warning` |
| Measurement failed, from the 2nd failure in a row on (`DOWN_AFTER_FAILURES`) | `Down` |
| A successful measurement again after a failure | `Up` (immediately) |

A single lost UDP packet therefore triggers no down alert. Thresholds per channel (warning/down) and rated status values are evaluated in `assess`
(see `docs/charts-and-thresholds.md`).
Device status = worst status of the active sensors (`Down` > `Warning` > `Unknown` > `Up`); paused sensors (`Paused`) do not count, only paused ones result in
`Paused`, no sensors or no poll yet result in `Unknown`.

## Database (migration `0006`)

- `sensor_status`: the current state per sensor (status, since when, last poll, message, failures in a row). Messages are stored as catalog messages and shown
  in the reader's language.
- `events`: log of the status changes (time, sensor, from → to, message); the first value of a sensor is logged with `from = NULL`.
  Only the **newest 500 status changes** across all sensors are kept (`storage::status::EVENTS_KEPT`): `record_poll` deletes older ones in the same
  transaction. The “Recent status changes” on the sensor detail page therefore only reach back that far; for the beginning of the hour, the status band of the
  sensor list falls back to the “from” of the oldest stored change or to the current state (`status_history`).

## Log (`/log`)

Menu item **“Log”** under Monitoring, for all users: warnings and failures and their end (changes from warning/down to “OK”) across all sensors, plus
**pauses** (`storage::pauses::log_entries`: “Paused” by whom, with reason and planned end, duration until resumed or “ongoing”; “Resumed” by whom; “Pause
ended” at the planned end; for devices and groups “whole device”/“whole group” instead of the sensor), both together **newest first** (`web::log`,
`storage::status::problem_log`). Columns: time, status with the previous status, sensor, device with group path, message, duration. For a warning/failure
the duration is the time until the next change, or “ongoing” with the time so far; for a recovery “after …” it is the duration of the problem before (from
`LAG`/`LEAD` over the changes of the same sensor). First polls, pausing and “Unknown” are not in the log but count towards the 500 stored changes. The pause
log (`pause_log`) also keeps the newest 500 entries. At the top right, “Refresh” with “Auto”.

Every column except the message sorts the log by a click on its head (`?sort=when|status|sensor|device|duration&dir=asc|desc`, like the status
lists; first click: newest and longest first, otherwise ascending; ties newest first). The search field above the table keeps only entries that contain
every word typed in status, previous status, sensor, device, group or message (`static/table-filter.js`, in the browser; the text is kept in the tab, so it
survives the automatic refresh, which waits while the field has the focus).

- `storage::status::record_poll` writes values, state and event in **one transaction** and returns the `StatusChange`
  (notifications hook in there; a change `NULL → Up` is no reason for an alert).

## Tests

Unit tests for the status logic and `storage::status`; poller tests against the fake agent (values, Warning → Down → Up, fail fast, rate after the second
cycle, paused/deleted sensors, start and stop). Manually against real hardware:
`cargo test --test omada_manual poller_cycle -- --ignored --nocapture` (temporary database, switch and AP, two cycles).
