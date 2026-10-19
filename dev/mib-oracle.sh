#!/usr/bin/env bash
# Compares opennetmon's MIB resolution with net-snmp's snmptranslate (needs the `snmp` package).
# Usage: dev/mib-oracle.sh [vendor MIB files ...]    e.g. dev/mib-oracle.sh dev/data/mibs/tplink/*.mib
# Prints names known to both with different OIDs (should be none) and what only one side knows.
set -euo pipefail
cd "$(dirname "$0")/.."
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/std" "$work/vendor"
cp mibs/*.txt "$work/std/"
for f in "$@"; do cp "$f" "$work/vendor/"; done
# Bundled modules win in opennetmon; give net-snmp the same picture.
for f in "$work"/vendor/*; do
  [ -e "$f" ] || continue
  mod=$(grep -m1 -oE '^[[:space:]]*[A-Za-z0-9-]+[[:space:]]+DEFINITIONS' "$f" | awk '{print $1}')
  [ -n "$mod" ] && [ -e "$work/std/$mod.txt" ] && rm "$f"
done
cargo run -q --example mib_dump -- "$@" > "$work/opennetmon.tsv" 2> "$work/opennetmon.err"
snmptranslate -M "$work/std:$work/vendor" -m ALL -Tz 2> "$work/netsnmp.err" | tr -d '"' | awk '{print $1"\t"$2}' > "$work/netsnmp.tsv"
python3 -I - "$work" <<'PY'
import collections, sys
w = sys.argv[1]
def load(p):
    d = collections.defaultdict(set)
    for line in open(p):
        if '\t' in line:
            n, o = line.rstrip('\n').split('\t')[:2]
            d[n].add(o.strip('.'))
    return d
ours, theirs = load(w + '/opennetmon.tsv'), load(w + '/netsnmp.tsv')
common = set(ours) & set(theirs)
bad = [(n, sorted(ours[n]), sorted(theirs[n])) for n in sorted(common) if not ours[n] & theirs[n]]
print(f"names: opennetmon {len(ours)}, net-snmp {len(theirs)}, both {len(common)}")
print(f"different OIDs: {len(bad)}")
for b in bad[:20]:
    print("  ", b)
only_ours, only_theirs = sorted(set(ours) - set(theirs)), sorted(set(theirs) - set(ours))
print(f"only opennetmon: {len(only_ours)} {only_ours[:10]}")
print(f"only net-snmp: {len(only_theirs)} {only_theirs[:10]}")
sys.exit(1 if bad else 0)
PY
echo "opennetmon problems:"; cat "$work/opennetmon.err"
