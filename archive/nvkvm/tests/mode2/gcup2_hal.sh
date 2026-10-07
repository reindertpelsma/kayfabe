#!/bin/bash
# Guest-side: break at the HAL dispatch in the cuCtxCreate crash function and
# inspect the call-target function pointer (*(rax+0x560)) to see if it's valid
# libcuda .text or garbage/NULL. Anchored to exported cuVDPAUCtxCreate (crash fn =
# cuVDPAUCtxCreate+0xc0f00; HAL call site = +0xc0ecf; earlier HAL call = +0xc0e9f).
set +e
NVMODS=/home/ubuntu/nvmods
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia_uvm nvidia 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null; sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
sudo mknod /dev/nvidia-uvm c 235 0 2>/dev/null; sudo mknod /dev/nvidia-uvm-tools c 235 1 2>/dev/null
sudo chmod 666 /dev/nvidia* /dev/nvidia-uvm* 2>/dev/null
sudo dmesg -C
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1
sudo chmod 666 /dev/nvidia-uvm* 2>/dev/null
rm -f /tmp/cup2; nvcc -g -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1 | head -2
cat > /tmp/hal.gdb <<'GDB'
set pagination off
set confirm off
python
import gdb
st = {"base": None}
class RetBP(gdb.Breakpoint):
    def __init__(self, spec, tag):
        super(RetBP, self).__init__(spec, gdb.BP_BREAKPOINT, internal=False)
        self.tag = tag
    def stop(self):
        rbp = int(gdb.parse_and_eval("$rbp")) & 0xffffffffffffffff
        rsp = int(gdb.parse_and_eval("$rsp")) & 0xffffffffffffffff
        if rbp < 0x100000:   # corrupted (near-NULL) -> the crash invocation
            gdb.write("[%s CORRUPT] rbp=0x%x rsp=0x%x\n" % (self.tag, rbp, rsp))
        return False
class CtxBP(gdb.Breakpoint):
    def stop(self):
        if st["base"] is None:
            b = int(gdb.parse_and_eval("(unsigned long)&cuVDPAUCtxCreate")) & 0xffffffffffffffff
            st["base"] = b
            RetBP("*0x%x" % (b + 0xf2628), "497b50 RET")   # 0x497c88
            RetBP("*0x%x" % (b + 0xd5698), "47acc0 RET")   # 0x47acf8
            gdb.write("base=0x%x; ret-BPs armed\n" % b)
        return False
CtxBP("cuCtxCreate_v2")
end
handle SIGSEGV stop nopass
run
echo \n==CRASH==\n
printf "pc=%p rbp=%p rsp=%p\n", $pc, $rbp, $rsp
GDB
echo "=== gdb HAL inspect ==="
sudo timeout 90 gdb -batch -nx -x /tmp/hal.gdb /tmp/cup2 2>&1 | grep -vE "Reading symbols|no debugging symbols|Thread|New Thread|^\[" | head -80
echo "=== dmesg ==="
sudo dmesg | grep -aiE "segfault|NVRM|Xid" | tail -4
