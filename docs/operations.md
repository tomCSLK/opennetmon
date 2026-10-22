# Running on an Ubuntu server

opennetmon runs as a single service (`opennetmon.service`) and is reachable over HTTPS out of the box (`https://<server>:8443`, see “HTTPS and access
from the network”); a reverse proxy is possible but not needed. There is just one binary and one SQLite database; no database server and no runtime
environment are installed.

## Requirements

- Ubuntu Server (the unit is tested with systemd 255 on Ubuntu 24.04; it should run from systemd 247 on), root access via `sudo`.
- The server reaches the monitored devices: **UDP 161** (SNMP) and **ICMP** (ping), and for e-mail the SMTP server (usually TCP 587/465/25).
- The workstations reach the server on **TCP 8443** (HTTPS, built in). A reverse proxy (Caddy, nginx) is optional.

## Installing the Debian package (recommended)

For Ubuntu and Debian (amd64) there is a `.deb` package. It contains the same static binary and sets up the same things as the installer below, only under
`/usr` instead of `/usr/local`; updates and removal go through `apt`. Download it with `SHA256SUMS` from a GitHub release (check with `sha256sum -c SHA256SUMS
--ignore-missing`), or build it yourself with `dev/deb.sh` (needs `cargo install cargo-deb --locked`) or together with the archive with `dev/release.sh`.

```bash
dev/deb.sh                                                    # -> target/debian/opennetmon_<version>-1_amd64.deb
scp target/debian/opennetmon_*_amd64.deb user@server:/tmp/
```

On the server:

```bash
sudo apt install /tmp/opennetmon_<version>-1_amd64.deb
sudo -u opennetmon opennetmon --config /etc/opennetmon/config.toml user add admin --role admin
sudo systemctl start opennetmon
curl -sk https://127.0.0.1:8443/healthz        # -> ok
```

| Part | Location |
|---|---|
| Binary, backup script | `/usr/bin/opennetmon`, `/usr/sbin/opennetmon-backup` |
| Configuration | `/etc/opennetmon/config.toml` (a conffile of the package: a changed version is kept on updates, `dpkg` asks), directory `root:opennetmon 0750` (via `dpkg-statoverride`) |
| Key | `/etc/opennetmon/secret.key`, created on the first installation (0600, owner `opennetmon`), never overwritten |
| HTTPS | `/etc/opennetmon/tls-cert.pem` and `tls-key.pem` (self-signed for the server's host name and IP addresses, never overwritten; replaceable, see “HTTPS”) |
| Units | `/usr/lib/systemd/system/opennetmon.service` (enabled, **not** started on the first installation because there is no administrator yet), `opennetmon-backup.service`/`.timer` (active) |
| Ping | `/usr/lib/sysctl.d/99-opennetmon-ping.conf`; switch it off with a file of the same name in `/etc/sysctl.d/` (e.g. a link to `/dev/null`) |
| Licenses, examples | `/usr/share/doc/opennetmon/` (`copyright`, `THIRD-PARTY-LICENSES.txt`, `README.md`, `examples/` with Caddy and nginx) |

**Back up the key separately right away**, as described below. Your own changes to the units belong in a drop-in (`sudo systemctl edit opennetmon`), not in the
files under `/usr/lib`, which an update replaces.

- **Update:** install the new package with `sudo apt install ./opennetmon_<new>_amd64.deb`; a running service is restarted, configuration, key and data stay.
- **Rollback:** `sudo dpkg -i opennetmon_<old>_amd64.deb` (same restriction for migrations as under “Update and rollback”).
- **Removal:** `sudo apt remove opennetmon` stops the services and removes the program files; configuration, key, data, backups and user stay.
  `sudo apt purge opennetmon` also deletes `/etc/opennetmon` **including key and certificate**; database, backups and user stay even then (delete them for good as under “Uninstalling”).

### Switching from the installer to the package

Configuration, key and data are taken over; only the installer's program files have to go first, because units under `/etc/systemd/system` hide those of the package:

```bash
sudo systemctl disable --now opennetmon opennetmon-backup.timer
sudo rm /etc/systemd/system/opennetmon.service /etc/systemd/system/opennetmon-backup.* /usr/local/bin/opennetmon* /usr/local/sbin/opennetmon-backup
sudo rm -r /usr/local/share/doc/opennetmon
sudo systemctl daemon-reload
sudo apt install ./opennetmon_<version>-1_amd64.deb     # keep the existing config.toml (dpkg's default answer)
sudo systemctl enable --now opennetmon opennetmon-backup.timer
```

If files of the installer are left over, the package points this out during installation and lists the same steps. `enable` is needed if the package was already
installed before the cleanup: `systemctl disable` then also removes its activation, and `dpkg` keeps that decision for later installations.

## Installing with the installer script

Build the static binary on the development machine (no glibc dependency, runs on any 64-bit Linux):

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
scp -r target/x86_64-unknown-linux-musl/release/opennetmon deploy LICENSE THIRD-PARTY-LICENSES.txt user@server:/tmp/opennetmon-install/
```

Alternatively, take the ready-made archive `opennetmon-<version>-x86_64-linux.tar.gz` with `SHA256SUMS` from a GitHub release (check with
`sha256sum -c SHA256SUMS`, then unpack); it contains the binary, `deploy/`, `docs/` and the license files. It is built with `dev/release.sh`.

On the server:

```bash
cd /tmp/opennetmon-install
sudo ./deploy/install.sh --binary ./opennetmon
```

The installer can be run again at any time and does the following:

| Step | Result |
|---|---|
| User | system user `opennetmon` (no login, no home) |
| Binary | `/usr/local/bin/opennetmon`; the previous version stays as `opennetmon.previous` |
| Licenses | `/usr/local/share/doc/opennetmon/` (`LICENSE`, `THIRD-PARTY-LICENSES.txt`) |
| Configuration | `/etc/opennetmon/config.toml` (from `deploy/config.example.toml`, **never overwritten**), directory `root:opennetmon 0750` |
| Key | `/etc/opennetmon/secret.key` (newly generated, mode 0600, owner `opennetmon`; **never overwritten**) |
| HTTPS | `/etc/opennetmon/tls-cert.pem`, `tls-key.pem` (self-signed, key 0600, owner `opennetmon`; **never overwritten**) |
| Service | `/etc/systemd/system/opennetmon.service`, enabled; data under `/var/lib/opennetmon` (0750, owner `opennetmon`) |
| Ping | `/etc/sysctl.d/99-opennetmon-ping.conf` (allows unprivileged ICMP sockets; skip with `--no-sysctl`) |
| Backup | daily at 03:30 (`opennetmon-backup.timer`), 14 copies in `/var/backups/opennetmon` (skip with `--no-backup`) |

Then create the **first administrator** and start the service:

```bash
sudo -u opennetmon /usr/local/bin/opennetmon --config /etc/opennetmon/config.toml user add admin --role admin
sudo systemctl start opennetmon
curl -sk https://127.0.0.1:8443/healthz        # -> ok
```

Important: **back up `/etc/opennetmon/secret.key` right away, separately from the database** (password manager, USB stick, another machine). Without the key, all
stored SNMP and SMTP passwords are unusable. The automatic database backups deliberately do not contain the key.

## HTTPS and access from the network

opennetmon speaks HTTPS itself. Out of the box (package and installer) it listens on **all IPv4 addresses on port 8443**, encrypted only. From any
machine in the network:

```
https://<IP address or name of the server>:8443
```

- A request to `http://…:8443` only gets a redirect to `https://`; pages, login data and cookies never cross the wire unencrypted.
  The server speaks TLS 1.2 and 1.3 and rejects older versions.
- Firewall: if `ufw` is active, `sudo ufw allow 8443/tcp`. opennetmon needs no other ports.
- Another port or address: change `listen` in the config, e.g. `listen = "0.0.0.0:9443"`, a single network card `listen = "192.168.0.10:8443"`,
  IPv6 as well `listen = "[::]:8443"`; then `sudo systemctl restart opennetmon`. For ports below 1024 see “Port 443”.
- The server sees the real address of every machine, so the login rate limiting needs no `client_ip_header` (that is only for a proxy).

### The bundled certificate

During installation, `opennetmon tls generate` creates a **self-signed** certificate: `/etc/opennetmon/tls-cert.pem`, with the key `tls-key.pem` in mode 0600
and owned by `opennetmon`. It is valid for ten years for the host name, all IP addresses of the server (`hostname -I`) and `localhost`. The command prints the
**SHA-256 fingerprint**, and the service writes it to the log on every start:

```bash
journalctl -u opennetmon -b | grep fingerprint
openssl x509 -in /etc/opennetmon/tls-cert.pem -noout -fingerprint -sha256 -ext subjectAltName
```

On the first visit the browser warns (“connection is not secure”) because no certificate authority it knows has signed the certificate. Show the certificate in
the browser (click the warning or padlock icon), compare the fingerprint with the one above and confirm the exception. The connection is encrypted just as well as
with a purchased certificate; the warning only means that the browser cannot verify the server's identity by itself. To get rid of the warning entirely, use
your own certificate (below).

**Generate it again**, e.g. after the IP address or host name changed. The command never overwrites, so move the old files aside first; add one `--name` for each
address or name under which the server should be reachable (the host name, `localhost` and the loopback addresses are added automatically):

```bash
sudo mv /etc/opennetmon/tls-cert.pem /etc/opennetmon/tls-cert.pem.old
sudo mv /etc/opennetmon/tls-key.pem /etc/opennetmon/tls-key.pem.old
sudo opennetmon --config /etc/opennetmon/config.toml tls generate --name 192.168.0.10 --name opennetmon.lan
sudo chown opennetmon:opennetmon /etc/opennetmon/tls-key.pem
sudo systemctl restart opennetmon
```

### Using your own certificate

Any certificate in PEM format works, e.g. from a company CA, your own small certificate authority (below) or Let's Encrypt:

- **Certificate file:** PEM, the server certificate first, followed by any intermediate certificates (like `fullchain.pem`). It must contain the names or
  IP addresses under which browsers open the server (Subject Alternative Name); a name only in the CN is not enough for today's browsers.
- **Key file:** PEM without a password, PKCS#8 (`BEGIN PRIVATE KEY`), RSA (`BEGIN RSA PRIVATE KEY`) or EC (`BEGIN EC PRIVATE KEY`); mode **0600**, owner
  `opennetmon`, otherwise the service does not start.
- **Install it:** replace the bundled files (or enter other paths in `tls_cert_file`/`tls_key_file`) and restart:

  ```bash
  sudo install -m 0644 -o root -g root my-certificate.pem /etc/opennetmon/tls-cert.pem
  sudo install -m 0600 -o opennetmon -g opennetmon my-key.pem /etc/opennetmon/tls-key.pem
  sudo systemctl restart opennetmon
  journalctl -u opennetmon -n 5          # "HTTPS only …" with the fingerprint of the new certificate
  ```

- If the pair is unusable (key does not match, wrong format, permissions too open), the service reports it at startup with the file name and the reason and does not start.
- Certificates are only read at startup: after a renewal, `sudo systemctl restart opennetmon` (signed-in users stay signed in).

#### Your own small certificate authority (no more warnings in a home network)

Create a certificate authority (CA) once, import its certificate as trusted on your own machines and sign the server certificate with it. This works on any
machine with `openssl`. **Keep `ca-key.pem` safe and do not leave it on the server:** whoever has it can issue any certificate for all machines that trust
the CA.

```bash
# 1. The CA, once (ten years)
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 3650 \
  -subj "/CN=Home network CA" -keyout ca-key.pem -out ca-cert.pem \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"

# 2. Key and certificate for the server: adjust names and IP address.
#    825 days is the longest validity Apple devices accept.
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -subj "/CN=opennetmon" -keyout tls-key.pem -out tls.csr
printf 'subjectAltName = DNS:opennetmon.lan, IP:192.168.0.10\nbasicConstraints = CA:FALSE\nkeyUsage = critical, digitalSignature\nextendedKeyUsage = serverAuth\n' > tls.ext
openssl x509 -req -in tls.csr -CA ca-cert.pem -CAkey ca-key.pem -CAcreateserial -days 825 -extfile tls.ext -out tls-cert.pem
openssl verify -CAfile ca-cert.pem tls-cert.pem            # -> tls-cert.pem: OK
```

Then install `tls-cert.pem` and `tls-key.pem` as above and import `ca-cert.pem` on every machine that opens opennetmon:

| System / browser | Import |
|---|---|
| Windows (Edge, Chrome) | double-click the file (rename it to `ca-cert.crt` first) → “Install certificate” → “Local machine” → store “Trusted Root Certification Authorities” |
| macOS (Safari, Chrome) | Keychain Access → keychain “System” → drag the file in → double-click → “Trust” → “Always trust” |
| Firefox (all systems) | Settings → Privacy & Security → View certificates → Authorities → Import → “Trust this CA to identify websites” |
| Chrome/Chromium on Linux | Settings → Privacy and security → Security → Manage certificates → Authorities → Import |
| Linux tools (`curl` …) | `sudo cp ca-cert.pem /usr/local/share/ca-certificates/home-network-ca.crt && sudo update-ca-certificates` |

After 825 days, repeat step 2; the CA and its import stay.

#### Let's Encrypt

Only works with your own public domain (e.g. `opennetmon.example.org`, pointing to the server in the LAN). Because the server usually should not be reachable
from the internet, get the certificate through the DNS challenge (certbot with the DNS plugin of your domain provider). So that opennetmon picks up every
renewal, add a deploy hook, e.g. `/etc/letsencrypt/renewal-hooks/deploy/opennetmon` (executable; an example, not tested with opennetmon):

```sh
#!/bin/sh
install -m 0644 "$RENEWED_LINEAGE/fullchain.pem" /etc/opennetmon/tls-cert.pem
install -m 0600 -o opennetmon -g opennetmon "$RENEWED_LINEAGE/privkey.pem" /etc/opennetmon/tls-key.pem
systemctl restart opennetmon
```

### Port 443

Because of the hardening, the service may not open ports below 1024; startup then fails with `binding 0.0.0.0:443 (ports below 1024 need CAP_NET_BIND_SERVICE …)`.
If you want `https://<server>` without a port, allow exactly this one capability in a drop-in:

```bash
sudo systemctl edit opennetmon
```

```ini
[Service]
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
AmbientCapabilities=CAP_NET_BIND_SERVICE
```

Then set `listen = "0.0.0.0:443"` in the config, run `sudo systemctl restart opennetmon` and, if needed, `sudo ufw allow 443/tcp`.

### Converting existing installations

The installer and the package never overwrite a changed `config.toml`. If you installed before 0.3.0 and changed the config, you therefore keep `127.0.0.1:8080`
without HTTPS; on a package update `dpkg` asks, and the default is “keep your version” (the new one is then placed next to it as `config.toml.dpkg-dist`).
The certificate is created anyway. Switch HTTPS on with these three lines in `/etc/opennetmon/config.toml` (replace an existing `listen` line, remove
`client_ip_header` if there is no proxy) and `sudo systemctl restart opennetmon`:

```toml
listen = "0.0.0.0:8443"
tls_cert_file = "/etc/opennetmon/tls-cert.pem"
tls_key_file = "/etc/opennetmon/tls-key.pem"
```

### Alternatively: behind a reverse proxy

If you already run Caddy or nginx, you can put opennetmon behind it. Then set `listen = "127.0.0.1:8080"` in the config, remove the two `tls_` lines
and add `client_ip_header = "x-forwarded-for"` (details in `docs/users-and-login.md`); otherwise the login rate limiting counts all requests as one address.

- Caddy: `deploy/Caddyfile.example` (automatic HTTPS; `tls internal` for a private network).
- nginx: `deploy/nginx.conf.example` (TLS, HSTS and an additional **limit on login attempts**: 10 per minute and address).
- `cookie_secure = true` (the default) requires HTTPS. Only set it to `false` for a test without HTTPS.
- Open only 443 (and 80 for the redirect) to the outside, **not 8080**: `sudo ufw allow 443/tcp && sudo ufw allow 80/tcp`.

In general: the server should not be reachable from the internet unless necessary; for access on the road, a VPN into your own network is the better choice.

## Configuration

`/etc/opennetmon/config.toml`; all keys are explained in `deploy/config.example.toml`. After changes, `sudo systemctl restart opennetmon`.
Only **behind a reverse proxy**, additionally set `client_ip_header = "x-forwarded-for"` (see above).
The e-mail settings are **not** in the file but in the user interface (“E-mail”, see `docs/notifications.md`) and take effect immediately.

You also create the devices' credentials (SNMP profiles) in the user interface. For TP-Link Omada devices: SNMPv3, authNoPriv, MD5, user as in the
controller under *Network Config → General Settings → SNMP* (`docs/snmp-profiles.md`).

## Day to day

```bash
systemctl status opennetmon                 # is the service running?
journalctl -u opennetmon -f                 # live log (never passwords; status changes, errors)
journalctl -u opennetmon --since today -p warning
sudo systemctl restart opennetmon
curl -sk https://127.0.0.1:8443/healthz     # ok / unavailable (no internal details)
systemctl list-timers opennetmon-backup     # next backup
```

When stopping, the service first stops polling and then still sends pending alert mails (at most about 30 s, after that systemd ends it).

## Update and rollback

Update: build the new binary, copy it as above and run `sudo ./deploy/install.sh --binary ./opennetmon` again. The installer replaces the binary and the units,
**leaves configuration, key and database untouched** and restarts the running service; database migrations run automatically at startup.

Before an update with a schema change (a new migration under `migrations/`), a backup is worthwhile: `sudo systemctl start opennetmon-backup`.

Rollback: migrations only go forward. An old binary is therefore only safe if the update brought **no** new migration; otherwise restore the database from the
backup taken before (see below) and put the old binary back:

```bash
sudo systemctl stop opennetmon
sudo cp -p /usr/local/bin/opennetmon.previous /usr/local/bin/opennetmon
# after a new migration, also restore the database from the backup taken before the update
sudo systemctl start opennetmon
```

## Backup and restore

- **Automatic:** `opennetmon-backup.timer` writes a consistent copy every day (`opennetmon backup`, SQLite `VACUUM INTO`, mode 0600) to
  `/var/backups/opennetmon/opennetmon-YYYYMMDD-HHMMSS.db` and keeps the last 14. This works while the service is running. **These files are on the same server**
  – for real safety, copy them to another machine regularly (e.g. `rsync -a /var/backups/opennetmon/ backup-host:opennetmon/`).
- **Manual:** `sudo -u opennetmon /usr/local/bin/opennetmon --config /etc/opennetmon/config.toml backup /path/to/new-file.db` (never overwrites an existing file).
- **Key:** keep it separately (see above). With a database backup **and** the key, everything can be restored.
- **Restore:**

  ```bash
  sudo systemctl stop opennetmon
  sudo cp -p /var/backups/opennetmon/opennetmon-20261015-033000.db /var/lib/opennetmon/opennetmon.db
  sudo rm -f /var/lib/opennetmon/opennetmon.db-wal /var/lib/opennetmon/opennetmon.db-shm
  sudo chown opennetmon:opennetmon /var/lib/opennetmon/opennetmon.db && sudo chmod 600 /var/lib/opennetmon/opennetmon.db
  sudo systemctl start opennetmon
  ```

  Without the matching key, users, devices, sensors and values are kept, but the stored passwords are unreadable: profiles and the SMTP password have to be
  entered again. (Key rotation is not implemented yet.)

## Data volume

About 70 sensors with 100 channels at a 60-second interval produce 144,000 values per day; with 7 days of raw data retention that is about 1 million rows,
i.e. a few tens of megabytes. Hourly and daily values are negligibly small. The database does not grow beyond the configured retention
(`retention_raw_days`, `retention_hourly_days`).

## Ping needs a permission

Ping sensors use unprivileged ICMP sockets. For this, `net.ipv4.ping_group_range` must include the service's group; the installer sets `0 2147483647`.
Check: `sysctl net.ipv4.ping_group_range`. Without the permission, a warning appears in the log at startup and ping sensors report “no ICMP socket available”.
Alternative without sysctl: in `/etc/systemd/system/opennetmon.service`, replace the two lines `CapabilityBoundingSet=`/`AmbientCapabilities=` with `CAP_NET_RAW`
(the unit has a note on this), then `systemctl daemon-reload && systemctl restart opennetmon`.

## The unit at a glance (hardening)

The service runs as `opennetmon` without privileges (empty `CapabilityBoundingSet=`, `NoNewPrivileges`), with a read-only system (`ProtectSystem=strict`; only
`/var/lib/opennetmon` is writable), without home, with a private `/tmp` and no devices, only with IPv4/IPv6/Unix sockets, with a syscall filter (`@system-service`
without `@privileged @resources`) and `MemoryDenyWriteExecute`. `systemd-analyze security opennetmon.service` rates it “OK” (exposure 1.3). It was tested as a
transient systemd service with the same options (without `ProtectHome` and `PrivateTmp`, which would have hidden the test binary): SNMPv3, ping including name
resolution, discovery and the poller run without being blocked by the syscall filter.

## Troubleshooting

| Observation | Cause and remedy |
|---|---|
| Service does not start, log: `secret key file … must not be accessible by group or others` | The key file has wrong permissions: `sudo chmod 600 /etc/opennetmon/secret.key && sudo chown opennetmon:opennetmon /etc/opennetmon/secret.key` |
| Log: `cannot read … secret.key` | The file is missing or owned by root: `sudo chown opennetmon:opennetmon …`; if it is missing entirely, `opennetmon key generate` – then the old passwords are lost |
| Web interface: login works, but you end up on the login page again | `cookie_secure = true`, but opened over HTTP: open it over HTTPS |
| Browser: “connection is not secure” | Self-signed certificate: compare the fingerprint and confirm the exception, or install your own certificate (see “HTTPS”) |
| Browser: page not reachable | Open the port in the firewall (`sudo ufw allow 8443/tcp`), open the address with `https://` and `:8443`; `ss -ltnp \| grep 8443` on the server |
| Service does not start, log: `loading the HTTPS certificate …` | File missing, key does not match the certificate or has too open permissions (`chmod 600`, owner `opennetmon`); to generate a new one see “HTTPS” |
| Service does not start, log: `binding 0.0.0.0:443 …` | Port below 1024: see “Port 443” |
| Ping sensors report “no ICMP socket available” | see “Ping needs a permission” |
| All SNMP sensors of a device report “No answer within the timeout” | Device/port not reachable, SNMP switched off on the device, wrong profile; run the connection test on the device, `tcpdump -ni any udp port 161` |
| “Authentication failed” with SNMPv3 | User, password or method (MD5/SHA) do not match |
| No mail, the table “Recent deliveries” shows an error | Read the message there; send a test e-mail; check SMTP port/encryption (587 STARTTLS, 465 TLS) |
| “Down” mail although the device is reachable | The first failure is only a “Warning”, only the second in a row is “Down”; check interval/timeout of the device |
| `healthz` returns `unavailable` | Database not reachable: disk space (`df -h /var/lib/opennetmon`), permissions, log |

## Uninstalling

```bash
sudo systemctl disable --now opennetmon opennetmon-backup.timer
sudo rm /etc/systemd/system/opennetmon.service /etc/systemd/system/opennetmon-backup.* /usr/local/bin/opennetmon* /usr/local/sbin/opennetmon-backup /etc/sysctl.d/99-opennetmon-ping.conf
sudo rm -r /usr/local/share/doc/opennetmon
sudo systemctl daemon-reload
# Data and key are kept on purpose; to delete them for good:
# sudo rm -r /var/lib/opennetmon /etc/opennetmon /var/backups/opennetmon && sudo userdel opennetmon
```

## Checklist before production use

- [ ] `secret.key` backed up separately
- [ ] `https://<server>:8443` works, the certificate fingerprint was compared once (or your own certificate is installed)
- [ ] Port 8443 only reachable in the LAN, not from the internet (access from outside via VPN)
- [ ] Only behind a reverse proxy: `client_ip_header = "x-forwarded-for"` set and 8080 not reachable from outside
- [ ] SNMP passwords of the devices long and unique (Omada: only authNoPriv/MD5, therefore only in a private network)
- [ ] Backups copied to another machine regularly, a restore tried once
- [ ] Test e-mail received
