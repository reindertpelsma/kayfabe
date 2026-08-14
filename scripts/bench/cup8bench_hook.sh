#!/usr/bin/env bash
# ★★★★★ w311 cup8bench POST_CAPTURE_HOOK — THE GUEST ARM of the throughput measurement.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/cup8bench_hook.sh scripts/bench/boot_capture.sh <tag>
#
# ## What this measures, and what it deliberately does NOT
#
# w308 proved cup8 CORRECT at N=2048 (`bad=0 maxerr=0`, bit-exact) and reported a 39 s
# workload wall. ⊘ THAT 39 s IS NOT A THROUGHPUT FIGURE — it is one shot fusing cuInit, ctx,
# PTX JIT, allocation, publication, two copies, one launch and an O(N^2) verify, and it must
# never be quoted as a per-launch cost. This hook runs `cup8bench`, which separates
# STARTUP / FIRST LAUNCH / STEADY STATE and reports a DISTRIBUTION per size.
#
# ★ The deliverable is a RATIO against `w311_native.sh` on the SAME physical GA106. This hook
#   produces only the numerator. It makes no comparison itself — a hook that both measures and
#   grades against a remembered constant is how an unreferenced number acquires a reference.
#
# ## ★★★ THE NEGATIVE CONTROL IS RUN IN THE GUEST, NOT ONLY NATIVELY — and here is why
#
# `bad=0` in the guest could in principle be produced with NO LAUNCH AT ALL: if the poison
# fill (`cuMemsetD32`) silently did nothing on our plane, C would still hold the PREVIOUS
# iteration's correct answer and every later iteration would verify. ⇒ the guest run of
# `BENCH_NOLAUNCH=1` is what proves the poison actually lands here. Natively it proves only
# that the verifier's arithmetic is alive.
# ⊘ It runs AFTER the measurement, as its own process, with its own timeout and its own rc
#   file: it is a SECOND CUDA context, and this tree has a known second-context hang (#12).
#   Ordering it second means a hang costs the control, never the measurement.
#
# ## ⚠ Traps carried forward, each already paid for in this tree
#
# - Every graded line is emitted at COLUMN 0, unindented, unconditionally, and the workload's
#   verbatim output is INDENTED. ⊘⊘ w307's inverted trap: a grader printed "UNMEASURED" while
#   its own verbatim block six lines above printed the value, because the hook indents.
# - START marker + rc file per invocation. `143` (the job was killed) and `124` (the LAUNCHER
#   expired while the job ran on fine) arrive as the same word without a terminator to check.
# - Delete the binary before building: `[ -x ]` cannot tell fresh from stale.
# - The PTX JIT is a GUEST-ENVIRONMENT precondition checked BY NAME, so a MODULE-stage failure
#   is attributable. An unattributable failure is not a measurement.
# - The poll loop is DEADLINE-based, never iteration-based: each gssh round trip costs ~1 s on
#   top, so an iteration-counted loop covers less wall time than its own name claims — fatal
#   on a rung whose whole question is HOW LONG THINGS TAKE.
# - SIZES ARE ORDERED SMALL FIRST. cup8bench prints each size's summary when THAT size ends,
#   so a deadline hit during the largest size still leaves the smaller sizes fully measured.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
SRC=${KAYFABE_BENCH_SRC:-$SRC_DIR/cup8bench.c}

# ★ The bound is DERIVED, not inherited. Guest per-launch cost is the unknown this rung
#   exists to measure, so the budget is set from the outer bound instead: w290p_run.sh caps
#   boot_capture at `timeout 1800`, boot+shutdown+grading needs ~350 s, leaving ~1400 s.
BENCH_TIMEOUT=${KAYFABE_BENCH_TIMEOUT:-1100}
NEG_TIMEOUT=${KAYFABE_BENCH_NEG_TIMEOUT:-180}
B_SIZES=${KAYFABE_BENCH_SIZES:-512,1024,2048}
B_ITERS=${KAYFABE_BENCH_ITERS:-20}
B_BATCH=${KAYFABE_BENCH_BATCH:-10}
B_VERIFY=${KAYFABE_BENCH_VERIFY:-1}
# ★★★★★ ADDED AFTER THE FIRST w311 BOOT, BECAUSE THAT BOOT MEASURED THE PROBLEM.
#   The negative control ran SECOND, as a second CUDA context, and hung at `cuCtxCreate`
#   while the first process's teardown was still emitting `STOP_CHANNEL` /
#   `GPU_EVICT_CTX` NOT_SUPPORTED — the known #12 shape. It hit its own 180 s bound
#   (rc=124 from the INNER timeout: the job was bounded and reached the bound; ⊘ NOT the
#   143 that means something killed it, and ⊘ NOT the 124 that means a launcher expired
#   while the job ran on fine). ⇒ `GUEST_NEGCTRL_TOTAL_BAD=NO_LINE`, and the measurement
#   was UNGUARDED.
#   `KAYFABE_BENCH_ONLY=negctrl` runs the control as the FIRST AND ONLY context of its own
#   boot, which is the configuration it needed all along. ⊘ It is a SEPARATE BOOT and not a
#   reordering: running it first in the SAME boot would have made the MEASUREMENT the second
#   context, moving the hang onto the thing we actually came to measure.
B_ONLY=${KAYFABE_BENCH_ONLY:-both}
# ★★★ w320 — the sync-floor knobs. DEFAULT EMPTY / DEFAULT 0, so a boot that does not set
#   them runs the byte-identical workload w315 and w318 were read against.
B_SWEEP=${KAYFABE_BENCH_BATCH_SWEEP:-}
B_REPS=${KAYFABE_BENCH_BATCH_REPS:-5}
B_CTXF=${KAYFABE_BENCH_CTX_FLAGS:-0}
# ★★★ w322 — the placement + aperture knobs. ⊘ BENCH_HOSTMEM was NEVER forwarded to the guest
#   before this rung (w320 could only run it natively), so a guest hostmem arm was impossible
#   to ask for; it is forwarded here so the guest can be measured at the SAME placement the
#   native control uses. All default to the previous behaviour.
B_HOSTMEM=${KAYFABE_BENCH_HOSTMEM:-0}
B_ALLOC=${KAYFABE_BENCH_ALLOC:-}
B_BW=${KAYFABE_BENCH_BW:-}
B_BWIT=${KAYFABE_BENCH_BW_ITERS:-7}
B_BWTGT=${KAYFABE_BENCH_BW_TARGET_MIB:-256}
B_BWONLY=${KAYFABE_BENCH_BW_ONLY:-0}
B_BWREPS=${KAYFABE_BENCH_BW_REPS:-0}
# ★ w327 — the fill chunk is the failure-offset MEASUREMENT'S OWN RESOLUTION. Defaulted to
#   8 MiB so an un-armed run is byte-identical; forwarded so a rung can bisect the offset.
#   ⚠ Forwarded AND printed, because `KAYFABE_BENCH_NOLAUNCH` was exported for a whole rung
#   and silently dropped here, and the omission produced a GREEN known-positive.
B_BWFC=${KAYFABE_BENCH_BW_FILL_CHUNK_MIB:-}
# ⊘⊘ KAYFABE_BENCH_NOLAUNCH WAS NEVER FORWARDED, AND THE OMISSION PRODUCED A GREEN.
#   w322's `bwneg` arm exported it, the hook dropped it, the workload ran in MEASURE mode and
#   reported `BENCH_VERDICT: PASS (every bw row verified) bad=0` — which is indistinguishable
#   from a passing measurement arm and is EXACTLY the shape of a vacuous known-positive. The
#   only tell was `BENCH_MODE=MEASURE` in a log whose arm is named `neg`. ⇒ forwarded here,
#   and ASSERTED by the caller: see w322_operands.sh, which VOIDs the arm if the workload did
#   not print `BENCH_MODE=NOLAUNCH`.
B_NOLAUNCH=${KAYFABE_BENCH_NOLAUNCH:-0}

die() { echo "★ cup8bench hook FAILED: $*"; exit 2; }

# ⊘ VALIDATED AFTER `die` EXISTS. The first draft put this `case` three lines ABOVE the
#   function it calls, so the one path that was supposed to reject a typo would itself have
#   died with `die: command not found` — a guard that fails in a different voice than the one
#   it was written in is not a guard.
case "$B_ONLY" in both|measure|negctrl) ;;
  *) die "KAYFABE_BENCH_ONLY must be both|measure|negctrl (got [$B_ONLY])" ;; esac
echo "BENCH_ONLY=$B_ONLY"

echo "=== source (md5 — a run cannot silently be a different copy) ==="
[ -f "$SRC" ] || die "no such source: $SRC"
printf '    %-56s %5s lines  md5 %s\n' "$SRC" "$(wc -l < "$SRC")" \
       "$(md5sum < "$SRC" | cut -d' ' -f1)"
echo "BENCH_SRC_MD5=$(md5sum < "$SRC" | cut -d' ' -f1)"
echo "    ★ THE SAME FILE the native arm builds — w311_native.sh copies THIS path. The md5 is"
echo "      printed by both arms so 'same source' is a comparison, not a template sentence."
echo "BENCH_PARAMS sizes=$B_SIZES iters=$B_ITERS batch=$B_BATCH verify=$B_VERIFY"
echo "BENCH_W322_PARAMS hostmem=$B_HOSTMEM alloc=[$B_ALLOC] bw=[$B_BW] bw_iters=$B_BWIT bw_target_mib=$B_BWTGT bw_only=$B_BWONLY bw_reps=$B_BWREPS nolaunch=$B_NOLAUNCH fill_chunk_mib=[${B_BWFC:-<default:8>}]"

# ---------------------------------------------------------------------------------------
# ★★★ PRECONDITIONS, BY NAME. Each fails in a way INDISTINGUISHABLE from our wall unless it
#     is checked separately, first.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★ GUEST PRECONDITION 1 — the PTX JIT (cuModuleLoadData needs it) ==="
JIT=$($G 'ls -1 /usr/lib/x86_64-linux-gnu/libnvidia-ptxjitcompiler.so* 2>/dev/null | head -3; \
          ls -1 /usr/lib/libnvidia-ptxjitcompiler.so* 2>/dev/null | head -3' 2>&1 | tr -d '\r')
if [ -z "$JIT" ]; then
  echo "    ★★★ ABSENT — libnvidia-ptxjitcompiler not found in the guest."
  echo "    ⊘ A MODULE-stage failure below is therefore NOT ATTRIBUTABLE to our stack."
  JIT_OK=no
else
  echo "$JIT" | sed 's/^/    /'
  JIT_OK=yes
fi
echo "BENCH_JIT_PRESENT=$JIT_OK"

echo ""
echo "=== ★★ GUEST PRECONDITION 2 — HOST-SIDE RAM. cup8bench mallocs 3 x N^2 x 4 per size ==="
echo "    ⊘ At N=2048 that is 48 MiB of GUEST RAM. If malloc fails it prints 'OOM host' and"
echo "      exits 1 — a GUEST-IMAGE failure, not our address plane."
$G 'free -m | head -2' 2>&1 | tr -d '\r' | sed 's/^/    /'
echo "BENCH_GUEST_MEMFREE_MB=$($G 'free -m | sed -n "s/^Mem: *[0-9]* *[0-9]* *\([0-9]*\).*/\1/p"' 2>/dev/null | tr -d '\r\n ')"

echo ""
echo "=== push + build in the guest ==="
echo "    ⊘ cup8bench.c does NOT include <cuda.h>: it declares the Driver-API entry points it"
echo "      uses, so guest and host build it with the same one-line gcc invocation. That is"
echo "      what makes the two arms comparable at all."
$G 'cat > /tmp/cup8bench.c' < "$SRC" || die "could not push cup8bench.c"
$G 'rm -f /tmp/cup8bench'   # no build => no file => no run
$G 'gcc -O2 -o /tmp/cup8bench /tmp/cup8bench.c -lcuda -lm 2>&1; echo BENCH_GCC_RC=$?' 2>&1 | tr -d '\r'
$G 'test -x /tmp/cup8bench' || die "cup8bench did not build in the guest"
echo "BENCH_GUEST_SRC_MD5=$($G 'md5sum < /tmp/cup8bench.c' 2>/dev/null | tr -d '\r' | cut -d' ' -f1)"

# ---------------------------------------------------------------------------------------
# run_detached <name> <timeout> <env-prefix>
#   Launches under setsid with a START marker, an rc file and an end-epoch, then polls to a
#   DEADLINE printing every new tail line with its elapsed timestamp. That poll IS the
#   per-stage timing ladder.
# ---------------------------------------------------------------------------------------
run_detached() {
  local NAME=$1 TMO=$2 ENVP=$3
  echo ""
  echo "=== launch [$NAME] DETACHED under its own ${TMO}s timeout ==="
  $G "cat > /tmp/run_${NAME}.sh" <<GUESTEOF
#!/bin/sh
rm -f /tmp/${NAME}.out /tmp/${NAME}.rc /tmp/${NAME}.endepoch
echo "STARTED \$(date -Is) epoch=\$(date +%s)" > /tmp/${NAME}.started
setsid sh -c 'cd /tmp && ${ENVP} timeout ${TMO} ./cup8bench >/tmp/${NAME}.out 2>&1; echo \$? >/tmp/${NAME}.rc; date +%s >/tmp/${NAME}.endepoch' \\
       </dev/null >/dev/null 2>&1 &
sleep 1
echo "LAUNCHED pid=\$(pgrep -x cup8bench | head -1)"
GUESTEOF
  $G "sh /tmp/run_${NAME}.sh" 2>&1 | tr -d '\r' | sed 's/^/    /'
  $G "cat /tmp/${NAME}.started" 2>&1 | tr -d '\r' | sed 's/^/    /'

  local T0 DEADLINE LAST NOW TERM
  T0=$(date +%s); DEADLINE=$(( T0 + TMO + 120 )); LAST=""; TERM=no
  echo "=== waiting for [$NAME] terminator — deadline $((TMO + 120))s from now ==="
  while [ "$(date +%s)" -lt "$DEADLINE" ]; do
    if $G "test -f /tmp/${NAME}.rc" 2>/dev/null; then
      echo "    [+$(( $(date +%s) - T0 ))s] ★ terminator present"; TERM=yes; break
    fi
    NOW=$($G "tail -1 /tmp/${NAME}.out 2>/dev/null" 2>/dev/null | tr -d '\r')
    if [ -n "$NOW" ] && [ "$NOW" != "$LAST" ]; then
      echo "    [+$(( $(date +%s) - T0 ))s] $NOW"; LAST="$NOW"
    fi
    sleep 5
  done
  echo "BENCH_${NAME}_TERMINATOR_SEEN=$TERM"
  echo "BENCH_${NAME}_HOOK_WAITED_S=$(( $(date +%s) - T0 ))"
  if [ "$TERM" = no ]; then
    echo "    ⊘⊘ NO TERMINATOR within the deadline. ⚠ Before reading this as a hang: a"
    echo "       LAUNCHER expiring (124) and the JOB being killed (143) arrive as the same"
    echo "       word. Liveness is checked DIRECTLY, right here:"
    $G 'pgrep -x cup8bench >/dev/null && echo "BENCH_STILL_ALIVE=yes pid=$(pgrep -x cup8bench|head -1)" || echo "BENCH_STILL_ALIVE=no"' \
       2>&1 | tr -d '\r'
  fi
  local SE EE
  SE=$($G "sed -n 's/.*epoch=//p' /tmp/${NAME}.started 2>/dev/null" 2>/dev/null | tr -d '\r\n ')
  EE=$($G "cat /tmp/${NAME}.endepoch 2>/dev/null" 2>/dev/null | tr -d '\r\n ')
  if [ -n "$SE" ] && [ -n "$EE" ]; then echo "BENCH_${NAME}_WALL_S=$(( EE - SE ))";
  else echo "BENCH_${NAME}_WALL_S=NO_TERMINATOR"; fi
  echo "BENCH_${NAME}_RC=$($G "cat /tmp/${NAME}.rc 2>/dev/null" 2>/dev/null | tr -d '\r\n ' | sed 's/^$/NO_RC_FILE/')"
}

# =======================================================================================
# ★★★★★ THE MEASUREMENT — FIRST, so a second-context hang can only cost the control.
# =======================================================================================
if [ "$B_ONLY" = negctrl ]; then
  echo ""
  echo "=== ⊘ MEASUREMENT SKIPPED (KAYFABE_BENCH_ONLY=negctrl) ==="
  echo "    This boot exists ONLY to run the negative control as the FIRST AND ONLY CUDA"
  echo "    context. Every measurement line below is therefore ABSENT BY CONSTRUCTION, and"
  echo "    ⊘ an absent line here must not be read as a measured zero."
  echo "GUEST_BENCH_TOTAL_BAD=SKIPPED_BY_ARM"
  echo "GUEST_SIZES_DONE=SKIPPED_BY_ARM"
else
run_detached measure "$BENCH_TIMEOUT" \
  "BENCH_SIZES=$B_SIZES BENCH_ITERS=$B_ITERS BENCH_BATCH=$B_BATCH BENCH_VERIFY=$B_VERIFY \
BENCH_BATCH_SWEEP=$B_SWEEP BENCH_BATCH_REPS=$B_REPS BENCH_CTX_FLAGS=$B_CTXF \
BENCH_HOSTMEM=$B_HOSTMEM BENCH_ALLOC=$B_ALLOC BENCH_BW=$B_BW BENCH_BW_ITERS=$B_BWIT \
BENCH_BW_TARGET_MIB=$B_BWTGT BENCH_BW_ONLY=$B_BWONLY BENCH_BW_REPS=$B_BWREPS \
BENCH_NOLAUNCH=$B_NOLAUNCH BENCH_BW_FILL_CHUNK_MIB=$B_BWFC"

echo ""
echo "=== ★ [measure] FULL OUTPUT, verbatim (⚠ INDENTED — the graded lines below are NOT) ==="
$G 'cat /tmp/measure.out 2>/dev/null' | sed 's/^/    /'
echo "BENCH_MEASURE_OUT_BYTES=$($G 'wc -c < /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r\n ')"

# ---------------------------------------------------------------------------------------
# ★★★★★ THE GRADED LINES — column 0, so an anchored `^` read works, and UNCONDITIONAL, so a
#        missing measurement prints AS missing rather than as an absent line the reader fills
#        in with a zero.
#        ⊘ Every metric is re-emitted with a GUEST_ prefix. The workload's own B<N>_* lines
#          are already in the verbatim block above; prefixing here is what lets the ratio
#          script tell a GUEST number from a NATIVE one when both logs are read together.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★★★ THE GUEST METRICS — anchored, one line per measured quantity ==="
for L in $($G 'grep -hE "^B[0-9]+_" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r' | tr ' ' '~'); do
  echo "GUEST_$(printf '%s' "$L" | tr '~' ' ')"
done
$G 'grep -h "^BSUM " /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r' | sed 's/^/GUEST_/'
# ★★★ w320 — the sweep rows, at column 0 with a GUEST_ prefix, exactly as BSUM is. ⊘ Emitted
#   UNCONDITIONALLY: with the sweep disarmed there are no rows and the count line says 0, which
#   is a measured absence rather than a line the reader has to notice is missing.
$G 'grep -h "^BSWEEPSUM " /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r' | sed 's/^/GUEST_/'
$G 'grep -h "^BSWEEP "    /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r' | sed 's/^/GUEST_/'
echo "GUEST_BSWEEPSUM_ROWS=$($G 'grep -c "^BSWEEPSUM " /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r\n ')"
echo "GUEST_BENCH_W320=$($G 'sed -n "s/^BENCH_W320 //p" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r')"
# ⊘ If the guest kernel has no per-thread CPU clock the whole thread-state breakdown is
#   meaningless. It must be read as a PRECONDITION, not discovered as a column of zeros.
echo "GUEST_TSTATE_AVAILABLE=$($G 'sed -n "s/^BENCH_TSTATE_AVAILABLE=\([a-zA-Z]*\).*/\1/p" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r\n ' | sed 's/^$/NO_LINE/')"
echo "GUEST_BENCH_DEVICE=$($G 'sed -n "s/^BENCH_DEVICE=//p" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r')"
echo "GUEST_BENCH_INIT_MS=$($G 'sed -n "s/^BENCH_INIT_MS=//p" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r')"
echo "GUEST_BENCH_CTX_MS=$($G 'sed -n "s/^BENCH_CTX_MS=//p" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r')"
echo "GUEST_BENCH_MODULE_MS=$($G 'sed -n "s/^BENCH_MODULE_MS=//p" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r')"
TB=$($G 'sed -n "s/^BENCH_TOTAL_BAD=//p" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r\n ')
echo "GUEST_BENCH_TOTAL_BAD=${TB:-NO_LINE}"
echo "GUEST_BENCH_VERDICT=$($G 'sed -n "s/^BENCH_VERDICT: //p" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r' | sed 's/^$/ABSENT/')"
echo "GUEST_SIZES_DONE=$($G 'sed -n "s/^BENCH_SIZES_DONE=//p" /tmp/measure.out 2>/dev/null' 2>/dev/null | tr -d '\r\n ' | sed 's/^$/NO_LINE/')"
echo "    ⊘ GUEST_SIZES_DONE counts sizes that reached their own summary. A size that was cut"
echo "      off by the deadline is UNMEASURED — it is not a slow measurement, it is none."

echo ""
echo "=== ★ THE STAGE LADDER — how far it got (furthest ✔ before the first ✘ is the wall) ==="
for s in "BENCH_MODE" "BENCH_INIT_MS" "BENCH_DEVICE" "BENCH_CTX_MS" "BENCH_MODULE_MS" \
         "BENCH_SIZE_BEGIN" "B512_H2D_MS" "ITER N=512" "BSUM N=512" \
         "BSUM N=1024" "BSUM N=2048" "BENCH_TOTAL_BAD" "DONE"; do
  if $G "grep -q '^$s' /tmp/measure.out 2>/dev/null"; then echo "    ✔ $s"; else echo "    ✘ $s"; fi
done
echo "    ⊘ ALL ✘ means the output file is empty — a state that needs its own check, NOT 'not yet'."
echo "=== ★ the FAIL line, if the workload named its own failure ==="
$G 'grep -h "^FAIL" /tmp/measure.out 2>/dev/null' | sed 's/^/    /'
fi

# =======================================================================================
# ★★★★★ THE NEGATIVE CONTROL — second in arm `both`, FIRST AND ONLY in arm `negctrl`.
# =======================================================================================
if [ "$B_ONLY" = measure ]; then
  echo ""
  echo "=== ⊘ NEGATIVE CONTROL SKIPPED (KAYFABE_BENCH_ONLY=measure) — the measurement above"
  echo "    is UNGUARDED by construction. Say so; do not report it as guarded."
  echo "GUEST_NEGCTRL_TOTAL_BAD=SKIPPED_BY_ARM"
else
# ★★★ w320 — THE NEGATIVE CONTROL NOW EXERCISES THE SWEEP TOO.
#   The batch sweep is a NEW way for a launch to go missing, so it needs its own known-positive:
#   a batched loop that silently skips work is the easiest possible false green, and a control
#   that only covers the per-iteration phase would grade the sweep vacuously. With NOLAUNCH the
#   inner K-loop is skipped and EVERY BSWEEP row must carry bad>0.
run_detached negctrl "$NEG_TIMEOUT" \
  "BENCH_NOLAUNCH=1 BENCH_SIZES=256 BENCH_ITERS=3 BENCH_BATCH=2 BENCH_VERIFY=1 \
BENCH_BATCH_SWEEP=1,4 BENCH_BATCH_REPS=2"
echo ""
echo "=== ★ [negctrl] OUTPUT (the lines that matter), verbatim and INDENTED ==="
$G 'grep -hE "^BENCH_MODE|^ITER |^B256_BAD=|^B256_MAXERR=|^BENCH_NOLAUNCH_TOTAL_BAD=|^BENCH_VERDICT|^BSWEEP" /tmp/negctrl.out 2>/dev/null' \
  | tr -d '\r' | sed 's/^/    /'
# ★★ THE SWEEP'S OWN INVERTED GRADE, separate from the program total: a sweep row with bad=0
#    under NOLAUNCH means THAT ROW's verification is vacuous, even if other rows fired.
SWZ=$($G 'grep -h "^BSWEEP " /tmp/negctrl.out 2>/dev/null | grep -c "bad=0$"' 2>/dev/null | tr -d '\r\n ')
SWT=$($G 'grep -c "^BSWEEP " /tmp/negctrl.out 2>/dev/null' 2>/dev/null | tr -d '\r\n ')
echo "GUEST_NEGCTRL_SWEEP_ROWS=${SWT:-NO_LINE}"
echo "GUEST_NEGCTRL_SWEEP_ROWS_WITH_BAD_0=${SWZ:-NO_LINE}"
echo "    ⊘ INVERTED: rows_with_bad_0 must be 0 and rows must be >0. rows=0 is UNMEASURED"
echo "      (the sweep never ran), NOT a pass — an absent row grades nothing."
NCB=$($G 'sed -n "s/^BENCH_NOLAUNCH_TOTAL_BAD=//p" /tmp/negctrl.out 2>/dev/null' 2>/dev/null | tr -d '\r\n ')
echo "GUEST_NEGCTRL_TOTAL_BAD=${NCB:-NO_LINE}"
echo "=== ★★★★★ WHAT THE NEGATIVE CONTROL MEANS — the grading here is INVERTED ==="
case "${NCB:-__none__}" in
  __none__) echo "    ⊘ UNMEASURED — the control never printed its own verdict. It did NOT pass,"
            echo "      and it did NOT fail. The measurement above is therefore UNGUARDED: a"
            echo "      bad=0 from it cannot be distinguished from a verifier that never fired." ;;
  0)        echo "    ✘✘ THE VERIFIER IS DEAD IN THE GUEST. With EVERY launch skipped it still"
            echo "       reported bad=0 — so the poison fill did not land, C held stale correct"
            echo "       data, and ⊘ THE MEASUREMENT'S bad=0 IS VACUOUS. Report it as such." ;;
  *)        echo "    ✔ THE VERIFIER FIRED IN THE GUEST (bad=$NCB with no launches). Both the"
            echo "      poison fill and the readback are live on our plane, so the measurement's"
            echo "      per-iteration bad=0 is a real assertion and not an absence." ;;
esac
fi

echo ""
echo "=== ★ guest dmesg tail (NVRM/Xid land here, not in the serial log) ==="
$G 'sudo dmesg | tail -30' | sed 's/^/    /'
echo "GUEST_XID_COUNT=$($G 'sudo dmesg | grep -c Xid' 2>/dev/null | tr -d '\r')"
echo "=== cup8bench hook DONE ==="
