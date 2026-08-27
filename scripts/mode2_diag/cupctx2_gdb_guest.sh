#!/usr/bin/env bash
# cupctx2_gdb_guest.sh — ON GUEST. cont.28: the CTX2 cuCtxCreate hang is a USERSPACE
# busy-spin in libcuda (main thread R/running, empty kernel stack). Attach gdb to the
# spinning process and dump the userspace backtrace + the instruction at RIP + a few
# repeated RIP samples, to pinpoint WHAT libcuda polls (the value/address it waits on).
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
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
sudo bash -c 'echo 0 > /proc/sys/kernel/yama/ptrace_scope' 2>/dev/null || true

if ! command -v gdb >/dev/null 2>&1; then
  echo "=== gdb absent; trying apt (may be offline) ==="
  sudo apt-get install -y gdb >/tmp/gdb_install.log 2>&1 || { echo "GDB INSTALL FAILED (offline?) — falling back to RIP sampling"; }
fi

gcc -O0 -g -o /tmp/cupctx2_min /tmp/cupctx2_min.c -lcuda 2>&1 | tail -3
echo "=== launch cupctx2_min bg ==="
LD_LIBRARY_PATH=$GUESTLIB stdbuf -oL -eL /tmp/cupctx2_min >/tmp/cupctx2_min.out 2>&1 &
PID=$!
echo "  pid=$PID; waiting for it to enter the CTX2 spin..."
# wait until [CTX2] appears (CTX1 done, CTX2 spinning)
for i in $(seq 1 40); do grep -q '\[CTX2\] cuCtxCreate' /tmp/cupctx2_min.out 2>/dev/null && break; sleep 2; done
sleep 6
echo "  out: $(tail -1 /tmp/cupctx2_min.out)"

if command -v gdb >/dev/null 2>&1; then
  echo "=== gdb backtrace of the spinning main thread (pid=$PID) ==="
  sudo gdb -p "$PID" -batch \
    -ex 'set pagination off' \
    -ex 'thread 1' \
    -ex 'bt' \
    -ex 'printf "MEMSET_DEST(rdi)=%p  rcx=%p\n", $rdi, $rcx' \
    -ex 'printf "DEST_MAPPING:\n"' -ex 'info proc mappings' \
    -ex 'detach' -ex 'quit' 2>&1 \
    | awk '/MEMSET_DEST/{d=strtonum($0 ~ /0x[0-9a-f]+/ ? "" : "")} {print}' \
    | grep -vE '^\[|Reading symbols|no debugging|^Download' > /tmp/gdb_out.txt
  # print backtrace + the memset dest + only the maps lines mentioning nvidia/devices or the dest range
  sed -n '1,/Mapped address spaces/p' /tmp/gdb_out.txt
  DEST=$(grep -oE 'MEMSET_DEST\(rdi\)=0x[0-9a-f]+' /tmp/gdb_out.txt | grep -oE '0x[0-9a-f]+')
  echo "  memset dest = $DEST"
  echo "  --- mappings backed by a device / nvidia (BAR/vidmem) ---"
  grep -iE 'nvidia|/dev/' /tmp/gdb_out.txt | head -20
  echo "  --- the mapping covering the memset dest (from /proc/$PID/maps) ---"
  sudo awk -v d=$(printf '%d' "$DEST" 2>/dev/null || echo 0) '
    { split($1,a,"-"); lo=strtonum("0x"a[1]); hi=strtonum("0x"a[2]);
      if (d>=lo && d<hi) print "    "$0 }' /proc/$PID/maps 2>/dev/null
else
  echo "=== no gdb: 8 RIP samples from /proc (best-effort) ==="
  for i in $(seq 1 8); do
    awk '{print "  rip-stat field30(kstkeip)="$30" state="$3}' /proc/$PID/stat 2>/dev/null
    sleep 1
  done
fi
echo "=== kill ==="; kill -9 "$PID" 2>/dev/null || true
echo "=== DONE_GDB ==="
