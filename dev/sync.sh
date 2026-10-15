#!/usr/bin/env bash
# Bidirectional sync of CLAUDE.md and PLAN.md with the Google Drive folder (see dev/sync-filter.txt).
# Usage: dev/sync.sh [--resync]   (--resync only for the first run or after errors)
set -euo pipefail
cd "$(dirname "$0")/.."
exec rclone bisync . gdrive: \
  --filters-file dev/sync-filter.txt \
  --resilient --recover \
  --conflict-resolve newer --conflict-loser num \
  "$@"
