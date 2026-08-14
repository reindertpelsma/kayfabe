#!/usr/bin/env bash
# ★★★★★ w328 — N boots of ONE arm, printing the BREADTH instruments beside the worst trap.
#
# Descends from `w326_arm.sh` and keeps every reason verbatim: one boot is not a grade, every
# metric is anchored, an absent metric prints `⊘UNMEASURED` and NEVER `0`, and a TERMINATOR
# line is written last so `143` (the job itself was killed) and `124` (the LAUNCHER expired
# while the job ran on) are distinguishable from a clean exit.
#
# ★ WHAT IS NEW — the two lines this rung exists to read, plus the one it is graded on:
#   W328SCOPE  — the PUBLICATION pass's breadth: `target_us` vs `other_us`, `other_published`,
#                `scoped_out`, and `breadth_share`. ⊘ `arm=`/`scoped=`/`target=` print on every
#                arm, so `absent` means an OLD BINARY and never `all`.
#   W328PIN    — the PIN pass's breadth: `other_us` over the SAMPLED VASes beside `drain_ms`,
#                which is the DOORBELLED one. The ratio between them is the whole question.
#   DRAIN[…]   — w319's grading invariant. `complete=true` AND `pinned == asked`.
#
# ⚠ THE GRADE IS ON STATE, NOT OUTCOME. A scoped pass that publishes too little presents as a
#   GPU fault indistinguishable from the pre-existing drain truncation, and anything that makes
#   the doorbell FASTER makes that truncation RARER WITHOUT FIXING IT. ⇒ `w319_attribute.sh`
#   is run on every boot, not only on the reds, and its verdict is printed either way.
#
#   usage: w328_arm.sh <TREE> <PREFIX> <N> [env assignments...]
set -uo pipefail
TREE=$1; PREFIX=$2; N=${3:-3}; shift 3 || true
LOG=/workspace/w328_arm_${PREFIX}.log
export PATH=/root/.cargo/bin:$PATH
export KAYFABE_REPO=$TREE
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-$(basename "$TREE")}
for kv in "$@"; do export "${kv?}"; done

{
echo "=== W328 ARM START tree=$TREE prefix=$PREFIX n=$N $(date -Is)"
# ⊘ The tree is rsync'd, not cloned, so `git rev-parse` has no repo to answer from.
echo "=== REV=[$(cat "$TREE/KAYFABE_REV.txt" 2>/dev/null || echo '⊘ NO REV STAMP — UNATTRIBUTABLE')]"
echo "=== ARM ENV (intent — the boot's own lines are the record of EXECUTION):"
for kv in "$@"; do echo "===   $kv"; done
echo "===   KAYFABE_PUBLISH_SCOPE=[${KAYFABE_PUBLISH_SCOPE:-⊘unset ⇒ all ⇒ master breadth}]"
echo "===   KAYFABE_DRAIN_BATCH=[${KAYFABE_DRAIN_BATCH:-⊘unset ⇒ off ⇒ master, one chain per row}]"
for i in $(seq 1 "$N"); do
  export KAYFABE_TAG=${PREFIX}$i
  pkill -x qemu-system-x86 2>/dev/null; sleep 4
  echo "--- boot $i/$N tag=$KAYFABE_TAG $(date -Is)"
  # shellcheck disable=SC2086
  bash "$TREE/scripts/bench/"${W328_WORKLOAD:-w297_cup3.sh} >/dev/null 2>&1
  P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
  Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
  D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
  G=/workspace/bench/run_${KAYFABE_TAG}_dmesg.log
  V=$(grep -aoE '^CUP3_VAL=[A-Za-z0-9_]+' "$P" 2>/dev/null | tail -1)
  R=$(grep -aoE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
  C8=$(grep -aoE '^CUP8_(BAD|MAXERR)=[0-9]+' "$P" 2>/dev/null | tr '\n' ' ')
  A1=$(grep -a '★     R33 arm 1 COPY' "$P" 2>/dev/null | tail -1)
  # ★★★★★ THE BREADTH INSTRUMENTS. `head -1` on the FIRST doorbell (where the drain runs)
  #   and `tail -1` on the LAST (where w326 measured `published=0`) — the two are different
  #   facts and a single sample would let one speak for the other.
  S1=$(grep -ao 'W328SCOPE\[[^]]*\]' "$Q" 2>/dev/null | head -1)
  SN=$(grep -ao 'W328SCOPE\[[^]]*\]' "$Q" 2>/dev/null | tail -1)
  SC=$(grep -aoc 'W328SCOPE\[' "$Q" 2>/dev/null)
  N1=$(grep -ao 'W328PIN\[[^]]*\]' "$Q" 2>/dev/null | head -1)
  # ⚠ The FIRST pin line with a non-zero drain, not merely the first: doorbells before the
  #   channel resolves carry `drain_ms=0` and would report the breadth as the whole cost.
  NW=$(grep -ao 'W328PIN\[[^]]*\]' "$Q" 2>/dev/null | grep -av 'drain_ms=0]' | head -1)
  TW=$(grep -ao 'TRAPWITNESS [^"]*' "$Q" 2>/dev/null | tail -1)
  # ★ w319's grading clause, with the NESTED bracket its own selftest was broken on.
  DR=$(grep -ao 'DRAIN\[visited=true asked=[0-9]* pinned=[0-9]* refused=[0-9]* DRAIN_MS=[0-9]* W319KNOB\[[^]]*\] complete=[a-z]*[^]]*' "$Q" 2>/dev/null | head -1)
  LV=$(grep -ao 'last_pinned_va=0x[0-9a-f]*' "$Q" 2>/dev/null | head -1)
  # ★ The publication pass's own per-pass cost, from the segment census — an independent
  #   reading of the same thing W328SCOPE brackets.
  KF=$(grep -ao 'KFTIME-SEG vas_publish.*' "$Q" 2>/dev/null | tail -1)
  HR=$(grep -ao 'host_rows[= ][0-9]*/[0-9]*' "$Q" 2>/dev/null | tail -1)
  X=$(grep -ac Xid "$D" 2>/dev/null)
  XF=$(grep -aoE 'Xid \(PCI:[^)]*\): *[0-9]+|ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|faulted @ 0x[0-9a-f_]+|FAULT_[A-Z]+[0-9]*|ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | tr '\n' ' ')
  UN=$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)
  NV=$(grep -ac NVRM "$G" 2>/dev/null)
  ST=$(grep -ciE 'soft lockup|rcu_sched|rcu_preempt|RCU stall|hung task|blocked for more than' "$G" 2>/dev/null)
  # ★★★★★ THE ATTRIBUTOR, ON EVERY BOOT — a green is graded too, per this rung's own trap.
  bash "$TREE/scripts/bench/w319_attribute.sh" "$KAYFABE_TAG" >/tmp/w328att.$$ 2>&1
  AT=$?
  AL=$(grep -aoE 'VERDICT=[0-9].*' /tmp/w328att.$$ 2>/dev/null | tail -1)
  rm -f /tmp/w328att.$$
  echo "    BOOT $i: [${V:-⊘ABSENT-UNMEASURED}] [${R:-⊘NO-TERMINATOR}] [${C8:-⊘no-cup8}]"
  echo "            r33_arm1=[${A1:-⊘ NO arm-1 LINE — the measurement did not happen, ⊘ NOT a fail}]"
  echo "            ★★★★★ FIRST  ${S1:-⊘W328SCOPE ABSENT — UNMEASURED / OLD BINARY, ⊘ NOT 'breadth is free'}"
  echo "            ★★★★★ LAST   ${SN:-⊘W328SCOPE ABSENT — UNMEASURED}   passes=[${SC:-⊘NOFILE}]"
  echo "            ★★★★★ ${NW:-${N1:-⊘W328PIN ABSENT — UNMEASURED, ⊘ NOT 'the sample costs nothing'}}"
  echo "            ★★★★★ ${TW:-⊘TRAPWITNESS ABSENT — UNMEASURED, ⊘ NOT 'zero exceptions'}"
  echo "            ★★★ DRAIN=[${DR:-⊘ NO DRAIN CLAUSE — UNMEASURED, ⊘ not complete}] ${LV:-⊘no last_pinned_va}"
  echo "            ★★★ ATTRIBUTOR rc=$AT ${AL:-⊘ no VERDICT line}"
  echo "            ${KF:-⊘KFTIME-SEG vas_publish ABSENT — UNMEASURED}"
  echo "            ${HR:-⊘host_rows UNMEASURED}  unserviced_distinct=[${UN:-⊘NOFILE}]"
  echo "            Xid_lines=[${X:-⊘NOFILE}] ${XF}"
  echo "            guest NVRM_lines=[${NV:-⊘NOFILE}] stall_lines=[${ST:-⊘NOFILE}]"
done
echo "=== W328 ARM EXIT status=0 $(date -Is)"
} >"$LOG" 2>&1
echo "=== W328 ARM TERMINATOR $PREFIX rc=0 $(date -Is)" >>"$LOG"
