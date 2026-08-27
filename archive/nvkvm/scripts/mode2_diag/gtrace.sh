#!/bin/bash
set +e
NVMODS=/home/ubuntu/nvmods
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo sysctl -w kernel.yama.ptrace_scope=0 >/dev/null 2>&1
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1
UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null; sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0 2>/dev/null; sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null
sudo ln -sf /usr/local/nvidia-guest/lib/libcuda.so.580.159.04 /lib/x86_64-linux-gnu/libcuda.so.1
gcc -shared -fPIC -O2 -o /tmp/nvioctl_trace.so /tmp/nvioctl_trace.c -ldl && echo "tracer built"
rm -f /tmp/cup2; nvcc -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1 | head -2
echo "=== guest trace run ==="
LD_LIBRARY_PATH="/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu" \
  NVALLOC=64 NVOUTER=64 NVTRACE=/tmp/guest_trace.txt LD_PRELOAD=/tmp/nvioctl_trace.so \
  timeout 60 /tmp/cup2 2>&1 | tail -4
echo "alloc lines: $(grep -c ALLOC /tmp/guest_trace.txt 2>/dev/null)"
