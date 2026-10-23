// SPDX-License-Identifier: GPL-2.0-only
/* Guest-only harness using the public ogkm UVM interface. Never load on the host. */
#include <linux/module.h>
#include <linux/delay.h>
#include <linux/ktime.h>
#include <linux/pci.h>
#include <linux/string.h>
#include <linux/sched.h>
#if __has_include(<linux/hex.h>)
#include <linux/hex.h>
#else
#include <linux/kernel.h>
#endif
#include "kf_rc_api.h"
#include "class/clc56f.h"
#include "class/cl2080_notification.h"

static char *uuid;
static char *run;
static bool fault;
module_param(uuid, charp, 0400);
module_param(run, charp, 0400);
module_param(fault, bool, 0400);
MODULE_PARM_DESC(uuid, "Guest GPU UUID as 32 hex digits (no separators)");
MODULE_PARM_DESC(run, "Host-authored capture identifier");
MODULE_PARM_DESC(fault, "Request the explicitly armed host bench hook");

#define TRIGGER_PUT 0xb1c00001U
#define DEADLINE_NS (5ULL * 1000 * 1000 * 1000)
#define ENTRIES 64
struct probe_channel {
    uvmGpuTsgHandle tsg;
    uvmGpuChannelHandle channel;
    UvmGpuChannelInfo info;
};
static NvProcessorUuid gpu_uuid;
static bool registered;
static uvmGpuSessionHandle session;
static uvmGpuDeviceHandle device;
static uvmGpuAddressSpaceHandle space;
static struct probe_channel victim, bystander;
static UvmGpuPointer pb_va;
static void *pb_cpu;
static NvU32 engine;

static void destroy_channel(struct probe_channel *p)
{
    if (p->channel) nvUvmInterfaceChannelDestroy(p->channel);
    p->channel = NULL;
    if (p->tsg) nvUvmInterfaceTsgDestroy(p->tsg);
    p->tsg = NULL;
}

static void cleanup(void)
{
    destroy_channel(&victim);
    destroy_channel(&bystander);
    if (pb_cpu) nvUvmInterfaceMemoryCpuUnMap(space, pb_cpu);
    if (pb_va) nvUvmInterfaceMemoryFree(space, pb_va);
    if (space) nvUvmInterfaceAddressSpaceDestroy(space);
    if (device) nvUvmInterfaceDeviceDestroy(device);
    if (session) nvUvmInterfaceSessionDestroy(session);
    if (registered) nvUvmInterfaceUnregisterGpu(&gpu_uuid);
}

static int create_channel(struct probe_channel *p, NvU32 ce)
{
    UvmGpuTsgAllocParams t = { .engineType = UVM_GPU_CHANNEL_ENGINE_TYPE_CE, .engineIndex = ce };
    UvmGpuChannelAllocParams c = { .numGpFifoEntries = ENTRIES,
        .gpFifoLoc = UVM_BUFFER_LOCATION_SYS, .gpPutLoc = UVM_BUFFER_LOCATION_SYS };
    NV_STATUS s = nvUvmInterfaceTsgAllocate(space, &t, &p->tsg);
    if (s == NV_OK) s = nvUvmInterfaceChannelAllocate(p->tsg, &c, &p->channel, &p->info);
    if (s != NV_OK) {
        pr_err("KF_RC_SETUP status=0x%x\n", s);
        return -EIO;
    }
    if (p->info.channelClassNum != 0xc56f || !p->info.gpPut || !p->info.gpGet ||
        !p->info.gpFifoEntries || !p->info.errorNotifier ||
        !p->info.workSubmissionOffset || !p->info.pWorkSubmissionToken ||
        READ_ONCE(*p->info.gpGet) != 0 || READ_ONCE(p->info.errorNotifier->status) != 0)
        return -EINVAL;
    return 0;
}

static void doorbell(struct probe_channel *p, NvU32 put)
{
    WRITE_ONCE(*p->info.gpPut, put);
    wmb();
    WRITE_ONCE(*p->info.workSubmissionOffset, READ_ONCE(*p->info.pWorkSubmissionToken));
}

/* GP_GET is accepted here only after a real Translated host fence has completed. */
static int progress(NvU32 seq)
{
    NvU32 *words = pb_cpu;
    u64 until = ktime_get_ns() + DEADLINE_NS;
    words[0] = (1U << 29) | (1U << 16) | (NVC56F_SET_REFERENCE >> 2);
    words[1] = seq;
    wmb();
    WRITE_ONCE(bystander.info.gpFifoEntries[seq-1], (pb_va & 0xfffffffffcULL) | (2ULL << 42));
    wmb();
    doorbell(&bystander, seq);
    while (ktime_get_ns() < until) {
        if (READ_ONCE(bystander.info.errorNotifier->status)) return -EIO;
        if (READ_ONCE(*bystander.info.gpGet) == seq) return 0;
        msleep(1);
    }
    return -ETIMEDOUT;
}

static int __init probe_init(void)
{
    UvmGpuPlatformInfo platform = {0};
    UvmPlatformInfo system = {0};
    UvmGpuInfo gpu = {0};
    UvmGpuClientInfo client = {0};
    UvmGpuAddressSpaceInfo vas = {0};
    UvmGpuAllocInfo allocation = {0};
    UvmGpuCopyEnginesCaps caps = {0};
    NV_STATUS s;
    NvU32 ce;
    NvU32 get, code, info16, status;
    NvU64 stamp;
    u64 until;
    int error = -EINVAL;
    size_t i;
    if (!uuid || strlen(uuid) != 32 || !run || !strlen(run) || strlen(run)>32)
        return -EINVAL;
    for (i=0; run[i]; i++)
        if (!((run[i]>='a' && run[i]<='z') || (run[i]>='A' && run[i]<='Z') ||
              (run[i]>='0' && run[i]<='9') || run[i]=='_' || run[i]=='-')) return -EINVAL;
    if (hex2bin((u8 *)&gpu_uuid, uuid, 16)) return -EINVAL;
    s = nvUvmInterfaceRegisterGpu(&gpu_uuid, &platform);
    if (s != NV_OK) goto bad;
    registered = true;
    s = nvUvmInterfaceSessionCreate(&session, &system);
    if (s != NV_OK) goto bad;
    if (system.confComputingEnabled) goto bad;
    s = nvUvmInterfaceGetGpuInfo(&gpu_uuid, &client, &gpu);
    if (s != NV_OK || gpu.hostClass != 0xc56f) goto bad;
    s = nvUvmInterfaceDeviceCreate(session, &gpu, &gpu_uuid, &device, NV_FALSE);
    if (s != NV_OK) goto bad;
    s = nvUvmInterfaceAddressSpaceCreate(device, 0x100000ULL, (1ULL<<39)-0x100000ULL,
                                        NV_FALSE, &space, &vas);
    if (s != NV_OK) goto bad;
    s = nvUvmInterfaceQueryCopyEnginesCaps(device, &caps);
    if (s != NV_OK) goto bad;
    /* This C56F control qualifies the first ten public COPY engine IDs only. */
    for (ce=0; ce<10; ce++)
        if (caps.copyEngineCaps[ce].supported && !caps.copyEngineCaps[ce].grce) break;
    if (ce==10) goto bad;
    engine = NV2080_ENGINE_TYPE_COPY(ce);
    error = create_channel(&bystander, ce);
    if (error) goto bad;
    error = create_channel(&victim, ce);
    if (error) goto bad;
    s = nvUvmInterfaceMemoryAllocSys(space, 4096, &pb_va, &allocation);
    if (s != NV_OK || !pb_va || pb_va >= (1ULL<<40)) goto bad;
    s = nvUvmInterfaceMemoryCpuMap(space, pb_va, 4096, &pb_cpu, 4096);
    if (s != NV_OK) goto bad;
    pr_info("KF_RC_PROBE_BEGIN run=%s mode=%s chid=0x%x engine=0x%x class=0xc56f async_ce=1 owner=%d\n",
            run, fault ? "on" : "off", victim.info.hwChannelId, engine, task_tgid_nr(current));
    error = progress(1);
    if (error) goto bad;
    pr_info("KF_RC_BYSTANDER run=%s phase=before value=1\n", run);
    /* This is a bench trigger, never a guest pushbuffer forwarded to the host. */
    doorbell(&victim, fault ? TRIGGER_PUT : 0);
    until = ktime_get_ns() + DEADLINE_NS;
    if (fault) {
        while (!READ_ONCE(victim.info.errorNotifier->status) && ktime_get_ns() < until)
            msleep(1);
        status = READ_ONCE(victim.info.errorNotifier->status);
        smp_rmb();
        code = READ_ONCE(victim.info.errorNotifier->info32);
        info16 = READ_ONCE(victim.info.errorNotifier->info16);
        /* Public NvNotification begins with the 64-bit timestamp. */
        stamp = READ_ONCE(victim.info.errorNotifier->timeStamp.nanoseconds[0]);
        stamp |= (NvU64)READ_ONCE(victim.info.errorNotifier->timeStamp.nanoseconds[1]) << 32;
        get = READ_ONCE(*victim.info.gpGet);
        pr_info("KF_RC_NOTIFIER run=%s chid=0x%x timestamp=0x%llx info32=%u info16=%u status=0x%x get=%u\n",
                run, victim.info.hwChannelId, (unsigned long long)stamp, code, info16, status, get);
        error = -EIO;
        if (status!=0xffff || code!=32 || info16!=engine || !stamp || get!=0) goto bad;
        /* Wait on receipt of the real guest event, not elapsed time before teardown. */
        until = ktime_get_ns() + DEADLINE_NS;
        while (!nvKfRcControlEventCount(victim.info.hwChannelId, engine) && ktime_get_ns()<until)
            msleep(1);
        if (nvKfRcControlEventCount(victim.info.hwChannelId, engine)!=1) goto bad;
    } else {
        /* Bench observation window, not a product timeout or progress mechanism. */
        msleep(50);
        if (READ_ONCE(victim.info.errorNotifier->status)) goto bad;
        pr_info("KF_RC_CLEAN run=%s status=0x0\n", run);
    }
    error = progress(2);
    if (error) goto bad;
    pr_info("KF_RC_BYSTANDER run=%s phase=after value=2\n", run);
    /* The other TSG must still run after the victim's channel and TSG are gone. */
    destroy_channel(&victim);
    error = progress(3);
    if (error) goto bad;
    pr_info("KF_RC_BYSTANDER run=%s phase=after_teardown value=3\n", run);
    pr_info("KF_RC_PROBE_END run=%s result=PASS\n", run);
    if (fault) {
        /* Report the channel error to the owning userspace loader syscall. */
        cleanup();
        return -EIO;
    }
    return 0;
bad:
    pr_err("KF_RC_PROBE_END run=%s result=FAIL status=0x%x error=%d\n", run, s, error);
    cleanup();
    return error ? error : -EIO;
}

module_init(probe_init);
module_exit(cleanup);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Guest-only Translated robust-channel error negative control");
