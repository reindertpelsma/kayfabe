#!/usr/bin/env bash
# ★★★★★ THE DECIDING EXPERIMENT, TAKE 2. ⊘ w390d WAS VOID: `KAYFABE_VAS_PUBLISH` is set
# UNCONDITIONALLY from $ARM at w290p_run.sh:105, so the env var I passed was overridden and
# both rungs ran arm=drain with the doorbell publishing normally. The arm is the POSITIONAL.
L=/workspace/bench/w390f.log
echo "F_START $(date -Is) pid=$$" > $L
cd /root/kayfabe && git fetch -q origin && git checkout -q master && git pull -q --ff-only
echo "HEAD=$(git rev-parse --short HEAD)" >> $L
run() { tag=$1; posarm=$2; shift 2
  echo "=== RUNG $tag posarm=$posarm env=[$*] START $(date -Is) ===" >> $L
  ( env "$@" KAYFABE_REPO=/root/kayfabe CARGO_TARGET_DIR=/workspace/bench/cargo-target-w290 \
      KAYFABE_TAG=$tag POST_CAPTURE_HOOK=/root/kayfabe/scripts/bench/cup3_hook.sh \
      GQ_TIMEOUT=900 scripts/bench/w290p_run.sh $posarm )
  echo "=== RUNG $tag EXIT rc=$? $(date -Is) ===" >> $L
  P=/workspace/bench/run_${tag}_probe.log; Q=/workspace/bench/run_${tag}_qemu.log
  # ★★★ THE ARM ACTUALLY IN FORCE, FIRST — a boot happening is not an arm running, and
  #     w390d proved that reading the env I passed is not reading the arm.
  echo "    $tag arm IN FORCE = [$(grep -ao "VAS-PUBLISH token=[^ ]* arm=[a-z]*" $Q 2>/dev/null | grep -o "arm=[a-z]*" | sort | uniq -c | tr "\n" " ")]" >> $L
  echo "    $tag CUP3_VAL = [$(grep -a "^CUP3_VAL=" $P 2>/dev/null || echo "⊘ NO ANCHORED LINE — UNMEASURED, not 0")]" >> $L
  echo "    $tag doorbell published= = [$(grep -ao "published=[0-9]*" $Q 2>/dev/null | sort | uniq -c | sort -rn | head -3 | tr "\n" " ")]" >> $L
  echo "    $tag host Xid = [$(grep -ac "Xid (PCI" /workspace/bench/run_${tag}_hostdmesg.log 2>/dev/null)] (MUST be 0)" >> $L
}
run w390f1 assert KAYFABE_MMU_INVAL=off
run w390f2 assert KAYFABE_MMU_INVAL=on
echo "F_EXIT rc=0 $(date -Is)" >> $L
