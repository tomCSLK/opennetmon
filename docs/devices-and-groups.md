# Devices, groups and the connection test

## Overview (home page)

Below the tiles, the home page shows a **tree like in PRTG**: groups with ⊟/⊞ boxes to expand and collapse them, below them first the group's own
devices, then the subgroups, nested to any depth and connected by dotted lines. Each entry is a compact row with the icon of its device type (tooltip: the type);
on the right, in a fixed column, is the **sensor label**: if all active sensors are in the same state, a single label such as “✓ 9 sensors” (green OK,
yellow warning, red down, grey unknown), otherwise one label per state with the count, worst first (✕ down, ! warning, ? unknown, ✓ OK).
Paused sensors appear as an outlined label ‖ at the end; the tooltip names count and state. Groups show the sum of their whole subtree (also when
collapsed) and the number of devices. For devices, name and labels lead to the sensor page; the name of a group leads to its **group page** (below),
while the ⊟/⊞ box and the rest of the row expand and collapse it. Whether a group is expanded or collapsed is remembered in the
browser (`localStorage`; without storage all groups are open). “No devices in this group.” only appears for groups that are really empty.
The order is described below.
Devices without a group come first under “Without group” (if there are no groups: “All devices”). Viewers see name, address and interval;
administrators additionally see the SNMP profile and, as icons, “Test” (only with SNMP) and “Edit”, on groups “Order” and “Edit group”, and on devices and
groups “Pause” ‖ or “Resume” ▶ (see below). Paused rows are dimmed and name the pause (“Paused until …”, “Paused with the group “Network””, reason). On narrow
screens (below 560 px) address and interval are left out.
The action messages (“Device created.” etc.) come in through `?msg=` and are only mapped to fixed texts.

**Phones:** the pages are laid out for narrow screens (tested at 360–430 px), without scrolling sideways:
- The buttons in the head of the overview, the group pages, the sensor page of a device and the detail page are an **“Actions”** menu (`details.more-actions`):
  a row of buttons on wide screens, a drop-down on screens up to 640 px (`static/app.js`; without JavaScript it stays open as a row).
- On the device, group and sensor pages only the everyday actions stand in that row: **Pause/Resume** and **Log** (groups have no log). Setting up
  (edit, dependency, test connection, find values or interfaces, new subgroup or device, order, delete sensor) is in a second “Actions” menu inside the
  row (`details.more-actions.setup`), a drop-down on every width, so the row fits on one line down to about 800 px. On phones its entries join the
  outer menu below a line instead of a menu in the menu. Readers see neither menu.
- Row actions everywhere are icons with the name as tooltip (`templates/icons.html`: edit ✎, delete 🗑, pause ‖, order ↕); deleting a sensor or a device
  always asks for confirmation, only an uploaded MIB file is deleted at once (it can be uploaded again).
- Up to 560 px the tiles take three columns, the tree hides the row actions and the address line, and sensor labels show just their count.
- Up to 640 px the lists (sensors, status lists, log, events, users, profiles, templates, MIB files) turn into one block per row (`table.m-cards`
  with a layout class such as `m-sensors` or `m-log`, cells marked `m-name`, `m-status`, `m-value` …, `m-hide` for left-out columns); sortable column heads
  become a bar of sort links. On the detail page the period selector spans the width, value tiles come two per row and the measurement table drops the
  volume columns and the sum row.
- The tables on the setup pages (interface and value discovery, “This server”, template rules) still scroll inside their frame.

**Refresh:** the overview, the status lists, the sensor page of a device and the sensor detail page have a **“Refresh”** button at the top right (only the
icon ↻, the name as a tooltip; reloads the page, dropping any `?msg=` message) and the switch **“Auto”**: the page then reloads every 30 s (as in PRTG), with a countdown next
to it. The countdown waits (“paused”) while an input field has the focus or a form was changed (e.g. ticked sensors), and only runs while the tab is visible;
after a longer absence the page reloads immediately on return. The switch is on by default and remembered per browser (`localStorage`). Without JavaScript,
“Refresh” stays a normal link to the page. Implementation: `templates/refresh.html`, `static/refresh.js`.

**Path at the top of device and sensor pages:** Overview › group › subgroup › device › sensor, e.g. “Overview › Network › Router › Router ER605 › Ping”
(`web::group_trail`, partial template `templates/device_trail.html`). Each group leads to its group page. Devices without a group only show Overview › device.
(Links to the overview with the anchor `/#group-<id>`, as after pausing a group there, still expand the group and the groups above it and jump there;
`static/app.js`.)

## Group page

A click on the name of a group opens its page `/groups/<id>` (`web::overview::group`, template `templates/group.html`, all signed-in users), like the
sensor page of a device:

- **Path** Overview › groups above › group, then the name, a status badge (worst state of the active sensors in the whole subtree; “Paused” if the group
  or everything in it is paused), the number of devices and sensors and, if any, “N devices down”. A paused group shows the pause and its reason.
- **Tiles** like on the overview, but counted for the group and all its subgroups. They open the status lists limited to this subtree
  (`/status/<filter>?group=<id>`; the list says “in “Network” and its subgroups” and offers “all groups”).
- **Tree** of the group's own devices and its subgroups, exactly as on the overview (the shared macros are in `templates/tree.html`).
- **Administrators** additionally get “Edit group”, “Pause”/“Resume”, “Change order”, “New subgroup” (with this group preselected as parent) and
  “New device” (with this group preselected). These pages and the edit/pause icons in the tree return to the group page afterwards (`back=group-<id>`;
  `web::back_to_group`).
- “Without group” has no page of its own; unknown ids answer 404.

## Pausing (administrators only)

As in PRTG, **groups, devices and individual sensors can be paused** (`src/pause.rs`, `storage::pauses`, pages `web::pause`):

- **Form** `/pause/<group|device|sensor>/<id>` (icon ‖ in the overview, a button on the sensor page of a device and on the detail page):
  duration **until resumed**, 15 minutes, 1 hour, 3 hours, 1 day or 1 week, plus an optional **reason** (at most 200 characters, e.g.
  “firmware update”). After saving, you return to the page you came from (`back=overview|device|sensor|group-<id>`).
- **Inheritance:** a paused group pauses all subgroups, devices and sensors below it, a paused device all of its sensors. The nearest pause is shown:
  first the object's own, then the device's, then the groups' from bottom to top. Own pauses further down are kept and apply again when the outer one ends.
- **Effect:** paused sensors are not polled, trigger no mail and do not count towards the device status; they appear as “Paused” in the overview, the status
  lists (tile “Paused / unknown”) and on the sensor page. When pausing, their state is set to “Paused” immediately; a running outage thus ends in the log, and
  resuming sends **no** belated “OK again” mail (the first poll afterwards is a fresh start without the old state).
- **Resume** (`POST /resume/<kind>/<id>`) only works on the object that is paused itself; a device that is only paused with its group shows a note and is
  resumed through the group. **Pauses with a time limit** end by themselves: the poller checks every 15 s (`pauses::expire`); an expired pause no longer
  applies even before that.
- **Log:** pausing (with user, reason and planned end), resuming and automatic ends appear in the log `/log` (table `pause_log`, the newest 500 entries,
  `pauses::LOG_KEPT`; see `docs/poller.md`).
- **Database** (migration `0016`): `groups`/`devices` have `paused`, `pause_until_ms`, `pause_reason`; for sensors the own pause remains `enabled = 0`
  (plus `pause_until_ms`, `pause_reason`). Sensors that used to be “disabled” are paused indefinitely since then.

## Dependencies (administrators only)

As in PRTG, objects can **depend on a sensor** (`src/pause.rs`, `storage::pauses`, page `web::dependency`): while that sensor is down, they are paused
instead of each reporting an outage. Example: the group “Switches” depends on the ping of the router; when the router fails, there is one mail for the
router instead of one per switch sensor.

- **Form** `/dependency/<group|device|sensor>/<id>` (button “Dependency” on the group page, the sensor page of a device and the sensor detail page):
  - *Inherit from the parent object* (default): group → subgroup → device → sensor, like pauses; the form says what is inherited and from where.
  - *Depend on a sensor*: any sensor, chosen by device (listed like the device tree of the overview, each device with the path of its group).
  - *No dependency*: stops the inheritance.
  - **Delay after recovery** in seconds (0–3600, empty = inherit, in the end 0), inherited on its own: how long the objects stay paused after the
    sensor is fine again, e.g. so that a switch can boot after the router restarted.
  - For a sensor: **master sensor of the device** (typically the ping): while it is down or paused, the other sensors of its device are paused. A device
    has at most one; setting another one replaces it.
- **Trigger:** the sensor depended on is “Down”, or paused (by hand, with its device or group, or through its own dependency, so chains like
  router → switch → access point work). “Warning” alone does not trigger. A stored “Paused” counts until that sensor's next poll, so a chain unwinds
  in order. The sensor depended on is always polled; a sensor never depends on itself (a device may depend on its own ping: its other sensors pause).
- **Effect:** like a pause – not polled, no mail, “Paused” with the reason “Paused by its dependency on “Ping” (Router)” (during the delay with its end)
  in the overview, the status lists and on the sensor pages; the sensor page also names what a sensor depends on. Afterwards the sensors start afresh
  (no belated “OK again” mail).
- **Race:** the router and a switch are polled by different device tasks. A failed poll of a sensor does not count while a sensor it depends on has
  failed polls of its own (or was never polled), because it is about to pause it; the notifier drops collected changes of sensors that are paused
  through a dependency by the time it sends. The outage mail of the sensor depended on says what it paused (“Paused by it: 14 sensors on 3 devices”).
- **Cycles** (A depends on B and B on A, also through inheritance or a master) are refused when saving.
- **Log:** one entry when a sensor starts pausing others (“Dependency active: 14 sensors on 3 devices paused”) and one when that ends, instead of a line
  per sensor (table `dependency_log`, the newest 500 entries). After every poll cycle that changed a status and with every rescan (15 s),
  `pauses::sync_all` marks newly paused sensors and writes these entries; a delay thus ends within 15 s.
- **Device templates** keep the master flag of a ping, uptime or OID sensor; applying a template sets it only if the device has no master yet.
- **Database** (migration `0019`): `dependency` (`parent`/`sensor`/`none`), `dependency_sensor_id` (`ON DELETE SET NULL`: a deleted sensor means
  “inherit” again) and `dependency_delay_secs` on `groups`, `devices` and `sensors`, plus `sensors.dependency_master`.

## Maintenance windows (administrators only)

Like the schedules of PRTG (`src/schedule.rs`, `storage::schedules`, pages `web::schedules`): planned interruptions such as the nightly backup of the
Omada controller or a server update pause the objects instead of causing outage mails, without pausing them by hand each time.

- **Schedules** on the page “Maintenance windows” (`/schedules`, sidebar): named, each with any number of **windows**:
  - *weekly*: weekdays, start time and duration in minutes (1 minute to one week), e.g. “Mon–Fri from 03:00, 15 min”; a window over midnight belongs
    to the day it starts on;
  - *once*: start and end, e.g. “2026-10-31 10:00 to 2026-10-31 11:00”.

  The list shows the windows of every schedule, whether one is open (“open until …”) or when the next starts, and to how many objects it is assigned.
- **Time zone:** the times are in the time zone of the server, so a daily window stays at 03:00 across daylight saving changes (on the day the clocks
  go forward, a start in the skipped hour moves to after the gap). It is the zone of the system (`/etc/localtime`, `timedatectl`; Ubuntu servers often
  run in UTC) unless the config sets `time_zone = "Europe/Berlin"`; the pages and the log at startup name it. Times on the schedule pages are in this
  zone, all other times of the interface in the browser's.
- **Assignment** `/maintenance/<group|device|sensor>/<id>` (button “Maintenance window” in the menu “⋯ Actions” of the group page, the sensor page of a
  device and the sensor detail page), like a dependency: *inherit from the parent object* (default), *use this schedule*, or *no maintenance window*
  (stops the inheritance, e.g. for the ping of the router during the backup of the controller). The form says what applies now, where it comes from and
  when its next window starts.
- **Effect:** while a window is open, the objects are paused like by hand: not polled, no mail, “Paused” with “Maintenance window “Nightly”” and the
  end of the window in the overview, the status lists and the sensor pages. A pause by hand comes first; objects that depend on a paused sensor follow
  through their dependency. Afterwards the sensors start afresh (no belated “OK again” mail). Nothing is stored as a pause: the poller checks the windows
  whenever it loads the pauses and every 15 s (`pauses::sync_all`), so a window starts and ends within 15 s.
- **Log:** one entry when a window starts pausing (“Maintenance window – “Nightly”: 14 sensors on 3 devices paused”) and one when it ends (table
  `schedule_log`, the newest 500 entries).
- Deleting a schedule makes the objects it was assigned to inherit again. Device templates do not carry schedules.
- **Database** (migration `0020`): `schedules`, `schedule_windows` (`kind` `weekly` with `weekdays` as a bit mask Monday = 1 … Sunday = 64,
  `start_minute`, `duration_minutes`; `once` with `start_ms`/`end_ms` in UTC), `schedule_assignments` (one row per object; no row = inherit, no
  `schedule_id` = none) and `schedule_log`.

## Order (administrators only)

- **Automatic** (the default): by name, case-insensitive, numbers by value (“Switch 2” before “Switch 10”).
- **By hand**: “Order” on a group arranges its subgroups and devices (`/groups/<id>/order`); on the section “Without group” or “All devices” it arranges the
  devices without a group and the top-level groups (`/groups/0/order`). “Arrange groups” at the top of the overview opens the same page; the button only
  appears with at least two top-level groups. Drag rows by the handle, move them with ↑/↓ or enter a position, then save. Without JavaScript, the position
  fields remain.
- Entries ordered by hand come first, new and unordered ones follow in automatic order. “Sort automatically” discards the manual order of the level.
- If a device is moved to another group (or a group below another group), it loses its place and is sorted in automatically there. Renaming does not change
  the place.
- The order applies to all users. Technical details: column `position` in `groups`, `devices`, `sensors` (migration `0010`, `NULL` = automatic), sorting in
  `src/ordering.rs`, pages in `src/web/order.rs`, script `static/order.js`.

## Groups (administrators only)

- Hierarchical, at most **8 levels**. Name 1–64 characters, **unique per level** (case-insensitive, for ASCII letters only).
- Create, rename and move (`/groups/…`). A group cannot be moved into itself or into one of its subgroups; the form does not even offer these targets.
- Deleting is only possible if the group contains **neither subgroups nor devices** (a confirmation tick is required).

## Devices (administrators only)

| Field | Rule |
|---|---|
| Name | 1–64 characters, unique (case-insensitive) |
| Device type | Generic (default), Router, Switch, Access point, Firewall, Server, NAS / storage, PC, Printer, Camera, Smart home / IoT, Internet service; only chooses the icon in the overview |
| Address | IPv4/IPv6 or a host name according to RFC 1123 (stored in lower case); `0.0.0.0`, `::`, multicast and broadcast are not allowed; `localhost` is allowed |
| Group | optional |
| SNMP profile | a profile or **“No SNMP – ping and services”**; a profile used by devices cannot be deleted (a message instead of an error) |
| UDP port | 1–65535, default 161 |
| Interval | 10–3600 s, default 60 s |
| Timeout per attempt | 1–60 s, default 5 s |
| Retries | 0–5, default 1 |

In addition, **timeout × (retries + 1) ≤ interval** must hold, so that polls of a device never overlap.
A single sensor can be polled at its **own interval** (settings page of the sensor, “General”; e.g. ping every 10 s on a device polled every 60 s); the same
limits apply, including the rule above with the device's timeout and retries. An empty field returns to the device's interval. The sensor page shows an own
interval after the description (“· every 10 s”); sensor lists (also the paused sensors) show only the name, so long OIDs do not widen the table.
All checks run on the server; after an error the entered values stay in the form (there are no passwords here).

“Edit device” shows the groups and the device in the path (a link to its sensor page). “Cancel” and “Save” return to the page from which the form was opened:
the overview (the link there carries `?back=overview`), a group page (`?back=group-<id>`) or otherwise the device's sensor page. “New device” on a group
page preselects the group (`/devices/new?group=<id>`) and returns there after saving.

## Devices without SNMP (ping and services)

For devices that do not speak SNMP (e.g. the ER605, printers, cameras), choose **“No SNMP – ping and services”** in the SNMP profile field. Websites and other services without a device of their own are added the same way. This also works before
any SNMP profile exists.

- When the device is created, an active **“Ping”** sensor is added right away and the page jumps to the device's sensor page. A chosen device template does
  not apply then.
- The sensor page has no “Test connection”, “Find values”, “Find interfaces” and no device template section; “Add sensor” only offers ping, the services (HTTP(S), TCP port, DNS,
  TLS certificate) and the values of this server (see `docs/sensors.md`). The overview shows “no SNMP” instead of the profile and no “Test” button. The server also rejects the SNMP actions with
  an explanation.
- Later you can choose an SNMP profile under “Edit device”. Going back to “No SNMP” is only possible when the device has no SNMP sensors left.
- Technical details: `devices.profile_id` is `NULL`. The poller opens no SNMP session for devices without SNMP sensors; should such a device still have an
  SNMP sensor, it reports “This device has no SNMP set up (only ping, services and the values of this server).”
- The column was mandatory up to migration 0011. SQLite only allows this change by rebuilding the table, which needs foreign keys switched off (otherwise
  dropping the old table would take all sensors with it) – that is not possible in an sqlx migration. Therefore `storage::migrate` does it once at startup
  (`allow_devices_without_profile`: new table, copy the data, indexes, `PRAGMA foreign_key_check`, only then `COMMIT`); afterwards it is a no-op. Take a backup
  with `opennetmon backup` before.

## Connection test

On the device (“Test” in the overview or “Test connection” on the edit page). The server queries `sysDescr.0` and `sysUpTime.0` with the stored profile and
shows the device description, uptime and response time, or an understandable error message:

| Message | Cause |
|---|---|
| “No answer within the timeout …” | Device or port not reachable, SNMP switched off, wrong community or wrong v3 credentials (with v2c, devices do not answer a wrong community) |
| “Authentication failed …” | SNMPv3: password, authentication or encryption method do not match |
| “… not reachable or the SNMP port is closed” | Host name cannot be resolved, or ICMP “port unreachable” |

The test uses at most a **10 s timeout and 1 retry**, however the device is configured (the request blocks the browser only briefly).
It is POST-only with a CSRF token and for administrators only, because it makes the server send SNMP packets to freely chosen addresses.
Passwords appear neither on the result page nor in the log (the log only names user, device and ok/failed).

## Technical details

- Tables `groups` and `devices` (migration `0004`), foreign keys with `ON DELETE RESTRICT` for group and profile.
- Device type: column `devices.kind` (migration `0015`, text without `CHECK`, so that new types need no table rebuild; unknown values count as “Generic”),
  in the code `devices::DeviceKind`; the icons are in the macro `device_icon` in `templates/tree.html`. Forms without the field result in “Generic”.
- SNMP client `src/snmp/mod.rs`: `SnmpClient::connect` (including SNMPv3 engine discovery), `get` with timeout, retries on timeouts and a single retry after
  a v3 context update. The poller uses the same client.
- `snmp2` with the feature `heap_buffers`: without it, a 64 KB buffer lives on the stack, which overflows in Tokio threads (2 MB stack) in debug builds.

## Tests

- Maintenance windows: the computation of windows (`schedule::tests`, also over midnight and on both days of the daylight saving change in
  Europe/Berlin), inheritance and precedence (`pause::tests`), storage and the pages (`storage::schedules`, `web::schedules`).
- Unit and handler tests use a small **fake SNMPv2c agent** (`src/snmp/testagent.rs`) which, like real agents, stays silent on a wrong community.
- `tests/snmp_local.rs`: integration tests against the local `snmpd` (v2c, v3 with `OPENNETMON_TEST_V3_*`); they skip themselves when the agent or the variables
  are missing.
- `tests/omada_manual.rs`: manual tests against real Omada devices, all `#[ignore]`. Addresses and credentials come from the environment
  (`OPENNETMON_OMADA_SWITCH`, `OPENNETMON_OMADA_AP`, `OPENNETMON_OMADA_USER`, `OPENNETMON_PROBE_AUTH_PASS`), e.g. from a file outside the repository:
  `set -a; . ~/.config/opennetmon-dev/omada.env; set +a; cargo test --test omada_manual -- --ignored --nocapture`
