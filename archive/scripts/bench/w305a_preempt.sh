#!/usr/bin/env bash
# ★★★★★ w305 ITEM A — THE PREEMPT DESTROY PATH, OBSERVED FOR THE FIRST TIME.
#
# `w303` landed `ObjectPolicy::respond_preempt` for `NVA06C_CTRL_CMD_PREEMPT` (`0xa06c0105`):
# `NV_OK` + echo **only** when the named channel group provably has no host twin, and
# `NV_ERR_INVALID_STATE` (`0x40`) otherwise. It replaced an unconditional `NV_OK`-that-did-
# nothing. Its own report named the gap:
#
#   "all ~15 committed boots that refused it were boots where `cuCtxCreate` had already
#    FAILED. A refusal on the successful destroy path has never been observed."
#
# and its own measurement named the cause: `scripts/bench/cup3.c` calls `cuCtxCreate` and
# **never `cuCtxDestroy`**, so `0xa06c0105` appears nowhere in either crossing boot.
#
# ⇒ This rung runs `cup3d.c` — cup3 byte-for-byte, plus an explicit teardown ending in
#   `cuCtxDestroy` — on the arming that crossed at `CUP3_VAL=43`. Nothing else moves.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none reads as the favourable one
#
#   (a) THE CONTROL ARRIVES AND IS ANSWERED on a successful destroy. Report WHICH branch
#       (`NV_OK`-no-twin vs `INVALID_STATE`) and whether the guest ACCEPTED it
#       (`CUP3D_CTXDESTROY_RC=0`). ★ This is the measurement the rung exists for.
#   (b) IT NEVER ARRIVES even on a successful destroy ⇒ the whole id is off the real path and
#       the w303 change is INERT-BUT-CORRECT. ⊘ **A FULL RESULT — say so, do not read it as a
#       failure to measure.** ⚠ Only if the KNOWN-POSITIVE below fires; otherwise the zero is
#       VACUOUS, not negative.
#   (c) IT ARRIVES AND THE GUEST FAILS where it did not before ⇒ a REGRESSION w303 introduced.
#       ★★★★★ The most important outcome to report, and it is reported LOUDLY.
#   (d) cup3d does not reach its own teardown at all ⇒ UNMEASURED. The stage ladder says where
#       it stopped; `CUP3_VAL` says whether the compute leg still crossed.
#
# ## ★★★ THE KNOWN-POSITIVE — a census zero needs one, and this one is measured
#
# `[measured 2026-08-14, w303]` the sibling group controls `0xa06c010a` (x5), `0xa06c0101`
# (x3) and `0xa06c0103` (x1) ARE named in both crossing boots. ⇒ if THEY appear here and
# `0xa06c0105` does not, the zero is a **measurement**. If none of them appears either, the
# census is blind and outcome (b) may not be claimed. ⊘ Printed before the verdict it governs.
set -uo pipefail

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w305}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w305}
export KAYFABE_TAG=${KAYFABE_TAG:-w305apreempt}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/cup3_hook.sh"
# ★ THE ONLY KNOB THAT MOVES: the workload source. cup3_hook.sh already reads it, so the
#   PROVEN hook grades this run and no second copy of it can drift.
export KAYFABE_CUP3_SRC="$REPO/scripts/bench/cup3d.c"
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

rm -f /workspace/bench/qemu-build/qemu-system-x86_64

# w290p_run.sh does the stamp gate, the boot and the address-plane grading. Invoked, never
# copied. `drain` is the byte-identical w297/w298 arm that produced CUP3_VAL=43.
"$REPO/scripts/bench/w290p_run.sh" "${W298_ARM:-drain}"
BRC=$?

OUT=/workspace/${KAYFABE_TAG}.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log

{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W305 ITEM A GRADING — THE PREEMPT DESTROY PATH  inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
echo ""
echo "=== ⊘ PRECONDITION 0 — the artefacts exist at all (zero bytes is not 'not yet')"
echo "    qemu log  = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)] bytes"
echo "    probe log = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)] bytes"
echo ""
echo "=== ⊘ PRECONDITION 1 — DID THE WORKLOAD REACH ITS TEARDOWN? If not, everything below"
echo "===    is UNMEASURED and outcome (d) is the result, not (b)."
echo "    ^CUP3_VAL          = [$(grep -oE '^CUP3_VAL=[0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "    ^CUP3_RC           = [$(grep -oE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "    TEARDOWN BEGIN     = [$(grep -c 'TEARDOWN BEGIN' "$P" 2>/dev/null)]  (0 ⇒ the destroy path was NEVER ENTERED ⇒ outcome (d))"
echo "    TEARDOWN DONE      = [$(grep -c 'TEARDOWN DONE' "$P" 2>/dev/null)]"
echo "    the teardown's own lines, verbatim:"
grep -hE 'DESTROY_[A-Z_]+_RC=|CUP3D_CTXDESTROY_(RC|STR)=' "$P" 2>/dev/null | sed 's/^/      /'
echo ""
echo "=== ★★★ THE KNOWN-POSITIVE — is the control census LIVE on this boot at all?"
echo "===     ⊘ Without this, a zero for 0xa06c0105 is VACUOUS and outcome (b) is NOT claimable."
for sib in 0xa06c010a 0xa06c0101 0xa06c0103 0xa06c0102; do
  echo "    sibling $sib mentions = [$(grep -c "$sib" "$Q" 2>/dev/null)]"
done
echo "    ⊘ w303 measured 0xa06c010a x5, 0xa06c0101 x3, 0xa06c0103 x1 in BOTH crossing boots."
echo "      If all four read 0 here, the census is BLIND and (b) may not be claimed."
echo ""
echo "=== ★★★★★ THE RUNG — DID 0xa06c0105 ARRIVE, AND WHICH BRANCH ANSWERED IT?"
NPRE=$(grep -c 'kayfabe: PREEMPT client=' "$Q" 2>/dev/null)
echo "    TOTAL 'kayfabe: PREEMPT client=' emissions = [$NPRE]"
echo "    ⊘ 0 here with a LIVE census above ⇒ outcome (b): the id is off the real path."
echo ""
echo "--- branch A: ★ NV_OK because there is provably NO HOST TWIN (the ack is TRUE)"
echo "    count = [$(grep -c '★ NV_OK, AND IT IS TRUE' "$Q" 2>/dev/null)]"
echo "--- branch B: ⊘ UNPERFORMED — a LIVE host twin exists, answered 0x40 NV_ERR_INVALID_STATE"
echo "    count = [$(grep -c '⊘ UNPERFORMED host_twins=' "$Q" 2>/dev/null)]"
echo "--- branch C: ⊘ UNROUTABLE — the group did not resolve, answered 0x40"
echo "    count = [$(grep -c '⊘ UNROUTABLE' "$Q" 2>/dev/null)]"
echo "--- branch D: ⊘ REFUSED BadParams — a shape refusal, answered 0x47"
echo "    count = [$(grep -c '⊘ REFUSED BadParams' "$Q" 2>/dev/null)]"
echo ""
echo "=== ★★★★★ EVERY PREEMPT EMISSION, VERBATIM AND UNTRUNCATED — this is the evidence"
grep -a 'kayfabe: PREEMPT client=' "$Q" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo "      ⊘ if nothing printed above, the id never reached respond_preempt on this boot."
echo ""
echo "=== ★★★★★ (c) THE REGRESSION CHECK — DID THE GUEST ACCEPT OUR ANSWER?"
# ⊘⊘⊘ **w305 DEFECT, FOUND BY THIS RUNG'S OWN FIRST RUN, AND IT IS THE ANCHOR TRAP INVERTED.**
#
#   The first version of this clause read `grep -oE '^CUP3D_CTXDESTROY_RC=…'`. `cup3_hook.sh`
#   prints the workload's output through `sed 's/^/    /'`, so EVERY line of it is indented
#   four spaces and `^` can never match. ⇒ on the `w305apreempt` boot this field printed
#   **"⊘ ABSENT — cuCtxDestroy never returned a line. UNMEASURED, NOT 0"** while the verbatim
#   teardown block SIX LINES ABOVE printed `CUP3D_CTXDESTROY_RC=0`. The same log said both.
#
#   ★★★ This tree's standing rule is *"anchor, because the unanchored read has printed the
#   headline success value on a failing arm"*. Here the anchor produced the MIRROR failure: a
#   FALSE "UNMEASURED" on a field that WAS measured — and "unmeasured" is the reading this
#   repo treats as safe, so it would have been believed. ⇒ **An anchor is only correct against
#   the layout the PRODUCER actually emits.** cup3's own `^CUP3_VAL=` lines are emitted by the
#   HOOK at column 0; the WORKLOAD's lines come through the indenting `sed`. Two different
#   producers in one file, and one anchor cannot be right for both.
#
#   ★ Anchored to the line START ALLOWING LEADING WHITESPACE, which is what the producer emits,
#     and the raw unanchored read is printed BESIDE it so a reader can see what it would say.
CTXD=$(grep -oE '^[[:space:]]*CUP3D_CTXDESTROY_RC=-?[0-9]+' "$P" 2>/dev/null | tr -d '[:space:]' | tail -1)
echo "    CUP3D_CTXDESTROY_RC (start-of-line, leading space allowed) = [${CTXD:-⊘ ABSENT — cuCtxDestroy never returned a line. UNMEASURED, NOT 0}]"
echo "    CUP3D_CTXDESTROY_STR = [$(grep -oE 'CUP3D_CTXDESTROY_STR=.*' "$P" 2>/dev/null | tail -1)]"
echo "    ⊘ STRICT ^ read, for contrast (empty is EXPECTED — the hook indents the workload): [$(grep -oE '^CUP3D_CTXDESTROY_RC=-?[0-9]+' "$P" 2>/dev/null | tail -1)]"
case "$CTXD" in
  CUP3D_CTXDESTROY_RC=0) echo "    ★ ACCEPTED — cuCtxDestroy returned CUDA_SUCCESS. No regression on this path." ;;
  "")                    echo "    ⊘ UNMEASURED — no line. Read the stage ladder; this is NOT an acceptance." ;;
  *)                     echo "    ★★★★★ (c) THE GUEST FAILED AT cuCtxDestroy WITH $CTXD — REPORT THIS LOUDLY."
                         echo "        ⚠ If branch B or C fired above, w303's refusal is the PROXIMATE CAUSE and"
                         echo "          this rung has found a REGRESSION w303 introduced." ;;
esac
echo ""
echo "=== ★ THE STAGE LADDER (cup3d keeps cup3's markers byte-for-byte, so this is comparable)"
grep -E '^    [✔✘] ' "$P" 2>/dev/null | sed 's/^/    /'
echo "=== ★ cup3d's own FAIL line, if it named one"
grep -hE '^ *FAIL ' "$P" 2>/dev/null | sed 's/^/    /'
echo ""
echo "=== ⚠ REGRESSION CHECK ON WHAT cup3 ESTABLISHED — printed EVEN ON A GREEN"
if [ -e "$D" ]; then
  echo "    Xid count = [$(grep -c Xid "$D")]   (host dmesg DELTA exists, $(stat -c%s "$D") bytes; 0 bytes IS the normal green)"
  grep -E 'Xid' "$D" 2>/dev/null | head -4 | sed 's/^/      /'
else
  echo "    Xid count = [⊘ NO HOST DMESG FILE AT ALL — UNMEASURED. ⊘ NOT zero.]"
fi
echo "    host_rows, every distinct reading:"
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
echo ""
echo "=== ★ THE UNSERVICED LEDGER — if the destroy path walled on a control we refuse"
grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -30 | sed 's/^/      /'
echo ""
echo "=== ★★ HARNESS SELF-CHECK — assert THIS block's own output exists"
echo "    w305a grading lines = [$(grep -c 'W305 ITEM A GRADING' "$OUT" 2>/dev/null)]  (MUST be >= 1)"
echo "=== W305A EXIT rc=$BRC at $(date -Is) ==="
} >>"$OUT" 2>&1

exit "$BRC"
