# Sensors and measurements

A **sensor** belongs to a device and returns one or more **channels** (values) on every poll.
All sensor types implement the trait `Sensor` (`src/sensors/mod.rs`); the poller knows no special types.

| Type (`kind`) | Channels | Query |
|---|---|---|
| Ping (`ping`) | `rtt`, `rtt_min`, `rtt_max`, `jitter` (ms), `loss` (%) | 5 ICMP echo requests per poll (1–10), timeout as configured on the device |
| Uptime (`uptime`) | `uptime` (s) | `sysUpTime.0` |
| Interface traffic (`if_traffic`) | `in`, `out` (bit/s) | `ifHCInOctets`/`ifHCOutOctets` (64 bit), otherwise `ifInOctets`/`ifOutOctets` (32 bit) |
| Interface status (`if_status`) | `oper_status` (code) | `ifOperStatus` (1 = up, 2 = down, 3 = testing, 5 = dormant, 7 = lowerLayerDown) |
| OID (`oid`) | `value` | any numeric OID, mode *gauge* (value × factor) or *counter* (rate/s × factor) |
| This server (`local`) | depends on the value, see “This server” | reads the machine opennetmon runs on directly (no SNMP) |
| HTTP(S) (`http`) | `rtt` (ms) | one request to a URL, see “Services” |
| TCP port (`tcp`) | `rtt` (ms) | opens a TCP connection to a port of the device |
| DNS (`dns`) | `rtt` (ms) | asks the device as DNS server for an A or AAAA record |
| TLS certificate (`tls`) | `days` (days until expiry) | TLS handshake with a port of the device, reads the server's certificate |

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

## Services (`src/sensors/service.rs`, `src/sensors/dns.rs`, `src/http_client.rs`)

Sensors that check whether a service answers, not only whether the device is alive: a website, Home Assistant, an AdGuard instance, the Omada
controller, a reverse proxy and its certificate. They need no SNMP and work on devices with “No SNMP” as well; for a target without a device of its
own (e.g. a website) add a device without SNMP. They are added with “Add sensor”; the form shows only the fields of the chosen type
(`static/sensor-form.js`; without the script all fields are shown and the server ignores those of other types). The settings are stored as JSON in
`sensors.params` and can be changed later under “Settings” (see “User interface”).

| Type | Settings | Fine when | Message when fine |
|---|---|---|---|
| HTTP(S) | URL (empty: `https://<device>/`), expected status codes (codes and ranges, default `200-399`), optional keyword in the content (case-sensitive, searched in the first MiB), follow redirects (up to 5), check the certificate, only HEAD | the status is expected and the keyword found | `HTTP 200` |
| TCP port | port | the connection opens | `Port 22 open` |
| DNS | name, type A or AAAA, port of the server (53), optional expected address | the server answers with at least one address (and the expected one) | the addresses |
| TLS certificate | port (443), server name (SNI; empty: the device's address), check the certificate | the chain is trusted and the name matches (if checked) | `valid until 2026-12-27 · issued by R11` |

- **Failures** count like a failed ping: the first one is a warning, the second in a row is down. The message says why (connection refused, timeout,
  `Unexpected status code 503.`, keyword not found, wrong DNS answer, `The name … does not exist`, certificate not trusted with the reason).
- **Response time** (`rtt`): HTTP until the response head arrived (connection, TLS and request included; with redirects the last request), TCP the
  connection, DNS the answer. Thresholds as for any channel.
- **Certificates:** the channel `days` starts with thresholds *warning below 14 days* and *down below 3 days*, changeable like any threshold. Let's
  Encrypt renews 30 days before expiry, so a warning means the renewal failed. An expired certificate still reports its (negative) days; with “check
  the certificate” any other problem (unknown issuer, wrong name, self-signed) fails the sensor. Switch the check off for self-signed certificates
  (e.g. opennetmon itself or the Omada controller) to watch only their expiry.
- **HTTP client** (`http_client`): HTTP/1.1, one connection per request, rustls with the Mozilla root certificates (`webpki-roots`, no OpenSSL),
  `User-Agent: opennetmon/<version>`, no cookies, one deadline (the device's timeout) for the whole request including redirects. URLs with a user
  name or password are refused, because the URL is stored and shown as it is. The certificate's expiry and issuer are read with `x509-parser`.
- **DNS:** a small query of its own over UDP (`sensors::dns`: query, answer with compressed names, A/AAAA). It asks the device directly rather than the
  system resolver, so a failing AdGuard instance shows up even while the other one answers for the system.
- The timeout is the device's; as with SNMP, choose the interval of a certificate sensor freely (e.g. 3600 s per sensor).
- Tests: local HTTP, HTTPS and DNS test servers (`http_client::testserver`, `dns::testserver`); certificates from `rcgen` with chosen expiry.

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
  (`if_index`; `oid`, `oid_mode`, `scale`; `target` for “This server”; `params` (JSON) for services); a `CHECK` enforces that exactly the matching
  columns are set; `enabled`. New types do not fit the type check of migration 0005: `storage::current_sensors_table` rebuilds the table once
  (outside a migration transaction, foreign keys checked afterwards) and copies the columns the old table has.
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

Like PRTG, a ping sensor sends **several echo requests per poll** (since 0.5.6; “Packets per poll”, 1 to 10, default 5, on “Add sensor” and the settings
page, stored as JSON in `sensors.params`). The requests go out 0.1 s apart and wait for their replies side by side, each up to the device's timeout, so a poll
takes at most the timeout plus 0.9 s. Channels:

| Channel | Meaning |
|---|---|
| Response time (`rtt`) | average of the replies (before 0.5.6 the time of the single reply; the history goes on) |
| Minimum, Maximum (`rtt_min`, `rtt_max`) | fastest and slowest reply of the poll |
| Jitter (`jitter`) | mean difference between consecutive replies; no value with fewer than two replies |
| Packet loss (`loss`) | lost requests in percent |

A poll **fails only when no reply comes back** (then as before: the first failure is a warning, the second in a row down). Single lost packets are a value of
the channel “Packet loss”; new ping sensors warn above 20 % (`PING_WARN_LOSS`), changeable like every threshold. Ping sensors from before 0.5.6 get the new
channels without thresholds (migration `0024`) and send 5 packets. Lists and their small charts show only the response time; the sensor page shows all
channels, the table of values hides minimum, maximum and jitter on phones. Device templates keep the number of packets (`Basic.packets`).

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
  With `?group=<id>` (the tiles of a group page) a list only contains the sensors of that group and its subgroups; tiles and sort links keep the filter,
  the path shows the group, and “all groups” drops the filter. An unknown group answers 404.
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
  Administrators additionally see the icons “Pause” ‖ and “Delete” per sensor, at the top “Pause device” or “Resume” (`docs/devices-and-groups.md`, “Pausing”), “Find
  interfaces”, “Find values” (`docs/mibs.md`) and the form “Add sensor” (ping, uptime, custom OID with the mode value/counter and a factor, this server;
  a decimal comma is accepted). Errors appear on the same page, the input is kept.
- **Order**: automatically by type (ping, uptime, traffic, interface status, then custom values), within a type by name with numbers by value (port 2 before
  port 10). “Change order” (`/devices/<id>/sensors/order`, administrators only) arranges them by hand by dragging, ↑/↓ or position number; the rules are the
  same as for devices (`docs/devices-and-groups.md`).
- **Delete**: the trash icon in the list leads to the section “Delete sensor” at the end of the detail page (`/sensors/<id>#delete`, administrators only);
  deleting needs the tick “Yes, delete this sensor” (`confirm=yes`, otherwise `422`) and removes the sensor with its measurements and events.
- **Settings** (`/sensors/<id>/edit`, administrators only, in the menu “⋯ Actions” of the detail page; `web::sensor_settings`, described in
  `docs/charts-and-thresholds.md`, “Settings page”): name, interval, thresholds and status ratings for every sensor; for a custom OID sensor (OID, mode,
  factor, unit) or a service sensor (URL, ports, names, checks) also the fields it was added with, the same fieldsets as “Add sensor”
  (`templates/sensor_fields.html`, saved by `storage::sensors::update_spec`). Renaming keeps values, chart, thresholds, status and events; the type
  cannot change. The poller uses new settings from its next cycle (a changed setting builds a new sensor instance, so a counter starts afresh).
  Changing an OID sensor between value and counter changes the unit of its channel; older values keep the old meaning. A ping sensor has its packets
  per poll. Uptime, interface and “This server” sensors have nothing type-specific (their interface or value is what they are). Changes appear field by field in the audit log.
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
