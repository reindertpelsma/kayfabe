#!/usr/bin/env bash
# cup2_keystone_run.sh — runs ON THE GUEST. Fresh-boot module setup + build + run
# cup2 (cuInit -> cuCtxCreate). For the Layer-2 keystone we only need the CE
# scrubber / GR channel rings to fire so we can see (host side) whether the host
# GPU runs forwarded work or MMU-faults. cuCtxCreate is expected to hang at
# MC_SERVICE_INTERRUPTS (post-M8.4) — we cap it with timeout.
set -u
NVMODS=/home/ubuntu/nvmods
sudo systemctl isolate multi-user.target 2>/dev/null || true
sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null || true
sudo modprobe ecdh_generic ecc 2>/dev/null || true
sudo dmesg -C || true
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1 || true
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1 || true
UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null || true
sudo mknod /dev/nvidiactl c 195 255 2>/dev/null || true
if [ -n "$UVM_MAJ" ]; then
  sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
  sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0 2>/dev/null || true
  sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null || true
fi
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null || true
sudo ln -sf /usr/local/nvidia-guest/lib/libcuda.so.580.159.04 /lib/x86_64-linux-gnu/libcuda.so.1
gcc -O0 -g -o /tmp/cup2 /tmp/cup2.c -I/usr/include -L/usr/lib/x86_64-linux-gnu/stubs -lcuda 2>&1 | tail -3
echo "=== running cup2 (timeout 60) ==="
GUESTLIB=/usr/local/nvidia-guest/lib
LD_LIBRARY_PATH=$GUESTLIB timeout 60 stdbuf -oL -eL /tmp/cup2 2>&1 | tail -40
echo "=== guest dmesg (nvrm/nvidia) tail ==="
sudo dmesg 2>/dev/null | grep -iE 'nvrm|nvidia|xid' | tail -20
