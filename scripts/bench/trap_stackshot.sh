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
echo "STACKSHOT start $(date -Is) n=$N gap=${GAP}s -> $OUT"
for i in $(seq 1 "$N"); do
  pid=$(pgrep -x qemu-system-x86 | head -1)
  [ -z "$pid" ] && { sleep "$GAP"; continue; }
  # ⊘ `-batch` stops the process, dumps, and detaches. The stop is what the owner's SIGSTOP
  # does, with the backtrace we actually want on top of it.
  timeout 25 gdb -p "$pid" -batch \
      -ex "set pagination off" \
      -ex "thread apply all bt 18" 2>/dev/null \
    | awk -v n="$i" '/^Thread /{t=$0} {print "[" n "] " $0}' >> "$OUT"
  echo "---- sample $i $(date -Is) ----" >> "$OUT"
  sleep "$GAP"
done
echo "STACKSHOT done $(date -Is)"
# ★ The verdict: which frames appear in a thread that is inside our MMIO write path.
echo "=== frames seen inside kayfabe MMIO/trap paths ==="
grep -aoE "kayfabe[a-z_]*::[a-zA-Z_:]+|nvkvm_[a-z_]+" "$OUT" | sort | uniq -c | sort -rn | head -25
