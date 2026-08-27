#!/bin/bash
# Guest-side: build cup2 + ioctl_trace.so, run cup2 with the ioctl tracer preloaded,
# dump the tail of the trace (the RM_CONTROL/ALLOC stream right before the crash).
set +e
NVMODS=/home/ubuntu/nvmods
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null; sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
sudo mknod /dev/nvidia-uvm c 235 0 2>/dev/null; sudo mknod /dev/nvidia-uvm-tools c 235 1 2>/dev/null
sudo chmod 666 /dev/nvidia* 2>/dev/null
sudo dmesg -C
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1
sudo chmod 666 /dev/nvidia-uvm* 2>/dev/null
rm -f /tmp/cup2 /tmp/ioctl_trace.so /tmp/ioctl_trace.log
nvcc -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1 | head -3
gcc -shared -fPIC -o /tmp/ioctl_trace.so /tmp/ioctl_trace.c -ldl 2>&1 | head -3
echo "=== running cup2 with ioctl tracer ==="
sudo env NVKVM_TRACE=/tmp/ioctl_trace.log LD_PRELOAD=/tmp/ioctl_trace.so timeout 60 /tmp/cup2 2>&1 | tail -8
echo "=== FULL CTRL OUT stream (for host-vs-guest diff) ==="
grep -E "^CTRL=" /tmp/ioctl_trace.log 2>/dev/null
echo "=== trace control-cmd histogram ==="
grep -oE "CTRL  cmd=0x[0-9a-f]+" /tmp/ioctl_trace.log 2>/dev/null | sort | uniq -c | sort -rn | head -20
echo "=== dmesg ==="
sudo dmesg | grep -aiE "segfault|NVRM|Xid|fault" | tail -6
