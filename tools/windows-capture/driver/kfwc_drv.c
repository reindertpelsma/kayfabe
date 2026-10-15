/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * kfwc.sys: Kayfabe Windows capture module (MEASUREMENT only).
 *
 * A legacy (non-PnP) WDM driver, loaded on demand on a machine we own, that
 *   1. finds the NVIDIA display device through PCI configuration space reads,
 *   2. maps the first 1.1 MiB of its BAR0 read-only (MmMapIoSpaceEx, PAGE_READONLY),
 *   3. runs the platform independent engine (lib/): boot-args mailbox discovery, RAM scan
 *      fallback, queue walker, loss accounting, on a PASSIVE_LEVEL system thread with a bounded
 *      per-iteration budget,
 *   4. publishes the resulting KFGSPCAP records through a lock-free ring that user mode reads
 *      with ReadFile.
 *
 * READ-ONLY GUARANTEE. This file contains no store to device memory, to a device register, to
 * PCI configuration space, or to guest RAM outside its own allocations:
 *   - the only mapped device pointer has type KFWC_MMIO (pointer to volatile const), so a store
 *     through it does not compile;
 *   - the mapping is created PAGE_READONLY, so a store that slipped through would fault;
 *   - PCI config is read with HalGetBusDataByOffset only (HalSetBusData* never appears);
 *   - guest RAM is read with MmCopyMemory(MM_COPY_MEMORY_PHYSICAL) only, after the range has
 *     been checked against MmGetPhysicalMemoryRanges;
 *   - tests/guard_readonly.sh fails the build when any forbidden API or pattern appears.
 * It hooks nothing, patches nothing, calls nothing in the NVIDIA driver and reads no board
 * specific datum (no UUID, serial, VBIOS, MAC; the PCI subsystem ids are never read).
 *
 * UNVERIFIED ON HARDWARE: this file compiles (see build/) but has not been loaded. Every
 * Windows API contract used below is the documented one; the behaviours that are guesses are
 * listed in docs/design/V3_WINDOWS_CAPTURE_MODULE.md.
 */
#include <ntddk.h>
#include <wdmsec.h>
#include "kfwc_ioctl.h"
#include "../lib/kfwc_ring.h"

#define TAG 'cwFK'
#define RING_BYTES (16u * 1024u * 1024u)
#define RING_RESERVE (256u * 1024u)
#define MAX_GPUS 8u
#define MAX_RANGES 4096u
#define BAR_STABLE_100NS (200000LL) /* 20 ms of identical BAR reads before mapping */

/* A mapped device range. Pointer to volatile CONST: the compiler rejects any store through it. */
typedef const volatile UCHAR *KFWC_MMIO;

C_ASSERT(KFWC_IOCTL_INFO == CTL_CODE(FILE_DEVICE_UNKNOWN, 0x800, METHOD_BUFFERED, FILE_READ_ACCESS));
C_ASSERT(KFWC_IOCTL_STOP == CTL_CODE(FILE_DEVICE_UNKNOWN, 0x804, METHOD_BUFFERED, FILE_READ_ACCESS));

typedef struct GPU {
    ULONG bus, dev, fn;
    USHORT vendor, device;
    UCHAR revision;
} GPU;

typedef struct STATE {
    FAST_MUTEX lock;            /* worker <-> ioctl access to the engine; never taken by the ring */
    KEVENT stop_evt;
    PVOID thread;               /* referenced thread object, waited on at stop */
    BOOLEAN running;
    BOOLEAN timer_res_set;
    volatile LONG reader_busy;
    kfwc_start_req req;
    kfwc_engine *engine;
    kfwc_ring ring;
    PUCHAR ring_mem;
    kfwc_platform plat;
    kfwc_sink sink;
    PHYSICAL_MEMORY_RANGE *ranges;
    ULONG range_count;
    GPU gpus[MAX_GPUS];
    ULONG gpu_count, gpu_sel;
    /* Only this module can seed rediscovery; no ioctl accepts a physical address. */
    kfwc_u64 recent_table_pa;
    GPU recent_gpu;
    BOOLEAN recent_valid;
    ULONG bar_state;
    KFWC_MMIO bar0;             /* the only way registers are touched */
    PVOID bar0_map;             /* the same mapping, kept untyped ONLY to hand back to MmUnmapIoSpace */
    ULONGLONG bar0_pa;           /* what is mapped */
    ULONGLONG bar_seen;          /* last BAR value read, awaiting stability */
    LONGLONG bar_seen_since;
    LARGE_INTEGER qpc_freq, qpc_start;
    ULONGLONG loops, bar_remaps, pci_scans;
} STATE;

static STATE *g_state;
static UNICODE_STRING g_link = RTL_CONSTANT_STRING(L"\\DosDevices\\KayfabeWinCapture");
static const GUID g_class = {0x7f3a0c5e, 0x4d1b, 0x4a6e, {0x9c, 0x2f, 0x5b, 0x8e, 0x1d, 0x3a, 0x9f, 0x40}};

DRIVER_INITIALIZE DriverEntry;
DRIVER_UNLOAD KfwcUnload;
DRIVER_DISPATCH KfwcDispatch;

/* ------------------------------------------------------------------------------------- */
/* PCI configuration space: READ ONLY                                                       */
/* ------------------------------------------------------------------------------------- */

static BOOLEAN pci_read(const GPU *g, ULONG off, PVOID buf, ULONG len)
{
    PCI_SLOT_NUMBER slot;
    slot.u.AsULONG = 0;
    slot.u.bits.DeviceNumber = g->dev;
    slot.u.bits.FunctionNumber = g->fn;
    return HalGetBusDataByOffset(PCIConfiguration, g->bus, slot.u.AsULONG, buf, off, len) == len;
}

/* Walk PCI buses for NVIDIA (vendor 10DE) display controllers (base class 03). Only vendor,
 * device, revision, class and header type are read from devices we do not select. */
static void pci_find_gpus(STATE *s)
{
    ULONG bus, dev, fn;
    s->gpu_count = 0;
    s->pci_scans++;
    for (bus = 0; bus < 256 && s->gpu_count < MAX_GPUS; bus++) {
        for (dev = 0; dev < 32 && s->gpu_count < MAX_GPUS; dev++) {
            for (fn = 0; fn < 8 && s->gpu_count < MAX_GPUS; fn++) {
                GPU g;
                USHORT vendor = 0xFFFF, device = 0;
                UCHAR cls = 0, rev = 0, htype = 0;
                g.bus = bus;
                g.dev = dev;
                g.fn = fn;
                if (!pci_read(&g, 0x00, &vendor, 2) || vendor == 0xFFFF) {
                    if (fn == 0)
                        break; /* no function 0: no device here */
                    continue;
                }
                if (fn == 0 && (!pci_read(&g, 0x0E, &htype, 1)))
                    htype = 0;
                if (vendor == 0x10DE && pci_read(&g, 0x0B, &cls, 1) && cls == 0x03 &&
                    pci_read(&g, 0x02, &device, 2) && pci_read(&g, 0x08, &rev, 1)) {
                    g.vendor = vendor;
                    g.device = device;
                    g.revision = rev;
                    s->gpus[s->gpu_count++] = g;
                }
                if (fn == 0 && !(htype & 0x80))
                    break; /* single function device */
            }
        }
    }
}

/* Power state D0 and memory decode enabled. Config reads only. */
static BOOLEAN pci_usable(const GPU *g)
{
    USHORT cmd = 0, status = 0, pmcsr = 0;
    UCHAR cap = 0, id = 0, next = 0, hops;
    if (!pci_read(g, 0x04, &cmd, 2) || !(cmd & 0x2)) /* memory space enable */
        return FALSE;
    if (!pci_read(g, 0x06, &status, 2) || !(status & 0x10)) /* no capability list: assume D0 */
        return TRUE;
    if (!pci_read(g, 0x34, &cap, 1))
        return FALSE;
    for (hops = 0; hops < 48 && cap >= 0x40; hops++) {
        if (!pci_read(g, cap, &id, 1) || !pci_read(g, cap + 1u, &next, 1))
            return FALSE;
        if (id == 0x01) { /* power management */
            if (!pci_read(g, cap + 4u, &pmcsr, 2))
                return FALSE;
            return (pmcsr & 3) == 0; /* D0 */
        }
        cap = next;
    }
    return TRUE;
}

static BOOLEAN pci_bar0(const GPU *g, ULONGLONG *pa)
{
    ULONG lo = 0, hi = 0;
    if (!pci_read(g, 0x10, &lo, 4) || (lo & 1) || lo == 0xFFFFFFFFul)
        return FALSE;
    *pa = lo & ~0xFul;
    if (((lo >> 1) & 3) == 2) { /* 64-bit memory BAR */
        if (!pci_read(g, 0x14, &hi, 4))
            return FALSE;
        *pa |= (ULONGLONG)hi << 32;
    }
    return *pa != 0 && (*pa & 0xFFF) == 0 && *pa != 0xFFFFFFF0ull;
}

/* ------------------------------------------------------------------------------------- */
/* BAR0: read-only mapping, validated before every register read                            */
/* ------------------------------------------------------------------------------------- */

static void bar_unmap(STATE *s)
{
    if (s->bar0_map) {
        MmUnmapIoSpace(s->bar0_map, KFWC_BAR0_MAP_BYTES);
        s->bar0_map = NULL;
        s->bar0 = NULL;
        s->bar0_pa = 0;
    }
}

/* Returns TRUE when the mapping may be read right now. */
static BOOLEAN bar_ready(STATE *s)
{
    const GPU *g;
    ULONGLONG pa = 0;
    LARGE_INTEGER now;
    if (!s->gpu_count || s->gpu_sel >= s->gpu_count) {
        s->bar_state = KFWC_BAR_NONE;
        return FALSE;
    }
    g = &s->gpus[s->gpu_sel];
    if (!pci_usable(g)) {
        s->bar_state = KFWC_BAR_DISABLED;
        return FALSE;
    }
    if (!pci_bar0(g, &pa)) {
        s->bar_state = KFWC_BAR_UNSTABLE;
        return FALSE;
    }
    now = KeQueryPerformanceCounter(NULL);
    if (pa != s->bar_seen) { /* new value: do not trust it until it stays put */
        bar_unmap(s);
        s->bar_seen = pa;
        s->bar_seen_since = now.QuadPart;
        s->bar_state = KFWC_BAR_UNSTABLE;
        return FALSE;
    }
    if (s->bar0 && s->bar0_pa == pa) {
        s->bar_state = KFWC_BAR_MAPPED;
        return TRUE;
    }
    if ((now.QuadPart - s->bar_seen_since) * 10000000LL < BAR_STABLE_100NS * s->qpc_freq.QuadPart) {
        s->bar_state = KFWC_BAR_UNSTABLE;
        return FALSE;
    }
    {
        PHYSICAL_ADDRESS phys;
        phys.QuadPart = (LONGLONG)pa;
        /* PAGE_READONLY | PAGE_NOCACHE: the only way this module ever maps device memory. */
        s->bar0_map = MmMapIoSpaceEx(phys, KFWC_BAR0_MAP_BYTES, PAGE_READONLY | PAGE_NOCACHE);
        s->bar0 = (KFWC_MMIO)s->bar0_map;
    }
    if (!s->bar0) {
        s->bar_state = KFWC_BAR_MAP_FAILED;
        return FALSE;
    }
    s->bar0_pa = pa;
    s->bar_remaps++;
    s->bar_state = KFWC_BAR_MAPPED;
    return TRUE;
}

/* ------------------------------------------------------------------------------------- */
/* platform callbacks for the engine                                                        */
/* ------------------------------------------------------------------------------------- */

static int plat_ram_contains(void *ctx, kfwc_u64 pa, kfwc_u64 len)
{
    STATE *s = (STATE *)ctx;
    ULONG i;
    if (!len || pa > MAXULONGLONG - len)
        return 0;
    for (i = 0; i < s->range_count; i++) {
        ULONGLONG lo = (ULONGLONG)s->ranges[i].BaseAddress.QuadPart;
        ULONGLONG bytes = (ULONGLONG)s->ranges[i].NumberOfBytes.QuadPart;
        if (pa >= lo && pa - lo < bytes && len <= bytes - (pa - lo))
            return 1;
    }
    return 0;
}

static int plat_read_ram(void *ctx, kfwc_u64 pa, void *dst, kfwc_u32 len)
{
    MM_COPY_ADDRESS from;
    SIZE_T got = 0;
    if (!plat_ram_contains(ctx, pa, len))
        return 1;
    from.PhysicalAddress.QuadPart = (LONGLONG)pa;
    if (!NT_SUCCESS(MmCopyMemory(dst, from, len, MM_COPY_MEMORY_PHYSICAL, &got)) || got != len)
        return 1;
    return 0;
}

static kfwc_u32 plat_range_count(void *ctx) { return ((STATE *)ctx)->range_count; }

static int plat_range(void *ctx, kfwc_u32 i, kfwc_u64 *base, kfwc_u64 *len)
{
    STATE *s = (STATE *)ctx;
    if (i >= s->range_count)
        return 1;
    *base = (kfwc_u64)s->ranges[i].BaseAddress.QuadPart;
    *len = (kfwc_u64)s->ranges[i].NumberOfBytes.QuadPart;
    return 0;
}

/* The library only passes whitelisted offsets (kfwc_reg_allowed); the offset is re-checked
 * against the mapped prefix here. A plain volatile load: there is no store form to misuse. */
static int plat_read_reg32(void *ctx, kfwc_u32 off, kfwc_u32 *out)
{
    STATE *s = (STATE *)ctx;
    if (!kfwc_reg_allowed(off) || (off & 3) || off + 4u > KFWC_BAR0_MAP_BYTES || !bar_ready(s))
        return 1;
    *out = *(const volatile ULONG *)(s->bar0 + off);
    return 0;
}

static kfwc_u64 plat_now(void *ctx)
{
    UNREFERENCED_PARAMETER(ctx);
    return (kfwc_u64)KeQueryPerformanceCounter(NULL).QuadPart;
}

static int sink_put(void *ctx, const kfwc_u8 *rec, kfwc_u32 len)
{
    return kfwc_ring_put(&((STATE *)ctx)->ring, rec, len);
}

/* ------------------------------------------------------------------------------------- */
/* worker                                                                                   */
/* ------------------------------------------------------------------------------------- */

static BOOLEAN stopping(STATE *s) { return KeReadStateEvent(&s->stop_evt) != 0; }

static VOID Worker(PVOID context)
{
    STATE *s = (STATE *)context;
    ULONG burst = 0;
    ULONGLONG last_pci = 0;
    KeSetPriorityThread(KeGetCurrentThread(), LOW_REALTIME_PRIORITY);
    while (!stopping(s)) {
        kfwc_u32 n;
        kfwc_u64 recent;
        ULONG wait_us;
        LARGE_INTEGER delay;
        ExAcquireFastMutex(&s->lock);
        if (!s->gpu_count && (ULONGLONG)KeQueryPerformanceCounter(NULL).QuadPart - last_pci > (ULONGLONG)s->qpc_freq.QuadPart) {
            last_pci = (ULONGLONG)KeQueryPerformanceCounter(NULL).QuadPart; /* rescan once a second */
            pci_find_gpus(s);
        }
        n = kfwc_engine_step(s->engine);
        recent = kfwc_engine_recent_table(s->engine);
        if (recent && s->gpu_sel < s->gpu_count) {
            /* Both old rings have advanced beyond retaining sequence zero. A
             * later revisit can therefore distinguish that old epoch from a
             * newly zeroed/published queue at the same validated RAM location. */
            s->recent_table_pa = recent;
            s->recent_gpu = s->gpus[s->gpu_sel];
            s->recent_valid = TRUE;
        }
        if (s->engine->state == KFWC_ST_SEARCHING && s->req.use_scan)
            /* scan steps at the scan cadence; in a back-off pause (or before the scan delay
             * ends) only the mailbox is read, and the engine reads it every 10 ms at most */
            wait_us = kfwc_engine_scan_due(s->engine) ? s->req.scan_period_us
                                                      : (s->req.poll_us > 10000 ? s->req.poll_us : 10000);
        else
            wait_us = s->req.poll_us;
        s->loops++;
        ExReleaseFastMutex(&s->lock);
        if (n && ++burst < 16) /* more may be waiting: a bounded burst, then yield */
            continue;
        burst = 0;
        delay.QuadPart = -(LONGLONG)wait_us * 10;
        (void)KeWaitForSingleObject(&s->stop_evt, Executive, KernelMode, FALSE, &delay);
    }
    PsTerminateSystemThread(STATUS_SUCCESS);
}

static NTSTATUS capture_start(STATE *s, const kfwc_start_req *req)
{
    kfwc_config cfg;
    OBJECT_ATTRIBUTES attrs;
    NTSTATUS status;
    kfwc_start_req r;
    PHYSICAL_MEMORY_RANGE *ranges;
    ULONG n = 0;

    if (s->running)
        return STATUS_DEVICE_BUSY;
    if (req->abi != KFWC_DRIVER_ABI || req->layout > KFWC_LAYOUT_610)
        return STATUS_REVISION_MISMATCH;
    r = *req;
    if (r.poll_us < 200) r.poll_us = 200;
    if (r.poll_us > 100000) r.poll_us = 100000;
    if (r.scan_period_us < 500) r.scan_period_us = 500;
    if (r.scan_period_us > 1000000) r.scan_period_us = 1000000;
    if (r.scan_bytes_per_step < 4096) r.scan_bytes_per_step = 4096;
    if (r.scan_bytes_per_step > KFWC_SCAN_CHUNK) r.scan_bytes_per_step = KFWC_SCAN_CHUNK;
    if (r.scan_delay_ms > 600000) r.scan_delay_ms = 600000;
    if (r.scan_backoff_initial_ms == 0) r.scan_backoff_initial_ms = KFWC_BACKOFF_INITIAL_MS_DEFAULT;
    if (r.scan_backoff_initial_ms != KFWC_BACKOFF_OFF && r.scan_backoff_initial_ms > 3600000) r.scan_backoff_initial_ms = 3600000;
    if (r.scan_backoff_max_ms == 0) r.scan_backoff_max_ms = KFWC_BACKOFF_MAX_MS_DEFAULT;
    if (r.scan_backoff_max_ms > 3600000) r.scan_backoff_max_ms = 3600000;
    if (r.max_msgs_per_step < 1) r.max_msgs_per_step = 1;
    if (r.max_msgs_per_step > 256) r.max_msgs_per_step = 256;
    s->req = r;

    ranges = MmGetPhysicalMemoryRanges();
    if (!ranges)
        return STATUS_INSUFFICIENT_RESOURCES;
    while (n < MAX_RANGES && ranges[n].NumberOfBytes.QuadPart)
        n++;
    if (s->ranges)
        ExFreePool(s->ranges);
    s->ranges = ranges;
    s->range_count = n;

    ExAcquireFastMutex(&s->lock);
    bar_unmap(s);
    s->bar_seen = 0;
    pci_find_gpus(s);
    s->gpu_sel = r.gpu_index < s->gpu_count ? r.gpu_index : 0;
    s->qpc_start = KeQueryPerformanceCounter(&s->qpc_freq);
    s->loops = 0;
    kfwc_config_defaults(&cfg, (kfwc_u64)s->qpc_freq.QuadPart);
    cfg.layout = r.layout;
    cfg.use_mailbox = r.use_mailbox ? 1 : 0;
    cfg.use_scan = r.use_scan ? 1 : 0;
    if (s->recent_valid && s->gpu_sel < s->gpu_count) {
        const GPU *g = &s->gpus[s->gpu_sel];
        const GPU *old = &s->recent_gpu;
        if (g->bus == old->bus && g->dev == old->dev && g->fn == old->fn &&
            g->vendor == old->vendor && g->device == old->device && g->revision == old->revision)
            cfg.recent_table_pa = s->recent_table_pa;
    }
    cfg.scan_bytes_per_step = r.scan_bytes_per_step;
    cfg.max_msgs_per_step = r.max_msgs_per_step;
    cfg.scan_delay_ticks = (kfwc_u64)r.scan_delay_ms * (kfwc_u64)s->qpc_freq.QuadPart / 1000u;
    /* the back-off restarts on a change of the mailbox pair, so without the mailbox there is
     * nothing to restart it: scan-only mode keeps the passes back to back */
    if (r.scan_backoff_initial_ms == KFWC_BACKOFF_OFF || !r.use_mailbox) {
        cfg.scan_backoff_initial_ticks = 0;
        cfg.scan_backoff_max_ticks = 0;
    } else {
        cfg.scan_backoff_initial_ticks = (kfwc_u64)r.scan_backoff_initial_ms * (kfwc_u64)s->qpc_freq.QuadPart / 1000u;
        cfg.scan_backoff_max_ticks = (kfwc_u64)r.scan_backoff_max_ms * (kfwc_u64)s->qpc_freq.QuadPart / 1000u;
    }
    kfwc_ring_init(&s->ring, s->ring_mem, RING_BYTES, RING_RESERVE);
    kfwc_engine_init(s->engine, &s->plat, &s->sink, &cfg);
    kfwc_engine_note_start(s->engine, s->gpu_count ? s->gpus[s->gpu_sel].device : 0, 0);
    ExReleaseFastMutex(&s->lock);

    KeClearEvent(&s->stop_evt);
    InitializeObjectAttributes(&attrs, NULL, OBJ_KERNEL_HANDLE, NULL, NULL);
    {
        HANDLE h = NULL;
        status = PsCreateSystemThread(&h, THREAD_ALL_ACCESS, &attrs, NULL, NULL, Worker, s);
        if (!NT_SUCCESS(status))
            return status;
        status = ObReferenceObjectByHandle(h, SYNCHRONIZE, *PsThreadType, KernelMode, &s->thread, NULL);
        ZwClose(h);
        if (!NT_SUCCESS(status)) {
            KeSetEvent(&s->stop_evt, 0, FALSE); /* the thread exits on its own; it cannot be joined */
            return status;
        }
    }
    s->timer_res_set = ExSetTimerResolution(10000, TRUE) != 0; /* ask for 1 ms; released at stop */
    s->running = TRUE;
    return STATUS_SUCCESS;
}

static void capture_stop(STATE *s)
{
    if (!s->running)
        return;
    KeSetEvent(&s->stop_evt, 0, FALSE);
    KeWaitForSingleObject(s->thread, Executive, KernelMode, FALSE, NULL);
    ObDereferenceObject(s->thread);
    s->thread = NULL;
    s->running = FALSE;
    if (s->timer_res_set) {
        ExSetTimerResolution(0, FALSE);
        s->timer_res_set = FALSE;
    }
    ExAcquireFastMutex(&s->lock);
    kfwc_engine_stop(s->engine); /* END record (always fits: priority reserve) */
    bar_unmap(s);
    ExReleaseFastMutex(&s->lock);
}

/* ------------------------------------------------------------------------------------- */
/* dispatch                                                                                 */
/* ------------------------------------------------------------------------------------- */

#define WRITE_RIGHTS (FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES | DELETE | WRITE_DAC | WRITE_OWNER)

NTSTATUS KfwcDispatch(PDEVICE_OBJECT device, PIRP irp)
{
    PIO_STACK_LOCATION stack = IoGetCurrentIrpStackLocation(irp);
    NTSTATUS status = STATUS_INVALID_DEVICE_REQUEST;
    ULONG_PTR info = 0;
    STATE *s = g_state;
    UNREFERENCED_PARAMETER(device);

    switch (stack->MajorFunction) {
    case IRP_MJ_CREATE:
        if (stack->FileObject->FileName.Length)
            status = STATUS_OBJECT_NAME_INVALID;
        else if (stack->Parameters.Create.SecurityContext->DesiredAccess & WRITE_RIGHTS)
            status = STATUS_ACCESS_DENIED; /* read-only device: write access is never granted */
        else
            status = STATUS_SUCCESS;
        break;
    case IRP_MJ_CLEANUP:
        capture_stop(s); /* the reader went away: stop spending CPU */
        status = STATUS_SUCCESS;
        break;
    case IRP_MJ_CLOSE:
        status = STATUS_SUCCESS;
        break;
    case IRP_MJ_WRITE:
        status = STATUS_ACCESS_DENIED;
        break;
    case IRP_MJ_READ: {
        ULONG cap = stack->Parameters.Read.Length;
        if (cap < KFWC_MIN_READ_BYTES) {
            status = STATUS_BUFFER_TOO_SMALL;
        } else if (InterlockedCompareExchange(&s->reader_busy, 1, 0) != 0) {
            status = STATUS_DEVICE_BUSY;
        } else {
            if (cap > KFWC_MAX_READ_BYTES)
                cap = KFWC_MAX_READ_BYTES;
            info = kfwc_ring_get(&s->ring, (kfwc_u8 *)irp->AssociatedIrp.SystemBuffer, cap);
            InterlockedExchange(&s->reader_busy, 0);
            status = STATUS_SUCCESS; /* zero bytes: nothing yet; the caller polls */
        }
        break;
    }
    case IRP_MJ_DEVICE_CONTROL: {
        ULONG code = stack->Parameters.DeviceIoControl.IoControlCode;
        ULONG in_len = stack->Parameters.DeviceIoControl.InputBufferLength;
        ULONG out_len = stack->Parameters.DeviceIoControl.OutputBufferLength;
        PVOID buf = irp->AssociatedIrp.SystemBuffer;
        if (code == KFWC_IOCTL_INFO && in_len == 0 && out_len >= sizeof(kfwc_info)) {
            kfwc_info *i = (kfwc_info *)buf;
            RtlZeroMemory(i, sizeof(*i));
            ExAcquireFastMutex(&s->lock);
            i->driver_abi = KFWC_DRIVER_ABI;
            i->engine_abi = KFWC_ENGINE_ABI;
            i->module_version = KFWC_MODULE_VERSION;
            i->gpu_count = s->gpu_count;
            if (s->gpu_count && s->gpu_sel < s->gpu_count) {
                i->pci_vendor_id = s->gpus[s->gpu_sel].vendor;
                i->pci_device_id = s->gpus[s->gpu_sel].device;
                i->pci_revision_id = s->gpus[s->gpu_sel].revision;
            }
            i->bar_state = s->bar_state;
            i->running = s->running;
            i->ring_bytes = RING_BYTES;
            i->ram_ranges = s->range_count;
            i->qpc_frequency = (kfwc_u64)s->qpc_freq.QuadPart;
            i->started_qpc = (kfwc_u64)s->qpc_start.QuadPart;
            ExReleaseFastMutex(&s->lock);
            info = sizeof(*i);
            status = STATUS_SUCCESS;
        } else if (code == KFWC_IOCTL_STATS && in_len == 0 && out_len >= sizeof(kfwc_stats_out)) {
            kfwc_stats_out *o = (kfwc_stats_out *)buf;
            ExAcquireFastMutex(&s->lock);
            o->engine = s->engine->stats;
            o->ring_puts = s->ring.puts;
            o->ring_refused = s->ring.refused;
            o->ring_bytes_in = s->ring.bytes_in;
            o->ring_bytes_out = s->ring.bytes_out;
            o->ring_high_water = s->ring.high_water;
            o->loop_iterations = s->loops;
            o->bar_remaps = s->bar_remaps;
            o->pci_scans = s->pci_scans;
            ExReleaseFastMutex(&s->lock);
            info = sizeof(*o);
            status = STATUS_SUCCESS;
        } else if (code == KFWC_IOCTL_EVENTS && in_len == 0 && out_len >= sizeof(kfwc_event)) {
            ExAcquireFastMutex(&s->lock);
            info = kfwc_engine_events(s->engine, (kfwc_event *)buf, out_len / (ULONG)sizeof(kfwc_event)) *
                   sizeof(kfwc_event);
            ExReleaseFastMutex(&s->lock);
            status = STATUS_SUCCESS;
        } else if (code == KFWC_IOCTL_START && in_len == sizeof(kfwc_start_req) && out_len == 0) {
            status = capture_start(s, (const kfwc_start_req *)buf);
        } else if (code == KFWC_IOCTL_STOP && in_len == 0 && out_len == 0) {
            capture_stop(s);
            status = STATUS_SUCCESS;
        } else if (code == KFWC_IOCTL_INFO || code == KFWC_IOCTL_STATS || code == KFWC_IOCTL_EVENTS ||
                   code == KFWC_IOCTL_START || code == KFWC_IOCTL_STOP) {
            status = STATUS_BUFFER_TOO_SMALL;
        }
        break;
    }
    default:
        break;
    }
    irp->IoStatus.Status = status;
    irp->IoStatus.Information = info;
    IoCompleteRequest(irp, IO_NO_INCREMENT);
    return status;
}

static void state_free(STATE *s)
{
    if (!s)
        return;
    bar_unmap(s);
    if (s->ranges)
        ExFreePool(s->ranges);
    if (s->ring_mem)
        ExFreePoolWithTag(s->ring_mem, TAG);
    if (s->engine)
        ExFreePoolWithTag(s->engine, TAG);
    ExFreePoolWithTag(s, TAG);
}

VOID KfwcUnload(PDRIVER_OBJECT driver)
{
    STATE *s = g_state;
    IoDeleteSymbolicLink(&g_link);
    if (s)
        capture_stop(s);
    g_state = NULL;
    state_free(s);
    IoDeleteDevice(driver->DeviceObject);
}

NTSTATUS DriverEntry(PDRIVER_OBJECT driver, PUNICODE_STRING registry_path)
{
    UNICODE_STRING name = RTL_CONSTANT_STRING(L"\\Device\\KayfabeWinCapture");
    UNICODE_STRING sddl = RTL_CONSTANT_STRING(L"D:P(A;;GA;;;SY)(A;;GA;;;BA)");
    PDEVICE_OBJECT device = NULL;
    NTSTATUS status;
    STATE *s;
    ULONG i;
    UNREFERENCED_PARAMETER(registry_path);

    status = IoCreateDeviceSecure(driver, 0, &name, FILE_DEVICE_UNKNOWN, FILE_DEVICE_SECURE_OPEN, TRUE, &sddl,
                                  &g_class, &device);
    if (!NT_SUCCESS(status))
        return status;
    s = (STATE *)ExAllocatePool2(POOL_FLAG_NON_PAGED, sizeof(*s), TAG);
    if (!s) {
        IoDeleteDevice(device);
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    ExInitializeFastMutex(&s->lock);
    KeInitializeEvent(&s->stop_evt, NotificationEvent, TRUE);
    s->engine = (kfwc_engine *)ExAllocatePool2(POOL_FLAG_NON_PAGED, sizeof(kfwc_engine), TAG);
    s->ring_mem = (PUCHAR)ExAllocatePool2(POOL_FLAG_NON_PAGED, RING_BYTES, TAG);
    if (!s->engine || !s->ring_mem) {
        state_free(s);
        IoDeleteDevice(device);
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    s->plat.ctx = s;
    s->plat.read_ram = plat_read_ram;
    s->plat.ram_contains = plat_ram_contains;
    s->plat.ram_range_count = plat_range_count;
    s->plat.ram_range = plat_range;
    s->plat.read_reg32 = plat_read_reg32;
    s->plat.now = plat_now;
    s->sink.ctx = s;
    s->sink.put = sink_put;
    s->qpc_start = KeQueryPerformanceCounter(&s->qpc_freq);
    kfwc_ring_init(&s->ring, s->ring_mem, RING_BYTES, RING_RESERVE);
    kfwc_engine_init(s->engine, &s->plat, &s->sink, &(kfwc_config){0});
    g_state = s;
    for (i = 0; i <= IRP_MJ_MAXIMUM_FUNCTION; i++)
        driver->MajorFunction[i] = KfwcDispatch;
    driver->DriverUnload = KfwcUnload;
    status = IoCreateSymbolicLink(&g_link, &name);
    if (!NT_SUCCESS(status)) {
        g_state = NULL;
        state_free(s);
        IoDeleteDevice(device);
        return status;
    }
    device->Flags |= DO_BUFFERED_IO;
    device->Flags &= ~DO_DEVICE_INITIALIZING;
    return STATUS_SUCCESS;
}
