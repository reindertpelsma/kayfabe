#!/usr/bin/env bash
# ★ v3 harness gates — build and run every `kf-gate*` on THIS box's GPU, no QEMU, no guest.
#
# Usage (on the box, in the kayfabe checkout):  bash scripts/bench/v3_gates.sh [out.log]
#
# ⊘ Three rules this script exists to encode, each paid for:
# - **The revision is part of the result** (CLAUDE.md: a bench claim without its source revision
#   is not a claim). HEAD and a dirty flag are the first lines written.
# - **A start marker and an exit line**, so "file exists but has no terminator" is detectable — a
#   killed job and a running one otherwise look identical.
# - **The verdict is each gate's own `GATEn_VERDICT=` line**, never "the binary exited" or "we got to
#   the end" (the_last_line_is_not_the_verdict). A gate that prints no verdict line is a FAIL.
set -uo pipefail
cd "$(dirname "$0")/../.."
OUT=${1:-/root/prov/v3_gates.log}
export PATH="$HOME/.cargo/bin:$PATH"
{
  echo "V3_GATES_START $(date -Is)"
  echo "HEAD=$(git rev-parse --short=8 HEAD) dirty=$(git status --porcelain --untracked-files=no | wc -l)"
  nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1 | sed 's/^/GPU=/'
  if ! cargo build -q --release -p kf-harness --bins 2>&1 | tail -20; then
    echo "BUILD=FAIL"; echo "V3_GATES_EXIT pass=0 fail=all $(date -Is)"; exit 1
  fi
  pass=0; fail=0; failed=""
  # Executables only: cargo also leaves `kf-gateN.d` dep files beside them (measured: a bare glob
  # ran them as six extra "gates" — counted FAIL, correctly, but they are not gates).
  for bin in $(ls target/release/kf-gate[0-9]* 2>/dev/null | grep -v '\.' | sort -V); do
    [ -x "$bin" ] || continue
    g=$(basename "$bin")
    echo "=== $g"
    out=$(timeout 180 "$bin" 2>&1); rc=$?
    echo "$out"
    v=$(echo "$out" | grep -E '^GATE[0-9]+_VERDICT=' | tail -1 | cut -d= -f2)
    if [ "$v" = "PASS" ] && [ "$rc" -eq 0 ]; then pass=$((pass+1)); else fail=$((fail+1)); failed="$failed $g(rc=$rc,verdict=${v:-NONE})"; fi
  done
  echo "V3_GATES_SUMMARY pass=$pass fail=$fail${failed:+ failed:$failed}"
  echo "V3_GATES_EXIT pass=$pass fail=$fail $(date -Is)"
} 2>&1 | tee "$OUT"
