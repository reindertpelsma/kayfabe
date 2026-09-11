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
for i in $(seq 1 "$N"); do
  pid=$(pgrep -x qemu-system-x86 | head -1)
  [ -z "$pid" ] && { sleep "$GAP"; continue; }
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
