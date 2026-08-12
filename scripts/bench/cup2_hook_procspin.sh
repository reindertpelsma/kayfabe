#!/usr/bin/env bash
# ★★★★★ w274 POST_CAPTURE_HOOK — PROVE THE HANG IS A USERSPACE SPIN, WITH A DISTRIBUTION.
#
# ## The question, verbatim in intent (owner, 2026-08-12)
#
#   "I want to 100% know for sure that if cuCtxCreate is still hanging (confirm it's really
#    that call), that it must be hanging in a forever spin loop until the end on a semaphore,
#    not in a syscall or something else."
#
# ## ⊘ WHY THIS IS NOT `cup2_hook_gdbspin.sh`
#
# That hook takes TWO samples ~20 s apart and single-steps with ptrace. Two samples cannot
# distinguish "spinning" from "sampled twice while running". **A spin is a DISTRIBUTION.**
# This hook samples every thread of the process N times and reports the histogram, so
# "userspace" is a rate and not an anecdote.
#
# ⊘ And gdb is ABSENT from the guest and uninstallable — briefed twice, could not have worked
# either time. Everything here is `/proc`. No debugger, no ptrace, no single-stepping, and
# therefore no perturbation of the loop being measured.
#
# ## THE FOUR INSTRUMENTS, and what each can and cannot say
#
#  1. `/proc/<tid>/syscall` — "running" means USERSPACE. Anything else is `nr a0..a5 sp pc`
#     and the thread is in a syscall. ★ This is the decisive spin-vs-syscall instrument, and
#     it is the only one of the four that answers the question directly.
#  2. `utime`/`stime` deltas from `/proc/<tid>/stat` — a thread that burns user time and NO
#     system time over the whole window made no syscall that cost kernel time. ★ This is an
#     INTEGRAL over the interval, not a sample of it, so it cannot miss a syscall between two
#     samples the way (1) can. The two instruments fail differently and that is why both run.
#  3. `voluntary_ctxt_switches` from `/proc/<tid>/status` — a tight spin does not yield. A
#     loop built on `sched_yield` or a futex would show these climbing.
#  4. The polled words themselves, read out of the semaphore page each sample. ⚠ "never
#     changes" and "changes but never reaches the awaited value" are DIFFERENT BUGS. w270
#     measured it reaching 2 while the wait wanted 3; this prints the series, not a verdict.
#
# `wchan` is read for any thread that IS blocked, so a blocked thread is NAMED rather than
# counted. ⚠ Two helper threads are legitimately parked in `poll(2)` and are NOT the stuck
# thread — the per-thread enumeration exists so they can never be reported as the answer.
#
# ## PRE-REGISTERED REFUTATIONS — what would make the spin story WRONG
#
#   R1. the main thread is sampled in a syscall in a meaningful fraction of samples
#   R2. `stime` advances materially over the window (⇒ syscalls, whatever (1) sampled)
#   R3. `voluntary_ctxt_switches` climbs (⇒ it yields; not a tight spin)
#   R4. ★ the polled word is ALREADY at or past its awaited value (⇒ the wait is SATISFIED and
#       the hang is somewhere else — the biggest finding available here, and the one this
#       campaign has hit before: w270's "the wait was SATISFIED and RE-ARMED")
#   R5. no thread is in state R at all (⇒ not a spin in any sense)
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/cup2_hook_procspin.sh scripts/bench/boot_capture.sh <tag>
#   ⚠ needs GQ_TIMEOUT >= 420.
set -uo pipefail
G="$(cd "$(dirname "$0")" && pwd)/gssh_nv"
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
CUP2_SRC=${KAYFABE_CUP2_SRC:-/workspace/bench/cup2.c}
NSAMP=${NVD_NSAMP:-24}
SLEEP=${NVD_SAMPLE_SLEEP:-1}

die() { echo "★ w274 procspin hook FAILED: $*"; exit 2; }

echo "=== sources (md5 — a run cannot silently be a different copy) ==="
[ -f "$CUP2_SRC" ] || die "no such source: $CUP2_SRC"
printf '    %-56s %5s lines  md5 %s\n' "$CUP2_SRC" "$(wc -l < "$CUP2_SRC")" \
       "$(md5sum < "$CUP2_SRC" | cut -d' ' -f1)"

echo "=== push + build cup2 in the guest ==="
$G 'cat > /tmp/cup2.c' < "$CUP2_SRC" || die "could not push cup2.c"
$G 'gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_CUP2_RC=$?'
$G 'test -x /tmp/cup2' || die "cup2 did not build in the guest"

echo "=== launch cup2 DETACHED under its own 180 s timeout (rc stays comparable to w232..w271) ==="
$G 'cat > /tmp/run_cup2_detached.sh' <<'GUESTEOF'
#!/bin/sh
rm -f /tmp/cup2.out /tmp/cup2.rc
echo "STARTED $(date -Is)" > /tmp/cup2.started
setsid sh -c 'cd /tmp && timeout 180 ./cup2 >/tmp/cup2.out 2>&1; echo $? >/tmp/cup2.rc' \
       </dev/null >/dev/null 2>&1 &
sleep 1
echo "LAUNCHED pid=$(pgrep -x cup2 | head -1)"
GUESTEOF
$G 'sh /tmp/run_cup2_detached.sh'

# ---- gate: cup2's OWN last print before cuCtxCreate ---------------------------------------
echo "=== waiting for cup2 to reach cuCtxCreate (its last print is \`totalMem=\`) ==="
ARRIVED=no
for i in $(seq 1 24); do
  if $G 'grep -q totalMem= /tmp/cup2.out 2>/dev/null'; then ARRIVED=yes; break; fi
  if $G 'test -f /tmp/cup2.rc' 2>/dev/null; then
    echo "★★ cup2 RETURNED before reaching the spin — that is itself the headline."
    break
  fi
  sleep 5
done
echo "    reached-cuCtxCreate = $ARRIVED after ~$((i*5))s"

# ★★★ CLAIM 1, re-confirmed cheaply rather than re-derived: cup2's CK() prints AFTER the call
#     returns, so the last line of stdout names the last call that COMPLETED.
echo "=== ★ CLAIM 1: WHICH CALL IS IT? (cup2's own stdout — the print is post-return) ==="
$G 'echo "--- cup2 stdout so far ---"; cat /tmp/cup2.out 2>/dev/null;
    echo "--- last line ---"; tail -1 /tmp/cup2.out 2>/dev/null;
    echo "--- did any line announce cuCtxCreate completing? ---";
    grep -c "CTX OK\|ok   cuCtxCreate" /tmp/cup2.out 2>/dev/null'

if [ "$ARRIVED" != yes ]; then
  echo "⊘ NOT SAMPLING: cup2 never printed \`totalMem=\`. Sampling now would measure a"
  echo "  different part of the program and read as an answer to this rung's question."
  exit 0
fi

sleep 5   # let the spin settle past cuCtxCreate's own setup work

echo "=== ★★★★★ /proc SAMPLING: $NSAMP samples, ${SLEEP}s apart, EVERY thread ==="
$G "cat > /tmp/procspin.sh" <<GUESTEOF
#!/bin/sh
# ⊘ Written to a file rather than inlined so the guest's shell parses it once and the
#   sampling loop is not competing with ssh round-trips for its own timing.
N=$NSAMP
S=$SLEEP
GUESTEOF
$G 'cat >> /tmp/procspin.sh' <<'GUESTEOF'
p=$(pgrep -x cup2 | head -1)
if [ -z "$p" ]; then echo "⊘ cup2 is GONE — nothing to sample"; exit 0; fi
echo "PID=$p"
echo "--- the semaphore page mapping we will read from ---"
grep -n 'nvidiactl' /proc/$p/maps | head -20
# ⊘ DERIVE the slot-region offset from the process's own maps; do NOT hardcode it. The
#   polled slots have been at 0x2_0440ff00..0x2_0440fff0 on every boot since w267, but a
#   hardcoded constant would read a different process's arbitrary bytes without saying so.
SEMBASE=$(awk -F'[- ]' '/nvidiactl/ {
             lo = strtonum("0x" $1); hi = strtonum("0x" $2);
             if (lo <= 8661303040 && 8661303040 < hi) { print 8661303040; exit }
          }' /proc/$p/maps)
if [ -z "$SEMBASE" ]; then
  echo "⊘ NO SEMAPHORE READ: 0x20440ff00 is not inside any /dev/nvidiactl mapping of this"
  echo "  process. That is a RESULT, not a missing measurement — the slot page moved."
else
  echo "SEMBASE=$SEMBASE (0x$(printf '%x' $SEMBASE))"
fi
echo "--- BASELINE per-thread counters ---"
for t in /proc/$p/task/*; do
  tid=${t##*/}
  echo "BASE tid=$tid comm=$(cat $t/comm 2>/dev/null) stat=$(awk '{print $3}' $t/stat 2>/dev/null) utime=$(awk '{print $14}' $t/stat 2>/dev/null) stime=$(awk '{print $15}' $t/stat 2>/dev/null) vctx=$(awk '/voluntary_ctxt/{print $2}' $t/status 2>/dev/null | head -1) nvctx=$(grep nonvoluntary_ctxt $t/status 2>/dev/null | awk '{print $2}')"
done
echo "--- SAMPLES ---"
i=0
while [ $i -lt $N ]; do
  i=$((i+1))
  ts=$(awk '{print $1}' /proc/uptime)
  for t in /proc/$p/task/*; do
    tid=${t##*/}
    sc=$(cat $t/syscall 2>/dev/null | tr -s ' ')
    st=$(awk '{print $3}' $t/stat 2>/dev/null)
    wc=$(cat $t/wchan 2>/dev/null)
    echo "S i=$i t=$ts tid=$tid state=$st wchan=[$wc] syscall=[$sc]"
  done
  # the polled words: the whole 16-slot region of the semaphore page, each sample
  if [ -n "$SEMBASE" ]; then
    dd if=/proc/$p/mem bs=1 count=256 iflag=skip_bytes skip=$SEMBASE status=none 2>/dev/null \
       | od -An -tx4 -w16 | sed "s/^/SEM i=$i t=$ts /"
  fi
  sleep $S
done
echo "--- FINAL per-thread counters ---"
for t in /proc/$p/task/*; do
  tid=${t##*/}
  echo "FIN tid=$tid comm=$(cat $t/comm 2>/dev/null) stat=$(awk '{print $3}' $t/stat 2>/dev/null) utime=$(awk '{print $14}' $t/stat 2>/dev/null) stime=$(awk '{print $15}' $t/stat 2>/dev/null) vctx=$(awk '/voluntary_ctxt/{print $2}' $t/status 2>/dev/null | head -1) nvctx=$(grep nonvoluntary_ctxt $t/status 2>/dev/null | awk '{print $2}')"
done
echo "--- maps (for resolving any PC the syscall lines carried) ---"
cat /proc/$p/maps
echo "PROCSPIN_DONE rc=0"
GUESTEOF
$G 'sudo sh /tmp/procspin.sh 2>&1'

echo "=== cup2 final state ==="
$G 'for k in $(seq 1 40); do test -f /tmp/cup2.rc && break; sleep 5; done
    echo "--- cup2 stdout ---"; cat /tmp/cup2.out 2>/dev/null
    echo "CUP2_RC=$(cat /tmp/cup2.rc 2>/dev/null || echo NO_RC_FILE)"'

# ==========================================================================================
# PHASE 2 — the nvdiff GUEST capture, in the same boot window
# ==========================================================================================
# ★★★ The ioctl differential is stale by ~60 commits: its standing headline (lockstep for
# 221/479 of cuCtxCreate's ioctls, then 0x20801702 x175) predates w210, where we SERVED that
# control and cuCtxCreate stopped returning at all. The divergence point today is UNKNOWN.
#
# ⊘ The guest's nvd_prog will hang at cuCtxCreate exactly as cup2 does. That is not a failed
#   capture — the PARTIAL jsonl up to the hang IS the measurement, and the shim writes each
#   record as it happens rather than at teardown, so a killed process still leaves a usable
#   trace. The timeout is therefore a parameter, not an error.
# ⚠ It runs strictly AFTER cup2 has exited. Two CUDA processes at once would make the /proc
#   sampling above measure a different machine than the one it reported on.
if [ "${NVD_GUEST_CAPTURE:-1}" = 1 ] && [ -n "${NVDIFF_SRC_DIR:-}" ]; then
  echo "=== ★★★ PHASE 2: nvdiff GUEST capture (stage=ce) ==="
  $G 'pgrep -x cup2 >/dev/null && { echo "★★ cup2 STILL ALIVE — refusing to start a second"
      echo "   CUDA process; the capture would contend with the one just measured."; exit 3; }
      echo "cup2 is gone; the GPU is free for the capture"'
  $G 'mkdir -p /tmp/nvdiff'
  for f in nvdiff_shim.c nvd_prog.c nvd_capture.sh nvd_cuda_min.h uvm_sizes.h; do
    $G "cat > /tmp/nvdiff/$f" < "$NVDIFF_SRC_DIR/$f" || die "could not push $f"
  done
  # ⚠ NVD_MIN_CUDA=1 on BOTH sides. The host reference on `vh` was built with the stand-in
  #   header because that box has no toolkit; building the guest against a real cuda.h would
  #   make the two binaries differ, and the binary is supposed to be the constant.
  $G 'cd /tmp/nvdiff && NVD_MIN_CUDA=1 NVD_TIMEOUT=150 bash nvd_capture.sh /tmp/nvdiff/out ce 1 \
        > /tmp/nvdiff/capture.log 2>&1; echo "NVD_CAPTURE_RC=$?"
      echo "--- ENOSPC check from the SAME invocation ---"
      grep -c "No space left on device\|LLVM ERROR" /tmp/nvdiff/capture.log
      echo "--- symbol-binding gate + run summary ---"
      grep -E "^==|^   ok|^   prog|FATAL|NOT BOUND" /tmp/nvdiff/capture.log
      echo "--- nvd_prog stdout (the last ok line names the last call that COMPLETED) ---"
      cat /tmp/nvdiff/out/ce_r1.stdout 2>/dev/null
      echo "--- capture size ---"
      wc -l /tmp/nvdiff/out/ce_r1.jsonl 2>/dev/null || echo "NO JSONL"'
  echo "=== pulling the guest capture out ==="
  $G 'cat /tmp/nvdiff/out/ce_r1.jsonl 2>/dev/null'   > "${NVD_GUEST_OUT:-/workspace/bench/nvdiff_guest_ce_r1.jsonl}"
  $G 'cat /tmp/nvdiff/out/ce_r1.stdout 2>/dev/null'  > "${NVD_GUEST_OUT:-/workspace/bench/nvdiff_guest_ce_r1.jsonl}.stdout"
  $G 'cat /tmp/nvdiff/out/env_ce.txt 2>/dev/null'    > "${NVD_GUEST_OUT:-/workspace/bench/nvdiff_guest_ce_r1.jsonl}.env"
  echo "    guest jsonl lines = $(wc -l < "${NVD_GUEST_OUT:-/workspace/bench/nvdiff_guest_ce_r1.jsonl}" 2>/dev/null || echo 0)"
else
  echo "=== ⊘ PHASE 2 SKIPPED (NVD_GUEST_CAPTURE=${NVD_GUEST_CAPTURE:-1} NVDIFF_SRC_DIR=${NVDIFF_SRC_DIR:-unset})"
  echo "    An absent capture reads as favourable. It is absent because it was not run."
fi
