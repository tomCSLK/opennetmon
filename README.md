# opennetmon

<p align="center"><img src="docs/assets/opennetmon-logo.jpg" alt="opennetmon – Network Monitoring" width="600"></p>

Agentless network monitoring over SNMP with a web interface, inspired by the way PRTG is operated.
A single, statically linked binary (Rust) with SQLite – meant to run on a headless Ubuntu server and to be used in the browser.

[![CI](https://github.com/tomCSLK/opennetmon/actions/workflows/ci.yml/badge.svg)](https://github.com/tomCSLK/opennetmon/actions/workflows/ci.yml)

> “PRTG” is a trademark of Paessler; opennetmon is not affiliated with Paessler.
> The target environment is a private network with TP-Link Omada (switches and access points via SNMPv3); it works with any device that speaks SNMP v2c or v3.

> **Language:** The user interface is available in German and English (language per user, by default the browser's).
> The documentation in [`docs/`](docs/) is in English; only the internal planning files (`PLAN.md`, `CLAUDE.md`) are written in German.

## Try it – testers welcome

opennetmon works and is tested on real hardware (TP-Link Omada); since 0.4.4 the releases are regular releases, no longer pre-releases.
Feedback from your network is very welcome – especially devices from other vendors.

1. Download the `.deb` package and `SHA256SUMS` from the [latest release](https://github.com/tomCSLK/opennetmon/releases) (amd64 only for now;
   for other systems with systemd there is an archive with `deploy/install.sh`).
2. Install and create the first administrator:

   ```bash
   sha256sum -c SHA256SUMS --ignore-missing
   sudo apt install ./opennetmon_<version>-1_amd64.deb
   sudo -u opennetmon opennetmon --config /etc/opennetmon/config.toml user add admin --role admin
   sudo systemctl start opennetmon
   ```

3. Open `https://<server address>:8443`. The browser warns once about the self-signed certificate; compare its fingerprint with the
   one printed during installation, or use your own certificate ([`docs/operations.md`](docs/operations.md)).
4. Report what you find as an [issue](https://github.com/tomCSLK/opennetmon/issues/new/choose) – never include passwords or keys.

## Status

Working state after MVP step 6 of 6. Details and order: [`PLAN.md`](PLAN.md).

| | |
|---|---|
| Done | Login with roles (admin/reader), SNMP profiles (v2c, v3 authNoPriv/authPriv, stored encrypted), groups and devices, connection test, sensors (ping, uptime, interface traffic, interface status, custom OID), interface discovery, renaming sensors, custom order of groups, devices and sensors (drag or ↑/↓), user management in the UI, devices without SNMP (ping and services), rating of status values, device templates (created from a device, applied automatically by device ID when adding a device), MIB import with “Find values” (tables as initially paused sensors), poller with status logic and event log, thresholds, charts (1 h / 24 h / 7 d / 30 d, up to 120 averaged points), rollups and retention, e-mail notifications (configured in the UI, test mail, send log), German and English (user interface per user, notification mails in one selectable language), sensor list with current values and a chart of the last hour, sortable sensor lists by status (opened from the status tiles), pausing groups, devices and sensors like PRTG (until resumed or for a set time, with a reason), dependencies like PRTG (objects pause while a sensor they depend on is down, e.g. the switches behind a failed router; with a delay after recovery and master sensors), maintenance windows like PRTG's schedules (weekly or once, in the server's time zone; paused without mails), availability reports per device and sensor (7/30 days or a month, CSV), a log of warnings, outages and pauses (the last 500 status changes; searchable, sortable, per device or sensor), a polling interval per sensor, automatic sign-out after 30 minutes without activity, sensors for the server opennetmon runs on (CPU, load, memory, file systems, temperatures, network traffic, uptime; read directly, no SNMP), sensors for services (HTTP(S) with status code and keyword, TCP port, DNS server, days until a TLS certificate expires), built-in HTTPS with a generated certificate, Debian package, notice of new versions for administrators (once a day from GitHub, can be switched off), an audit log of changes and sign-ins |
| Open | End-to-end integration test, polish |
| Not planned | SNMP traps, NetFlow/sFlow, distributed probes, maps, SNMP SET, Omada controller API |

## Features

- **Devices and groups:** hierarchical groups, devices with their own interval, timeout and retries; connection test with one click.
- **SNMP:** read-only. SNMPv2c and SNMPv3 (MD5, SHA-1/2; DES, AES) with reusable profiles. 64-bit counters (`ifXTable`) with automatic fallback to 32 bit (`ifTable`).
- **Sensors:** ping (several packets per poll: average, minimum, maximum, jitter, packet loss), uptime, interface traffic, interface status, custom OID (value or counter, with a factor). Interfaces are found by a walk and added by ticking a box; further values (PoE per port, CPU, memory, temperatures, fans …) are found in the MIB tables by “Find values” and created **paused** until an administrator resumes them. “Values of this server” adds sensors for the machine opennetmon runs on (CPU, load, memory and swap, file systems, temperatures, network traffic, uptime), read from `/proc`, `/sys` and `statvfs` without SNMP. **Services** are checked without SNMP: HTTP(S) (status code, keyword, response time), TCP ports, DNS servers (asked directly, with an expected address) and TLS certificates (warning 14 days before they expire).
- **Counters done right:** wraps and device reboots are detected; in that case there is **no** sample instead of a spike.
- **Status:** `OK`, `Warning`, `Down`, `Unknown`, `Paused`. A single failed poll is only a warning, the second in a row is down. Thresholds per channel (warning/down, above/below). Groups, devices and sensors can be paused until resumed or for 15 minutes up to a week, with an optional reason; everything below a paused group or device is paused too, sends no mails and starts afresh when resumed. Objects can depend on a sensor: while it is down or paused they are paused too, so an outage of the router sends one mail instead of one per sensor behind it.
- **User interface:** sidebar, status tiles (each opens a sortable list of its sensors: by status, name, device, value or time of the last change) and a device tree like PRTG's (collapsible groups, an icon per device type, status labels with sensor counts) with a page per group (its subtree and tiles limited to it); automatic order (port 2 before port 10, sensors by type) or by hand via drag and drop; a refresh button with automatic reload every 30 s (paused while you type); light and dark theme (follows the system, switchable); usable on a phone.
- **Maintenance windows:** schedules like PRTG's with weekly (“Mon–Fri from 03:00, 15 min”) or single windows, assigned to groups, devices or sensors and inherited downwards; during a window everything is paused without mails, one log entry per window.
- **Reports:** availability and downtime per device or sensor for 7 or 30 days or a calendar month, sortable, as CSV; tiles for 24 h / 7 / 30 days on each sensor page. Paused time and gaps do not count.
- **Samples table:** below each chart all values of the period (newest first, with volume for traffic, in pages of 50, 100 or 250 rows) plus sum, average, minimum and maximum; failed polls show as red or yellow rows with their message – like the live data in PRTG.
- **Users:** administrators and readers; managed in the UI (create, change role, set password, delete; the last administrator stays protected); an audit log for administrators (who changed what and when, old → new, sign-ins; never secrets); signed out after 30 minutes without activity (configurable; an exception per user for wall displays); under “My account” everyone changes their own password and language. The first administrator is created on the command line.
- **Devices without SNMP:** add routers, printers, cameras or websites without SNMP as “No SNMP – ping and services”; the ping sensor is added automatically.
- **Status values:** rate named values (e.g. PoE status overload, fan failed, port down) individually as OK, warning or down; new sensors get a suggestion based on the names.
- **Device templates:** save the sensors of a fully set-up device as a template (ports as “active”, “all” or “the same”) and apply it automatically to further devices by their device ID (`sysObjectID`); only missing sensors are created.
- **MIB files:** standard MIBs built in, vendor MIBs (e.g. TP-Link) by upload, listed by vendor (from the enterprise number of their objects) with problems first and a filter; custom OID sensors by name instead of a string of numbers, with type, unit and plain text for status values from the MIB.
- **Charts:** [uPlot](https://github.com/leeoniya/uPlot), bundled locally (no CDN); all times (charts and tables) in the browser's time zone.
- **Log:** warnings, outages, recoveries, pauses, dependencies and maintenance windows of all sensors, searchable and sortable by every column; the “Log” button of a device or sensor opens just its entries.
- **Notifications:** e-mail on down, warning and recovery; simultaneous changes (e.g. a failed device) arrive as one mail. SMTP server, login and recipients can be changed in the UI at any time (test mail included).
- **Data:** SQLite in WAL mode; raw values 7 days, hourly values 1 year, daily values forever (configurable).

## Quick start (development)

Requirement: a current stable Rust toolchain (edition 2024).

```bash
cargo run -- --config ./dev/config.toml key generate                    # once: key for stored SNMP credentials
cargo run -- --config ./dev/config.toml user add admin --role admin     # create the first administrator (asks for the password)
cargo run -- --config ./dev/config.toml                                 # start the server, http://127.0.0.1:8080
```

Then in the browser: create an **SNMP profile** → **New device** → on the device **Find interfaces** → create sensors. Values appear after the first polling interval.
E-mails can be tried without a real mail server with `python3 dev/smtp_sink.py` (server `127.0.0.1`, port `1025`, “No encryption”); the mails end up in `dev/data/mail/`.

Tests and quality:

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

The tests need no network (built-in fake SNMP agent). Optionally, further tests run against a local `snmpd` ([`docs/dev-testagent.md`](docs/dev-testagent.md))
and manual `#[ignore]` tests against real devices (`tests/omada_manual.rs`).

## Configuration

TOML file (`--config`) plus environment variables (`OPENNETMON_LISTEN`, `OPENNETMON_DATABASE_PATH`, `OPENNETMON_LOG_FILTER`, `OPENNETMON_SECRET_KEY_FILE`).
Example with all keys and their defaults (the installed `/etc/opennetmon/config.toml` instead uses `listen = "0.0.0.0:8443"` with the two `tls_` keys, see below):

```toml
listen = "127.0.0.1:8080"                  # plain HTTP, e.g. behind a reverse proxy; with tls_* any address, HTTPS only
database_path = "/var/lib/opennetmon/opennetmon.db"
log_filter = "info"
secret_key_file = "/etc/opennetmon/secret.key" # mode 0600, otherwise the server does not start
# tls_cert_file = "/etc/opennetmon/tls-cert.pem" # built-in HTTPS: certificate chain (PEM)
# tls_key_file = "/etc/opennetmon/tls-key.pem"   # and its key (mode 0600)
cookie_secure = true                       # set to false only for development without TLS
session_ttl_hours = 168
session_idle_minutes = 30                  # sign out after 30 minutes without activity (0 = off)
poller_enabled = true
poll_concurrency = 16                      # devices polled at the same time
retention_raw_days = 7
retention_hourly_days = 365                # daily values are kept forever
retention_audit_days = 365                 # audit log (changes, sign-ins)
update_check = false                       # once a day ask GitHub for a newer release (on in the installed config)
# time_zone = "Europe/Berlin"              # zone of the maintenance windows; unset = the system's
```

## Running on a server

On Ubuntu or Debian (amd64), install the `.deb` package (built with `dev/deb.sh`, which needs `cargo install cargo-deb --locked`):

```bash
sudo apt install ./opennetmon_<version>-1_amd64.deb
sudo -u opennetmon opennetmon --config /etc/opennetmon/config.toml user add admin --role admin
sudo systemctl start opennetmon
```

Elsewhere, or without the package, use the installer script:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl     # static binary, no glibc dependency
sudo deploy/install.sh --binary target/x86_64-unknown-linux-musl/release/opennetmon   # on the server (after copying the binary and deploy/)
sudo -u opennetmon /usr/local/bin/opennetmon --config /etc/opennetmon/config.toml user add admin --role admin
sudo systemctl start opennetmon
```

Then open **`https://<server address>:8443`** from any computer in the network (allow port 8443 in the firewall if one is active).

Both create the user, configuration, key, a self-signed HTTPS certificate for the server's host name and addresses, a hardened systemd unit, the ping
permission and a daily database backup, and leave existing configuration, key, certificate and data untouched on updates (the package installs to `/usr`,
the script to `/usr/local`). The browser warns about the self-signed certificate once; compare the SHA-256 fingerprint printed on installation (and logged on
every start) before accepting it, or use your own certificate. [`docs/operations.md`](docs/operations.md) covers your own certificate (including a small private CA
and Let's Encrypt), port 443, running behind Caddy or nginx instead, update, rollback, backup, restore and troubleshooting.

## Security

- SNMP is used **read-only**.
- SNMP credentials are stored in the database encrypted with XChaCha20-Poly1305; the key is in a separate file. **Back up this file separately from the database** – without it the profiles are unusable.
- Passwords with Argon2id, server-side sessions (signed out after 30 minutes without activity, with an exception per user for wall displays), CSRF protection on all forms, `HttpOnly`/`SameSite=Strict` cookies, a strict Content Security Policy (no inline scripts).
- Built-in HTTPS (rustls, TLS 1.2/1.3): when configured, the port speaks only HTTPS, and plain HTTP gets nothing but a redirect. Without TLS settings it listens on `127.0.0.1` only, for a reverse proxy in front.
- Failed logins are slowed down per user name and per address (rejection with a waiting time, never a permanent lock). Behind a proxy, set `client_ip_header`.
- The SMTP password is stored encrypted like the SNMP credentials and never shown.
- Secrets never belong in the repository, in logs or in HTML output.

Please report vulnerabilities privately, not as an issue: see [`SECURITY.md`](SECURITY.md).

## Documentation

| | |
|---|---|
| [`PLAN.md`](PLAN.md) | Phases, status, next steps (German) |
| [`CLAUDE.md`](CLAUDE.md) | Architecture, conventions, commands (working instructions for development with Claude Code; German) |
| [`docs/users-and-login.md`](docs/users-and-login.md) | Users, roles, sessions, language, audit log |
| [`docs/snmp-profiles.md`](docs/snmp-profiles.md) | SNMP profiles and encryption |
| [`docs/devices-and-groups.md`](docs/devices-and-groups.md) | Devices, groups, pausing, dependencies, maintenance windows, connection test |
| [`docs/sensors.md`](docs/sensors.md) | Sensor types, services (HTTP, TCP, DNS, TLS), counter logic, discovery |
| [`docs/poller.md`](docs/poller.md) | Polling, status logic, event log |
| [`docs/charts-and-thresholds.md`](docs/charts-and-thresholds.md) | Detail page, charts, thresholds, status values, rollups |
| [`docs/reports.md`](docs/reports.md) | Availability: counting, reports, CSV |
| [`docs/operations.md`](docs/operations.md) | Installation, proxy, update and notice of new versions, backup, troubleshooting |
| [`docs/device-templates.md`](docs/device-templates.md) | Device templates: create from a device, apply when adding a device or later |
| [`docs/mibs.md`](docs/mibs.md) | MIB files, custom OID by name, finding values |
| [`docs/notifications.md`](docs/notifications.md) | E-mail settings, rules, test SMTP server |
| [`docs/translations.md`](docs/translations.md) | Translations: Fluent catalogs, rules, texts in JavaScript, adding a language |
| [`docs/decisions/0001-language.md`](docs/decisions/0001-language.md) | Decision for Rust (spike results) |

## Contributing

Bug reports, tests with other devices and pull requests are welcome. Please read [`CONTRIBUTING.md`](CONTRIBUTING.md) first: commits need
a DCO sign-off (`git commit -s`), and `cargo fmt`, `cargo clippy` and `cargo test` must pass.

## License

Copyright © 2026 Thomas Cieslik

opennetmon is free software under the **GNU Affero General Public License, version 3 or (at your option) any later version**
(`AGPL-3.0-or-later`; the full and solely authoritative text is in [`LICENSE`](LICENSE)). It may be used, modified and redistributed,
including commercially – **without any warranty**.

Anyone who offers a **modified** version to others over a network must offer them its source code (section 13). For this, the user interface
links to `repository` from `Cargo.toml` in the sidebar and on the login page – for your own version, enter the address of your own source code there.

Third-party software in the binary (Rust crates, uPlot under MIT, standard MIBs from IETF RFCs) with all license texts:
[`THIRD-PARTY-LICENSES.txt`](THIRD-PARTY-LICENSES.txt), generated with `python3 -I dev/third-party-licenses.py` (regenerate after changes to
`Cargo.lock`; `--check` verifies that the file is up to date).
