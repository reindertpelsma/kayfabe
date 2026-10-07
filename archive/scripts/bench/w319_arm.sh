#!/usr/bin/env bash
# ★★★★★ w319 — N cup3 boots of ONE modulation arm, printing the mechanism variable per boot.
#
# Descends from `w317_repeat.sh` and keeps its reasons verbatim: one boot is not a grade, every
# metric is anchored, absent prints `⊘UNMEASURED` and never `0`, and a TERMINATOR line is
# written last so `143` (the job was killed) and `124` (the LAUNCHER expired, job fine) are
# distinguishable from a clean exit.
#
# ⚠ WHAT IS NEW HERE, and it is the point: this rung's outcome is NOT the binary `CUP3_VAL`.
#   It is the DRAIN's own completeness — `pinned/asked`, `complete=`, `last_pinned_va`,
#   `budget_hit` — which is a per-boot CONTINUOUS observable of the mechanism rather than a
#   20 %-probability consequence of it. A 20 % event needs ~15 boots to grade; this needs one.
#
#   usage: w319_arm.sh <TREE> <PREFIX> <N> [env assignments...]
#   e.g.:  w319_arm.sh /workspace/kayfabe_w319 w319r 3 KAYFABE_VAS_DRAIN_ROW_LIMIT=11800
set -uo pipefail
TREE=$1; PREFIX=$2; N=${3:-3}; shift 3 || true
LOG=/workspace/w319_arm_${PREFIX}.log
export PATH=/root/.cargo/bin:$PATH
export KAYFABE_REPO=$TREE
export CARGO_TARGET_DIR=/workspace/bench/cargo-target-$(basename "$TREE")
# ★ The arm's knobs, exported here and ECHOED below — the log records its own arming.
for kv in "$@"; do export "${kv?}"; done

{
echo "=== W319 ARM START tree=$TREE prefix=$PREFIX n=$N $(date -Is)"
echo "=== HEAD=$(cd "$TREE" && git rev-parse HEAD)"
echo "=== DIRT=[$(cd "$TREE" && git status --porcelain --untracked-files=no | head -3)]"
echo "=== ARM ENV (intent — the boot's own W319KNOB line is the record of EXECUTION):"
for kv in "$@"; do echo "===   $kv"; done
echo "===   KAYFABE_VAS_DRAIN_BUDGET_MS=[${KAYFABE_VAS_DRAIN_BUDGET_MS:-⊘unset ⇒ 3000 default}]"
echo "===   KAYFABE_VAS_DRAIN_ROW_LIMIT=[${KAYFABE_VAS_DRAIN_ROW_LIMIT:-⊘unset ⇒ 65536 default}]"
for i in $(seq 1 "$N"); do
  export KAYFABE_TAG=${PREFIX}$i
  pkill -x qemu-system-x86 2>/dev/null; sleep 4
  echo "--- boot $i/$N tag=$KAYFABE_TAG $(date -Is)"
  bash "$TREE/scripts/bench/w297_cup3.sh" >/dev/null 2>&1
  P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
  Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
  D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
  G=/workspace/bench/run_${KAYFABE_TAG}_dmesg.log
  V=$(grep -aoE '^CUP3_VAL=[A-Za-z0-9_]+' "$P" 2>/dev/null | tail -1)
  R=$(grep -aoE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
  # ★★★ THE MECHANISM VARIABLE — the drain clause of the FIRST (whole-VAS) drain.
  DR=$(grep -ao 'DRAIN\[visited=true asked=[0-9]* pinned=[0-9]* refused=[0-9]* DRAIN_MS=[0-9]* W319KNOB\[[^]]*\] complete=[a-z]*[^]]*' "$Q" 2>/dev/null | head -1)
  KN=$(grep -ao 'W319KNOB\[[^]]*\]' "$Q" 2>/dev/null | head -1)
  LV=$(grep -ao 'last_pinned_va=0x[0-9a-f]*' "$Q" 2>/dev/null | head -1)
  BH=$(grep -ac 'WALL BUDGET HIT' "$Q" 2>/dev/null)
  CH=$(grep -ac 'ROW CAP.*HIT' "$Q" 2>/dev/null)
  # ★ The host driver's own words — the ONLY place the Xid lives (never guest dmesg).
  X=$(grep -ac Xid "$D" 2>/dev/null)
  XF=$(grep -aoE 'Xid \(PCI:[^)]*\): *[0-9]+|ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|faulted @ 0x[0-9a-f_]+|FAULT_[A-Z]+[0-9]*|ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | tr '\n' ' ')
  HB=$(stat -c%s "$D" 2>/dev/null)
  NV=$(grep -ac NVRM "$G" 2>/dev/null)
  ST=$(grep -ciE 'soft lockup|rcu_sched|rcu_preempt|RCU stall|hung task|blocked for more than' "$G" 2>/dev/null)
  echo "    BOOT $i: [${V:-⊘ABSENT-UNMEASURED}] [${R:-⊘NO-TERMINATOR}]"
  echo "            ★ KNOB=[${KN:-⊘UNMEASURED — OLD BINARY, ⊘ not 'default'}]"
  echo "            ★★★ DRAIN=[${DR:-⊘ NO DRAIN CLAUSE — UNMEASURED, ⊘ not complete}]"
  echo "            ${LV:-⊘last_pinned_va UNMEASURED}  budget_hit_lines=[${BH:-⊘NOFILE}] row_cap_hit_lines=[${CH:-⊘NOFILE}]"
  echo "            hostdmesg_bytes=[${HB:-⊘NOFILE}] Xid_lines=[${X:-⊘NOFILE}] ${XF}"
  echo "            guest NVRM_lines=[${NV:-⊘NOFILE}] stall_lines=[${ST:-⊘NOFILE}]"
done
echo "=== W319 ARM EXIT status=0 $(date -Is)"
} >"$LOG" 2>&1
echo "=== W319 ARM TERMINATOR $PREFIX rc=0 $(date -Is)" >>"$LOG"
