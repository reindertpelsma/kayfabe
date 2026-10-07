#!/usr/bin/env bash
# cupctx2_hangstack_guest.sh — ON GUEST. Find WHERE CTX2 cuCtxCreate actually hangs
# (cont.28: the cwfp(84) framing is suspect — it only fires at teardown). Boots the
# nvidia stack, runs cupctx2_min in the BACKGROUND, then every few seconds dumps the
# process's kernel stack (/proc/PID/stack), wchan, current syscall, and userspace
# instruction pointer — so we see exactly where it spins during CTX2 create.
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

gcc -O0 -g -o /tmp/cupctx2_min /tmp/cupctx2_min.c -lcuda 2>&1 | tail -3
echo "=== launch cupctx2_min in background ==="
LD_LIBRARY_PATH=$GUESTLIB stdbuf -oL -eL /tmp/cupctx2_min >/tmp/cupctx2_min.out 2>&1 &
PID=$!
echo "  pid=$PID"
SAMPLES=${SAMPLES:-14}
for s in $(seq 1 $SAMPLES); do
  sleep 8
  if ! kill -0 "$PID" 2>/dev/null; then echo "=== [t~$((s*8))s] process EXITED ==="; break; fi
  echo "===== [t~$((s*8))s] cupctx2_min state ====="
  echo "  --- /proc/$PID/status (State + last out line) ---"
  awk '/^State:/{print "  "$0}' /proc/$PID/status 2>/dev/null
  echo "  out: $(tail -1 /tmp/cupctx2_min.out 2>/dev/null)"
  echo "  --- wchan: $(cat /proc/$PID/wchan 2>/dev/null) ; syscall: $(cat /proc/$PID/syscall 2>/dev/null) ---"
  echo "  --- kernel stack (/proc/$PID/stack) ---"
  sudo cat /proc/$PID/stack 2>/dev/null | sed 's/^/    /' | head -25
  # also dump all threads' stacks (libcuda may spin on a helper thread)
  for t in /proc/$PID/task/*; do
    tid=$(basename "$t")
    [ "$tid" = "$PID" ] && continue
    st=$(awk '/^State:/{print $2}' "$t/status" 2>/dev/null)
    echo "  --- thread $tid State=$st wchan=$(cat $t/wchan 2>/dev/null) ---"
    sudo cat "$t/stack" 2>/dev/null | sed 's/^/      /' | head -12
  done
done
echo "=== kill cupctx2_min ==="
kill -9 "$PID" 2>/dev/null || true
echo "=== nvidia dmesg tail (any NVRM timeout/Xid clue) ==="
sudo dmesg | grep -iE "NVRM|Xid|timeout|timed out" | tail -15
echo "=== DONE_HANGSTACK ==="
