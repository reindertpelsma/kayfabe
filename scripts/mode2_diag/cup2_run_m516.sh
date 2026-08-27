#!/bin/bash
# cup2_run_m516.sh — guest-side: swap Mode-1 nvkvm_guest for the real open nvidia.ko
# against the emulated GA106, fix the DYNAMIC nvidia-uvm major (the hardcoded 235
# was the cuInit-999 trap), then run cup2 (cuInit -> cuCtxCreate) so the M5.16
# BAR1-WR probe in QEMU captures the compute-channel GP-entry write path.
set +e
NVMODS=/home/ubuntu/nvmods
[ -f "$NVMODS/nvidia.ko" ] || { echo "NO MODULES at $NVMODS"; exit 1; }

# 1. tear down the Mode-1 forwarder that squats /sys/module/nvidia
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null

# 2. load the real open driver against the emulated GPU
sudo sysctl -w kernel.yama.ptrace_scope=0 >/dev/null 2>&1
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 \
     NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1
echo "nvidia loaded: $(lsmod | grep -c '^nvidia ')  uvm: $(lsmod | grep -c '^nvidia_uvm')"

# 3. device nodes — nvidia frontend is static 195; nvidia-uvm is DYNAMIC (read it!)
UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
echo "nvidia-uvm dynamic major = ${UVM_MAJ:-MISSING}"
sudo mknod /dev/nvidia0       c 195 0   2>/dev/null
sudo mknod /dev/nvidiactl     c 195 255 2>/dev/null
if [ -n "$UVM_MAJ" ]; then
  sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
  sudo mknod /dev/nvidia-uvm       c "$UVM_MAJ" 0 2>/dev/null
  sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null
fi
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null

# 4. libcuda symlinks (guest-staged 580.159.04)
sudo ln -sf libcuda.so.580.159.04 /usr/local/nvidia-guest/lib/libcuda.so.1
sudo rm -f /lib/x86_64-linux-gnu/libcuda.so.1
sudo ln -sf /usr/local/nvidia-guest/lib/libcuda.so.580.159.04 /lib/x86_64-linux-gnu/libcuda.so.1

# 5. build + run cup2 (with nvtrace if present)
gcc -O2 -o /tmp/nvtrace /tmp/nvtrace.c 2>/dev/null
rm -f /tmp/cup2; nvcc -o /tmp/cup2 /tmp/cup2.c -lcuda -L/lib/x86_64-linux-gnu 2>&1 | head -3
echo "=== cup2 run ==="
if [ -x /tmp/nvtrace ]; then
  LD_LIBRARY_PATH="/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu" \
    timeout 40 /tmp/nvtrace -o /tmp/guest_nvtrace.txt -- /tmp/cup2 2>&1 | tail -12
  echo "guest ioctls=$(grep -c '^IOCTL' /tmp/guest_nvtrace.txt 2>/dev/null)"
else
  LD_LIBRARY_PATH="/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu" \
    timeout 40 /tmp/cup2 2>&1 | tail -12
fi
echo "=== guest dmesg tail (RmInitAdapter / WPR2) ==="
sudo dmesg | grep -iE "NVRM|nvidia|WPR2|RmInit|GSP" | tail -15
