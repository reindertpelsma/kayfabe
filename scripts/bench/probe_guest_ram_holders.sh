#!/usr/bin/env bash
# ★★★ WHICH PROCESSES HOLD THE HYPERVISOR'S GUEST-RAM BLOCK? — asked of `/proc`, keyed on
# the INODE, while a guest with the crossing armed is running.
#
#   usage: scripts/bench/probe_guest_ram_holders.sh [<tag>]     (default tag: gramhold)
#   writes: /workspace/bench/run_<tag>_isolatefd.log
#
# It exists because `guest_ram_crossing.md` §5's central claim — *the isolate really holds
# the descriptor* — cannot be read off any log the device writes. The device can only say
# what it granted; only `/proc` can say what the child has.
#
# ## ⚠ BOTH OBVIOUS SELECTORS ARE WRONG, and three attempts were spent on them
#
# `[measured 2026-08-10, vh, shim rev 7b0694f]`:
#
#   1. **`comm` is `memfd:kayfabe-i`, not `kayfabe-isolate`.** The isolate is `execveat`-ed
#      from a `memfd`, so the kernel derives `comm` from the descriptor's own name —
#      INCLUDING the `memfd:` prefix — and truncates it at 15 characters. So
#      `pgrep -x kayfabe-isolate` can NEVER match, exactly as `pgrep -x qemu-system-x86_64`
#      can never match. Second sighting of the same 15-character truncation in this project,
#      and the first one is already written down in `boot_capture.sh`.
#   2. **It is not a direct child of QEMU.** `ps -eo ppid` finds nothing under the QEMU pid;
#      the namespaced spawn reparents it.
#
# ⇒ The only selector that works is the INODE of the block itself, which is the same lesson
# the census makes structural: a name and a position are guesses, an identity is a fact.
#
# ⊘ It boots its OWN guest and powers it down afterwards, because GPU/bench runs are
# strictly serial and the sample has to be taken while the driver is live.
set -uo pipefail

REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:-gramhold}
OUT=$BENCH/run_${TAG}_isolatefd.log

cd "$REPO"
: > "$OUT"
exec >>"$OUT" 2>&1

echo "=== $TAG — WHICH PROCESSES HOLD GUEST RAM ==="
echo "=== source revision: $(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo UNKNOWN) ==="
echo "=== archive rev STAMPED IN THE BINARY: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null | grep -o 'kayfabe-rev:[0-9a-f]*' | sort -u | tr '\n' ' ')"

if pgrep -x qemu-system-x86 >/dev/null 2>&1; then
  echo "★ a QEMU is already running — GPU/bench runs are STRICTLY SERIAL. Refusing."
  exit 2
fi

NVKVM_RAM_BACKEND=memfd KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd \
  ./scripts/bench/boot_nvkvm.sh "$TAG" >/dev/null 2>&1 &
BOOT=$!
trap 'pkill -x qemu-system-x86 2>/dev/null' EXIT

for _ in $(seq 1 90); do ./scripts/bench/gssh_nv true >/dev/null 2>&1 && break; sleep 2; done
Q=$(pgrep -x qemu-system-x86 | head -1)
if [ -z "$Q" ]; then echo "★ QEMU is gone before the guest answered; see run_${TAG}_qemu.log"; exit 2; fi
echo "qemu pid=$Q (guest answered ssh)"

# The block, by the SAME properties `MemfdCensus` keys on — a `/memfd:` prefix of the whole
# readlink, and the hypervisor's backend-type name.
INO=$(for f in /proc/$Q/fd/*; do case "$(readlink "$f" 2>/dev/null)" in
        /memfd:memory-backend-memfd*) stat -L -c %i "$f"; break;; esac; done)
if [ -z "$INO" ]; then
  echo "★ no /memfd:memory-backend-memfd in the QEMU process — was NVKVM_RAM_BACKEND=memfd set?"
  exit 3
fi
echo "guest-RAM inode=$INO"

# ★ The isolate is materialized by guest work, not by realize, so the driver has to run.
./scripts/bench/gssh_nv 'sudo rmmod nvidia_drm nvidia_modeset nvidia_uvm nvidia 2>/dev/null; sudo modprobe nvidia; timeout 40 nvidia-smi >/dev/null 2>&1; echo LOADED' 2>&1 | tail -1

scan() {
  for d in /proc/[0-9]*; do
    p=${d#/proc/}
    for f in "$d"/fd/*; do
      [ -e "$f" ] || continue
      [ "$(stat -L -c %i "$f" 2>/dev/null)" = "$INO" ] || continue
      echo "  pid=$p comm=$(cat "$d/comm" 2>/dev/null) fd=${f##*/} exe=$(readlink "$d/exe" 2>/dev/null)"
    done
  done
}

# ⊘ SAMPLED, not sampled once: the isolate's lifetime is bounded by the guest's use of the
# GPU, and a single instant can miss it entirely — which is `a_correct_capture_can_answer_
# the_wrong_question` in its simplest form. `sort -u` folds the repeats.
echo "--- holders, sampled 20x over ~40s while the driver runs ---"
for _ in $(seq 1 20); do scan; sleep 2; done | sort -u
echo "--- for reference, every guest-RAM descriptor the hypervisor itself holds ---"
for f in /proc/$Q/fd/*; do case "$(readlink "$f" 2>/dev/null)" in
  /memfd:memory-backend-memfd*) echo "  qemu fd=${f##*/} inode=$(stat -L -c %i "$f")";; esac; done

pkill -x qemu-system-x86
wait $BOOT 2>/dev/null
echo "=== done ==="
