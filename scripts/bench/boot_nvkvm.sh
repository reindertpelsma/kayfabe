#!/usr/bin/env bash
# Boot the guest against the kayfabe QOM shim device on QEMU 10.2.4 + KVM.
#   usage: boot_nvkvm.sh <tag> [extra qemu args...]
# Runs QEMU in the FOREGROUND (invoke this script with run_in_background) so that
# error_report/info_report on stderr are captured -- `-daemonize` sends them to /dev/null.
# Guest net = tap (host 192.168.77.1, guest 192.168.77.2); the shim build has no slirp.
set -euo pipefail
TAG="${1:?usage: boot_nvkvm.sh <tag> [extra args]}"; shift || true
cd /workspace/bench
Q=/workspace/bench/qemu-build/qemu-system-x86_64
LOG=/workspace/bench/run_${TAG}
rm -f "${LOG}_serial.log" "${LOG}_qemu.log" "${LOG}.mon"

# ★★★ NVKVM_RAM_BACKEND=memfd — the launch-time half of the guest-RAM crossing (#233).
# Guest RAM must be a SHARED, fd-backed block before any of it can be handed to an
# isolate; `-m 2048` alone gives an anonymous MAP_PRIVATE block that no other process can
# ever map. Default is EMPTY, so a boot that does not ask for it is byte-for-byte the
# boot every earlier capture took -- this must not silently become a property of the bench.
# ⊘ It is deliberately NOT `memory-backend-file`: a file backing puts guest RAM at a
# filesystem path, and the isolate boundary is supposed to be that the VMM hands DOWN a
# descriptor, not that guest RAM is openable by anything with the path.
RAMARGS=()
case "${NVKVM_RAM_BACKEND:-}" in
  memfd)
    # ⚠ `-m` is still required and must MATCH the backend size exactly, or QEMU refuses
    # with "Machine memory size does not match memory backend size".
    RAMARGS=(-object "memory-backend-memfd,id=ram0,size=2048M,share=on"
             -machine "q35,accel=kvm,memory-backend=ram0" -m 2048)
    ;;
  ""|none) RAMARGS=(-machine "q35,accel=kvm" -m 2048) ;;
  *) echo "★ NVKVM_RAM_BACKEND=${NVKVM_RAM_BACKEND} is not a backend I know" >&2; exit 2 ;;
esac

exec "$Q" \
  "${RAMARGS[@]}" -cpu host -smp 3 \
  -drive if=virtio,file=/workspace/bench/guest.qcow2,format=qcow2 \
  -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no \
  -device virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56 \
  `# NVKVM_DEV_EXTRA appends properties to the device line (e.g.
   # NVKVM_DEV_EXTRA=probe-arm-notifier=35 for a PROBE boot). It is an env var rather
   # than a positional arg because the device line is one argument and cannot be extended
   # from "$@"; the device's own end-of-run census reports the probe set it actually ran
   # with, so a boot cannot silently diverge from what this variable claims.` \
  -device nvkvm-gpu,bar1-size=268435456,bar2-size=33554432,id=kf0${NVKVM_DEV_EXTRA:+,$NVKVM_DEV_EXTRA} \
  -display none \
  `# ★★★ E2 — TIMESTAMP every error_report/info_report the device writes.
   # The device's per-doorbell line is the ATTRIBUTION instrument: a ring is only
   # attributable to a guest action if its arrival can be bracketed between two instants
   # recorded by somebody other than the device. Without this the qemu log's lines are
   # ordered and undated, and ordering alone cannot exclude "it happened during boot".` \
  -msg timestamp=on \
  -serial "file:${LOG}_serial.log" \
  -monitor "unix:${LOG}.mon,server,nowait" \
  "$@" \
  > "${LOG}_qemu.log" 2>&1
