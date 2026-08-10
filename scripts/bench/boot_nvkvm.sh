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

# ★★★ THE MODEL NAME THE GUEST WILL REPORT, read from the HOST rather than declared.
#
# `nvidia-smi --query-gpu=gpu_name` is the host driver's own answer to
# NV2080_CTRL_CMD_GPU_GET_NAME_STRING (0x20800110) -- the same control, surfaced by
# NVIDIA's own tool -- so this is the owner's READ-NATIVE ruling applied to a value that
# CANNOT be queried later: fn 65 (GET_GSP_STATIC_INFO) is the second RPC of the guest
# driver's whole life, before any RM object or isolate exists, so the name must be in hand
# before the device realizes.  A host holding a different card presents that card's name
# with no table to edit.
#
# ⊘ On a box with no nvidia-smi this stays EMPTY, which means ABSENT: the arrays stay zero
# and the guest reports `Name: ERR!`.  That is today's behaviour and it is a statement, not
# a placeholder -- an absent measurement must not decode as a value.
# ⊘ There is deliberately no short-name query: `gpuShortNameString` ("GA106-A") is answered
# by 0x20800111 and surfaced by nothing nvidia-smi prints, so it stays absent rather than
# being invented.  Set NVKVM_GPU_SHORT_NAME if you have measured one.
: "${NVKVM_GPU_NAME:=$(nvidia-smi --query-gpu=gpu_name --format=csv,noheader 2>/dev/null | head -1 || true)}"
: "${NVKVM_GPU_SHORT_NAME:=}"
DEV="nvkvm-gpu,bar1-size=268435456,bar2-size=33554432,id=kf0"
# QemuOpts splits a device line on commas, so a comma inside a value must be doubled.  A
# model name containing one would otherwise become two half-parsed properties and refuse
# realize with a message about neither.
[ -n "$NVKVM_GPU_NAME" ] && DEV="$DEV,gpu-name=${NVKVM_GPU_NAME//,/,,}"
[ -n "$NVKVM_GPU_SHORT_NAME" ] && DEV="$DEV,gpu-short-name=${NVKVM_GPU_SHORT_NAME//,/,,}"
[ -n "${NVKVM_DEV_EXTRA:-}" ] && DEV="$DEV,$NVKVM_DEV_EXTRA"
echo "boot_nvkvm: -device $DEV" >&2

exec "$Q" \
  -machine q35,accel=kvm -cpu host -smp 3 -m 2048 \
  -drive if=virtio,file=/workspace/bench/guest.qcow2,format=qcow2 \
  -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no \
  -device virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56 \
  `# NVKVM_DEV_EXTRA appends properties to the device line (e.g.
   # NVKVM_DEV_EXTRA=probe-arm-notifier=35 for a PROBE boot). It is an env var rather
   # than a positional arg because the device line is one argument and cannot be extended
   # from "$@"; the device's own end-of-run census reports the probe set it actually ran
   # with, so a boot cannot silently diverge from what this variable claims.` \
  -device "$DEV" \
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
