# Running in a container

opennetmon is also available as a container image for Docker and Podman, next to the `.deb` package ([`operations.md`](operations.md)).
It runs on any Linux host with amd64 or arm64 (Raspberry Pi 4/5, many NAS, ARM servers) and under Docker Desktop.

| | |
|---|---|
| Image | `ghcr.io/tomcslk/opennetmon` (GitHub Container Registry), since 0.7.0 |
| Tags | `X.Y.Z` (one release), `X.Y` (newest patch release of that line), `latest` (newest release) |
| Architectures | `linux/amd64`, `linux/arm64` |
| Content | The static binary, `LICENSE` and `THIRD-PARTY-LICENSES.txt` in `/usr/share/doc/opennetmon/`. Nothing else: no shell, no package manager, about 26 MB |
| User | UID/GID `10001`, never root |
| Port | `8443`, HTTPS |
| Volumes | `/etc/opennetmon` (key, certificate), `/var/lib/opennetmon` (database, daily backups) |

The image is built from [`Containerfile`](../Containerfile) by the workflow `.github/workflows/container.yml` when a release is tagged; every
architecture is built on its own hardware and smoke-tested before it is pushed.

## Quick start

Take [`compose.yaml`](../compose.yaml) from the repository (or from the release), put it into an empty folder and run there:

```bash
docker compose up -d
docker compose exec opennetmon opennetmon user add admin --role admin     # asks for the password twice
docker compose logs opennetmon | grep fingerprint
```

Then open `https://<address of this computer>:8443`. On the first start the server creates the encryption key and a self-signed certificate in
the config volume; the browser warns once. Compare the fingerprint in the warning with the one from the log before you accept it.
There is no default user: until you create one, nobody can sign in.

Without Compose, the same as one command:

```bash
docker run -d --name opennetmon --restart unless-stopped -p 8443:8443 \
  -v opennetmon-config:/etc/opennetmon -v opennetmon-data:/var/lib/opennetmon \
  -e TZ=Europe/Berlin --sysctl net.ipv4.ping_group_range="0 65535" \
  --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true \
  ghcr.io/tomcslk/opennetmon:latest
docker exec -it opennetmon opennetmon user add admin --role admin
```

The other subcommands work the same way: `opennetmon user list`, `user passwd <name>`, `backup <file>`, `healthcheck`
(`docker compose exec opennetmon opennetmon …`). They read the same settings as the server.

## Settings

The image sets everything through variables ([`configuration.md`](configuration.md) lists all of them); add or override them under
`environment:` in `compose.yaml`. What the image sets:

| Variable | Value in the image |
|---|---|
| `OPENNETMON_LISTEN` | `0.0.0.0:8443` |
| `OPENNETMON_DATABASE_PATH` | `/var/lib/opennetmon/opennetmon.db` |
| `OPENNETMON_SECRET_KEY_FILE` | `/etc/opennetmon/secret.key` |
| `OPENNETMON_TLS_CERT_FILE`, `OPENNETMON_TLS_KEY_FILE` | `/etc/opennetmon/tls-cert.pem`, `/etc/opennetmon/tls-key.pem` |
| `OPENNETMON_GENERATE_MISSING` | `true` (key and certificate on the first start) |
| `OPENNETMON_BACKUP_DIR` | `/var/lib/opennetmon/backups` (daily backup at 03:30, 14 kept) |
| `OPENNETMON_UPDATE_CHECK` | `true` (asks GitHub once a day for a newer release) |
| `OPENNETMON_PACKAGING` | `container` (the update notice names `docker compose pull`) |

**Time zone:** `TZ` (e.g. `Europe/Berlin`) sets the zone of the maintenance windows and of the backup time; without it the container runs in UTC.
The image carries its own time zone database.

A config file still works: mount it and name it in `OPENNETMON_CONFIG`; variables win over it.

## HTTPS and certificates

The generated certificate is valid for ten years and names the container's host name, `localhost` and the loopback addresses, not the address
you type, so browsers keep warning until you accept it once. To use your own certificate (from your own small CA or Let's Encrypt,
see [`operations.md`](operations.md#using-your-own-certificate)), mount it and point the variables to it:

```yaml
    volumes:
      - ./certs:/certs:ro
    environment:
      OPENNETMON_TLS_CERT_FILE: /certs/fullchain.pem
      OPENNETMON_TLS_KEY_FILE: /certs/privkey.pem
```

The key must belong to UID 10001 and have mode 0600, otherwise the server does not start: `sudo chown 10001:10001 certs/privkey.pem && chmod 600 certs/privkey.pem`
(rootless Podman: `podman unshare chown 10001:10001 certs/privkey.pem`). Restart with `docker compose up -d`.

To get a new self-signed certificate with other names, remove the two files and let the server create them again, or create them yourself:

```bash
docker compose run --rm --no-deps opennetmon tls generate --name 192.168.1.10 --name opennetmon.lan
```

(after removing `tls-cert.pem` and `tls-key.pem` from the config volume; the command never overwrites).

### Behind a reverse proxy

If a reverse proxy (Traefik, Caddy, nginx) terminates HTTPS, switch the built-in HTTPS off with **empty** variables and let opennetmon speak
plain HTTP inside the container network:

```yaml
    environment:
      OPENNETMON_TLS_CERT_FILE: ""
      OPENNETMON_TLS_KEY_FILE: ""
      OPENNETMON_LISTEN: 0.0.0.0:8080
      OPENNETMON_CLIENT_IP_HEADER: x-forwarded-for
    # no "ports:" - only the proxy reaches the container
```

Keep `OPENNETMON_COOKIE_SECURE` at its default (`true`): the browser talks HTTPS to the proxy. The log warns about plain HTTP on a non-loopback
address; that is expected here. Set `OPENNETMON_CLIENT_IP_HEADER` only when nothing but the proxy can reach port 8080, because the sign-in
throttling trusts that header. The health check of the image follows the setting and asks over HTTP.

## Network and ping

By default the container sits in Docker's bridge network: SNMP, ping and the service checks reach your devices through the host (NAT), and the
devices see the address of the host. If a device only answers certain addresses, allow the host's.

**Ping** uses unprivileged ICMP sockets, no capability. Docker allows them in the container network already; Podman does not, so
`compose.yaml` sets `net.ipv4.ping_group_range` to `0 65535` (rootless Podman refuses larger ranges than its user namespace maps). Without it,
ping sensors fail with "permission denied" and the log warns at startup.

**Host network:** with `network_mode: host` (and without `ports:` and `sysctls:`) the container uses the host's network directly; the web
interface then listens on port 8443 of the host, and ping needs the host's `net.ipv4.ping_group_range` to include GID 10001 (the `.deb` sets
`0 2147483647`; on other hosts `sudo sysctl -w net.ipv4.ping_group_range="0 2147483647"` and a file in `/etc/sysctl.d/`).

## "This server" sensors

The sensors of the type "This server" read `/proc` and `/sys`. In a container:

- **CPU, load, memory, uptime and temperatures** show the **host** (the kernel does not separate them), except under Docker Desktop on Windows
  and macOS, where the host is Docker's Linux VM.
- **File systems:** none are offered; the volumes are parts of the host's file systems.
- **Network interfaces:** only the container's own; with `network_mode: host` the host's.

For the disks and interfaces of the host, monitor it over SNMP instead (`snmpd` on the host, see [`dev-testagent.md`](dev-testagent.md)).

## Backups

The server writes a consistent copy of the database every day at 03:30 (local time, `TZ`) to `backups/` in the data volume and keeps 14
([`configuration.md`](configuration.md#backups)); a backup missed while the container was down follows five minutes after the start. Change it
with `OPENNETMON_BACKUP_TIME` and `OPENNETMON_BACKUP_KEEP`.

The backups stay in the same volume as the database, so **copy them off the host** as well, and the config volume with the key **separately**:
without `secret.key`, the stored SNMP and SMTP passwords of every backup are unreadable. For example (the volume names start with the folder name
of `compose.yaml`; `docker volume ls` shows them):

```bash
docker run --rm -v opennetmon_data:/data:ro -v "$PWD":/out alpine \
  tar -czf /out/opennetmon-backups.tar.gz -C /data backups
docker run --rm -v opennetmon_config:/config:ro -v "$PWD":/out alpine \
  cp /config/secret.key /out/opennetmon-secret.key
```

A backup by hand at any time: `docker compose exec opennetmon opennetmon backup /var/lib/opennetmon/backups/before-update.db`
(never overwrites an existing file).

**Restore:** stop the container (`docker compose stop`), replace `opennetmon.db` in the data volume with the backup and delete
`opennetmon.db-wal` and `opennetmon.db-shm` next to it, keep the key that belongs to it in the config volume, then `docker compose up -d`.

## Updates

```bash
docker compose exec opennetmon opennetmon backup /var/lib/opennetmon/backups/before-<new version>.db
docker compose pull
docker compose up -d
```

Settings → Updates in the web interface shows these steps with the version filled in, as soon as the daily check has found a newer release.
New database migrations run on the first start of the new image. With `image: ghcr.io/tomcslk/opennetmon:latest` the pull fetches the newest
release; with a pinned tag (`:0.7.0`), change the tag first.

**Going back** to an older image works only as long as the newer one brought no migration; otherwise restore the backup made before the update
(release notes say whether a release changes the database).

## Health check

The image runs `opennetmon healthcheck` every 30 seconds; `docker compose ps` and `docker inspect` show `healthy` or `unhealthy`. The command
asks `/healthz` of the server itself ([`configuration.md`](configuration.md#health-check)). Other monitoring can call
`https://<host>:8443/healthz` directly.

## Security

`compose.yaml` starts the container read-only (`read_only`, writable are only the two volumes and a `tmpfs` at `/tmp`), without any
capability (`cap_drop: ALL`) and with `no-new-privileges`; the process runs as UID 10001. The image contains no shell, so a compromised
process finds no tools. As with the `.deb`, put the server into your own network only and reach it from outside through a VPN.

The image is AGPL-3.0-or-later like the source; the user menu and the sign-in page link the source code of the running version.

## Podman

Everything above works with Podman: `podman compose` (with `podman-compose` or `docker-compose` installed) or the `podman run` line with the
same options. Rootless Podman needs no changes; ports below 1024 are not allowed there, 8443 is fine. The health check needs an image in Docker
format, which the published image is (Podman's own builds need `--format docker`, as `dev/container.sh` does).

## Building the image yourself

```bash
dev/container.sh            # static binary for this machine's architecture, then the image opennetmon:<version>
dev/container.sh --test     # plus the smoke test: start read-only, health check, create an admin, sign in, restart
```

It uses Podman if present, else Docker, and builds from a prepared context in `target/container/` without `RUN` steps.
