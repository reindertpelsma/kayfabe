#!/usr/bin/env bash
# ★★★★★ w297 — cup3. FIRST COMPUTE, on a stock guest driver, against a real GA106.
#
#   `cup2` crossed at `^CUP2_RC=0`, 2/2 (w294, `1c8e508`). ⊘ **`cup2` is a CE round-trip, not
#   compute.** `cup3` loads a PTX module, launches a shader that computes `out = in*3 + 1`
#   with `in = 14`, and reads `out` back. **43 is un-forgeable** — see `cup3_hook.sh` for why,
#   and for the four-rung value ladder that separates the ways it can fail.
#
# ## ⊘ THE RELAXATIONS ARE CARRIED AND LABELLED — a relaxed green is a MAP, not the milestone
#
#   Byte for byte the w294 arming that produced `^CUP2_RC=0`. Changing the workload AND the
#   arming in one step would make an outcome unattributable, so the arming does not move.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none reads as the favourable one
#
#   (A) `^CUP3_VAL=43`  ⇒ ★★★★★ FIRST COMPUTE. The host GR engine ran the guest's shader.
#       Report EVERY relaxation that was on; it is a relaxed green until they come off.
#   (B) `^CUP3_VAL=14`  ⇒ ★★★ a COPY happened where a COMPUTE belonged. The most diagnostic
#       failure available: the data plane moved bytes, the shader did not run. Name the
#       engine that moved them.
#   (C) `^CUP3_VAL=0` or `61166` ⇒ the launch or the DtoH leg respectively. Both are FULL
#       results. Report the stage ladder and the first ✘.
#   (D) `^CUP3_VAL=` ABSENT ⇒ ⊘ **THE MEASUREMENT DID NOT HAPPEN.** It is NOT a failure value
#       and NOT 0. Report where the ladder stopped, and whether `CUP3_JIT_PRESENT=no`
#       attributes it to the guest image rather than to us.
#   (E) REGRESSION on what cup2 established: `Xid` != 0, or `host_rows` != 18295/18309.
#       ⚠ Checked and reported EVEN ON A GREEN — a pass that regressed the address plane is
#       not a pass, and this tree has shipped a green that hid one.
#   (F) `CUP3_JIT_PRESENT=no` ⇒ a MODULE-stage failure is UNATTRIBUTABLE. Say so instead of
#       reading it as our wall.
#
# ## ⚠ What this rung deliberately does NOT do
#
#   No arming is added, no relaxation removed, no device code changes. The question is
#   exactly *"does what we built for cup2 already carry a kernel launch?"* — and the honest
#   and likely answer is **no, with a named wall**, which is a full result.
set -uo pipefail

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w297}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w297}
# ⊘ OVERRIDABLE — the unconditional form cost w294 a confirmation run: a second boot
#   overwrote the first boot's logs while the grading block printed a perfect result under
#   the first boot's filename from the second boot's data.
export KAYFABE_TAG=${KAYFABE_TAG:-w297cup3}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/cup3_hook.sh"
# cup3 does everything cup2 does and then some; the outer wait must exceed the inner timeout.
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

rm -f /workspace/bench/qemu-build/qemu-system-x86_64
rm -f /workspace/bench/cup3 2>/dev/null

# w290p_run.sh does the stamp gate, the boot, and the address-plane grading, and writes
# everything to /workspace/$KAYFABE_TAG.log. Invoked, never copied, so the rungs cannot drift.
# ★ w298 — the VAS_PUBLISH arm is POSITIONAL in w290p_run.sh, so it is the one relaxation an
#   env override could not reach. `W298_ARM` makes it reachable; ⊘ DEFAULTED to `drain`, the
#   byte-identical w297 value, so every existing caller is unchanged.
"$REPO/scripts/bench/w290p_run.sh" "${W298_ARM:-drain}"
BRC=$?

OUT=/workspace/${KAYFABE_TAG}.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log

{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W297 GRADING — cup3, FIRST COMPUTE  inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
echo ""
echo "=== (F) ATTRIBUTION PRECONDITION — was the PTX JIT present in the guest?"
echo "    CUP3_JIT_PRESENT = [$(grep -oE 'CUP3_JIT_PRESENT=(yes|no)' "$P" 2>/dev/null | tail -1)]"
echo "    ⊘ if 'no', a MODULE-stage failure is the guest image's, not ours."
echo ""
echo "=== ★★★★★ THE METRIC — ^ANCHORED. ⊘ The unanchored read matches GCC_CUP3_RC=0 and has"
echo "===     printed the headline success value on a FAILING arm on seven cup2 rungs."
VAL=$(grep -oE '^CUP3_VAL=[0-9]+' "$P" 2>/dev/null | tail -1)
RC=$(grep -oE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
echo "--- ★★★★★ ${VAL:-⊘ NO LINE BEGINS WITH CUP3_VAL= — THE MEASUREMENT DID NOT HAPPEN. ⊘ NOT 0, NOT a failure value.}"
echo "--- ★★★★  ${RC:-⊘ NO LINE BEGINS WITH CUP3_RC= — no terminator was written.}"
echo "    the kernel line, verbatim = [$(grep -h '^CUP3_KERNEL_LINE=' "$P" 2>/dev/null | tail -1)]"
echo "    UNANCHORED, for contrast  = [$(grep -oh 'CUP3_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
echo ""
echo "=== ★★★★★ THE VERDICT, stated once, in the pre-registered vocabulary"
case "$VAL" in
  CUP3_VAL=43)    echo "    (A) ★★★★★ FIRST COMPUTE. 43 cannot be copied, filled, or forged." ;;
  CUP3_VAL=14)    echo "    (B) ★★★ A COPY WHERE A COMPUTE BELONGED — the input came back." ;;
  CUP3_VAL=0)     echo "    (C) the memset landed; the kernel wrote nothing." ;;
  CUP3_VAL=61166) echo "    (C) 0xeeee — the host sentinel is INTACT; the DtoH leg never wrote." ;;
  "")             echo "    (D) ⊘ UNMEASURED. Read the stage ladder; do not read this as 0." ;;
  *)              echo "    ⚠ $VAL — no stage of cup3 produces this by design. Reported raw." ;;
esac
echo ""
echo "=== ★ THE STAGE LADDER, as the hook recorded it"
grep -E '^    [✔✘] ' "$P" 2>/dev/null | sed 's/^/    /'
echo "=== ★ cup3's own FAIL line, if it named its failure"
grep -hE '^ *FAIL ' "$P" 2>/dev/null | sed 's/^/    /'
echo ""
echo "=== (E) REGRESSION CHECK — cup2's established values are Xid=0 and host_rows 18295/18309"
echo "===     ⚠ printed EVEN ON A GREEN: a pass that regressed the address plane is not a pass."
# ⊘⊘ w297 DEFECT FIX — **THIS LINE PRINTED A MEASURED ZERO AND "UNMEASURED" AT THE SAME
#   TIME, AND THE MEASURED RUN LOOKED LIKE THE UNMEASURED ONE.** `grep -c` prints `0` AND
#   exits 1 when it matches nothing, so the `||` fallback ALSO fired and the field read
#   `[0\n⊘ NO HOST DMESG — UNMEASURED]`. ⇒ Exactly the distinction this repo insists on —
#   *"an empty artefact is not evidence of emptiness"* — collapsed into an unreadable field
#   by the very check written to preserve it. ★ Existence is now its OWN test, before the
#   count, so `0 Xid in a file that exists` and `no file at all` are different words.
#   ⚠ `run_<tag>_hostdmesg.log` is a per-boot DELTA: **0 bytes is the normal green** (no new
#   host dmesg since the watermark). Its emptiness must therefore read as `Xid=0`, not as a
#   failed capture — which is only sound because the probe log states the watermark
#   (`HOST dmesg delta ... watermark N → N`) independently.
if [ -e "$D" ]; then
  echo "    Xid count = [$(grep -c Xid "$D")]   (host dmesg DELTA file exists, $(stat -c%s "$D") bytes)"
  grep -E 'Xid' "$D" 2>/dev/null | head -4 | sed 's/^/      /'
else
  echo "    Xid count = [⊘ NO HOST DMESG FILE AT ALL — UNMEASURED. ⊘ NOT zero.]"
fi
echo "    host_rows, every distinct reading:"
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
echo ""
echo "=== ★ THE UNSERVICED LEDGER — if cup3 walled on a control we refuse, it is named here"
grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -30 | sed 's/^/      /'
echo "      distinct ids = [$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)]"
echo "    ⊘ compare against the cup2 baseline ledger: an id that appears HERE and not there"
echo "      is a cup3-specific demand, which is the most likely shape of a new wall."
echo ""
echo "=== ★ DOORBELLS BY ENGINE — a launch is GR work; cup2's shape was GrCompute + Ce"
# ⊘ w297 DEFECT FIX — the pattern was `GrCompute=[0-9]+ Ce=[0-9]+`, and the device emits
#   `by engine: GrCompute=125 GrGraphics=0 Ce=355 NvEnc=0 ... unrouted=0`. `GrGraphics=`
#   sits BETWEEN the two halves, so the anchored pair could never match and this clause
#   printed NOTHING on a run that had the number. ⇒ A grading clause that greps a shape the
#   producer no longer emits is silent, and silence here is indistinguishable from zero
#   doorbells — which on a compute rung is the failure it was written to detect.
#   ★ Anchor on the emitter's own label instead of on a field adjacency that can drift.
grep -o 'by engine: .*' "$Q" 2>/dev/null | tail -2 | sed 's/^/      /'
echo "      ⊘ if the line above is ABSENT the summary never printed — UNMEASURED, not zero."
echo "      per-doorbell routing tally (independent of the summary line):"
grep -ho 'engine=[A-Za-z]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | sed 's/^/        /'
echo ""
echo "=== ⊘ EVERY RELAXATION THAT WAS ON — a relaxed green is a MAP, not the milestone"
# ★ w298 — KAYFABE_PT_WITNESS_EXEC added. It was set by w290p_run.sh from the first arm and
#   was the ONLY armed variable this "EVERY RELAXATION THAT WAS ON" block did not enumerate,
#   so the block's own heading was false by one even after the w297 echo fix landed.
for v in KAYFABE_PT_SWEEP KAYFABE_OPERAND_JOIN KAYFABE_FB_JOIN KAYFABE_VAS_PUBLISH \
         KAYFABE_GR_ROUTE KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF KAYFABE_GUEST_SEMA \
         KAYFABE_GUEST_OPERAND KAYFABE_ISOLATES KAYFABE_CE_EXECUTOR KAYFABE_PT_WITNESS_EXEC; do
  echo "    $v = [$(grep -oE "$v=[a-z]+" "$OUT" 2>/dev/null | tail -1)]"
done
echo ""
echo "=== ★★ HARNESS SELF-CHECK — assert THIS block's own output exists"
echo "    w297 grading lines = [$(grep -c 'W297 GRADING' "$OUT" 2>/dev/null)]  (MUST be >= 1)"
echo "    qemu log bytes     = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)]"
echo "    probe log bytes    = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W297 EXIT rc=$BRC at $(date -Is) ==="
} >>"$OUT" 2>&1

exit "$BRC"
