#!/usr/bin/env bash
# ★★★★★ w298 — the ablation sequence. ONE VARIABLE PER BOOT, others at the w297 values.
# ⊘ Serial by construction: GPU tests on this bench are strictly serial and a parallel arm
#   would contend for the same tap, the same qemu-build path and the same GPU.
set -uo pipefail
cd /workspace/kayfabe_w298 || exit 90
S=/workspace/w298_summary.txt
# Order: PT_SWEEP and VAS_PUBLISH first (the brief's two most informative), then the supply-side
# pins, then the plane selectors. Each label is also the log tag, so every arm's artefacts are
# addressable after the fact.
run() { # run <label> <VAR=VALUE>
  echo "############ w298 ARM $1 [$2] $(date -Is) ############" | tee -a "$S"
  # ⊘ A leftover qemu from the previous arm would make the next boot's port bind fail and the
  #   arm would read as OUR wall. `-x` with the 15-char comm truncation, per the standing trap.
  pkill -x qemu-system-x86 2>/dev/null && sleep 5
  bash scripts/bench/w298_ablate.sh "$1" "$2" >"/workspace/w298_$1.out" 2>&1
  echo "############ w298 ARM $1 DONE rc=$? $(date -Is) ############" | tee -a "$S"
}
run ptsweep    KAYFABE_PT_SWEEP=off
run vaspub     KAYFABE_VAS_PUBLISH=assert
run opjoin     KAYFABE_OPERAND_JOIN=assert
run grroute    KAYFABE_GR_ROUTE=refuse
run gring      KAYFABE_GUEST_RING=off
run gpushbuf   KAYFABE_GUEST_PUSHBUF=off
run gsema      KAYFABE_GUEST_SEMA=off
run goperand   KAYFABE_GUEST_OPERAND=off
run fbjoin     KAYFABE_FB_JOIN=off
run isolates   KAYFABE_ISOLATES=stillborn
run ceexec     KAYFABE_CE_EXECUTOR=local
run ptwitness  KAYFABE_PT_WITNESS_EXEC=off
echo "############ w298 SEQUENCE COMPLETE $(date -Is) ############" | tee -a "$S"
