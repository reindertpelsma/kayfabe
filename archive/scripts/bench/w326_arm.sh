#!/usr/bin/env bash
# ★★★★★ w326 — N boots of ONE arm, printing the PUBLISH PLANE's three instruments.
#
# Descends from `w321_arm.sh` and keeps every one of its reasons verbatim: one boot is not a
# grade, every metric is anchored, an absent metric prints `⊘UNMEASURED` and NEVER `0`, and a
# TERMINATOR line is written last so `143` (the job itself was killed) and `124` (the LAUNCHER
# expired while the job ran on) are distinguishable from a clean exit.
#
# ★ WHAT IS NEW — the three lines this rung exists to read:
#   MMUINVAL   — the guest's own TLB invalidate at BAR0 0xB830B0: writes, triggers,
#                ALL_PDB fraction, HUBTLB_ONLY split, polls, and the invalidates-per-doorbell
#                ratio. ⊘ Both terms of that ratio are taken by ONE observer over ONE
#                interval, so it cannot come to describe two boots.
#   PUBQUEUE   — the deferred publication lane: refused / coalesced / high_water.
#   TRAPWITNESS— inline_exceptions and worst_trap_us: the headline before/after.
#
#   usage: w326_arm.sh <TREE> <PREFIX> <N> [env assignments...]
set -uo pipefail
TREE=$1; PREFIX=$2; N=${3:-3}; shift 3 || true
LOG=/workspace/w326_arm_${PREFIX}.log
export PATH=/root/.cargo/bin:$PATH
export KAYFABE_REPO=$TREE
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-$(basename "$TREE")}
for kv in "$@"; do export "${kv?}"; done

{
echo "=== W326 ARM START tree=$TREE prefix=$PREFIX n=$N $(date -Is)"
# ⊘ The tree is rsync'd, not cloned, so `git rev-parse` has no repo to answer from. A
#   stamped file is the honest substitute — `no_provenance_looks_cleaner_than_bad_provenance`.
echo "=== REV=[$(cat "$TREE/KAYFABE_REV.txt" 2>/dev/null || echo '⊘ NO REV STAMP — UNATTRIBUTABLE')]"
echo "=== ARM ENV (intent — the boot's own lines are the record of EXECUTION):"
for kv in "$@"; do echo "===   $kv"; done
echo "===   KAYFABE_PUBLISH_PLANE=[${KAYFABE_PUBLISH_PLANE:-⊘unset ⇒ off ⇒ master behaviour}]"
for i in $(seq 1 "$N"); do
  export KAYFABE_TAG=${PREFIX}$i
  pkill -x qemu-system-x86 2>/dev/null; sleep 4
  echo "--- boot $i/$N tag=$KAYFABE_TAG $(date -Is)"
  # shellcheck disable=SC2086
  bash "$TREE/scripts/bench/"${W326_WORKLOAD:-w297_cup3.sh} >/dev/null 2>&1
  P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
  Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
  D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
  G=/workspace/bench/run_${KAYFABE_TAG}_dmesg.log
  V=$(grep -aoE '^CUP3_VAL=[A-Za-z0-9_]+' "$P" 2>/dev/null | tail -1)
  R=$(grep -aoE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
  C8=$(grep -aoE '^CUP8_(BAD|MAXERR)=[0-9]+' "$P" 2>/dev/null | tr '\n' ' ')
  A1=$(grep -a '★     R33 arm 1 COPY' "$P" 2>/dev/null | tail -1)
  # ★★★★★ THE THREE INSTRUMENTS.
  MI=$(grep -ao 'MMUINVAL [^"]*' "$Q" 2>/dev/null | tail -1)
  PQ=$(grep -ao 'PUBQUEUE [^"]*' "$Q" 2>/dev/null | tail -1)
  TW=$(grep -ao 'TRAPWITNESS [^"]*' "$Q" 2>/dev/null | tail -1)
  DR=$(grep -ao 'DRAIN\[visited=true asked=[0-9]* pinned=[0-9]* refused=[0-9]* DRAIN_MS=[0-9]* W319KNOB\[[^]]*\] complete=[a-z]*[^]]*' "$Q" 2>/dev/null | head -1)
  HR=$(grep -ao 'host_rows[= ][0-9]*/[0-9]*' "$Q" 2>/dev/null | tail -1)
  X=$(grep -ac Xid "$D" 2>/dev/null)
  XF=$(grep -aoE 'Xid \(PCI:[^)]*\): *[0-9]+|ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|faulted @ 0x[0-9a-f_]+|FAULT_[A-Z]+[0-9]*|ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | tr '\n' ' ')
  UN=$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)
  NV=$(grep -ac NVRM "$G" 2>/dev/null)
  ST=$(grep -ciE 'soft lockup|rcu_sched|rcu_preempt|RCU stall|hung task|blocked for more than' "$G" 2>/dev/null)
  echo "    BOOT $i: [${V:-⊘ABSENT-UNMEASURED}] [${R:-⊘NO-TERMINATOR}] [${C8:-⊘no-cup8}]"
  echo "            r33_arm1=[${A1:-⊘ NO arm-1 LINE — the measurement did not happen, ⊘ NOT a fail}]"
  echo "            ★★★★★ ${MI:-⊘MMUINVAL ABSENT — UNMEASURED, ⊘ NOT 'the register never fired'}"
  echo "            ★★★★★ ${PQ:-⊘PUBQUEUE ABSENT — UNMEASURED, ⊘ NOT 'nothing was queued'}"
  echo "            ★★★★★ ${TW:-⊘TRAPWITNESS ABSENT — UNMEASURED, ⊘ NOT 'zero exceptions'}"
  echo "            ★★★ DRAIN=[${DR:-⊘ NO DRAIN CLAUSE — UNMEASURED, ⊘ not complete}]"
  echo "            ${HR:-⊘host_rows UNMEASURED}  unserviced_distinct=[${UN:-⊘NOFILE}]"
  echo "            Xid_lines=[${X:-⊘NOFILE}] ${XF}"
  echo "            guest NVRM_lines=[${NV:-⊘NOFILE}] stall_lines=[${ST:-⊘NOFILE}]"
done
echo "=== W326 ARM EXIT status=0 $(date -Is)"
} >"$LOG" 2>&1
echo "=== W326 ARM TERMINATOR $PREFIX rc=0 $(date -Is)" >>"$LOG"
