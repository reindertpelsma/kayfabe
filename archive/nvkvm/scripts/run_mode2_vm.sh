#!/usr/bin/env bash
# run_mode2_vm.sh — boot a guest with the Mode-2 emulated NVIDIA GPU
# (nvkvm-gpu-emul) and the STOCK open NVIDIA driver, to capture the BAR0
# register-access trace (M0) and iterate the fake-the-boot state machine.
#
# Non-destructive: uses -snapshot so the base qcow2 is never modified (host
# disk is tight; we cannot afford a copy).  Driver build happens on a tmpfs in
# the guest and the open-driver source is shared read-only over 9p.
#
# Differences from the Mode-1 run_test_vm.sh:
#   - q35 machine (real PCIe, like a GeForce) instead of i440fx.
#   - NO virtio-nvgpu / nvkvm-gpu identity device — Mode-2 forwards nothing yet.
#   - Adds -device nvkvm-gpu-emul on the PCIe root complex.
#   - SSH on 2223 (so it can coexist with a Mode-1 VM on 2222).
#   - QEMU log (-D) captures the BAR0 trace from the device's qemu_log() calls.
#
# Env overrides: QEMU_BIN, MEM, SMP, EXTRA_QEMU_ARGS.
set -euo pipefail

QEMU="${QEMU_BIN:-/opt/qemu-nvkvm/bin/qemu-system-x86_64}"
BASE="/opt/nvkvm-guest/ubuntu-24.04.qcow2"
SEED="/opt/nvkvm-guest/seed.iso"
# Open driver source + matching GSP firmware.  Aligned to 580.159.04 = the host
# driver version (best for downstream Mode-1 forwarding) and the guest's staged
# userspace (NVML 580.159).  Both shared read-only over 9p.
OGKM="${OGKM:-/usr/src/nvidia-580.159.04}"   # open driver DKMS source 580.159.04
NVFW="${NVFW:-/usr/lib/firmware/nvidia/580.159.04}"  # gsp_ga10x.bin etc.
NVVER="${NVVER:-580.159.04}"
SSH_PORT="${SSH_PORT:-2223}"
QLOG="${QLOG:-/tmp/m0_qemu.log}"
SERIAL="${SERIAL:-/tmp/m0_serial.log}"
MEM="${MEM:-8G}"
SMP="${SMP:-4}"

# Persistent overlay backed by the pristine base.  Survives QEMU restarts (so
# the guest's blacklist tweaks + stashed nvidia.ko persist across Mode-2 device
# rebuilds), while the 29G base is never modified (read-only backing file).
# Set NVKVM_FRESH=1 to discard the overlay and start clean.
OVL="/opt/nvkvm-guest/mode2-overlay.qcow2"
[ -f "$BASE" ] || { echo "ERROR: $BASE missing"; exit 1; }
[ -f "$SEED" ] || { echo "ERROR: $SEED missing"; exit 1; }
if [ "${NVKVM_FRESH:-0}" = "1" ]; then rm -f "$OVL"; fi
if [ ! -f "$OVL" ]; then
    /opt/qemu-nvkvm/bin/qemu-img create -f qcow2 -F qcow2 -b "$BASE" "$OVL" >/dev/null
    echo "created overlay $OVL"
fi
IMG="$OVL"

rm -f "$QLOG" "$SERIAL"

# ── #90: §6 replay-trace provenance ──────────────────────────────────────
# The recorder writes this verbatim into the trace file header.  An oracle whose
# provenance is not in the artefact stops being an oracle the moment the bench
# dies, so this is composed HERE (where the versions actually are) rather than
# guessed by a reader later.  Only costs anything when NVKVM_M2REC is set.
if [ -n "${NVKVM_M2REC:-}" ]; then
    export NVKVM_M2REC_PROV="$(
        echo "captured: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "bench-host: $(hostname) $(uname -srm)"
        echo "host-driver: $(cat /proc/driver/nvidia/version 2>/dev/null | head -1)"
        echo "guest-driver-source: ${OGKM} (NVVER=${NVVER})"
        echo "guest-fw: ${NVFW}"
        echo "guest-kernel-pin: $(ls /opt/nvkvm-guest/drivers 2>/dev/null | head -3 | tr '\n' ' ')"
        echo "vbios: ${NVKVM_VBIOS:-/opt/nvkvm-guest/ga106_vbios.rom} md5=$(md5sum "${NVKVM_VBIOS:-/opt/nvkvm-guest/ga106_vbios.rom}" 2>/dev/null | cut -d' ' -f1)"
        # ★ A bench claim without a SOURCE REVISION is worthless: this bench
        # silently served a binary built from 862c7c2 for weeks.  The bench tree
        # is not a git checkout, so `git rev-parse` yields nothing there — fall
        # back to a `.srcrev` file written by whoever synced the tree, then to
        # $NVKVM_SRCREV, and say "UNKNOWN" out loud rather than emit a blank.
        _srcrev="$(cd "$(dirname "$0")/.." && git rev-parse HEAD 2>/dev/null)"
        [ -n "$_srcrev" ] || _srcrev="$(head -1 "$(dirname "$0")/../.srcrev" 2>/dev/null)"
        [ -n "$_srcrev" ] || _srcrev="${NVKVM_SRCREV:-UNKNOWN}"
        echo "emulator-src-commit: $_srcrev"
        echo "emulator-src-md5: $(md5sum "$(dirname "$0")/../src/qemu/nvkvm_gpu_emul.c" 2>/dev/null | cut -d" " -f1) nvkvm_gpu_emul.c"
        echo "recorder-src-md5: $(md5sum "$(dirname "$0")/../src/qemu/nvkvm_m2_rec.c" 2>/dev/null | cut -d" " -f1) nvkvm_m2_rec.c"
        echo "qemu: $("$QEMU" --version 2>/dev/null | head -1)"
        echo "nvidia-smi:"
        nvidia-smi --query-gpu=name,uuid,driver_version,vbios_version,memory.total,pci.bus_id \
                   --format=csv 2>/dev/null | sed 's/^/  /'
        echo "extra: ${NVKVM_M2REC_NOTE:-}"
    )"
fi

echo "Mode-2 VM:"
echo "  QEMU     : $QEMU"
echo "  Image    : $IMG  (snapshot — base preserved)"
echo "  SSH      : localhost:$SSH_PORT  (ubuntu)"
echo "  QEMU log : $QLOG   (BAR0 trace)"
echo "  Serial   : $SERIAL"
echo ""

# -d unimp,guest_errors enables the global logfile so the device's qemu_log()
# BAR0 trace lands in $QLOG.
exec "$QEMU" \
    `# M6.0 (item-4): back guest RAM with a SHARED memfd so the Mode-2 stub can mmap any` \
    `# guest GPA + OS_DESCRIPTOR it for host-GPU DMA into the guest's sysmem GR buffers.` \
    -machine q35,accel=kvm,memory-backend=pcram \
    -object memory-backend-memfd,id=pcram,size="$MEM",share=on \
    -cpu host \
    -m "$MEM" \
    -smp "$SMP" \
    \
    -drive if=none,id=hd0,file="$IMG",format=qcow2 \
    -device virtio-blk-pci,drive=hd0,addr=0x9 \
    -drive if=none,id=seed,file="$SEED",format=raw,readonly=on \
    -device virtio-blk-pci,drive=seed,addr=0xa \
    \
    -netdev user,id=net0,hostfwd=tcp::"$SSH_PORT"-:22 \
    -device virtio-net-pci,netdev=net0,addr=0x2 \
    \
    `# Mode-2 emulated NVIDIA GPU — put it directly on root slot 7 so the` \
    `# guest RM-generated gpuId encodes as 0x7, matching the forwarded host GPU.` \
    -device nvkvm-gpu-emul,addr=0x7,vbios="${NVKVM_VBIOS:-/opt/nvkvm-guest/ga106_vbios.rom}"${NVKVM_M2FWD:+,m2fwd=on}${NVKVM_M2FWD_OFF:+,m2fwd=off}${NVKVM_M2EXEC:+,m2exec=on}${NVKVM_M2EXEC_OFF:+,m2exec=off}${NVKVM_M2RING:+,m2ring=on}${NVKVM_M2HOSTSEM:+,m2hostsem=on}${NVKVM_M2CEFWD:+,m2cefwd=on}${NVKVM_M2CEXEC:+,m2cexec=on}${NVKVM_M2OPAQUE:+,m2opaque=on}${NVKVM_M2TRACE:+,m2trace=on}${NVKVM_M2ROMREGS:+,m2romregs=on}${NVKVM_M2REC:+,m2rec=on}${NVKVM_M2RECFILE:+,m2recfile=$NVKVM_M2RECFILE}${NVKVM_M2RECMASK:+,m2recmask=$NVKVM_M2RECMASK}${NVKVM_M2SEMVAL:+,m2semval=$NVKVM_M2SEMVAL}${NVKVM_M2SEMPAGE:+,m2sempage=$NVKVM_M2SEMPAGE} \
    \
    `# Open driver source + GSP firmware (RO) + repo, all over 9p.` \
    -virtfs local,path="$OGKM",mount_tag=ogkm,security_model=mapped,readonly=on \
    -virtfs local,path="$NVFW",mount_tag=nvfw,security_model=mapped,readonly=on \
    -virtfs local,path=/workspace/nvkvm,mount_tag=nvkvm_src,security_model=mapped \
    \
    -serial file:"$SERIAL" \
    -D "$QLOG" -d unimp,guest_errors \
    -display none \
    ${EXTRA_QEMU_ARGS:-}
