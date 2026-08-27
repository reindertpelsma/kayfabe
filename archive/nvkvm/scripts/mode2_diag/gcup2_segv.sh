#!/bin/bash
set +e
NVMODS=/home/ubuntu/nvmods
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo sysctl -w kernel.yama.ptrace_scope=0 >/dev/null 2>&1
sudo dmesg -C
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
cat > /tmp/cup2.gdb <<'GDB'
set pagination off
set confirm off
handle SIGSEGV stop nopass
run
echo \n==== FAULT ====\n
printf "si_addr = %p\n", $_siginfo._sifields._sigfault.si_addr
printf "pc=%p sp=%p rbp=%p\n", $pc, $sp, $rbp
info sharedlibrary libcuda
echo \n==== BACKTRACE ====\n
bt 30
echo \n==== INSN AT PC ====\n
x/4i $pc
echo \n==== registers ====\n
printf "rax=%p rbx=%p rcx=%p rdx=%p rsi=%p rdi=%p\n", $rax,$rbx,$rcx,$rdx,$rsi,$rdi
GDB
echo "=== cup2 under gdb ==="
LD_LIBRARY_PATH="/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu" \
  sudo -E timeout 90 gdb -batch -nx -x /tmp/cup2.gdb /tmp/cup2 2>&1 | \
  grep -vE "Reading symbols|no debugging symbols|^\[New Thread|^\[Thread" | head -90
echo "=== dmesg ==="; sudo dmesg | grep -aiE "segfault|NVRM|Xid|trap" | tail -8
