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
# CUP3 SHOULD FINISH IN UNDER A SECOND. A LONG BOUND WAITS ON THE BUG.
#
# Owner, 2026-09-13: "cup3 shouldn't take longer than a second, compared to bare metal, if it
# does kill it and strace up to that point to decode why. Its not worth waiting minutes every
# launch thats slow because of the bug itself rather than stopping in the bug."
#
# Correct, and the 300s default was costing ~6 minutes per iteration to learn one bit. The bound
# is now a FAST FAIL: when it blows we capture WHERE the process is, which is the thing that
# actually decodes the hang, and we get it ~50x sooner.
#
# The bound is deliberately generous against a bare-metal reference of milliseconds - it exists
# to bound the WAIT, not to judge the run. A pass well inside it is still a pass.
CUP3_TIMEOUT=${KAYFABE_CUP3_TIMEOUT:-10}

die() { echo "★ cup3 hook FAILED: $*"; exit 2; }

echo "=== source (md5 — a run cannot silently be a different copy) ==="
[ -f "$CUP3_SRC" ] || die "no such source: $CUP3_SRC"
printf '    %-56s %5s lines  md5 %s\n' "$CUP3_SRC" "$(wc -l < "$CUP3_SRC")" \
       "$(md5sum < "$CUP3_SRC" | cut -d' ' -f1)"
# ⊘⊘ **w305 DEFECT FIX — THIS PROVENANCE LINE WAS UNCONDITIONAL AND THEREFORE FALSE THE
#    MOMENT `KAYFABE_CUP3_SRC` POINTED ANYWHERE ELSE.** It printed *"byte-identical to the C
#    artifact's tests/mode2/cup3.c (md5 3c90b0f5…)"* directly underneath a source line
#    reporting a DIFFERENT md5 — measured on w305's `cup3d.c` run (`md5 67f93d72…`), where the
#    two lines contradicted each other three lines apart and the false one was the one that
#    read as a check. ⇒ Same class as the citation traps this tree keeps paying for: a
#    provenance claim that is TEMPLATE TEXT rather than a comparison asserts nothing, and it
#    is worse than silence because it looks verified.
#    ★ Now an actual COMPARISON, printed either way.
CUP3_VENDORED_MD5=3c90b0f5f9b7deedc9d9bea471ee551a
CUP3_THIS_MD5=$(md5sum < "$CUP3_SRC" | cut -d' ' -f1)
if [ "$CUP3_THIS_MD5" = "$CUP3_VENDORED_MD5" ]; then
  echo "    ★ byte-identical to the C artifact's tests/mode2/cup3.c at vendoring time"
  echo "      (md5 $CUP3_VENDORED_MD5) — so the two stacks run THE SAME program."
else
  echo "    ⊘ NOT the vendored cup3.c: this source is md5 $CUP3_THIS_MD5, the C artifact's"
  echo "      tests/mode2/cup3.c is md5 $CUP3_VENDORED_MD5. ⇒ this run is a VARIANT and the"
  echo "      two stacks are NOT running the same program. Read every comparison accordingly."
fi

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
rm -f /tmp/cup3.out /tmp/cup3.rc /tmp/cup3.ioctl
echo "STARTED \$(date -Is)" > /tmp/cup3.started
# ★★★★★ TRACED FROM THE START, not sampled.
#   [measured w695g] sampling raced its own budget: `strace -c` took 5s of a 10s timeout and
#   the follow-up ioctl trace attached exactly as `timeout` fired, capturing one SIGTERM and
#   nothing else. A sampler that needs three tools inside the subject lifetime does not fit.
#   Tracing from exec captures EVERY ioctl with no race, which is what identifies a repeat.
# ⊘ Falls back to an untraced run if strace is missing, so the rung still grades.
if command -v strace >/dev/null 2>&1; then
  setsid sh -c 'cd /tmp && timeout ${CUP3_TIMEOUT} strace -f -tt -e trace=ioctl -o /tmp/cup3.ioctl stdbuf -oL -eL ./cup3 >/tmp/cup3.out 2>&1; echo \$? >/tmp/cup3.rc' \\
       </dev/null >/dev/null 2>&1 &
else
  setsid sh -c 'cd /tmp && timeout ${CUP3_TIMEOUT} ./cup3 >/tmp/cup3.out 2>&1; echo \$? >/tmp/cup3.rc' \\
       </dev/null >/dev/null 2>&1 &
fi
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
# THE LABEL IS THE ELAPSED TIME BEFORE THIS POLL, NOT AFTER IT - and the off-by-one
# cost hours (w680).
#
# The `sleep 10` is at the END of the body, so poll i runs at roughly (i-1)*10 seconds, not
# i*10. Poll 1 fires ~1s after launch and used to print `[~10s]`; poll 2 fires ~12s in and
# printed `[~20s]`.
#
# An EMPTY tail prints nothing at all (the `-n "$NOW"` test). So a run whose only output is
# `[~20s] ok cuDeviceGet(&d,0)` means: "at poll 1 the file was still empty; by poll 2 it had
# reached cuDeviceGet". It attributes NO time to any individual call - and it was read as
# `cuDeviceGet took 20 seconds`, which sent an entire investigation after a number that does
# not exist. The real cost was `cuInit` (~6-8s) re-initialising an adapter that nvidia-smi
# had just closed.
#
# Two fixes, both about making the wrong reading unavailable:
#   - the label now says when the poll HAPPENED;
#   - an empty tail is PRINTED as `<no output yet>` rather than skipped, so "nothing has run"
#     and "something ran between two polls" stop looking identical.
# SAMPLE THE SPIN WHILE IT IS ALIVE, NOT AFTER IT IS DEAD (w690).
#
# The post-mortem block below fires when the bound blows - by which time `timeout` has already
# killed cup3, so it reported `CUP3_GONE` and captured nothing. `[measured w689a]` exactly that.
#
# A hang at 100% CPU is only diagnosable from INSIDE the hang: which syscall is repeating, and
# whether it is in a syscall at all. So take one short sample on the FIRST poll where cup3 is
# still running - ~1s in, while it is spinning - and print it whatever happens afterwards.
#
# The oracle's known signature for this wall is libcuda repeating
# NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS (0x20801702), an id hardware calls ZERO times in the
# whole program. A histogram dominated by ioctl, with that id in the argument, says libcuda is
# asking RM to service an interrupt tree that never reports progress.
SPIN_SAMPLED=0
for i in $(seq 1 "$LIMIT"); do
  AT=$(( (i - 1) * 10 ))
  if [ "$SPIN_SAMPLED" = "0" ]; then
    SPIN_SAMPLED=1
    $G 'P=$(pgrep -x cup3 | head -1)
        if [ -n "$P" ]; then
          echo "SPIN-SAMPLE pid=$P state=$(awk "{print \$3}" /proc/$P/stat 2>/dev/null) wchan=$(cat /proc/$P/wchan 2>/dev/null)"
          echo "SPIN-SAMPLE syscall=$(cut -c1-40 /proc/$P/syscall 2>/dev/null)"
          # ⊘⊘⊘ `head`, NOT `tail`. `strace -c` sorts by %time DESCENDING, so `tail` reads
          # the CHEAPEST calls — the bottom of the table. This exact pipe read as
          # "the process makes no interesting syscalls" for a whole session, which was a
          # fact about the sort order, not about the process.
          echo "      -- strace -c, TOP of a %time-DESCENDING table --"
          sudo timeout 5 strace -c -f -p $P 2>&1 | head -16
          # ★★★★★ **WHAT IT IS SPINNING ON.** `state=R` with ~100 syscalls says libcuda is
          # polling memory, and nothing above says WHICH memory or from which frame. A
          # userspace backtrace is the only instrument that names it.
          # ⊘ Attaching to the GUEST own own process, not to QEMU — the campaign rule that
          # `gdb` manufactures slow traps is about sampling the vCPU thread, and does not
          # apply here. The process is already stopped-and-resumed by the strace above.
          # ★★★★★ **WHICH ioctl.** `[measured w695f]` the summary above is 78% ioctl over 71
          # calls — so this is a REPEATED RM call, not a memory spin, and the repeated call
          # identity is the whole question. ⊘ The summary counts them and cannot name them.
          echo "      -- the ioctl stream itself (which call is being repeated) --"
          sudo timeout 5 strace -e trace=ioctl -p $P 2>&1 | head -14
          echo "      -- userspace backtrace (names the spin, or says why it could not) --"
          # ⊘ Fetch it ONCE if absent rather than spend a whole boot discovering the tool is
          # missing. The guest has network (provisioning apt-installs build-essential over the
          # same path), and cup3 is spinning meanwhile, so the wait costs nothing it was doing.
          if ! command -v gdb >/dev/null 2>&1; then
            echo "      (no gdb in the guest; fetching it once)"
            sudo DEBIAN_FRONTEND=noninteractive timeout 180 apt-get install -y -qq gdb >/dev/null 2>&1 \
              || echo "      apt-get gdb FAILED (no network, or no such package)"
          fi
          if command -v gdb >/dev/null 2>&1; then
            sudo timeout 20 gdb -p $P -batch -ex "thread apply all bt 12" 2>&1 \
              | grep -E "^.#|^Thread" | head -40
          elif command -v eu-stack >/dev/null 2>&1; then
            sudo timeout 20 eu-stack -p $P 2>&1 | head -40
          else
            echo "      NO BACKTRACE TOOL in the guest (no gdb, no eu-stack) - install one:"
            echo "      sudo apt-get install -y gdb   # then re-run this hook"
          fi
        else
          echo "SPIN-SAMPLE cup3 not running at the first poll (finished or never started)"
        fi' 2>&1 | sed 's/^/    /'
  fi
  if $G 'test -f /tmp/cup3.rc' 2>/dev/null; then echo "    terminator present at ~${AT}s"; break; fi
  NOW=$($G 'tail -1 /tmp/cup3.out 2>/dev/null' 2>/dev/null | tr -d '\r')
  if [ -z "$NOW" ]; then
    NOW="<no output yet>"
  fi
  if [ "$NOW" != "$LAST" ]; then
    # A poll boundary bounds the call only between THIS stamp and the previous one; it never
    # times a call. Say so in the line itself, because the line outlives the reader.
    echo "    [at ~${AT}s, i.e. since the previous poll] $NOW"
    LAST="$NOW"
  fi
  sleep 10
done

echo ""
echo "=== ★ cup3 FINAL OUTPUT, verbatim ==="
$G 'cat /tmp/cup3.out 2>/dev/null' | sed 's/^/    /'
$G 'wc -c < /tmp/cup3.out 2>/dev/null | sed "s/^/CUP3_OUT_BYTES=/"'

# ★★★★★ THE IOCTL STREAM, which is where cup3 actually is.
# [measured w695f] 88% of its time is ioctl over 71 calls. The LAST distinct calls before the
# timeout name what it was repeating; the histogram names how lopsided the repeat is.
echo "=== the last ioctls cup3 issued (it dies HERE) ==="
$G 'tail -14 /tmp/cup3.ioctl 2>/dev/null' | cut -c1-170 | sed 's/^/    /'
# ★★★★★ THE UVM CALL SEQUENCE, IN ORDER — the spec for porting this failure into rmladder.
# [measured w695i] cup3 dies inside UVM ioctl 0x21 = UVM_MAP_EXTERNAL_ALLOCATION. To reproduce
# that in the raw client (the owner directive: extend rmladder until IT fails the same way) the
# PRECEDING calls have to be right, and guessing them means a failure-to-reproduce proves
# nothing. ⊘ fd 9 is /dev/nvidia-uvm, whose ioctls are raw integers with no size encoding.
echo "=== the UVM call sequence in order (the port spec) ==="
$G 'grep -o "ioctl(9, _IOC(_IOC_NONE, 0, 0x[0-9a-f]*" /tmp/cup3.ioctl 2>/dev/null | sed "s/.*0x/0x/" | awk "!seen[\$0]++ || \$0 != prev {print} {prev=\$0}" | head -30 | tr "\n" " "' | sed 's/^/    /'
echo
echo "    (decode: 0x21=33 MAP_EXTERNAL_ALLOCATION, 0x25=37 REGISTER_GPU, 0x19=25 REGISTER_GPU_VASPACE,"
echo "             0x49=73 CREATE_EXTERNAL_RANGE, 0x17=23 CREATE_RANGE_GROUP, 0x46=70 PAGEABLE_MEM_ACCESS)"
echo "=== ioctl request histogram (the repeat stands out) ==="
$G 'grep -o "ioctl([0-9]*, [^,]*" /tmp/cup3.ioctl 2>/dev/null | sort | uniq -c | sort -rn | head -10' | sed 's/^/    /'
echo "    CUP3_IOCTL_LINES=$($G 'wc -l < /tmp/cup3.ioctl 2>/dev/null' 2>/dev/null | tr -d '\r')"

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
# WHEN IT DOES NOT FINISH, SAY WHERE IT IS - the hang's location is the finding.
#
# A timeout that reports only "it timed out" makes the next run mandatory. These three cheap
# reads usually make it unnecessary:
#   - /proc/<tid>/stack and /wchan: kernel-side, names the driver function it is inside;
#   - /proc/<tid>/syscall: whether it is in a syscall AT ALL - a 100% CPU userspace spin shows
#     `running`, which already rules out "blocked on us";
#   - a short `strace -c`: which ioctl it is repeating. `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`
#     (0x20801702) appearing here is the oracle's known signature for libcuda spinning on a
#     completion that never arrives - hardware calls that id ZERO times in the whole program.
if [ ! -f /tmp/cup3.rc ] || [ "${RV:-}" = "" ]; then
  echo ""
  echo "=== cup3 did not produce a value within ${CUP3_TIMEOUT}s - WHERE IS IT? ==="
  $G 'P=$(pgrep -x cup3 | head -1); if [ -n "$P" ]; then
        echo "CUP3_STILL_RUNNING pid=$P"
        echo "  state : $(awk "{print \$3}" /proc/$P/stat 2>/dev/null)"
        echo "  wchan : $(cat /proc/$P/wchan 2>/dev/null || echo unreadable)"
        echo "  syscall: $(cat /proc/$P/syscall 2>/dev/null | cut -c1-60 || echo unreadable)"
        echo "  utime/stime: $(awk "{print \$14, \$15}" /proc/$P/stat 2>/dev/null)"
        sudo timeout 6 strace -c -f -p $P 2>&1 | tail -14
      else
        echo "CUP3_GONE - it exited without writing a value"
      fi' 2>&1 | sed 's/^/    /'
fi
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
