#!/usr/bin/env bash
# mp14_run_guest.sh — ON THE GUEST.  Task #14: two concurrent CUDA applications.
#
# Same module/firmware/node prep as cup8_run_guest.sh, then launches N (default 2)
# cup8 instances SIMULTANEOUSLY against one fresh GSP.  Where cup8_concurrent_run_guest.sh
# only reports "A HUNG", this one answers the question that actually matters —
# *where* it stopped:
#
#   - the last CUDA API line each process printed (i.e. which call never returned),
#   - /proc/PID/syscall  -> the in-flight syscall + args (ioctl fd/cmd for an
#     un-returning RM ioctl; nothing if the process is spinning in userspace),
#   - /proc/PID/wchan + /proc/PID/stack -> kernel sleep site (empty for a busy poll),
#   - State: R = busy-poll in userspace/RM, D = stuck in an uninterruptible RM ioctl.
#     The distinction is load-bearing: the #14 signature is R (libcuda spins on
#     MC_SERVICE_INTERRUPTS), NOT a D-state wedge.
#   - the guest-visible fd behind the ioctl, so the hang can be attributed to
#     /dev/nvidiactl vs /dev/nvidia0 vs /dev/nvidia-uvm.
#
# Env: MP14_N (procs, default 2), CUP8_N (matrix dim, default 2048),
#      MP14_TIMEOUT (seconds, default 150).
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
NPROC=${MP14_N:-2}
TIMEOUT=${MP14_TIMEOUT:-150}

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
sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /usr/lib/x86_64-linux-gnu/libcuda.so 2>/dev/null
sudo ldconfig 2>/dev/null

gcc -O0 -g -o /tmp/cup8 /tmp/cup8.c -lcuda -lm 2>&1 | tail -2

echo "=== MP14: $NPROC concurrent cup8 (N=${CUP8_N:-2048}), one fresh GSP ==="
PIDS=""
for i in $(seq 1 "$NPROC"); do
    LD_LIBRARY_PATH=$GUESTLIB CUP8_N=${CUP8_N:-2048} stdbuf -oL -eL \
        /tmp/cup8 > "/tmp/mp14_p$i.out" 2>&1 &
    PIDS="$PIDS $!"
    echo "  launched p$i pid=$!"
done

# Poll for completion; record the exit rc of each as it lands.
declare -A RC
t=0
while [ "$t" -lt "$TIMEOUT" ]; do
    sleep 5; t=$((t + 5))
    done_n=0; i=0
    for p in $PIDS; do
        i=$((i + 1))
        if [ -n "${RC[$p]:-}" ]; then done_n=$((done_n + 1)); continue; fi
        if ! kill -0 "$p" 2>/dev/null; then
            wait "$p"; RC[$p]=$?
            echo "  [${t}s] p$i (pid $p) EXITED rc=${RC[$p]}"
            done_n=$((done_n + 1))
        fi
    done
    [ "$done_n" -eq "$NPROC" ] && break
done

echo ""
echo "=== PER-PROCESS STATE ==="
i=0
for p in $PIDS; do
    i=$((i + 1))
    echo "--- p$i (pid $p) ---"
    if [ -n "${RC[$p]:-}" ]; then
        echo "  rc=${RC[$p]}"
    else
        RC[$p]=124
        ST=$(awk '/^State/{print $2}' "/proc/$p/status" 2>/dev/null)
        echo "  rc=124 HUNG  State=$ST  (R = busy-poll, D = uninterruptible RM ioctl)"
        echo "  wchan   : $(cat "/proc/$p/wchan" 2>/dev/null)"
        # /proc/PID/syscall: "nr a0 a1 a2 a3 a4 a5 sp pc"; 16 = ioctl(fd, cmd, arg)
        SC=$(sudo cat "/proc/$p/syscall" 2>/dev/null)
        echo "  syscall : $SC"
        NR=$(echo "$SC" | awk '{print $1}')
        if [ "$NR" = "16" ]; then
            FD=$((  $(echo "$SC" | awk '{print $2}') ))
            echo "  ** in ioctl(fd=$FD, cmd=$(echo "$SC" | awk '{print $3}')) on \
$(readlink "/proc/$p/fd/$FD" 2>/dev/null)"
        elif [ "$NR" = "-1" ]; then
            echo "  ** NOT in a syscall — spinning in userspace (libcuda poll loop)"
        fi
        echo "  kstack  :"; sudo cat "/proc/$p/stack" 2>/dev/null | head -8 | sed 's/^/    /'
        echo "  nvidia fds:"; ls -l "/proc/$p/fd" 2>/dev/null | grep -c nvidia | sed 's/^/    /'
        kill -INT "$p" 2>/dev/null
    fi
    echo "  last API line: $(grep -E '^ok |^CUP8|CTX OK|MODULE OK|FUNC OK' "/tmp/mp14_p$i.out" 2>/dev/null | tail -1)"
    echo "  verdict      : $(grep -E 'VERDICT|RESULT' "/tmp/mp14_p$i.out" 2>/dev/null | tail -1)"
done

echo ""
echo "=== FULL OUTPUT PER PROCESS ==="
i=0
for p in $PIDS; do i=$((i + 1)); echo "----- p$i -----"; cat "/tmp/mp14_p$i.out"; done

echo ""
echo "=== guest dmesg tail (Xid / RM errors) ==="
sudo dmesg 2>/dev/null | grep -iE 'xid|nvrm|nvidia' | tail -10

PASS=0; FAIL=0
for p in $PIDS; do
    if [ "${RC[$p]}" = "0" ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); fi
done
echo ""
echo "=== MP14 VERDICT: pass=$PASS fail=$FAIL of $NPROC (fail=0 => #14 RESOLVED) ==="
