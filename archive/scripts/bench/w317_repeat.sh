#!/usr/bin/env bash
# ★★★★★ w317 — N cup3 boots of a given tree, printing EVERY boot's anchored values.
#
# Descends from w314_repeat.sh, whose reason stands verbatim: `[measured 2026-08-14, vh]`
# `^CUP3_VAL=43` came back **4/5 on a branch AND 4/5 on clean master**, both reds identical
# field for field ⇒ **one boot is not a grade** and the DISTRIBUTION is the artefact.
#
# ⚠ ADDED HERE, and it is this rung's subject: `max_drain_us`, `budget_hit`, and the
#   `DRAIN-DEFER` trajectory. `max_reap_us` is kept because it is the number that must MOVE.
# ⊘ Every metric is anchored and prints `⊘UNMEASURED` when absent — never 0.
# ⊘ Numbers are sorted with `sort -n` over a bare numeric stream, never `sort -t= -k2 -n`
#   (which errors "separator must be exactly one character long" and silently empties the list).
set -uo pipefail
TREE=$1; PREFIX=$2; N=${3:-4}
LOG=/workspace/w317_repeat_${PREFIX}.log
export PATH=/root/.cargo/bin:$PATH
export KAYFABE_REPO=$TREE
export CARGO_TARGET_DIR=/workspace/bench/cargo-target-$(basename "$TREE")
{
echo "=== W317 REPEAT START tree=$TREE prefix=$PREFIX n=$N $(date -Is)"
echo "=== HEAD=$(cd "$TREE" && git rev-parse HEAD)"
echo "=== DIRT=[$(cd "$TREE" && git status --porcelain --untracked-files=no | head -3)]"
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
  X=$(grep -c Xid "$D" 2>/dev/null)
  XI=$(grep -oE 'ENGINE [A-Z0-9_]+|faulted @ 0x[0-9a-f_]+|ACCESS_TYPE_[A-Z_]+|FAULT_[A-Z]+' "$D" 2>/dev/null | tr '\n' ' ')
  PR=$(grep -ao 'PIN-RELEASE released=[0-9]* refused_no_host_vas=[0-9]* rows_deduped=[0-9]*' "$Q" 2>/dev/null | tail -1)
  RP=$(grep -ac 'kayfabe: REAP reaped=' "$Q" 2>/dev/null)
  RL=$(grep -ao 'REAP reaped=[0-9]* still_retired=[0-9]* deferred_for_drain=[0-9]*' "$Q" 2>/dev/null | tail -2 | tr '\n' ' ')
  TM=$(grep -ao 'max_reap_us=[0-9]*' "$Q" 2>/dev/null | grep -oE '[0-9]+$' | sort -n | tail -1)
  DM=$(grep -ao 'max_drain_us=[0-9]*' "$Q" 2>/dev/null | grep -oE '[0-9]+$' | sort -n | tail -1)
  DT=$(grep -ao 'DRAIN-TIMING max_drain_us=[0-9]* disposed=[0-9]* residue=[0-9]* turns=[0-9]* budget_hit=[a-z]*' "$Q" 2>/dev/null | tail -1)
  DD=$(grep -ao 'DRAIN-DEFER deferred_for_drain=[0-9]* still_retired=[0-9]*' "$Q" 2>/dev/null | tr '\n' ' | ')
  DDN=$(grep -ac 'DRAIN-DEFER' "$Q" 2>/dev/null)
  ST=$(grep -ciE 'soft lockup|rcu_sched|rcu_preempt|RCU stall|hung task|blocked for more than' "$G" 2>/dev/null)
  NV=$(grep -ac NVRM "$G" 2>/dev/null)
  HR=$(grep -ao 'host_rows=[0-9]*' "$Q" 2>/dev/null | grep -oE '[0-9]+$' | sort -n | tail -1)
  echo "    BOOT $i: [${V:-⊘ABSENT-UNMEASURED}] [${R:-⊘NO-TERMINATOR}] Xid=[${X:-⊘NOFILE}] ${XI}"
  echo "            [${PR:-⊘ NO PIN-RELEASE LINE — UNMEASURED, ⊘ not released=0}] REAP_lines=[${RP:-0}]"
  echo "            REAP_last=[${RL:-⊘UNMEASURED}]"
  echo "            ★ max_reap_us=[${TM:-⊘UNMEASURED}]   ★ max_drain_us=[${DM:-⊘UNMEASURED}]"
  echo "            DRAIN-TIMING=[${DT:-⊘UNMEASURED}]"
  echo "            DRAIN-DEFER lines=[${DDN:-0}] traj=[${DD:-⊘UNMEASURED}]"
  echo "            guest_stall_lines=[${ST:-⊘NOFILE}] NVRM_lines=[${NV:-⊘NOFILE}] host_rows_max=[${HR:-⊘UNMEASURED}]"
  bash "$TREE/scripts/bench/regression_check_e.sh" "$Q" "$D" 2>&1 | grep -E "VERDICT|E1|E2|E3" | sed 's/^/            E: /'
done
echo "=== W317 REPEAT EXIT status=0 $(date -Is)"
} >"$LOG" 2>&1
echo "=== W317 REPEAT TERMINATOR $PREFIX rc=0 $(date -Is)" >>"$LOG"
