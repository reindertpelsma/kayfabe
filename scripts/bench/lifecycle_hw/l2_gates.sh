#!/bin/bash
# l2_gates.sh <worktree> <outlog>   -- v3_gates.sh under the shared GPU lock, own cargo target.
W=$1; OUT=$2
export CARGO_TARGET_DIR=/data/claude-life/target CARGO_BUILD_JOBS=12
cd "$W" || exit 3
echo "GATES_RUN_START rev=$(git rev-parse --short=8 HEAD) $(date -Is)" > "$OUT.run"
flock -o /tmp/kayfabe-fastguest.lock bash scripts/bench/v3_gates.sh "$OUT" > /dev/null 2>&1
rc=$?
echo "GATES_RUN_EXIT rc=$rc $(date -Is)" >> "$OUT.run"
grep -a 'V3_GATES_SUMMARY\|V3_GATES_BIRTHS\|V3_GATES_EXIT' "$OUT" >> "$OUT.run"
cat "$OUT.run"
