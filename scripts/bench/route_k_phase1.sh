#!/usr/bin/env bash
# ★★★★★ w750 PHASE 1 — route K's birth-client probe, run on a plain GPU box.
#
# `docs/design/w750_route_k_prereg.md` is the spec and the grading sheet. This script runs
# the probe and nothing else: **no guest, no KVM, no QEMU.**
#
# ⚠ THREE TRAPS THIS SCRIPT EXISTS TO ENCODE, all of them measured before:
#
# 1. ★★★ **A KILLED job is indistinguishable from a RUNNING one if absence-of-result is the
#    only check.** So the log gets a START marker and a terminating `PHASE1_RC=` line, and a
#    file with no terminator is a *third* state — neither "running" nor "finished" — that a
#    reader can actually name.
# 2. ★★ **`pgrep -f <literal>` always matches the asker**, and `pgrep -x` can never match a
#    name longer than 15 characters. This script therefore never greps for itself; the
#    binary's own `K_EXIT=` line is the liveness statement.
# 3. ⊘ **Exit status is not the instrument.** The probe prints every `K_*` row whether it
#    held or not; the grader below reads the ROWS, and the script's own status is a summary
#    of that reading rather than a substitute for it.
set -uo pipefail
TAG=${1:-phase1}
GPU=${ROUTE_K_GPU:-0}
OUT=${ROUTE_K_OUT:-/workspace/bench/route_k_${TAG}.log}
mkdir -p "$(dirname "$OUT")"

BIN=""
for c in "${CARGO_TARGET_DIR:-target}"/release/kayfabe-rm-ladder \
         "${CARGO_TARGET_DIR:-target}"/debug/kayfabe-rm-ladder \
         target/release/kayfabe-rm-ladder target/debug/kayfabe-rm-ladder; do
  [ -x "$c" ] && { BIN="$c"; break; }
done
if [ -z "$BIN" ]; then
  echo "PHASE1_RC=127 ⊘ UNMEASURED_NO_BINARY — build kayfabe-rm-ladder first" | tee -a "$OUT"
  exit 127
fi

{
  echo "PHASE1_START=$(date -Is)"
  echo "PHASE1_HOST=$(hostname)"
  echo "PHASE1_BIN=$BIN"
  echo "PHASE1_REV=$(git -C "$(dirname "$0")/../.." rev-parse HEAD 2>/dev/null || echo unknown)"
  echo "PHASE1_DRIVER=$(cat /proc/driver/nvidia/version 2>/dev/null | head -1 || echo none)"
  echo "PHASE1_GPU_INDEX=$GPU"
} | tee "$OUT"

# ⚠ The probe re-executes itself for its two other roles, so it must be run DIRECTLY and
#   not under anything that rewrites argv.
timeout "${ROUTE_K_TIMEOUT:-300}" "$BIN" --route-k --gpu "$GPU" 2>&1 | tee -a "$OUT"
RC=${PIPESTATUS[0]}
echo "PHASE1_PROBE_RC=$RC" | tee -a "$OUT"

# ★ And the second arm: constraint 32 step 6 DELIBERATELY SKIPPED, which is the
#   known-positive for `K_DUP_OUTSTANDING`. A zero that cannot be made non-zero is not a
#   measurement.
timeout "${ROUTE_K_TIMEOUT:-300}" "$BIN" --route-k --route-k-skip-free --gpu "$GPU" 2>&1 \
  | sed 's/^K_/KP2_K_/' | tee -a "$OUT"
RC2=${PIPESTATUS[0]}
echo "PHASE1_SKIPFREE_RC=$RC2" | tee -a "$OUT"

# ---- the grading sheet, read from the ROWS ------------------------------------------------
grade() {
  local key="$1" want="$2"
  local got
  got=$(grep -m1 "^${key}=" "$OUT" | cut -d= -f2-)
  if [ -z "$got" ]; then
    echo "ROW ${key}: ⊘ ABSENT — UNMEASURED"
    return 1
  fi
  if [ "$got" = "$want" ]; then
    echo "ROW ${key}=${got} (want ${want}) HELD"
    return 0
  fi
  echo "ROW ${key}=${got} (want ${want}) ⊘ NOT-AS-PREDICTED"
  return 1
}

echo "=== w750 PHASE 1 GRADING (docs/design/w750_route_k_prereg.md §2) ===" | tee -a "$OUT"
{
  grade K_BIT5 0
  grade K_BIT5_KP 1
  grade K_PID_I_IN_CHAN 1
  grade K_PID_S_IN_CHAN 0
  grade K_PID_I_IN_DEV 1
  grade K_PID_S_IN_DEV 1
  grade K_DUP_MAP_PRE_RC 0
  grade K_CHANNEL_LIVE 1
  grade K_DUP_OUTSTANDING 0
  grade KP2_K_DUP_OUTSTANDING 1
  grade K_UVM_REG_CHAN_RC 0x0
  echo "ROW K_DUP_MAP_POST_RC=$(grep -m1 '^K_DUP_MAP_POST_RC=' "$OUT" | cut -d= -f2-) \
(want ANY NON-ZERO)"
  echo "ROW K_UVM_REG_CHAN_NEG_RC=$(grep -m1 '^K_UVM_REG_CHAN_NEG_RC=' "$OUT" | cut -d= -f2-) \
(want ANY NON-ZERO)"
  echo "ROW K_USERD_POISON_SURVIVED=$(grep -m1 '^K_USERD_POISON_SURVIVED=' "$OUT" | cut -d= -f2-) \
(recorded, NOT a gate)"
} | tee -a "$OUT"

echo "PHASE1_RC=$((RC != 0 || RC2 != 0))" | tee -a "$OUT"
