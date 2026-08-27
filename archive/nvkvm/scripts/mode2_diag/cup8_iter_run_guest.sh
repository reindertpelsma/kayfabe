#!/usr/bin/env bash
# cup8_iter_run_guest.sh — ON THE GUEST. Same module/firmware/node prep as
# cupctx2_run_guest.sh, but builds + runs cup8_iter (create->destroy->create,
# NO compute) to isolate whether the #12 2nd-context hang needs CTX1 compute.
# Env: ITERS (default 2), MIN_TIMEOUT (default 60).
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
TIMEOUT=${MIN_TIMEOUT:-60}
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

gcc -O0 -g -o /tmp/cup8_iter /tmp/cup8_iter.c -lcuda -lm 2>&1 | tail -3
echo "=== cup8_iter bg (ITERS=${ITERS:-5}) + progress/stack probe ==="
LD_LIBRARY_PATH=$GUESTLIB ITERS=${ITERS:-5} stdbuf -oL -eL /tmp/cup8_iter >/tmp/c8.out 2>&1 &
CP=$!
for s in $(seq 1 16); do sleep 8; kill -0 $CP 2>/dev/null || { echo "EXITED_at_$((s*8))s"; break; }; echo "[$((s*8))s] $(tail -1 /tmp/c8.out)"; done
echo "===== FULL OUTPUT ====="; cat /tmp/c8.out
if kill -0 $CP 2>/dev/null; then
  echo "===== HUNG ====="
  echo "State=$(awk '/^State/{print $2}' /proc/$CP/status) wchan=$(cat /proc/$CP/wchan 2>/dev/null)"
  echo "--- kernel stack ---"; sudo cat /proc/$CP/stack 2>/dev/null | head -6
  echo "--- last dmesg ---"; sudo dmesg 2>/dev/null | tail -5
  kill -INT $CP 2>/dev/null
fi
echo "=== DONE ==="
