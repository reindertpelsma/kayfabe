#!/bin/bash
# mode2_load_run_dbg.sh — runs INSIDE the Mode-2 guest (as root).
# Loads the DEBUG open driver (nvmods-dbg: GR-allocparams-null + CE-caps shims),
# creates device nodes, builds + runs cup2 (cuInit→cuCtxCreate→CE round-trip).
# cup2 is expected to BLOCK at cuCtxCreate (MC_SERVICE_INTERRUPTS) until the
# emulator posts the os-event interrupt; that's the wall we're diagnosing.
#
# libcuda: /usr/local/nvidia-guest/lib/libcuda.so.580.159.04 is the valid staged
# guest libcuda (the .so.1 symlink in that dir is broken → points at the
# allocator lib), so we LD_PRELOAD the real one explicitly.  [[guest_lib_version_staging]]
set -u
NVVER=580.159.04
DBG=${DBG:-/home/ubuntu/nvmods-dbg}
GUESTLIB=/usr/local/nvidia-guest/lib
LIBCUDA="$GUESTLIB/libcuda.so.$NVVER"

modprobe ecdh_generic ecc 2>/dev/null || true

# firmware (idempotent)
if [ ! -f "/lib/firmware/nvidia/$NVVER/gsp_ga10x.bin" ]; then
    mkdir -p /mnt/nvfw "/lib/firmware/nvidia/$NVVER"
    mountpoint -q /mnt/nvfw || mount -t 9p -o trans=virtio,version=9p2000.L,msize=1048576,ro nvfw /mnt/nvfw
    cp /mnt/nvfw/gsp_*.bin "/lib/firmware/nvidia/$NVVER/" 2>/dev/null || true
fi

# load debug nvidia.ko
if ! lsmod | grep -q '^nvidia '; then
    insmod "$DBG/nvidia.ko" NVreg_EnableGpuFirmware=1 2>&1 \
        && echo "insmod nvidia(dbg) ok" || { echo "insmod nvidia FAILED"; dmesg | tail -20; exit 1; }
fi

# frontend nodes
if [ ! -e /dev/nvidia0 ]; then
    mknod /dev/nvidia0 c 195 0; mknod /dev/nvidiactl c 195 255
    chmod 666 /dev/nvidia0 /dev/nvidiactl
fi

# uvm (dynamic major)
if ! lsmod | grep -q '^nvidia_uvm'; then
    insmod "$DBG/nvidia-uvm.ko" 2>&1 && echo "insmod uvm(dbg) ok" || echo "insmod uvm FAILED (continuing)"
fi
UVMMAJ=$(awk '/nvidia-uvm/{print $1}' /proc/devices | head -1)
if [ -n "$UVMMAJ" ] && [ ! -e /dev/nvidia-uvm ]; then
    mknod /dev/nvidia-uvm c "$UVMMAJ" 0; mknod /dev/nvidia-uvm-tools c "$UVMMAJ" 1
    chmod 666 /dev/nvidia-uvm /dev/nvidia-uvm-tools
fi

ls -la /dev/nvidia* 2>/dev/null

# build cup2 (header from toolkit, stub for link; real lib preloaded at run)
gcc -O0 -g -o /tmp/cup2 /tmp/cup2.c \
    -I/usr/include -L/usr/lib/x86_64-linux-gnu/stubs -lcuda 2>/tmp/cup2_build.err \
    || { echo "cup2 build failed:"; cat /tmp/cup2_build.err; exit 1; }
echo "cup2 built"

if [ "${NORUN:-0}" = "1" ]; then echo "NORUN=1: driver loaded + cup2 built, skipping run"; exit 0; fi

dmesg -C 2>/dev/null || true
echo "=== running cup2 (bounded 40s; expect block at cuCtxCreate) ==="
LD_PRELOAD="$LIBCUDA" LD_LIBRARY_PATH="$GUESTLIB" timeout 40 /tmp/cup2
echo "cup2_rc=$?"
echo "=== last dmesg ==="
dmesg | grep -iE "nvrm|nvidia|gsp|fault|timeout|fail" | tail -30
