#!/usr/bin/env bash
# ★★★★★ w321 — N cup3 boots of ONE arm, printing the DECOMPOSITION as well as the mechanism.
#
# Descends from `w319_arm.sh` and keeps every one of its reasons verbatim: one boot is not a
# grade, every metric is anchored, an absent metric prints `⊘UNMEASURED` and NEVER `0`, and a
# TERMINATOR line is written last so `143` (the job itself was killed) and `124` (the LAUNCHER
# expired while the job ran on) are distinguishable from a clean exit.
#
# ★ WHAT IS NEW, and it is w321's whole first move: the three DECOMPOSITION rows.
#   W321IPC    — the parent-side bracket: ipc_calls/row, ipc_us/row, ours_us/row, ipc_share%.
#   W321CHILD  — the child's OWN service time per request kind, off its stderr.
#   W321CENSUS — the contiguity distribution of the rows the drain walked.
#   Together they answer *"is the ~225 µs/row the SOCKET or the IOCTL?"*, which decides
#   whether the fix needs physical contiguity at all.
#
#   usage: w321_arm.sh <TREE> <PREFIX> <N> [env assignments...]
set -uo pipefail
TREE=$1; PREFIX=$2; N=${3:-3}; shift 3 || true
LOG=/workspace/w321_arm_${PREFIX}.log
export PATH=/root/.cargo/bin:$PATH
export KAYFABE_REPO=$TREE
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-$(basename "$TREE")}
for kv in "$@"; do export "${kv?}"; done

{
echo "=== W321 ARM START tree=$TREE prefix=$PREFIX n=$N $(date -Is)"
echo "=== HEAD=$(cd "$TREE" && git rev-parse HEAD)"
echo "=== DIRT=[$(cd "$TREE" && git status --porcelain --untracked-files=no | head -3)]"
echo "=== ARM ENV (intent — the boot's own KNOB lines are the record of EXECUTION):"
for kv in "$@"; do echo "===   $kv"; done
echo "===   KAYFABE_VAS_DRAIN_BUDGET_MS=[${KAYFABE_VAS_DRAIN_BUDGET_MS:-⊘unset ⇒ 3000 default}]"
echo "===   KAYFABE_VAS_DRAIN_ROW_LIMIT=[${KAYFABE_VAS_DRAIN_ROW_LIMIT:-⊘unset ⇒ 65536 default}]"
echo "===   KAYFABE_DRAIN_BATCH=[${KAYFABE_DRAIN_BATCH:-⊘unset ⇒ off ⇒ master behaviour}]"
for i in $(seq 1 "$N"); do
  export KAYFABE_TAG=${PREFIX}$i
  pkill -x qemu-system-x86 2>/dev/null; sleep 4
  echo "--- boot $i/$N tag=$KAYFABE_TAG $(date -Is)"
  # ⊘ UNQUOTED on purpose — `W321_WORKLOAD` may carry an argument (`w309_crit1.sh fresh`),
  #   and the three workloads this rung must clear do not share one invocation shape.
  # shellcheck disable=SC2086
  bash "$TREE/scripts/bench/"${W321_WORKLOAD:-w297_cup3.sh} >/dev/null 2>&1
  P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
  Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
  D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
  G=/workspace/bench/run_${KAYFABE_TAG}_dmesg.log
  V=$(grep -aoE '^CUP3_VAL=[A-Za-z0-9_]+' "$P" 2>/dev/null | tail -1)
  R=$(grep -aoE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
  C8=$(grep -aoE '^CUP8_(BAD|MAXERR)=[0-9]+' "$P" 2>/dev/null | tr '\n' ' ')
  # ⚠ The R33 workload's own line. It is graded on the CLIENT's words, never on `CUP3_VAL` —
  #   `w309_crit1.sh fresh` PROVOKES a fault on purpose, so its Xid is printed, not judged.
  A1=$(grep -a '★     R33 arm 1 COPY' "$P" 2>/dev/null | tail -1)
  RR=$(grep -aoE '^R33_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
  DR=$(grep -ao 'DRAIN\[visited=true asked=[0-9]* pinned=[0-9]* refused=[0-9]* DRAIN_MS=[0-9]* W319KNOB\[[^]]*\] complete=[a-z]*[^]]*' "$Q" 2>/dev/null | head -1)
  KN=$(grep -ao 'W319KNOB\[[^]]*\]' "$Q" 2>/dev/null | head -1)
  LV=$(grep -ao 'last_pinned_va=0x[0-9a-f]*' "$Q" 2>/dev/null | head -1)
  BH=$(grep -ac 'WALL BUDGET HIT' "$Q" 2>/dev/null)
  CH=$(grep -ac 'ROW CAP.*HIT' "$Q" 2>/dev/null)
  # ★★★ THE DECOMPOSITION — the first drain's own rows.
  CE=$(grep -ao 'W321CENSUS\[[^]]*\]' "$Q" 2>/dev/null | head -1)
  IP=$(grep -ao 'W321IPC\[[^]]*\]' "$Q" 2>/dev/null | head -1)
  BA=$(grep -ao 'W321BATCH\[[^]]*\]' "$Q" 2>/dev/null | head -1)
  # ⊘ The child's counters are CUMULATIVE and monotonic; the LAST line of the boot is the
  #   whole boot's total, and the first drain dominates it by two orders of magnitude.
  CL=$(grep -ao 'W321CHILD worker=[0-9]* served=[0-9]*.*' "$Q" 2>/dev/null | tail -1)
  CN=$(grep -ac 'W321CHILD' "$Q" 2>/dev/null)
  X=$(grep -ac Xid "$D" 2>/dev/null)
  XF=$(grep -aoE 'Xid \(PCI:[^)]*\): *[0-9]+|ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|faulted @ 0x[0-9a-f_]+|FAULT_[A-Z]+[0-9]*|ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | tr '\n' ' ')
  HB=$(stat -c%s "$D" 2>/dev/null)
  NV=$(grep -ac NVRM "$G" 2>/dev/null)
  ST=$(grep -ciE 'soft lockup|rcu_sched|rcu_preempt|RCU stall|hung task|blocked for more than' "$G" 2>/dev/null)
  echo "    BOOT $i: [${V:-⊘ABSENT-UNMEASURED}] [${R:-⊘NO-TERMINATOR}] [${C8:-⊘no-cup8}] [${RR:-⊘no-r33}]"
  echo "            r33_arm1=[${A1:-⊘ NO arm-1 LINE — the measurement did not happen, ⊘ NOT a fail}]"
  echo "            ★ KNOB=[${KN:-⊘UNMEASURED — OLD BINARY, ⊘ not 'default'}]"
  echo "            ★★★ DRAIN=[${DR:-⊘ NO DRAIN CLAUSE — UNMEASURED, ⊘ not complete}]"
  echo "            ${LV:-⊘last_pinned_va UNMEASURED}  budget_hit_lines=[${BH:-⊘NOFILE}] row_cap_hit_lines=[${CH:-⊘NOFILE}]"
  echo "            ★★★★★ ${IP:-⊘W321IPC ABSENT — UNMEASURED, ⊘ not 0}"
  echo "            ★★★★★ ${BA:-⊘W321BATCH ABSENT — UNMEASURED, ⊘ not 'off'}"
  echo "            ★★★★★ ${CE:-⊘W321CENSUS ABSENT — UNMEASURED, ⊘ not 'contiguous'}"
  echo "            ★★★★★ child_lines=[${CN:-⊘NOFILE}] LAST=[${CL:-⊘W321CHILD ABSENT — UNMEASURED}]"
  echo "            hostdmesg_bytes=[${HB:-⊘NOFILE}] Xid_lines=[${X:-⊘NOFILE}] ${XF}"
  echo "            guest NVRM_lines=[${NV:-⊘NOFILE}] stall_lines=[${ST:-⊘NOFILE}]"
done
echo "=== W321 ARM EXIT status=0 $(date -Is)"
} >"$LOG" 2>&1
echo "=== W321 ARM TERMINATOR $PREFIX rc=0 $(date -Is)" >>"$LOG"
