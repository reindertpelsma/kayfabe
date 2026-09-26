#!/usr/bin/env bash
# ★ w390 LADDER. Serial (GPU tests never overlap). Each rung writes its own terminator.
# Rungs:
#   b3,b4  baseline @5aaebee8      — n=3 with b2, because a single 43 is wrong 1 in 5
#   c1     MMU_INVAL=off @w390     — asserts `off` really is byte-identical (the control)
#   c2     MMU_INVAL=on  @w390     — THE RUNG
L=/workspace/bench/w390_ladder.log
echo "LADDER_START $(date -Is) pid=$$" > $L
run() { # run <tag> <extra-env...>
  local tag=$1; shift
  echo "=== RUNG $tag START $(date -Is) env=[$*] ===" >> $L
  ( cd /root/kayfabe && env "$@" \
      KAYFABE_REPO=/root/kayfabe \
      CARGO_TARGET_DIR=/workspace/bench/cargo-target-w290 \
      KAYFABE_TAG=$tag \
      POST_CAPTURE_HOOK=/root/kayfabe/scripts/bench/cup3_hook.sh \
      GQ_TIMEOUT=900 \
      scripts/bench/w290p_run.sh drain )
  echo "=== RUNG $tag EXIT rc=$? $(date -Is) ===" >> $L
  # ⊘ Grade from the PROBE log, ANCHORED. `GCC_CUP3_RC=0` matches an unanchored read and has
  #   printed the headline success value on a FAILING arm seven times in this campaign.
  local P=/workspace/bench/run_${tag}_probe.log
  echo "    ${tag} CUP3_VAL = [$(grep -a "^CUP3_VAL=" $P 2>/dev/null || echo "⊘ NO ANCHORED LINE — NOT a zero, UNMEASURED")]" >> $L
  echo "    ${tag} CUP3_RC  = [$(grep -a "^CUP3_RC=" $P 2>/dev/null || echo "⊘ ABSENT")]" >> $L
  echo "    ${tag} MMUINVAL = [$(grep -ao "MMUINVAL[^|]\{0,200\}" /workspace/bench/run_${tag}_qemu.log 2>/dev/null | tail -1)]" >> $L
  echo "    ${tag} BLOCKAGE = [$(grep -ao "BLOCKAGE-COVERAGE[^|]\{0,240\}" /workspace/bench/run_${tag}_qemu.log 2>/dev/null | tail -1)]" >> $L
  echo "    ${tag} host Xid = [$(grep -ac "Xid (PCI" /workspace/bench/run_${tag}_hostdmesg.log 2>/dev/null)]  (MUST be 0)" >> $L
}
run w390b3
run w390b4
echo "=== PULL to the w390 commit $(date -Is) ===" >> $L
( cd /root/kayfabe && git fetch -q origin && git checkout -q master && git pull -q --ff-only ) >> $L 2>&1
echo "=== HEAD NOW = $(cd /root/kayfabe && git rev-parse --short HEAD) ===" >> $L
run w390c1 KAYFABE_MMU_INVAL=off
run w390c2 KAYFABE_MMU_INVAL=on
echo "LADDER_EXIT rc=0 $(date -Is)" >> $L
