#!/usr/bin/env bash
# POST_CAPTURE_HOOK: build and run cup2 INSIDE the guest, WITH A DEADLINE THE GUEST CANNOT
# DEFEAT, and sample the blocked stack if it does not finish.
#
# ⊘⊘ WHY THIS REPLACES `timeout 120 /tmp/cup2`. `[measured 2026-08-09, boot `fmb1` at
# `319d29a`]` `cuInit` stopped FAILING and started HANGING: UVM spins in `uvm_spin_loop`
# (a BUSY spin in the kernel, process state **R**, not D) waiting for a GPU tracking
# semaphore that never advances. `timeout` sends SIGTERM and then WAITS FOR THE CHILD —
# and the child can never take the signal, so `timeout` itself hangs. That boot ran 30+
# minutes, the harness never returned, QEMU had to be killed, and **the end-of-run census
# was never printed**: `run_fmb1_qemu.log` is 1745 bytes with no unserviced ledger at all.
#
# ⇒ Run it DETACHED, poll for its own result file, and give up on OUR clock. The spinning
# thread is left alive on purpose — it is one of three vcpus, the guest stays sshable, and
# QEMU still exits cleanly through the monitor so the census gets printed.
#
# ★ And when the deadline fires, the stack IS the measurement. For an execution-plane wall
# `/proc/<pid>/stack` names the waiting FUNCTION, which the unserviced ledger structurally
# cannot: the ledger only sees commands nobody claimed, and a hang is a command that WAS
# claimed and never completed.
set -uo pipefail
# ★★★ SELF-LOCATING, and this is not tidiness. `[measured 2026-08-10, boot `w226`]` these two
# paths were hardcoded to `/workspace/bench/kayfabe/...` and `/workspace/bench/cup2.c`. A rung
# run from a tree checked out at `/workspace/kayfabe_w226` — which is the *correct* thing to do
# when the campaign must not overwrite a tree another boot is using — got `HOOK_RC=127`, so
# `cuCtxCreate` never ran, so **2 doorbells arrived and `GrCompute=0`**. The observer started,
# declared nothing, and the boot was evidence of nothing. ⊘ A hook that silently cannot run is
# the same defect class `boot_capture.sh`'s own header is about, one level out.
#
# So: the sibling copy beside THIS file wins (it is the one that came with the tree that built
# the binary), and the box copy is the named fallback. `CUP2_SRC` overrides the workload.
SELFDIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
GSSH=${KAYFABE_GSSH:-$SELFDIR/gssh_nv}
[ -x "$GSSH" ] || GSSH=/workspace/bench/kayfabe/scripts/bench/gssh_nv
CUP2_SRC=${CUP2_SRC:-/workspace/bench/cup2.c}
if [ ! -x "$GSSH" ] || [ ! -f "$CUP2_SRC" ]; then
  echo "★ cup2_hook_deadline: CANNOT RUN — gssh=$GSSH (exec=$([ -x "$GSSH" ] && echo y || echo n))"
  echo "  cup2 source=$CUP2_SRC (exists=$([ -f "$CUP2_SRC" ] && echo y || echo n))."
  echo "  ⊘ Reporting this loudly rather than emitting 127 into a probe log: a boot whose hook"
  echo "  did not run measures the CONTROL PLANE ONLY and no completion claim may cite it."
  echo "CUP2_RC=HOOK_UNRUNNABLE"
  exit 127
fi
DEADLINE=${CUP2_DEADLINE:-150}

echo "=== cup2: locating a REAL cuda.h (3 of 5 hits in this guest are the PowerMac ADB"
echo "===       header and sort AHEAD of it — check CONTENT, never the path) ==="
$GSSH 'for h in $(find / -name cuda.h 2>/dev/null); do
         if grep -q CUDA_VERSION "$h"; then echo "REAL: $h"; fi
       done | head'
echo "=== cup2: build ==="
$GSSH 'cat > /tmp/cup2.c' < "$CUP2_SRC"
$GSSH 'gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_RC=$?'

echo "=== cup2: run (detached, deadline ${DEADLINE}s on the HOST's clock) ==="
$GSSH 'rm -f /tmp/cup2.out /tmp/cup2.rc /tmp/cup2.pid
       setsid sh -c "/tmp/cup2 >/tmp/cup2.out 2>&1; echo \$? >/tmp/cup2.rc" </dev/null >/dev/null 2>&1 &
       sleep 1; pgrep -x cup2 > /tmp/cup2.pid 2>/dev/null; echo "LAUNCHED pid=$(cat /tmp/cup2.pid 2>/dev/null)"'

done=0
for _ in $(seq 1 $((DEADLINE / 5))); do
  if $GSSH 'test -f /tmp/cup2.rc' 2>/dev/null; then done=1; break; fi
  sleep 5
done

if [ "$done" = 1 ]; then
  $GSSH 'cat /tmp/cup2.out; echo CUP2_RC=$(cat /tmp/cup2.rc)'
else
  echo "★★★ cup2 DID NOT RETURN within ${DEADLINE}s — this is a HANG, not a failure."
  echo "CUP2_RC=TIMEOUT"
  echo "=== cup2: partial stdout ==="
  $GSSH 'cat /tmp/cup2.out 2>/dev/null'
  echo "=== cup2: WHERE IT IS BLOCKED (five samples — the instrument for an execution-plane wall) ==="
  $GSSH 'p=$(pgrep -x cup2 | head -1); echo "pid=$p state=$(ps -o stat= -p $p 2>/dev/null)";
         for i in 1 2 3 4 5; do echo "--- sample $i ---"; sudo cat /proc/$p/stack 2>&1; sleep 1; done'
fi

echo "=== cup2: guest dmesg delta ==="
$GSSH 'sudo dmesg | tail -40'
