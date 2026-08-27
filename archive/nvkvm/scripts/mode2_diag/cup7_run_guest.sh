#!/usr/bin/env bash
# cup7_run_guest.sh — ON THE GUEST. Same module/firmware/node prep as cup4_run_guest.sh,
# but builds + runs cup7 (D1/D2 discriminator: host GR READ of a LARGE user buffer). PTX is
# JITed in-guest (libnvidia-ptxjitcompiler must match libcuda). Env: CUP7_MB (default 64).
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
TIMEOUT=${CUP7_TIMEOUT:-120}
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
sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /lib/x86_64-linux-gnu/libcuda.so.1
sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /usr/lib/x86_64-linux-gnu/libcuda.so 2>/dev/null; sudo ldconfig 2>/dev/null

echo "=== ptxjit availability (REQUIRED for cuModuleLoadData PTX) ==="
ls -l "$GUESTLIB"/libnvidia-ptxjitcompiler* /usr/lib/x86_64-linux-gnu/libnvidia-ptxjitcompiler* 2>/dev/null || echo "  (none in GUESTLIB or system)"

gcc -O0 -g -o /tmp/cup7 /tmp/cup7.c -lcuda 2>&1 | tail -3

echo "=== cup7 (foreground, timeout ${TIMEOUT}s, CUP7_MB=${CUP7_MB:-64}) ==="
LD_LIBRARY_PATH=$GUESTLIB CUP7_MB=${CUP7_MB:-64} timeout --signal=INT "$TIMEOUT" stdbuf -oL -eL /tmp/cup7
RC=$?
echo "=== cup7 exit rc=$RC (124=timeout/hang) ==="
