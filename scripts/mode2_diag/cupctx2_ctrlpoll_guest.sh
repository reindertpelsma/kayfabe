#!/usr/bin/env bash
# cupctx2_ctrlpoll_guest.sh — ON GUEST. cont.28: CTX2 cuCtxCreate spins in a userspace
# libcuda loop issuing NV_ESC_RM_CONTROL (0x2A). Catch it IN the ioctl and decode the
# NVOS54_PARAMETERS (arg = rdx): hClient@0, hObject@4, cmd@8, flags@12, params@16,
# paramsSize@24, status@32. Sample several times — if 'cmd' is constant it's the poll
# libcuda waits on (the #12 fix target: make that control return the awaited value).
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

gcc -O0 -g -o /tmp/cupctx2_min /tmp/cupctx2_min.c -lcuda 2>&1 | tail -2
LD_LIBRARY_PATH=$GUESTLIB stdbuf -oL -eL /tmp/cupctx2_min >/tmp/cupctx2_min.out 2>&1 &
PID=$!
echo "  pid=$PID; waiting for CTX2 spin..."
for i in $(seq 1 40); do grep -q '\[CTX2\] cuCtxCreate' /tmp/cupctx2_min.out 2>/dev/null && break; sleep 2; done
sleep 6
echo "=== catch ioctl, decode NVOS54 (arg=rdx): hClient hObject cmd flags paramsPtr paramsSz status ==="
GC=/tmp/gdbcmds; : > "$GC"
echo 'set pagination off' >> "$GC"
echo 'catch syscall ioctl' >> "$GC"
for n in $(seq 1 24); do
  echo 'continue' >> "$GC"
  echo 'printf "--- ioctl fd=%d req=0x%x argp=%p\n", $rdi, $rsi, $rdx' >> "$GC"
  echo 'x/9wx $rdx' >> "$GC"
done
echo 'detach' >> "$GC"
echo 'quit' >> "$GC"
sudo gdb -p "$PID" -batch -x "$GC" 2>&1 \
  | grep -vE 'Reading symbols|no debugging|warning:|No such file|Catchpoint|Using host|^\[' | head -120
echo "=== (NVOS54: word0=hClient word1=hObject word2=cmd word3=flags w4/5=paramsPtr w6=paramsSz w8=status) ==="
kill -9 "$PID" 2>/dev/null || true
echo "=== DONE_CTRLPOLL ==="
