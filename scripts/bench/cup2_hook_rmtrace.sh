#!/usr/bin/env bash
# POST_CAPTURE_HOOK: run `cup2` under the RM-ioctl and UVM-ioctl LD_PRELOAD interposers, so
# the refusal that ends `cuCtxCreate` is NAMED **AT THE BOUNDARY**, with an ORDER.
#
# ────────────────────────────────────────────────────────────────────────────────────────
# ★★★ WHY THIS HOOK EXISTS — the device's own census structurally CANNOT answer the question
#
# `s43`'s question was "what refuses `cuCtxCreate`?" and the device's end-of-run report was
# read for the answer. It cannot carry one, for three reasons that are properties of the data
# structures, not of the reader (all verified in source, 2026-08-10):
#
#   1. **The unserviced ledger is a SET.** `crates/kayfabe-device/src/unserviced.rs:206-283` —
#      `seen: Vec<UnservicedCommand>`, deduplicated by `contains`, element
#      `{ function, cmd }` with **no count field, no timestamp, no sequence number**. `s43`
#      logged **104 unserviced commands as 42 rows**; 62 events left no trace of themselves,
#      and nothing says whether a given id fired during `cuInit`, during `cuCtxCreate`, or in
#      teardown.
#   2. **The bridge-refusal census discards the class id.** `kayfabe-rmrpc/src/lib.rs:406-411`
#      builds `AllocClassNotPermitted { class, denial }` — the `hClass` IS captured in the
#      value — and `lib.rs:898-905` maps it to a `FaultTag` with `..`, dropping `class`.
#      `policy.rs:377` then counts by tag alone. ⇒ `NotOnAllowlist x10` is ten refusals of
#      unknown classes. ⊘ The previous rung read its empty greps for class ids as "the log
#      format is not what I assumed"; the truth is stronger — **the datum is not in the log.**
#   3. **The control census is keyed on `(cmd, result)` with a count** (`census.rs:249-268`)
#      — better, but still no order, and it only sees controls that were *answered*.
#
# ⇒ Every one of the three is an AGGREGATION that destroys the very thing the question needs:
# WHICH call, WITH WHICH ARGUMENT, AT WHICH POINT. That is `§16.51.3` one level out — there
# the reduction was a last-wins latch, here it is a set-union and a key projection.
#
# ────────────────────────────────────────────────────────────────────────────────────────
# ★★ AND THE INSTRUMENT WAS ALREADY BUILT (§16.40's class, for the third time)
#
# Nothing here is new code. `scripts/rpctrace/cuda_ioctl_trace.c` (666 lines) already decodes
# `NV_ESC_RM_CONTROL` / `NV_ESC_RM_ALLOC` / `NV_ESC_RM_FREE` with `hClient`, `hObject`, `cmd`,
# `hClass` and — the point — the `status` word the caller reads back, plus the params buffer
# before and after. `scripts/bench/uvm_ioctl_trace.c` covers the UVM plane, whose verdict
# lives in `params.rmStatus` and which `_IOC_TYPE=='F'` filters structurally miss.
#
# ⊘ Both were written to run on the HOST against real firmware. Pointing them at the GUEST,
# against our emulated GPU, is the new measurement — not new code.
#
# ────────────────────────────────────────────────────────────────────────────────────────
# ★ THE READOUT SURVIVES ITS OWN AGGREGATION — because there is none
#
# One `write(2)` per ioctl to an `O_APPEND` fd. No latch, no max, no per-key overwrite, no
# sampling, no dedup: the increment IS the line. The only bound is `NVTRACE_MAX`, which caps
# the **hex width of a payload inside a line** and can therefore never hide a record; it is
# set explicitly below rather than left to a default so the bound is on the record.
#
# ⚠ INJECTION MUST BE OFF. `cuda_ioctl_trace.c` doubles as a fault injector
# (`NVFAULT_CTRL` / `NVFAULT_ALLOC` / `NVFAULT_STATUS` / `NVSWEEP_GPUINFO`). A trace taken
# with any of them set is NOT a capture of what the port does. They are explicitly unset
# below, and the trace is asserted to contain no `INJECT` line.
set -uo pipefail
SELFDIR=$(cd "$(dirname "$0")" && pwd)
GSSH=${GSSH:-$SELFDIR/gssh_nv}
REPO=$(cd "$SELFDIR/../.." && pwd)
DEADLINE=${CUP2_DEADLINE:-180}
NVTRACE_MAX=${NVTRACE_MAX:-128}
CUP2_SRC=${CUP2_SRC:-/workspace/bench/cup2.c}

RM_SRC=$REPO/scripts/rpctrace/cuda_ioctl_trace.c
UVM_SRC=$REPO/scripts/bench/uvm_ioctl_trace.c

echo "=== rmtrace: WHAT ACTUALLY RAN (hashes, because two copies of a helper is a silent no-op) ==="
for f in "$RM_SRC" "$UVM_SRC" "$CUP2_SRC" "$0"; do
  printf '    %s  sha256=%s\n' "$f" "$(sha256sum "$f" 2>/dev/null | cut -c1-16)"
done
[ -r "$RM_SRC" ]  || { echo "★ MISSING $RM_SRC"; exit 2; }
[ -r "$UVM_SRC" ] || { echo "★ MISSING $UVM_SRC"; exit 2; }

echo "=== rmtrace: ship + build the two interposers and cup2 IN THE GUEST ==="
$GSSH 'mkdir -p /tmp/instr && rm -f /tmp/instr/* /tmp/nvtrace.txt /tmp/uvmtrace.txt'
$GSSH 'cat > /tmp/instr/cuda_ioctl_trace.c' < "$RM_SRC"
$GSSH 'cat > /tmp/instr/uvm_ioctl_trace.c'  < "$UVM_SRC"
$GSSH 'cat > /tmp/cup2.c'                   < "$CUP2_SRC"
$GSSH 'cd /tmp/instr &&
       gcc -shared -fPIC -O2 -o cuda_ioctl_trace.so cuda_ioctl_trace.c -ldl 2>&1; echo RM_SO_RC=$?
       gcc -shared -fPIC -O1 -o uvm_ioctl_trace.so  uvm_ioctl_trace.c  -ldl 2>&1; echo UVM_SO_RC=$?
       gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_RC=$?
       ls -l /tmp/instr/*.so /tmp/cup2 2>&1'

# ★ The deadline discipline is `cup2_hook_deadline.sh`'s, verbatim in intent: `cuInit` can
#   spin in `uvm_spin_loop` (process state R, takes no signal), so `timeout` would itself hang
#   waiting for a child that cannot die. Detach, poll, and give up on OUR clock.
echo "=== rmtrace: run cup2 under BOTH interposers (detached, deadline ${DEADLINE}s, host clock) ==="
$GSSH "rm -f /tmp/cup2.out /tmp/cup2.rc /tmp/cup2.pid
       env -u NVFAULT_CTRL -u NVFAULT_ALLOC -u NVFAULT_STATUS -u NVSWEEP_GPUINFO \
           setsid sh -c 'NVTRACE_OUT=/tmp/nvtrace.txt NVTRACE_MAX=${NVTRACE_MAX} \
                         UVMTRACE_OUT=/tmp/uvmtrace.txt \
                         LD_PRELOAD=/tmp/instr/cuda_ioctl_trace.so:/tmp/instr/uvm_ioctl_trace.so \
                         /tmp/cup2 >/tmp/cup2.out 2>&1; echo \$? >/tmp/cup2.rc' \
           </dev/null >/dev/null 2>&1 &
       sleep 1; pgrep -x cup2 > /tmp/cup2.pid 2>/dev/null; echo \"LAUNCHED pid=\$(cat /tmp/cup2.pid 2>/dev/null)\""

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
  $GSSH 'cat /tmp/cup2.out 2>/dev/null
         p=$(pgrep -x cup2 | head -1); echo "pid=$p state=$(ps -o stat= -p $p 2>/dev/null)"
         for i in 1 2 3; do echo "--- stack sample $i ---"; sudo cat /proc/$p/stack 2>&1; sleep 1; done'
fi

# ── phase V: VERIFY THE CAPTURE BEFORE ANY OF IT IS READ AS EVIDENCE ────────────────────
#
# ⊘ "A harness that writes an empty file and exits 0 is worse than none, because the file's
# existence reads as capture" (CLAUDE.md). `LD_PRELOAD` fails SILENTLY in at least four ways
# that all leave a file: a .so that did not build, a .so the loader rejected (it prints to
# the child's stderr, which is /tmp/cup2.out, not here), an `NVTRACE_OUT` the child could not
# open (it then falls back to **stderr** and the file stays empty), and a cup2 that never got
# far enough to issue an RM ioctl. So the assertion is on CONTENT, and specifically on a
# control we independently KNOW this boot issues.
echo "=== rmtrace: VERIFY (content, never existence) ==="
$GSSH 'for f in /tmp/nvtrace.txt /tmp/uvmtrace.txt; do
         printf "%s bytes=%s lines=%s\n" "$f" "$(stat -c%s $f 2>/dev/null || echo MISSING)" "$(wc -l < $f 2>/dev/null || echo 0)"
       done
       echo "INJECT_LINES=$(grep -c INJECT /tmp/nvtrace.txt 2>/dev/null || echo 0)"
       echo "PROMOTE_CTX_SEEN=$(grep -c "cmd=0x2080012b" /tmp/nvtrace.txt 2>/dev/null || echo 0)"'

# ── the readout, unreduced ──────────────────────────────────────────────────────────────
#
# ★ Every line below is a FILTER (all records having a property, in order), never a
#   REDUCTION (a summary that can hide a record). The census at the end is explicitly
#   labelled as the one reduction, and it is printed BESIDE the stream it summarises.
echo "=== rmtrace: ★★★ EVERY RM RECORD WITH A NON-ZERO status, IN ORDER (this is the answer) ==="
$GSSH 'grep -n "status=0x[0-9a-f]*" /tmp/nvtrace.txt 2>/dev/null | grep -v "status=0x00000000" | cut -c1-400'

# ⚠ The UVM interposer prints `rmStatus = 0x%08x` — **with spaces around the `=`** — while
#   the RM one prints `status=0x%08x`. A grep shaped like the RM row matches ZERO UVM rows and
#   the zero reads as "no UVM failure". That is the exact shape that cost this campaign two
#   rungs. Both patterns are written against the `fprintf` they must match
#   (`uvm_ioctl_trace.c:167`, `cuda_ioctl_trace.c:543`), not against each other.
echo "=== rmtrace: ★★★ EVERY UVM RECORD WITH A NON-ZERO rmStatus, IN ORDER ==="
$GSSH 'grep -n "rmStatus = 0x" /tmp/uvmtrace.txt 2>/dev/null | grep -v "rmStatus = 0x00000000" | cut -c1-400'
echo "--- and the UVM total, so an EMPTY list above is distinguishable from a MISSING one ---"
$GSSH 'echo "uvm rmStatus rows total: $(grep -c "rmStatus = 0x" /tmp/uvmtrace.txt 2>/dev/null || echo 0)"'

# ⊘ `s44` carried only the 80-record tail + the census into the repo; the full 249-record
#   stream stayed in the guest's /tmp and died with the poweroff. The stream is ~250 lines —
#   there was never a reason to cap it. Carry ALL of it, so the evidence in the tree is the
#   measurement and not a window onto it.
echo "=== rmtrace: ★ THE COMPLETE RM STREAM, every record, in order ==="
$GSSH 'cat -n /tmp/nvtrace.txt' | cut -c1-300

echo "=== rmtrace: the LAST 80 RM records (the tail is cuCtxCreate + teardown) ==="
$GSSH 'tail -80 /tmp/nvtrace.txt 2>/dev/null | cut -c1-300'

echo "=== rmtrace: the LAST 40 UVM records ==="
$GSSH 'tail -40 /tmp/uvmtrace.txt 2>/dev/null | cut -c1-300'

echo "=== rmtrace: census (⊘ A REDUCTION — printed beside the stream, never instead of it) ==="
# ⚠ Field positions, taken from the `snprintf`s themselves, NOT from eyeballing a line:
#   CTRL  $1 $2=cmd= $3=hClient $4=hObject $5=size $6=status   (cuda_ioctl_trace.c:542-546)
#   ALLOC $1 $2=hClass $3=hRoot $4=hParent $5=hObject $6=size $7=shape $8=status  (:615-621)
$GSSH 'echo "-- RM records by escape:"; awk "{print \$1}" /tmp/nvtrace.txt 2>/dev/null | sort | uniq -c | sort -rn | head -20
       echo "-- CTRL by (cmd,status), count desc:"; awk "\$1==\"CTRL\"{print \$2, \$6}" /tmp/nvtrace.txt 2>/dev/null | sort | uniq -c | sort -rn | head -60
       echo "-- ALLOC by (hClass,status), count desc:"; awk "\$1==\"ALLOC\"{print \$2, \$8}" /tmp/nvtrace.txt 2>/dev/null | sort | uniq -c | sort -rn | head -40'

echo "=== rmtrace: guest dmesg delta (full, not tail -40 — a cap is how a wall goes unseen) ==="
$GSSH 'sudo dmesg | sed -n "/cup2\|NVRM/p" | tail -120'
