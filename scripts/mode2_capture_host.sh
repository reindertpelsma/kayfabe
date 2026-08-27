#!/usr/bin/env bash
# mode2_capture_host.sh — RUN ON THE HOST (vast.ai). DISRUPTIVE but self-restoring.
#
# Captures the REAL GA106 GSP control responses (device-info-table etc.) that
# Mode-2's fake GSP must replay. ROUTE_TO_PHYSICAL controls can't be called from
# userspace (NV_ERR_INSUFFICIENT_PERMISSIONS — see capture_devinfo.c), so we
# instrument the host's OPEN driver, load it, let the host's own RmInitAdapter
# issue the controls to the real GSP, and dump them via printk.
#
# SAFETY: always restores the DKMS driver on exit (trap). Host GPU must be idle
# (only nvidia-persistenced). Recoverable via `vastai reboot instance` if wedged.
#
# Prereq: instrument applied to /root/open-gpu-kernel-modules (see PATCH below),
# then `make modules` in that tree. This script does build + swap + capture +
# restore.
set -u
OGKM=/root/open-gpu-kernel-modules
LOG=/tmp/nvkvm_cap_dmesg.log

restore() {
    echo "==> RESTORE: reload DKMS driver"
    rmmod nvidia_uvm nvidia_drm nvidia_modeset nvidia 2>/dev/null
    modprobe nvidia 2>/dev/null
    modprobe nvidia_uvm 2>/dev/null
    systemctl start nvidia-persistenced 2>/dev/null
    nvidia-smi -L 2>&1 | head -2 || echo "WARN: nvidia-smi failed after restore — may need 'vastai reboot instance'"
}
trap restore EXIT

echo "==> build instrumented open driver"
( cd "$OGKM" && make -j"$(nproc)" modules >/tmp/ogkm_build.log 2>&1 ) || { echo BUILD-FAIL; tail -20 /tmp/ogkm_build.log; exit 1; }

echo "==> stop GPU users + unload DKMS modules"
systemctl stop nvidia-persistenced 2>/dev/null
sleep 1
rmmod nvidia_uvm nvidia_drm nvidia_modeset nvidia 2>/dev/null
lsmod | grep -q "^nvidia " && { echo "nvidia still loaded (busy); aborting"; exit 2; }

echo "==> insmod instrumented open nvidia.ko + deps"
insmod "$OGKM/kernel-open/nvidia.ko" NVreg_OpenRmEnableUnsupportedGpus=1 2>/tmp/insmod.err || { echo "insmod nvidia failed"; cat /tmp/insmod.err; exit 3; }
insmod "$OGKM/kernel-open/nvidia-modeset.ko" 2>/dev/null

echo "==> clear dmesg, trigger RmInitAdapter (nvidia-smi)"
dmesg -C
nvidia-smi -L 2>&1 | head -3
sleep 2

echo "==> capture NVKVM_CAP lines"
dmesg | grep -E "NVKVM_CAP" > "$LOG"
echo "  captured $(wc -l < $LOG) lines -> $LOG"
head -60 "$LOG"
echo "==> done (restore runs on exit)"
