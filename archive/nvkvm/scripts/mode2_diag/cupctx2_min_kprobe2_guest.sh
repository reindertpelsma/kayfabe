#!/usr/bin/env bash
# cupctx2_min_kprobe2_guest.sh — ON GUEST. trace_kprobe refuses the nvidia
# (notrace) module, so use the raw register_kprobe() API via a tiny built module.
# Plants a kprobe at channelWaitForFinishPayload (resolved by addr from kallsyms)
# logging pChannel (rdi) + targetPayload (rsi) on each entry. Runs cupctx2_min;
# the LAST "NVKVMKP cwfp" dmesg line = the CTX2 hang's wait {channel ptr,target}.
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
TIMEOUT=${MIN_TIMEOUT:-60}
sudo systemctl isolate multi-user.target 2>/dev/null || true
sleep 2
sudo rmmod nvkvm_cwfp_kprobe 2>/dev/null || true
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

# === build the register_kprobe module ===
KPD=/tmp/kp; rm -rf "$KPD"; mkdir -p "$KPD"
cat > "$KPD/nvkvm_cwfp_kprobe.c" <<'CEOF'
#include <linux/module.h>
#include <linux/kprobes.h>
#include <linux/ptrace.h>
static unsigned long addr;
module_param(addr, ulong, 0);
static struct kprobe kp;
static int pre(struct kprobe *p, struct pt_regs *regs){
    pr_info("NVKVMKP cwfp pChannel=0x%lx target=%llu(0x%llx)\n",
        (unsigned long)regs->di,
        (unsigned long long)regs->si,(unsigned long long)regs->si);
    return 0;
}
static int __init kpinit(void){
    int r;
    if(!addr){pr_err("NVKVMKP addr=0\n");return -EINVAL;}
    memset(&kp,0,sizeof(kp));
    kp.addr=(kprobe_opcode_t*)addr; kp.pre_handler=pre;
    r=register_kprobe(&kp);
    if(r<0){pr_err("NVKVMKP register_kprobe failed %d\n",r);return r;}
    pr_info("NVKVMKP planted at 0x%lx\n",addr);
    return 0;
}
static void __exit kpexit(void){unregister_kprobe(&kp);pr_info("NVKVMKP removed\n");}
module_init(kpinit); module_exit(kpexit);
MODULE_LICENSE("GPL");
CEOF
echo 'obj-m += nvkvm_cwfp_kprobe.o' > "$KPD/Makefile"
make -C /lib/modules/$(uname -r)/build M="$KPD" modules >/tmp/kp_build.log 2>&1 \
  || { echo "KP BUILD FAILED:"; tail -20 /tmp/kp_build.log; exit 1; }

ADDR=$(sudo grep -E 'channelWaitForFinishPayload\s+\[nvidia\]' /proc/kallsyms | awk '{print "0x"$1}')
echo "=== cwfp addr=$ADDR ; loading kprobe module ==="
sudo insmod "$KPD/nvkvm_cwfp_kprobe.ko" addr=$ADDR 2>&1 | tail -2
sudo dmesg | grep NVKVMKP | tail -2

sudo dmesg -C >/dev/null 2>&1 || true
gcc -O0 -g -o /tmp/cupctx2_min /tmp/cupctx2_min.c -lcuda 2>&1 | tail -3
echo "=== cupctx2_min (timeout ${TIMEOUT}s, ITERS=${ITERS:-2}) ==="
LD_LIBRARY_PATH=$GUESTLIB ITERS=${ITERS:-2} timeout --signal=INT "$TIMEOUT" stdbuf -oL -eL /tmp/cupctx2_min
echo "=== cupctx2_min exit rc=$? ==="

echo "=== cwfp entry count + LAST 12 (last line = CTX2 hang's {pChannel,target}) ==="
echo "  count: $(sudo dmesg | grep -c 'NVKVMKP cwfp')"
sudo dmesg | grep 'NVKVMKP cwfp' | tail -12
echo "=== distinct (pChannel,target) tally ==="
sudo dmesg | grep 'NVKVMKP cwfp' | grep -oE 'pChannel=0x[0-9a-f]+ target=[0-9]+' | sort | uniq -c | tail -20
sudo rmmod nvkvm_cwfp_kprobe 2>/dev/null || true
echo "=== DONE_KPROBE2 ==="
