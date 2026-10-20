#!/usr/bin/env bash
# ONE-OFF grader-validity check (docs/design/V3_RELAY_GRADER_20261020.md, section "Validity"): run the grader's lineage-neutral
# tests against a revision (a known-bad control, or any head) and list which go red. GPU-free.
#
#   scripts/relay_validity.sh <rev> [<rev> ...]
#
# For each rev: a detached scratch worktree, the grader files of THIS checkout overlaid (grade_adapter/model/locks/static.rs,
# scripts/relay_grade/*), the lineage detected from the source (simple / look_every / budget) and passed as
# --cfg kf_grade_lineage="..."; then `cargo test --release -p kf-chan --test grade_model --test grade_locks --test grade_static`.
# grade_stress.rs needs the simple lineage's harness hooks (chaos-hooks, Inflight, set_park_net, user_loop::look): on the other
# lineages it is NOT APPLICABLE (listed, not counted as a pass). Results: <out>/<rev>.tests (one "name ok|FAILED" per line).
# Env: KF_VALIDITY_DIR (scratch, default /workspace/kf-grade-ctl), CARGO_TARGET_DIR (default /workspace/kf-grade-ctl-target),
# KF_VALIDITY_KEEP=1 keeps the worktrees.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
scratch="${KF_VALIDITY_DIR:-/workspace/kf-grade-ctl}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/workspace/kf-grade-ctl-target}"
mkdir -p "$scratch"
avail_kb=$(df -Pk "$scratch" | awk 'NR==2{print $4}')
[ "$avail_kb" -gt 4000000 ] || { echo "less than 4 GB free in $scratch" >&2; exit 2; }
files=(crates/kf-chan/tests/grade_adapter.rs crates/kf-chan/tests/grade_model.rs crates/kf-chan/tests/grade_locks.rs
       crates/kf-chan/tests/grade_static.rs)
for rev in "$@"; do
  short=$(git -C "$here" rev-parse --short=9 "$rev")
  wt="$scratch/$short"
  [ -d "$wt" ] || git -C "$here" worktree add --detach -q "$wt" "$short"
  for f in "${files[@]}"; do cp "$here/$f" "$wt/$f"; done
  mkdir -p "$wt/scripts/relay_grade" && cp "$here"/scripts/relay_grade/* "$wt/scripts/relay_grade/"
  if grep -q 'pub const fn look(' "$wt/crates/kf-chan/src/user_loop.rs"; then lineage=simple
  elif grep -q 'LOOK_EVERY' "$wt/crates/kf-chan/src/worker.rs"; then lineage=look_every
  else lineage=budget; fi
  tests=(--test grade_model --test grade_locks --test grade_static)
  na=""
  if [ "$lineage" = simple ]; then cp "$here/crates/kf-chan/tests/grade_stress.rs" "$wt/crates/kf-chan/tests/"; tests+=(--test grade_stress)
  else na="grade_stress (needs chaos-hooks / Inflight / set_park_net / user_loop::look)"; fi
  echo "== $short lineage=$lineage${na:+ NOT APPLICABLE: $na}"
  log="$scratch/$short.log"
  ( cd "$wt" && RUSTFLAGS="--cfg kf_grade_lineage=\"$lineage\"" nice -n 19 cargo test --release -p kf-chan -j2 "${tests[@]}" \
      --no-fail-fast -- --test-threads=2 ) > "$log" 2>&1 || true
  grep -E '^test .* \.\.\. (ok|FAILED|ignored)' "$log" | sed -E 's/^test (.*) \.\.\. (ok|FAILED|ignored).*/\1 \2/' | sort > "$scratch/$short.tests" || true
  if grep -E '^error(\[E[0-9]+\])?: ' "$log" | grep -qvE 'test failed|targets? failed'; then
    echo "   COMPILE ERRORS (see $log):"; grep -E '^error' "$log" | grep -vE 'test failed|targets? failed' | sort | uniq -c | head -5; fi
  echo "   red: $(grep -c ' FAILED$' "$scratch/$short.tests" || true)  green: $(grep -c ' ok$' "$scratch/$short.tests" || true)"
  grep ' FAILED$' "$scratch/$short.tests" | sed 's/^/   RED /' || true
  [ "${KF_VALIDITY_KEEP:-0}" = 1 ] || git -C "$here" worktree remove --force "$wt"
done
