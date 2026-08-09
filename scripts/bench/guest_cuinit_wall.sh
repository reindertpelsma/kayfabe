#!/usr/bin/env bash
# POST_CAPTURE_HOOK: name the call `cuInit` actually fails on, on BOTH planes at once.
#
# ## ⊘⊘ WHY THIS EXISTS AND `guest_cuinit_trace.sh` DOES NOT ANSWER THE QUESTION
#
# §16.28.10 nominated `scripts/bench/guest_cuinit_trace.sh` as the instrument for "which call
# does `cuInit` fail on". ⊘ That script preloads `scripts/rpctrace/cuda_ioctl_trace.c` ONLY,
# and that file gates every decode on `_IOC_TYPE(request) != NV_IOCTL_MAGIC` (`'F'`, line 493)
# — while **UVM's ioctl magic is 0** (`UVM_IOCTL_BASE(i)` is literally `i`,
# `ogkm-580: uvm_ioctl.h:41`). So `/dev/nvidia-uvm` traffic passes that interposer untouched
# and a trace from it is SILENT on the plane §14.39/§14.40/§14.41 already measured `cuInit`
# to die on. ★ The brief that nominated it said so itself, one paragraph earlier, and still
# named the blind instrument — a warning is not a wiring change.
#
# ## ★ What is actually owed: a RE-READ, not a new instrument
#
# `scripts/bench/uvm_ioctl_trace.c` + `scripts/bench/guest_uvm_status.sh` were built at §14.40
# and DID answer the question: `UVM_REGISTER_GPU rmStatus` = `0x56` (`fb1503`), then `0x1f`
# (`sh1605`). ⊘ So "guest-side instrumentation is owed for six rungs" is false — what is owed
# is its VALUE AT THE CURRENT REVISION. This hook runs both interposers in ONE boot so the RM
# plane and the UVM plane are read from the same process, on the same clock.
#
# ## ★★ Both interposers, chained — and why that is sound rather than a coin flip
#
# Two `.so`s each defining `ioctl` do NOT shadow one another: each resolves its next hop with
# `dlsym(RTLD_NEXT, "ioctl")`, and `RTLD_NEXT` searches the load order AFTER the calling
# object. So `LD_PRELOAD=a.so:b.so` yields app -> a -> b -> libc, and each interposer passes
# through what it does not claim (`cuda_ioctl_trace.c:493`, `uvm_ioctl_trace.c:145`).
# ⊘ But "sound in principle" is not "ran": an interposer that COMPILES is not one that RUNS.
# Hence both trace files are ASSERTED non-empty below, and a missing one is reported as a
# failure of the instrument rather than as the absence of the traffic it watches.
#
# ## ⊘ Observation only
#
# No `NVFAULT_*` and no `NVSWEEP_GPUINFO`: `cuda_ioctl_trace.c` can MUTATE (inject a status,
# sweep an index) and every one of those is off here. This hook must be able to run beside a
# census claimed byte-identical to the previous boot's; a mutating instrument cannot.
#
# ## ⚠ Deadline, not `timeout`
#
# `cup2_hook_deadline.sh` records the cost of getting this wrong: when `cuInit` HANGS it spins
# in `uvm_spin_loop` — a busy spin in the kernel that takes no signal — so `timeout` blocks
# forever waiting for a child that can never die, the harness never returns, QEMU has to be
# `kill -9`ed and **the end-of-run census is lost**. `guest_uvm_status.sh` still uses bare
# `timeout 180`; this hook does not. Detached + poll on OUR clock, and when the deadline fires
# `/proc/<pid>/stack` is the measurement.
set -uo pipefail
REPO=${KAYFABE_REPO:-/workspace/bench/kayfabe}
G="$REPO/scripts/bench/gssh_nv"
DEADLINE=${CUP2_DEADLINE:-150}
CUP2_SRC=${KAYFABE_CUP2_SRC:-/workspace/bench/cup2.c}
RM_SRC=$REPO/scripts/rpctrace/cuda_ioctl_trace.c
UVM_SRC=$REPO/scripts/bench/uvm_ioctl_trace.c

die() { echo "★ guest_cuinit_wall hook FAILED: $*"; exit 2; }
stamp() { # ⊘ WHICH COPY RAN — the trap that made §14.29's bisect unreachable for two rungs.
  [ -f "$1" ] || die "no such source: $1"
  printf '    %-58s %5s lines  md5 %s\n' "$1" "$(wc -l < "$1")" "$(md5sum < "$1" | cut -d' ' -f1)"
}

echo "=== sources that will run (md5, so a run cannot silently be the other copy) ==="
stamp "$CUP2_SRC"; stamp "$RM_SRC"; stamp "$UVM_SRC"

echo "=== push + build inside the guest ==="
$G 'cat > /tmp/cup2.c'             < "$CUP2_SRC" || die "could not push cup2.c"
$G 'cat > /tmp/cuda_ioctl_trace.c' < "$RM_SRC"   || die "could not push the RM interposer"
$G 'cat > /tmp/uvm_ioctl_trace.c'  < "$UVM_SRC"  || die "could not push the UVM interposer"
$G 'gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_CUP2_RC=$?'
$G 'gcc -O0 -fPIC -shared -o /tmp/cuda_ioctl_trace.so /tmp/cuda_ioctl_trace.c -ldl 2>&1; echo GCC_RM_RC=$?'
$G 'gcc -O1 -fPIC -shared -o /tmp/uvm_ioctl_trace.so  /tmp/uvm_ioctl_trace.c  -ldl 2>&1; echo GCC_UVM_RC=$?'
$G 'test -x /tmp/cup2 && test -s /tmp/cuda_ioctl_trace.so && test -s /tmp/uvm_ioctl_trace.so' \
  || die "cup2 or one of the two interposers did not build"

# ⊘ Clear the ring buffer so what follows is `cuInit`'s ALONE. The device-open output (RC
# watchdog, CeUtils scrubber, the I2C `0x402c` refusal) otherwise drowns it, and has already
# been misread as `cuInit`'s once in this campaign.
echo "=== clear dmesg, then run cup2 under BOTH interposers (detached, deadline ${DEADLINE}s) ==="
$G 'sudo dmesg -C && echo CLEARED'
$G 'cat > /tmp/run_traced.sh' <<'GUESTEOF'
#!/bin/sh
rm -f /tmp/cup2.out /tmp/cup2.rc /tmp/rmtrace.txt /tmp/uvm.txt
setsid sh -c 'LD_PRELOAD=/tmp/cuda_ioctl_trace.so:/tmp/uvm_ioctl_trace.so \
              NVTRACE_OUT=/tmp/rmtrace.txt UVMTRACE_OUT=/tmp/uvm.txt \
              /tmp/cup2 >/tmp/cup2.out 2>/tmp/cup2.err; echo $? >/tmp/cup2.rc' \
       </dev/null >/dev/null 2>&1 &
sleep 1
echo "LAUNCHED pid=$(pgrep -x cup2 | head -1)"
GUESTEOF
$G 'sh /tmp/run_traced.sh'

done=0
for _ in $(seq 1 $((DEADLINE / 5))); do
  if $G 'test -f /tmp/cup2.rc' 2>/dev/null; then done=1; break; fi
  sleep 5
done

if [ "$done" = 1 ]; then
  $G 'cat /tmp/cup2.out; echo CUP2_RC=$(cat /tmp/cup2.rc)'
else
  echo "★★★ cup2 DID NOT RETURN within ${DEADLINE}s — this is a HANG, not a failure."
  echo "CUP2_RC=TIMEOUT"
  $G 'cat /tmp/cup2.out 2>/dev/null
      p=$(pgrep -x cup2 | head -1); echo "pid=$p state=$(ps -o stat= -p $p 2>/dev/null)"
      for i in 1 2 3; do echo "--- stack sample $i ---"; sudo cat /proc/$p/stack 2>&1; sleep 1; done'
fi

# ---- ★★ DID THE INSTRUMENT RUN? Assert before citing, on BOTH planes -------------------
echo "=== instrument liveness (⊘ an empty trace is evidence of NOTHING, not of no traffic) ==="
$G 'for f in /tmp/rmtrace.txt /tmp/uvm.txt; do
      if [ -s "$f" ]; then echo "OK   $f  $(wc -l < $f) lines"
      else echo "★ EMPTY OR ABSENT: $f — this plane was NOT observed in this run"; fi
    done
    echo "--- cup2 stderr (where an interposer reports its own failure) ---"
    cat /tmp/cup2.err 2>/dev/null | head -20'

echo "=== ★★★ THE UVM PLANE — rmStatus is the verdict; rc=0 only means DISPATCHED ==="
$G 'grep -a "rmStatus\|UVM ioctl" /tmp/uvm.txt 2>/dev/null \
      || echo "NO UVM LINE — the interposer saw no UVM ioctl at all"'

echo "=== ★★★ THE UVM PLANE, in full (struct bytes IN and OUT) ==="
$G 'cat /tmp/uvm.txt 2>&1'

echo "=== ★★★ THE RM PLANE — every call carrying a NON-ZERO status, in order ==="
$G 'grep -an "status=0x[0-9a-f]*[1-9a-f]" /tmp/rmtrace.txt 2>/dev/null | cut -c1-400 \
      || echo "NO NON-ZERO STATUS on the RM plane — every RM ioctl cuInit made returned NV_OK"'

echo "=== the RM plane: LAST 25 calls before cuInit gave up ==="
$G 'tail -25 /tmp/rmtrace.txt 2>/dev/null | cut -c1-400'

echo "=== RM plane line count + the MARK lines ==="
$G 'wc -l /tmp/rmtrace.txt 2>&1; grep -a "^MARK" /tmp/rmtrace.txt 2>/dev/null || echo "(no MARK lines)"'

echo "=== dmesg produced by cup2 ALONE (ring cleared immediately before it) ==="
$G 'sudo dmesg'

echo "=== ★★ is the CE wall ARMED or DORMANT? (one line decides it) ==="
$G 'sudo dmesg | grep -aiE "Initializing global CeUtils instance|Skipping global CeUtils creation" \
      || echo "NEITHER LINE PRESENT at this loglevel"'
