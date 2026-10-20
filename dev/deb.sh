#!/usr/bin/env bash
# Builds the Debian package target/debian/opennetmon_<version>-1_amd64.deb
# from the static binary. Extra arguments go to cargo-deb (e.g. --no-build after a release build).
#
# The package mirrors deploy/install.sh, but installs to /usr instead of /usr/local; the units and the
# backup script are adapted from deploy/ into target/deb-assets/. Needs cargo-deb (cargo install cargo-deb --locked).
set -euo pipefail

cd "$(dirname "$0")/.."
command -v cargo-deb >/dev/null || { echo "cargo-deb is missing: cargo install cargo-deb --locked" >&2; exit 1; }

GEN=target/deb-assets
rm -rf "$GEN" && mkdir -p "$GEN"
for f in opennetmon.service opennetmon-backup.service; do
  sed -e 's|/usr/local/bin/|/usr/bin/|g' -e 's|/usr/local/sbin/|/usr/sbin/|g' "deploy/$f" > "$GEN/$f"
done
sed -e 's|/usr/local/bin/|/usr/bin/|g' -e 's|/usr/local/sbin/|/usr/sbin/|g' deploy/opennetmon-backup.sh > "$GEN/opennetmon-backup"
if grep -n /usr/local "$GEN"/*; then
  echo "error: paths under /usr/local left in the package files above" >&2
  exit 1
fi

cargo deb --target x86_64-unknown-linux-musl "$@"
