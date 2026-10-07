#!/bin/bash
set +e
gcc -O2 -o /tmp/nvtrace /tmp/mode2_diag/nvtrace.c 2>&1 | head -3
sudo ln -sf libcuda.so.580.159.04 "/usr/local/nvidia-guest/lib/libcuda.so.1"; sudo rm -f "/lib/x86_64-linux-gnu/libcuda.so.1"; sudo ln -sf libcuda.so.580.159.04 "/lib/x86_64-linux-gnu/libcuda.so.1"
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia 2>/dev/null; sudo modprobe ecdh_generic ecc 2>/dev/null
for n in "nvidia0 c 195 0" "nvidiactl c 195 255" "nvidia-uvm c 235 0" "nvidia-uvm-tools c 235 1"; do sudo mknod /dev/${n% *} ${n#* } 2>/dev/null; done
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null
sudo sysctl -w kernel.yama.ptrace_scope=0 2>/dev/null
sudo insmod "/home/ubuntu/nvmods/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "/home/ubuntu/nvmods/nvidia-uvm.ko" 2>&1 | tail -1; sudo chmod 666 /dev/nvidia-uvm* 2>/dev/null
rm -f /tmp/cup2; nvcc -o /tmp/cup2 /tmp/cup2.c -lcuda -L"/lib/x86_64-linux-gnu" 2>&1 | head -2
echo "=== cup2 under nvtrace (ptrace) ==="
LD_LIBRARY_PATH="/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu" /tmp/nvtrace -o /tmp/guest_nvtrace.txt -- /tmp/cup2 2>&1 | tail -6
echo "guest nvtrace: $(wc -l < /tmp/guest_nvtrace.txt) lines, ioctls=$(grep -c "^IOCTL" /tmp/guest_nvtrace.txt), uvm=$(grep -c nvidia-uvm /tmp/guest_nvtrace.txt)"
