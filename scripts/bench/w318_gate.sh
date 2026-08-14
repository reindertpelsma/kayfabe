#!/usr/bin/env bash
# ★★★★★ w318 — THE DIRTY GATE, MEASURED THE WAY w315 MEASURED THE THING IT GATES.
#
#   usage: scripts/bench/w318_gate.sh <off|on|pub|wit>
#     off — BOTH GATES DISARMED. ★ THE CONTROL, and it is a better control than master's
#           binary would be: same binary, same revision, ONE VARIABLE (two environment
#           strings). A master boot would also differ in the two `AddressTable` fields and
#           the `SparseFb` counter this branch adds, and nothing would say which difference
#           moved a number.
#     on  — BOTH GATES ARMED. The measurement.
#     pub — only the publication gate (the 55.7 % term).
#     wit — only the executor-witness gate (which is what makes `pt_decode`'s 25.7 % term
#           run at all).
#
# ⊘ It delegates to `w315_floor.sh full` and changes NOTHING about how the numbers are taken:
#   the same `KAYFABE_KFTIME=on` per-event brackets, the same `w290p_run.sh` arming, the same
#   `cup8bench` hook at N=512/12 iters, the same 1 Hz KVM sampler. That is the point — w318's
#   before/after must be comparable to w315's table, and a re-implemented instrument would
#   not be.
#
# ## ⚠ WHAT THIS ARM CANNOT SAY, stated before the first boot
#
# `KAYFABE_BENCH_ONLY=measure` and no `BENCH_NOLAUNCH` negative control, inherited from
# w315 §8. ⇒ **A green `bad=0` here is UNGUARDED**, exactly as it was there. Correctness for
# this rung is graded by `relaxation_inert_gate.sh` on BOTH workloads at n ≥ 3, in separate
# boots, and NOT by anything this script prints.
#
# ## ★★★ THE DIAGNOSTIC THIS RUNG TURNS ON — the fire/skip ratio
#
# Pre-registered outcome (B) is *"the gate fires and the trap does not drop"*, and it is
# **indistinguishable from a working gate by `trap_ms` alone**: both produce a number, and a
# number that did not move reads as "no effect" whether the gate skipped nothing or skipped
# everything and the cost was elsewhere. `DIRTY-GATE publish[fired=… skipped=…]
# witness[fired=… skipped=…]` on every `PT-DECODE` line is what separates them, and it is
# extracted below whatever the timing says.
set -uo pipefail
ARM="${1:-}"
case "$ARM" in off|on|pub|wit) ;;
  *) echo "usage: $0 off|on|pub|wit" >&2; exit 64 ;;
esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w318}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w318}
export KAYFABE_TAG=${KAYFABE_TAG:-w318$ARM}

# ---- THE ONE VARIABLE -------------------------------------------------------------------
# ⊘ Two strings, and nothing else in this file differs between the arms. Both variables are
#   set EXPLICITLY on every arm, including to `off`: an unset variable and one set to `off`
#   behave identically in the device, but only one of them appears in the log — and a rung
#   whose control is "I did not export something" cannot be read back from its own evidence.
case "$ARM" in
  off) export KAYFABE_DIRTY_GATE_PUBLISH=off KAYFABE_DIRTY_GATE_WITNESS=off ;;
  on)  export KAYFABE_DIRTY_GATE_PUBLISH=on  KAYFABE_DIRTY_GATE_WITNESS=on  ;;
  pub) export KAYFABE_DIRTY_GATE_PUBLISH=on  KAYFABE_DIRTY_GATE_WITNESS=off ;;
  wit) export KAYFABE_DIRTY_GATE_PUBLISH=off KAYFABE_DIRTY_GATE_WITNESS=on  ;;
esac

"$REPO/scripts/bench/w315_floor.sh" full
IRC=$?

Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
OUT=/workspace/${KAYFABE_TAG}.log

{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W318 arm=$ARM  inner_rc=$IRC  $(date -Is)"
echo "===     PUBLISH=[$KAYFABE_DIRTY_GATE_PUBLISH]  WITNESS=[$KAYFABE_DIRTY_GATE_WITNESS]"
echo "================================================================================"
echo "    ⊘ THE ARMING AS THE DEVICE SAW IT — a script exporting a variable is not a device"
echo "      reading one, and this rung's whole subject is work that did or did not happen."
echo "      gate= on the publication line, and gate= on the EXEC-WITNESS line:"
grep -oE 'VAS-PUBLISH token=[^ ]+ arm=[a-z]+ gate=(on|off)' "$Q" 2>/dev/null | tail -1 | sed 's/^/      /'
echo "      EXEC-WITNESS gate readings, every distinct value:"
grep -oE 'EXEC-WITNESS (ARMED|⊘SKIPPED|DISARMED)[^|]*' "$Q" 2>/dev/null \
  | sed -E 's/resident=[0-9]+ by-executor=[0-9]+ refused-at-cap=[0-9]+ exec_writes=[0-9]+/<counts>/' \
  | sort | uniq -c | sort -rn | head -6 | sed 's/^/      /'
echo "      ⊘ EXEC-WITNESS lines total = [$(grep -c 'EXEC-WITNESS' "$Q" 2>/dev/null)] — 0 means the"
echo "        pass never printed, so every zero below is VACUOUS rather than clean."

echo ""
echo "=== ★★★★★ THE FIRE/SKIP RATIO — the diagnostic pre-registered outcome (B) turns on"
echo "    ⊘ A gate that fires on every doorbell and a gate that is working produce the SAME"
echo "      trap_ms if the cost was never where it was thought to be. Only this tells them"
echo "      apart. Last emission = the whole-boot running total."
grep -o 'DIRTY-GATE .*' "$Q" 2>/dev/null | tail -1 | sed 's/^/      /'
echo "    --- per-doorbell counts, every distinct reading:"
grep -oE 'this_doorbell\[fired=[0-9]+ skipped=[0-9]+\]' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -8 | sed 's/^/      /'
echo "    --- ⊘SKIPPED rows in the publication census (per VAS, per doorbell):"
echo "        [$(grep -o '⊘SKIPPED(w318 dirty gate' "$Q" 2>/dev/null | wc -l)]"

echo ""
echo "=== ★★★★★ THE PUBLICATION WALL — before/after is READ HERE, not inferred"
echo "    ⊘ Every distinct (published, refused, wall) triple. On the OFF arm w315 measured a"
echo "      single value repeated on every doorbell: published=0 refused=8 in ~43 ms."
grep -oE 'published=[0-9]+ refused=[0-9]+ in [0-9]+ ms' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -10 | sed 's/^/      /'

echo ""
echo "=== ★★★ THE DECODE — did gating the WITNESS quiet it, and did it change what it BOUND?"
echo "    ⊘ `bound=` is the correctness-relevant half. A drop in drained/latched with `bound`"
echo "      unchanged is the intended outcome; a drop in `bound` is outcome (C)."
grep -oE 'PT-DECODE drained=[0-9]+ latched=[0-9]+ [^→]*→ bound=[0-9]+ unchanged=[0-9]+ repointed=[0-9]+ unbound=[0-9]+' "$Q" 2>/dev/null \
  | sed -E 's/requeued=[0-9]+ rounds=[0-9]+ //' | sort | uniq -c | sort -rn | head -8 | sed 's/^/      /'
echo "    --- PT-SWEEP, which the decode's dirty bit arms:"
grep -oE 'PT-SWEEP tasks=[0-9]+ skipped=[0-9]+ ran=[0-9]+[^→]*→ bound=[0-9]+' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -6 | sed 's/^/      /'

echo ""
echo "=== ⚠ THE FAULT FINGERPRINT — reported whatever the timing says (w318 brief, outcome C)"
echo "    ⊘ w314 measured a ~20 % false-negative rate on a SINGLE-boot cup3 grade on these"
echo "      boxes, and the reds are identical field for field. If this gate moves that rate in"
echo "      EITHER direction that is a headline finding, so every red is printed here."
echo "    Xid lines in the host dmesg delta:"
grep -oE 'Xid[^,]*,[^,]*' /workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log 2>/dev/null | sort | uniq -c | head -6 | sed 's/^/      /'
echo "    [$(grep -c Xid /workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log 2>/dev/null)] Xid line(s); ⊘ 0 with a 0-byte delta log is the normal green"
} >> "$OUT" 2>&1

tail -120 "$OUT"
exit $IRC
