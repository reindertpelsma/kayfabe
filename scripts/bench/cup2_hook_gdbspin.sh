#!/usr/bin/env bash
# ★★★★★ w269 POST_CAPTURE_HOOK — READ THE ADDRESS `cuCtxCreate` IS POLLING.
#
# ## Why this is not `cup2_hook_w232.sh` plus a `gdb` line
#
# `cup2_hook_w232.sh` runs `timeout 180 ./cup2` in the FOREGROUND of an `ssh`. Nothing can
# attach to it: by the time the command returns, the process it would have attached to is
# gone. ⇒ the run must be DETACHED and the probe must fire DURING it.
#
# ⚠ And the `timeout 180` is KEPT, deliberately: it is what produces the `CUP2_RC=124` that
# every rung since `w232` is compared against. A hook that killed `cup2` itself would report
# a number that means something different while looking identical — the exact shape
# `CLAUDE.md` records as `143` vs `124`.
#
# ## ⊘ `tests/mode2/gcup2_gdb.sh` (the C repo) does NOT answer this
#
# It is written for a **`SIGSEGV`** — `handle SIGSEGV stop nopass`, `run`, then
# `$_siginfo._sifields._sigfault.si_addr`. It never interrupts a HANG, and on a hang it prints
# nothing at all. Read before writing, per the brief; it is the wrong instrument as written.
#
# ## The rate limit is STRUCTURAL — the owner's warning, discharged
#
# No device-side read trap is enabled here or anywhere in this rung. The probe
# (`guest_spinprobe.c`) single-steps a FIXED budget, histograms RIP into a CAPPED number of
# buckets, prints how many it dropped, and takes exactly ONE register snapshot. It never
# writes to the target. See that file's header for the disassembly it decodes.
#
# ## Two samples, ~20 s apart, and that is a measurement not a retry
#
# The question "is the polled value MOVING?" cannot be answered by one sample. Two are taken;
# if the value is identical the wait is stuck on a word nothing writes, and if it advances the
# wait is progressing and the wall is a THRESHOLD, not a write.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/cup2_hook_gdbspin.sh scripts/bench/boot_capture.sh <tag>
#   ⚠ needs GQ_TIMEOUT >= 300.
set -uo pipefail
G="$(cd "$(dirname "$0")" && pwd)/gssh_nv"
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
CUP2_SRC=${KAYFABE_CUP2_SRC:-/workspace/bench/cup2.c}
PROBE_SRC="$SRC_DIR/guest_spinprobe.c"
BENCHDIR=${BENCH_DIR:-/workspace/bench}
TAG=${1:-w269}

die() { echo "★ w269 gdbspin hook FAILED: $*"; exit 2; }

# ⊘ WHICH COPY RAN. Two md5s, printed, so a run can never silently be the other copy.
echo "=== sources (md5 — a run cannot silently be a different copy) ==="
for f in "$CUP2_SRC" "$PROBE_SRC"; do
  [ -f "$f" ] || die "no such source: $f"
  printf '    %-56s %5s lines  md5 %s\n' "$f" "$(wc -l < "$f")" "$(md5sum < "$f" | cut -d' ' -f1)"
done

echo "=== push + build in the guest ==="
$G 'cat > /tmp/cup2.c'           < "$CUP2_SRC"  || die "could not push cup2.c"
$G 'cat > /tmp/guest_spinprobe.c' < "$PROBE_SRC" || die "could not push guest_spinprobe.c"
# ★★★ DELETE THE CLIENT FIRST: no build ⇒ no file ⇒ no run. `[ -x ]` alone cannot tell a
#     fresh binary from a stale one, and a stale client once exited 95 while looking healthy.
$G 'rm -f /tmp/cup2'
$G 'gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_CUP2_RC=$?'
$G 'gcc -O1 -o /tmp/guest_spinprobe /tmp/guest_spinprobe.c 2>&1; echo GCC_PROBE_RC=$?'
$G 'test -x /tmp/cup2' || die "cup2 did not build in the guest"
# ⊘ NOT a die: a probe that failed to build must not cost the boot its census. Its absence is
#   reported where its output would have been, and `CUP2_RC` is still produced.
$G 'test -x /tmp/guest_spinprobe' \
  || echo "★★★ THE PROBE DID NOT BUILD — every section below it will be EMPTY, and an empty section is evidence of NOTHING"

echo "=== is gdb available? (a cross-check, never the primary instrument) ==="
HAVE_GDB=$($G 'command -v gdb >/dev/null 2>&1 && echo yes || echo no' | tr -d "\r\n ")
echo "    gdb = $HAVE_GDB"

echo "=== launch cup2 DETACHED under its own 180 s timeout (rc stays comparable to w232..w268) ==="
$G 'cat > /tmp/run_cup2_detached.sh' <<'GUESTEOF'
#!/bin/sh
rm -f /tmp/cup2.out /tmp/cup2.rc
# ★ START marker + rc file: "exists but has no terminator" must be detectable at all.
echo "STARTED $(date -Is)" > /tmp/cup2.started
setsid sh -c 'cd /tmp && timeout 180 ./cup2 >/tmp/cup2.out 2>&1; echo $? >/tmp/cup2.rc' \
       </dev/null >/dev/null 2>&1 &
sleep 1
echo "LAUNCHED pid=$(pgrep -x cup2 | head -1)"
GUESTEOF
$G 'sh /tmp/run_cup2_detached.sh'

# ---- wait until cup2 is demonstrably INSIDE cuCtxCreate ---------------------------------
# ★ The gate is `totalMem=` in cup2's OWN output — the print immediately before `cuCtxCreate`.
#   ⊘ A fixed sleep would be a guess; this is the guest telling us it has arrived.
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
$G 'echo "--- cup2 output so far ---"; cat /tmp/cup2.out 2>/dev/null'

if [ "$ARRIVED" != yes ]; then
  echo "⊘ NOT SAMPLING: cup2 never printed \`totalMem=\`. Sampling now would measure a "
  echo "  different part of the program and read as an answer to this rung's question."
else
  sleep 10   # let the spin settle past cuCtxCreate's own setup work
  for PASS in 1 2; do
    echo "=== ★★★★★ SPIN PROBE, PASS $PASS of 2 ==="
    $G 'p=$(pgrep -x cup2 | head -1)
        if [ -z "$p" ]; then echo "⊘ cup2 is GONE — nothing to probe"; exit 0; fi
        echo "pid=$p state=$(ps -o stat= -p $p 2>/dev/null)"
        echo "--- per-thread ps (R = userspace spin; S/D = blocked in the kernel) ---"
        ps -L -o tid,stat,pcpu,wchan:22,comm -p $p 2>&1
        echo "--- /proc/<tid>/syscall (\"running\" = userspace, i.e. NOT a blocking syscall) ---"
        for t in /proc/$p/task/*; do echo "    ${t##*/}: $(sudo cat $t/syscall 2>&1)"; done
        if [ -x /tmp/guest_spinprobe ]; then
          sudo timeout 90 /tmp/guest_spinprobe "$p" 6000 2>&1
          echo "SPINPROBE_RC=$?"
        else
          echo "★ PROBE ABSENT — not run"
        fi'
    if [ "$PASS" = 1 ]; then
      echo "=== the mappings the polled address must be joined against ==="
      $G 'p=$(pgrep -x cup2 | head -1); [ -n "$p" ] && sudo cat /proc/$p/maps | grep -viE "^7f.*\.so|\[vvar|\[vdso" | head -40'
      if [ "$HAVE_GDB" = yes ]; then
        echo "=== gdb CROSS-CHECK (bounded; the C probe is the primary instrument) ==="
        $G 'p=$(pgrep -x cup2 | head -1)
            [ -z "$p" ] && { echo "⊘ cup2 gone"; exit 0; }
            sudo timeout 70 gdb -p "$p" -batch -nx \
              -ex "set pagination off" -ex "set confirm off" \
              -ex "info threads" \
              -ex "thread apply all bt 12" \
              -ex "info registers rip rsp rbx r12 r13 r15" \
              -ex "x/24i \$pc-32" \
              -ex "detach" 2>&1 | head -120
            echo "GDB_RC=$?"'
      else
        echo "=== ⊘ gdb NOT PRESENT in the guest — cross-check skipped, C probe stands alone ==="
      fi
      echo "=== sleeping 20 s before pass 2 (⊘ the question is whether the value MOVES) ==="
      sleep 20
    fi
  done
fi

# ---- let cup2 finish its own 180 s and report the SAME number every rung is compared on ----
echo "=== waiting for cup2's own timeout so CUP2_RC stays comparable to w232..w268 ==="
for i in $(seq 1 60); do
  $G 'test -f /tmp/cup2.rc' 2>/dev/null && break
  sleep 5
done
$G 'echo "--- cup2 FINAL output ---"; cat /tmp/cup2.out 2>/dev/null; wc -c < /tmp/cup2.out 2>/dev/null | sed "s/^/CUP2_OUT_BYTES=/"; echo "CUP2_RC=$(cat /tmp/cup2.rc 2>/dev/null || echo NO_RC_FILE)"'
$G 'sudo dmesg | tail -25'
echo "=== w269 gdbspin hook DONE ==="
