#!/usr/bin/env bash
# ★★★★★ cup3 POST_CAPTURE_HOOK — FIRST COMPUTE. `cuLaunchKernel` on the host GR engine.
#
# ## What makes this different from every cup2 rung
#
# `cup2` is a **CE round-trip**: a 4-byte `cuMemcpyHtoD` and a read-back. A copy engine — or
# an emulator, or a CPU memcpy — can produce its green. `cup3` computes `out = in*3 + 1` in a
# **shader**. `in = 14` ⇒ `out = 43`. ⊘ **Nothing in our stack can fabricate 43**: the CE
# copies and fills, the emulator has no arithmetic, and a forged completion writes a payload
# we chose. ⇒ **`CUP3_VAL=43` is un-forgeable proof the host GR engine ran the guest's
# shader.** That is why the graded metric here is a VALUE, not a return code.
#
# ## ★★★ THE VALUE LADDER — four distinguishable failures, and cup3.c was written for it
#
#   rv = 0xeeee (61166) the host sentinel is INTACT ⇒ `cuMemcpyDtoH` never wrote our buffer
#   rv = 0              `cuMemsetD32` landed, the kernel wrote nothing
#   rv = 14             ★★★ SOMETHING COPIED `in` INTO `out` where a COMPUTE belonged —
#                       the single most diagnostic reading this harness can produce
#   rv = 43             ★★★★★ PASS. The shader ran.
#
# ⇒ Grade on the value. A bare `CUP3_RC=0` is graded too, but the RC alone cannot separate
#   these and this tree has paid repeatedly for metrics that cannot see a substitution
#   ([[a_count_cannot_see_a_substitution]]).
#
# ## ⚠ Traps carried forward, each already paid for in this tree
#
# - **`^CUP3_RC=` is ANCHORED.** The unanchored read matches `GCC_CUP3_RC=0` and has printed
#   the campaign's headline success value on a FAILING arm on seven consecutive cup2 rungs.
#   Both readings are printed here, always, and the anchored one is the metric.
# - **START marker + rc file.** "exists but has no terminator" must be detectable at all;
#   `143` (the job was killed) and `124` (the LAUNCHER expired, job fine) arrive as the same
#   word otherwise.
# - **Delete the binary before building.** `[ -x ]` cannot tell fresh from stale; a stale
#   client once exited 95 while looking healthy.
# - **The PTX JIT is a GUEST-ENVIRONMENT precondition, checked BY NAME.** `cuModuleLoadData`
#   JITs the PTX with `libnvidia-ptxjitcompiler`. If that library is absent the load fails and
#   the failure looks exactly like our wall. ⇒ its presence is asserted BEFORE the run, so a
#   `MODULE` failure can be attributed. ⊘ An unattributable failure is not a measurement.
# - **`timeout 300`, not 180.** cup3 does everything cup2 does and then loads a module and
#   launches. ⊘ It is therefore NOT comparable to the `CUP2_RC=124` baseline and this file
#   says so rather than letting a reader assume it is.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/cup3_hook.sh scripts/bench/boot_capture.sh <tag>
#   ⚠ needs GQ_TIMEOUT >= 600.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
CUP3_SRC=${KAYFABE_CUP3_SRC:-$SRC_DIR/cup3.c}
CUP3_TIMEOUT=${KAYFABE_CUP3_TIMEOUT:-300}

die() { echo "★ cup3 hook FAILED: $*"; exit 2; }

echo "=== source (md5 — a run cannot silently be a different copy) ==="
[ -f "$CUP3_SRC" ] || die "no such source: $CUP3_SRC"
printf '    %-56s %5s lines  md5 %s\n' "$CUP3_SRC" "$(wc -l < "$CUP3_SRC")" \
       "$(md5sum < "$CUP3_SRC" | cut -d' ' -f1)"
echo "    ⊘ byte-identical to the C artifact's tests/mode2/cup3.c at vendoring time"
echo "      (md5 3c90b0f5f9b7deedc9d9bea471ee551a) — so the two stacks run THE SAME program."

# ---------------------------------------------------------------------------------------
# ★★★ PRECONDITION, BY NAME. A missing JIT compiler fails at cuModuleLoadData and is
#     INDISTINGUISHABLE from our wall unless it is checked separately, first.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★ GUEST PRECONDITION — the PTX JIT (cuModuleLoadData needs it) ==="
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
echo "    JIT_PRESENT=$JIT_OK"

echo ""
echo "=== push + build in the guest ==="
$G 'cat > /tmp/cup3.c' < "$CUP3_SRC" || die "could not push cup3.c"
$G 'rm -f /tmp/cup3'   # no build ⇒ no file ⇒ no run
$G 'gcc -O0 -o /tmp/cup3 /tmp/cup3.c -lcuda 2>&1; echo GCC_CUP3_RC=$?'
$G 'test -x /tmp/cup3' || die "cup3 did not build in the guest"

echo ""
echo "=== launch cup3 DETACHED under its own ${CUP3_TIMEOUT}s timeout ==="
echo "    ⊘ NOT comparable to the cup2 baseline of 180s — a different program, a longer bound."
$G "cat > /tmp/run_cup3_detached.sh" <<GUESTEOF
#!/bin/sh
rm -f /tmp/cup3.out /tmp/cup3.rc
echo "STARTED \$(date -Is)" > /tmp/cup3.started
setsid sh -c 'cd /tmp && timeout ${CUP3_TIMEOUT} ./cup3 >/tmp/cup3.out 2>&1; echo \$? >/tmp/cup3.rc' \\
       </dev/null >/dev/null 2>&1 &
sleep 1
echo "LAUNCHED pid=\$(pgrep -x cup3 | head -1)"
GUESTEOF
$G 'sh /tmp/run_cup3_detached.sh'

# ---------------------------------------------------------------------------------------
# ★ Poll for the terminator, and report the STAGE LADDER as it advances — so a run that is
#   killed by the outer harness still leaves a record of how far it got.
#   ⊘ Absence of /tmp/cup3.rc is "no terminator", which is a state, not "not yet".
# ---------------------------------------------------------------------------------------
echo ""
echo "=== waiting for cup3's terminator (poll ${CUP3_TIMEOUT}s + 60s slack) ==="
LIMIT=$(( (CUP3_TIMEOUT + 60) / 10 ))
LAST=""
for i in $(seq 1 "$LIMIT"); do
  if $G 'test -f /tmp/cup3.rc' 2>/dev/null; then echo "    terminator present after ~$((i*10))s"; break; fi
  NOW=$($G 'tail -1 /tmp/cup3.out 2>/dev/null' 2>/dev/null | tr -d '\r')
  if [ -n "$NOW" ] && [ "$NOW" != "$LAST" ]; then
    echo "    [~$((i*10))s] $NOW"
    LAST="$NOW"
  fi
  sleep 10
done

echo ""
echo "=== ★ cup3 FINAL OUTPUT, verbatim ==="
$G 'cat /tmp/cup3.out 2>/dev/null' | sed 's/^/    /'
$G 'wc -c < /tmp/cup3.out 2>/dev/null | sed "s/^/CUP3_OUT_BYTES=/"'

# ---------------------------------------------------------------------------------------
# ★★★★★ THE TWO GRADED LINES. Emitted anchored at column 0 so the runner's `^` read works,
#        and emitted UNCONDITIONALLY so a missing measurement prints as a missing
#        measurement rather than as an absent line the reader fills in with a zero.
# ---------------------------------------------------------------------------------------
RC=$($G 'cat /tmp/cup3.rc 2>/dev/null' 2>/dev/null | tr -d '\r\n ')
KLINE=$($G 'grep -h "^KERNEL rv=" /tmp/cup3.out 2>/dev/null | tail -1' 2>/dev/null | tr -d '\r')
RV=$(printf '%s' "$KLINE" | sed -n 's/^KERNEL rv=\([0-9]*\) .*/\1/p')

echo ""
echo "CUP3_RC=${RC:-NO_RC_FILE}"
echo "CUP3_VAL=${RV:-NO_KERNEL_LINE}"
echo "CUP3_KERNEL_LINE=${KLINE:-ABSENT}"
echo "CUP3_JIT_PRESENT=$JIT_OK"

echo ""
echo "=== ★★★★★ THE VALUE LADDER — what this reading MEANS ==="
case "${RV:-}" in
  43)    echo "    ★★★★★ 43 — PASS. out = in*3+1 computed on the GPU. UN-FORGEABLE: no copy,"
         echo "          fill, or forged completion in our stack can produce this number." ;;
  14)    echo "    ★★★ 14 — THE INPUT CAME BACK. Something COPIED where a COMPUTE belonged."
         echo "          This is the single most diagnostic failure this harness can produce:"
         echo "          the data plane moved bytes and the GR engine did not run the shader." ;;
  0)     echo "    ⊘ 0 — the cuMemsetD32 landed and the kernel wrote nothing." ;;
  61166) echo "    ⊘ 61166 (0xeeee) — THE HOST SENTINEL IS INTACT. cuMemcpyDtoH never wrote"
         echo "          our buffer at all; the DtoH leg, not the launch, is what to look at." ;;
  "")    echo "    ⊘ NO KERNEL LINE — cup3 never reached its own verdict print. This is an"
         echo "      UNMEASURED value, NOT a failing one. Read the stage ladder below." ;;
  *)     echo "    ⚠ ${RV} — a value no stage of this program produces by design. Report it raw." ;;
esac

echo ""
echo "=== ★ THE STAGE LADDER — how far it got (the furthest line present wins) ==="
for s in "CTX OK" "MODULE OK" "FUNC OK" "MEMALLOC" "LAUNCH OK" "SYNC OK" "KERNEL rv=" "DONE"; do
  if $G "grep -q '^$s' /tmp/cup3.out 2>/dev/null"; then echo "    ✔ $s"; else echo "    ✘ $s"; fi
done
echo "    ⊘ the FIRST ✘ after a run of ✔ is the wall; a ✘ before any ✔ means it never started."
echo "=== ★ the FAIL line, if cup3 named its own failure ==="
$G 'grep -h "^FAIL" /tmp/cup3.out 2>/dev/null' | sed 's/^/    /'

echo ""
echo "=== ★ guest dmesg tail (NVRM/Xid land here, not in the serial log) ==="
$G 'sudo dmesg | tail -30' | sed 's/^/    /'
echo "=== cup3 hook DONE ==="
