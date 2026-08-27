#!/usr/bin/env bash
# cap1_coldboot_guest.sh — runs ON THE GUEST.  The workload of reference capture
# `cap1_coldboot_hermetic`: the hermetic cold GSP bring-up, nothing else.
#
#   insmod nvidia.ko -> fake-boot -> FWSEC/WPR2 -> LibOS -> msgq handshake ->
#   GSP_INIT_DONE -> `nvidia-smi -q` enumerates the emulated GA106 -> poweroff.
#
# No CUDA, no UVM, no host GPU (the capture runs m2fwd=off m2exec=off).  The
# `poweroff` is load-bearing: the trace file is only COMPLETE once QEMU exits
# and its exit notifier flushes the staging buffer.
#
# The first capture campaign drove this by hand; it is a script now so that two
# captures taken weeks apart are the same experiment.
set -u
NVMODS=/home/ubuntu/nvmods

sudo systemctl isolate multi-user.target 2>/dev/null || true
sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null || true
sudo modprobe ecdh_generic ecc 2>/dev/null || true
sudo dmesg -C || true

if [ ! -f /lib/firmware/nvidia/580.159.04/gsp_ga10x.bin ]; then
  sudo mkdir -p /lib/firmware/nvidia/580.159.04 /mnt/nvfw
  sudo mount -t 9p -o trans=virtio,version=9p2000.L,ro nvfw /mnt/nvfw 2>/dev/null || true
  sudo cp -n /mnt/nvfw/*.bin /lib/firmware/nvidia/580.159.04/ 2>/dev/null || true
  sudo umount /mnt/nvfw 2>/dev/null || true
fi

echo "=== insmod nvidia.ko ==="
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 \
     NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1 || true
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null || true
sudo mknod /dev/nvidiactl c 195 255 2>/dev/null || true
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null || true

# NB: no `| head`, ever.  A closed pipe SIGPIPEs nvidia-smi part way through its
# enumeration, which silently changes the RPC stream this capture exists to
# record — and `$?` after the pipeline is head's, so it looks like a clean run.
echo "=== nvidia-smi -q ==="
sudo nvidia-smi -q >/tmp/cap1_smi.txt 2>&1
echo "=== nvidia-smi -q rc=$? ($(wc -l </tmp/cap1_smi.txt) lines) ==="
head -12 /tmp/cap1_smi.txt
sudo dmesg | grep -aiE "nvrm|nvidia" | tail -20 || true

echo "=== poweroff (flushes the trace) ==="
sync
sudo poweroff
