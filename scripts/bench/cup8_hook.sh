#!/usr/bin/env bash
# ★★★★★ cup8 POST_CAPTURE_HOOK — REAL COMPUTE AT SCALE. The rung that removes cup3's caveat.
#
# ## What this adds over cup3, and why the metric had to change
#
# `cup3` proved the host GR engine runs the guest's shader: `out = in*3 + 1`, `in = 14`,
# `^CUP3_VAL=43`, un-forgeable. ⊘ **But it is a 1×1×1 launch of a six-instruction shader**,
# and every report of it has had to carry that caveat. `cup8` is an `N×N` fp32 matmul on a
# **2D grid of 16×16 blocks** — at the default `N=2048` that is **16 MiB per matrix, 48 MiB
# of device memory, 16 384 blocks, 4 194 304 threads**, each reading a full row of `A` and a
# full column of `B`. ⇒ The kernel genuinely touches EVERY byte of both inputs, so **a backing
# hole anywhere in 32 MiB shows up as a wrong value**, which is the property cup3 cannot test.
#
# ## ★★★ THE GRADED METRIC IS A NUMBER THE STACK CANNOT FABRICATE — and it is cup8's OWN
#
# cup8 computes its own verdict in `O(N^2)` against a closed form: `A[i][k]=(i&3)+1` and
# `B[k][j]=(j&3)+1` are each independent of `k`, so `C[i][j] = N*((i&3)+1)*((j&3)+1)`
# exactly in fp32. ⇒ the graded lines are **`^CUP8_BAD=` and `^CUP8_MAXERR=`**, read out of
# the workload's own `CUP8 RESULT` line. Same principle as `43`: **grade a value the stack
# cannot fabricate, not a return code.** A CE copy, a memset, a forged completion and an
# emulator all produce a *wrong* `C`; only the GR engine executing the guest's shader over a
# fully-backed VAS produces `bad=0`.
#
# ## ⊘⊘ `bad` IS AN EARLY-EXIT COUNT, NOT A TOTAL — read cup8.c's verifier before quoting it
#
#   `for(i=0; i<N && bad<8; i++) for(j=0; j<N; j++)`
#
# The `bad<8` guard is on the **OUTER** loop, so the inner loop always completes a full row.
# ⇒ `bad` can legitimately exceed 8, and the scan **stops at the first row boundary after 8
# mismatches** — it never examines the remaining rows. ⊘ Therefore:
#   * `bad=0`      IS a whole-matrix statement — every one of the N² elements was checked.
#   * `bad>0`      is NOT a mismatch total, and `maxerr` is NOT a whole-matrix maximum. Both
#                  are bounded by however many rows were scanned before the guard tripped.
# This hook prints that caveat next to the number so it cannot be quoted as a total. ⚠ It is
# the same class this tree keeps paying for: a partial reading that carries no marker
# distinguishing it from a complete one.
#
# ## ★★★ THE VALUE LADDER — what each reading MEANS
#
#   bad=0                  ★★★★★ (A) REAL COMPUTE AT SCALE. Every element of an N×N matmul
#                          correct ⇒ the GR engine ran a 16 384-block grid and the address
#                          plane held over all 48 MiB.
#   bad>0, maxerr large    ★★★ (B) THE PLANE RAN AND PRODUCED WRONG DATA — the most
#                          interesting failure available. Read `FIRST bad` and `C[0]`:
#                          got=0 says an unbacked/zero leaf; got=garbage says a mis-mapped
#                          one; a SYSTEMATIC offset and a RANDOM scatter are different
#                          diagnoses and this hook prints the evidence for both.
#   no RESULT line         ⊘ (C)/(D) UNMEASURED — the run did not reach its own verdict.
#                          ⊘ NOT `bad=0` and NOT a failure value. Read the STAGE LADDER.
#
# ## ⚠ Traps carried forward, each already paid for in this tree
#
# - **`^CUP8_BAD=` is ANCHORED**, and the runner prints an INDENT-TOLERANT read beside it.
#   ⊘⊘ The inverted form bit this tree at w307: a grader printed *"⊘ UNMEASURED"* while its
#   own verbatim block six lines above printed the value, because the hook INDENTS workload
#   output. *"Unmeasured" is the reading this repo treats as safe, so it gets believed.*
#   ⇒ every graded line here is emitted at **column 0**, unindented, unconditionally.
# - **START marker + rc file.** `143` (the job was killed) and `124` (the LAUNCHER expired
#   while the job ran on fine) arrive as the same word without a terminator to check.
# - **Delete the binary before building.** `[ -x ]` cannot tell fresh from stale.
# - **The PTX JIT is a GUEST-ENVIRONMENT precondition, checked BY NAME**, so a MODULE-stage
#   failure is attributable. ⊘ An unattributable failure is not a measurement.
# - **The poll loop is DEADLINE-based, not iteration-based.** cup3's loop counted iterations
#   and assumed each cost ~10 s; every `gssh` round trip through the ProxyJump actually costs
#   ~1 s on top, so an iteration-counted loop covers **less wall time than its own name
#   claims** — and on a rung whose whole question is *how long does it take*, a waiter that
#   silently under-waits would manufacture outcome (D).
# - **A LONG RUN IS NOT A HANG.** The timeout is picked from the workload, not inherited:
#   see `KAYFABE_CUP8_TIMEOUT` below for the derivation and the number.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/cup8_hook.sh scripts/bench/boot_capture.sh <tag>
#   ⚠ needs GQ_TIMEOUT > KAYFABE_CUP8_TIMEOUT, and w290p_run.sh's hardcoded
#     `timeout 1800 boot_capture.sh` is the true outer bound — do not exceed ~1500 here.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
CUP8_SRC=${KAYFABE_CUP8_SRC:-$SRC_DIR/cup8.c}

# ★★★ THE TIMEOUT, DERIVED — ⚠ NOT inherited from cup3's 300 s.
#   cup3's whole boot (build cached, boot, run, grade) took 116 s and cup3 itself is
#   sub-second. cup8 moves ~8×10^6 times more data through the HtoD/DtoH legs (32 MiB in,
#   16 MiB out, vs 4 bytes each way), allocates 48 MiB of device memory, and launches 16 384
#   blocks. The GR work itself is trivial for a GA106 (8.6 GFMA, ~ms). ⊘ The unknown is the
#   COPY path and the publication cost, neither of which has ever been measured at this size.
#   1200 s is ~10× a linear-in-bytes projection of the copy legs and still leaves ~500 s of
#   the outer 1800 s bound for boot + grading. A run that needs more than 1200 s has answered
#   pre-registered outcome (D) anyway, and the stage ladder says where the time went.
CUP8_TIMEOUT=${KAYFABE_CUP8_TIMEOUT:-1200}
# ⊘ N is NOT overridden by default: the C research artifact ran cup8 at ITS default N=2048 to
#   `bad=0 maxerr=0`, and changing the size would forfeit that comparison. `KAYFABE_CUP8_N`
#   exists so a follow-up can ablate size deliberately, and it is REPORTED either way.
CUP8_N=${KAYFABE_CUP8_N:-}

die() { echo "★ cup8 hook FAILED: $*"; exit 2; }

echo "=== source (md5 — a run cannot silently be a different copy) ==="
[ -f "$CUP8_SRC" ] || die "no such source: $CUP8_SRC"
printf '    %-56s %5s lines  md5 %s\n' "$CUP8_SRC" "$(wc -l < "$CUP8_SRC")" \
       "$(md5sum < "$CUP8_SRC" | cut -d' ' -f1)"
# ★ A REAL COMPARISON, printed either way.
#   ⊘⊘ w305 measured what the UNCONDITIONAL form costs: cup3_hook.sh printed *"byte-identical
#   to the C artifact's tests/mode2/cup3.c (md5 3c90b0f5…)"* three lines under a source line
#   reporting a DIFFERENT md5. A provenance claim that is TEMPLATE TEXT rather than a
#   comparison asserts nothing, and is worse than silence because it looks verified.
CUP8_VENDORED_MD5=593c1ef978503af6a6a67c58430dda5b
CUP8_THIS_MD5=$(md5sum < "$CUP8_SRC" | cut -d' ' -f1)
if [ "$CUP8_THIS_MD5" = "$CUP8_VENDORED_MD5" ]; then
  echo "    ★ byte-identical to the C artifact's tests/mode2/cup8.c at vendoring time"
  echo "      (md5 $CUP8_VENDORED_MD5) — so THE SAME PROGRAM the C ran to bad=0 maxerr=0."
  CUP8_SAME_PROGRAM=yes
else
  echo "    ⊘ NOT the vendored cup8.c: this source is md5 $CUP8_THIS_MD5, the C artifact's"
  echo "      tests/mode2/cup8.c is md5 $CUP8_VENDORED_MD5. ⇒ this run is a VARIANT and the"
  echo "      two stacks are NOT running the same program. Read every comparison accordingly."
  CUP8_SAME_PROGRAM=no
fi
echo "CUP8_SAME_PROGRAM=$CUP8_SAME_PROGRAM"

# ---------------------------------------------------------------------------------------
# ★★★ PRECONDITIONS, BY NAME. Each of these fails in a way that is INDISTINGUISHABLE from
#     our wall unless it is checked separately, first.
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
echo "CUP8_JIT_PRESENT=$JIT_OK"

echo ""
echo "=== ★★ GUEST PRECONDITION 2 — HOST-SIDE RAM. cup8 mallocs 3 x N^2 x 4 bytes ==="
echo "    ⊘ At N=2048 that is 48 MiB of GUEST RAM in a 2048 MiB guest. If malloc fails cup8"
echo "      prints 'OOM host' and exits 1 — a GUEST-IMAGE failure, not our address plane."
$G 'free -m | head -2' 2>&1 | tr -d '\r' | sed 's/^/    /'
echo "GUEST_MEMFREE_MB=$($G 'free -m | sed -n "s/^Mem: *[0-9]* *[0-9]* *\([0-9]*\).*/\1/p"' 2>/dev/null | tr -d '\r\n ')"

echo ""
echo "=== push + build in the guest ==="
$G 'cat > /tmp/cup8.c' < "$CUP8_SRC" || die "could not push cup8.c"
$G 'rm -f /tmp/cup8'   # ★ no build => no file => no run. `[ -x ]` cannot tell fresh from stale.
$G 'gcc -O0 -o /tmp/cup8 /tmp/cup8.c -lcuda -lm 2>&1; echo GCC_CUP8_RC=$?'
$G 'test -x /tmp/cup8' || die "cup8 did not build in the guest"
$G 'md5sum /tmp/cup8.c' 2>&1 | tr -d '\r' | sed 's/^/    guest-side source md5: /'

echo ""
echo "=== launch cup8 DETACHED under its own ${CUP8_TIMEOUT}s timeout ==="
echo "    ⊘ NOT comparable to cup3's 300s and NOT to the cup2 baseline of 180s — a different"
echo "      program at a different size, with a bound DERIVED from it (see header)."
echo "    CUP8_N_OVERRIDE=[${CUP8_N:-<unset — cup8 uses its OWN default, N=2048>}]"
$G "cat > /tmp/run_cup8_detached.sh" <<GUESTEOF
#!/bin/sh
rm -f /tmp/cup8.out /tmp/cup8.rc
echo "STARTED \$(date -Is) epoch=\$(date +%s)" > /tmp/cup8.started
setsid sh -c 'cd /tmp && ${CUP8_N:+CUP8_N=$CUP8_N }timeout ${CUP8_TIMEOUT} ./cup8 >/tmp/cup8.out 2>&1; echo \$? >/tmp/cup8.rc; date +%s >/tmp/cup8.endepoch' \\
       </dev/null >/dev/null 2>&1 &
sleep 1
echo "LAUNCHED pid=\$(pgrep -x cup8 | head -1)"
GUESTEOF
$G 'sh /tmp/run_cup8_detached.sh'
$G 'cat /tmp/cup8.started' 2>&1 | tr -d '\r' | sed 's/^/    /'

# ---------------------------------------------------------------------------------------
# ★★★ THE POLL — DEADLINE-BASED, and it TIMESTAMPS EVERY STAGE TRANSITION.
#
#   This is the instrument pre-registered outcome (D) needs: "it works but is unusably slow"
#   is only a full result if it says WHERE THE TIME WENT. cup8 fflushes after every stage, so
#   the tail of its output IS a progress cursor; sampling it against elapsed wall time turns
#   that cursor into a per-stage timing ladder at ~5 s resolution.
#   ⊘ Absence of /tmp/cup8.rc is "no terminator", which is a STATE, not "not yet".
# ---------------------------------------------------------------------------------------
echo ""
echo "=== waiting for cup8's terminator — deadline $((CUP8_TIMEOUT + 120))s from now ==="
T0=$(date +%s)
DEADLINE=$(( T0 + CUP8_TIMEOUT + 120 ))
LAST=""
TERM_SEEN=no
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  if $G 'test -f /tmp/cup8.rc' 2>/dev/null; then
    echo "    [+$(( $(date +%s) - T0 ))s] ★ terminator present"
    TERM_SEEN=yes
    break
  fi
  NOW=$($G 'tail -1 /tmp/cup8.out 2>/dev/null' 2>/dev/null | tr -d '\r')
  if [ -n "$NOW" ] && [ "$NOW" != "$LAST" ]; then
    echo "    [+$(( $(date +%s) - T0 ))s] $NOW"
    LAST="$NOW"
  fi
  sleep 5
done
WAITED=$(( $(date +%s) - T0 ))
echo "CUP8_TERMINATOR_SEEN=$TERM_SEEN"
echo "CUP8_HOOK_WAITED_S=$WAITED"
if [ "$TERM_SEEN" = no ]; then
  echo "    ⊘⊘ NO TERMINATOR within the deadline. ⚠ Before reading this as a hang, note the"
  echo "       distinction this tree has paid for: a LAUNCHER expiring (124) and the JOB being"
  echo "       killed (143) arrive as the same word. Liveness is checked DIRECTLY below."
  $G 'pgrep -x cup8 >/dev/null && echo "CUP8_STILL_ALIVE=yes pid=$(pgrep -x cup8|head -1)" || echo "CUP8_STILL_ALIVE=no"' \
     2>&1 | tr -d '\r'
fi

echo ""
echo "=== ★ cup8 FINAL OUTPUT, verbatim (⚠ INDENTED — the graded lines below are NOT) ==="
$G 'cat /tmp/cup8.out 2>/dev/null' | sed 's/^/    /'
$G 'wc -c < /tmp/cup8.out 2>/dev/null | sed "s/^/CUP8_OUT_BYTES=/"' 2>&1 | tr -d '\r'

# ---------------------------------------------------------------------------------------
# ★★★★★ THE GRADED LINES. Column 0, so the runner's `^` read works. UNCONDITIONAL, so a
#        missing measurement prints AS a missing measurement rather than as an absent line
#        the reader fills in with a zero.
# ---------------------------------------------------------------------------------------
RC=$($G 'cat /tmp/cup8.rc 2>/dev/null' 2>/dev/null | tr -d '\r\n ')
RLINE=$($G 'grep -h "^CUP8 RESULT " /tmp/cup8.out 2>/dev/null | tail -1' 2>/dev/null | tr -d '\r')
VLINE=$($G 'grep -h "^CUP8 VERDICT: " /tmp/cup8.out 2>/dev/null | tail -1' 2>/dev/null | tr -d '\r')
SLINE=$($G 'grep -h "^CUP8 matmul " /tmp/cup8.out 2>/dev/null | tail -1' 2>/dev/null | tr -d '\r')
GLINE=$($G 'grep -h "^LAUNCH grid=" /tmp/cup8.out 2>/dev/null | tail -1' 2>/dev/null | tr -d '\r')
MLINE=$($G 'grep -h "^MEMALLOC " /tmp/cup8.out 2>/dev/null | tail -1' 2>/dev/null | tr -d '\r')
FBAD=$($G 'grep -h "FIRST bad " /tmp/cup8.out 2>/dev/null | tail -1' 2>/dev/null | tr -d '\r' | sed 's/^ *//')

# ⚠ Parsed with `sed -n .../p` so a NON-MATCH yields EMPTY, never a partial or a zero.
NN=$(printf '%s' "$RLINE"     | sed -n 's/^CUP8 RESULT N=\([0-9]*\) .*/\1/p')
BAD=$(printf '%s' "$RLINE"    | sed -n 's/.* bad=\([0-9-]*\) .*/\1/p')
MAXERR=$(printf '%s' "$RLINE" | sed -n 's/.* maxerr=\([^ ]*\) .*/\1/p')
C0=$(printf '%s' "$RLINE"     | sed -n 's/.* C\[0\]=\([^(]*\)(.*/\1/p')

# ★ The workload's own wall time, from the guest's own clock — independent of the poll loop.
SEPOCH=$($G 'sed -n "s/.*epoch=//p" /tmp/cup8.started 2>/dev/null' 2>/dev/null | tr -d '\r\n ')
EEPOCH=$($G 'cat /tmp/cup8.endepoch 2>/dev/null' 2>/dev/null | tr -d '\r\n ')
if [ -n "$SEPOCH" ] && [ -n "$EEPOCH" ]; then WALL=$(( EEPOCH - SEPOCH )); else WALL=""; fi

echo ""
echo "CUP8_RC=${RC:-NO_RC_FILE}"
echo "CUP8_BAD=${BAD:-NO_RESULT_LINE}"
echo "CUP8_MAXERR=${MAXERR:-NO_RESULT_LINE}"
echo "CUP8_N=${NN:-NO_RESULT_LINE}"
echo "CUP8_C0=${C0:-NO_RESULT_LINE}"
echo "CUP8_WALL_S=${WALL:-NO_TERMINATOR}"
echo "CUP8_RESULT_LINE=${RLINE:-ABSENT}"
echo "CUP8_VERDICT_LINE=${VLINE:-ABSENT}"
echo "CUP8_SIZE_LINE=${SLINE:-ABSENT}"
echo "CUP8_GRID_LINE=${GLINE:-ABSENT}"
echo "CUP8_MEMALLOC_LINE=${MLINE:-ABSENT}"
echo "CUP8_FIRSTBAD_LINE=${FBAD:-ABSENT}"

echo ""
echo "=== ★★★★★ THE VALUE LADDER — what this reading MEANS ==="
case "${BAD:-__none__}" in
  __none__)
      echo "    ⊘ NO 'CUP8 RESULT' LINE — cup8 never reached its own verdict print."
      echo "      ⊘⊘ This is an UNMEASURED value. It is NOT bad=0 and NOT a failing one."
      echo "      Read the STAGE LADDER below: the first ✘ after a run of ✔ is the wall." ;;
  0)  echo "    ★★★★★ bad=0 — REAL COMPUTE AT SCALE. Every one of the N^2 elements matched the"
      echo "          closed form. ⊘ Note this direction IS a whole-matrix statement: the"
      echo "          early-exit guard only trips on a MISMATCH, so bad=0 means the verifier"
      echo "          scanned all N rows. The GR engine ran a full grid and the address plane"
      echo "          held over every byte of A, B and C."
      echo "          maxerr=${MAXERR:-?} (0 => bit-exact, not merely within tolerance)." ;;
  *)  echo "    ★★★ bad=${BAD} — (B) THE PLANE RAN AND PRODUCED WRONG DATA. The most"
      echo "        interesting failure this rung can produce: the launch happened, the"
      echo "        readback happened, and the numbers are wrong."
      echo "        ⊘⊘ bad=${BAD} IS NOT A MISMATCH TOTAL. cup8.c's verifier is"
      echo "           'for(i=0; i<N && bad<8; i++) for(j=0; j<N; j++)' — the guard is on the"
      echo "           OUTER loop, so it stops at the first ROW BOUNDARY after 8 mismatches and"
      echo "           never scans the remaining rows. maxerr=${MAXERR:-?} is likewise a maximum"
      echo "           over SCANNED ROWS ONLY. Quote neither as a whole-matrix figure."
      echo "        ★ THE DIAGNOSIS SPLITS — read FIRST bad and C[0] together:"
      echo "          got=0            => an UNBACKED or zeroed leaf; the store never landed or"
      echo "                              the load read a hole. A SYSTEMATIC diagnosis."
      echo "          got=k*expected   => a partial-sum / truncated-k diagnosis: the loop read"
      echo "                              only part of A's row or B's column."
      echo "          got=garbage      => a MIS-MAPPED leaf: the VA resolved to someone else's"
      echo "                              page. A RANDOM SCATTER diagnosis."
      echo "          C[0] correct but later elements wrong => the FIRST block is fine and the"
      echo "                              grid's later blocks are not — a per-block or"
      echo "                              per-extent backing story, not a launch story."
      echo "        FIRST bad = [${FBAD:-⊘ ABSENT — cup8 prints it only for the first mismatch}]" ;;
esac

echo ""
echo "=== ★ THE STAGE LADDER — how far it got (the furthest ✔ before the first ✘ is the wall) ==="
for s in "CTX OK" "MODULE OK" "FUNC OK" "CUP8 matmul" "MEMALLOC" "LAUNCH grid=" \
         "LAUNCH OK" "SYNC OK" "CUP8 RESULT" "CUP8 VERDICT" "DONE"; do
  if $G "grep -q '^$s' /tmp/cup8.out 2>/dev/null"; then echo "    ✔ $s"; else echo "    ✘ $s"; fi
done
echo "    ⊘ a ✘ before any ✔ means it never started; ⊘ ALL ✘ means the output file is empty,"
echo "      which is a state that needs its own check and is NOT 'not yet'."
echo ""
echo "=== ★ THE PER-CALL LADDER — cup8 wraps EVERY CUDA call in CK() and prints it on success"
echo "    (⊘ so the LAST 'ok' line names the last call that RETURNED, and any 'FAIL' line"
echo "     names the exact call that did not — a finer wall than the stage ladder)"
$G 'grep -h "^ok   " /tmp/cup8.out 2>/dev/null | tail -12' | sed 's/^/    /'
echo "    ★ the FAIL line, if cup8 named its own failure:"
$G 'grep -h "^FAIL" /tmp/cup8.out 2>/dev/null' | sed 's/^/    /'

echo ""
echo "=== ★ guest dmesg tail (NVRM/Xid land here, not in the serial log) ==="
$G 'sudo dmesg | tail -30' | sed 's/^/    /'
echo "    ★ guest-side Xid count = [$($G 'sudo dmesg | grep -c Xid' 2>/dev/null | tr -d '\r')]"
echo "=== cup8 hook DONE ==="
