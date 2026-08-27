#!/usr/bin/env bash
# cupctx2_sample_guest.sh — ON GUEST. Random-interrupt sample the spinning CTX2 thread
# ~14x: each attach catches it at a random point -> top frame + (if memset) whether the
# dest is a /dev/nvidia (BAR/vidmem MMIO) mapping. Tally tells us where the time goes:
# memset-over-BAR1 (perf wall) vs RM-ioctl latency vs a userspace retry loop.
set -u
NVMODS=/home/ubuntu/nvmods; GUESTLIB=/usr/local/nvidia-guest/lib
sudo systemctl isolate multi-user.target 2>/dev/null||true; sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null||true
sudo modprobe ecdh_generic ecc 2>/dev/null||true; sudo dmesg -C||true
if [ ! -f /lib/firmware/nvidia/580.159.04/gsp_ga10x.bin ]; then
  sudo mkdir -p /lib/firmware/nvidia/580.159.04 /mnt/nvfw
  sudo mount -t 9p -o trans=virtio,version=9p2000.L,ro nvfw /mnt/nvfw 2>/dev/null||true
  sudo cp -n /mnt/nvfw/*.bin /lib/firmware/nvidia/580.159.04/ 2>/dev/null||true
  sudo umount /mnt/nvfw 2>/dev/null||true; fi
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1|tail -1||true
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1|tail -1||true
UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null||true; sudo mknod /dev/nvidiactl c 195 255 2>/dev/null||true
if [ -n "$UVM_MAJ" ]; then sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
  sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0 2>/dev/null||true
  sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null||true; fi
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null||true
sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /lib/x86_64-linux-gnu/libcuda.so.1
sudo bash -c 'echo 0 > /proc/sys/kernel/yama/ptrace_scope' 2>/dev/null||true
gcc -O0 -g -o /tmp/cupctx2_min /tmp/cupctx2_min.c -lcuda 2>&1|tail -2
LD_LIBRARY_PATH=$GUESTLIB stdbuf -oL -eL /tmp/cupctx2_min >/tmp/cupctx2_min.out 2>&1 &
PID=$!; echo "  pid=$PID; waiting CTX2..."
for i in $(seq 1 40); do grep -q '\[CTX2\] cuCtxCreate' /tmp/cupctx2_min.out 2>/dev/null && break; sleep 2; done
sleep 5
echo "=== 14 random samples: top frame (+ syscall if in-kernel) ==="
for s in $(seq 1 14); do
  kill -0 $PID 2>/dev/null || { echo "  [exited]"; break; }
  SC=$(cat /proc/$PID/syscall 2>/dev/null | awk '{print $1}')
  TOP=$(sudo gdb -p $PID -batch -ex 'set pagination off' -ex 'frame 0' \
        -ex 'printf "TOP %s | rdi=%p\n", "x", $rdi' -ex 'bt 3' -ex detach -ex quit 2>/dev/null \
        | grep -E '^#0' | head -1)
  echo "  [$s] syscall=$SC  $TOP"
  sleep 1.5
done
echo "=== /proc/$PID/maps device (BAR/vidmem) regions ==="
sudo grep -E '/dev/nvidia' /proc/$PID/maps 2>/dev/null | head
kill -9 $PID 2>/dev/null||true; echo "=== DONE_SAMPLE ==="
