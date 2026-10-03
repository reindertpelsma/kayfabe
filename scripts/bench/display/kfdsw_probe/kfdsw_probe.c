// SPDX-License-Identifier: GPL-2.0
/*
 * kfdsw_probe — a HOST-side instrument for the x11-dispsw experiment (docs/design/V3_DISPLAY.md).
 * Bench-only: built and loaded on a rented bench host by dispsw_trace.sh, never shipped.
 *
 * Why a module: host RM's display-SW release is silent by design — the GSP event dispatcher drops
 * `_kgspRpcSemaphoreScheduleCallback`'s status (ogkm-580: kernel_gsp.c:1526-1528) and the writers
 * skip the write when the DMA mapping has no kernel CPU mapping, returning NV_OK
 * (method_notification.c:349-351, :624-627). And trace_kprobe refuses every RM-core function of
 * the NVIDIA module ("Could not probe notrace function", measured 2026-10-03 on 6.8.0-59: the
 * module is built without fentry), so kprobe EVENTS cannot see it; a module's own kprobes can.
 *
 * Probed (x86-64 SysV argument registers):
 *   dispswReleaseSemaphoreAndNotifierFill(pGpu, gpuVA, hVASpace, value, flags, status, pDevice)
 *       (disp_sw.c:129-179): flags bit0|1 ADDR_VALID, bit2 SEMAPHORE_RELEASE, bit3 NOTIFIER_FILL
 *   semaphoreFillGPUVATimestamp(pGpu, pDevice, hMemCtx, va, value, index, bcast, time)
 *   notifyFillNotifierGPUVATimestamp(pGpu, pDevice, hMemCtx, va, info32, info16, status, index, time)
 *   intermapGetDmaMapping(pVirtualMemory, dmaOffset, gpuMask) -> CLI_DMA_MAPPING_INFO *
 *       only while a writer above runs on the same task: the mapping host RM resolved the VA to
 *       (mapping_list.h:52-71: DmaOffset +0, KernelVAddr[0] +8, pMemDesc +208, Flags +216 = the
 *       NVOS46 flags the map was made with, aperture +240: 1 VIDEO, 3 SYS_NONCOH, 4 SYS_COH)
 * POSITIVE CONTROL — so that zero release calls means "none asked", not "the probe is blind" or
 * "the event died before the release": _kgspRpcDrainEvents (the GSP event drain; the dispatcher
 * _kgspProcessRpcEvent and _kgspRpcSemaphoreScheduleCallback are inlined into it) arms the task,
 * and inside it every serverGetClientUnderLock (hClient, status) and deviceGetByHandle_IMPL
 * (hDevice, status) is counted. SEMAPHORE_SCHEDULE_CALLBACK resolves its client and device BEFORE
 * it calls the release (kernel_gsp.c:1118-1126), so an event that failed there shows up as a
 * failed lookup in the drain, and one that resolved shows up as a device lookup in the drain.
 * And CliGetEventInfo (hClient, hEvent) inside the drain is POST_EVENT's lookup
 * (kernel_gsp.c:497-498) — the event every channel's non-stall notification arrives as — so a
 * non-zero count there proves the in-drain counting itself sees the drain's lookups.
 * Output: the first 400 events and every 1000th after as `kfdsw:` lines in the kernel log, and
 * running totals in /sys/module/kfdsw_probe/parameters/.
 */
#include <linux/module.h>
#include <linux/kprobes.h>
#include <linux/atomic.h>
#include <linux/sched.h>

MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("kayfabe bench instrument: host RM display-SW release path");

static unsigned long dsw_calls, dsw_addr_valid, dsw_sem, dsw_ntf, dsw_err;
static unsigned long sem_calls, sem_err, ntf_calls, ntf_err;
static unsigned long map_found, map_null, map_kva_null, map_kva_set;
static unsigned long drains, drain_client_lookups, drain_client_fail, drain_dev_lookups, drain_dev_fail;
module_param(drains, ulong, 0444);
module_param(drain_client_lookups, ulong, 0444);
module_param(drain_client_fail, ulong, 0444);
module_param(drain_dev_lookups, ulong, 0444);
module_param(drain_dev_fail, ulong, 0444);
static unsigned long drain_event_lookups, drain_event_fail;
module_param(drain_event_lookups, ulong, 0444);
module_param(drain_event_fail, ulong, 0444);
module_param(dsw_calls, ulong, 0444);
module_param(dsw_addr_valid, ulong, 0444);
module_param(dsw_sem, ulong, 0444);
module_param(dsw_ntf, ulong, 0444);
module_param(dsw_err, ulong, 0444);
module_param(sem_calls, ulong, 0444);
module_param(sem_err, ulong, 0444);
module_param(ntf_calls, ulong, 0444);
module_param(ntf_err, ulong, 0444);
module_param(map_found, ulong, 0444);
module_param(map_null, ulong, 0444);
module_param(map_kva_null, ulong, 0444);
module_param(map_kva_set, ulong, 0444);

static atomic_t printed = ATOMIC_INIT(0);
static atomic_long_t seq = ATOMIC_LONG_INIT(0);
/* the task inside a writer (sem/ntf), so only ITS mapping lookups are reported */
static atomic_long_t armed_pid = ATOMIC_LONG_INIT(-1);
/* the task inside the GSP event drain (positive control) */
static atomic_long_t drain_pid = ATOMIC_LONG_INIT(-1);
static atomic_t drain_printed = ATOMIC_INIT(0);

static bool say(void)
{
	long n = atomic_long_inc_return(&seq);

	return atomic_inc_return(&printed) <= 400 || (n % 1000) == 0;
}

/* --- dispswReleaseSemaphoreAndNotifierFill --- */
static int dsw_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	u32 flags = (u32)regs->r8;

	dsw_calls++;
	if (flags & 3)
		dsw_addr_valid++;
	else if (flags & 4)
		dsw_sem++;
	else if (flags & 8)
		dsw_ntf++;
	if (say())
		pr_info("kfdsw: dsw va=%#llx vas=%#x val=%#x flags=%#x st=%#x pid=%d\n",
			(u64)regs->si, (u32)regs->dx, (u32)regs->cx, flags, (u32)regs->r9,
			current->pid);
	return 0;
}

static int dsw_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	u32 r = (u32)regs_return_value(regs);

	if (r != 0)
		dsw_err++;
	if (r != 0 && say())
		pr_info("kfdsw: dsw_ret %#x\n", r);
	return 0;
}

/* --- semaphoreFillGPUVATimestamp --- */
static int sem_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	sem_calls++;
	atomic_long_set(&armed_pid, current->pid);
	if (say())
		pr_info("kfdsw: sem ctx=%#x va=%#llx val=%#x idx=%u\n", (u32)regs->dx,
			(u64)regs->cx, (u32)regs->r8, (u32)regs->r9);
	return 0;
}

static int sem_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	u32 r = (u32)regs_return_value(regs);

	atomic_long_set(&armed_pid, -1);
	if (r != 0) {
		sem_err++;
		if (say())
			pr_info("kfdsw: sem_ret %#x\n", r);
	}
	return 0;
}

/* --- notifyFillNotifierGPUVATimestamp --- */
static int ntf_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	ntf_calls++;
	atomic_long_set(&armed_pid, current->pid);
	if (say())
		pr_info("kfdsw: ntf ctx=%#x va=%#llx info32=%#x\n", (u32)regs->dx,
			(u64)regs->cx, (u32)regs->r8);
	return 0;
}

static int ntf_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	u32 r = (u32)regs_return_value(regs);

	atomic_long_set(&armed_pid, -1);
	if (r != 0) {
		ntf_err++;
		if (say())
			pr_info("kfdsw: ntf_ret %#x\n", r);
	}
	return 0;
}

/* --- intermapGetDmaMapping (only inside a writer) --- */
static int map_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	const u8 *p = (const u8 *)regs_return_value(regs);
	u64 dma = 0, kva = 0, md = 0;
	u32 flags = 0, ap = 0;

	if (atomic_long_read(&armed_pid) != current->pid)
		return 0;
	if (!p) {
		map_null++;
		if (say())
			pr_info("kfdsw: map NONE (no DMA mapping at that VA)\n");
		return 0;
	}
	map_found++;
	if (copy_from_kernel_nofault(&dma, p + 0, 8) || copy_from_kernel_nofault(&kva, p + 8, 8) ||
	    copy_from_kernel_nofault(&md, p + 208, 8) ||
	    copy_from_kernel_nofault(&flags, p + 216, 4) || copy_from_kernel_nofault(&ap, p + 240, 4))
		return 0;
	if (kva)
		map_kva_set++;
	else
		map_kva_null++;
	if (say())
		pr_info("kfdsw: map dma=%#llx kva=%s flags=%#x kernel_mapping=%u aperture=%u\n", dma,
			kva ? "set" : "NULL", flags, (flags >> 5) & 1, ap);
	return 0;
}

/* --- positive control: the GSP event drain and the lookups made inside it --- */
static int drain_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	drains++;
	atomic_long_set(&drain_pid, current->pid);
	return 0;
}

static int drain_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	atomic_long_set(&drain_pid, -1);
	return 0;
}

static bool in_drain(void)
{
	return atomic_long_read(&drain_pid) == current->pid;
}

struct lookup { u32 handle; bool drain; };

static int client_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct lookup *l = (struct lookup *)ri->data;

	l->handle = (u32)regs->si;
	l->drain = in_drain();
	return 0;
}

static int client_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct lookup *l = (struct lookup *)ri->data;
	u32 r = (u32)regs_return_value(regs);

	if (!l->drain)
		return 0;
	drain_client_lookups++;
	if (r)
		drain_client_fail++;
	if (atomic_inc_return(&drain_printed) <= 64 || r)
		pr_info("kfdsw: drain client %#x -> %#x\n", l->handle, r);
	return 0;
}

static int dev_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct lookup *l = (struct lookup *)ri->data;

	l->handle = (u32)regs->si;
	l->drain = in_drain();
	return 0;
}

static int dev_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct lookup *l = (struct lookup *)ri->data;
	u32 r = (u32)regs_return_value(regs);

	if (!l->drain)
		return 0;
	drain_dev_lookups++;
	if (r)
		drain_dev_fail++;
	pr_info("kfdsw: drain device %#x -> %#x\n", l->handle, r);
	return 0;
}

struct evlookup { u32 client, event; bool drain; };

static int event_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct evlookup *l = (struct evlookup *)ri->data;

	l->client = (u32)regs->di;
	l->event = (u32)regs->si;
	l->drain = in_drain();
	return 0;
}

static int event_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct evlookup *l = (struct evlookup *)ri->data;
	u32 found = (u32)(regs_return_value(regs) & 0xff); /* NvBool */

	if (!l->drain)
		return 0;
	drain_event_lookups++;
	if (!found)
		drain_event_fail++;
	if (drain_event_lookups <= 16 || !found)
		pr_info("kfdsw: drain event client %#x event %#x -> found=%u\n", l->client, l->event,
			found);
	return 0;
}

static struct kretprobe probes[] = {
	{ .kp.symbol_name = "dispswReleaseSemaphoreAndNotifierFill", .entry_handler = dsw_entry,
	  .handler = dsw_ret, .maxactive = 64 },
	{ .kp.symbol_name = "semaphoreFillGPUVATimestamp", .entry_handler = sem_entry,
	  .handler = sem_ret, .maxactive = 64 },
	{ .kp.symbol_name = "notifyFillNotifierGPUVATimestamp", .entry_handler = ntf_entry,
	  .handler = ntf_ret, .maxactive = 64 },
	{ .kp.symbol_name = "intermapGetDmaMapping", .handler = map_ret, .maxactive = 256 },
	{ .kp.symbol_name = "_kgspRpcDrainEvents", .entry_handler = drain_entry,
	  .handler = drain_ret, .maxactive = 16 },
	{ .kp.symbol_name = "serverGetClientUnderLock", .entry_handler = client_entry,
	  .handler = client_ret, .data_size = sizeof(struct lookup), .maxactive = 256 },
	{ .kp.symbol_name = "deviceGetByHandle_IMPL", .entry_handler = dev_entry,
	  .handler = dev_ret, .data_size = sizeof(struct lookup), .maxactive = 256 },
	{ .kp.symbol_name = "CliGetEventInfo", .entry_handler = event_entry,
	  .handler = event_ret, .data_size = sizeof(struct evlookup), .maxactive = 64 },
};

static int __init kfdsw_init(void)
{
	int i, ok = 0;

	for (i = 0; i < ARRAY_SIZE(probes); i++) {
		int r = register_kretprobe(&probes[i]);

		if (r)
			pr_info("kfdsw: PROBE_REFUSED %s (%d)\n", probes[i].kp.symbol_name, r);
		else
			ok++;
	}
	pr_info("kfdsw: loaded, %d/%zu probes\n", ok, ARRAY_SIZE(probes));
	return ok ? 0 : -ENOENT;
}

static void __exit kfdsw_exit(void)
{
	int i;

	for (i = 0; i < ARRAY_SIZE(probes); i++)
		if (probes[i].kp.addr)
			unregister_kretprobe(&probes[i]);
	pr_info("kfdsw: unloaded dsw=%lu addr_valid=%lu sem=%lu ntf=%lu dsw_err=%lu sem_calls=%lu sem_err=%lu ntf_calls=%lu ntf_err=%lu map_found=%lu map_null=%lu kva_null=%lu kva_set=%lu drains=%lu drain_client=%lu/%lu_failed drain_device=%lu/%lu_failed drain_event=%lu/%lu_failed\n",
		dsw_calls, dsw_addr_valid, dsw_sem, dsw_ntf, dsw_err, sem_calls, sem_err, ntf_calls,
		ntf_err, map_found, map_null, map_kva_null, map_kva_set, drains,
		drain_client_lookups, drain_client_fail, drain_dev_lookups, drain_dev_fail,
		drain_event_lookups, drain_event_fail);
}

module_init(kfdsw_init);
module_exit(kfdsw_exit);
