#!/usr/bin/env bash
# ★★★★★ w319 — ATTRIBUTE A BOOT: is a red YOURS, or the pre-existing drain truncation?
#
#   usage: w319_attribute.sh <tag>                      (reads /workspace/bench/run_<tag>_*)
#          w319_attribute.sh -q <qemu.log> -d <hostdmesg.log> [-v <CUP3_VAL>]
#          w319_attribute.sh --selftest                 (offline, no GPU, no bench — RUN FIRST)
#
# ## Why this exists
#
# `[measured w319]` `^CUP3_VAL != 43` on this box has **at least three** distinct causes, and
# grading on that one string cannot tell them apart. That is what made every rung pay a 4-boot
# tax plus a same-hour control. The drain's own completeness clause is a **per-boot
# deterministic** observable of the same event, and it is already printed on every boot — so
# one boot can grade, if you read the right line.
#
# ## The verdicts
#
#   0 GREEN / NOT-THIS-DEFECT   no fault, or the drain COMPLETED (`complete=true`,
#                               `pinned == asked`). ⇒ a fault here is YOURS.
#   1 PRE-EXISTING              drain TRUNCATED and the faulting VA is ABOVE `last_pinned_va`.
#                               ⇒ the w319 defect fired. Not yours.
#   2 UNMEASURED                the clause or the log is missing. ⊘ NOT a pass, NOT a zero.
#   3 OTHER-INTERMITTENT        RED with **no host Xid at all** (the `cuInit -> 999` mode).
#   4 TRUNCATED-BUT-BELOW       drain truncated, but the fault VA is at/below `last_pinned_va`,
#                               so the truncation does not explain it. ⇒ probably YOURS.
#
# ## ⊘⊘ TWO TRAPS, BOTH PAID FOR — one of them by THIS SCRIPT, on its first real input
#
# 1. **`budget_hit` ALONE IS NOT THE DISCRIMINATOR.** `[measured w314]` boot `br4` HIT the
#    budget and came back GREEN, because it stopped ONE ROW short — past the page that
#    matters. The discriminator is **`last_pinned_va` versus the faulting VA**, never the flag.
#
# 2. ★★★ **THIS SCRIPT'S OWN FIXTURES PASSED 5/5 WHILE IT WAS BROKEN ON EVERY REAL LOG.**
#    `[measured w319]` the clause was extracted with `DRAIN\[...[^]]*`, which stops at the
#    FIRST `]`. Production lines carry a NESTED bracket — `W319KNOB[budget_ms=… row_limit=…]`
#    — *before* `complete=`, so `complete=` was **never captured** and read as empty on all
#    four real boots, while the selftest was green because the fixtures had no nested bracket.
#    ⇒ **a known-positive that does not match the PRODUCTION SHAPE proves only that the
#    matcher matches the fixture.** The fixtures below now carry the nested bracket verbatim,
#    and one is copied byte-for-byte from a real boot.
set -uo pipefail

# ★ Bounded span rather than a bracket class, so a NESTED `[...]` cannot terminate the match.
CLAUSE_RE='DRAIN\[visited=true.\{0,300\}complete=[a-z]*'

verdict() {
  local Q=$1 D=$2 VAL=${3:-}
  local CLAUSE COMPLETE ASKED PINNED LPV FAULT
  [ -r "$Q" ] || { echo "VERDICT=2 UNMEASURED — no qemu log at $Q"; return 2; }
  CLAUSE=$(grep -ao "$CLAUSE_RE" "$Q" 2>/dev/null | head -1)
  LPV=$(grep -ao 'last_pinned_va=0x[0-9a-f]*' "$Q" 2>/dev/null | head -1 | cut -d= -f2)
  if [ -n "$CLAUSE" ]; then
    COMPLETE=$(printf '%s' "$CLAUSE" | grep -o 'complete=[a-z]*' | cut -d= -f2)
    ASKED=$(printf '%s' "$CLAUSE" | grep -o 'asked=[0-9]*' | head -1 | cut -d= -f2)
    PINNED=$(printf '%s' "$CLAUSE" | grep -o 'pinned=[0-9]*' | head -1 | cut -d= -f2)
    echo "DRAIN: complete=${COMPLETE:-⊘} asked=${ASKED:-⊘} pinned=${PINNED:-⊘} last_pinned_va=${LPV:-⊘UNMEASURED}"
  else
    echo "DRAIN: ⊘ NO CLAUSE — UNMEASURED, ⊘ not 'complete'"
  fi

  # ⊘ The host Xid lives ONLY in the host dmesg delta — never in the guest's, never in QEMU's.
  [ -r "$D" ] && FAULT=$(grep -ao 'faulted @ 0x[0-9a-f_]*' "$D" 2>/dev/null | head -1 | sed 's/faulted @ //; s/_//g')

  if [ -z "${FAULT:-}" ]; then
    # ★★★ A boot with no fault is only interesting if it FAILED. Reading "no Xid" as a defect
    #     class would grade every GREEN boot as the other intermittent — which this script did
    #     on its first real input (`w319xon2`, `CUP3_VAL=43`, called OTHER-INTERMITTENT).
    if [ "${VAL:-}" = "43" ]; then
      echo "VERDICT=0 GREEN — CUP3_VAL=43 and no host Xid. Nothing to attribute."
      return 0
    fi
    if [ -z "${VAL:-}" ]; then
      echo "VERDICT=2 UNMEASURED — no host Xid, and no CUP3_VAL to say whether that is a"
      echo "  green boot or a failure with no fault. ⊘ Pass -v, or a tag, so this can decide."
      return 2
    fi
    echo "VERDICT=3 OTHER-INTERMITTENT — RED (CUP3_VAL=$VAL) with NO host Xid at all."
    echo "  Neither this defect nor a publication miss; see RESULTS.md §3 (cuInit -> 999)."
    return 3
  fi
  echo "FAULT: $FAULT"

  [ -n "$CLAUSE" ] || { echo "VERDICT=2 UNMEASURED — a fault, but no DRAIN clause to attribute it against."; return 2; }
  if [ "$COMPLETE" = "true" ] && [ "$ASKED" = "$PINNED" ]; then
    echo "VERDICT=0 NOT-THIS-DEFECT — the drain completed, every asked row was pinned."
    echo "  ⇒ the fault is NOT explained by the w319 truncation. It is YOURS."
    return 0
  fi
  [ -n "${LPV:-}" ] || { echo "VERDICT=2 UNMEASURED — truncated, but no last_pinned_va to compare."; return 2; }
  if [ "$((FAULT))" -gt "$((LPV))" ]; then
    echo "VERDICT=1 PRE-EXISTING — drain TRUNCATED and the fault VA is ABOVE last_pinned_va."
    echo "  ⇒ the w319 drain-budget truncation. NOT yours. Discard or attribute here."
    return 1
  fi
  echo "VERDICT=4 TRUNCATED-BUT-BELOW — the drain was short, but the fault VA is at or below"
  echo "  last_pinned_va, so the truncation does NOT explain it. ⇒ probably YOURS."
  return 4
}

selftest() {
  local T rc fail=0 i
  T=$(mktemp -d)
  # ★★★ EVERY fixture carries the NESTED `W319KNOB[...]` bracket, because that is the
  #     production shape — see trap 2 in the header.
  # 1) COMPLETE drain, green ⇒ 0
  printf 'DRAIN[visited=true asked=13313 pinned=13313 refused=0 DRAIN_MS=2672 W319KNOB[budget_ms=3000 row_limit=65536] complete=true ] x last_pinned_va=0x2047ff000\n' >"$T/q1"
  : >"$T/d1"; echo 43 >"$T/v1"
  # 2) TRUNCATED + fault ABOVE ⇒ 1   (byte-for-byte the shape of real boot w319r1)
  printf 'kayfabe: VAS-PUBLISH token=0x00020013 arm=drain PINRATE(x) -> pinned=11800 refused=0 in 2693 ms, per_row=228 us/row, degrade[a] SEMAPIN[b] DRAIN[visited=true asked=11800 pinned=11800 refused=0 DRAIN_MS=2693 W319KNOB[budget_ms=3000 row_limit=11800] complete=false ROW CAP HIT ] SCOPE[c] over 1 VAS row(s) [proc=2 last_pinned_va=0x203217000 d]\n' >"$T/q2"
  printf 'NVRM: Xid (PCI:0000:00:07): 31, ... faulted @ 0x2_0440f000. FAULT_PDE ACCESS_TYPE_VIRT_WRITE\n' >"$T/d2"; echo NO_KERNEL_LINE >"$T/v2"
  # 3) missing clause + fault ⇒ 2
  printf 'nothing useful here\n' >"$T/q3"; cp "$T/d2" "$T/d3"; echo NO_KERNEL_LINE >"$T/v3"
  # 4) RED, no Xid ⇒ 3
  cp "$T/q1" "$T/q4"; : >"$T/d4"; echo NO_KERNEL_LINE >"$T/v4"
  # 5) ★ THE br4 TRAP: budget HIT, one row short, fault BELOW last_pinned_va ⇒ 4, never 1
  printf 'DRAIN[visited=true asked=13313 pinned=13312 refused=0 DRAIN_MS=3000 W319KNOB[budget_ms=3000 row_limit=65536] complete=false WALL BUDGET HIT ] x last_pinned_va=0x2047fe000\n' >"$T/q5"
  cp "$T/d2" "$T/d5"; echo NO_KERNEL_LINE >"$T/v5"
  # 6) ★ COMPLETE drain WITH a fault ⇒ 0 — the clause w318 actually needs
  cp "$T/q1" "$T/q6"; cp "$T/d2" "$T/d6"; echo NO_KERNEL_LINE >"$T/v6"
  local -a want=(0 1 2 3 4 0)
  for i in 1 2 3 4 5 6; do
    ( verdict "$T/q$i" "$T/d$i" "$(cat "$T/v$i")" >/dev/null 2>&1 ); rc=$?
    if [ "$rc" != "${want[$((i-1))]}" ]; then
      echo "  ✘ fixture $i: got $rc, want ${want[$((i-1))]}"; fail=1
    else echo "  ✔ fixture $i ⇒ $rc"; fi
  done
  # ★★ AND A NEGATIVE CONTROL ON THE MATCHER ITSELF: the nested-bracket clause MUST yield a
  #    non-empty `complete=`. This is the assertion whose absence let the broken version pass.
  local c
  c=$(grep -ao "$CLAUSE_RE" "$T/q2" | head -1 | grep -o 'complete=[a-z]*')
  if [ "$c" = "complete=false" ]; then echo "  ✔ nested-bracket extraction ⇒ $c"
  else echo "  ✘ nested-bracket extraction got [$c], want complete=false"; fail=1; fi
  rm -rf "$T"
  [ "$fail" = 0 ] && echo "SELFTEST PASS (6 fixtures + 1 matcher assertion)" || echo "★ SELFTEST FAIL"
  return "$fail"
}

[ "${1:-}" = "--selftest" ] && { selftest; exit $?; }

Q=""; D=""; VAL=""
if [ "${1:-}" = "-q" ]; then
  while [ $# -gt 0 ]; do case $1 in -q) Q=$2; shift 2;; -d) D=$2; shift 2;; -v) VAL=$2; shift 2;; *) shift;; esac; done
else
  TAG=${1:?usage: w319_attribute.sh <tag> | -q <qemu> -d <hostdmesg> [-v VAL] | --selftest}
  B=${W319_BENCH:-/workspace/bench}
  Q=$B/run_${TAG}_qemu.log; D=$B/run_${TAG}_hostdmesg.log
  VAL=$(grep -aoE '^CUP3_VAL=[A-Za-z0-9_]+' "$B/run_${TAG}_probe.log" 2>/dev/null | tail -1 | cut -d= -f2)
fi
echo "CUP3_VAL=${VAL:-⊘UNMEASURED}"
verdict "$Q" "$D" "$VAL"; exit $?
