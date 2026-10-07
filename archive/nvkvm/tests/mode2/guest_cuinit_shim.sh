#!/bin/bash
# guest_cuinit_shim.sh — FRESH-boot, cup is FIRST nvidia0 client, run under
# shim.so so we capture the exact cuInit RM control/alloc sequence + statuses to
# diff against the host PHASE-1 golden.  NO rmmod, NO nvidia-smi.
set +e
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo modprobe ecdh_generic ecc 2>/dev/null
lsmod | grep -q '^nvidia ' || sudo insmod "$HOME/nvmods/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
lsmod | grep -q '^nvidia_uvm' || sudo insmod "$HOME/nvmods/nvidia-uvm.ko" 2>&1 | tail -1
for n in "nvidia0 c 195 0" "nvidiactl c 195 255" "nvidia-uvm c 235 0" "nvidia-uvm-tools c 235 1"; do
  set -- $n; sudo mknod /dev/$1 $2 $3 $4 2>/dev/null; done
sudo chmod 666 /dev/nvidia* 2>/dev/null
cc -O2 -fPIC -shared -o /tmp/shim.so /tmp/shim.c -ldl 2>/dev/null
nvcc -o /tmp/cup /tmp/cup.c -lcuda 2>&1 | head -3
echo "=== cuInit under shim (FIRST client) ==="
sudo LD_PRELOAD=/tmp/shim.so timeout 45 /tmp/cup; echo "exit=$?"
echo "=== full guest cuInit control/alloc sequence (/tmp/shim.log) ==="
cat /tmp/shim.log 2>/dev/null
