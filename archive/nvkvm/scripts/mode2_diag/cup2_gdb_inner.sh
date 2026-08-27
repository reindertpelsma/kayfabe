#!/bin/bash
# Runs INSIDE the guest: set up the open driver, then run cup2 under gdb to catch the rbp=0
# SIGSEGV after 0xc7c0 and dump backtrace + registers + disassembly (find the load that
# returned 0 into rbp). libcuda is stripped -> frames show as libcuda+offset (still localizes).
set +e
sudo ln -sf libcuda.so.580.159.04 "/usr/local/nvidia-guest/lib/libcuda.so.1"; sudo rm -f "/lib/x86_64-linux-gnu/libcuda.so.1"; sudo ln -sf libcuda.so.580.159.04 "/lib/x86_64-linux-gnu/libcuda.so.1"
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia 2>/dev/null; sudo modprobe ecdh_generic ecc 2>/dev/null
for n in "nvidia0 c 195 0" "nvidiactl c 195 255" "nvidia-uvm c 235 0" "nvidia-uvm-tools c 235 1"; do sudo mknod /dev/${n% *} ${n#* } 2>/dev/null; done
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null
sudo sysctl -w kernel.yama.ptrace_scope=0 2>/dev/null
sudo insmod "/home/ubuntu/nvmods/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "/home/ubuntu/nvmods/nvidia-uvm.ko" 2>&1 | tail -1; sudo chmod 666 /dev/nvidia-uvm* 2>/dev/null
rm -f /tmp/cup2; nvcc -o /tmp/cup2 /tmp/cup2.c -lcuda -L"/lib/x86_64-linux-gnu" 2>&1 | head -2
which gdb >/dev/null 2>&1 || { sudo apt-get install -y gdb >/dev/null 2>&1; }
cat > /tmp/gdb.cmds <<'GDB'
set pagination off
set confirm off
handle SIGSEGV stop nopass
run
echo \n===== SIGNAL CAUGHT =====\n
info registers rax rbx rcx rdx rsi rdi rbp rsp r12 r13 r14 r15 rip
echo \n===== FULL FUNCTION DISAS (find where rbp is assigned) =====\n
disassemble $rip-0x140, $rip+0x10
echo \n===== STACK around frame (look for overflow / zeroed saved-rbp) =====\n
x/96gx $rsp-0x80
echo \n===== r13 object first 0x80 bytes =====\n
x/16gx $r13
GDB
LD_LIBRARY_PATH="/usr/local/nvidia-guest/lib:/lib/x86_64-linux-gnu" \
  gdb -batch -x /tmp/gdb.cmds --args /tmp/cup2 >/tmp/gdb_out.txt 2>&1
echo "=== gdb done, $(wc -l < /tmp/gdb_out.txt) lines ==="
tail -60 /tmp/gdb_out.txt
