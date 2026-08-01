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

exec "$Q" \
  -machine q35,accel=kvm -cpu host -smp 3 -m 2048 \
  -drive if=virtio,file=/workspace/bench/guest.qcow2,format=qcow2 \
  -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no \
  -device virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56 \
  -device nvkvm-gpu,bar1-size=268435456,bar2-size=33554432,id=kf0 \
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
