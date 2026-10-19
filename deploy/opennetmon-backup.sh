#!/bin/sh
# Daily database backup: writes a consistent copy and keeps the newest $KEEP copies.
# The copy holds the *encrypted* secrets; the key file (/etc/opennetmon/secret.key) is NOT included and
# must be backed up separately. Installed to /usr/local/sbin/opennetmon-backup, run by opennetmon-backup.timer.
set -eu

OPENNETMON="${OPENNETMON_BIN:-/usr/local/bin/opennetmon}"
CONFIG="${OPENNETMON_CONFIG:-/etc/opennetmon/config.toml}"
DIR="${BACKUP_DIR:-/var/backups/opennetmon}"
KEEP="${KEEP:-14}"

mkdir -p "$DIR"
"$OPENNETMON" --config "$CONFIG" backup "$DIR/opennetmon-$(date +%Y%m%d-%H%M%S).db" >/dev/null

# Prune: the file names sort chronologically, so everything but the newest $KEEP goes.
count=0
for file in $(ls -1r "$DIR"/opennetmon-*.db 2>/dev/null); do
  count=$((count + 1))
  if [ "$count" -gt "$KEEP" ]; then
    rm -f -- "$file"
  fi
done
