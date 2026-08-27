#!/bin/bash
# Runs INSIDE the guest: set up the open driver, build nvmemsnap.so + cup2, then run cup2 3x
# under the snapshot tool (crashes at 0xc7c0 each time, but the c7c0 'post' snapshot lands first).
set +e
gcc -shared -fPIC -O2 -o /tmp/nvmemsnap.so /tmp/mode2_diag/nvmemsnap.c -ldl -lpthread 2>&1 | head -3
sudo ln -sf libcuda.so.580.159.04 "/usr/local/nvidia-guest/lib/libcuda.so.1"; sudo rm -f "/lib/x86_64-linux-gnu/libcuda.so.1"; sudo ln -sf libcuda.so.580.159.04 "/lib/x86_64-linux-gnu/libcuda.so.1"
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia 2>/dev/null; sudo modprobe ecdh_generic ecc 2>/dev/null
for n in "nvidia0 c 195 0" "nvidiactl c 195 255" "nvidia-uvm c 235 0" "nvidia-uvm-tools c 235 1"; do sudo mknod /dev/${n% *} ${n#* } 2>/dev/null; done
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null
sudo sysctl -w kernel.yama.ptrace_scope=0 2>/dev/null
sudo insmod "/home/ubuntu/nvmods/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "/home/ubuntu/nvmods/nvidia-uvm.ko" 2>&1 | tail -1; sudo chmod 666 /dev/nvidia-uvm* 2>/dev/null
rm -f /tmp/cup2; nvcc -o /tmp/cup2 /tmp/cup2.c -lcuda -L"/lib/x86_64-linux-gnu" 2>&1 | head -2
i="${1:-1}"   # single run per boot (reboot between to avoid the post-crash GPU wedge)
LD_LIBRARY_PATH="/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu" \
  LD_PRELOAD=/tmp/nvmemsnap.so NVSNAP=/tmp/snap_guest_$i.txt NVSNAP_CLASS=0xc7c0 NVSNAP_FROM=124 NVSNAP_MAX=65536 \
  /tmp/cup2 >/tmp/cup2_$i.log 2>&1
echo "guest run $i: exit=$? crash=$(grep -c '^CRASH' /tmp/snap_guest_$i.txt 2>/dev/null) io_max=$(grep -oE '^SNAP io[0-9]+' /tmp/snap_guest_$i.txt 2>/dev/null | grep -oE '[0-9]+' | sort -n | tail -1) regs=$(grep -c '^REG' /tmp/snap_guest_$i.txt 2>/dev/null) last=$(tail -1 /tmp/cup2_$i.log)"
