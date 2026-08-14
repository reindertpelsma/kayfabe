#!/usr/bin/env bash
# w304 — the confirmation sequence. One variable per boot, baseline FIRST.
# ★ Start marker + explicit terminator per arm, and an exit-status line, so
#   "file exists but has no terminator" is detectable at all: 143 (killed) and 124 (the
#   LAUNCHER expired while the job ran on fine) otherwise arrive as the same word.
set -uo pipefail
R=/workspace/kayfabe_w304
S=/workspace/w304_summary.txt
run() {
  local label=$1; shift
  echo "############ w304 ARM $label [$*] $(date -Is) ############" | tee -a "$S"
  "$R/scripts/bench/w304_confirm.sh" "$label" "$@"
  local rc=$?
  echo "############ w304 ARM $label DONE rc=$rc $(date -Is) ############" | tee -a "$S"
}
run base
run ptsweep  KAYFABE_PT_SWEEP=off
run opjoin   KAYFABE_OPERAND_JOIN=assert
run gpushbuf KAYFABE_GUEST_PUSHBUF=off
run gsema    KAYFABE_GUEST_SEMA=off
run goperand KAYFABE_GUEST_OPERAND=off
echo "############ w304 SEQ COMPLETE $(date -Is) ############" | tee -a "$S"
