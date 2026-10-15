#!/usr/bin/env bash
# Installs or upgrades netmon on a systemd-based Ubuntu/Debian server. Run as root:
#
#   sudo deploy/install.sh [--binary PATH] [--no-sysctl] [--no-backup]
#
# First run: creates the service user, /etc/netmon (config + encryption key) and the systemd unit.
# Re-running replaces the binary and unit, never touches an existing config, key or database,
# and restarts the service if it was running (schema migrations run on start).
#
# For a dry run into a directory instead of the real system:  DESTDIR=/tmp/root deploy/install.sh
# (no user, no chown, no systemctl, no sysctl).
set -euo pipefail

cd "$(dirname "$0")/.."
DESTDIR="${DESTDIR:-}"
BINARY="target/x86_64-unknown-linux-musl/release/netmon"
SYSCTL=yes
BACKUP=yes
while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BINARY="${2:?--binary needs a path}"; shift 2 ;;
    --no-sysctl) SYSCTL=no; shift ;;
    --no-backup) BACKUP=no; shift ;;
    -h|--help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

USER_NAME=netmon
BIN_DIR="$DESTDIR/usr/local/bin"
CONF_DIR="$DESTDIR/etc/netmon"
UNIT_DIR="$DESTDIR/etc/systemd/system"
SYSCTL_DIR="$DESTDIR/etc/sysctl.d"
SBIN_DIR="$DESTDIR/usr/local/sbin"
BACKUP_DIR="$DESTDIR/var/backups/netmon"
CONFIG="$CONF_DIR/config.toml"
KEY="$CONF_DIR/secret.key"
REAL=yes
[ -n "$DESTDIR" ] && REAL=no

if [ "$REAL" = yes ] && [ "$(id -u)" -ne 0 ]; then
  echo "Please run as root (sudo)." >&2
  exit 1
fi
if [ ! -x "$BINARY" ]; then
  echo "Binary not found: $BINARY" >&2
  echo "Build it first:  cargo build --release --target x86_64-unknown-linux-musl" >&2
  exit 1
fi
if ! "$BINARY" --version >/dev/null 2>&1; then
  echo "$BINARY does not run on this machine (wrong architecture?)." >&2
  exit 1
fi

step() { printf '==> %s\n' "$*"; }

# 1. Service user (no login, no home).
if [ "$REAL" = yes ] && ! id "$USER_NAME" >/dev/null 2>&1; then
  step "creating system user $USER_NAME"
  useradd --system --user-group --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin "$USER_NAME"
fi

# 2. Binary.
step "installing the binary to $BIN_DIR/netmon"
install -d -m 0755 "$BIN_DIR"
# Keep the previous binary for a quick rollback (see docs/betrieb.md).
[ -e "$BIN_DIR/netmon" ] && cp -p "$BIN_DIR/netmon" "$BIN_DIR/netmon.previous"
install -m 0755 "$BINARY" "$BIN_DIR/netmon.new"
mv -f "$BIN_DIR/netmon.new" "$BIN_DIR/netmon"

# 3. Configuration directory, config (kept if present) and encryption key (kept if present).
install -d -m 0750 "$CONF_DIR"
[ "$REAL" = yes ] && chown root:"$USER_NAME" "$CONF_DIR"
if [ ! -e "$CONFIG" ]; then
  step "writing $CONFIG"
  install -m 0640 deploy/config.example.toml "$CONFIG"
  [ "$REAL" = yes ] && chown root:"$USER_NAME" "$CONFIG"
else
  step "keeping the existing $CONFIG"
fi
if [ ! -e "$KEY" ]; then
  step "generating the encryption key $KEY"
  # The key path in the config is absolute; with DESTDIR, generate into the prefixed path instead.
  NETMON_SECRET_KEY_FILE="$KEY" "$BIN_DIR/netmon" --config "$CONFIG" key generate >/dev/null
  chmod 0600 "$KEY"
  [ "$REAL" = yes ] && chown "$USER_NAME":"$USER_NAME" "$KEY"
  KEY_IS_NEW=yes
else
  step "keeping the existing key $KEY"
  KEY_IS_NEW=no
fi

# 4. systemd unit and the ping sysctl.
step "installing the systemd unit"
install -d -m 0755 "$UNIT_DIR"
install -m 0644 deploy/netmon.service "$UNIT_DIR/netmon.service"
if [ "$SYSCTL" = yes ]; then
  step "allowing unprivileged ICMP sockets for ping sensors"
  install -d -m 0755 "$SYSCTL_DIR"
  install -m 0644 deploy/99-netmon-ping.conf "$SYSCTL_DIR/99-netmon-ping.conf"
  [ "$REAL" = yes ] && sysctl -q -p "$SYSCTL_DIR/99-netmon-ping.conf"
fi

# 5. Daily database backup (without the key file).
if [ "$BACKUP" = yes ]; then
  step "installing the daily backup (netmon-backup.timer, 14 copies in /var/backups/netmon)"
  install -d -m 0755 "$SBIN_DIR"
  install -m 0755 deploy/netmon-backup.sh "$SBIN_DIR/netmon-backup"
  install -d -m 0700 "$BACKUP_DIR"
  [ "$REAL" = yes ] && chown "$USER_NAME":"$USER_NAME" "$BACKUP_DIR"
  install -m 0644 deploy/netmon-backup.service deploy/netmon-backup.timer "$UNIT_DIR/"
fi

if [ "$REAL" = yes ]; then
  systemctl daemon-reload
  systemctl enable netmon.service >/dev/null 2>&1
  [ "$BACKUP" = yes ] && systemctl enable --now netmon-backup.timer >/dev/null 2>&1
  if systemctl is-active --quiet netmon.service; then
    step "restarting the running service"
    systemctl restart netmon.service
  fi
fi

echo
echo "Done."
if [ "$REAL" = yes ]; then
  if [ "$KEY_IS_NEW" = yes ]; then
    echo
    echo "IMPORTANT: back up $KEY separately from the database. Without it the stored"
    echo "SNMP/SMTP passwords cannot be decrypted."
  fi
  if ! systemctl is-active --quiet netmon.service; then
    echo
    echo "Next steps:"
    echo "  1. Create the first administrator:"
    echo "       sudo -u $USER_NAME $BIN_DIR/netmon --config $CONFIG user add admin --role admin"
    echo "  2. Start the service:   sudo systemctl start netmon"
    echo "  3. Put a TLS reverse proxy in front (deploy/Caddyfile.example, deploy/nginx.conf.example)."
    echo "  4. Check:               systemctl status netmon; journalctl -u netmon -e; curl -s http://127.0.0.1:8080/healthz"
  fi
else
  echo "(dry run into $DESTDIR: no user, ownership, systemctl or sysctl changes)"
fi
