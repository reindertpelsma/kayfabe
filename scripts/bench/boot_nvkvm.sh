#!/usr/bin/env bash
# Boot the guest against the kayfabe QOM shim device on QEMU 10.2.4 + KVM.
#   usage: boot_nvkvm.sh <tag> [extra qemu args...]
# Runs QEMU in the FOREGROUND (invoke this script with run_in_background) so that
# error_report/info_report on stderr are captured -- `-daemonize` sends them to /dev/null.
# Guest net = tap (host 192.168.77.1, guest 192.168.77.2); the shim build has no slirp.
set -euo pipefail
TAG="${1:?usage: boot_nvkvm.sh <tag> [extra args]}"; shift || true
cd /workspace/bench
Q=${QEMU_BIN:-/workspace/bench/qemu-build/qemu-system-x86_64}
# ★★★ w827 — KF_DEVICE=kf3 boots THIS fat guest against the v3 device (`-device kf3-gpu`),
# owner 2026-09-25: *"run the CUDA ladder in the FAT guest … booted with the CURRENT kayfabe v3
# device"*. Same selection rules as the thin lane (`scripts/fastguest/run_fast_guest.sh`):
#   - the binary is the one built from THIS checkout's revision (`build_kf3.sh` installs one per
#     revision under <bench>/kf3-bins/<rev>), never the shared build dir's;
#   - guest RAM MUST be a shared memfd — the store/isolate plane adopts it, and an anonymous
#     MAP_PRIVATE block makes every store-backed path refuse (a harness fault that reads as ours);
#   - BAR1 defaults to 128 MiB (256 + headroom does not fit the host's 256 MiB aperture).
KF_DEVICE=${KF_DEVICE:-kf3}
if [ "$KF_DEVICE" = kf3 ]; then
    KF3_REPO="$(cd "$(dirname "$0")/../.." && pwd)"
    KF3_REV=$(git -C "$KF3_REPO" rev-parse --short=8 HEAD 2>/dev/null || echo unknown)
    [ -z "$(git -C "$KF3_REPO" status --porcelain --untracked-files=no 2>/dev/null)" ] || KF3_REV="$KF3_REV-dirty"
    Q=${QEMU_BIN:-/workspace/bench/kf3-bins/$KF3_REV/qemu-system-x86_64}
    [ -x "$Q" ] || { echo "★ boot_nvkvm: no kf3 binary for revision $KF3_REV at $Q — run scripts/bench/build_kf3.sh from this checkout" >&2; exit 2; }
    NVKVM_RAM_BACKEND=memfd
    KAYFABE_GUEST_BAR1_MB=${KAYFABE_GUEST_BAR1_MB:-128}
    export KAYFABE_GUEST_BAR1_MB
    DEVICE_ARG="kf3-gpu,fb-mb=${KF3_FB_MB:-8192},bar1-size=$(( KAYFABE_GUEST_BAR1_MB * 1024 * 1024 )),bar2-size=33554432,id=kf0${KF3_DEV_EXTRA:+,$KF3_DEV_EXTRA}"
    echo "== kf3 binary: $Q (rev $KF3_REV)  device: $DEVICE_ARG" >&2
elif [ "$KF_DEVICE" = nvkvm ]; then
    echo "★ boot_nvkvm: KF_DEVICE=nvkvm refused — the old nvkvm device (crates/kayfabe-qemu-raw) was archived at v3 — see archive/README.md; use KF_DEVICE=kf3 (the default)" >&2; exit 2
else
    echo "★ KF_DEVICE must be nvkvm or kf3, got [$KF_DEVICE]" >&2; exit 2
fi
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
# ★★ GUEST RAM IS A PARAMETER, and 2048 is not enough for every workload.
# Measured 2026-09-06 (w376): the LLM workload was SIGKILLed by the guest's OOM killer --
# `Killed`, no LLM_TOKENS line, so the harness correctly graded it (D) UNMEASURED rather than
# a compute failure. Qwen2-0.5B in fp16 plus torch's own RSS does not fit in 2 GiB.
# ⊘ It is NOT a bug in the emulator and must not be read as one: TORCH_CUDA_AVAILABLE was
#   True and TORCH_DEV_COUNT was 1 in the same run, so libcuda had already seen the device.
# ⚠ `-m` and the memfd backend's `size=` MUST match exactly, or QEMU refuses with
#   "Machine memory size does not match memory backend size" -- so they derive from one var.
NVKVM_RAM_MB=${NVKVM_RAM_MB:-2048}
RAMARGS=()
case "${NVKVM_RAM_BACKEND:-}" in
  memfd)
    # ⚠ `-m` is still required and must MATCH the backend size exactly, or QEMU refuses
    # with "Machine memory size does not match memory backend size".
    RAMARGS=(-object "memory-backend-memfd,id=ram0,size=${NVKVM_RAM_MB}M,share=on"
             -machine "q35,accel=kvm,memory-backend=ram0" -m "$NVKVM_RAM_MB")
    ;;
  ""|none) RAMARGS=(-machine "q35,accel=kvm" -m "$NVKVM_RAM_MB") ;;
  *) echo "★ NVKVM_RAM_BACKEND=${NVKVM_RAM_BACKEND} is not a backend I know" >&2; exit 2 ;;
esac

exec "$Q" \
  "${RAMARGS[@]}" -cpu host -smp "${KF_SMP:-3}" \
  -drive if=virtio,file=/workspace/bench/guest.qcow2,format=qcow2 \
  -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no \
  -device virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56 \
  `# NVKVM_DEV_EXTRA appends properties to the device line (e.g.
   # NVKVM_DEV_EXTRA=probe-arm-notifier=35 for a PROBE boot). It is an env var rather
   # than a positional arg because the device line is one argument and cannot be extended
   # from "$@"; the device's own end-of-run census reports the probe set it actually ran
   # with, so a boot cannot silently diverge from what this variable claims.` \
  `# w734 -- SS-w727 BAR1 KNOB, BOTH HALVES FROM ONE VARIABLE.
   # The chip row is patched by KAYFABE_GUEST_BAR1_MB (read in the archive chip_for), and
   # THIS property is what the hypervisor registers. nvkvm_apply_identity REFUSES at realize
   # if they differ (nvkvm.c:3552-3559) -- deliberately: a device that registers 128 MiB and
   # tells the guest 256 lets the guest map past the end of a region the hypervisor decodes,
   # with nothing logged on either side. So this derives from the SAME variable rather than
   # being a second literal an operator has to remember. Unset = 256 MiB, as shipped.
   #
   # !! NO BACKTICKS AND NO PARENTHESES IN THIS COMMENT. It is itself a BACKTICK command
   # substitution, so a backtick inside it CLOSES it and the rest of the prose becomes
   # commands -- measured w734: a comment written in this file style with backticks around
   # identifiers produced "REFUSES: command not found" and QEMU was handed the comment as an
   # argument. And bash -n PASSED it, which is the half worth remembering: the syntax gate
   # does not see inside a command substitution it can still parse.` \
  -device "$DEVICE_ARG" \
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
