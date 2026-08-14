#!/usr/bin/env bash
# ★★★★★ w304 — THE KNOWN-POSITIVE HARNESS FOR CRITERION (E).
#
#   A criterion nobody has watched FAIL is a wish, and a criterion nobody has watched PASS on
#   the very run that broke its predecessor is untested where it matters most. This file does
#   both, offline, with no GPU — the shape `nvd_selftest.sh` already uses in the C tree.
#
#   PART A — one crafted fixture per clause. Each is a MINIMAL log that differs from the
#            passing fixture in exactly the string the clause reads, so a clause that fires
#            fires FOR ITS OWN REASON. ⚠ The passing fixture carries NO `host_rows` line at
#            all: it is the `w298ptsweep` shape, the real green the old criterion failed.
#   PART B — the new (E) and the OLD (E) run side by side over every recorded `w298` boot,
#            with each boot's headline `^CUP3_VAL`. This is where the defect is exhibited:
#            the old criterion must be seen calling a `43` a REGRESSION.
#
# usage: w304_e_selftest.sh [dir-with-recorded-w298-logs]
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E="$HERE/regression_check_e.sh"
W298DIR=${1:-/workspace/bench}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FAILS=0

# The OLD criterion, implemented faithfully so the comparison is not a paraphrase:
#   "(E) REGRESSION on what cup2 established: `Xid` != 0, or `host_rows` != 18295/18309."
old_e() { # old_e <qemu.log> <dmesg.log> -> prints PASS|REGRESSION
  local q=$1 d=$2
  local xid=0
  [ -e "$d" ] && xid=$(grep -c Xid "$d" 2>/dev/null || true)
  if [ "$xid" -ne 0 ]; then echo REGRESSION; return; fi
  if grep -q 'host_rows=18295 of 18309' "$q" 2>/dev/null; then echo PASS; else echo REGRESSION; fi
}

mk() { # mk <name> <drained?> <sumok_false?> <hostrows?>
  local n=$1 dr=$2 sf=$3 hr=$4
  : > "$TMP/$n.dmesg"
  {
    echo "kayfabe: VAS-PUBLISH token=0x7 arm=drain fb_join=shared host_isolates=yes"
    [ "$dr" = yes ] && echo "kayfabe:   [proc=2 pdb=0x201000 ★DRAINED(this doorbell's VAS) asked=13312 pinned=13312 refused=0 in 2789 ms last_pinned_va=0x2047ff000]"
    [ "$sf" = yes ] && echo "kayfabe: bucket identity sum_ok=false"
    [ "$hr" = yes ] && echo "kayfabe: HOST-PUBLISHED host_rows=18295 of 18309"
    echo "kayfabe: by engine: GrCompute=125 GrGraphics=0 Ce=355 unrouted=0"
  } > "$TMP/$n.qemu"
}

chk() { # chk <label> <expected-rc> <qemu> <dmesg>
  local label=$1 want=$2 q=$3 d=$4
  local out rc
  out=$("$E" "$q" "$d" 2>&1); rc=$?
  if [ "$rc" -eq "$want" ]; then
    printf '  ✔ %-42s rc=%s (expected %s)\n' "$label" "$rc" "$want"
  else
    printf '  ✘ %-42s rc=%s (EXPECTED %s)  ← SELFTEST FAILURE\n' "$label" "$rc" "$want"
    echo "$out" | sed 's/^/        /'
    FAILS=$((FAILS + 1))
  fi
  echo "$out" | grep -E '\(E[123]\)' | sed 's/^/        /'
}

echo "================================================================================"
echo "=== PART A — ONE CRAFTED FIXTURE PER CLAUSE  (offline, no GPU)"
echo "================================================================================"

# ★ THE PASSING FIXTURE IS THE `w298ptsweep` SHAPE: green, drain ran, and NO host_rows line.
mk pass yes no no
chk "PASS: drain ran, no host_rows at all" 0 "$TMP/pass.qemu" "$TMP/pass.dmesg"

# (E1) known-positive — an Xid in the delta.
mk e1 yes no yes
echo "[  123.456] NVRM: Xid (PCI:0000:00:07): 31, pid=2960505, faulted @ 0x70ab_a8e00000" > "$TMP/e1.dmesg"
chk "(E1) fires on an Xid" 1 "$TMP/e1.qemu" "$TMP/e1.dmesg"

# (E2) known-positive — the drain never ran. This is the `KAYFABE_VAS_PUBLISH=assert` shape.
mk e2 no no yes
chk "(E2) fires when the drain never ran" 1 "$TMP/e2.qemu" "$TMP/e2.dmesg"

# (E3) known-positive — a pass invariant violated ON AN OTHERWISE GREEN RUN.
mk e3 yes yes yes
chk "(E3) fires on sum_ok=false, everything else green" 1 "$TMP/e3.qemu" "$TMP/e3.dmesg"

# (E3) known-positive #2 — NOT ARMABLE.
mk e3b yes no yes
echo "kayfabe: VAS-PUBLISH token=0x7 ⊘ NOT ARMABLE" >> "$TMP/e3b.qemu"
chk "(E3) fires on NOT ARMABLE" 1 "$TMP/e3b.qemu" "$TMP/e3b.dmesg"

# UNMEASURED is its own state, distinct from PASS and from REGRESSION.
chk "missing inputs ⇒ UNMEASURED, not pass" 2 "$TMP/nope.qemu" "$TMP/nope.dmesg"

echo ""
echo "================================================================================"
echo "=== PART B — NEW (E) vs OLD (E) OVER EVERY RECORDED w298 BOOT"
echo "===   dir = $W298DIR"
echo "===   ★ the defect: the OLD criterion must be seen calling a ^CUP3_VAL=43 a REGRESSION."
echo "================================================================================"
printf '  %-18s %-22s %-11s %-11s %s\n' BOOT '^CUP3_VAL' 'OLD (E)' 'NEW (E)' NOTE
DISAGREE=0
# ⊘ Every recorded boot in the directory, w298 AND w304 — enumerated from the FILESYSTEM, not
#   from a hand-written list, so a boot that exists and was forgotten cannot be silently
#   skipped. A named-but-absent log prints "SKIPPED, not passed".
for q in "$W298DIR"/run_w[23]9[0-9]*_qemu.log "$W298DIR"/run_w30[0-9]*_qemu.log; do
  [ -e "$q" ] || continue
  t=$(basename "$q" _qemu.log); t=${t#run_}
  d="$W298DIR/run_${t}_hostdmesg.log"; p="$W298DIR/run_${t}_probe.log"
  val=$(grep -oE '^CUP3_VAL=[0-9]+' "$p" 2>/dev/null | tail -1); val=${val:-⊘ABSENT-UNMEASURED}
  o=$(old_e "$q" "$d")
  "$E" "$q" "$d" >/dev/null 2>&1; nrc=$?
  case $nrc in 0) n=PASS;; 1) n=REGRESSION;; *) n=UNMEASURED;; esac
  note=""
  if [ "$val" = "CUP3_VAL=43" ] && [ "$o" = REGRESSION ]; then
    note="★★★ OLD (E) FAILS A REAL GREEN"; DISAGREE=$((DISAGREE + 1))
  fi
  if [ "$val" = "CUP3_VAL=43" ] && [ "$n" != PASS ]; then
    note="$note  ✘✘ NEW (E) FAILS A GREEN — THE REWRITE IS WRONG"; FAILS=$((FAILS + 1))
  fi
  printf '  %-18s %-22s %-11s %-11s %s\n' "$t" "$val" "$o" "$n" "$note"
done

echo ""
echo "  ⇒ boots where the OLD criterion called a ^CUP3_VAL=43 a REGRESSION: $DISAGREE"
echo "    (MUST be >= 1, or the defect this rung fixes is not exhibited by this corpus)"
[ "$DISAGREE" -ge 1 ] || { echo "  ✘ the corpus does not exhibit the defect"; FAILS=$((FAILS + 1)); }
echo ""
if [ "$FAILS" -eq 0 ]; then
  echo "=== ★ SELFTEST OK — every clause watched firing, and no green graded as a regression."
else
  echo "=== ✘ SELFTEST FAILURES = $FAILS"
fi
exit "$FAILS"
