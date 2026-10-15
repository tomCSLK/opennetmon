#!/bin/sh
# Daily database backup: writes a consistent copy and keeps the newest $KEEP copies.
# The copy holds the *encrypted* secrets; the key file (/etc/netmon/secret.key) is NOT included and
# must be backed up separately. Installed to /usr/local/sbin/netmon-backup, run by netmon-backup.timer.
set -eu

NETMON="${NETMON_BIN:-/usr/local/bin/netmon}"
CONFIG="${NETMON_CONFIG:-/etc/netmon/config.toml}"
DIR="${BACKUP_DIR:-/var/backups/netmon}"
KEEP="${KEEP:-14}"

mkdir -p "$DIR"
"$NETMON" --config "$CONFIG" backup "$DIR/netmon-$(date +%Y%m%d-%H%M%S).db" >/dev/null

# Prune: the file names sort chronologically, so everything but the newest $KEEP goes.
count=0
for file in $(ls -1r "$DIR"/netmon-*.db 2>/dev/null); do
  count=$((count + 1))
  if [ "$count" -gt "$KEEP" ]; then
    rm -f -- "$file"
  fi
done
