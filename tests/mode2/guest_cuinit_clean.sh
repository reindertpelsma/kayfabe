#!/bin/bash
# guest_cuinit_clean.sh — CLEAN first-boot cuInit probe.  NEVER rmmod nvidia
# (rmmod+insmod triggers the WPR2 re-boot cascade).  Load nvidia.ko once if not
# present, add uvm, run cup under shim3.  Capture RmInitAdapter outcome + the
# post-enumeration stall behaviour.
set +e
KO="$HOME/nvmods/nvidia.ko"
UVM="$HOME/nvmods/nvidia-uvm.ko"
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo modprobe ecdh_generic ecc 2>/dev/null
if ! lsmod | grep -q '^nvidia '; then
  echo "loading nvidia.ko (fresh)"
  sudo insmod "$KO" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
else
  echo "nvidia.ko already loaded (NOT reloading)"
fi
lsmod | grep -q '^nvidia_uvm' || sudo insmod "$UVM" 2>&1 | tail -1
for n in "nvidia0 c 195 0" "nvidiactl c 195 255" "nvidia-uvm c 235 0" "nvidia-uvm-tools c 235 1"; do
  set -- $n; sudo mknod /dev/$1 $2 $3 $4 2>/dev/null
done
sudo chmod 666 /dev/nvidia* 2>/dev/null
echo "modules: nvidia=$(lsmod | grep -c '^nvidia ') uvm=$(lsmod | grep -c '^nvidia_uvm')"

cc -O2 -fPIC -shared -o /tmp/shim3.so /tmp/shim3.c -ldl 2>/dev/null
nvcc -o /tmp/cup /tmp/cup.c -lcuda 2>&1 | head -3

echo "=== nvidia-smi (proves device alive) ==="
sudo timeout 25 nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader 2>&1 | head -3

echo "=== run cup under shim3 (timeout 45, timed) ==="
sudo dmesg -C
/usr/bin/time -v sudo LD_PRELOAD=/tmp/shim3.so timeout 45 /tmp/cup 2>/tmp/cup_time.txt; echo "exit=$?"
grep -E "Elapsed|FAIL|ok|devices|name|compute" /tmp/cup_time.txt 2>/dev/null
echo "--- guest dump3 GPU_GET_INFO_V2 only ---"; sed -n '/GPU_GET_INFO_V2/,/GR_GET_INFO/p' /tmp/dump3.log 2>/dev/null
echo "=== dmesg after cup (RmInitAdapter / fault) ==="
sudo dmesg | grep -aiE "NVRM|RmInit|WPR2|fault|Xid|fail|timeout|init done" | tail -14
