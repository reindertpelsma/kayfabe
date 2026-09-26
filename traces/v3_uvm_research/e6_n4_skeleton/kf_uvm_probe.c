// SPDX-License-Identifier: MIT
//
// kf_uvm_probe.c  -- E6 / N4 skeleton
//
// A MINIMAL out-of-tree kernel module that takes NVIDIA UVM's place on the
// stock nvidia.ko exported `nvUvmInterface*` contract, with nvidia-uvm
// UNLOADED. It uses ONLY exported nvUvmInterface* functions declared in
// nv_uvm_interface.h (types in nv_uvm_types.h).
//
// ORDERING NOTE (measured): RM's nvGpuOpsGetGpuInfo() and NV01_DEVICE_0 alloc
// gate on osIsGpuAccessible(pGpu), which on Linux checks that the *calling
// process* has an open fd to /dev/nvidiaN (nv_is_gpu_accessible ->
// iterate_fd). module_init runs in the insmod process, which has no GPU fd,
// so those stages return NV_ERR_INSUFFICIENT_PERMISSIONS. Real UVM is called
// from a CUDA process that holds the GPU open. Therefore we do the sole-owner
// callback registration in module_init (no GPU fd needed) and run the
// GPU-touching stages from a /proc trigger written by a helper that holds
// /dev/nvidia0 open (so `current` has the fd).
//
//   STAGE A  RegisterUvmCallbacks (init; must NOT be IN_USE) + SessionCreate,
//            GetGpuInfo, RegisterGpu, DeviceCreate, AddressSpaceCreate (trigger)
//   STAGE B  InitFaultInfo + OwnPageFaultIntr(TRUE); print buffer/get/put/mask
//   STAGE C  (best effort, do_stage_c=1): TsgAllocate + ChannelAllocate (CE),
//            build a CE copy from an UNMAPPED src VA, ring the doorbell, poll
//            the fault buffer PUT, dump the 32-byte packet, decode faulting VA.

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/delay.h>
#include <linux/slab.h>
#include <linux/pci.h>
#include <linux/ktime.h>
#include <linux/string.h>
#include <linux/proc_fs.h>
#include <linux/uaccess.h>

#ifndef NVBIT
#define NVBIT(b) (1U << (b))
#endif
#include "nv_uvm_types.h"

#define KLOG "kf_uvm: "

typedef struct {
    struct pci_dev *pci_dev;
    NvU64 dma_addressable_start;
    NvU64 dma_addressable_limit;
} KfUvmGpuPlatformInfo;

extern NV_STATUS nvUvmInterfaceRegisterUvmCallbacks(struct UvmOpsUvmEvents *importedUvmOps);
extern void      nvUvmInterfaceDeRegisterUvmOps(void);
extern NV_STATUS nvUvmInterfaceSessionCreate(uvmGpuSessionHandle *session, UvmPlatformInfo *platformInfo);
extern NV_STATUS nvUvmInterfaceSessionDestroy(uvmGpuSessionHandle session);
extern NV_STATUS nvUvmInterfaceRegisterGpu(const NvProcessorUuid *gpuUuid, KfUvmGpuPlatformInfo *gpuInfo);
extern void      nvUvmInterfaceUnregisterGpu(const NvProcessorUuid *gpuUuid);
extern NV_STATUS nvUvmInterfaceGetGpuInfo(const NvProcessorUuid *gpuUuid, const UvmGpuClientInfo *pGpuClientInfo, UvmGpuInfo *pGpuInfo);
extern NV_STATUS nvUvmInterfaceDeviceCreate(uvmGpuSessionHandle session, const UvmGpuInfo *pGpuInfo, const NvProcessorUuid *gpuUuid, uvmGpuDeviceHandle *device, NvBool bCreateSmcPartition);
extern void      nvUvmInterfaceDeviceDestroy(uvmGpuDeviceHandle device);
extern NV_STATUS nvUvmInterfaceAddressSpaceCreate(uvmGpuDeviceHandle device, unsigned long long vaBase, unsigned long long vaSize, NvBool enableAts, uvmGpuAddressSpaceHandle *vaSpace, UvmGpuAddressSpaceInfo *vaSpaceInfo);
extern void      nvUvmInterfaceAddressSpaceDestroy(uvmGpuAddressSpaceHandle vaSpace);
extern NV_STATUS nvUvmInterfaceInitFaultInfo(uvmGpuDeviceHandle device, UvmGpuFaultInfo *pFaultInfo);
extern NV_STATUS nvUvmInterfaceDestroyFaultInfo(uvmGpuDeviceHandle device, UvmGpuFaultInfo *pFaultInfo);
extern NV_STATUS nvUvmInterfaceOwnPageFaultIntr(uvmGpuDeviceHandle device, NvBool bOwnInterrupts);
extern NV_STATUS nvUvmInterfaceTsgAllocate(uvmGpuAddressSpaceHandle vaSpace, const UvmGpuTsgAllocParams *allocParams, uvmGpuTsgHandle *tsg);
extern void      nvUvmInterfaceTsgDestroy(uvmGpuTsgHandle tsg);
extern NV_STATUS nvUvmInterfaceChannelAllocate(const uvmGpuTsgHandle tsg, const UvmGpuChannelAllocParams *allocParams, uvmGpuChannelHandle *channel, UvmGpuChannelInfo *channelInfo);
extern void      nvUvmInterfaceChannelDestroy(uvmGpuChannelHandle channel);
extern NV_STATUS nvUvmInterfaceMemoryAllocFB(uvmGpuAddressSpaceHandle vaSpace, NvLength length, UvmGpuPointer *gpuPointer, UvmGpuAllocInfo *allocInfo);
extern void      nvUvmInterfaceMemoryFree(uvmGpuAddressSpaceHandle vaSpace, UvmGpuPointer gpuPointer);
extern NV_STATUS nvUvmInterfaceMemoryCpuMap(uvmGpuAddressSpaceHandle vaSpace, UvmGpuPointer gpuPointer, NvLength length, void **cpuPtr, NvU64 pageSize);
extern void      nvUvmInterfaceMemoryCpuUnMap(uvmGpuAddressSpaceHandle vaSpace, void *cpuPtr);
extern NV_STATUS nvUvmInterfaceFlushReplayableFaultBuffer(UvmGpuFaultInfo *pFaultInfo, NvBool bCopyAndFlush);

static char *gpu_uuid = "";
module_param(gpu_uuid, charp, 0444);
MODULE_PARM_DESC(gpu_uuid, "Physical GPU UUID from nvidia-smi (GPU-...)");
static int do_stage_c = 0;
module_param(do_stage_c, int, 0644);
MODULE_PARM_DESC(do_stage_c, "1 = attempt Stage C fault generation");

#define SUBCH_CE                4
#define DMA_SEC_OP_INC_METHOD   1u
static inline u32 METHOD_INC(u32 subch, u32 addr, u32 count)
{ return (DMA_SEC_OP_INC_METHOD << 29) | (count << 16) | (subch << 13) | (addr >> 2); }
#define NVC56F_SET_OBJECT       0x0000
#define NVC7B5_OFFSET_IN_UPPER   0x0400
#define NVC7B5_OFFSET_IN_LOWER   0x0404
#define NVC7B5_OFFSET_OUT_UPPER  0x0408
#define NVC7B5_OFFSET_OUT_LOWER  0x040c
#define NVC7B5_LINE_LENGTH_IN    0x0418
#define NVC7B5_LAUNCH_DMA        0x0300
#define NVC7B5_LAUNCH_DMA_VAL    ( (2u) | (1u<<2) )

static struct UvmOpsUvmEvents g_events;
static NvBool g_cb_registered = NV_FALSE;
static uvmGpuSessionHandle g_session;         static NvBool g_have_session = NV_FALSE;
static NvProcessorUuid g_uuid;
static NvBool g_gpu_registered = NV_FALSE;
static uvmGpuDeviceHandle g_device;           static NvBool g_have_device = NV_FALSE;
static uvmGpuAddressSpaceHandle g_vaspace;    static NvBool g_have_vaspace = NV_FALSE;
static UvmGpuInfo g_gpuinfo;
static UvmGpuFaultInfo g_fault;               static NvBool g_have_fault = NV_FALSE;
static NvBool g_own_intr = NV_FALSE;
static uvmGpuTsgHandle g_tsg;                 static NvBool g_have_tsg = NV_FALSE;
static uvmGpuChannelHandle g_channel;         static NvBool g_have_channel = NV_FALSE;
static NvBool g_stages_done = NV_FALSE;

static NV_STATUS kf_isr_top_half(const NvProcessorUuid *uuid) { return NV_ERR_NO_INTR_PENDING; }
static NV_STATUS kf_suspend(void) { return NV_OK; }
static NV_STATUS kf_resume(void)  { return NV_OK; }
static NV_STATUS kf_start_dev(const NvProcessorUuid *u) { return NV_OK; }
static NV_STATUS kf_stop_dev(const NvProcessorUuid *u)  { return NV_OK; }
static NV_STATUS kf_drainP2P(const NvProcessorUuid *u)  { return NV_OK; }
static NV_STATUS kf_resumeP2P(const NvProcessorUuid *u) { return NV_OK; }

static int parse_uuid(const char *s, NvProcessorUuid *out)
{
    int n = 0; const char *p = s;
    if (!strncmp(p, "GPU-", 4)) p += 4;
    while (*p && n < 16) {
        int hi, lo;
        while (*p == '-') p++;
        if (!*p) break;
        hi = hex_to_bin(*p++); if (hi < 0) return -1;
        while (*p == '-') p++;
        if (!*p) return -1;
        lo = hex_to_bin(*p++); if (lo < 0) return -1;
        out->uuid[n++] = (u8)((hi << 4) | lo);
    }
    return (n == 16) ? 0 : -1;
}

static void hexdump32(const char *tag, const volatile u8 *b)
{
    int i; char line[3*32 + 1];
    for (i = 0; i < 32; i++) scnprintf(line + i*3, 4, "%02x ", b[i]);
    pr_info(KLOG "%s: %s\n", tag, line);
}

static void unwind(void)
{
    if (g_have_channel)  { nvUvmInterfaceChannelDestroy(g_channel); g_have_channel = NV_FALSE; }
    if (g_have_tsg)      { nvUvmInterfaceTsgDestroy(g_tsg); g_have_tsg = NV_FALSE; }
    if (g_own_intr)      { nvUvmInterfaceOwnPageFaultIntr(g_device, NV_FALSE);
                           pr_info(KLOG "cleanup: returned fault-intr ownership to RM\n"); g_own_intr = NV_FALSE; }
    if (g_have_fault)    { nvUvmInterfaceDestroyFaultInfo(g_device, &g_fault); g_have_fault = NV_FALSE; }
    if (g_have_vaspace)  { nvUvmInterfaceAddressSpaceDestroy(g_vaspace); g_have_vaspace = NV_FALSE; }
    if (g_have_device)   { nvUvmInterfaceDeviceDestroy(g_device); g_have_device = NV_FALSE; }
    if (g_gpu_registered){ nvUvmInterfaceUnregisterGpu(&g_uuid); g_gpu_registered = NV_FALSE; }
    if (g_have_session)  { nvUvmInterfaceSessionDestroy(g_session); g_have_session = NV_FALSE; }
}

static void stage_c(void)
{
    NV_STATUS st;
    UvmGpuTsgAllocParams tsgp;
    UvmGpuChannelAllocParams chp;
    UvmGpuChannelInfo chi;
    UvmGpuAllocInfo ai;
    UvmGpuPointer pb_va = 0;
    void *pb_cpu = NULL;
    u32 *pb = NULL;
    u32 put0 = 0, put1, i, pb_dwords;
    const u64 UNMAPPED_SRC = 0x0000700000000000ULL;
    u32 ceClass = g_gpuinfo.ceClass;
    ktime_t t0 = 0, tobs = 0;

    pr_info(KLOG "STAGE C: begin (CE fault generator) ceClass=0x%x\n", ceClass);

    memset(&tsgp, 0, sizeof(tsgp));
    tsgp.engineType  = UVM_GPU_CHANNEL_ENGINE_TYPE_CE;
    tsgp.engineIndex = 0;
    st = nvUvmInterfaceTsgAllocate(g_vaspace, &tsgp, &g_tsg);
    pr_info(KLOG "STAGE C: TsgAllocate(CE,0) -> 0x%x tsg=%px\n", st, (void*)g_tsg);
    if (st != NV_OK) { pr_err(KLOG "STAGE C: BLOCKED at TsgAllocate 0x%x\n", st); return; }
    g_have_tsg = NV_TRUE;

    memset(&chp, 0, sizeof(chp));
    chp.numGpFifoEntries = 32;
    chp.gpFifoLoc = UVM_BUFFER_LOCATION_SYS;
    chp.gpPutLoc  = UVM_BUFFER_LOCATION_SYS;
    memset(&chi, 0, sizeof(chi));
    st = nvUvmInterfaceChannelAllocate(g_tsg, &chp, &g_channel, &chi);
    pr_info(KLOG "STAGE C: ChannelAllocate -> 0x%x channel=%px class=0x%x hwRunlist=%u hwChid=%u\n",
            st, (void*)g_channel, chi.channelClassNum, chi.hwRunlistId, chi.hwChannelId);
    if (st != NV_OK) { pr_err(KLOG "STAGE C: BLOCKED at ChannelAllocate 0x%x\n", st); return; }
    g_have_channel = NV_TRUE;
    pr_info(KLOG "STAGE C: gpFifoEntries=%px gpPut=%px numGpFifo=%u workSubmitOff=%px pWorkToken=%px token=0x%x\n",
            (void*)chi.gpFifoEntries, (void*)chi.gpPut, chi.numGpFifoEntries,
            (void*)chi.workSubmissionOffset, (void*)chi.pWorkSubmissionToken, chi.workSubmissionToken);

    memset(&ai, 0, sizeof(ai));
    st = nvUvmInterfaceMemoryAllocFB(g_vaspace, 128*1024, &pb_va, &ai);
    pr_info(KLOG "STAGE C: MemoryAllocFB(128k) -> 0x%x gpuVa=0x%llx pageSize=%llu\n",
            st, (unsigned long long)pb_va, (unsigned long long)ai.pageSize);
    if (st != NV_OK) { pr_err(KLOG "STAGE C: BLOCKED at MemoryAllocFB 0x%x\n", st); return; }
    st = nvUvmInterfaceMemoryCpuMap(g_vaspace, pb_va, 128*1024, &pb_cpu, ai.pageSize);
    pr_info(KLOG "STAGE C: MemoryCpuMap -> 0x%x cpu=%px\n", st, pb_cpu);
    if (st != NV_OK) { pr_err(KLOG "STAGE C: BLOCKED at MemoryCpuMap 0x%x\n", st); return; }

    // methods at +64KB in the buffer; dst = pb_va (mapped), src = UNMAPPED_SRC
    {
        u64 methods_gpu = (u64)pb_va + 65536;
        u32 *m = (u32*)((u8*)pb_cpu + 65536);
        volatile unsigned *gpput = chi.gpPut;
        u32 *gpfifo = (u32*)chi.gpFifoEntries;
        i = 0;
        m[i++] = METHOD_INC(SUBCH_CE, NVC56F_SET_OBJECT, 1);         m[i++] = ceClass;
        m[i++] = METHOD_INC(SUBCH_CE, NVC7B5_OFFSET_IN_UPPER, 2);
        m[i++] = (u32)(UNMAPPED_SRC >> 32);                          m[i++] = (u32)(UNMAPPED_SRC & 0xffffffff);
        m[i++] = METHOD_INC(SUBCH_CE, NVC7B5_OFFSET_OUT_UPPER, 2);
        m[i++] = (u32)((u64)pb_va >> 32);                            m[i++] = (u32)((u64)pb_va & 0xffffffff);
        m[i++] = METHOD_INC(SUBCH_CE, NVC7B5_LINE_LENGTH_IN, 1);     m[i++] = 256;
        m[i++] = METHOD_INC(SUBCH_CE, NVC7B5_LAUNCH_DMA, 1);         m[i++] = NVC7B5_LAUNCH_DMA_VAL;
        pb_dwords = i;
        pb = m;

        if (!gpfifo || !gpput || !chi.workSubmissionOffset) {
            pr_err(KLOG "STAGE C: channel submit fields NULL (gpfifo=%px gpput=%px wso=%px) -- cannot submit\n",
                   (void*)gpfifo, (void*)gpput, (void*)chi.workSubmissionOffset);
            return;
        }
        gpfifo[0] = (u32)(methods_gpu & 0xFFFFFFFCULL);
        gpfifo[1] = (u32)((methods_gpu >> 32) & 0xff) | (pb_dwords << 10);
        pr_info(KLOG "STAGE C: GPFIFO=0x%08x 0x%08x methods_gpu=0x%llx dwords=%u\n",
                gpfifo[0], gpfifo[1], (unsigned long long)methods_gpu, pb_dwords);

        put0 = g_fault.replayable.pFaultBufferPut ? *g_fault.replayable.pFaultBufferPut : 0;
        pr_info(KLOG "STAGE C: fault PUT before submit=%u  (*GET=%u)\n",
                put0, g_fault.replayable.pFaultBufferGet ? *g_fault.replayable.pFaultBufferGet : 0xffffffff);
        wmb();
        *gpput = 1;
        wmb();
        {
            u32 tok = chi.pWorkSubmissionToken ? *chi.pWorkSubmissionToken : chi.workSubmissionToken;
            t0 = ktime_get();
            *chi.workSubmissionOffset = tok;
            wmb();
            pr_info(KLOG "STAGE C: doorbell rung token=0x%x\n", tok);
        }
    }

    for (i = 0; i < 3000; i++) {
        put1 = g_fault.replayable.pFaultBufferPut ? *g_fault.replayable.pFaultBufferPut : 0;
        if (put1 != put0) { tobs = ktime_get(); break; }
        udelay(1000);
    }
    put1 = g_fault.replayable.pFaultBufferPut ? *g_fault.replayable.pFaultBufferPut : 0;
    pr_info(KLOG "STAGE C: fault PUT after poll=%u (was %u) after ~%u ms\n", put1, put0, i);
    if (put1 != put0 && g_fault.replayable.bufferAddress) {
        u32 idx = put0;
        const volatile u8 *pkt = (const volatile u8*)g_fault.replayable.bufferAddress + (u64)idx*32;
        s64 lat_ns = ktime_to_ns(ktime_sub(tobs, t0));
        const volatile u32 *dw = (const volatile u32*)pkt;
        u64 faddr = ((u64)dw[1] << 32) | dw[0];
        pr_info(KLOG "STAGE C: FAULT OBSERVED idx=%u fault-to-module latency=%lld ns\n", idx, lat_ns);
        hexdump32("STAGE C: raw fault packet", pkt);
        pr_info(KLOG "STAGE C: decoded fault VA(dw0/1)=0x%llx (expected src ~0x%llx)\n",
                (unsigned long long)(faddr & ~0xfffULL), (unsigned long long)UNMAPPED_SRC);
        pr_info(KLOG "STAGE C: PARTIAL PASS -- real HW fault reached the module\n");
    } else {
        pr_info(KLOG "STAGE C: no fault packet observed (PUT did not advance)\n");
    }
}

static void run_stages(void)
{
    NV_STATUS st;
    UvmPlatformInfo platInfo;
    KfUvmGpuPlatformInfo regInfo;
    UvmGpuClientInfo clientInfo;
    UvmGpuAddressSpaceInfo vaInfo;

    if (g_stages_done) { pr_info(KLOG "stages already run; ignoring trigger\n"); return; }
    g_stages_done = NV_TRUE;

    pr_info(KLOG "==== trigger: running stages (do_stage_c=%d) ====\n", do_stage_c);

    memset(&platInfo, 0, sizeof(platInfo));
    st = nvUvmInterfaceSessionCreate(&g_session, &platInfo);
    pr_info(KLOG "STAGE A: SessionCreate -> 0x%x ats=%d cc=%d\n", st, platInfo.atsSupported, platInfo.confComputingEnabled);
    if (st != NV_OK) goto out;
    g_have_session = NV_TRUE;

    memset(&regInfo, 0, sizeof(regInfo));
    st = nvUvmInterfaceRegisterGpu(&g_uuid, &regInfo);
    pr_info(KLOG "STAGE A: RegisterGpu -> 0x%x pci_dev=%px dma=[%llx..%llx]\n", st, regInfo.pci_dev,
            (unsigned long long)regInfo.dma_addressable_start, (unsigned long long)regInfo.dma_addressable_limit);
    if (st != NV_OK) goto out;
    g_gpu_registered = NV_TRUE;

    memset(&g_gpuinfo, 0, sizeof(g_gpuinfo));
    memset(&clientInfo, 0, sizeof(clientInfo));
    st = nvUvmInterfaceGetGpuInfo(&g_uuid, &clientInfo, &g_gpuinfo);
    pr_info(KLOG "STAGE A: GetGpuInfo -> 0x%x name='%s' arch=0x%x impl=0x%x host=0x%x ce=0x%x compute=0x%x smc=%d\n",
            st, g_gpuinfo.name, g_gpuinfo.gpuArch, g_gpuinfo.gpuImplementation,
            g_gpuinfo.hostClass, g_gpuinfo.ceClass, g_gpuinfo.computeClass, g_gpuinfo.smcEnabled);
    if (st != NV_OK) goto out;

    st = nvUvmInterfaceDeviceCreate(g_session, &g_gpuinfo, &g_uuid, &g_device, NV_FALSE);
    pr_info(KLOG "STAGE A: DeviceCreate -> 0x%x device=%px\n", st, (void*)g_device);
    if (st != NV_OK) goto out;
    g_have_device = NV_TRUE;

    memset(&vaInfo, 0, sizeof(vaInfo));
    st = nvUvmInterfaceAddressSpaceCreate(g_device, 0, 0, NV_FALSE, &g_vaspace, &vaInfo);
    pr_info(KLOG "STAGE A: AddressSpaceCreate -> 0x%x vaSpace=%px bigPageSize=%llu ats=%d\n",
            st, (void*)g_vaspace, (unsigned long long)vaInfo.bigPageSize, vaInfo.atsEnabled);
    if (st != NV_OK) goto out;
    g_have_vaspace = NV_TRUE;
    pr_info(KLOG "STAGE A: PASS (sole-owner takeover complete)\n");

    memset(&g_fault, 0, sizeof(g_fault));
    st = nvUvmInterfaceInitFaultInfo(g_device, &g_fault);
    pr_info(KLOG "STAGE B: InitFaultInfo -> 0x%x\n", st);
    if (st != NV_OK) { pr_err(KLOG "STAGE B: InitFaultInfo FAILED 0x%x\n", st); goto out; }
    g_have_fault = NV_TRUE;
    pr_info(KLOG "STAGE B: replayable.bufferAddress=%px (non-NULL=%d) bufferSize=%u\n",
            g_fault.replayable.bufferAddress, g_fault.replayable.bufferAddress != NULL, g_fault.replayable.bufferSize);
    pr_info(KLOG "STAGE B: pFaultBufferGet=%px (non-NULL=%d) pFaultBufferPut=%px (non-NULL=%d)\n",
            (void*)g_fault.replayable.pFaultBufferGet, g_fault.replayable.pFaultBufferGet != NULL,
            (void*)g_fault.replayable.pFaultBufferPut, g_fault.replayable.pFaultBufferPut != NULL);
    pr_info(KLOG "STAGE B: replayableFaultMask=0x%x pPmcIntrEnSet=%px pPmcIntrEnClear=%px pPrefetchCtrl=%px\n",
            g_fault.replayable.replayableFaultMask, (void*)g_fault.replayable.pPmcIntrEnSet,
            (void*)g_fault.replayable.pPmcIntrEnClear, (void*)g_fault.replayable.pPrefetchCtrl);
    if (g_fault.replayable.pFaultBufferGet && g_fault.replayable.pFaultBufferPut)
        pr_info(KLOG "STAGE B: *GET=%u *PUT=%u\n", *g_fault.replayable.pFaultBufferGet, *g_fault.replayable.pFaultBufferPut);

    st = nvUvmInterfaceOwnPageFaultIntr(g_device, NV_TRUE);
    pr_info(KLOG "STAGE B: OwnPageFaultIntr(TRUE) -> 0x%x %s\n", st,
            st == NV_OK ? "(OK: module owns replayable fault interrupt)" : "(err)");
    if (st != NV_OK) { pr_err(KLOG "STAGE B: OwnPageFaultIntr FAILED 0x%x\n", st); goto out; }
    g_own_intr = NV_TRUE;
    pr_info(KLOG "STAGE B: PASS (fault-plane ownership: HW fault buffer obtained, nvidia-uvm absent)\n");

    if (do_stage_c) stage_c();
    else pr_info(KLOG "STAGE C: skipped (echo 1 > /sys/module/kf_uvm_probe/parameters/do_stage_c then re-trigger, or load with do_stage_c=1)\n");

    pr_info(KLOG "==== stages done; module stays loaded (rmmod to clean up) ====\n");
    return;
out:
    pr_err(KLOG "stages aborted at status 0x%x; unwinding\n", st);
    unwind();
}

static ssize_t kf_trigger_write(struct file *f, const char __user *buf, size_t len, loff_t *off)
{
    run_stages();
    return len;
}
static const struct proc_ops kf_trigger_ops = {
    .proc_write = kf_trigger_write,
};

static int __init kf_init(void)
{
    NV_STATUS st;
    pr_info(KLOG "==== E6/N4 skeleton init (do_stage_c=%d) ====\n", do_stage_c);
    if (parse_uuid(gpu_uuid, &g_uuid)) {
        pr_err(KLOG "STAGE A: bad/empty gpu_uuid '%s'\n", gpu_uuid);
        return -EINVAL;
    }
    pr_info(KLOG "STAGE A: parsed UUID %02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x\n",
            g_uuid.uuid[0],g_uuid.uuid[1],g_uuid.uuid[2],g_uuid.uuid[3],g_uuid.uuid[4],g_uuid.uuid[5],
            g_uuid.uuid[6],g_uuid.uuid[7],g_uuid.uuid[8],g_uuid.uuid[9],g_uuid.uuid[10],g_uuid.uuid[11],
            g_uuid.uuid[12],g_uuid.uuid[13],g_uuid.uuid[14],g_uuid.uuid[15]);

    memset(&g_events, 0, sizeof(g_events));
    g_events.suspend = kf_suspend; g_events.resume = kf_resume;
    g_events.startDevice = kf_start_dev; g_events.stopDevice = kf_stop_dev;
    g_events.isrTopHalf = kf_isr_top_half;
    g_events.drainP2P = kf_drainP2P; g_events.resumeP2P = kf_resumeP2P;

    st = nvUvmInterfaceRegisterUvmCallbacks(&g_events);
    pr_info(KLOG "STAGE A: RegisterUvmCallbacks -> 0x%x %s\n", st,
            st == NV_OK ? "(OK: sole UVM registrant -- no other UVM present)" :
            st == NV_ERR_IN_USE ? "(IN_USE: another UVM already registered!)" : "(err)");
    if (st != NV_OK) return -EBUSY;
    g_cb_registered = NV_TRUE;

    if (!proc_create("kf_uvm_trigger", 0200, NULL, &kf_trigger_ops)) {
        pr_err(KLOG "failed to create /proc/kf_uvm_trigger\n");
        nvUvmInterfaceDeRegisterUvmOps(); g_cb_registered = NV_FALSE;
        return -ENOMEM;
    }
    pr_info(KLOG "init OK. Trigger GPU stages: hold /dev/nvidia0 open, then write /proc/kf_uvm_trigger\n");
    return 0;
}

static void __exit kf_exit(void)
{
    pr_info(KLOG "==== exit: unwinding ====\n");
    remove_proc_entry("kf_uvm_trigger", NULL);
    unwind();
    if (g_cb_registered) { nvUvmInterfaceDeRegisterUvmOps(); g_cb_registered = NV_FALSE; }
    pr_info(KLOG "==== exit done ====\n");
}

module_init(kf_init);
module_exit(kf_exit);
MODULE_LICENSE("Dual MIT/GPL");
MODULE_AUTHOR("kayfabe E6/N4");
MODULE_DESCRIPTION("Minimal 3rd-party UVM replacement probe on nvUvmInterface");
