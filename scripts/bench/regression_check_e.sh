#!/usr/bin/env bash
# ★★★★★ CRITERION (E) — THE ADDRESS-PLANE REGRESSION CHECK. Rewritten at w304.
#
#   usage: regression_check_e.sh <qemu.log> <hostdmesg.log>
#   exit:  0 = no regression detected · 1 = REGRESSION · 2 = UNMEASURED (an input is missing)
#
# ## ⊘⊘ WHY IT WAS REWRITTEN: THE OLD (E) WOULD HAVE CALLED A REAL GREEN A REGRESSION
#
#   w297's (E) read, verbatim:
#       "(E) REGRESSION on what cup2 established: `Xid` != 0, or `host_rows` != 18295/18309."
#
#   `[measured 2026-08-14, w298, boot `w298ptsweep`, traces/w298_ablation/w298_summary.txt]`
#   `KAYFABE_PT_SWEEP=off` returned **`^CUP3_VAL=43`** — the milestone value, ladder 8/8,
#   `Xid=0` — with **`host_rows = ⊘ABSENT-UNMEASURED`**: not a different number, *no
#   `HOST-PUBLISHED` line at all* (`grep -c HOST-PUBLISHED` = **0**, against 229 on the
#   baseline). ⇒ `host_rows=18295/18309` is a CONSEQUENCE of the framebuffer page-table sweep,
#   not a PRECONDITION of the pass, and the criterion as written **fails on success**.
#
#   ★ A criterion that cannot fail is worse than one that is absent; a criterion that fails on
#     success is worse still — it teaches the reader to override it, and then it is absent
#     anyway, with a comment.
#
# ## ★★★ WHAT REPLACES IT — three clauses, each read from the DEVICE'S OWN EMISSIONS
#
#   (E1) `Xid` = 0 in the host dmesg delta. ⊘ Sound and kept unchanged: the host driver
#        reporting a GPU fault is a regression under every arming.
#   (E2) THE WHOLE-VAS DRAIN RAN AND PINNED. `★DRAINED` rows >= 1 and the summed `pinned=`
#        >= 1. This is the half that actually carries publication — w298 arm 13 (`bothoff`:
#        sweep off PLUS publish off) faults the same way as publish-off alone, so publication
#        is load-bearing ON ITS OWN rather than cleaning up after the sweep.
#   (E3) THE PUBLICATION PASS'S OWN INVARIANTS. `NOT ARMABLE` = 0, `host_isolates=NO` = 0,
#        `sum_ok=false` = 0. These are already declared MUST-be-0 by `w290p_run.sh` and were
#        printed and never graded. ★ This is the clause that can fail ON A GREEN, which is
#        what (E) exists for.
#
# ## ⊘ WHY (E2)'s FLOOR IS `>= 1` AND NOT A NUMBER
#
#   Pinning a specific count is the same mistake as `host_rows=18295/18309`: it grades a
#   CONSEQUENCE of one workload's size. A larger cup would pin more and a smaller one fewer,
#   and either would read as a regression. The floor asks the question that is actually a
#   precondition — *did the pass run at all* — and the count is PRINTED beside it so drift is
#   visible without being graded.
#   ⚠ ON A RUN THAT DID NOT REACH 43 (E2) IS REDUNDANT, not wrong: a run that stops early
#   emits fewer drains because there were fewer doorbells. (E) firing on an already-red run
#   costs nothing; (E) firing on a green is the failure mode this rewrite removes.
#
# ## ★ WHAT IS DELIBERATELY *NOT* GRADED
#
#   `host_rows` is PRINTED, in full, and explicitly not graded, with the reason inline — so
#   the next reader does not re-derive the deleted criterion from the number they can see.
#
# ## ★ KNOWN-POSITIVES (a criterion nobody watched fail is a wish)
#
#   `w304_e_selftest.sh` drives this file over (a) crafted fixtures, one per clause, and
#   (b) all thirteen recorded `w298` boots, and asserts the verdict for each.
set -uo pipefail

Q=${1:-}
D=${2:-}
[ -n "$Q" ] && [ -n "$D" ] || { echo "usage: $0 <qemu.log> <hostdmesg.log>" >&2; exit 64; }

VERDICT=0   # 0 pass · 1 regression · 2 unmeasured
note() { printf '    %s\n' "$*"; }

echo "=== (E) ADDRESS-PLANE REGRESSION CHECK  [w304 rewrite]"

# --- (E1) -------------------------------------------------------------------------------
# ⚠ `run_<tag>_hostdmesg.log` is a per-boot DELTA: 0 bytes is the NORMAL GREEN. Existence is
#   its own test, before any count, so "0 Xid in a file that exists" and "no file at all" are
#   different words — `grep -c` prints 0 AND exits 1, so a `||` fallback fires on both.
if [ -e "$D" ]; then
  XID=$(grep -c Xid "$D" 2>/dev/null || true)
  if [ "$XID" -eq 0 ]; then
    note "(E1) Xid = 0                          ✔ PASS   (delta file exists, $(stat -c%s "$D") bytes)"
  else
    note "(E1) Xid = $XID                          ✘ REGRESSION"
    grep -E 'Xid' "$D" 2>/dev/null | head -4 | sed 's/^/           /'
    VERDICT=1
  fi
else
  note "(E1) ⊘ NO HOST DMESG FILE AT ALL — UNMEASURED. ⊘ NOT zero."
  [ "$VERDICT" -eq 1 ] || VERDICT=2
fi

if [ ! -e "$Q" ]; then
  note "(E2)(E3) ⊘ NO QEMU LOG — UNMEASURED. ⊘ NOT zero."
  [ "$VERDICT" -eq 1 ] || VERDICT=2
  echo "=== (E) VERDICT = $VERDICT (0 pass · 1 REGRESSION · 2 unmeasured)"
  exit "$VERDICT"
fi

# --- (E2) -------------------------------------------------------------------------------
DROWS=$(grep -c '★DRAINED' "$Q" 2>/dev/null || true)
PSUM=$(grep -o "★DRAINED(this doorbell's VAS) asked=[0-9]* pinned=[0-9]*" "$Q" 2>/dev/null \
        | grep -oE 'pinned=[0-9]+' | cut -d= -f2 | paste -sd+ | bc 2>/dev/null)
PSUM=${PSUM:-0}
# ⚠ NUMERIC sort. w298 published `4 of 13348` as a peak by sorting these lexicographically
#   and had to withdraw a mechanism claim built on it.
PMAX=$(grep -o "★DRAINED(this doorbell's VAS) asked=[0-9]* pinned=[0-9]*" "$Q" 2>/dev/null \
        | grep -oE 'pinned=[0-9]+' | cut -d= -f2 | sort -n | tail -1)
if [ "$DROWS" -ge 1 ] && [ "$PSUM" -ge 1 ]; then
  note "(E2) drain ran: ★DRAINED rows=$DROWS  Σpinned=$PSUM  max_single=${PMAX:-0}   ✔ PASS"
else
  note "(E2) drain DID NOT RUN OR PINNED NOTHING: ★DRAINED rows=$DROWS  Σpinned=$PSUM   ✘ REGRESSION"
  note "     ⊘ the whole-VAS publication is the half that carries the address plane (w298 arm 13)."
  VERDICT=1
fi

# --- (E3) -------------------------------------------------------------------------------
NARM=$(grep -c 'VAS-PUBLISH.*NOT ARMABLE' "$Q" 2>/dev/null || true)
NISO=$(grep -c 'VAS-PUBLISH.*host_isolates=NO' "$Q" 2>/dev/null || true)
NSUM=$(grep -o 'sum_ok=false' "$Q" 2>/dev/null | wc -l)
if [ "$NARM" -eq 0 ] && [ "$NISO" -eq 0 ] && [ "$NSUM" -eq 0 ]; then
  note "(E3) pass invariants: NOT ARMABLE=$NARM  host_isolates=NO=$NISO  sum_ok=false=$NSUM   ✔ PASS"
else
  note "(E3) pass invariants VIOLATED: NOT ARMABLE=$NARM  host_isolates=NO=$NISO  sum_ok=false=$NSUM   ✘ REGRESSION"
  note "     ★ this clause can fire on a GREEN run — that is what (E) is for."
  VERDICT=1
fi

# --- INFORMATIONAL, NEVER GRADED ---------------------------------------------------------
HPUB=$(grep -c 'HOST-PUBLISHED' "$Q" 2>/dev/null || true)
note "--- host_rows: ⊘ REPORTED, NEVER GRADED. w298/ptsweep returned ^CUP3_VAL=43 with"
note "    ZERO HOST-PUBLISHED lines, so any host_rows assertion fails on a correct result."
note "    HOST-PUBLISHED lines = $HPUB   (0 means the line never printed — unmeasured, not zero)"
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null \
  | sort -t= -k2 -n | uniq | tail -6 | sed 's/^/        /'

echo "=== (E) VERDICT = $VERDICT (0 pass · 1 REGRESSION · 2 unmeasured)"
exit "$VERDICT"
