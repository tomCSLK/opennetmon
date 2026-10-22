# Devices, groups and the connection test

## Overview (home page)

Below the tiles, the home page shows a **tree like in PRTG**: groups with ⊟/⊞ boxes to expand and collapse them, below them first the group's own
devices, then the subgroups, nested to any depth and connected by dotted lines. Each entry is a compact row with the icon of its device type (tooltip: the type);
on the right, in a fixed column, is the **sensor label**: if all active sensors are in the same state, a single label such as “✓ 9 sensors” (green OK,
yellow warning, red down, grey unknown), otherwise one label per state with the count, worst first (✕ down, ! warning, ? unknown, ✓ OK).
Paused sensors appear as an outlined label ‖ at the end; the tooltip names count and state. Groups show the sum of their whole subtree (also when
collapsed) and the number of devices. For devices, name and labels lead to the sensor page. Whether a group is expanded or collapsed is remembered in the
browser (`localStorage`; without storage all groups are open). “No devices in this group.” only appears for groups that are really empty.
The order is described below.
Devices without a group come first under “Without group” (if there are no groups: “All devices”). Viewers see name, address and interval;
administrators additionally see the SNMP profile and, as icons, “Test” (only with SNMP) and “Edit”, on groups “Order” and “Edit group”, and on devices and
groups “Pause” ‖ or “Resume” ▶ (see below). Paused rows are dimmed and name the pause (“Paused until …”, “Paused with the group “Network””, reason). On narrow
screens (below 560 px) address and interval are left out.
The action messages (“Device created.” etc.) come in through `?msg=` and are only mapped to fixed texts.

**Refresh:** the overview, the status lists, the sensor page of a device and the sensor detail page have a **“Refresh”** button at the top right (only the
icon ↻, the name as a tooltip; reloads the page, dropping any `?msg=` message) and the switch **“Auto”**: the page then reloads every 60 s, with a countdown next
to it. The countdown waits (“paused”) while an input field has the focus or a form was changed (e.g. ticked sensors), and only runs while the tab is visible;
after a longer absence the page reloads immediately on return. The switch is on by default and remembered per browser (`localStorage`). Without JavaScript,
“Refresh” stays a normal link to the page. Implementation: `templates/refresh.html`, `static/refresh.js`.

**Path at the top of device and sensor pages:** Overview › group › subgroup › device › sensor, e.g. “Overview › Network › Router › Router ER605 › Ping”
(`web::group_trail`, partial template `templates/device_trail.html`). Each group leads to the overview with the anchor `/#group-<id>`; `static/app.js`
expands the group and the groups above it, jumps there and highlights the group header. Devices without a group only show Overview › device.

## Pausing (administrators only)

As in PRTG, **groups, devices and individual sensors can be paused** (`src/pause.rs`, `storage::pauses`, pages `web::pause`):

- **Form** `/pause/<group|device|sensor>/<id>` (icon ‖ in the overview, a button on the sensor page of a device and on the detail page):
  duration **until resumed**, 15 minutes, 1 hour, 3 hours, 1 day or 1 week, plus an optional **reason** (at most 200 characters, e.g.
  “firmware update”). After saving, you return to the page you came from (`back=overview|device|sensor`).
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
| SNMP profile | a profile or **“No SNMP – ping only”**; a profile used by devices cannot be deleted (a message instead of an error) |
| UDP port | 1–65535, default 161 |
| Interval | 10–3600 s, default 60 s |
| Timeout per attempt | 1–60 s, default 5 s |
| Retries | 0–5, default 1 |

In addition, **timeout × (retries + 1) ≤ interval** must hold, so that polls of a device never overlap.
All checks run on the server; after an error the entered values stay in the form (there are no passwords here).

“Edit device” shows the groups and the device in the path (a link to its sensor page). “Cancel” and “Save” return to the page from which the form was opened:
the overview (the link there carries `?back=overview`) or otherwise the device's sensor page.

## Devices without SNMP (ping only)

For devices that do not speak SNMP (e.g. the ER605, printers, cameras), choose **“No SNMP – ping only”** in the SNMP profile field. This also works before
any SNMP profile exists.

- When the device is created, an active **“Ping”** sensor is added right away and the page jumps to the device's sensor page. A chosen device template does
  not apply then.
- The sensor page has no “Test connection”, “Find values”, “Find interfaces” and no device template section; “Add sensor” only offers ping (and the values of
  this server, see `docs/sensors.md`). The overview shows “no SNMP” instead of the profile and no “Test” button. The server also rejects the SNMP actions with
  an explanation.
- Later you can choose an SNMP profile under “Edit device”. Going back to “No SNMP” is only possible when the device has no SNMP sensors left.
- Technical details: `devices.profile_id` is `NULL`. The poller opens no SNMP session for devices without SNMP sensors; should such a device still have an
  SNMP sensor, it reports “This device has no SNMP set up (ping only).”
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
  in the code `devices::DeviceKind`; the icons are in the macro `device_icon` in `templates/home.html`. Forms without the field result in “Generic”.
- SNMP client `src/snmp/mod.rs`: `SnmpClient::connect` (including SNMPv3 engine discovery), `get` with timeout, retries on timeouts and a single retry after
  a v3 context update. The poller uses the same client.
- `snmp2` with the feature `heap_buffers`: without it, a 64 KB buffer lives on the stack, which overflows in Tokio threads (2 MB stack) in debug builds.

## Tests

- Unit and handler tests use a small **fake SNMPv2c agent** (`src/snmp/testagent.rs`) which, like real agents, stays silent on a wrong community.
- `tests/snmp_local.rs`: integration tests against the local `snmpd` (v2c, v3 with `OPENNETMON_TEST_V3_*`); they skip themselves when the agent or the variables
  are missing.
- `tests/omada_manual.rs`: manual tests against real Omada devices, all `#[ignore]`. Addresses and credentials come from the environment
  (`OPENNETMON_OMADA_SWITCH`, `OPENNETMON_OMADA_AP`, `OPENNETMON_OMADA_USER`, `OPENNETMON_PROBE_AUTH_PASS`), e.g. from a file outside the repository:
  `set -a; . ~/.config/opennetmon-dev/omada.env; set +a; cargo test --test omada_manual -- --ignored --nocapture`
