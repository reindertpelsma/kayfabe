#!/bin/bash
# Guest-side: build cup2, run under gdb to capture the cuCtxCreate SIGSEGV
# (faulting VA, $pc, backtrace, and which mapping each falls in).
set +e
NVMODS=/home/ubuntu/nvmods
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null; sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
sudo mknod /dev/nvidia-uvm c 235 0 2>/dev/null; sudo mknod /dev/nvidia-uvm-tools c 235 1 2>/dev/null
sudo chmod 666 /dev/nvidia* 2>/dev/null
sudo dmesg -C
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1
sudo chmod 666 /dev/nvidia-uvm* 2>/dev/null
rm -f /tmp/cup2; nvcc -g -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1 | head -3
which gdb >/dev/null 2>&1 || { echo "NO GDB"; sudo apt-get install -y gdb >/dev/null 2>&1; }
cat > /tmp/cup2.gdb <<'GDB'
set pagination off
set confirm off
handle SIGSEGV stop nopass
run
echo \n==== FAULT ====\n
printf "si_addr = %p\n", $_siginfo._sifields._sigfault.si_addr
printf "pc      = %p   (libcuda .text base 0x7ffff2366000, file-off 0x166000)\n", $pc
printf "sp=%p rbp=%p\n", $sp, $rbp
printf "rax=%p rbx=%p rcx=%p rdx=%p\n", $rax, $rbx, $rcx, $rdx
printf "rsi=%p rdi=%p r8=%p r9=%p\n", $rsi, $rdi, $r8, $r9
printf "r12=%p r13=%p r14=%p r15=%p\n", $r12, $r13, $r14, $r15
echo \n==== BACK-DISASM (how rbp got set) ====\n
x/50i $pc-120
echo \n==== INSN AT/AFTER PC ====\n
x/6i $pc
echo \n==== STACK SCAN (manual unwind: libcuda return addrs) ====\n
x/120gx $sp
echo \n==== eh_frame backtrace attempt ====\n
bt 40
GDB
echo "=== cup2 under gdb ==="
sudo timeout 90 gdb -batch -nx -x /tmp/cup2.gdb /tmp/cup2 2>&1 | \
  grep -vE "^\[Thread|^\[New Thread|Reading symbols|no debugging symbols" | head -120
echo "=== dmesg (fault/nvrm/xid) ==="
sudo dmesg | grep -aiE "segfault|NVRM|Xid|fault|trap" | tail -10
