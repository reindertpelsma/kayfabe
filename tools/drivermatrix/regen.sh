#!/usr/bin/env bash
# ★ Regenerate the driver matrix: MEASURE every tag, collapse to version runs, emit Rust.
#
#   tools/drivermatrix/regen.sh                 # the committed tag set (tags.txt)
#   tools/drivermatrix/regen.sh 580.82.07 ...   # ADD tags to tags.txt, then regenerate
#
# env: DM_WORK (default /var/tmp/kf-drivermatrix — ~110 MB per tag while it is measured, deleted
#      after), DM_JOBS (parallel tags, default 3). Needs git, gcc, python3 with `pyelftools`.
#
# Outputs (all committed — the diff is the review):
#   traces/driver_matrix/ranges.tsv        consumed items, as runs of identical measured tags
#   traces/driver_matrix/boundaries.txt    every adjacent-tag transition: how much moved
#   traces/driver_matrix/report.md         per consumed struct: which fields appear/vanish/move
#   crates/kf-abi/src/generated/matrix.rs  the ranges as Rust data (`kf_abi::matrix` reads it)
#
# ⊘ Evidence discipline (tools/drivermatrix/dm.py): every cell is measured or MISSING; nothing
# is interpolated between tags and nothing is typed by hand. A tag that is not in tags.txt is
# UNMEASURED and kf-abi refuses it by name.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
WORK=${DM_WORK:-/var/tmp/kf-drivermatrix}
JOBS=${DM_JOBS:-3}
OUT="$REPO/traces/driver_matrix"
mkdir -p "$WORK" "$OUT"
python3 -c "import elftools" 2>/dev/null || { echo "⊘ pip install pyelftools (the DWARF reader)"; exit 2; }
for t in "$@"; do grep -qx "$t" "$HERE/tags.txt" || echo "$t" >> "$HERE/tags.txt"; done
TAGS=$(grep -v '^#' "$HERE/tags.txt" | grep . | sort -V | tr '\n' ' ')
echo "== measuring $(echo $TAGS | wc -w) tag(s) into $WORK/sweep"
python3 "$HERE/dm.py" sweep --tags "$TAGS" --spec "$HERE/gsp.spec" --spec "$HERE/sdk.spec" \
    --spec "$HERE/os.spec" --spec "$HERE/host.spec" --work "$WORK/src" --out "$WORK/sweep" --jobs "$JOBS"
# ⊘ Only the measured tags of THIS tag list — a stale directory from an earlier list must not
# widen the table.
mkdir -p "$WORK/sel"; rm -rf "$WORK/sel"/*
for t in $TAGS; do ln -s "$WORK/sweep/$t" "$WORK/sel/$t"; done
python3 "$HERE/collapse.py" ranges --sweep "$WORK/sel" --only "$HERE/consumed.txt" --out "$OUT/ranges.tsv"
python3 "$HERE/collapse.py" report --sweep "$WORK/sel" --only "$HERE/consumed.txt" --out "$OUT/report.md"
python3 "$HERE/collapse.py" boundaries --sweep "$WORK/sel" > "$OUT/boundaries.txt"
python3 "$HERE/emit_rust.py" --ranges "$OUT/ranges.tsv" --out "$REPO/crates/kf-abi/src/generated/matrix.rs"
echo "== regenerated: $(grep -vc '^#' "$OUT/ranges.tsv") runs over $(echo $TAGS | wc -w) tags"
