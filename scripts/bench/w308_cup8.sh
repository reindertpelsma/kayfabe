#!/usr/bin/env bash
# ★★★★★ w308 — cup8. REAL COMPUTE AT SCALE. The rung that removes cup3's caveat.
#
#   $1 = baseline | cup8   (⊘ REQUIRED, never defaulted — a defaulted arm makes an evidence
#                           run and its own control indistinguishable at the call site)
#     baseline : the cup3 hook, at THIS branch's HEAD. Reproduces `^CUP3_VAL=43` and, just as
#                importantly, produces the UNSERVICED-LEDGER and DOORBELL baselines that the
#                `cup8` arm is diffed against. ⊘ If it does not reproduce, STOP: everything
#                after it would be unattributable.
#     cup8     : the cup8 hook — an N=2048 fp32 matmul on a 16x16 grid of 16x16 blocks.
#
# ## ★ WHY THE BASELINE IS ITS OWN BOOT AND NOT A HISTORICAL NUMBER
#
#   The brief supplies "cup3's 40-id baseline", and `run_w304joint_qemu.log` does contain
#   exactly 40 distinct `unserviced fn 76 cmd` ids. ⊘ But that boot ran a DIFFERENT commit,
#   and this tree has paid repeatedly for comparisons against numbers whose source revision
#   nobody re-checked ("the bench silently served a binary built from 862c7c2 for weeks").
#   ⇒ The baseline is re-measured HERE, at this HEAD, with this binary, and the historical 40
#   is printed beside it as a CROSS-CHECK rather than used as the reference.
#
# ## ★★★ THE ORACLE, SCOPED — the C ran cup8 to `bad=0 maxerr=0`, and that is NOT our claim
#
#   The C research artifact ran `tests/mode2/cup8.c` and `cup8_iter.c` to **bad=0 maxerr=0**
#   on a STOCK, unpatched guest (ladder `cup2 -> cupctx2_min -> cup8 -> cup8_iter`). ⚠ SCOPE
#   IT CORRECTLY: the C's green was a **CONTROL-PLANE** result — `m2cexec` was OFF,
#   completions were emulator-written and the copies were CPU copies. ⇒ **The oracle tells us
#   the workload is legitimate and what a correct answer looks like. It does NOT tell us a
#   hardware forwarding path worked.** That half is ours, and it is what this rung measures.
#   ★ The vendored source is md5-COMPARED in the hook, so "the same program" is a measurement
#     and not a template sentence (the w305 defect).
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none reads as the favourable one
#
#   (A) `^CUP8_BAD=0` ⇒ ★★★★★ REAL COMPUTE AT SCALE. Report N, the grid, the wall time and
#       EVERY relaxation still in force. cup3's caveat is removed.
#   (B) `^CUP8_BAD=` > 0 ⇒ ★★★ THE MOST INTERESTING FAILURE: the plane ran and produced WRONG
#       DATA. Report `maxerr` and WHERE — a SYSTEMATIC offset and a RANDOM scatter are
#       different diagnoses. ⊘ And `bad` is an EARLY-EXIT count, not a mismatch total (the
#       guard is on cup8.c's outer loop); it must not be quoted as one.
#   (C) IT DOES NOT LAUNCH — a named refusal, a fault, or a hang. Report the stage ladder, the
#       unserviced ledger DIFFED against the cup3 baseline measured by arm `baseline`, and the
#       `Xid`.
#   (D) ★★ IT WORKS BUT IS UNUSABLY SLOW. ⊘ **A FULL RESULT, NOT A FAILURE.** Report rows
#       published, drain wall time, and where the time went. First real datum on whether
#       publication amortises.
#       ⚠ THIS RUNNER CARRIES A PRE-RUN COUNTER-PREDICTION, RECORDED HERE SO IT CANNOT BE
#       RETROFITTED: `[measured, run_w304joint_qemu.log, the last green cup3 boot]` there are
#       **229 `★DRAINED` rows totalling 3935 ms**, and **the FIRST doorbell drains 13313 rows
#       in 2732 ms** while the other 228 drain `asked=0 pinned=0 in 0 ms`. ⇒ **publication
#       ALREADY amortises across doorbells**, and cup8's extra 48 MiB is ~12k more 4 KiB rows
#       ≈ a few seconds ONCE. So if cup8 is slow, the pre-registered expectation is that the
#       cost is NOT in publication but in the COPY legs (32 MiB HtoD + 16 MiB DtoH, against
#       cup3's 4 bytes each way). The grading below prints both, separately, so the prediction
#       can be scored rather than asserted.
#   (E) REGRESSION on the address plane — checked and reported EVEN ON A GREEN, because a pass
#       that regressed the address plane is not a pass and this tree has shipped a green that
#       hid one. ⊘ Uses `regression_check_e.sh` (the w304 rewrite): `Xid=0`, the drain RAN AND
#       PINNED (a FLOOR, not a number), and the publication pass's MUST-be-0 invariants.
#       ⊘⊘ `host_rows` is PRINTED AND NEVER GRADED — the pre-w304 criterion graded it as an
#       exact value and FAILED A REAL GREEN. cup8 maps far more memory than cup3, so its
#       host_rows MUST be expected to differ; re-grading it would manufacture a regression.
#   (F) `CUP8_JIT_PRESENT=no` ⇒ a MODULE-stage failure is UNATTRIBUTABLE to us. Say so.
#
# ## ⚠ What this rung deliberately does NOT do
#
#   No arming is added and no relaxation removed. The arming is the CURRENT MASTER DEFAULT —
#   w304 deleted the five inert flags, so this is the leanest arming any compute rung has run.
#   Changing the workload AND the arming in one step would make an outcome unattributable.
#   ⊘ ONE WORKLOAD, THEN REPORT. No iterating toward a green.
set -uo pipefail
ARM="${1:-}"
case "$ARM" in
  baseline) HOOK=cup3_hook.sh; DEFTAG=w308cup3base ;;
  cup8)     HOOK=cup8_hook.sh; DEFTAG=w308cup8 ;;
  *) echo "usage: $0 baseline|cup8" >&2; exit 64 ;;
esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w308}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w308}
export KAYFABE_TAG=${KAYFABE_TAG:-$DEFTAG}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/$HOOK"
# ⊘ GQ_TIMEOUT bounds boot_capture's OWN guest commands (nvidia-smi &c), NOT the hook — the
#   hook is invoked bare. The true outer bound is w290p_run.sh's hardcoded
#   `timeout 1800 boot_capture.sh`, and cup8_hook.sh's 1200 s inner deadline is chosen to fit
#   inside it with ~350 s to spare for boot and shutdown. Stated so the numbers are checkable.
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

rm -f /workspace/bench/qemu-build/qemu-system-x86_64

# ★ w290p_run.sh does the stamp gate, the boot and the address-plane grading, and writes to
#   /workspace/$KAYFABE_TAG.log. INVOKED, never copied, so the rungs cannot drift.
"$REPO/scripts/bench/w290p_run.sh" "${W308_ARM:-drain}"
BRC=$?

OUT=/workspace/${KAYFABE_TAG}.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
# The cup3 baseline this arm is diffed against. ⊘ Overridable, defaulted to arm `baseline`'s.
BASEQ=${KAYFABE_BASELINE_Q:-/workspace/bench/run_w308cup3base_qemu.log}

{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W308 GRADING — arm=$ARM  inner_rc=$BRC  $(date -Is)"
echo "================================================================================"

# ---------------------------------------------------------------------------------------
# (F) ATTRIBUTION PRECONDITIONS
# ---------------------------------------------------------------------------------------
echo ""
echo "=== (F) ATTRIBUTION PRECONDITIONS — checked BEFORE any verdict is read"
echo "    JIT present in guest = [$(grep -oE 'CUP[38]_JIT_PRESENT=(yes|no)' "$P" 2>/dev/null | tail -1)]"
echo "    ⊘ if 'no', a MODULE-stage failure is the guest image's, not ours."
if [ "$ARM" = cup8 ]; then
echo "    same program as the C = [$(grep -oE '^CUP8_SAME_PROGRAM=(yes|no)' "$P" 2>/dev/null | tail -1)]"
echo "    ⊘ if 'no', the C's bad=0 maxerr=0 is NOT a comparison for this run."
echo "    guest free RAM        = [$(grep -oE 'GUEST_MEMFREE_MB=[0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "    ⊘ cup8 mallocs 48 MiB host-side at N=2048; 'OOM host' would be the guest image's."
fi

# ---------------------------------------------------------------------------------------
# ★★★★★ THE METRIC — ANCHORED, with an INDENT-TOLERANT read printed beside it.
#
# ⊘⊘ THE TRAP THIS BLOCK EXISTS FOR, AND IT IS THE INVERTED ONE. The familiar failure is an
#    UNANCHORED read matching something else (`GCC_CUP3_RC=0`) and printing the headline
#    success value on a failing arm — that one is handled by anchoring. The form that bit this
#    tree most recently is the OPPOSITE: a grader printed "⊘ UNMEASURED" while its own
#    verbatim block six lines above printed the value, because the hook INDENTS the workload's
#    output and only the indented copy existed. *"Unmeasured" is the reading this repo treats
#    as safe, so it gets believed.*
#    ⇒ BOTH reads are taken and DISAGREEMENT IS CALLED OUT BY NAME. The anchored read is the
#      metric; a tolerant hit with an anchored miss is a HARNESS defect, not a measurement.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★★★ THE METRIC — ^ANCHORED, with the indent-tolerant read as a CONTRADICTION CHECK"
if [ "$ARM" = cup8 ]; then
  A_BAD=$(grep -oE '^CUP8_BAD=[0-9]+' "$P" 2>/dev/null | tail -1)
  A_ERR=$(grep -oE '^CUP8_MAXERR=[^ ]+' "$P" 2>/dev/null | tail -1)
  A_N=$(grep -oE '^CUP8_N=[0-9]+' "$P" 2>/dev/null | tail -1)
  A_RC=$(grep -oE '^CUP8_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
  A_WALL=$(grep -oE '^CUP8_WALL_S=[0-9]+' "$P" 2>/dev/null | tail -1)
  T_BAD=$(grep -oE '[[:space:]]*bad=[0-9]+' "$P" 2>/dev/null | tr -d ' ' | tail -1)
  echo "--- ★★★★★ ${A_BAD:-⊘ NO LINE BEGINS WITH CUP8_BAD= — THE MEASUREMENT DID NOT HAPPEN. ⊘ NOT 0.}"
  echo "--- ★★★★★ ${A_ERR:-⊘ NO LINE BEGINS WITH CUP8_MAXERR= — UNMEASURED. ⊘ NOT 0.}"
  echo "--- ★★★★  ${A_N:-⊘ NO LINE BEGINS WITH CUP8_N= — the size is UNMEASURED.}"
  echo "--- ★★★★  ${A_RC:-⊘ NO LINE BEGINS WITH CUP8_RC= — no terminator was written.}"
  echo "--- ★★★★  ${A_WALL:-⊘ NO LINE BEGINS WITH CUP8_WALL_S= — the guest-side wall time is UNMEASURED.}"
  echo ""
  echo "    ⊘⊘ CONTRADICTION CHECK (the w307 inverted-UNMEASURED trap):"
  echo "       anchored   ^CUP8_BAD= = [${A_BAD:-<none>}]"
  echo "       tolerant     ...bad=  = [${T_BAD:-<none>}]  (matches the INDENTED verbatim copy too)"
  if [ -z "$A_BAD" ] && [ -n "$T_BAD" ]; then
    echo "       ★★★ DISAGREE — the tolerant read FOUND a value the anchored read did not."
    echo "           ⊘ This is a HARNESS DEFECT, not an unmeasured run. DO NOT report UNMEASURED."
  elif [ -n "$A_BAD" ] && [ -z "$T_BAD" ]; then
    echo "       ⚠ tolerant read empty while anchored read hit — impossible unless the log moved."
  else
    echo "       ✔ the two reads agree on presence; the anchored one is the metric."
  fi
  echo ""
  echo "    the workload's own verdict, verbatim:"
  echo "      [$(grep -h '^CUP8_RESULT_LINE=' "$P" 2>/dev/null | tail -1)]"
  echo "      [$(grep -h '^CUP8_VERDICT_LINE=' "$P" 2>/dev/null | tail -1)]"
  echo "      [$(grep -h '^CUP8_SIZE_LINE=' "$P" 2>/dev/null | tail -1)]"
  echo "      [$(grep -h '^CUP8_GRID_LINE=' "$P" 2>/dev/null | tail -1)]"
  echo "      [$(grep -h '^CUP8_MEMALLOC_LINE=' "$P" 2>/dev/null | tail -1)]"
  echo "      [$(grep -h '^CUP8_FIRSTBAD_LINE=' "$P" 2>/dev/null | tail -1)]"
  echo "      [$(grep -h '^CUP8_C0=' "$P" 2>/dev/null | tail -1)]"
  echo ""
  echo "=== ★★★★★ THE VERDICT, stated once, in the pre-registered vocabulary"
  case "${A_BAD:-}" in
    CUP8_BAD=0) echo "    (A) ★★★★★ REAL COMPUTE AT SCALE. An N=$(printf '%s' "${A_N:-?}" | sed 's/.*=//') fp32 matmul, every element correct."
                echo "        ⇒ cup3's '1x1x1 launch of a six-instruction shader' caveat is REMOVED." ;;
    "")         echo "    (C)/(D) ⊘ UNMEASURED — cup8 never printed its own verdict. Read the ladder."
                echo "        ⊘ Do NOT read this as bad=0 and do NOT read it as a failure value." ;;
    *)          echo "    (B) ★★★ THE PLANE RAN AND PRODUCED WRONG DATA — $A_BAD."
                echo "        ⊘ an EARLY-EXIT count, not a mismatch total. Read FIRST bad + C[0]." ;;
  esac
else
  A_VAL=$(grep -oE '^CUP3_VAL=[0-9]+' "$P" 2>/dev/null | tail -1)
  A_RC=$(grep -oE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
  echo "--- ★★★★★ ${A_VAL:-⊘ NO LINE BEGINS WITH CUP3_VAL= — THE MEASUREMENT DID NOT HAPPEN. ⊘ NOT 0.}"
  echo "--- ★★★★  ${A_RC:-⊘ NO LINE BEGINS WITH CUP3_RC= — no terminator was written.}"
  echo "    UNANCHORED, for contrast = [$(grep -oh 'CUP3_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
  echo ""
  echo "=== ★★★★★ THE BASELINE GATE — ⊘ if this is not 43, STOP. Everything after is unattributable."
  if [ "$A_VAL" = "CUP3_VAL=43" ]; then
    echo "    ✔ BASELINE REPRODUCES — ^CUP3_VAL=43 at this HEAD, with this binary."
    echo "      ⇒ the cup8 arm's outcome is attributable to THE WORKLOAD, not to a moved floor."
  else
    echo "    ✘ BASELINE DID NOT REPRODUCE — [${A_VAL:-⊘ ABSENT/UNMEASURED}], expected CUP3_VAL=43."
    echo "      ⊘⊘ STOP. Do not run the cup8 arm: an outcome measured on a floor that has"
    echo "         itself moved cannot be attributed to the workload."
  fi
fi

echo ""
echo "=== ★ THE STAGE LADDER, as the hook recorded it"
grep -E '^    [✔✘] ' "$P" 2>/dev/null | sed 's/^/    /'
echo "=== ★ the workload's own FAIL line, if it named its failure"
grep -hE '^ *FAIL ' "$P" 2>/dev/null | head -5 | sed 's/^/    /'
if [ "$ARM" = cup8 ]; then
echo "=== ★ THE PER-CALL LADDER — the LAST CUDA call that RETURNED (finer than the stages)"
grep -hE '^ *ok   cu' "$P" 2>/dev/null | tail -6 | sed 's/^/    /'
fi

# ---------------------------------------------------------------------------------------
# (D) WHERE THE TIME WENT — printed on EVERY outcome, not only a slow one.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★ (D) WHERE THE TIME WENT — printed on EVERY outcome, not only a slow one"
echo "--- the workload's own wall clock (guest-side, independent of the poll loop):"
echo "      [$(grep -h '^CUP8_WALL_S=\|^CUP8_HOOK_WAITED_S=\|^CUP8_TERMINATOR_SEEN=' "$P" 2>/dev/null | tr '\n' ' ')]"
echo "--- ★ THE PER-STAGE TIMING LADDER (the hook's deadline poll, ~5 s resolution):"
grep -hE '^    \[\+[0-9]+s\] ' "$P" 2>/dev/null | sed 's/^/    /'
echo "      ⊘ absent here means the run finished between two polls OR never produced output;"
echo "        the terminator flag above distinguishes them."
echo "--- ★★★ PUBLICATION COST — the pre-registered (D) question: does the drain amortise?"
DROWS=$(grep -c '★DRAINED' "$Q" 2>/dev/null || true)
DMS=$(grep -o "★DRAINED(this doorbell's VAS) asked=[0-9]* pinned=[0-9]* refused=[0-9]* in [0-9]* ms" "$Q" 2>/dev/null \
       | grep -oE 'in [0-9]+ ms' | grep -oE '[0-9]+' | paste -sd+ | bc 2>/dev/null)
echo "      ★DRAINED rows          = ${DROWS:-0}"
echo "      Σ drain wall time      = ${DMS:-0} ms"
echo "      the NON-ZERO drains, verbatim (⚠ NUMERIC sort — w298 published '4 of 13348' as a"
echo "      peak by sorting these lexicographically and had to withdraw the claim):"
grep -o "★DRAINED(this doorbell's VAS) asked=[0-9]* pinned=[0-9]* refused=[0-9]* in [0-9]* ms" "$Q" 2>/dev/null \
  | grep -v 'asked=0 pinned=0' | sort -u | head -8 | sed 's/^/        /'
echo "      max single drain ms    = [$(grep -oE 'DRAIN_MS=[0-9]+' "$Q" 2>/dev/null | cut -d= -f2 | sort -n | tail -1)]"
echo "      ⇒ SCORE THE PREDICTION: on cup3 (w304joint) this was 229 rows / 3935 ms total,"
echo "        with 13313 of the rows pinned by the FIRST doorbell and 228 later drains at 0."
echo "        If cup8's Σ is of the same order, publication AMORTISES and any slowness is"
echo "        elsewhere. If it is not, (D)'s mechanism is publication after all."
echo "--- ★ ROWS PUBLISHED (⊘ REPORTED, NEVER GRADED — see (E)):"
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | sed 's/^/        /'
echo "      HOST-PUBLISHED lines   = [$(grep -c 'HOST-PUBLISHED' "$Q" 2>/dev/null)]  (0 = the line never printed: unmeasured, not zero)"
echo "--- ★ DOORBELLS BY ENGINE — a launch is GR work; cup3's shape was GrCompute=125 Ce=355"
grep -o 'by engine: .*' "$Q" 2>/dev/null | tail -2 | sed 's/^/      /'
echo "      ⊘ if the line above is ABSENT the summary never printed — UNMEASURED, not zero."
echo "      per-doorbell routing tally (independent of the summary line):"
grep -ho 'engine=[A-Za-z]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | sed 's/^/        /'

# ---------------------------------------------------------------------------------------
# (E) REGRESSION
# ---------------------------------------------------------------------------------------
echo ""
echo "=== (E) REGRESSION CHECK — the w304 rewrite, printed EVEN ON A GREEN"
echo "===     ⊘ host_rows is NOT re-graded as an exact value: the pre-w304 criterion did that"
echo "===     and FAILED A REAL GREEN. cup8 maps far more memory than cup3, so its host_rows"
echo "===     MUST differ — grading it would manufacture a regression on a correct result."
"$REPO/scripts/bench/regression_check_e.sh" "$Q" "$D"
ERC=$?
echo "    ⇒ (E) exit status = $ERC   (0 pass · 1 REGRESSION · 2 UNMEASURED — ⊘ 2 is NOT a pass)"

# ---------------------------------------------------------------------------------------
# (C) THE UNSERVICED LEDGER, DIFFED
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★ (C) THE UNSERVICED LEDGER — DIFFED against the cup3 baseline"
echo "    this arm's distinct ids  = [$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)]"
if [ -e "$BASEQ" ]; then
  echo "    baseline ($(basename "$BASEQ")) distinct ids = [$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$BASEQ" 2>/dev/null | sort -u | wc -l)]"
  echo "    ⊘ the brief's historical figure is 40 (run_w304joint_qemu.log, a DIFFERENT commit)."
  echo "      It is a cross-check, not the reference — the reference is the line above."
  grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$BASEQ" 2>/dev/null | sort -u > /tmp/w308_base_ids.txt
  grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q"     2>/dev/null | sort -u > /tmp/w308_this_ids.txt
  echo "    ★★★ IDS PRESENT HERE AND NOT IN THE BASELINE (a cup8-SPECIFIC demand — the most"
  echo "        likely shape of a new wall):"
  comm -13 /tmp/w308_base_ids.txt /tmp/w308_this_ids.txt | sed 's/^/        + /' | head -30
  echo "        [$(comm -13 /tmp/w308_base_ids.txt /tmp/w308_this_ids.txt | wc -l) new]"
  echo "    --- ids in the BASELINE and not here (fewer demands, usually because it got further"
  echo "        or less far — informational):"
  comm -23 /tmp/w308_base_ids.txt /tmp/w308_this_ids.txt | sed 's/^/        - /' | head -20
  echo "        [$(comm -23 /tmp/w308_base_ids.txt /tmp/w308_this_ids.txt | wc -l) absent]"
else
  echo "    ⊘⊘ NO BASELINE LOG AT [$BASEQ] — THE DIFF IS UNMEASURED, NOT EMPTY."
  echo "       Run arm 'baseline' first. A missing baseline reads as 'no new ids', which is"
  echo "       exactly the benign-looking absence this tree keeps paying for."
fi
echo "    --- this arm's ledger, by count:"
grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -15 | sed 's/^/      /'

# ---------------------------------------------------------------------------------------
# THE FAULT + THE RELAXATIONS
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★ (C) THE FAULT, BY IDENTITY (host dmesg delta, watermarked to THIS boot)"
echo "    Xid count = [$(grep -c Xid "$D" 2>/dev/null)]   (file $( [ -e "$D" ] && stat -c%s "$D" || echo MISSING) bytes)"
grep -E 'Xid' "$D" 2>/dev/null | head -4 | sed 's/^/      /'
echo "    ★★ ENGINE / HUBCLIENT / ACCESS = [$(grep -oE 'ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | sort | uniq -c | tr '\n' ' ')]"
echo "    ★★ DESCENT LEVEL              = [$(grep -oE 'FAULT_(PDE|PTE)[0-9]*' "$D" 2>/dev/null | sort | uniq -c | tr '\n' ' ')]"
echo "       ⊘ empty means NO FAULT LEVEL WAS PRINTED — UNMEASURED, not 'no fault'."

echo ""
echo "=== ⊘ EVERY RELAXATION STILL IN FORCE — read from the DEVICE'S OWN EMISSIONS first"
echo "--- ★★★ THE ARMING ACTUALLY IN FORCE (a boot happening is not an arm running):"
grep -oE 'VAS-PUBLISH arm=[a-z]+ fb_join=[a-z]+ host_isolates=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/      /'
grep -oE 'OPERAND-JOIN arm=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/      /'
grep -oE 'GR-ROUTE arm=[a-z]+|GUEST-RING arm=[a-z]+|CE-EXECUTOR arm=[a-z]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
echo "      ⊘ an ABSENT line here means the device never emitted that arm — UNMEASURED, and"
echo "        specifically NOT 'the relaxation was off'."
echo "--- the arming as the RUNNER SET IT (a record of INTENT, not of execution):"
# ⊘ w304 deleted KAYFABE_PT_SWEEP, KAYFABE_GUEST_PUSHBUF, KAYFABE_GUEST_SEMA,
#   KAYFABE_GUEST_OPERAND and KAYFABE_OPERAND_JOIN's `join` arm FROM THE DEVICE. They are not
#   listed: a variable the device ignores, printed under a heading that says it was armed, is
#   worse than an absent one.
for v in KAYFABE_OPERAND_JOIN KAYFABE_FB_JOIN KAYFABE_VAS_PUBLISH \
         KAYFABE_GR_ROUTE KAYFABE_GUEST_RING \
         KAYFABE_ISOLATES KAYFABE_CE_EXECUTOR KAYFABE_PT_WITNESS_EXEC; do
  echo "      $v = [$(grep -oE "$v=[a-z]+" "$OUT" 2>/dev/null | tail -1)]"
done

echo ""
echo "=== ★★ HARNESS SELF-CHECK — assert THIS block's own output exists"
echo "    w308 grading lines = [$(grep -c 'W308 GRADING' "$OUT" 2>/dev/null)]  (MUST be >= 1)"
echo "    qemu log bytes     = [$( [ -e "$Q" ] && stat -c%s "$Q" || echo MISSING)]"
echo "    probe log bytes    = [$( [ -e "$P" ] && stat -c%s "$P" || echo MISSING)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W308 EXIT arm=$ARM rc=$BRC at $(date -Is) ==="
} >>"$OUT" 2>&1

exit "$BRC"
