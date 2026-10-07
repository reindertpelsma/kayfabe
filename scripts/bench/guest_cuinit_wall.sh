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
#
# ## ⊘⊘ ★★★★★ THE PARAGRAPH ABOVE IS REFUTED BY THIS HOOK'S OWN OUTPUT (§16.78)
#
# `[measured 2026-08-10, boot `w214_9b65664_ctl`, `traces/guest_boots/run_w214_9b65664_ctl_probe.log`]`
# — the three `/proc/<pid>/stack` samples this hook took at its deadline were:
#
#     pid=1811 state=Rl
#     --- stack sample 1 ---            (empty)
#     --- stack sample 2 ---            [<0>] ktime_get_raw_ts64+0x41/0xd0
#     --- stack sample 3 ---            [<0>] __x64_sys_clock_gettime+0xb4/0x110
#
# ⇒ `state=R`, and a kernel stack that is nothing but `clock_gettime`. **The process is NOT
# spinning in `uvm_spin_loop` and is NOT blocked in the kernel at all — it is spinning in
# USERSPACE**, inside `libcuda`, and `/proc/<pid>/stack` is structurally incapable of saying
# where. The instrument ran, printed output, and that output is about a different plane from
# the one the wall is on. ⊘ Three consecutive boots (`w212`/`w213`/`w214`) recorded `state=Rl`
# beside an empty kernel stack and the wall was still described as a kernel spin.
#
# ⇒ `scripts/bench/guest_userstack.c` is added below and runs BESIDE the kernel sample (not
# instead of it — the kernel sample is what proves the userspace claim). It resolves `RIP`
# per thread through the target's own `/proc/<pid>/maps`.
set -uo pipefail
REPO=${KAYFABE_REPO:-/workspace/bench/kayfabe}
G="$REPO/scripts/bench/gssh_nv"
DEADLINE=${CUP2_DEADLINE:-150}
CUP2_SRC=${KAYFABE_CUP2_SRC:-/workspace/bench/cup2.c}
RM_SRC=$REPO/scripts/rpctrace/cuda_ioctl_trace.c
UVM_SRC=$REPO/scripts/bench/uvm_ioctl_trace.c
USTACK_SRC=$REPO/scripts/bench/guest_userstack.c

die() { echo "★ guest_cuinit_wall hook FAILED: $*"; exit 2; }
stamp() { # ⊘ WHICH COPY RAN — the trap that made §14.29's bisect unreachable for two rungs.
  [ -f "$1" ] || die "no such source: $1"
  printf '    %-58s %5s lines  md5 %s\n' "$1" "$(wc -l < "$1")" "$(md5sum < "$1" | cut -d' ' -f1)"
}

echo "=== sources that will run (md5, so a run cannot silently be the other copy) ==="
stamp "$CUP2_SRC"; stamp "$RM_SRC"; stamp "$UVM_SRC"; stamp "$USTACK_SRC"

echo "=== push + build inside the guest ==="
$G 'cat > /tmp/cup2.c'             < "$CUP2_SRC" || die "could not push cup2.c"
$G 'cat > /tmp/cuda_ioctl_trace.c' < "$RM_SRC"   || die "could not push the RM interposer"
$G 'cat > /tmp/uvm_ioctl_trace.c'  < "$UVM_SRC"  || die "could not push the UVM interposer"
$G 'cat > /tmp/guest_userstack.c'  < "$USTACK_SRC" || die "could not push the userspace sampler"
$G 'gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_CUP2_RC=$?'
$G 'gcc -O0 -fPIC -shared -o /tmp/cuda_ioctl_trace.so /tmp/cuda_ioctl_trace.c -ldl 2>&1; echo GCC_RM_RC=$?'
$G 'gcc -O1 -fPIC -shared -o /tmp/uvm_ioctl_trace.so  /tmp/uvm_ioctl_trace.c  -ldl 2>&1; echo GCC_UVM_RC=$?'
$G 'gcc -O1 -o /tmp/guest_userstack /tmp/guest_userstack.c 2>&1; echo GCC_USTACK_RC=$?'
$G 'test -x /tmp/cup2 && test -s /tmp/cuda_ioctl_trace.so && test -s /tmp/uvm_ioctl_trace.so' \
  || die "cup2 or one of the two interposers did not build"
# ⊘ NOT a `die`: the sampler is a NEW instrument on its own first run and a build failure in
# it must not cost the boot the census. Its absence is reported where it would have printed.
$G 'test -x /tmp/guest_userstack' || echo "★ the userspace sampler did not build — its section below will be EMPTY, and an empty section is evidence of NOTHING"

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
      echo "--- per-thread state (⊘ R = spinning in USERSPACE; D/S = blocked in the kernel) ---"
      ps -L -o tid,stat,pcpu,wchan:24,comm -p $p 2>&1
      echo "--- /proc/<tid>/syscall per thread (nr arg0..arg5 sp pc; \"running\" = userspace) ---"
      for t in /proc/$p/task/*; do echo "    ${t##*/}: $(sudo cat $t/syscall 2>&1)"; done
      for i in 1 2 3; do echo "--- kernel stack sample $i (⊘ EXPECTED TO BE EMPTY OR clock_gettime — see this hook'"'"'s header) ---"; sudo cat /proc/$p/stack 2>&1; sleep 1; done'

  # ★★★★★ §16.78 — THE MEASUREMENT THE KERNEL STACK CANNOT MAKE.
  echo "=== ★★★★★ USERSPACE STACK — where in libcuda is cuCtxCreate actually spinning? ==="
  $G 'p=$(pgrep -x cup2 | head -1)
      if [ -x /tmp/guest_userstack ] && [ -n "$p" ]; then
        sudo /tmp/guest_userstack "$p" 4 400 2>&1
      else
        echo "★ NOT RUN: sampler=$( [ -x /tmp/guest_userstack ] && echo present || echo ABSENT ) pid=${p:-NONE}"
      fi'
  # ★ The join key for every offset above. libcuda is stripped of local symbols, so this is
  # the exported surface only — enough to bracket an offset, never to name an internal.
  echo "=== libcuda identity + exported symbol table (the join for the offsets above) ==="
  $G 'l=$(ls -1 /usr/lib/x86_64-linux-gnu/libcuda.so.* 2>/dev/null | head -1)
      echo "libcuda=$l"; [ -n "$l" ] && { md5sum "$l"; ls -la "$l"; }'
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

# ★★★ `[measured 2026-08-09, THIS HOOK'S OWN FIRST RUN, boot `s27_c73d3ab_uvm`]` — THE
# ARTIFACTS DIED WITH THE GUEST. This hook printed EXCERPTS and left the two trace files in
# the guest's `/tmp`; `boot_capture.sh` phase 4 then powers the guest down, and an `scp` from
# the guest afterwards produced two ZERO-BYTE files. The middle ~50 lines of that boot's RM
# trace are unrecoverable. ⊘ Exactly the shape of every trap this repository already records
# — the operation succeeds either way, and only an explicit copy separates "observed" from
# "kept where anybody will find it later." A new instrument's first run is itself unverified.
# ⇒ The FULL traces go into the probe log (which phase 6 carries into the repository) AND to
# the bench directory, BEFORE anything powers anything off.
BENCHDIR=${BENCH_DIR:-/workspace/bench}
TAG=${1:-cuinitwall}
echo "=== ★ THE FULL RM PLANE — verbatim, because this file dies with the guest ==="
$G 'cat /tmp/rmtrace.txt 2>&1'
for f in rmtrace uvm; do
  $G "cat /tmp/$f.txt" > "$BENCHDIR/run_${TAG}_${f}.txt" 2>/dev/null
  if [ -s "$BENCHDIR/run_${TAG}_${f}.txt" ]; then
    echo "kept: $BENCHDIR/run_${TAG}_${f}.txt ($(wc -l < "$BENCHDIR/run_${TAG}_${f}.txt") lines)"
  else
    echo "★ FAILED TO KEEP $f — it survives ONLY as the probe-log excerpt above"
  fi
done

echo "=== dmesg produced by cup2 ALONE (ring cleared immediately before it) ==="
$G 'sudo dmesg'

echo "=== ★★ is the CE wall ARMED or DORMANT? (one line decides it) ==="
$G 'sudo dmesg | grep -aiE "Initializing global CeUtils instance|Skipping global CeUtils creation" \
      || echo "NEITHER LINE PRESENT at this loglevel"'
