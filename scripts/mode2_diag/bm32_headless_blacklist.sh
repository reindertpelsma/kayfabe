#!/usr/bin/env bash
set -u
systemctl stop bm32guest bm32phase0 2>/dev/null; pkill -9 -f "[q]emu-system-x86_64"; sleep 3
NBD=/opt/qemu-nvkvm/bin/qemu-nbd
modprobe nbd max_part=16 2>/dev/null
$NBD --disconnect /dev/nbd0 >/dev/null 2>&1; sleep 1
$NBD --connect=/dev/nbd0 /opt/nvkvm-guest/ubuntu-24.04.qcow2; sleep 2
mkdir -p /mnt/ginj; mount /dev/nbd0p1 /mnt/ginj
# headless: default to multi-user (no gdm/graphical GPU auto-probe that crashes Mode-1 nvkvm_guest)
ln -sf /lib/systemd/system/multi-user.target /mnt/ginj/etc/systemd/system/default.target
echo "default.target -> $(readlink /mnt/ginj/etc/systemd/system/default.target)"
# also blacklist the Mode-1 module from auto-loading (Mode-2 uses stock nvidia; cup8 loads it)
echo "blacklist nvkvm_guest" > /mnt/ginj/etc/modprobe.d/nvkvm-mode2.conf
sync; umount /mnt/ginj; $NBD --disconnect /dev/nbd0
rm -f /opt/nvkvm-guest/mode2-overlay.qcow2
echo TARGET_FIXED
