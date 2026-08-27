#!/usr/bin/env bash
# cup8_run_guest.sh — ON THE GUEST. Same module/firmware/node prep as cup4_run_guest.sh,
# but builds + runs cup8 (grid matmul at scale (general-compute map-on-touch proof)). PTX is
# JITed in-guest (libnvidia-ptxjitcompiler must match libcuda). Env: CUP8_N (default 2048).
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
TIMEOUT=${CUP8_TIMEOUT:-180}
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

echo "=== ptxjit availability (REQUIRED for cuModuleLoadData PTX) ==="
ls -l "$GUESTLIB"/libnvidia-ptxjitcompiler* /usr/lib/x86_64-linux-gnu/libnvidia-ptxjitcompiler* 2>/dev/null || echo "  (none in GUESTLIB or system)"

gcc -O0 -g -o /tmp/cup8 /tmp/cup8.c -lcuda -lm 2>&1 | tail -2
echo "=== CONCURRENCY: two cup8 instances at once (fresh boot, one GSP) ==="
LD_LIBRARY_PATH=$GUESTLIB stdbuf -oL /tmp/cup8 >/tmp/cc_a.out 2>&1 &
PA=$!
LD_LIBRARY_PATH=$GUESTLIB stdbuf -oL /tmp/cup8 >/tmp/cc_b.out 2>&1 &
PB=$!
echo "  launched A=$PA B=$PB"
RA=; RB=
for s in $(seq 1 30); do
  sleep 4
  kill -0 $PA 2>/dev/null || { [ -z "$RA" ] && { wait $PA; RA=$?; echo "  A exited rc=$RA at $((s*4))s"; }; }
  kill -0 $PB 2>/dev/null || { [ -z "$RB" ] && { wait $PB; RB=$?; echo "  B exited rc=$RB at $((s*4))s"; }; }
  [ -n "$RA" ] && [ -n "$RB" ] && break
done
[ -z "$RA" ] && { echo "  A HUNG (state=$(awk '/^State/{print $2}' /proc/$PA/status 2>/dev/null))"; kill -INT $PA 2>/dev/null; RA=124; }
[ -z "$RB" ] && { echo "  B HUNG (state=$(awk '/^State/{print $2}' /proc/$PB/status 2>/dev/null))"; kill -INT $PB 2>/dev/null; RB=124; }
echo "=== A verdict ==="; grep -iE "VERDICT|RESULT" /tmp/cc_a.out | tail -1
echo "=== B verdict ==="; grep -iE "VERDICT|RESULT" /tmp/cc_b.out | tail -1
echo "=== CONCURRENCY rc: A=$RA B=$RB (0/0 = PASS) ==="
