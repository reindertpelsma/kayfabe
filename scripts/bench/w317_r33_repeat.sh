#!/usr/bin/env bash
# ★★★★★ w317 — N boots of the SECOND workload: `R33 arm 1`, a raw CE client with no libcuda.
#
# ⊘ `43` alone is not sufficient evidence — `relaxation_inert_gate.sh` exists on master
#   because a single-workload grade let a regression in (w304 → w313). Same n>=3 rule as the
#   cup3 arm: one boot is not a grade on this box.
# ⚠ A, B and F of w310's list are cup3-only and MUST NOT be graded here: `w309_crit1.sh`'s
#   `fresh` arm PROVOKES A FAULT ON PURPOSE, so its boot legitimately carries
#   `Xid 31 CE0 … @ 0x7_00100000`. This script therefore grades the CLIENT'S OWN LINE and the
#   drain instruments, and prints the Xid rather than judging it.
set -uo pipefail
TREE=$1; PREFIX=$2; N=${3:-3}
LOG=/workspace/w317_repeat_${PREFIX}.log
export PATH=/root/.cargo/bin:$PATH
export KAYFABE_REPO=$TREE
export CARGO_TARGET_DIR=/workspace/bench/cargo-target-$(basename "$TREE")
{
echo "=== W317 R33 REPEAT START tree=$TREE prefix=$PREFIX n=$N $(date -Is)"
echo "=== HEAD=$(cd "$TREE" && git rev-parse HEAD)"
echo "=== DIRT=[$(cd "$TREE" && git status --porcelain --untracked-files=no | head -3)]"
for i in $(seq 1 "$N"); do
  export KAYFABE_TAG=${PREFIX}$i
  pkill -x qemu-system-x86 2>/dev/null; sleep 4
  echo "--- boot $i/$N tag=$KAYFABE_TAG $(date -Is)"
  bash "$TREE/scripts/bench/w309_crit1.sh" fresh >/dev/null 2>&1
  P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
  Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
  D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
  G=/workspace/bench/run_${KAYFABE_TAG}_dmesg.log
  A1=$(grep -a '★     R33 arm 1 COPY' "$P" 2>/dev/null | tail -1)
  RC=$(grep -aoE '^R33_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
  TM=$(grep -ao 'max_reap_us=[0-9]*' "$Q" 2>/dev/null | grep -oE '[0-9]+$' | sort -n | tail -1)
  DM=$(grep -ao 'max_drain_us=[0-9]*' "$Q" 2>/dev/null | grep -oE '[0-9]+$' | sort -n | tail -1)
  DT=$(grep -ao 'DRAIN-TIMING max_drain_us=[0-9]* disposed=[0-9]* residue=[0-9]* turns=[0-9]* budget_hit=[a-z]*' "$Q" 2>/dev/null | tail -1)
  DD=$(grep -ao 'DRAIN-DEFER deferred_for_drain=[0-9]* still_retired=[0-9]*' "$Q" 2>/dev/null | tr '\n' ' | ')
  PR=$(grep -ao 'PIN-RELEASE released=[0-9]* refused_no_host_vas=[0-9]* rows_deduped=[0-9]*' "$Q" 2>/dev/null | tail -1)
  RP=$(grep -ac 'kayfabe: REAP reaped=' "$Q" 2>/dev/null)
  XI=$(grep -oE 'ENGINE [A-Z0-9_]+|faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | tr '\n' ' ')
  ST=$(grep -ciE 'soft lockup|rcu_sched|rcu_preempt|RCU stall|hung task|blocked for more than' "$G" 2>/dev/null)
  echo "    BOOT $i: arm1 verbatim: [${A1:-⊘ NO arm-1 LINE — THE MEASUREMENT DID NOT HAPPEN, ⊘ this is NOT a fail}]"
  echo "            [${RC:-⊘NO-TERMINATOR}]  (⊘ the fresh arm provokes a fault on purpose; Xid here is EXPECTED)"
  echo "            host Xid ctx: ${XI:-<none>}"
  echo "            ★ max_reap_us=[${TM:-⊘UNMEASURED}]   ★ max_drain_us=[${DM:-⊘UNMEASURED}]"
  echo "            DRAIN-TIMING=[${DT:-⊘UNMEASURED}]   DRAIN-DEFER traj=[${DD:-⊘UNMEASURED}]"
  echo "            [${PR:-⊘ NO PIN-RELEASE LINE — UNMEASURED}] REAP_lines=[${RP:-0}] guest_stall_lines=[${ST:-⊘NOFILE}]"
done
echo "=== W317 R33 REPEAT EXIT status=0 $(date -Is)"
} >"$LOG" 2>&1
echo "=== W317 R33 REPEAT TERMINATOR $PREFIX rc=0 $(date -Is)" >>"$LOG"
