#!/bin/bash
# guest_cuinit_strace.sh — clean-boot cuInit under strace to see the mmap set on
# nvidia fds + the exact bail point + any stall.  No rmmod.
set +e
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo modprobe ecdh_generic ecc 2>/dev/null
lsmod | grep -q '^nvidia ' || sudo insmod "$HOME/nvmods/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
lsmod | grep -q '^nvidia_uvm' || sudo insmod "$HOME/nvmods/nvidia-uvm.ko" 2>&1 | tail -1
for n in "nvidia0 c 195 0" "nvidiactl c 195 255" "nvidia-uvm c 235 0" "nvidia-uvm-tools c 235 1"; do
  set -- $n; sudo mknod /dev/$1 $2 $3 $4 2>/dev/null; done
sudo chmod 666 /dev/nvidia* 2>/dev/null
nvcc -o /tmp/cup /tmp/cup.c -lcuda 2>&1 | head -3
echo "=== strace cuInit (mmap/ioctl/openat, timestamped) ==="
sudo timeout 60 strace -f -tt -T -e trace=openat,ioctl,mmap,munmap,close /tmp/cup 2>/tmp/cup.strace
echo "exit=$?"
echo "--- all mmap on nvidia fds ---"
grep -nE "mmap\(" /tmp/cup.strace | grep -vE "/lib|\.so|MAP_ANON|/usr|/dev/zero" | tail -40
echo "--- the bail tail (last 30 syscalls) ---"
tail -30 /tmp/cup.strace
echo "--- biggest inter-syscall gaps (stall hunt) ---"
awk -F'[ :]' '{split($0,t,/[:.]/); ts=t[1]*3600+t[2]*60+t[3]+("0."t[4]); if(prev){d=ts-prev; if(d>0.3) print d"s before: "$0} prev=ts}' /tmp/cup.strace 2>/dev/null | tail -15
