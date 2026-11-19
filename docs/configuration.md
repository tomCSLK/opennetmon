# Configuration

opennetmon reads its settings from a TOML file and from environment variables. Every setting can be given either way; a variable wins over the file.
Without a file the defaults below apply, so a server can also be configured with variables alone (that is how the container image works).

- **File:** `--config <file>`, else the file named in `OPENNETMON_CONFIG`, else none. The `.deb` and the installer use `/etc/opennetmon/config.toml`;
  [`deploy/config.example.toml`](../deploy/config.example.toml) is that file with comments. An unknown key is an error, so typos do not go unnoticed.
- **Variables:** `OPENNETMON_` plus the key in capitals, e.g. `OPENNETMON_POLL_CONCURRENCY=8`. Switches take `true` or `false`.
  An **empty** variable unsets an optional setting (the ones without a default below), e.g. `OPENNETMON_TLS_CERT_FILE=` and `OPENNETMON_TLS_KEY_FILE=`
  switch off built-in HTTPS that the file or the image sets.
- The subcommands (`user add`, `backup`, `healthcheck` …) read the same file and variables as the server, so they find the same database.
- Changes take effect when the server starts again (`sudo systemctl restart opennetmon`, `docker compose up -d`). The e-mail settings are not here
  but in the user interface ([`notifications.md`](notifications.md)).

## Settings

| Key | Variable | Default | Meaning |
|---|---|---|---|
| `listen` | `OPENNETMON_LISTEN` | `127.0.0.1:8080` | Address and port of the web interface. Without the `tls_` keys only loopback is sensible (plain HTTP behind a reverse proxy); with them any address, e.g. `0.0.0.0:8443` or `[::]:8443`. |
| `database_path` | `OPENNETMON_DATABASE_PATH` | `/var/lib/opennetmon/opennetmon.db` | The SQLite database. Its directory must be writable (SQLite adds `-wal` and `-shm` files). |
| `log_filter` | `OPENNETMON_LOG_FILTER` | `info` | What is logged, e.g. `info,opennetmon=debug`. `RUST_LOG` takes precedence. |
| `secret_key_file` | `OPENNETMON_SECRET_KEY_FILE` | `/etc/opennetmon/secret.key` | Key that encrypts the stored SNMP and SMTP passwords (64 hex characters, mode 0600). Back it up separately from the database. |
| `tls_cert_file` | `OPENNETMON_TLS_CERT_FILE` | unset | Certificate chain for built-in HTTPS (PEM). Together with `tls_key_file` the port speaks only HTTPS; plain HTTP there is redirected. |
| `tls_key_file` | `OPENNETMON_TLS_KEY_FILE` | unset | Its private key (PEM, mode 0600). |
| `generate_missing` | `OPENNETMON_GENERATE_MISSING` | `false` | Create a missing key file and, when both `tls_` files are set and neither exists, a self-signed certificate at startup (the log shows its fingerprint). Existing files are never changed; a certificate without its key stops the start. The container image sets it; the `.deb` and the installer create both files during installation instead. |
| `cookie_secure` | `OPENNETMON_COOKIE_SECURE` | `true` | Mark the session cookie `Secure`. Keep it on with HTTPS and behind a TLS proxy; off only for plain-HTTP development. |
| `session_ttl_hours` | `OPENNETMON_SESSION_TTL_HOURS` | `168` | Longest session, 1 to 2160 hours. |
| `session_idle_minutes` | `OPENNETMON_SESSION_IDLE_MINUTES` | `30` | Sign out after this many minutes without activity, 0 to 1440 (`0` = never); see [`users-and-login.md`](users-and-login.md). |
| `client_ip_header` | `OPENNETMON_CLIENT_IP_HEADER` | unset | Only behind a reverse proxy: `x-forwarded-for` (last entry) or `x-real-ip`, for the sign-in throttling. Only set it when the server is reachable through the proxy alone. |
| `poller_enabled` | `OPENNETMON_POLLER_ENABLED` | `true` | Poll the devices. Off for a second instance that only shows data. |
| `poll_concurrency` | `OPENNETMON_POLL_CONCURRENCY` | `16` | Devices polled at the same time, 1 to 256. |
| `retention_raw_days` | `OPENNETMON_RETENTION_RAW_DAYS` | `7` | Days raw measurements are kept, 1 to 3650. |
| `retention_hourly_days` | `OPENNETMON_RETENTION_HOURLY_DAYS` | `365` | Days hourly values and status events are kept, at least `retention_raw_days`, at most 36500. Daily values are kept forever. |
| `retention_audit_days` | `OPENNETMON_RETENTION_AUDIT_DAYS` | `365` | Days entries of the audit log are kept, 1 to 36500. |
| `update_check` | `OPENNETMON_UPDATE_CHECK` | `false` | Ask GitHub once a day for a newer release and tell administrators ([`operations.md`](operations.md#notice-of-new-versions)). On in the shipped config and in the container image. |
| `time_zone` | `OPENNETMON_TIME_ZONE` | unset | Time zone of the maintenance windows, of `backup_time` and of the times in notification mails, e.g. `Europe/Berlin`. Unset: the system's (`TZ`, else `/etc/localtime`, else UTC). The log at startup names the zone in use. |
| `backup_dir` | `OPENNETMON_BACKUP_DIR` | unset | Directory for a daily backup made by the server itself. Unset: none. The container image sets `/var/lib/opennetmon/backups`; the `.deb` uses its systemd timer instead (see [Backups](#backups)). |
| `backup_keep` | `OPENNETMON_BACKUP_KEEP` | `14` | How many of those backups are kept, 1 to 1000. |
| `backup_time` | `OPENNETMON_BACKUP_TIME` | `03:30` | Local time of that backup, `HH:MM`. |
| `packaging` | `OPENNETMON_PACKAGING` | `deb` | How this copy was installed: `deb` (package or installer) or `container`. It decides whether the update notice names `apt install` or `docker compose pull`; the container image sets it. |

## Time zones

The binary carries a copy of the time zone database. opennetmon uses it only when the system has none (the container image has no
`/usr/share/zoneinfo`); otherwise the system's copy, which the distribution keeps up to date, wins. `TZ=Europe/Berlin` therefore works in the
container as well.

## Backups

With `backup_dir` set, the server writes a consistent copy of the database every day at `backup_time`: `opennetmon-YYYYMMDD-HHMMSS.db` (local time),
mode 0600, while it keeps running. It keeps the newest `backup_keep` copies and deletes older ones; other files in the directory (e.g. a
`before-0.7.0.db` you made by hand) stay. A copy is first written as `….db.part` and renamed when it is complete. If the newest copy is more than
25 hours old when the server starts (it was off at backup time), it makes one five minutes after the start.

Like every backup of opennetmon, the copies hold the encrypted passwords but **not** the key file (`secret_key_file`); back that up separately, and keep
both apart. Restoring works as described in [`operations.md`](operations.md#backup-and-restore).

## Health check

`opennetmon healthcheck` asks the running server for `/healthz` on its own address (an address on all interfaces is reached over loopback) and exits
with status 0 when the answer is OK, otherwise with 1 and a message. With built-in HTTPS it does not check the certificate, because the other end is the
server itself. The container image uses it as its `HEALTHCHECK`; monitoring tools can call `/healthz` directly instead.
