#!/usr/bin/env bash
# ★★★★★ w311 — THE GUEST ARM of the throughput measurement. One boot.
#
#   usage: scripts/bench/w311_run.sh
#
# ## What this rung is, and what it is NOT
#
# w308 proved CORRECTNESS at scale (2048² matmul, `bad=0 maxerr=0`, bit-exact). ⊘ Its 39 s
# workload wall is a ONE-SHOT figure and is not a throughput number. This rung measures
# THROUGHPUT, and the deliverable is a RATIO: guest GFLOP/s ÷ native GFLOP/s on the SAME
# physical GA106. The absolute number is nearly meaningless — the kernel is a naive triple
# loop and both arms are far off peak. THE RATIO IS THE FORWARDING OVERHEAD.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none reads as the favourable one
#
#   (A) ratio ≳ 0.5        ⇒ an LLM is PLAUSIBLE. Report the residual per-launch overhead.
#   (B) ratio ~0.1–0.5     ⇒ possible but needs work. NAME WHERE THE TIME GOES — doorbell,
#                            publication, completion polling, or the engine itself. The batch
#                            phase is the instrument that separates them.
#   (C) ratio < 0.1        ⇒ the LLM target needs a different plan. ★ A FULL RESULT, NOT A
#                            FAILURE, and far more valuable now than after building on the
#                            assumption. ⊘ DO NOT TUNE TOWARD A GOOD RATIO.
#   (D) per-launch cost FLAT IN N      ⇒ fixed overhead dominates. That is GOOD news for large
#                            models and must be stated as such, not buried as "overhead".
#   (E) per-launch cost SCALES WITH N  ⇒ we are in the data path somewhere we should not be.
#                            Name where.
#   (F) no native reference obtainable ⇒ guest-only, EXPLICITLY UNREFERENCED. ⊘ A ratio
#                            against a remembered or published figure is not a ratio.
#
# ## ⚠ What this rung deliberately does NOT do
#
# No arming is added and no relaxation removed — the arming is the CURRENT MASTER DEFAULT.
# Changing the workload AND the arming in one step would make an outcome unattributable.
# ⊘ ONE BOOT, THEN REPORT. No iterating toward a better number.
set -uo pipefail

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w311}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w311}
export KAYFABE_TAG=${KAYFABE_TAG:-w311bench}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/cup8bench_hook.sh"
# ⊘ GQ_TIMEOUT bounds boot_capture's OWN guest commands (nvidia-smi &c), NOT the hook — the
#   hook is invoked bare. The true outer bound is w290p_run.sh's hardcoded
#   `timeout 1800 boot_capture.sh`; cup8bench_hook.sh's 1100 s + 180 s inner deadlines are
#   chosen to fit inside it with room for boot and shutdown. Stated so it is checkable.
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

rm -f /workspace/bench/qemu-build/qemu-system-x86_64

# ★ w290p_run.sh does the stamp gate, the boot and the address-plane grading, and writes to
#   /workspace/$KAYFABE_TAG.log. INVOKED, never copied, so the rungs cannot drift.
"$REPO/scripts/bench/w290p_run.sh" "${W311_ARM:-drain}"
BRC=$?

OUT=/workspace/${KAYFABE_TAG}.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
NAT=${W311_NATIVE_LOG:-/workspace/bench/w311_native.log}

{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W311 GRADING — inner_rc=$BRC  $(date -Is)"
echo "================================================================================"

echo ""
echo "=== (F) ATTRIBUTION PRECONDITIONS — checked BEFORE any verdict is read"
echo "    JIT present in guest  = [$(grep -oE '^BENCH_JIT_PRESENT=(yes|no)' "$P" 2>/dev/null | tail -1)]"
echo "    ⊘ if 'no', a MODULE-stage failure is the guest image's, not ours."
echo "    guest source md5      = [$(grep -oE '^BENCH_GUEST_SRC_MD5=[0-9a-f]*' "$P" 2>/dev/null | tail -1)]"
echo "    hook source md5       = [$(grep -oE '^BENCH_SRC_MD5=[0-9a-f]*' "$P" 2>/dev/null | tail -1)]"
echo "    guest free RAM        = [$(grep -oE '^BENCH_GUEST_MEMFREE_MB=[0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "    guest gcc rc          = [$(grep -oE '^BENCH_GCC_RC=[0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "    guest device name     = [$(grep -h '^GUEST_BENCH_DEVICE=' "$P" 2>/dev/null | tail -1)]"

# ---------------------------------------------------------------------------------------
# ★★★★★ THE GUARD BEFORE THE METRIC. An unguarded bad=0 is not a correctness statement.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★★★ THE NEGATIVE CONTROL — read BEFORE the numbers, because it licenses them"
NCB=$(grep -oE '^GUEST_NEGCTRL_TOTAL_BAD=[0-9]+' "$P" 2>/dev/null | tail -1 | cut -d= -f2)
echo "    ${NCB:+GUEST_NEGCTRL_TOTAL_BAD=$NCB}${NCB:-⊘ NO ANCHORED GUEST_NEGCTRL_TOTAL_BAD LINE — UNMEASURED, ⊘ NOT 0}"
case "${NCB:-__none__}" in
  __none__) echo "    ⊘ THE MEASUREMENT BELOW IS UNGUARDED. Report it as such." ;;
  0)        echo "    ✘✘ THE VERIFIER IS DEAD IN THE GUEST — every bad=0 below is VACUOUS." ;;
  *)        echo "    ✔ the verifier fired in the guest with launches skipped ⇒ the poison fill"
            echo "      and the readback are live on our plane, so bad=0 is a real assertion." ;;
esac

echo ""
echo "=== ★★★★★ THE GUEST NUMBERS — anchored, per size"
grep -hE '^GUEST_BSUM ' "$P" 2>/dev/null | sed 's/^/    /'
echo "    ⊘ a size with no BSUM line above is UNMEASURED — not slow, not zero. NONE."
echo ""
echo "    correctness across every timed iteration:"
echo "      [$(grep -hE '^GUEST_BENCH_TOTAL_BAD=' "$P" 2>/dev/null | tail -1)]"
echo "      [$(grep -hE '^GUEST_BENCH_VERDICT=' "$P" 2>/dev/null | tail -1)]"
echo "      [$(grep -hE '^GUEST_SIZES_DONE=' "$P" 2>/dev/null | tail -1)]"
echo "    ⚠ this verifier has NO early-exit guard, so bad IS a whole-matrix total and maxerr"
echo "      IS a whole-matrix maximum — unlike cup8.c's, which had to be caveated every time."
echo ""
echo "    startup costs, paid ONCE per process (⊘ NOT throughput):"
for k in GUEST_BENCH_INIT_MS GUEST_BENCH_CTX_MS GUEST_BENCH_MODULE_MS; do
  echo "      $k = [$(grep -h "^$k=" "$P" 2>/dev/null | tail -1 | cut -d= -f2-)]"
done

# ---------------------------------------------------------------------------------------
# ★★★★★ THE RATIO
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★★★ THE RATIO — guest ÷ native, SAME physical GA106"
if [ -s "$NAT" ]; then
  "$REPO/scripts/bench/w311_ratio.sh" "$P" "$NAT"
else
  echo "    ⊘⊘ (F) NO NATIVE LOG AT [$NAT] — THE RATIO IS UNMEASURED, NOT 1.0 AND NOT ABSENT."
  echo "       Run scripts/bench/w311_native.sh on the bench host. Guest-only numbers must be"
  echo "       reported as EXPLICITLY UNREFERENCED; a ratio against a remembered figure is"
  echo "       not a ratio."
fi

# ---------------------------------------------------------------------------------------
# WHERE THE TIME WENT
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★ WHERE THE TIME WENT — printed on EVERY outcome, not only a slow one"
echo "--- the workload's own wall clock (guest-side, independent of the poll loop):"
echo "      [$(grep -hE '^BENCH_measure_WALL_S=|^BENCH_measure_HOOK_WAITED_S=|^BENCH_measure_TERMINATOR_SEEN=|^BENCH_measure_RC=' "$P" 2>/dev/null | tr '\n' ' ')]"
echo "      [$(grep -hE '^BENCH_negctrl_WALL_S=|^BENCH_negctrl_TERMINATOR_SEEN=|^BENCH_negctrl_RC=' "$P" 2>/dev/null | tr '\n' ' ')]"
echo "--- ★ THE PER-STAGE TIMING LADDER (the hook's deadline poll, ~5 s resolution):"
grep -hE '^    \[\+[0-9]+s\] ' "$P" 2>/dev/null | tail -40 | sed 's/^/    /'
echo "--- ★★★ PUBLICATION COST — is it paid once, or per launch? THE (D)/(E) DISCRIMINATOR."
DROWS=$(grep -c '★DRAINED' "$Q" 2>/dev/null || true)
DMS=$(grep -o "★DRAINED(this doorbell's VAS) asked=[0-9]* pinned=[0-9]* refused=[0-9]* in [0-9]* ms" "$Q" 2>/dev/null \
       | grep -oE 'in [0-9]+ ms' | grep -oE '[0-9]+' | paste -sd+ | bc 2>/dev/null)
echo "      ★DRAINED rows          = ${DROWS:-0}"
echo "      Σ drain wall time      = ${DMS:-0} ms"
echo "      the NON-ZERO drains, verbatim (⚠ NUMERIC sort — w298 published a peak by sorting"
echo "      these lexicographically and had to withdraw the claim):"
grep -o "★DRAINED(this doorbell's VAS) asked=[0-9]* pinned=[0-9]* refused=[0-9]* in [0-9]* ms" "$Q" 2>/dev/null \
  | grep -v 'asked=0 pinned=0' | sort -u | head -8 | sed 's/^/        /'
echo "      max single drain ms    = [$(grep -oE 'DRAIN_MS=[0-9]+' "$Q" 2>/dev/null | cut -d= -f2 | sort -n | tail -1)]"
echo "      ⇒ ★★★ THE READING: w308 measured 275 rows / 3894 ms for ONE launch. This rung runs"
echo "        ~60+ launches. If Σ drain time is of the SAME ORDER as w308's, publication is"
echo "        paid ONCE and is a STARTUP cost — outcome (D). If it scales with launch count,"
echo "        publication is IN the per-launch path — outcome (E), and that names the place."
echo "--- ★ ROWS PUBLISHED (⊘ REPORTED, NEVER GRADED — see the (E) check):"
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | tail -4 | sed 's/^/        /'
echo "--- ★ DOORBELLS BY ENGINE — a launch is GR work"
grep -o 'by engine: .*' "$Q" 2>/dev/null | tail -2 | sed 's/^/      /'
echo "      per-doorbell routing tally (independent of the summary line):"
grep -ho 'engine=[A-Za-z]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | sed 's/^/        /'
echo "      ⊘ an ABSENT line here is UNMEASURED, not zero."

echo ""
echo "=== (E) REGRESSION CHECK — the w304 rewrite, printed EVEN ON A GREEN"
echo "===     ⊘ host_rows is NOT graded as an exact value: the pre-w304 criterion did that and"
echo "===     FAILED A REAL GREEN. This workload maps far more than cup3, so it MUST differ."
"$REPO/scripts/bench/regression_check_e.sh" "$Q" "$D"
ERC=$?
echo "    ⇒ (E) exit status = $ERC   (0 pass · 1 REGRESSION · 2 UNMEASURED — ⊘ 2 is NOT a pass)"

echo ""
echo "=== ★ THE FAULT, BY IDENTITY (host dmesg delta, watermarked to THIS boot)"
echo "    Xid count = [$(grep -c Xid "$D" 2>/dev/null)]   (file $( [ -e "$D" ] && stat -c%s "$D" || echo MISSING) bytes)"
grep -E 'Xid' "$D" 2>/dev/null | head -4 | sed 's/^/      /'
echo "    guest-side Xid count = [$(grep -h '^GUEST_XID_COUNT=' "$P" 2>/dev/null | tail -1)]"

echo ""
echo "=== ⊘ EVERY RELAXATION STILL IN FORCE — read from the DEVICE'S OWN EMISSIONS first"
grep -oE 'VAS-PUBLISH arm=[a-z]+ fb_join=[a-z]+ host_isolates=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/      /'
grep -oE 'OPERAND-JOIN arm=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/      /'
grep -oE 'GR-ROUTE arm=[a-z]+|GUEST-RING arm=[a-z]+|CE-EXECUTOR arm=[a-z]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
echo "      ⊘ an ABSENT line means the device never emitted that arm — UNMEASURED, and"
echo "        specifically NOT 'the relaxation was off'."
echo "--- the arming as the RUNNER SET IT (a record of INTENT, not of execution):"
for v in KAYFABE_OPERAND_JOIN KAYFABE_FB_JOIN KAYFABE_VAS_PUBLISH \
         KAYFABE_GR_ROUTE KAYFABE_GUEST_RING \
         KAYFABE_ISOLATES KAYFABE_CE_EXECUTOR KAYFABE_PT_WITNESS_EXEC; do
  echo "      $v = [$(grep -oE "$v=[a-z]+" "$OUT" 2>/dev/null | tail -1)]"
done

echo ""
echo "=== ★★ HARNESS SELF-CHECK — assert THIS block's own output exists"
echo "    w311 grading lines = [$(grep -c 'W311 GRADING' "$OUT" 2>/dev/null)]  (MUST be >= 1)"
echo "    qemu log bytes     = [$( [ -e "$Q" ] && stat -c%s "$Q" || echo MISSING)]"
echo "    probe log bytes    = [$( [ -e "$P" ] && stat -c%s "$P" || echo MISSING)]"
echo "    native log bytes   = [$( [ -e "$NAT" ] && stat -c%s "$NAT" || echo MISSING)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check. And a"
echo "      TRUNCATED artifact reads as PRESENT and looks healthier than an empty one —"
echo "      check the expected size, not merely non-emptiness."
echo "=== W311 EXIT rc=$BRC at $(date -Is) ==="
} >>"$OUT" 2>&1

exit "$BRC"
