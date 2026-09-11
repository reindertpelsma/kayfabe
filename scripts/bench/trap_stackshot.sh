#!/usr/bin/env bash
# ★★★★★ **CATCH THE LONG TRAP IN THE ACT — owner, 2026-09-11:**
#   *"1 second is long enough to send SIGSTP to the kayfabe process under that trap, that
#    allows you to know exactly where its halted"*
#
# `[measured w448-w450]` `worst_trap=1 862 807us at=bar0+0x110c00` survived TWO wrong
# hypotheses of mine — "the drain holds the lock across all doorbells" (w448, no effect) and
# "one command's servicing is slow" (w450, `GSP-DRAIN` never printed, so no single command even
# reached 1 ms). ⊘ A trap that lasts 1.8 s does not need to be reasoned about. It needs to be
# looked at.
#
# ⚠ Sample, do not single-shot: the trap is ~1.8 s inside a boot of tens of seconds, so one
# well-timed look is luck. This takes N stacks and keeps the ones sitting in an MMIO write.
set -uo pipefail
N=${STACKSHOT_N:-60}
GAP=${STACKSHOT_GAP:-1}
OUT=${STACKSHOT_OUT:-/workspace/bench/stackshots.txt}
: > "$OUT"
# ⚠ `yama/ptrace_scope=1` lets a process attach only to its own DESCENDANTS, and the QEMU we
# want is started by a different session. Root bypasses it via CAP_SYS_PTRACE, but say so
# out loud rather than discovering it as an empty file.
if [ -r /proc/sys/kernel/yama/ptrace_scope ]; then
  echo "ptrace_scope=$(cat /proc/sys/kernel/yama/ptrace_scope) euid=$(id -u)"
  [ "$(id -u)" -ne 0 ] && echo "⚠ not root with yama on — attaches to non-descendants WILL fail"
fi
echo "STACKSHOT start $(date -Is) n=$N gap=${GAP}s -> $OUT"
# ⊘⊘ **WAIT for the process, do not race it.** `[measured w453]` the sampler reported
# `samples with a backtrace: 0` AND `samples refused: 0` — it never found a process at all,
# because it started sampling the instant the boot was launched and the boot spends its first
# minute on content checks before QEMU exists. Zero attaches and zero refusals is the
# signature of a sampler that ran at the wrong time, and it prints the same as "nothing was
# ever in that state".
WAIT=${STACKSHOT_WAIT:-300}
echo "STACKSHOT waiting up to ${WAIT}s for qemu-system-x86 to appear"
for _ in $(seq 1 "$WAIT"); do
  pgrep -x qemu-system-x86 >/dev/null && break
  sleep 1
done
if ! pgrep -x qemu-system-x86 >/dev/null; then
  echo "⊘⊘ STACKSHOT: qemu never appeared in ${WAIT}s — NOTHING was sampled. This is a"
  echo "   harness result, not a finding about the device."
  exit 3
fi
echo "STACKSHOT qemu is up; sampling"
for i in $(seq 1 "$N"); do
  pid=$(pgrep -x qemu-system-x86 | head -1)
  # ⊘ The process going away mid-run is the END of the window, not a sample to skip.
  [ -z "$pid" ] && { echo "[$i] qemu exited — sampling window over" >> "$OUT"; break; }
  # ⊘ `-batch` stops the process, dumps, and detaches. The stop is what the owner's SIGSTOP
  # does, with the backtrace we actually want on top of it.
  # ⊘⊘ **stderr is KEPT.** The first version had `2>/dev/null` here and produced a ZERO-BYTE
  # file across a whole boot — and a zero-byte file reads as *"nothing was in that state"*
  # when it actually meant *"gdb told us why and we threw it away"*. Third time this session a
  # redirect has eaten the evidence; the ledger's own rule is that an empty artefact is not
  # benign.
  timeout 25 gdb -p "$pid" -batch \
      -ex "set pagination off" \
      -ex "thread apply all bt 18" 2>&1 \
    | awk -v n="$i" '{print "[" n "] " $0}' >> "$OUT"
  echo "---- sample $i $(date -Is) ----" >> "$OUT"
  sleep "$GAP"
done
echo "STACKSHOT done $(date -Is)"
# ★ The verdict: which frames appear in a thread that is inside our MMIO write path.
# ⊘ Say how many samples actually ATTACHED before reporting what they saw: "no frames" from
# 110 successful attaches and "no frames" from 110 refusals are opposite findings.
echo "=== attach outcome ==="
echo "  samples with a backtrace: $(grep -ac '^\[[0-9]*\] #' "$OUT")"
echo "  samples refused:          $(grep -acE 'ptrace|Operation not permitted|No such process' "$OUT")"
echo "=== frames seen inside kayfabe MMIO/trap paths ==="
grep -aoE "kayfabe[a-z_]*::[a-zA-Z_:]+|nvkvm_[a-z_]+" "$OUT" | sort | uniq -c | sort -rn | head -25
