#!/usr/bin/env bash
# cupctx2_min_kprobe3_guest.sh — ON GUEST. cont.25 ground-truth probe.
# Plants register_kprobe on channelWaitForFinishPayload and, using OBJCHANNEL /
# MEMORY_DESCRIPTOR field offsets computed from the open driver headers
# (g_mem_mgr_nvoc.h / g_mem_desc_nvoc.h), walks
#   pChannel -> pChannelBufferMemdesc -> _pteArray
# to report the EXACT physical address the guest's finishPayload semaphore lives
# at (gpfifo+0x8004 == pbGpuVA+finishPayloadOffset), its aperture (SYSMEM/FBMEM),
# and — when SYSMEM — the CURRENT value there (what channelWaitForFinishPayload
# actually polls). Compare finPA against the emulator's forge target to settle
# WHERE to write. Self-validating: finPayOff must ≈ channelPbSize+0x8004 and
# pbGpuVA ≈ 0x120060000.
#
# OBJCHANNEL offsets:  channelPbSize 0x40, finishPayloadOffset 0x50, finishPayload 0x58,
#                      pbGpuVA 0xb8, pChannelBufferMemdesc 0xd8, pbCpuVA 0xe8, bUseBar1 0x194
# MEMORY_DESCRIPTOR:   _flags 0x8, _pageSize 0x10, Size 0x30, _addressSpace 0x68,
#                      pageArrayGranularity 0x160, _pteArray 0x170
#   NV_ADDRESS_SPACE: 0=UNKNOWN 1=SYSMEM 2=FBMEM 3=REGMEM 4=VIRTUAL
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
TIMEOUT=${MIN_TIMEOUT:-60}
sudo systemctl isolate multi-user.target 2>/dev/null || true
sleep 2
sudo rmmod nvkvm_cwfp_kp3 2>/dev/null || true
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

KPD=/tmp/kp3; rm -rf "$KPD"; mkdir -p "$KPD"
cat > "$KPD/nvkvm_cwfp_kp3.c" <<'CEOF'
#include <linux/module.h>
#include <linux/kprobes.h>
#include <linux/ptrace.h>
#include <linux/mm.h>
#include <linux/io.h>
#include <linux/uaccess.h>
extern phys_addr_t slow_virt_to_phys(void *);
static unsigned long addr;
module_param(addr, ulong, 0);
static struct kprobe kp;
static u32 rd32(void *p){ u32 v=0; if(copy_from_kernel_nofault(&v,p,4)) return 0xdeadbeef; return v; }
static u64 rd64(void *p){ u64 v=0; if(copy_from_kernel_nofault(&v,p,8)) return 0xdeadbeefULL; return v; }
static int pre(struct kprobe *kpb, struct pt_regs *regs){
    unsigned long ch = regs->di;
    unsigned long target = regs->si;
    u32 pbsz=rd32((void*)(ch+0x40)), fpo=rd32((void*)(ch+0x50)), fpv=rd32((void*)(ch+0x58));
    u64 pbgpuva=rd64((void*)(ch+0xb8));
    u32 usebar1=rd32((void*)(ch+0x194)) & 0xff;
    u64 md=rd64((void*)(ch+0xd8));
    u64 pbcpu=rd64((void*)(ch+0xe8));
    pr_info("NVKVMKP3 cwfp ch=0x%lx target=%lu pbSize=0x%x finPayOff=0x%x finPayloadField=%u pbGpuVA=0x%llx bUseBar1=%u memdesc=0x%llx pbCpuVA=0x%llx\n",
        ch,target,pbsz,fpo,fpv,pbgpuva,usebar1,md,pbcpu);
    /* GROUND TRUTH: pbCpuVA is the channel-buffer mapping the guest actually reads
     * (BAR1 ioremap when bUseBar1). slow_virt_to_phys gives the guest-physical
     * address the finishPayload read hits (a BAR1-window GPA for bUseBar1), and the
     * value there is exactly what channelWaitForFinishPayload polls. */
    if(pbcpu && pbcpu!=0xdeadbeefULL){
        void *finva = (void*)(pbcpu + fpo);
        phys_addr_t finphys = slow_virt_to_phys(finva);
        u32 curval = rd32(finva);   /* the EXACT value the guest sees (MMIO read if BAR1) */
        pr_info("NVKVMKP3   GT: finCpuVA=0x%llx finGPA(slow_virt_to_phys)=0x%llx CURVAL=%u (need>=target=%lu) bUseBar1=%u\n",
            (u64)(uintptr_t)finva,(u64)finphys,curval,target,usebar1);
    }
    if(md && md!=0xdeadbeefULL){
        u64 flags=rd64((void*)((u8*)md+0x8));
        u64 pgsz =rd64((void*)((u8*)md+0x10));
        u64 sz   =rd64((void*)((u8*)md+0x30));
        u32 as   =rd32((void*)((u8*)md+0x68));
        u32 gran =rd32((void*)((u8*)md+0x160));
        u64 pte0 =rd64((void*)((u8*)md+0x170));
        u64 g = gran ? gran : (pgsz?pgsz:0x1000);
        u32 idx = (g? (u32)(fpo/g) : 0);
        u64 pte = rd64((void*)((u8*)md+0x170+(u64)idx*8));
        u64 pa  = pte + (g? (fpo%g) : fpo);
        pr_info("NVKVMKP3   memdesc: addrSpace=%u(0=UNK,1=SYS,2=FB) size=0x%llx pageSize=0x%llx gran=0x%x flags=0x%llx pte[0]=0x%llx finPA=0x%llx idx=%u\n",
            as,sz,pgsz,gran,flags,pte0,pa,idx);
        if(as==1){ /* SYSMEM: phys is a guest GPA in the direct map */
            u32 curval=0;
            if(!copy_from_kernel_nofault(&curval, phys_to_virt(pa), 4))
                pr_info("NVKVMKP3   SYSMEM finishPayload CURRENT value @finPA=0x%llx => %u (need >= target=%lu)\n", pa, curval, target);
            else
                pr_info("NVKVMKP3   SYSMEM read @finPA=0x%llx FAULTED\n", pa);
        } else {
            pr_info("NVKVMKP3   non-SYSMEM(addrSpace=%u): finishPayload in FB/other; emulator must write that aperture\n", as);
        }
    }
    return 0;
}
static int __init kpinit(void){
    int r; if(!addr){pr_err("NVKVMKP3 addr=0\n");return -EINVAL;}
    memset(&kp,0,sizeof(kp)); kp.addr=(kprobe_opcode_t*)addr; kp.pre_handler=pre;
    r=register_kprobe(&kp); if(r<0){pr_err("NVKVMKP3 register failed %d\n",r);return r;}
    pr_info("NVKVMKP3 planted at 0x%lx\n",addr); return 0;
}
static void __exit kpexit(void){unregister_kprobe(&kp);pr_info("NVKVMKP3 removed\n");}
module_init(kpinit); module_exit(kpexit);
MODULE_LICENSE("GPL");
CEOF
echo 'obj-m += nvkvm_cwfp_kp3.o' > "$KPD/Makefile"
make -C /lib/modules/$(uname -r)/build M="$KPD" modules >/tmp/kp3_build.log 2>&1 \
  || { echo "KP3 BUILD FAILED:"; tail -25 /tmp/kp3_build.log; exit 1; }

ADDR=$(sudo grep -E 'channelWaitForFinishPayload\s+\[nvidia\]' /proc/kallsyms | awk '{print "0x"$1}')
echo "=== cwfp addr=$ADDR ; loading kp3 ==="
sudo insmod "$KPD/nvkvm_cwfp_kp3.ko" addr=$ADDR 2>&1 | tail -2
sudo dmesg | grep NVKVMKP3 | tail -2

sudo dmesg -C >/dev/null 2>&1 || true
gcc -O0 -g -o /tmp/cupctx2_min /tmp/cupctx2_min.c -lcuda 2>&1 | tail -3
echo "=== cupctx2_min (timeout ${TIMEOUT}s, ITERS=${ITERS:-2}) ==="
LD_LIBRARY_PATH=$GUESTLIB ITERS=${ITERS:-2} timeout --signal=INT "$TIMEOUT" stdbuf -oL -eL /tmp/cupctx2_min
echo "=== cupctx2_min exit rc=$? ==="

echo "=== NVKVMKP3 ground-truth dump (last block = CTX2 target=84 hang) ==="
sudo dmesg | grep NVKVMKP3 | tail -20
sudo rmmod nvkvm_cwfp_kp3 2>/dev/null || true
echo "=== DONE_KPROBE3 ==="
