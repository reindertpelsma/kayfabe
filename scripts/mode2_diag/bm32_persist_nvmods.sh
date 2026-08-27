#!/usr/bin/env bash
set -u
SSHO="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=8"
echo "=== A: copy ~/nvmods out of the running guest ==="
rm -rf /tmp/nvmods; mkdir -p /tmp/nvmods
scp -P 2223 $SSHO ubuntu@localhost:nvmods/nvidia.ko ubuntu@localhost:nvmods/nvidia-uvm.ko /tmp/nvmods/ 2>&1 | tail -1
ls -la /tmp/nvmods/
echo "=== B: stop guest, inject nvmods into BASE ==="
systemctl stop bm32guest 2>/dev/null; pkill -9 -f "[q]emu-system-x86_64"; sleep 3
NBD=/opt/qemu-nvkvm/bin/qemu-nbd
modprobe nbd max_part=16 2>/dev/null; $NBD --disconnect /dev/nbd0 >/dev/null 2>&1; sleep 1
$NBD --connect=/dev/nbd0 /opt/nvkvm-guest/ubuntu-24.04.qcow2; sleep 2
mkdir -p /mnt/ginj; mount /dev/nbd0p1 /mnt/ginj
mkdir -p /mnt/ginj/home/ubuntu/nvmods
cp /tmp/nvmods/*.ko /mnt/ginj/home/ubuntu/nvmods/
chown -R 1000:1000 /mnt/ginj/home/ubuntu/nvmods
echo "BASE nvmods now:"; ls -la /mnt/ginj/home/ubuntu/nvmods/
sync; umount /mnt/ginj; $NBD --disconnect /dev/nbd0
rm -f /opt/nvkvm-guest/mode2-overlay.qcow2
echo "=== C: fresh-boot guest (clean GSP) ==="
systemd-run --unit=bm32guest --working-directory=/workspace/nvkvm \
  bash -c "MEM=4G SMP=2 NVKVM_M2CEFWD=1 exec bash scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1"
echo "PERSIST_REBOOT_DONE rc=$?"
