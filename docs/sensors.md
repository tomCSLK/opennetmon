# Sensors and measurements

A **sensor** belongs to a device and returns one or more **channels** (values) on every poll.
All sensor types implement the trait `Sensor` (`src/sensors/mod.rs`); the poller knows no special types.

| Type (`kind`) | Channels | Query |
|---|---|---|
| Ping (`ping`) | `rtt` (ms) | ICMP echo, timeout as configured on the device |
| Uptime (`uptime`) | `uptime` (s) | `sysUpTime.0` |
| Interface traffic (`if_traffic`) | `in`, `out` (bit/s) | `ifHCInOctets`/`ifHCOutOctets` (64 bit), otherwise `ifInOctets`/`ifOutOctets` (32 bit) |
| Interface status (`if_status`) | `oper_status` (code) | `ifOperStatus` (1 = up, 2 = down, 3 = testing, 5 = dormant, 7 = lowerLayerDown) |
| OID (`oid`) | `value` | any numeric OID, mode *gauge* (value × factor) or *counter* (rate/s × factor) |
| This server (`local`) | depends on the value, see “This server” | reads the machine opennetmon runs on directly (no SNMP) |

Traffic rates are in bit/s (octets × 8). A poll needs exactly one SNMP GET per sensor; counters also fetch `sysUpTime`.

## This server (`src/sensors/local.rs`, page `web::server_values`)

Sensors for the machine opennetmon itself runs on – without `snmpd`, without credentials, read-only. What is measured is stored in the column
`sensors.target`:

| Value (`target`) | Channels | Source | Limits when created |
|---|---|---|---|
| CPU load (`cpu`) | `total`, `iowait` (%) | `/proc/stat`, difference of two polls (the first one returns no value yet) | none (short peaks are normal) |
| Load (`load`) | `load1`, `load5`, `load15` | `/proc/loadavg` | none |
| Memory (`memory`) | `used` (%, without buffers/cache), `available` (bytes), `swap_used` (%; 0 without swap) | `/proc/meminfo` (`MemAvailable`) | `used`: warning above 90 %, down above 95 % |
| Uptime (`uptime`) | `uptime` (s) | `/proc/uptime` | none |
| File system (`disk:<mount point>`) | `used` (%), `free` (bytes) | `statvfs` (like `df`: reserved blocks count as used), at most 5 s | `used`: warning above 85 %, down above 95 % |
| Temperature (`temp:<chip>/<label>`) | `temperature` (°C) | `/sys/class/hwmon/*/temp*_input` | warning above `temp*_max`, down above `temp*_crit` (if the hardware reports them) |
| Network traffic (`net:<interface>`) | `in`, `out` (bit/s) | `/sys/class/net/<interface>/statistics/rx_bytes`/`tx_bytes` (64 bit) | none |

- **Creating them:** on the sensor page of any device, choose the type “This server (CPU, memory, file systems …)” under “Add sensor”, or use the button
  “Values of this server” at the top (administrators only). The button appears on devices of type **Server**, on devices with the address `127.x.x.x`, `::1` or
  `localhost`, and on devices that already have such sensors. The page lists what the machine currently reports, with the current value; general values,
  local file systems and active network cards are preselected, temperatures and network drives are not. Existing values are ticked and locked; only what the
  machine offers at that moment is created.
- **Recommendation:** create the server as a device of its own (type “Server”, without SNMP, with its LAN address for the ping). The values, however, always come
  from the machine opennetmon runs on – even if they are created on another device.
- **File systems:** only real file systems (ext4, xfs, btrfs, zfs, vfat, NFS, CIFS …); pseudo file systems and snap images (squashfs) are left out. Every device
  appears once (bind mounts such as systemd's read-only view of `/` do not count). Under the unit's hardening, `/home` is not visible (`ProtectHome=yes`);
  if `/home` is a file system of its own, it is missing from the list.
- **Temperatures:** the hwmon numbers change between reboots, so the key consists of chip name and label (`coretemp/Package id 0`); a second chip with the
  same name is called `nvme#2`. If the sensor disappears after a hardware change, the sensor reports “Unusable value”.
- **Network:** all interfaces except `lo`; “Network card” means that a device is behind it (`/sys/class/net/<interface>/device`), otherwise “Virtual”
  (bridge, VPN, container). An interface that has disappeared reports an error.
- **Unit:** the sensors need the system files in `/proc`; `deploy/opennetmon.service` therefore no longer sets `ProcSubset=pid`
  (`systemd-analyze security` stays at 1.3). Everything else about the hardening stays.
- **Templates:** device templates do not take over these sensors; they belong to exactly one machine.
- **Database:** new type `local` and column `target`. SQLite can only change the table's `CHECK` by rebuilding the table; `storage::migrate` does that once
  at the first start with foreign keys switched off (`allow_local_sensors`, as for devices without SNMP).

## Counters and rates (`src/sensors/counter.rs`)

Rate = difference / time delta. **No value is written** (instead of an outlier) if

- it is the first value after the start or after a reset (a reset costs exactly one value),
- `sysUpTime` became smaller (the device rebooted),
- a 64-bit counter went down,
- a 32-bit counter went down and neither `sysUpTime` nor a plausible rate limit confirm a real wrap,
- no time has passed.

A confirmed 32-bit wrap (access points) is calculated correctly. If a device switches between 64-bit and 32-bit counters, the sensor starts over.

## Database (migration `0005`)

- `sensors`: device (`ON DELETE CASCADE`), name (unique per device, case-insensitive), type and the columns matching the type
  (`if_index`; `oid`, `oid_mode`, `scale`); a `CHECK` enforces that exactly the matching columns are set; `enabled`.
- `channels`: per sensor the channels of its type (`key`, `unit`), created in the same transaction as the sensor.
- `samples`: `(channel_id, ts, value)`, `ts` in milliseconds since the Unix epoch (UTC), primary key `(channel_id, ts)`
  (`WITHOUT ROWID`); a second value for the same timestamp is ignored. Deleting a sensor removes its channels and values too.
- Access: `storage::sensors` (`create_sensor`, `list_sensors`, `list_enabled_sensors`, `set_enabled`, `delete_sensor`, `insert_samples`, `samples_since`).

## Input (`src/sensors/spec.rs`)

OIDs only numeric (`1.3.6.1.2.1.1.3.0`, a leading dot is allowed, at least 2 and at most 128 parts), interface index 1 to 2147483647,
factor finite and not 0. On the switches the index is not consecutive (e.g. 49153).

## Ping

`surge-ping` with unprivileged ICMP sockets (`net.ipv4.ping_group_range`); without the permission, the address family stays unusable and the ping fails with
a clear message. On the server: set `ping_group_range` or `AmbientCapabilities=CAP_NET_RAW` in the systemd unit.

## Tests

Unit tests against the fake agent (`src/snmp/testagent.rs`, with a settable OID table), storage tests with a temporary database.
Manually against real hardware: `cargo test --test omada_manual sensors_work -- --ignored --nocapture` (switch 64 bit, AP 32 bit, ping).

## User interface (`src/web/sensors.rs`)

- **Overview** (`/`): per device the status (worst sensor status), a link to the sensor page and the number of sensors.
- **Status lists** (`/status/<filter>`, `src/web/status_list.rs`, all signed-in users): each tile of the overview links to the list of its sensors across all
  devices – `all` (sensors), `up` (OK), `warning`, `down` and `inactive` (Paused / unknown). At the top are the same tiles as on the overview (shared template
  `templates/status_tiles.html`, `status_list::tiles`); the list shown has a blue ring. Columns: status, sensor, device with group path, message, current value
  (as on the sensor page) and “Since” (time of the last status change and its duration; empty for paused sensors and sensors never polled; for paused ones the
  message names the pause). Sort by clicking the column headers (`?sort=status|sensor|device|value|since&dir=asc|desc`, without a script; clicking again
  reverses). Default: warning and down newest changes first, “Sensors” worst status first, otherwise by device. For the value, sensors with the same unit stay
  together (traffic first, then rates, ms, %, counts, uptime, status values); within a unit they are sorted by the sum of the channels (traffic: in + out).
  Rows without a value or time always come last; ties keep the sensor order per device.
- **Sensor page** (`/devices/<id>/sensors`, all signed-in users): a table with name, type, status, last message, current value (one line per channel without
  wrapping, e.g. traffic in/out one below the other, channel names as tooltip; only if the last value is at most three intervals plus one minute old) and a
  **mini chart of the last hour** (`src/web/sparkline.rs`, SVG from the server, without a script; a click leads to the detail page):
  - Measurements (ping, traffic, custom OID): one line per channel in the colours of the large charts, a shared scale from 0, averages in sections of at least
    1 minute and at least two polling intervals (at 60 s, 30 sections of 2 minutes; `storage::measurements::bucket_averages`);
    sections without a value interrupt the line.
  - Status values, interface status and uptime: a status band (green/yellow/red) from the event log (`storage::status::status_history`), because a curve says
    nothing there.
  - Paused sensors have no chart; on narrow screens (below 560 px) the column is hidden.
  The time of the last poll is on the detail page.
  Administrators additionally see “Pause” and “Delete” per sensor, at the top “Pause device” or “Resume” (`docs/devices-and-groups.md`, “Pausing”), “Find
  interfaces”, “Find values” (`docs/mibs.md`) and the form “Add sensor” (ping, uptime, custom OID with the mode value/counter and a factor, this server;
  a decimal comma is accepted). Errors appear on the same page, the input is kept.
- **Order**: automatically by type (ping, uptime, traffic, interface status, then custom values), within a type by name with numbers by value (port 2 before
  port 10). “Change order” (`/devices/<id>/sensors/order`, administrators only) arranges them by hand by dragging, ↑/↓ or position number; the rules are the
  same as for devices (`docs/devices-and-groups.md`).
- **Rename** on the detail page (section “Name”, administrators only, `POST /sensors/<id>/rename`): at most 64 characters, unique per device;
  values, chart, thresholds, status and events are kept.
- **Interface discovery** (`POST /devices/<id>/discover`, administrators only, CSRF): `SnmpClient::walk` (GETBULK) reads `ifDescr`, `ifType`, `ifSpeed`,
  `ifOperStatus`, `ifName`, `ifHighSpeed` and `ifAlias` (`src/discovery.rs`) and merges them per `ifIndex`. Name = `ifName`, otherwise `ifDescr`
  (access points have no `ifXTable`), otherwise “Interface N”. The page shows two ticks per interface (traffic, status). **Preselected** is traffic for
  active ports of the types Ethernet/WLAN; existing sensors are locked. Like the connection test, discovery is limited to a 10 s timeout and 1 retry,
  and at most 2000 rows per table.
- **Paused sensors** (paused themselves: `enabled = false`, status “Paused”) are not polled and appear in a collapsible section of their own,
  “Paused sensors (n)”, with the pause in the column “Message” and bulk actions (resume, resume all, delete; `POST /devices/<id>/sensors/bulk`,
  only sensors of this device). If the whole device or its group is paused, a note appears above and all sensors count as paused.
- **Create** (`POST /devices/<id>/sensors/interfaces`): fields `t_<ifIndex>`, `s_<ifIndex>` and `n_<ifIndex>` (display name). Sensor names “Traffic <name>” or
  “Status <name>” (at most 64 characters); with an equal name, “ #<index>” is appended. Existing interface sensors are never created twice.
- Measured on real hardware: a switch (9 interfaces, indexes 1 and 49153–49160, `ifName` e.g. “gigabitEthernet 1/0/1”, alias “Port1”) and an AP (13 interfaces,
  only `ifDescr`, e.g. `eth0`, `ath0`).
