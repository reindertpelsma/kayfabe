#!/usr/bin/env bash
# rebuild_guest_mods.sh — runs ON THE GUEST. Rebuild the Mode-2 guest kernel
# modules (nvidia.ko + nvidia-uvm.ko) from the 9p `ogkm` source (already carries
# the load-bearing uvm_channel.c 0xFFF500 CE-sema backdoor; nv.c is stock — the
# DEBUG nv.c patches are superseded by QEMU-side M8.4/M14 fixes). Stage to
# /home/ubuntu/nvmods. Recovery after the overlay was lost. See MODE2_RECOVERY.md.
set -eu
KREL=$(uname -r)
SYSSRC=/lib/modules/$KREL/build
NVMODS=/home/ubuntu/nvmods
BUILD=/home/ubuntu/nvbuild

echo "=== mount 9p ogkm source ==="
sudo mkdir -p /mnt/ogkm
mountpoint -q /mnt/ogkm || sudo mount -t 9p -o trans=virtio,version=9p2000.L,ro ogkm /mnt/ogkm
ls /mnt/ogkm/Kbuild >/dev/null && echo "  ogkm mounted"

echo "=== copy source to local (9p can't host the build) ==="
rm -rf "$BUILD"; mkdir -p "$BUILD"
cp -a /mnt/ogkm/. "$BUILD"/
echo "  uvm patch present?"; grep -q "0xFFF500" "$BUILD/nvidia-uvm/uvm_channel.c" && echo "    YES (backdoor present)" || echo "    NO -- WARNING"

echo "=== build modules (this takes a few minutes) ==="
cd "$BUILD"
make -j"$(nproc)" modules SYSSRC="$SYSSRC" 2>&1 | tail -8

echo "=== stage to $NVMODS ==="
mkdir -p "$NVMODS"
cp -v "$BUILD/nvidia.ko" "$NVMODS/" 2>&1 | tail -1
cp -v "$BUILD/nvidia-uvm.ko" "$NVMODS/" 2>&1 | tail -1
echo "=== result ==="
ls -la "$NVMODS"/*.ko
modinfo "$NVMODS/nvidia.ko" 2>/dev/null | grep -E "^version|^vermagic" | head
echo "REBUILD_DONE"
