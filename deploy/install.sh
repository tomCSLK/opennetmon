#!/usr/bin/env bash
# Installs or upgrades opennetmon on a systemd-based Ubuntu/Debian server. Run as root:
#
#   sudo deploy/install.sh [--binary PATH] [--no-sysctl] [--no-backup]
#
# First run: creates the service user, /etc/opennetmon (config + encryption key) and the systemd unit.
# Re-running replaces the binary and unit, never touches an existing config, key or database,
# and restarts the service if it was running (schema migrations run on start).
#
# For a dry run into a directory instead of the real system:  DESTDIR=/tmp/root deploy/install.sh
# (no user, no chown, no systemctl, no sysctl).
set -euo pipefail

cd "$(dirname "$0")/.."
DESTDIR="${DESTDIR:-}"
BINARY="target/x86_64-unknown-linux-musl/release/opennetmon"
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

USER_NAME=opennetmon
BIN_DIR="$DESTDIR/usr/local/bin"
CONF_DIR="$DESTDIR/etc/opennetmon"
UNIT_DIR="$DESTDIR/etc/systemd/system"
SYSCTL_DIR="$DESTDIR/etc/sysctl.d"
SBIN_DIR="$DESTDIR/usr/local/sbin"
DOC_DIR="$DESTDIR/usr/local/share/doc/opennetmon"
STATE_DIR="$DESTDIR/var/lib/opennetmon"
BACKUP_DIR="$DESTDIR/var/backups/opennetmon"
CONFIG="$CONF_DIR/config.toml"
KEY="$CONF_DIR/secret.key"
TLS_CERT="$CONF_DIR/tls-cert.pem"
TLS_KEY="$CONF_DIR/tls-key.pem"
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
step "installing the binary to $BIN_DIR/opennetmon"
install -d -m 0755 "$BIN_DIR"
# Keep the previous binary for a quick rollback (see docs/operations.md).
[ -e "$BIN_DIR/opennetmon" ] && cp -p "$BIN_DIR/opennetmon" "$BIN_DIR/opennetmon.previous"
install -m 0755 "$BINARY" "$BIN_DIR/opennetmon.new"
mv -f "$BIN_DIR/opennetmon.new" "$BIN_DIR/opennetmon"
# License of opennetmon and of everything compiled into it.
install -d -m 0755 "$DOC_DIR"
install -m 0644 LICENSE THIRD-PARTY-LICENSES.txt "$DOC_DIR/"

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
  OPENNETMON_SECRET_KEY_FILE="$KEY" "$BIN_DIR/opennetmon" --config "$CONFIG" key generate >/dev/null
  chmod 0600 "$KEY"
  [ "$REAL" = yes ] && chown "$USER_NAME":"$USER_NAME" "$KEY"
  KEY_IS_NEW=yes
else
  step "keeping the existing key $KEY"
  KEY_IS_NEW=no
fi
# Self-signed HTTPS certificate for the host name and every address of the server (replaceable, see docs/operations.md).
if [ ! -e "$TLS_CERT" ] && [ ! -e "$TLS_KEY" ]; then
  step "generating the HTTPS certificate $TLS_CERT"
  NAMES=()
  for name in $(hostname -f 2>/dev/null) $(hostname -I 2>/dev/null); do
    NAMES+=(--name "$name")
  done
  "$BIN_DIR/opennetmon" --config "$CONFIG" tls generate --cert "$TLS_CERT" --key "$TLS_KEY" "${NAMES[@]}" | sed 's/^/    /'
  [ "$REAL" = yes ] && chown "$USER_NAME":"$USER_NAME" "$TLS_KEY"
else
  step "keeping the existing certificate $TLS_CERT"
fi

# Database directory: systemd creates it on the first start, but `user add` (next steps) runs before that.
if [ ! -d "$STATE_DIR" ]; then
  install -d -m 0750 "$STATE_DIR"
  [ "$REAL" = yes ] && chown "$USER_NAME":"$USER_NAME" "$STATE_DIR"
fi

# 4. systemd unit and the ping sysctl.
step "installing the systemd unit"
install -d -m 0755 "$UNIT_DIR"
install -m 0644 deploy/opennetmon.service "$UNIT_DIR/opennetmon.service"
if [ "$SYSCTL" = yes ]; then
  step "allowing unprivileged ICMP sockets for ping sensors"
  install -d -m 0755 "$SYSCTL_DIR"
  install -m 0644 deploy/99-opennetmon-ping.conf "$SYSCTL_DIR/99-opennetmon-ping.conf"
  [ "$REAL" = yes ] && sysctl -q -p "$SYSCTL_DIR/99-opennetmon-ping.conf"
fi

# 5. Daily database backup (without the key file).
if [ "$BACKUP" = yes ]; then
  step "installing the daily backup (opennetmon-backup.timer, 14 copies in /var/backups/opennetmon)"
  install -d -m 0755 "$SBIN_DIR"
  install -m 0755 deploy/opennetmon-backup.sh "$SBIN_DIR/opennetmon-backup"
  install -d -m 0700 "$BACKUP_DIR"
  [ "$REAL" = yes ] && chown "$USER_NAME":"$USER_NAME" "$BACKUP_DIR"
  install -m 0644 deploy/opennetmon-backup.service deploy/opennetmon-backup.timer "$UNIT_DIR/"
fi

if [ "$REAL" = yes ]; then
  systemctl daemon-reload
  systemctl enable opennetmon.service >/dev/null 2>&1
  [ "$BACKUP" = yes ] && systemctl enable --now opennetmon-backup.timer >/dev/null 2>&1
  if systemctl is-active --quiet opennetmon.service; then
    step "restarting the running service"
    systemctl restart opennetmon.service
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
  if ! systemctl is-active --quiet opennetmon.service; then
    echo
    echo "Next steps:"
    echo "  1. Create the first administrator:"
    echo "       sudo -u $USER_NAME $BIN_DIR/opennetmon --config $CONFIG user add admin --role admin"
    echo "  2. Start the service:   sudo systemctl start opennetmon"
    echo "  3. Open https://<address of this server>:8443 (allow port 8443 in the firewall if one is active)."
    echo "     The certificate is self-signed: compare the fingerprint above before accepting it."
    echo "  4. Check:               systemctl status opennetmon; journalctl -u opennetmon -e; curl -sk https://127.0.0.1:8443/healthz"
  fi
else
  echo "(dry run into $DESTDIR: no user, ownership, systemctl or sysctl changes)"
fi
