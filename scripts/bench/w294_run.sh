#!/usr/bin/env bash
# ★★★★★ w294 — SERVE THE CUDA PERF LIMIT PAIR (`0x00802009` + `0x00802004`).
#
#   $1 = cup2 | nvd     (⊘ REQUIRED — never defaulted. The two instruments answer DIFFERENT
#                        questions and a defaulted arm makes them indistinguishable in the
#                        log: `cup2` produces `^CUP2_RC=`, `nvd` produces the ioctl stream
#                        and NO `CUP2_RC` at all.)
#
# ⊘ RELAXATIONS CARRIED AND LABELLED, byte for byte from w290p/w292: KAYFABE_PT_SWEEP=on,
#   KAYFABE_OPERAND_JOIN=join, KAYFABE_FB_JOIN=shared, KAYFABE_VAS_PUBLISH=drain.
#   ★ A relaxed green is a MAP, not the milestone.
#
# ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none can be read as the
#     favourable one afterwards:
#   (A) `^CUP2_RC=` MOVES OFF 1 ⇒ the rung crossed. Name EVERY relaxation that was on.
#   (B) `^CUP2_RC=` STAYS 1 ⇒ EIGHTH necessary-not-sufficient. ★ This is a FULL RESULT and
#       the honest possibility; it has been the honest answer seven rungs running.
#       Report the NEXT WALL BY IDENTITY: control id, in-band status, psize vs dlen, our
#       refusal name, and whether the C and native serve it.
#   (C) `^CUP2_RC=` ABSENT ⇒ THE MEASUREMENT DID NOT HAPPEN. ⊘ It is NOT 0. The block below
#       prints that sentence rather than a number.
#   (D) THE PAIR IS NOT SERVED (`unserviced fn 76 cmd 0x0080200[49]` still > 0) ⇒ the build
#       did not do its job and every number after it is VACUOUS, not zero. Checked FIRST.
#   (E) REGRESSION on the address plane: `Xid` != 0, or `host_rows` != 18295/18309.
set -uo pipefail
ARM="${1:-}"
case "$ARM" in cup2|nvd) ;; *) echo "usage: $0 cup2|nvd" >&2; exit 64 ;; esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w294}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w294}
export KAYFABE_TAG=w294${ARM}
case "$ARM" in
  cup2) export POST_CAPTURE_HOOK="$REPO/scripts/bench/cup2_hook_gdbspin.sh" ;;
  nvd)  export POST_CAPTURE_HOOK="$REPO/scripts/bench/nvdiff_hook.sh"; export GQ_TIMEOUT=${GQ_TIMEOUT:-900} ;;
esac

# ★★★ DELETE BOTH BINARIES BEFORE THE BUILD. `[measured]` a stale musl client once exited 95
# and `[ -x ]` alone would have run it: "the file exists" is not "the file is this revision".
rm -f /workspace/bench/qemu-build/qemu-system-x86_64
rm -f "$REPO"/target/*/cup2 /workspace/bench/cup2 2>/dev/null

# w290p_run.sh does the stamp gate, the boot, and the address-plane grading, and it writes
# everything to /workspace/$KAYFABE_TAG.log. It is invoked rather than copied so the two
# rungs cannot drift.
"$REPO/scripts/bench/w290p_run.sh" drain
BRC=$?

OUT=/workspace/${KAYFABE_TAG}.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log

{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W294 GRADING — arm=$ARM  inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
echo ""
echo "=== (D) DID THE BUILD DO ITS JOB? — checked FIRST, because everything else is"
echo "===     VACUOUS if it did not. ⊘ These are the ids that ARRIVE; 0x00801909 never does."
for id in 0x00802009 0x00802004; do
  U=$(grep -c "unserviced fn 76 cmd $id" "$Q" 2>/dev/null || echo 0)
  S=$(grep -c "control $id result 0x00000000" "$Q" 2>/dev/null || echo 0)
  R=$(grep -oE "control $id result 0x[0-9a-f]+( x[0-9]+)?" "$Q" 2>/dev/null | sort | uniq -c | tr '\n' ' ')
  echo "    $id : unserviced=[$U] (MUST be 0)   served_ok=[$S]   every reading=[${R:-⊘ NONE — the id never reached us on this boot, which is UNMEASURED not served}]"
done
echo "    ⊘ 0x00801909 in our QEMU log = [$(grep -c '00801909' "$Q" 2>/dev/null || echo 0)] — MUST be 0 by construction:"
echo "      it is flags=0x118, not ROUTE_TO_PHYSICAL, so the guest's own kernel answers it."
echo ""
echo "=== ★ THE WHOLE UNSERVICED LEDGER — the next wall's candidates, by identity"
grep -ho "unserviced fn 76 cmd 0x[0-9a-f]*" "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -50 | sed 's/^/      /'
echo "      distinct ids = [$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)]"
echo ""
echo "=== ★★★★★ CUP2_RC — ANCHORED. Baseline 1. ⊘ The anchor trap has fired SEVEN rungs."
strict=$(grep -oE '^CUP2_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
echo "    ^ANCHORED  = [${strict:-⊘ NO LINE BEGINS WITH CUP2_RC= — THE MEASUREMENT DID NOT HAPPEN. ⊘ NOT 0}]"
echo "    UNANCHORED, for contrast = [$(grep -oh 'CUP2_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
echo "    ⊘ on arm=nvd there is NO cup2 and the absence above is CORRECT, not a failure."
echo ""
echo "=== (E) REGRESSION CHECK — today's values are Xid=0 and host_rows 18295/18309"
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
echo "    Xid count  = [$(grep -c Xid "$D" 2>/dev/null || echo '⊘ NO HOST DMESG — UNMEASURED')]"
grep -E 'Xid' "$D" 2>/dev/null | head -4 | sed 's/^/      /'
echo "    host_rows, every distinct reading:"
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
echo ""
echo "=== ⊘ EVERY RELAXATION THAT WAS ON — a relaxed green is a MAP, not the milestone"
for v in KAYFABE_PT_SWEEP KAYFABE_OPERAND_JOIN KAYFABE_FB_JOIN KAYFABE_VAS_PUBLISH \
         KAYFABE_GR_ROUTE KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF KAYFABE_GUEST_SEMA \
         KAYFABE_GUEST_OPERAND KAYFABE_ISOLATES KAYFABE_CE_EXECUTOR; do
  echo "    $v = [$(grep -oE "$v=[a-z]+" "$OUT" 2>/dev/null | tail -1)]"
done
grep -oE 'VAS-PUBLISH arm=[a-z]+ fb_join=[a-z]+ host_isolates=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/    /'
echo ""
echo "=== ★★ HARNESS SELF-CHECK — assert THIS block's own output exists"
echo "    w294 grading lines = [$(grep -c 'W294 GRADING' "$OUT" 2>/dev/null)]  (MUST be >= 1)"
echo "    qemu log bytes     = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)]"
echo "    probe log bytes    = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W294 EXIT rc=$BRC arm=$ARM at $(date -Is) ==="
} >>"$OUT" 2>&1

exit "$BRC"
