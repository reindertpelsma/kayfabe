#!/bin/bash
# Guest-side: run cup2, let it hang in cuCtxCreate, attach gdb and backtrace
# every thread to find what libcuda is polling/waiting on.
set +e
NVMODS=/home/ubuntu/nvmods
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo sysctl -w kernel.yama.ptrace_scope=0 >/dev/null 2>&1
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1
UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null; sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0 2>/dev/null; sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null
sudo ln -sf /usr/local/nvidia-guest/lib/libcuda.so.580.159.04 /lib/x86_64-linux-gnu/libcuda.so.1
which gdb >/dev/null 2>&1 || sudo apt-get install -y gdb >/dev/null 2>&1
rm -f /tmp/cup2; nvcc -g -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1 | head -3
echo "=== launch cup2 (background), wait for cuCtxCreate hang ==="
LD_LIBRARY_PATH="/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu" /tmp/cup2 > /tmp/cup2_run.txt 2>&1 &
CPID=$!
# wait until it has passed the device queries (it prints totalMem just before cuCtxCreate)
for i in $(seq 1 25); do grep -q "totalMem" /tmp/cup2_run.txt 2>/dev/null && break; sleep 1; done
sleep 4   # let it settle into the cuCtxCreate wait
echo "cup2 pid=$CPID state=$(cat /proc/$CPID/status 2>/dev/null | awk '/State/{print $2}')"
echo "--- cup2 stdout so far ---"; cat /tmp/cup2_run.txt
echo "=== gdb attach: all-thread backtraces (x2 samples) ==="
sudo gdb -batch -nx -p $CPID \
  -ex "set pagination off" \
  -ex "info threads" \
  -ex "thread apply all bt 12" \
  -ex "continue &" -ex "shell sleep 2" -ex "interrupt" \
  -ex "echo \n==== SAMPLE 2 ====\n" \
  -ex "thread apply all bt 6" \
  2>&1 | grep -vE "Reading symbols|no debugging symbols|^\[New LWP|^\[Thread debug" | head -160
kill -9 $CPID 2>/dev/null
