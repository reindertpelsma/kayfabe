#!/bin/bash
# mode2_guest_test.sh — runs INSIDE the Mode-2 guest. Idempotent.
#  1. one-time: disable Mode-1 nvkvm autoload + blacklist conflicting modules
#  2. build the open nvidia.ko once (tmpfs) and stash on the persistent overlay
#  3. load crypto deps + nvidia.ko (fresh each boot)
#  4. trigger rm_init_adapter (opens /dev/nvidia0) and dump the boot dmesg
# The QEMU-side BAR0 trace is captured separately by the host via -D.
set -u
NVVER=580.159.04
NVMODS=/home/ubuntu/nvmods
OPENER=/tmp/nvopen

# 1. neutralize Mode-1 cruft (nvkvm_guest wedges /sys/module/nvidia)
if [ ! -f /etc/modprobe.d/zz-mode2.conf ]; then
    sudo sed -i '/nvkvm/d' /etc/modules-load.d/*.conf 2>/dev/null || true
    printf 'blacklist nvkvm_guest\nblacklist nvidia\nblacklist nvidiafb\n' \
        | sudo tee /etc/modprobe.d/zz-mode2.conf >/dev/null
fi

# 1b. stage the matching GSP firmware (driver loads nvidia/$NVVER/gsp_*.bin)
if [ ! -f "/lib/firmware/nvidia/$NVVER/gsp_ga10x.bin" ]; then
    sudo mkdir -p /mnt/nvfw "/lib/firmware/nvidia/$NVVER"
    mountpoint -q /mnt/nvfw || sudo mount -t 9p -o trans=virtio,version=9p2000.L,msize=1048576,ro nvfw /mnt/nvfw
    sudo cp /mnt/nvfw/gsp_*.bin "/lib/firmware/nvidia/$NVVER/" 2>/dev/null
    echo "staged firmware: $(ls /lib/firmware/nvidia/$NVVER/ 2>/dev/null)"
fi

# 2. build + stash nvidia.ko once (580 DKMS-tree: top Makefile, `make modules`)
if [ ! -f "$NVMODS/nvidia.ko" ]; then
    echo "=== building open nvidia.ko $NVVER (one-time) ==="
    sudo mkdir -p /mnt/ogkm /mnt/build
    mountpoint -q /mnt/ogkm  || sudo mount -t 9p -o trans=virtio,version=9p2000.L,msize=1048576,ro ogkm /mnt/ogkm
    mountpoint -q /mnt/build || sudo mount -t tmpfs -o size=4G tmpfs /mnt/build
    sudo cp -aL /mnt/ogkm/. /mnt/build/ 2>/dev/null
    ( cd /mnt/build && sudo make modules -j2 >/tmp/ogkm_build.log 2>&1 )
    KO=$(find /mnt/build -name nvidia.ko | head -1)
    mkdir -p "$NVMODS"
    if [ -n "$KO" ]; then cp "$KO" "$NVMODS/"; echo "stashed $(ls -la $NVMODS/nvidia.ko)";
    else echo "BUILD FAILED — tail:"; tail -20 /tmp/ogkm_build.log; exit 1; fi
fi

# 3. load deps + nvidia.ko (fresh; bypass blacklist with insmod-by-path)
sudo modprobe ecdh_generic ecc 2>/dev/null
if ! lsmod | grep -q '^nvidia '; then
    sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 2>&1 \
        && echo "insmod ok" || echo "insmod FAILED (see dmesg)"
fi

# device nodes
if [ ! -e /dev/nvidia0 ]; then
    sudo mknod /dev/nvidia0 c 195 0 2>/dev/null
    sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
    sudo chmod 666 /dev/nvidia0 /dev/nvidiactl 2>/dev/null
fi

# 4. trigger rm_init_adapter
gcc -o "$OPENER" /tmp/nvopen.c 2>/dev/null || { echo "gcc failed"; exit 1; }
sudo dmesg -C 2>/dev/null
echo "=== rm_init_adapter trigger (bounded 60s) ==="
sudo timeout 60 "$OPENER"
echo "opener_rc=$?"
echo "=== DMESG ==="
sudo dmesg | grep -iE "nvrm|nvidia|gsp|init|timeout|fail|fault|halt|booter|fwsec|wpr|falcon|riscv|gfw|msix" | head -70
