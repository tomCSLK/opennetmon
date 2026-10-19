# opennetmon

<p align="center"><img src="docs/assets/opennetmon-logo.jpg" alt="opennetmon – Network Monitoring" width="600"></p>

Agentless network monitoring over SNMP with a web interface, inspired by the way PRTG is operated.
A single, statically linked binary (Rust) with SQLite – meant to run on a headless Ubuntu server and to be used in the browser.

> “PRTG” is a trademark of Paessler; opennetmon is not affiliated with Paessler.
> The target environment is a private network with TP-Link Omada (switches and access points via SNMPv3); it works with any device that speaks SNMP v2c or v3.

> **Language:** The user interface is available in German and English (language per user, by default the browser's).
> The project documentation in [`docs/`](docs/) and the planning files are currently written in German.

## Status

Working state after MVP step 6 of 6. Details and order: [`PLAN.md`](PLAN.md).

| | |
|---|---|
| Done | Login with roles (admin/reader), SNMP profiles (v2c, v3 authNoPriv/authPriv, stored encrypted), groups and devices, connection test, sensors (ping, uptime, interface traffic, interface status, custom OID), interface discovery, renaming sensors, custom order of groups, devices and sensors (drag or ↑/↓), user management in the UI, devices without SNMP (ping only), rating of status values, device templates (created from a device, applied automatically by device ID when adding a device), MIB import with “Find values” (tables as initially inactive sensors), poller with status logic and event log, thresholds, charts (1 h / 24 h / 7 d / 30 d), rollups and retention, e-mail notifications (configured in the UI, test mail, send log), German and English (user interface per user, notification mails in one selectable language) |
| Open | Installation on the real server, test with the real mail server, htmx polish |
| Not planned | SNMP traps, NetFlow/sFlow, distributed probes, maps, SNMP SET, Omada controller API |

## Features

- **Devices and groups:** hierarchical groups, devices with their own interval, timeout and retries; connection test with one click.
- **SNMP:** read-only. SNMPv2c and SNMPv3 (MD5, SHA-1/2; DES, AES) with reusable profiles. 64-bit counters (`ifXTable`) with automatic fallback to 32 bit (`ifTable`).
- **Sensors:** ping, uptime, interface traffic, interface status, custom OID (value or counter, with a factor). Interfaces are found by a walk and added by ticking a box; further values (PoE per port, CPU, memory, temperatures, fans …) are found in the MIB tables by “Find values” and created **inactive** until an administrator activates them.
- **Counters done right:** wraps and device reboots are detected; in that case there is **no** sample instead of a spike.
- **Status:** `OK`, `Warning`, `Down`, `Unknown`, `Inactive`. A single failed poll is only a warning, the second in a row is down. Thresholds per channel (warning/down, above/below).
- **User interface:** sidebar, status tiles and device list with a sensor status bar; automatic order (port 2 before port 10, sensors by type) or by hand via drag and drop; light and dark theme (follows the system, switchable); usable on a phone.
- **Samples table:** below each chart all values of the period (newest first, with volume for traffic) plus sum, average, minimum and maximum – like the live data in PRTG.
- **Users:** administrators and readers; managed in the UI (create, change role, set password, delete; the last administrator stays protected); under “My account” everyone changes their own password and language. The first administrator is created on the command line.
- **Devices without SNMP:** add routers, printers or cameras without SNMP as “No SNMP – ping only”; the ping sensor is added automatically.
- **Status values:** rate named values (e.g. PoE status overload, fan failed, port down) individually as OK, warning or down; new sensors get a suggestion based on the names.
- **Device templates:** save the sensors of a fully set-up device as a template (ports as “active”, “all” or “the same”) and apply it automatically to further devices by their device ID (`sysObjectID`); only missing sensors are created.
- **MIB files:** standard MIBs built in, vendor MIBs (e.g. TP-Link) by upload; custom OID sensors by name instead of a string of numbers, with type, unit and plain text for status values from the MIB.
- **Charts:** [uPlot](https://github.com/leeoniya/uPlot), bundled locally (no CDN); all times (charts and tables) in the browser's time zone.
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
Example with all keys and their defaults:

```toml
listen = "127.0.0.1:8080"                  # TLS is handled by a reverse proxy in front (Caddy, nginx)
database_path = "/var/lib/opennetmon/opennetmon.db"
log_filter = "info"
secret_key_file = "/etc/opennetmon/secret.key" # mode 0600, otherwise the server does not start
cookie_secure = true                       # set to false only for development without TLS
session_ttl_hours = 168
poller_enabled = true
poll_concurrency = 16                      # devices polled at the same time
retention_raw_days = 7
retention_hourly_days = 365                # daily values are kept forever
```

## Running on a server

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl     # static binary, no glibc dependency
sudo deploy/install.sh --binary target/x86_64-unknown-linux-musl/release/opennetmon   # on the server (after copying the binary and deploy/)
sudo -u opennetmon /usr/local/bin/opennetmon --config /etc/opennetmon/config.toml user add admin --role admin
sudo systemctl start opennetmon
```

The installer creates the user, configuration, key, a hardened systemd unit, the ping permission and a daily database backup, and leaves existing configuration,
key and data untouched on updates. In addition: examples for Caddy and nginx (nginx also with login rate limiting) and a guide to update, rollback,
backup, restore and troubleshooting in [`docs/betrieb.md`](docs/betrieb.md).

## Security

- SNMP is used **read-only**.
- SNMP credentials are stored in the database encrypted with XChaCha20-Poly1305; the key is in a separate file. **Back up this file separately from the database** – without it the profiles are unusable.
- Passwords with Argon2id, server-side sessions, CSRF protection on all forms, `HttpOnly`/`SameSite=Strict` cookies, a strict Content Security Policy (no inline scripts).
- Listens only on `127.0.0.1` by default; for access from the network, put a TLS-terminating reverse proxy in front.
- Failed logins are slowed down per user name and per address (rejection with a waiting time, never a permanent lock). Behind a proxy, set `client_ip_header`.
- The SMTP password is stored encrypted like the SNMP credentials and never shown.
- Secrets never belong in the repository, in logs or in HTML output.

## Documentation

The documentation is in German.

| | |
|---|---|
| [`PLAN.md`](PLAN.md) | Phases, status, next steps |
| [`CLAUDE.md`](CLAUDE.md) | Architecture, conventions, commands (working instructions for development with Claude Code) |
| [`docs/benutzer-und-login.md`](docs/benutzer-und-login.md) | Users, roles, sessions, language |
| [`docs/snmp-profile.md`](docs/snmp-profile.md) | SNMP profiles and encryption |
| [`docs/geraete-und-gruppen.md`](docs/geraete-und-gruppen.md) | Devices, groups, connection test |
| [`docs/sensoren.md`](docs/sensoren.md) | Sensor types, counter logic, discovery |
| [`docs/poller.md`](docs/poller.md) | Polling, status logic, event log |
| [`docs/graphen-und-schwellwerte.md`](docs/graphen-und-schwellwerte.md) | Detail page, charts, thresholds, status values, rollups |
| [`docs/betrieb.md`](docs/betrieb.md) | Installation, proxy, update, backup, troubleshooting |
| [`docs/geraetevorlagen.md`](docs/geraetevorlagen.md) | Device templates: create from a device, apply when adding a device or later |
| [`docs/mibs.md`](docs/mibs.md) | MIB files, custom OID by name, finding values |
| [`docs/benachrichtigung.md`](docs/benachrichtigung.md) | E-mail settings, rules, test SMTP server |
| [`docs/uebersetzungen.md`](docs/uebersetzungen.md) | Translations: Fluent catalogs, rules, texts in JavaScript, adding a language |
| [`docs/decisions/0001-sprache.md`](docs/decisions/0001-sprache.md) | Decision for Rust (spike results) |

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
