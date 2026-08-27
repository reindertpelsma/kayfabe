#!/bin/bash
# guest_cuinit_diff.sh — load 580 modules, run a CUDA cuInit probe under the
# host's shim3 (GR_GET_INFO / GPU_GET_INFO_V2 entry dumper) + shim (control
# sequence) so the guest dumps land in the SAME format as the host golden
# /tmp/dump3.log + /tmp/shim.log for a byte-diff.  Self-contained, no heredoc.
set +e
KO="$HOME/nvmods/nvidia.ko"
UVM="$HOME/nvmods/nvidia-uvm.ko"
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm 2>/dev/null; sudo rmmod nvidia 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo dmesg -C
sudo insmod "$KO" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$UVM" 2>&1 | tail -1
for n in "nvidia0 c 195 0" "nvidiactl c 195 255" "nvidia-uvm c 235 0" "nvidia-uvm-tools c 235 1"; do
  set -- $n; sudo mknod /dev/$1 $2 $3 $4 2>/dev/null
done
sudo chmod 666 /dev/nvidia* 2>/dev/null
echo "modules: $(lsmod | grep -cE '^nvidia')  uvm: $(lsmod | grep -c nvidia_uvm)"
sleep 2

cc -O2 -fPIC -shared -o /tmp/shim3.so /tmp/shim3.c -ldl 2>&1 | head -3
cc -O2 -fPIC -shared -o /tmp/shim.so  /tmp/shim.c  -ldl 2>&1 | head -3
nvcc -o /tmp/cup /tmp/cup.c -lcuda 2>&1 | head -5

echo "=== run #1: shim3 (GR/GPU info entry dump) ==="
sudo LD_PRELOAD=/tmp/shim3.so timeout 40 /tmp/cup; echo "exit=$?"
echo "--- guest /tmp/dump3.log ---"; cat /tmp/dump3.log 2>/dev/null

echo "=== run #2: shim (control sequence + status) ==="
sudo LD_PRELOAD=/tmp/shim.so timeout 40 /tmp/cup >/dev/null 2>&1; echo "exit=$?"
echo "--- control status lines (non-zero status only) ---"
grep -aE "st=0x[1-9a-f]" /tmp/shim.log 2>/dev/null | head -40
echo "--- total RM_CONTROL count ---"; grep -ac RM_CONTROL /tmp/shim.log 2>/dev/null

echo "=== dmesg NVRM tail ==="
sudo dmesg | grep -aiE "NVRM|fault|Xid|fail|timeout" | tail -10
