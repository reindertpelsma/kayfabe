#!/usr/bin/env bash
# ★ Which PROCESSES hold the hypervisor's guest-RAM block? Asked of /proc, keyed on the
# INODE — the isolate is not a direct child of QEMU and does not carry a predictable
# `comm`, so parentage and name are both unusable as selectors.
set -uo pipefail
cd /workspace/bench/kayfabe
OUT=/workspace/bench/run_w224d_isolatefd.log
: > "$OUT"
exec >>"$OUT" 2>&1
echo "=== w224d [measured 2026-08-10, vh, shim rev 7b0694f] — WHICH PROCESSES HOLD GUEST RAM ==="
NVKVM_RAM_BACKEND=memfd KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd \
  ./scripts/bench/boot_nvkvm.sh w224d >/dev/null 2>&1 &
BOOT=$!
for i in $(seq 1 90); do ./scripts/bench/gssh_nv true >/dev/null 2>&1 && break; sleep 2; done
Q=$(pgrep -x qemu-system-x86 | head -1)
echo "qemu pid=$Q (guest answered ssh)"
INO=$(for f in /proc/$Q/fd/*; do case "$(readlink "$f" 2>/dev/null)" in
        /memfd:memory-backend-memfd*) stat -L -c %i "$f"; break;; esac; done)
echo "guest-RAM inode=$INO"
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
echo "--- holders, sampled 20x over ~40s while the driver runs ---"
for i in $(seq 1 20); do scan; sleep 2; done | sort -u
echo "--- for reference, the qemu-side descriptors ---"
for f in /proc/$Q/fd/*; do case "$(readlink "$f" 2>/dev/null)" in
  /memfd:memory-backend-memfd*) echo "  qemu fd=${f##*/} inode=$(stat -L -c %i "$f")";; esac; done
pkill -x qemu-system-x86
wait $BOOT 2>/dev/null
echo "=== done ==="
