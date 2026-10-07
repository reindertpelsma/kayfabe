/*
 * nvkvm_gpu_emul.c — Mode-2 emulated NVIDIA GPU PCI device (M0).
 *
 * Mode-2 "reverse driver": the guest runs the real, stock NVIDIA driver against
 * this emulated GPU.  We "fake the boot" (never run a real GSP), translate the
 * addresses the driver programs, and forward real compute to the host GPU via
 * the Mode-1 core.  See docs/design/mode2_plan.md and
 * docs/design/nvidia_gpu_internals.md.
 *
 * M0 scope (this commit): present a self-consistent GA106 (RTX 3060) PCI
 * function with the real NVIDIA BAR layout and a BAR0 register aperture that
 *   (a) logs every access (offset, size, value, R/W) — the ground-truth trace
 *       of what RmInitAdapter wants, and
 *   (b) answers the chip-identity registers (NV_PMC_BOOT_0 / _42) so the
 *       driver's chip detection + HAL selection succeed and it proceeds far
 *       enough to expose the next stall.
 * Everything else reads back 0 for now.  The boot-register state machine
 * (M1/M2: GFW_BOOT, HWCFG2._RISCV, RISCV_STATUS, mailboxes) is stubbed behind
 * the reg_read/reg_write switch and filled in next.
 *
 * Design rules carried from the plan:
 *  - MULTI-GPU: per-instance state only.  No globals, no g_nvkvm_device
 *    singleton (the Mode-1 anti-pattern).  Every register block, BAR, and (later)
 *    RPC endpoint / isolate table is a field of NvkvmGpuEmul, so N of these can
 *    coexist in one VM, each bound to its own host GPU.
 *  - The chip identity lives in a single NvkvmGpuChip descriptor so PCI IDs,
 *    PMC_BOOT_0, PMC_BOOT_42 and HWCFG all describe the same silicon
 *    (spike §5.2 self-consistent identity).
 *
 * This is the thin C QEMU device shell.  The untrusted-input logic core
 * (GSP-RPC decode, address virtualization) will be Rust behind a narrow C ABI
 * (docs/design/mode2_plan.md "Language"); none of that exists yet at M0.
 */

#include "qemu/osdep.h"
#include "qemu/log.h"
#include "qemu/error-report.h"
#include "qemu/timer.h"
#include "hw/pci/pci.h"
#include "hw/pci/pci_device.h"
#include "hw/pci/pcie.h"
#include "hw/pci/msix.h"
#include "hw/qdev-properties.h"
#include "qapi/error.h"
#include "qemu/module.h"
#include "qom/object.h"
#include "mode2_devinfo_ga106.h"   /* captured GA106 engine table (M5 replay) */
#include "mode2_initctrl_ga106.h"  /* captured GA106 init-control responses    */
#include "mode2_intrtable_ga106.h" /* captured GA106 interrupt table (M5)      */
#include "mode2_gspstaticinfo_ga106.h" /* captured GA106 GSP static config (M5) */
#include "mode2_compute_ctrls_ga106.h" /* captured GA106 cuInit compute-cap ctrls */
#include "virtio_nvgpu.h"   /* M5: Mode-1 forwarding stack (isolate API + NVOS structs) */
#include "exec/cpu-common.h" /* M6.0: qemu_ram_foreach_block/get_fd — guest-RAM memfd for item-4 */
#include "sysemu/sysemu.h"  /* #90: qemu_add_exit_notifier — a killed QEMU still flushes */
#include "nvkvm_m2_rec.h"   /* #90: the §6 replay-trace recorder (m2rec=on) */

/* ── Chip identity ─────────────────────────────────────────────────────────
 *
 * Single source of truth for "which silicon are we pretending to be".  GA106
 * (RTX 3060) by default so the in-guest driver selects the same HAL family
 * (ampere/ga10x) the host GPU uses and the downstream Mode-1 forwarding speaks
 * the same register/RPC dialect.
 *
 * NV_PMC_BOOT_0 (dev_boot / nv_ref.h):
 *   ARCHITECTURE_0 [28:24] = GA100 family = 0x17
 *   IMPLEMENTATION [23:20] = 6  (GA106; see g_hal_archimpl.h:76)
 *   MAJOR_REVISION [7:4], MINOR_REVISION [3:0] = stepping (0xA1)
 *   => 0x176000A1
 * NV_PMC_BOOT_42 (0xA00):
 *   ARCHITECTURE [29:24] = 0x17, IMPLEMENTATION [23:20] = 6,
 *   MAJOR_REVISION [19:16] = 0xA, MINOR_REVISION [15:12] = 1
 *   => 0x176A1000  (CHIP_ID [29:20] = 0x176)
 */
typedef struct NvkvmGpuChip {
    const char *name;
    uint16_t    vendor_id;        /* 0x10DE NVIDIA */
    uint16_t    device_id;        /* 0x2503 GA106 / RTX 3060 */
    uint16_t    sub_vendor_id;
    uint16_t    sub_device_id;
    uint8_t     revision;         /* PCI revision id (0xA1 for GA106-300) */
    uint32_t    pmc_boot_0;       /* BAR0 + 0x000 */
    uint32_t    pmc_boot_42;      /* BAR0 + 0xA00 */
    uint64_t    bar0_size;        /* REGS aperture (16 MiB) */
    uint64_t    bar1_size;        /* FB aperture (16 GiB — resizable-BAR-equiv, covers full fake FB) */
    uint64_t    bar3_size;        /* usermode/IMEM aperture (32 MiB) */
} NvkvmGpuChip;

static const NvkvmGpuChip nvkvm_chip_ga106 = {
    .name          = "GA106",
    .vendor_id     = 0x10DE,
    .device_id     = 0x2504,        /* RTX 3060 LHR — matches the dev host card */
    .sub_vendor_id = 0x1462,        /* MSI */
    .sub_device_id = 0x397D,        /* matches host SSID + dumped VBIOS PCIR */
    .revision      = 0xA1,
    .pmc_boot_0    = 0x176000A1u,
    .pmc_boot_42   = 0x176A1000u,
    .bar0_size     = 16ull  << 20,  /* 16 MiB  */
    .bar1_size     = 16ull  << 30,  /* 16 GiB: resizable-BAR-equivalent. A 256 MiB BAR1 capped how
                                     * much vidmem the guest CPU could aperture-map at once (a hard
                                     * prod-correctness ceiling, independent of the host-side BAR1 /
                                     * CE-forward work). 16 GiB covers the full GSP-advertised fake FB
                                     * (~11.7 GiB usable). 64-bit prefetchable BAR → lands in the q35
                                     * above-4G PCI window; aperture is sparse MMIO (no real backing),
                                     * GMMU-walked via bar1_pdb, so only the addressable RANGE grows. */
    .bar3_size     = 32ull  << 20,  /* 32 MiB  */
};

#include "mode2_regs_ga10x.h"  /* GA10x register offsets + GMMU VER2 format */

/* Max bytes for a single GSP RPC response message (header + body + params),
 * spanning multiple 4 KiB queue elements.  GET_DEVICE_INFO_TABLE is the largest
 * at paramsSize=24580 (+120 hdr); round up with headroom. */
#define NVKVM_RESP_MAX 40960u

/* GMMU walk "no translation" sentinel (returned by nvkvm_{bar2,chan}_translate). */
#define NVKVM_GMMU_FAULT        (~0ull)

/* DIAG (removable): low-FB window where the UVM/RM-internal channel's
 * GPFIFO/USERD/instblk/semaphore are allocated (observed 0x311xxxx..0x315xxxx). */
#define NVKVM_DIAG_LOFB_LO      0x3000000ull
#define NVKVM_DIAG_LOFB_HI      0x3300000ull

/* ── Device state (per instance — multi-GPU safe) ──────────────────────────*/
#define TYPE_NVKVM_GPU_EMUL "nvkvm-gpu-emul"
OBJECT_DECLARE_SIMPLE_TYPE(NvkvmGpuEmul, NVKVM_GPU_EMUL)

#define NVKVM_GPU_MSIX_VECTORS 8   /* room for PMC top-level + per-engine */

struct NvkvmGpuEmul {
    PCIDevice parent_obj;

    /* identity */
    const NvkvmGpuChip *chip;

    /* BARs */
    MemoryRegion bar0;   /* REGS  — container (BAR0); holds bar0_io + optional gsp_falcon overlay */
    MemoryRegion bar0_io;/* the trapped/logged MMIO ops region (fills all of BAR0 at priority 0)   */
    MemoryRegion gsp_falcon; /* M5.64: rom-device overlay on BAR0 0x110000 — GSP falcon status page
                              * served from a RAM buffer (poll READS hit RAM = no vmexit), WRITES still
                              * trap (QUEUE_HEAD doorbell, CPUCTL STARTCPU, IRQSCLR). Kills the
                              * DEBUGINFO(0x94)/IRQSTAT/DMATRFCMD poll vmexit storm. */
    uint8_t *gsp_falcon_ram; /* host pointer to the rom-device RAM (kept current by *_sync) */
    MemoryRegion bar1;   /* FB    — MMIO stub for M0 (address-virt layer L8) */
    MemoryRegion bar3;   /* IMEM/usermode — MMIO stub                        */
    MemoryRegion msix;   /* MSI-X table/PBA BAR (BAR5)                       */

    /* VBIOS served from the BAR0 PROM window (M2) */
    char    *vbios_path;     /* "vbios=" property: file with a real VBIOS dump */
    uint8_t *vbios;          /* loaded image (NV_PROM_DATA_SIZE bytes, padded)  */
    uint64_t prom_reads;     /* count (don't per-access trace — VBIOS is ~1 MiB)*/

    /* M3 — GSP-RPC message queue */
    uint32_t mbox0, mbox1;   /* GSP falcon mailbox halves (LibOS boot-args GPA) */
    uint32_t sec_mbox0;      /* #12 L3 (2026-06-20): SEC2 falcon MAILBOX0 latch.  The
                              * SEC2 Booter is run for both LOAD (raises WPR2) and
                              * UNLOAD (lowers WPR2); from BAR0 alone they differ only
                              * by the mailbox args set before SEC2 STARTCPU.  A NORMAL
                              * Booter Unload writes MAILBOX0/1 = 0xff (Load writes 0 or
                              * the WprMeta GPA), so we latch MAILBOX0 and, on the SEC2
                              * STARTCPU, bring WPR2 down iff it is 0xff — otherwise the
                              * driver's kgspExecuteBooterUnloadIfNeeded reads WPR2 still
                              * up after Unload and asserts (osinit.c:2363).            */
    bool     bootargs_dumped;/* one-shot: read+log the queue region once        */
    bool     fwsec_ran;      /* set when GSP falcon STARTCPU written: FWSEC "ran"
                              * -> WPR2 becomes "initialized" (stateful: the
                              * driver checks WPR2 DOWN before FWSEC, UP after).  */
    bool     gsp_suspended;  /* set on fn=47 UNLOADING teardown.  kgspUnloadRm ->
                              * kgspWaitForProcessorSuspend polls FALCON_MAILBOX0
                              * for INTERRUPT_PROCESSOR_SUSPENDED_VALUE(0x80000000);
                              * the faked GSP must report suspended or the close
                              * hangs 4s (_threadNodeCheckTimeout) and WPR2 stays
                              * up -> next open EIO (WPR2 re-boot cascade).        */
    bool     gsp_reloaded;   /* #12 (2026-06-20): set when the guest re-loads the GSP
                              * falcon image (a DMATRFCMD transfer) WHILE suspended —
                              * the unambiguous "this is a genuine GSP re-boot" signal
                              * for a context RE-ACQUIRE (cuCtxDestroy of the last ctx
                              * sends fn-47 UNLOADING; the next cuCtxCreate reloads the
                              * falcon + STARTCPUs).  Distinguishes that re-boot STARTCPU
                              * (must raise WPR2) from a bare trailing-teardown STARTCPU
                              * (must NOT), so a 2nd context boots instead of hanging
                              * forever waiting for GSP_INIT_DONE (#12 next-layer).   */

    /* M4 — GSP-RM RPC shim. Cached message-queue layout (from RMARGS) + ring
     * state. The driver posts a command on the cmd queue then writes the cmd
     * QUEUE_HEAD doorbell (0x110c00); we read the command and post an echo
     * response (rpc_result=NV_OK) on the status queue. */
    bool     q_ready;        /* queue layout cached, GSP_INIT_DONE posted        */
    uint64_t q_shmem;        /* message-queue shared region GPA                  */
    uint32_t q_cmd_base;     /* cmdQueueOffset (cmd queue backing store offset)  */
    uint32_t q_stat_base;    /* statQueueOffset (status queue backing store off) */
    uint32_t q_msgsize;      /* msgq entry size (GSP_MSG_QUEUE_ELEMENT_SIZE_MIN) */
    uint32_t q_msgcount;     /* entries per queue                                */
    uint32_t q_cmd_entryoff; /* cmd queue entries offset                         */
    uint32_t q_stat_entryoff;/* status queue entries offset                      */
    uint32_t stat_writeptr;  /* status queue monotonic writePtr (in ELEMENTS)    */
    uint32_t stat_seqnum;    /* per-MESSAGE seqNum (guest rxSeqNum, +1 per reply) */
    uint32_t cmd_readptr;    /* cmd queue messages we've consumed/answered        */

    /* M6 — GPU memory: sparse FB backing + BAR0 PRAMIN window.  The driver
     * accesses VRAM before BAR2 is up via a 1MB window in BAR0 (NV_PRAMIN @
     * 0x700000): it programs NV_PBUS_BAR0_WINDOW (0x1700) BASE=FBaddr>>16,
     * TARGET=aperture, then reads/writes NV_PRAMIN+(FBaddr&0xffff).  We back
     * that with a sparse page table so writes read back (kbusVerifyBar2). */
    uint32_t bar0_window;    /* NV_PBUS_BAR0_WINDOW (0x1700): BASE[23:0]|TARGET[25:24] */
    GHashTable *fb_pages;    /* sparse FB: page index (addr>>12) -> malloc'd 4KB  */
    bool     bar2_virtual;   /* BAR2_BLOCK MODE bit31: 1=VIRTUAL (walk), 0=PHYSICAL (id) */
    uint64_t bar2_pdb;       /* BAR2 page-dir base from GspStaticConfigInfo.bar2PdeBase
                              * (the GSP binds BAR2; CPU bind is a no-op on GSP-client) */
    uint64_t bar2_inst_block;/* FB addr of the BAR2 instance block (NV_PBUS_BAR2_BLOCK
                              * 0x1714: PTR[27:0]<<12).  Holds the BAR2 page-dir base;
                              * BAR2 accesses are GMMU-VER2-walked through it.        */
    uint64_t bar1_pdb;       /* BAR1 (FB aperture) page-dir base from
                              * GspStaticConfigInfo.bar1PdeBase (offset 1664) + the
                              * UPDATE_BAR_PDE(BAR_1) root entry.  The driver maps
                              * channel USERD/pushbuffers into BAR1 and the CPU
                              * writes GP_PUT through it — must GMMU-walk to FB.     */

    /* M5 channel tracking: captured from the most-recent *_CHANNEL_GPFIFO_A
     * GSP_RM_ALLOC (fn 103).  During init there is a single CE channel (the
     * scrubber), so the doorbell-rung channel is the last one allocated.  These
     * locate the GPFIFO ring so the doorbell handler can walk submitted work. */
    uint64_t chan_gpfifo_va;   /* gpFifoOffset: GPU VA of the channel's GPFIFO ring */
    uint32_t chan_gpfifo_ent;  /* gpFifoEntries */
    uint32_t chan_class;       /* hClass of the tracked channel                     */
    uint64_t chan_inst_block;  /* instanceMem.base: channel instance block (unused: GSP-managed, empty) */
    bool     chan_inst_sys;    /* instanceMem.addressSpace == ADDR_SYSMEM(1)        */
    uint64_t chan_pdb;         /* PDB read from the executing channel's instance
                                * block (RAMIN +0x200): HW-authoritative VAS root.
                                * 0 if the instblk is empty (GSP-managed) -> fall
                                * back to the snooped chan_vas[] heuristic.        */
    uint32_t chan_payload;     /* completion payload counter (incr per doorbell)    */
    bool     chan_sem_released; /* set by chan_execute when it honored an explicit
                                  * NVC56F SEM_EXECUTE release from the pushbuffer    */
    uint64_t chan_userd;       /* userdMem.base: USERD memory (holds GP_PUT/GP_GET) */
    bool     chan_userd_sys;   /* userdMem.addressSpace == ADDR_SYSMEM              */
    uint32_t chan_gp_get;      /* our consumed GPFIFO index (entries [get,put) pend)*/

    /* Multi-channel table.  Init allocates several GPFIFO channels (e.g. the
     * CeUtils memory scrubber AND its self-verify channel), so a doorbell can
     * target ANY of them — not just the most-recently allocated one (which the
     * single chan_* fields above tracked, dropping the scrubber's work -> the
     * ce_utils.c:349 timeout).  On a doorbell we walk EVERY channel's pending
     * GPFIFO so we never need to map the doorbell token's chid to a channel. */
#define NVKVM_MAX_CHANS 64   /* #14: two concurrent processes register ~27 channels EACH */
    struct nvkvm_chan_entry {
        uint64_t gpfifo_va, userd;
        uint32_t gpfifo_ent, gp_get, hvaspace, payload;
        uint32_t fin_payload;   /* #12: monotonic count of submitted GPFIFO entries on this
                                 * channel == CeUtils lastSubmittedPayload (each memset/memcopy
                                 * does payload=lastSubmitted+1 and submits exactly one entry,
                                 * ce_utils.c:611). Used to forge the VIDMEM finishPayload sema
                                 * the GSP-managed scrub channel never lets us parse/write. */
        uint64_t fin_fb;        /* #12: forward-populated FB address of THIS channel's VIDMEM
                                 * finishPayload sema (ring_base_BAR1off + (gpfifo_va&0xfff) +
                                 * 0x8004, GMMU-walked through bar1_pdb).  Resolved ONCE — the
                                 * first doorbell the channel advances, when its just-written ring
                                 * page is freshest in bar1_wpg so the M5.16 MRU scan picks the
                                 * RIGHT page — then PINNED.  Re-resolving every doorbell drifts
                                 * (the global chan_gpfifo_bar1off is stomped by whichever channel
                                 * last decoded plausibly), splitting the monotonic payload across
                                 * two FB pages so the guest's real sema never reaches lastSubmitted
                                 * (#12 root cause, proven 2026-06-20).  0 = unresolved. */
        bool     fin_sys;       /* #12 L3b: aperture of fin_fb — true if the finishPayload
                                 * resolved to SYSMEM (kernel CeUtils channel buffer is sysmem
                                 * by default), false = FB (VIDMEM ring).  Selects phys_rd32/
                                 * wr32 aperture so the forge hits the page the guest reads.
                                 * (cont.24: forge now routes through nvkvm_chan_sem_wr32, which
                                 * resolves per-call; fin_fb/fin_sys retained but unused.) */
        uint32_t client;        /* owning RM client (hClient) — VAS scope key */
        bool     userd_sys;
        uint32_t hobject;       /* M5.12: the channel's RM handle (== host handle: shadow_fwd
                                 * creates the host channel with the SAME hObject) */
        uint32_t host_token;    /* M5.12: host channel work-submit token (0xc36f0108), for the
                                 * GP_PUT-driven doorbell demux: ring THIS channel's token */
        bool     token_valid;
        uint32_t tsg;           /* M5.25: parent TSG (a06c) handle — must be GPFIFO_SCHEDULE'd
                                 * before a ring runs (guest's schedule control isn't forwarded) */
        bool     scheduled;     /* M5.25: TSG GPFIFO_SCHEDULE'd on the host once */
        uint32_t sweep_put;     /* M5.48c: GP_PUT as of the last working-set sweep — sweep
                                 * again only when a channel's PUT ADVANCES (new submission =
                                 * the guest just mapped+filled new working-set leaves) */
        bool     ce_route_logged; /* CE-fwd Step 0: one-shot NVKVM-CEFWD-ROUTE probe per channel */
        /* M5.62 OPAQUE fast-path (user's plan): once a userspace channel's working set is fully
         * resident (the dirty-sweep backs everything; the walk maps NOTHING new for K subs), skip
         * the per-doorbell GPFIFO walk + va_seen (uncached-vidmem reads) — just advance gp_get and
         * ring, like Mode-1's opaque channel. Re-armed (resident=false) on ANY sweep (a mapping
         * changed) so discovery resumes; self-heals the same way the sweep already does. */
        uint32_t stable_subs;   /* consecutive submissions with newpushbufs==0 */
        bool     resident;      /* working set fully resident -> skip the walk */
        /* #14 P1: the guest vChid, recovered from the channel-alloc USERD_INDEX flags
         * (GSP-client CPU-RM encodes its already-decided ChID there so "Physical RMAPI
         * uses our ChID", kernel_channel.c:2688: vchid = flags[20:12]*8 + flags[10:8]).
         * E0 proved the doorbell token[11:0] == vChid, distinct per channel — this is
         * the doorbell -> channel demux key (plan §1.4).  P1: resolution+logging only. */
        uint32_t vchid;
        bool     vchid_valid;
    } chans[NVKVM_MAX_CHANS];
    int chan_n;
    uint32_t chan_client;       /* working-set: client of the channel chan_exec runs */

    /* ── Address-virtualization #2 side-table (the reverse-driver core) ────────
     * For GSP-managed VASes the leaf PTEs are filled GSP-side and never land in
     * our FB, so nvkvm_walk_pdb FAULTs.  Instead we reconstruct GPU-VA -> physical
     * from the RM op that establishes the mapping: NV2080_CTRL_CMD_GPU_PROMOTE_CTX
     * (0x2080012b) hands GSP a table of context-buffer entries
     * {gpuPhysAddr, gpuVirtAddr, size, physAttr(aperture)}.  We record them here,
     * keyed by the channel's RM client (hChanClient) so VAs don't collide across
     * processes/VASes.  nvkvm_chan_translate consults this FIRST.
     * docs/design/mode2_address_virtualization.md (capture path #2). */
#define NVKVM_MAX_MAPS 1024
    struct nvkvm_va_map {
        uint32_t client;
        uint64_t va, phys, size;
        bool     sys;           /* aperture: true=sysmem(COH/NCOH), false=FB(vidmem) */
    } va_map[NVKVM_MAX_MAPS];
    int va_map_n;

    /* M5.16 — physical FB pages the guest CPU has WRITTEN via BAR1 (vidmem).
     * For a GSP-managed channel VAS the channel-GMMU walk of gpFifoVA resolves to
     * a stale aliasing page (reads 0), but the guest's OWN CPU mapping of the ring
     * goes through BAR1 (bar1_pdb, CPU-built PTEs) and lands in our FB at the TRUE
     * backing page (proven: compute GPFIFO write hit FB 0x3130000 via BAR1 off
     * 0xa0000, while the VAS walk wrongly gave 0x2eee10000).  Record those pages,
     * most-recent-first, so chan_execute can resolve the GPFIFO to where the guest
     * actually wrote it rather than trusting the unreliable channel-VAS walk. */
#define NVKVM_MAX_BAR1PG 64
    struct { uint64_t page; uint64_t seq; uint64_t off; } bar1_wpg[NVKVM_MAX_BAR1PG];
    int      bar1_wpg_n;
    uint64_t bar1_wpg_seq;
    uint64_t chan_gpfifo_phys;  /* M5.16: if non-0, read GP entries DIRECTLY from this
                                 * FB phys (the BAR1-resolved true ring), bypassing the
                                 * stale channel-VAS walk for the GPFIFO entry. */
    uint64_t chan_gpfifo_bar1off; /* #12: BAR1 page-offset M5.16 resolved chan_gpfifo_phys at.
                                 * The channel buffer is contiguous in BAR1-offset space (one
                                 * memdesc -> one BAR1 VA range) even when FB-fragmented, so the
                                 * VIDMEM finishPayload FB page = walk_pdb(bar1_pdb, this +
                                 * (gpfifo_va&0xfff) + 0x8004) — robust to FB non-contiguity. */
    uint64_t chan_fin_ring_off; /* #12 cont.25: BAR1 page-offset of THIS channel's GPFIFO ring,
                                 * captured by M5.16's scan even when the pushbuffer-VAS validation
                                 * FAILS (the GSP-managed bUseBar1 CeUtils case: chan_pdb stays 0
                                 * so chan_gpfifo_bar1off is NOT pinned, but the ring page is still
                                 * identified by its GP entry decoding to a pushbuffer pointer just
                                 * below gpfifo_va).  The forge resolves the finishPayload FB the
                                 * guest's BAR1 poll reads = walk_pdb(bar1_pdb, this +
                                 * (gpfifo_va&0xfff) + 0x8004).  0 = not found this chan_execute. */
    bool     chan_fin_ring_found;

    /* M7 — CPU interrupt tree (raise MSI-X on LEAF_TRIGGER; ISR reads TOP/LEAF) */
    uint32_t intr_leaf[NVKVM_VF_INTR_NLEAF];     /* pending per leaf reg */
    uint32_t intr_leaf_en[NVKVM_VF_INTR_NLEAF];  /* enables */
    uint32_t intr_top;                           /* pending subtree bitmask (TOP(0)) */
    uint32_t intr_top_en;

    /* M5/M7 — GSP os-event delivery.  cuCtxCreate's blocking-sync wait parks
     * libcuda in poll() on an os-event fd; the channel completes (semaphore
     * released) but no completion interrupt is delivered, so poll() never wakes
     * (lost wakeup).  We record every NV01_EVENT_OS_EVENT (0x0079) alloc, and on
     * a doorbell-completed channel post a GSP NV_VGPU_MSG_EVENT_POST_EVENT
     * (0x1003) for each + raise the GSP falcon SWGEN0 interrupt (vector 155 =
     * MC_ENGINE_IDX_GSP stall) so kgspService drains the queue -> _kgspRpcPostEvent
     * -> osNotifyEvent -> nv_post_event wakes the poll.  Mirrors Mode-1 #127. */
    struct { uint32_t hclient, hevent, notify_index; } osevents[64];
    int      osevent_n;
    bool     gsp_swgen0_pending;                 /* GSP falcon IRQSTAT SWGEN0 latched */
    /* VAS root page-dir bases snooped from VASPACE_COPY_SERVER_RESERVED_PDES
     * (0x90f10106): levels[0].physAddress roots the WHOLE VAS (the params' VA
     * range is only the reserved window, not the VAS extent), keyed by the
     * VASpace handle (control hObject).  Matched to a channel via the channel's
     * hVASpace.  This is the channel PDB source (the GSP-managed instblk is empty
     * in our FB). */
    /* root_sys: the page-directory ROOT lives in sysmem (guest RAM) rather than
     * FB.  False for the FB-rooted VASes snooped from VASPACE_COPY_SERVER_RESERVED
     * _PDES; set true for UVM-managed VASes whose root we learn from
     * NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY (0x801813) with a SYSMEM aperture. */
    /* .uvm: root captured from NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY — the transport
     * UVM uses to hand RM/GSP the page-directory root of a UVM-MANAGED user VAS (issued
     * under UVM's gpu-ops session client, nv_gpu_ops.c nvGpuOpsSetPageDirectory).  Kernel-
     * internal VASes (CeUtils scrubber etc.) never take this path — #13 uses the flag to
     * scope the CE-PT-write backing trigger to user-compute address spaces. */
    struct { uint32_t hvas; uint32_t client; uint64_t pdb; bool root_sys; bool uvm; } chan_vas[64]; /* #14 P0: 16->64 for N procs */
    int      chan_vas_n;
    /* #12-L3c: STICKY per-client VAS roots — same captures as chan_vas[] but NEVER
     * dropped on RM-handle free.  A GSP-managed UVM CE channel (empty instblk, hvas=0)
     * whose VAS *handle* the guest frees during init still runs later and releases its
     * tracking semaphore; chan_vas[] has been pruned (L1, correctly — gpfifo/pushbuffer
     * translation must not use a freed VAS) so the sema VA falls to the stale global
     * chan_pdb (a FOREIGN channel's root, e.g. CeUtils 0x3114000) → distinct clients'
     * semaphores at the same guest VA collapse onto one phys → uvm_gpu_semaphore.c:776
     * backward jump → 2nd-context hang.  This sticky table is consulted ONLY for the
     * sema write (nvkvm_chan_sem_wr32), client-keyed, so each channel's completion sema
     * resolves under its OWN client's VAS even after the handle is gone. */
    struct { uint32_t client; uint64_t pdb; bool root_sys; } m2_cli_vas[64];
    int      m2_cli_vas_n;
    /* #14: DUP_OBJECT (RPC fn=21, NVOS55) handle-graph edges.  UVM's per-process
     * gpu-ops client DUPs the user's FERMI_VASPACE_A out of libcuda's compute client
     * (nvGpuOpsDupAddressSpace), then issues SET_PAGE_DIRECTORY under the dup handle —
     * so chan_vas[] captures the user VAS under UVM's client, NOT the compute client
     * that owns the executing channel.  With ONE process that ambiguity is harmless;
     * with TWO, the stock driver hands both processes IDENTICAL guest VAs, and any
     * client-blind VAS pick resolves process B's channel under process A's PDB (the
     * #14 concurrent-cuCtxCreate hang).  These observed dup edges are the forward-
     * populated ownership facts (address-table principle: transport-observed, never
     * reverse-derived) that let the pick require "VAS belongs to the executing
     * channel's client".  Edges are dropped when either side's client is freed
     * (RM handle values are reused across process lifetimes). */
    struct { uint32_t dst_client, dst_obj, src_client, src_obj; } m2_dup[64];
    int      m2_dup_n;
    uint32_t chan_hvaspace;    /* the tracked channel's hVASpace handle */

    /* DEBUG-PROOF backdoor (mode2_uvm_complete): the patched guest UVM reports
     * its tracking-semaphore GPA + payload here so QEMU can forge the channel
     * completion for GSP-internal UVM channels whose sysmem mappings aren't in
     * the RPC stream (see docs/design/mode2_address_virtualization.md).  This is
     * a bring-up PROOF that forging the completion unblocks cuInit; production
     * needs a validated guest<->VMM mapping-report channel (untrusted guest). */
    uint32_t dbg_gpa_lo, dbg_gpa_hi;
    /* #12-L3c: page bases (GPA & ~0xfff) the backdoor (0xFFF508) has written.  The
     * backdoor is the AUTHORITATIVE writer for kernel UVM/CeUtils tracking semas (the
     * guest reports the exact GPA + payload).  The CE_SEM_RELEASE pushbuffer parser
     * (nvkvm_chan_sem_wr32) cannot resolve these kernel semas reliably — one GPU-VA
     * (0x121000010) maps to 4+ distinct GPAs across the kernel VASes, so it collapses
     * DISTINCT channels' tracking semas onto one pool slot (CeUtils' climb landing on
     * the UVM channel's slot → uvm_gpu_semaphore.c:776 backward jump → 2nd-ctx hang).
     * So the parser must DEFER: never software-write a page the backdoor owns. */
    uint64_t m2_bd_pages[128];
    int      m2_bd_pages_n;

    /* M5 compute forwarding (docs/design/mode2_compute_forwarding.md). The
     * emulated GPU hosts its own forwarding backend (separate QEMU process from
     * any Mode-1 instance, so this is additive and cannot disturb Mode-1). M5.0
     * smoke test validates the Mode-2-process -> host-GPU path at realize(). */
    bool     m2fwd;                      /* device prop: host-GPU forwarding (default ON; debug-only off) */
    struct nvkvm_isolate_table m2_iso;   /* per-Mode-2 isolate table (own host stub) */
    bool     m2_iso_ready;               /* lazy: isolate created + devices opened   */
    uint32_t m2_iso_id;                  /* the per-guest host isolate id            */
    uint32_t m2_ctl_h;                   /* handle for /dev/nvidiactl in the isolate */
    uint32_t m2_gpu_h;                   /* handle for /dev/nvidia0                   */
    int      m2_gpu_fd;                  /* the stub's /dev/nvidia0 fd in QEMU (SCM_RIGHTS) — for mmap */
    uint32_t m2_fwd_n;                   /* count of forwarded RPCs (diag)           */
    /* M5.1b: guest RM client handle -> host (synthetic, non-colliding) client.
     * Guest client handles live in the global 0xc1xxxxxx namespace and collide
     * with pre-existing host clients (NV_ERR_INSERT_DUPLICATE_NAME), so we remap
     * each to a 0xdeadNNNN handle the host RM accepts, and translate client refs
     * (h_root, and h_object_parent when it names a client). Objects stay verbatim. */
    struct { uint32_t g, h; } m2_cmap[128];
    int      m2_cmap_n;
    uint32_t m2_cmap_next;      /* #14 P0: MONOTONIC host-handle mint counter.  The old
                                 * mint (0xdead0001 + m2_cmap_n) breaks once proc-exit reap
                                 * shrinks the table: a re-mint could collide with a live
                                 * handle.  Sequence is identical while nothing is reaped. */
    /* #14 P0 DEFERRED reap: root-freed clients whose RESOLUTION/BACKING state
     * (m2_cli_vas / va_map / m2_objs / m2_gpga) must NOT be reaped at the free
     * itself — the dying context's userspace still busy-polls overlay-backed
     * pages after its root frees (bench-proven: immediate reap hung cupctx2_min
     * CTX2 destroy).  Consumed at the GSP queue RE-HANDSHAKE (tx-header write =
     * the next context/process boot), when the GPU was idle-released and no
     * poller can exist.  Bounded: on overflow the oldest hygiene is simply
     * skipped (same as the pre-P0 never-reaped behavior). */
    uint32_t m2_reap_pend[32];
    int      m2_reap_pend_n;
    /* ── #14 P1: the per-process registry (plan §3) ─────────────────────────────
     * One entry per live guest CUDA process.  PRIMARY identity = the process's
     * PDB-set (the GR/UVM VASes guest kernel RM created for it), grouped via the
     * dup-edge chain (user compute client = the dup SRC; its UVM gpu-ops client =
     * the dup DST) — NO CR3 anywhere (E0: the doorbell token vChid is distinct per
     * channel, plan §1.4).  Kernel/GSP/scrubber clients (CeUtils, guest-RM internal)
     * never join a proc — they are the implicit SYSTEM class (m2_sys becomes explicit
     * in P2 with the per-proc isolates).  P1 = registry + logging ONLY: nothing
     * consumes these yet, so behavior is byte-identical. */
#define NVKVM_MAX_PROCS       16
#define NVKVM_PROC_MAX_VAS     8
#define NVKVM_PROC_MAX_CLIENTS 8
    struct nvkvm_proc {
        bool     live;
        uint32_t clients[NVKVM_PROC_MAX_CLIENTS]; /* [0] = the anchoring user compute client */
        int      clients_n;
        uint64_t pdbs[NVKVM_PROC_MAX_VAS];        /* the PDB-set ("the GPU's CR3"s) */
        int      pdbs_n;
    } m2_proc[NVKVM_MAX_PROCS];
    int m2_proc_n;
    /* M5.3: per-mapping fresh /dev/nvidia0 isolate-fd handle allocator. Handles
     * 1=ctl, 2=gpu are fixed; on-demand context-buffer mappings draw from here
     * (nvidia binds exactly one CPU mapping per device fd, so each needs its own). */
    uint32_t m2_maph_next;
    /* M5.3: VASpaces (FERMI_VASPACE_A 0x90f1) forwarded under each (client,device),
     * so we can give the GR channelgroup an explicit hVASpace when the guest left it
     * 0 (device-default), which doesn't resolve on the forwarded host device. */
    struct { uint32_t client, dev, vas; } m2_devvas[32];
    int      m2_devvas_n;
    /* M5.3: TSG (0xa06c) handle -> engineType, so a channel that passes engineType=0
     * (NULL, "inherit") can be given its TSG's engine explicitly on the host. */
    struct { uint32_t tsg, engine, client; } m2_tsgeng[64];   /* #14: 2 procs; P0: +client
                                 * (populate-site hClient) so proc-exit reap can drop a dead
                                 * process's entries — lookups stay tsg-keyed (byte-identical). */
    int      m2_tsgeng_n;
    /* M5.3 data-plane: the GR-client subdevice (NV20_SUBDEVICE_0 0x2080) handle, needed
     * to issue GR_GET_CTX_BUFFER_INFO on the host shadow context after the compute object
     * is forwarded — the first step of backing the guest's context buffers with real host
     * GPU state (the proven cuCtxCreate fix). Tracked per GR client. */
    struct { uint32_t client, subdev; } m2_subdev[64];
    int      m2_subdev_n;
    /* M14: device-info-table (engine enumeration) captured LIVE from the host GPU via a
     * private QEMU-owned client/subdevice — no hardcoded per-GPU blob. 100B/entry, 32/page. */
    uint8_t  m2_devinfo[256 * 100];
    uint32_t m2_devinfo_n;               /* entries captured (0 = none) */
    bool     m2_devinfo_tried;           /* capture attempted once (success or fail) */
    /* M5.3 DATA-PLANE (double-mmap): FB ranges backed by real host GPU memory. When the
     * guest reads/writes a context-buffer FB address that we've backed with the host
     * shadow context's counterpart (mapped via the proven RM_MAP_MEMORY primitive), serve
     * it from host_qva instead of the local g_malloc0 FB page — so the guest sees real
     * GPU-initialized state (the proven cuCtxCreate fix). Inert until populated. The
     * bring-up mechanism; KVM-memslot backing is the perf endpoint (see design doc). */
    struct { uint64_t fb_base, size; void *host_qva; } m2_fbback[128];  /* #14: 2 procs */
    int      m2_fbback_n;
    /* M5.3 DIAG: crash-window FB-read probe. Set true the moment the GR compute
     * object (0xc7c0) alloc returns OK — libcuda then reads GR-context GPU memory
     * (no further ioctl per the trace) and crashes (rbp=0). Logging every FB read
     * after this flag pins the EXACT buffer (fb_addr+value+backed?) and its access
     * path: appears here => served via FB/BAR1 (FB-overlay backing applies); window
     * empty => libcuda reads it via the UVM mmap to guest-RAM (needs memslot). */
    bool     m2_crashwin;
    uint32_t m2_crashwin_reads;
    uint32_t m2_own_pdb_diag;   /* M5.32 Step-1: bounded diag count for chan_own_pdb misses */
    bool     m2_in_walk;     /* true while reading a GMMU PDE/PTE — excludes page-walk
                              * noise from the CRASHWIN probe so only LEAF data reads
                              * (the buffer values libcuda actually consumes) are logged */
    /* M5.4 data-plane: per forwarded channel, the host-allocated USERD we provide as
     * hUserdMemory[0] (handle-bearing -> mappable, unlike RM's own USERD) and mmap into
     * QEMU, registered in m2_fbback at the guest's userd.base. So the host channel's real
     * USERD IS the guest's USERD view (double-mmap): guest GP_PUT lands in host USERD and
     * the host GPU advances GP_GET there for the guest's poll to see. */
    struct { uint32_t client, chan; uint32_t h_userd; void *qva; uint64_t fb_base, size; }
             m2_chanbuf[96];   /* #14: two processes register ~27 channels EACH */
    int      m2_chanbuf_n;
    uint32_t m2_databuf_next;   /* unique host handle allocator for data-plane objects */
    uint64_t m2_cur_gva;        /* BAR1 GPU VA of the in-flight aperture access (~0=none),
                                 * so the CRASHWIN probe can report the guest GPU VA that
                                 * maps to a polled FB address (correlate 0x2efbaf000) */
    bool     m2_mapdma_tested;  /* M5.5: one-shot RM_MAP_MEMORY_DMA-FIXED primitive validation */
    bool     m2_inventory_done; /* M5.6: one-shot GR working-set inventory dump at doorbell */
    bool     m2_sem_probe_done; /* M5.13: one-shot DRY-RUN locate of the completion semaphore PDB */
    bool     m2exec;            /* M5.7 prop: execution-plane backing (default ON; debug-only off) */
    bool     m2hostsem;         /* M5.35 prop: host GPU owns the completion semaphore — suppress
                                 * QEMU's Phase-B stub sema writes (CE_SEM_RELEASE / NVC56F
                                 * SEM_RELEASE / +0x8004 fallback) so the real host release is the
                                 * SOLE writer.  Fixes the double-writer 2^32 UVM jump once M5.34
                                 * makes the host actually execute the channel.  Default OFF (A/B). */
    bool     m2cefwd;           /* CE-fwd prop. Enables: (Step 0) the NVKVM-CEFWD-ROUTE probe
                                 * of user-CE channel routing, and (Phase A / M5.60) re-walking
                                 * the compute VAS at a user-CE LAUNCH_DMA when the copy dst is an
                                 * un-backed fb_page, so the full dst becomes a real host vidmem
                                 * object in the GR fvas (prereq for the host-CE forward, Phase B).
                                 * Does NOT change completion or the CPU copy (those stay
                                 * authoritative) — regression-safe.  Default OFF. */
    bool     m2cexec;           /* CE-EXEC fwd (perf, 2026-06-15): the host GPU's CE executes the
                                 * user-CE channel's LAUNCH_DMA for real (approach A) instead of the
                                 * CPU byte-copy. Extends the exec_doorbell pushbuffer-forward + token
                                 * ring (today GR-only) to user-CE channels keyed on their OWN client,
                                 * and suppresses chan_execute's CPU copy + CPU sema for those
                                 * channels (the host CE pushbuffer writes the data AND the completion
                                 * sema). Identity map_dma means src/dst VAs are already valid in the
                                 * CE channel's VAS once resident. Sub-flag of m2cefwd (needs the dst
                                 * real-backed). Default OFF — A/B vs the CPU-copy 23.6 tok/s. */
    bool     m2romregs;         /* M5.64: install the GSP-falcon rom-device overlay (reads from RAM,
                                 * no vmexit) — the 0x110094 poll-storm fix. Default OFF (A/B). */
    bool     m2_trace;          /* M5.63: enable the high-volume per-doorbell/per-fb-access DIAG
                                 * qemu_log spew (M5.9/RANG/CE-INSTR/M5.22 + crashwin M5.31/M5.15/
                                 * CRASHWIN-RD/DMAW — ~94 lines/doorbell, ~360k lines/run). Default
                                 * OFF: synchronous qemu_log I/O is pure overhead in control-heavy
                                 * phases. Errors / one-shots / NVKVM-TWIN stay unconditional. */
    bool     m2opaque;          /* M5.62 (user's plan): skip the GPFIFO walk for a userspace channel
                                 * once it's fully resident (newpushbufs==0 for K subs) — ring only.
                                 * Re-armed on any sweep. Default OFF (perf experiment). */
    bool     m2_exec_done;      /* M5.7: one-shot working-set back+map */
    uint32_t m2_exec_sweeps;    /* M5.10: # of doorbell-time GR-VAS re-sweeps done (bounded) */
    /* M5.10 PERF (2026-06-15): the per-submission GR-VAS re-sweep was ~100% wasted on a real LLM
     * (m568: 91932/91960 walks backed nothing). Re-sweep ONLY when a GR page table actually
     * CHANGED: enum_gr_sysmem records the vidmem PT pages it walks (m2_gr_pt_set); a guest write
     * to any tracked PT page (every PTE/PDE edit writes a tracked page or an ancestor of one) sets
     * m2_gr_vas_dirty -> the next doorbell sweeps and rebuilds the set; otherwise it skips. New
     * VAS (chan_vas_n grew) and budget-truncated walks force a sweep too; a periodic net bounds any
     * missed trigger. Fault-safe: a mapping is always backed before the engine that uses it runs. */
    bool     m2_gr_vas_dirty;   /* a tracked GR PT page was written since the last sweep */
    bool     m2_recording_gr_pt;/* true only while enum_gr_sysmem walks (record PT pages) */
    bool     m2_gr_pt_trunc;    /* last sweep hit the walk budget -> coverage incomplete, keep sweeping */
    int      m2_last_swept_vas_n;/* chan_vas_n as of the last sweep (new VAS -> force a sweep) */
    uint32_t m2_db_submits;     /* # of new-work doorbells seen (drives the sparse periodic net) */
    uint64_t m2_gr_pt_lo, m2_gr_pt_hi; /* min/max tracked PT-page base (cheap fb_write pre-filter) */
    uint64_t m2_gr_pt_last;     /* last page recorded (skip re-hashing sequential entry reads) */
    int      m2_gr_pt_n;        /* live entries in m2_gr_pt_set */
    uint64_t m2_gr_pt_set[8192];/* open-addressing hash set of 4 KiB vidmem PT-page bases */
    /* #13: page-table-page ownership for COMPUTE VAS(es).  Each entry = one table the
     * compute-VAS walk visited: its 4 KiB page, owning PDB, GMMU level, and the VA base
     * that level covers.  Populated by the same walks that build m2_gr_pt_set (see
     * nvkvm_m2_cpt_record).  nvkvm_m2_ce_fb_write_hook LATCHES an entry dirty (O(1)) when
     * a CPU-emulated CE write lands on its page; nvkvm_m2_cpt_sync_at_release then decodes
     * each dirtied page DIRECTLY (not via a root walk) and backs its new leaves into the
     * persistent host GR VAS — at the map push's completion-semaphore release, before the
     * release un-gates the (already-rung, host-resident) GR channel into the new mapping.
     * Decoding the written page directly (vs from the root) is load-bearing: the guest
     * fills a leaf PT page THEN links it under the root a push later, so at the release a
     * root walk can't yet reach it (runs=0) but the page itself holds committed PTEs.
     * Keyed by 4 KiB page base (open addressing, page==0 = empty; PT pages never live at
     * FB page 0).  Same lifecycle as m2_gr_pt_set: reset + rebuilt on every recorded sweep. */
    struct { uint64_t page, pdb, vabase; uint8_t level; bool dirty; } m2_cpt[4096];
    int      m2_cpt_n;          /* live entries in m2_cpt (capped at 3/4 fill) */
    uint64_t m2_cpt_lo, m2_cpt_hi; /* min/max tracked page base (cheap CE-write pre-filter) */
    int      m2_cpt_dirty[256]; /* indices of m2_cpt entries a CE write dirtied since last sync */
    int      m2_cpt_dirty_n;    /* live entries in m2_cpt_dirty (0 = clean) */
    uint32_t m2_last_db_token;  /* M5.11: last guest work-submit token seen at the doorbell (dedup log) */
    bool     m2_last_db_valid;
    uint32_t m2_gr_client;      /* M5.7: the GR compute client (set at crashwin arm) */
    /* #14: ALL user GR compute clients (one per guest process — every client that
     * allocs an AMPERE_COMPUTE_B 0xc7c0).  m2_gr_client stays the FIRST (legacy
     * single-process scalar, unchanged); the sweep/ring/backing paths iterate this
     * list so a 2nd process's client is a peer, not invisible.  Entries drop on
     * root client free (RM reuses handle values across process lifetimes). */
    uint32_t m2_gr_clients[16];   /* #14 P0: 8->16 for N procs */
    int      m2_gr_clients_n;
    /* #14 EARLY-ARM: user compute clients derived from DUP_OBJECT SRC (the UVM
     * handover dups the user's VASpace OUT of libcuda's client), recorded at the
     * fn=21 snoop — which lands at cuCtxCreate's UVM-registration step, BEFORE the
     * client's channels, working-set mappings, or 0xc7c0 GR object exist (bench
     * log: both processes' dup edges precede even the FIRST 0xc7c0).  The 0xc7c0-
     * keyed m2_gr_clients[] list above flips the multiproc gate only after BOTH
     * processes alloc their GR object — by then the shared single-process state
     * has already aliased the two processes (the round-3 transition-window wall).
     * This list arms nvkvm_m2_multiproc() at the 2nd distinct user client's dup,
     * before any aliasing is possible.  It feeds ONLY multiproc()-gated behavior
     * (pass-1 refusal, foreign-VAS skips, per-owner backing), so with a single
     * process (one dup-src client, incl. #12's client-reusing 2-context case) it
     * is inert and behavior is byte-identical.  Entries drop on root client free
     * (RM reuses handle values across process lifetimes). */
    uint32_t m2_user_clients[16]; /* #14 P0: 8->16 for N procs */
    int      m2_user_clients_n;
    /* #14 piece-2: deferred completion-retry kick.  Set while servicing a guest
     * MC_SERVICE_INTERRUPTS poll (fn=76 ctrl 0x20801702) in multiproc mode;
     * consumed at the end of service_cmdq (after the RPC response is posted +
     * acked) by replaying the work-submit doorbell service.  See the hook site. */
    bool     m2_poll_kick;
    /* M5.7: per-client NV01_MEMORY_VIRTUAL mapper over the client's GR VASpace, so the
     * execution path can map_dma FIXED the guest's working-set buffers into the host
     * channel's address space at the guest VAs (see [[mode2-mapdma-primitive]]). */
    struct { uint32_t client, hvirt, hvas, hdev; } m2_grmap[32]; /* #14 P0: 8->32 for N procs */
    int      m2_grmap_n;
    /* M5.49b: libcuda's CE-copy clients (the user-observable data path, e.g. the
     * cuMemcpyHtoD/DtoH that produce rv).  Identified as the clients that hit the
     * M5.20 grmapper FRESH-VAS fallback (their forwarded VAS is RM-rejected) — UVM/
     * CeUtils clients take the st==0 path and are NOT recorded, the GR client returns
     * early via its cvas.  Used to force HOST-only completion for ONLY the user CE
     * round-trip while UVM kernel-internal scrubs keep their simulated completion. */
    uint32_t m2_user_ce_clients[16];
    int      m2_user_ce_n;
    /* M5.28 PER-CHANNEL VAS (user-directed): each forwarded GR channel runs in its OWN
     * fresh nvkvm-allocated VAS (FERMI_VASPACE_A under the channel's forwarded device),
     * NOT the guest's forwarded VAS (0xcaf00005) — that one the host RM auto-promoted its
     * GR ctx into, so the guest's working-set VAs collide (st=0x51 / Xid 32). Keyed by the
     * parent TSG handle (all channels in a TSG share its VAS). The working set is mapped
     * into THIS vas (fvirt over fvas) instead of the per-client grmapper. m2_cur_cvas is the
     * active index for the current map ops (set per-channel in the doorbell loop; -1 = use
     * the legacy per-client grmapper, e.g. CeUtils). */
    struct { uint32_t client, tsg, hdev, fvas, fvirt; bool populated; } m2_cvas[64]; /* #14 P0: 16->64 for N procs */
    int      m2_cvas_n;
    int      m2_cur_cvas;
    uint32_t m2_gr_channel;     /* M5.8: the host GR channel handle (c56f under GR TSG) */
    uint32_t m2_gr_tsg;         /* M5.8: the host GR TSG handle (a06c, channel's parent) */
    /* #12 cont.34 / #14: which GR TSGs we GPFIFO_SCHEDULE'd, keyed (client, tsg).
     * doorbell_setup schedules the first GR TSG once, but early-returns on
     * m2_doorbell_ready (sticky), so a 2nd context's NEW GR TSG was never scheduled
     * -> its 8 rl=0 GR channels rang but the host never consumed (gp_get stuck 0)
     * -> 8/16 pool semas never advanced -> #12 residual.  The ring loop schedules
     * any GR TSG not in this set.  #14: the key MUST include the client — two
     * concurrent processes get IDENTICAL RM handle values (both GR TSGs 0x5c000012),
     * so the old value-keyed scalar made process B's GR TSG look already-scheduled
     * and it sat off-runlist forever.  Entries drop on client/TSG free (handle
     * values are reused across contexts and process lifetimes). */
    struct { uint32_t client, tsg; } m2_tsg_sched[16];
    int      m2_tsg_sched_n;
    /* M7 (cuCtxCreate fix): the HOST's real GR-object alloc reply params (NV_GR_ALLOCATION_
     * PARAMETERS, 16B incl the GSP-filled `caps` output @+12). Captured by shadow_fwd after the
     * forwarded 0xc7c0 alloc, then passed through into the GSP-RPC reply (resp+112) so the guest
     * copies the REAL caps back instead of the echoed request (caps=0). The auditor proved the
     * guest copies the LOCAL class size (16B) from resp+112 ignoring reply paramsSize, so the
     * old M5.3 force-paramsSize->0 was moot; forwarding the real params is the correct fix. */
    uint8_t  m2_gr_reply[64];
    uint32_t m2_gr_reply_obj;   /* hObject this reply belongs to (match in the reply builder) */
    uint32_t m2_gr_reply_psize; /* host's RETURNED alloc_parms_size (paramsSize the real RM wrote) */
    bool     m2_gr_reply_valid;
    void    *m2_usermode_qva;   /* M5.8: mmap of host AMPERE_USERMODE_A doorbell page */
    uint32_t m2_gr_token;       /* M5.8: host GR channel work-submit token (doorbell value) */
    bool     m2_doorbell_ready; /* M5.8: usermode mapped + token fetched */
    uint64_t m2semval;          /* M5.14 DIAG prop: if nonzero, fb_read of m2sempage returns this
                                 * (satisfy the guest-kernel post-PROMOTE_CTX ctx-completion poll
                                 * that nothing writes in the fake-GSP model; userspace never
                                 * observes it). 0 = disabled. */
    uint64_t m2sempage;         /* M5.14: guest-FB page (4K-aligned) the sentinel applies to */
    /* M5.27: VAs already backed+mapped (dedup pushbuffer/sema/gpfifo maps).  Was 128 — the
     * compute working set (30 GP entries x ~8 channels of pushbuffers + semas + gpfifos) blows
     * past that, and once full the dedup silently STOPPED recording, so every VA re-mapped on
     * EVERY doorbell -> dmaAllocMapping flood + a leaked host mem object per re-map + host VAS
     * exhaustion -> legitimate buffer maps then failed -> the host GPU stalled mid-channel
     * (GP_GET stuck). Size it well past any cuCtxCreate/matmul working set. */
#define NVKVM_MAX_MAPPED_VA 65536
    /* M5.34: dedup key is (client, va) NOT va alone.  Host VASpaces are PER-CLIENT
     * (m2_devvas[] selects {dev,vas} by client), so a VA placed into client A's VAS
     * is NOT present in client B's VAS.  A va-only dedup wrongly skipped placing a
     * shared pushbuffer (e.g. 0x120000000) into the CE-scrubber's VAS after another
     * client claimed the slot first -> host GR0_PBDMA0 MMU FAULT_PTE -> channel halt. */
    /* #12 cont.31: sysmem-backed entries also record the GUEST GPA the VA resolved to at
     * back time + the host OS-descriptor handle pinning that page.  A later sweep that
     * resolves the SAME {client,VA} to a DIFFERENT GPA means the guest tore down and
     * re-created the mapping (2nd cuCtxCreate re-allocs channels/sema-pools at the same
     * VAs but fresh pages) — the host GR-VAS then still targets the OLD page, so host
     * completion-semaphore writes land in stale memory (libcuda's 16-sema pool at VA
     * 0x20440f000 reads 0 forever = the #12 hang; the stale writes also corrupt whoever
     * reuses the old page = the UVM MAX_JUMP asserts).  gpa==0 => legacy mark (no
     * staleness semantics: vidmem chunks, pushbuffer marks).  reback counts re-backs
     * (ping-pong guard). */
    struct { uint32_t client; uint64_t va; uint64_t gpa; uint32_t hmem;
             uint16_t reback; } m2_mapped_va[NVKVM_MAX_MAPPED_VA];
    int      m2_mapped_va_n;
    /* M6.0 (item-4 prereq): guest RAM as a shared memfd, so the STUB can mmap any guest GPA
     * and OS_DESCRIPTOR-register it -> the host GPU can DMA into the guest's sysmem GR buffers
     * (the un-backed objects libcuda reads, [[mode2-cuctxcreate-pagetable-poll]]). Found at
     * realize from the largest fd-backed RAMBlock (memory-backend-memfd,share=on). */
    int      m2_guest_ram_fd;   /* memfd fd of guest RAM (-1 if not memfd-backed) */
    void    *m2_guest_ram_hva;  /* QEMU host VA of guest RAM base */
    uint64_t m2_guest_ram_size;
    /* M6.1 (item-4 step 2): the guest-RAM memfd shared into the STUB. handle_table = the
     * Mode-1 fd registry ([[mode2-per-proc-isolate-handle-reuse]]) so we can send_handle +
     * isolate_mmap. The stub MAP_FIXEDs guest RAM at m2_stub_ram_base; for a guest GPA G the
     * stub VA is m2_stub_ram_base+G -> OS_DESCRIPTOR there for host-GPU DMA (item-4 step 3). */
    struct nvkvm_handle_table m2_ht;
    uint32_t m2_guest_ram_handle;
    uint64_t m2_stub_ram_base;  /* stub VA where guest RAM is MAP_FIXED (0 = not shared) */
    bool     m2_ram_shared;
    bool     m2_gpu_registered;  /* M6.2: m2_gpu_h REGISTER_FD'd to the ctl session */

    /* M7 REFACTOR (user-directed) — the proper memory model that replaces fb_pages/m2_fbback:
     * gpu_memory_object = ONE real backing (host RM alloc, double-mmapped: cpu_qva for the
     * guest-CPU/QEMU view, gr_va for the host-GPU view via the host GR VAS). The GPGA page
     * table maps a guest-GPU-physical range -> (object, offset). The SAME nvkvm/RM handle backs
     * both views, so host GPU and guest CPU are coherent. See docs/design/mode2_dataplane_
     * architecture.md "REFACTOR PLAN". m2_fbback stays as the legacy fallback until retired. */
    struct {
        uint8_t  mode;        /* 0=physical(FB-backed general), 1=special(reg page) */
        void    *cpu_qva;     /* QEMU/guest-CPU mapping of the host object (NULL=none) */
        uint64_t size;
        uint32_t client;      /* host RM client (nvkvm-tracked) */
        uint32_t hMemory;     /* host RM object handle = the 'real' backing */
        uint64_t gr_va;       /* host GR-VAS VA where map_dma'd (0=not GPU-mapped) */
        uint8_t  promote;     /* CE-fwd map-on-touch: 0=normal, 1=gpu_only(promotable on
                               * first CPU touch -> RM_MAP_MEMORY same hMem), 2=promotion
                               * given up (BAR1 full at touch; serve fb_pages, no retry) */
    } m2_objs[1024];            /* M5.48d: 2-MiB chunked mirror needs headroom (was 128) */
    int      m2_objs_n;
    struct {                  /* GPGA page-range -> (object, offset_in_target) */
        uint64_t gpga_base, size;
        int      obj_idx;     /* index into m2_objs[] (-1 = none) */
        uint64_t off;         /* offset_in_target */
        bool     readable, writable;
    } m2_gpga[2048];            /* M5.48d: 2-MiB chunked mirror needs headroom (was 256) */
    int      m2_gpga_n;
    /* M5.11c PERF: sorted-by-base index over m2_gpga[] so the hot fb_host_overlay lookup is a
     * binary search (log n) instead of an O(n~430) linear scan that dominated exec_doorbell
     * (60M fruitless iters/window). Ranges are non-overlapping (one obj per GPGA range). Lazily
     * rebuilt when m2_gpga_n changes; mark dirty at the single insert site. */
    uint16_t m2_gpga_sorted[2048];  /* gpga indices, ascending by gpga_base */
    int      m2_gpga_sorted_n;      /* count built into m2_gpga_sorted */
    bool     m2_gpga_idx_dirty;
    uint64_t m2_gpga_idx_audit;     /* lookups remaining to cross-check vs the old linear scan */
    uint64_t m2_gpga_idx_mismatch;  /* audit: binary-search vs linear-scan disagreements (must be 0) */

    /* knobs */
    bool     trace;          /* log every BAR0 access                        */
    uint64_t access_count;   /* monotonically increasing, for the trace      */

    /* ── #90: the §6 replay-trace recorder ────────────────────────────────
     * A NEW property, deliberately NOT a reuse of m2trace: m2trace is not
     * observationally neutral (it sets m2_gpga_idx_audit=3000000, sets
     * m2_crashwin, and causes two extra nvkvm_fb_read calls), so turning it on
     * changes what the device DOES.  m2rec only observes. */
    bool     m2rec;          /* enable the recorder                          */
    char    *m2recfile;      /* output path (default /tmp/m0_rec.bin)        */
    uint64_t m2recmask;      /* the DECLARED FILTER, NVKVM_REC_M_*           */
    Notifier m2rec_exit;     /* flush a killed QEMU's dense prefix           */
};

/* ── #90: §6 replay-trace emit helpers ─────────────────────────────────────
 *
 * Every one of these is a no-op (one predictable branch on a file-static bool)
 * when the recorder is off.  There are NO counter caps here — see R2 in
 * nvkvm_m2_rec.h: the consumer's differential compares stream POSITIONS, so a
 * cap does not shorten a trace, it corrupts every position after it.
 *
 * ★ Which BAR index each region reports:
 *     bar=0  the BAR0 register aperture (incl. the m2romregs falcon overlay)
 *     bar=1  PCI BAR1 — the FB aperture ("BAR1" in RM terms)
 *     bar=3  PCI BAR3 — the 32 MiB GPU-virtual window ("BAR2" in RM terms)
 *   i.e. the PCI BAR index, not RM's naming, because that is what a replay
 *   harness sees trapping. */

/* The exact QEMU_CLOCK_VIRTUAL sample the last PTIMER read was derived from.
 * Stashed by nvkvm_reg_read so the Clock record carries the SAME ns the served
 * value came from, rather than a second, later sample. */
static uint64_t g_nvkvm_rec_ptimer_ns;

/* Region bit for a BAR0 offset — the declared filter's second axis. */
static inline uint64_t nvkvm_rec_bar0_region(hwaddr off)
{
    if (off >= NV_PROM_DATA_BASE && off < NV_PROM_DATA_BASE + NV_PROM_DATA_SIZE) {
        return NVKVM_REC_M_PROM;
    }
    if (off >= NVKVM_PRAMIN_BASE && off < NVKVM_PRAMIN_BASE + NVKVM_PRAMIN_SIZE) {
        return NVKVM_REC_M_PRAMIN;
    }
    if (off == NV_PTIMER_TIME_0_GA10X || off == NV_PTIMER_TIME_1_GA10X) {
        return NVKVM_REC_M_PTIMER;
    }
    return NVKVM_REC_M_BAR0;
}

static inline void nvkvm_rec_mmio(uint8_t kind, uint64_t kindbit, uint8_t bar,
                                  uint64_t region, hwaddr off, unsigned size,
                                  uint64_t val)
{
    if (!nvkvm_rec_on() || !(nvkvm_rec_mask() & region)) {
        return;
    }
    nvkvm_rec_emit(kindbit, kind, bar, (uint8_t)size, (uint64_t)off, val, NULL, 0);
}

static inline void nvkvm_rec_mmio_rd(uint8_t bar, uint64_t region, hwaddr off,
                                     unsigned size, uint64_t val)
{
    nvkvm_rec_mmio(NVKVM_REC_MMIO_RD, NVKVM_REC_M_MMIO_RD, bar, region, off,
                   size, val);
}

static inline void nvkvm_rec_mmio_wr(uint8_t bar, uint64_t region, hwaddr off,
                                     unsigned size, uint64_t val)
{
    nvkvm_rec_mmio(NVKVM_REC_MMIO_WR, NVKVM_REC_M_MMIO_WR, bar, region, off,
                   size, val);
}

/* GAP-I6: the FULL payload, not the first 8 bytes.  The 4096-byte queue
 * elements ARE the GSP reply protocol; recording v0 recorded nothing. */
static inline void nvkvm_rec_guest_wr(uint64_t gpa, const void *buf, uint64_t len)
{
    if (!nvkvm_rec_on()) {
        return;
    }
    nvkvm_rec_emit(NVKVM_REC_M_GUEST_WR, NVKVM_REC_GUEST_WR, 0xFF, 0, gpa, 0,
                   buf, (uint32_t)len);
}

static inline void nvkvm_rec_guest_rd(uint64_t gpa, const void *buf, uint64_t len)
{
    if (!nvkvm_rec_on()) {
        return;
    }
    nvkvm_rec_emit(NVKVM_REC_M_GUEST_RD, NVKVM_REC_GUEST_RD, 0xFF, 0, gpa, 0,
                   buf, (uint32_t)len);
}

/* a=0 -> MSI-X with b=vector; a=1 -> legacy INTx with b=level.  Matches the
 * rewrite's IrqSpec::{Msix(u16), IntxLevel(bool)}. */
static inline void nvkvm_rec_irq_msix(uint32_t vec)
{
    nvkvm_rec_emit(NVKVM_REC_M_IRQ, NVKVM_REC_IRQ, 0xFF, 0, 0, vec, NULL, 0);
}

static inline void nvkvm_rec_irq_intx(int level)
{
    nvkvm_rec_emit(NVKVM_REC_M_IRQ, NVKVM_REC_IRQ, 0xFF, 0, 1, (uint64_t)level,
                   NULL, 0);
}

/* ── BAR0 register aperture ────────────────────────────────────────────────*/

static const char *nvkvm_reg_name(hwaddr off)
{
    switch (off) {
    case NV_PMC_BOOT_0:  return "PMC_BOOT_0";
    case NV_PMC_BOOT_1:  return "PMC_BOOT_1";
    case NV_PMC_BOOT_42: return "PMC_BOOT_42";
    case NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK: return "GFW_BOOT_PLM";
    case NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT:      return "GFW_BOOT";
    case NV_PGSP_FALCON_CPUCTL:                              return "GSP_CPUCTL";
    case NV_PGSP_FALCON_HWCFG2:                              return "GSP_HWCFG2";
    case NV_PTIMER_TIME_0_GA10X:                            return "PTIMER_TIME_0";
    case NV_PTIMER_TIME_1_GA10X:                            return "PTIMER_TIME_1";
    case NV_PTIMER_TIME_PRIV_LEVEL_MASK:                    return "PTIMER_PLM";
    case NV_PGSP_FALCON_DMATRFCMD:                          return "GSP_DMATRFCMD";
    case NV_PSEC_FALCON_DMATRFCMD:                          return "SEC_DMATRFCMD";
    case NV_PSEC_FALCON_CPUCTL:                             return "SEC_CPUCTL";
    default:             return NULL;
    }
}

/* M6: sparse FB backing + BAR0 PRAMIN window (offsets in mode2_regs_ga10x.h) */

/* FB address that PRAMIN+off currently maps to: BASE[23:0]<<16 + window offset. */
static uint64_t nvkvm_pramin_fb_addr(NvkvmGpuEmul *s, hwaddr off)
{
    uint64_t base = (uint64_t)(s->bar0_window & 0x00FFFFFFu) << 16;
    return base + (off - NVKVM_PRAMIN_BASE);
}

static uint8_t *nvkvm_fb_page(NvkvmGpuEmul *s, uint64_t fb_addr, bool alloc)
{
    gpointer key = (gpointer)(uintptr_t)(fb_addr >> 12);
    uint8_t *p = g_hash_table_lookup(s->fb_pages, key);
    if (!p && alloc) {
        p = g_malloc0(4096);
        g_hash_table_insert(s->fb_pages, key, p);
    }
    return p;
}

/* M5.3 DATA-PLANE: if fb_addr falls in a range backed by real host GPU memory
 * (double-mmap), return the host VA for that byte; else NULL (use local FB page).
 * Inert until m2_fbback[] is populated. */
/* M5.15 DIAG: log every device->guest DMA write while the crash-window is armed, to catch the
 * mistranslated pci_dma_write that zeroes libcuda's saved-rbp slot (the cuCtxCreate rbp=0 SIGSEGV).
 * Set from realize; gated on m2_crashwin so it only fires after the 0xc7c0 GR alloc. */
static NvkvmGpuEmul *g_nvkvm_dma_s;
static int g_nvkvm_dma_logs;
static MemTxResult nvkvm_dmaw(PCIDevice *dev, dma_addr_t gpa, const void *buf, dma_addr_t len)
{
    if (g_nvkvm_dma_s && g_nvkvm_dma_s->m2_crashwin && g_nvkvm_dma_logs < 200000) {
        g_nvkvm_dma_logs++;
        uint64_t v0 = (len >= 8) ? ldq_le_p(buf) : (len >= 4 ? ldl_le_p(buf) : 0);
        void *caller = __builtin_return_address(0);
        qemu_log("nvkvm-gpu[GA106] M5.15 DMAW gpa=0x%llx len=%llu v0=0x%llx site=%p\n",
                 (unsigned long long)gpa, (unsigned long long)len, (unsigned long long)v0, caller);
    }
    /* #90 GAP-I6: record the WHOLE payload.  The old DIAG above keeps only the
     * first 8 bytes, which throws away exactly the thing that matters — the
     * 4096-byte GSP queue elements are the reply protocol. */
    nvkvm_rec_guest_wr(gpa, buf, len);
    return pci_dma_write(dev, gpa, buf, len);
}

/* #90 GAP-I7: the guest-RAM READ chokepoint the C never had.
 *
 * All 8 pci_dma_read sites go through here, so a replay can answer a DMA read
 * — without which no trace is hermetic (§6.1).  The bytes recorded are the
 * bytes RETURNED, and nothing is recorded when the read failed: a failed read
 * returned no bytes, and inventing a record for it would put a phantom in the
 * stream.  This wrapper mirrors nvkvm_dmaw exactly and changes no behaviour. */
static MemTxResult nvkvm_dmar(PCIDevice *dev, dma_addr_t gpa, void *buf,
                              dma_addr_t len)
{
    MemTxResult r = pci_dma_read(dev, gpa, buf, len);
    if (r == MEMTX_OK) {
        nvkvm_rec_guest_rd(gpa, buf, len);
    }
    return r;
}

/* NVKVM-DPLANE (cup6 diag): quantify where the bulk cuMemcpyHtoD 64MB actually
 * lands. (i) CE LAUNCH_DMA dest resolution: real m2_fbback / m2_gpga backing vs a
 * fake g_malloc0 fb_page fallback. (ii) which path moves the bytes: the emulated CE
 * byte-copy loop, or kernel/CPU (BAR1) writes into fb_pages. Pure logging; no
 * behavior change. Dumped via nvkvm_dplane_summary() after any CE copy >= 1MB. */
static uint64_t nvkvm_dp_ce_launchdma_calls;
static uint64_t nvkvm_dp_ce_bytes_total;      /* sum of `bytes` over LAUNCH_DMA */
static uint64_t nvkvm_dp_ce_dst_fbback_hits;
static uint64_t nvkvm_dp_ce_dst_gpga_hits;
static uint64_t nvkvm_dp_ce_dst_fbpage_fallback;
static uint64_t nvkvm_dp_fbpage_write_bytes;      /* bytes into a g_malloc0 fb_page */
static uint64_t nvkvm_dp_overlay_real_write_bytes;/* bytes into real fbback/gpga backing */

/* M5.11 PERF time-share: wall-clock (host REALTIME) ns + call counts in each Mode-2
 * hot path, so we can see where a token's latency actually goes before building the
 * CE-forward data path. Leaf regions (ce_emul copy loop, fb_read/fb_write window
 * traps, the doorbell re-sweep walk) are non-overlapping; doorbell/chan_exec are the
 * enclosing forwards (their ns INCLUDES any sweep done inside, reported separately so
 * the caller can subtract). Pure measurement; dumped by nvkvm_timeshare_dump(). The
 * per-call qemu_clock_get_ns() is a clock_gettime — ~tens of ns, negligible vs a
 * ~1.6s/token run even at 100k window traps. */
static uint64_t nvkvm_t_run_start_ns;             /* first hot-path touch (lazy) */
static uint64_t nvkvm_t_ce_emul_ns,   nvkvm_t_ce_emul_calls;   /* emulated-CE LAUNCH_DMA byte copy */
static uint64_t nvkvm_t_win_rd_ns,    nvkvm_t_win_rd_calls;    /* guest-CPU PRAMIN-window read trap */
static uint64_t nvkvm_t_win_wr_ns,    nvkvm_t_win_wr_calls;    /* guest-CPU PRAMIN-window write trap */
static uint64_t nvkvm_t_sweep_ns,     nvkvm_t_sweep_calls;     /* doorbell GR-VAS re-sweep walk */
static uint64_t nvkvm_t_doorbell_ns,  nvkvm_t_doorbell_calls;  /* whole exec_doorbell WALL time (incl. sweep) */
static uint64_t nvkvm_t_doorbell_cpu_ns;                       /* exec_doorbell THREAD-CPU time (vs wall = desched) */
static uint64_t nvkvm_t_chan_exec_ns, nvkvm_t_chan_exec_calls; /* whole chan_execute (incl. doorbell) */
static uint64_t nvkvm_t_event_ns,     nvkvm_t_event_calls;     /* nvkvm_gsp_deliver_events (os-event wake) */
static uint64_t nvkvm_t_resolve_ns,   nvkvm_t_resolve_calls;   /* nvkvm_m2_resolve_fb GMMU walk */
static uint64_t nvkvm_t_fbrd_calls;                            /* nvkvm_fb_read total calls */
static uint64_t nvkvm_t_overlay_iters;                         /* fbback+gpga linear-scan iterations (O(n) cost) */
static uint64_t nvkvm_t_vaseen_iters, nvkvm_t_vaseen_calls;    /* m2_mapped_va[] linear-scan (va_seen/va_check) */
static uint64_t nvkvm_t_backmap_ns,   nvkvm_t_backmap_calls;   /* nvkvm_m2_back_and_map (host RM ioctls) */
static uint64_t nvkvm_t_fbrd_ov_ns,   nvkvm_t_fbrd_ov_calls;   /* fb_read served from host overlay (GPU-BAR?) */
static uint64_t nvkvm_t_fbrd_pg_ns,   nvkvm_t_fbrd_pg_calls;   /* fb_read served from fb_page (host RAM) */
static inline uint64_t nvkvm_now_ns(void) { return qemu_clock_get_ns(QEMU_CLOCK_REALTIME); }
/* M5.11e: per-thread CPU time (NOT wall) — db_cpu << db_wall would prove exec_doorbell is
 * descheduling-bound (host preempts the BQL/vCPU thread mid-trap) rather than CPU-work-bound. */
static inline uint64_t nvkvm_now_cpu_ns(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}
static inline void nvkvm_t_mark_start(void) { if (!nvkvm_t_run_start_ns) nvkvm_t_run_start_ns = nvkvm_now_ns(); }

/* Classify an FB address the SAME way nvkvm_fb_host_overlay does, WITHOUT touching
 * the copy: 1=fbback, 2=gpga(real obj), 0=fb_page fallback (incl. gpga-known-no-obj). */
static int nvkvm_dp_classify_fb(NvkvmGpuEmul *s, uint64_t fb_addr)
{
    for (int i = 0; i < s->m2_fbback_n; i++) {
        if (fb_addr >= s->m2_fbback[i].fb_base &&
            fb_addr <  s->m2_fbback[i].fb_base + s->m2_fbback[i].size) {
            return 1;
        }
    }
    for (int i = 0; i < s->m2_gpga_n; i++) {
        if (fb_addr >= s->m2_gpga[i].gpga_base &&
            fb_addr <  s->m2_gpga[i].gpga_base + s->m2_gpga[i].size) {
            int oi = s->m2_gpga[i].obj_idx;
            if (oi < 0 || oi >= s->m2_objs_n) {
                break;                 /* gpga known but no object -> fb_pages */
            }
            /* CE-fwd map-on-touch: a gpu_only object has cpu_qva==NULL but IS host-reachable
             * (real host vidmem + map_dma) -> still "backed" (return 2). Only gpu_only objects
             * ever have cpu_qva==NULL, so a valid obj_idx always means real host backing. This
             * keeps the M5.60 re-back gate from redundantly re-backing a gpu_only dst. */
            return 2;
        }
    }
    return 0;
}

static void nvkvm_dplane_summary(NvkvmGpuEmul *s, const char *why)
{
    qemu_log("nvkvm-gpu[%s] NVKVM-DPLANE SUMMARY (%s): ce_launchdma_calls=%llu "
             "ce_bytes_total=%llu dst_fbback_hits=%llu dst_gpga_hits=%llu "
             "dst_fbpage_fallback=%llu | fbpage_write_bytes=%llu "
             "overlay_real_write_bytes=%llu\n", s->chip->name, why,
             (unsigned long long)nvkvm_dp_ce_launchdma_calls,
             (unsigned long long)nvkvm_dp_ce_bytes_total,
             (unsigned long long)nvkvm_dp_ce_dst_fbback_hits,
             (unsigned long long)nvkvm_dp_ce_dst_gpga_hits,
             (unsigned long long)nvkvm_dp_ce_dst_fbpage_fallback,
             (unsigned long long)nvkvm_dp_fbpage_write_bytes,
             (unsigned long long)nvkvm_dp_overlay_real_write_bytes);
}

/* M5.11 PERF: dump the time-share buckets. `elapsed` = wall ns since first hot-path
 * touch; each bucket is reported as ns, calls, and % of elapsed so the dominant cost
 * is obvious at a glance. doorbell/chan_exec include nested sweep/doorbell time, so
 * "doorbell-only" = doorbell_ns - sweep_ns is also printed. */
static void nvkvm_timeshare_dump(NvkvmGpuEmul *s, const char *why)
{
    uint64_t now = nvkvm_now_ns();
    uint64_t elapsed = nvkvm_t_run_start_ns ? (now - nvkvm_t_run_start_ns) : 1;
    if (!elapsed) elapsed = 1;
#define NVKVM_PCT(x) ((double)(x) * 100.0 / (double)elapsed)
    uint64_t db_only = nvkvm_t_doorbell_ns > nvkvm_t_sweep_ns
                     ? nvkvm_t_doorbell_ns - nvkvm_t_sweep_ns : 0;
    qemu_log("nvkvm-gpu[%s] NVKVM-TIMESHARE (%s): elapsed=%llums | "
             "ce_emul=%llums/%lluc(%.1f%%) win_rd=%llums/%lluc(%.1f%%) "
             "win_wr=%llums/%lluc(%.1f%%) sweep=%llums/%lluc(%.1f%%) "
             "doorbell=%llums/%lluc(%.1f%%) [db_only=%llums/%.1f%%] "
             "chan_exec=%llums/%lluc(%.1f%%)\n", s->chip->name, why,
             (unsigned long long)(elapsed / 1000000ull),
             (unsigned long long)(nvkvm_t_ce_emul_ns / 1000000ull),
             (unsigned long long)nvkvm_t_ce_emul_calls, NVKVM_PCT(nvkvm_t_ce_emul_ns),
             (unsigned long long)(nvkvm_t_win_rd_ns / 1000000ull),
             (unsigned long long)nvkvm_t_win_rd_calls, NVKVM_PCT(nvkvm_t_win_rd_ns),
             (unsigned long long)(nvkvm_t_win_wr_ns / 1000000ull),
             (unsigned long long)nvkvm_t_win_wr_calls, NVKVM_PCT(nvkvm_t_win_wr_ns),
             (unsigned long long)(nvkvm_t_sweep_ns / 1000000ull),
             (unsigned long long)nvkvm_t_sweep_calls, NVKVM_PCT(nvkvm_t_sweep_ns),
             (unsigned long long)(nvkvm_t_doorbell_ns / 1000000ull),
             (unsigned long long)nvkvm_t_doorbell_calls, NVKVM_PCT(nvkvm_t_doorbell_ns),
             (unsigned long long)(db_only / 1000000ull), NVKVM_PCT(db_only),
             (unsigned long long)(nvkvm_t_chan_exec_ns / 1000000ull),
             (unsigned long long)nvkvm_t_chan_exec_calls, NVKVM_PCT(nvkvm_t_chan_exec_ns));
#undef NVKVM_PCT
}

/* M5.11b PERF window dump: per-WINDOW deltas (vs the cumulative dump, which is dominated by
 * the model-load phase). Keyed to doorbell count from the exec_doorbell timing site so it
 * samples steady-state GENERATION evenly. Decisive split for the 22->60 tok/s lever:
 *   inside_qemu = (exec_doorbell + chan_execute + deliver_events) wall time in the window
 *                 = the SUBMIT-side cost the vCPU is blocked in QEMU for.
 *   outside     = wall - inside = guest-side launch overhead + libcuda's completion SPIN
 *                 (cuStreamSynchronize waiting for the sema). If `outside` dominates while
 *                 the host GPU is idle, the cost is completion-visibility latency (H2), not
 *                 QEMU submit cost. event_us isolates the os-event wake path specifically. */
static uint64_t nvkvm_tw_last_ns, nvkvm_tw_db_ns, nvkvm_tw_db_calls, nvkvm_tw_db_cpu_ns, nvkvm_tw_cx_ns,
                nvkvm_tw_sweep_ns, nvkvm_tw_event_ns, nvkvm_tw_event_calls,
                nvkvm_tw_winrd_calls, nvkvm_tw_winwr_calls, nvkvm_tw_ce_calls,
                nvkvm_tw_resolve_ns, nvkvm_tw_resolve_calls, nvkvm_tw_fbrd_calls,
                nvkvm_tw_overlay_iters, nvkvm_tw_vaseen_iters, nvkvm_tw_vaseen_calls,
                nvkvm_tw_backmap_ns, nvkvm_tw_backmap_calls,
                nvkvm_tw_fbrd_ov_ns, nvkvm_tw_fbrd_ov_calls,
                nvkvm_tw_fbrd_pg_ns, nvkvm_tw_fbrd_pg_calls;
static void nvkvm_timeshare_window_dump(NvkvmGpuEmul *s, const char *why)
{
    uint64_t now = nvkvm_now_ns();
    uint64_t wall = nvkvm_tw_last_ns ? (now - nvkvm_tw_last_ns) : 1;
    if (!wall) wall = 1;
    uint64_t d_db  = nvkvm_t_doorbell_ns   - nvkvm_tw_db_ns;
    uint64_t d_dbc = nvkvm_t_doorbell_calls - nvkvm_tw_db_calls;
    uint64_t d_dbcpu = nvkvm_t_doorbell_cpu_ns - nvkvm_tw_db_cpu_ns;
    uint64_t d_cx  = nvkvm_t_chan_exec_ns  - nvkvm_tw_cx_ns;
    uint64_t d_sw  = nvkvm_t_sweep_ns      - nvkvm_tw_sweep_ns;
    uint64_t d_ev  = nvkvm_t_event_ns      - nvkvm_tw_event_ns;
    uint64_t d_evc = nvkvm_t_event_calls   - nvkvm_tw_event_calls;
    uint64_t d_wrd = nvkvm_t_win_rd_calls  - nvkvm_tw_winrd_calls;
    uint64_t d_wwr = nvkvm_t_win_wr_calls  - nvkvm_tw_winwr_calls;
    uint64_t d_ce  = nvkvm_dp_ce_launchdma_calls - nvkvm_tw_ce_calls;
    uint64_t d_rsn = nvkvm_t_resolve_ns    - nvkvm_tw_resolve_ns;
    uint64_t d_rsc = nvkvm_t_resolve_calls - nvkvm_tw_resolve_calls;
    uint64_t d_fbr = nvkvm_t_fbrd_calls    - nvkvm_tw_fbrd_calls;
    uint64_t d_ovl = nvkvm_t_overlay_iters - nvkvm_tw_overlay_iters;
    uint64_t d_vsi = nvkvm_t_vaseen_iters  - nvkvm_tw_vaseen_iters;
    uint64_t d_vsc = nvkvm_t_vaseen_calls  - nvkvm_tw_vaseen_calls;
    uint64_t d_bmn = nvkvm_t_backmap_ns    - nvkvm_tw_backmap_ns;
    uint64_t d_bmc = nvkvm_t_backmap_calls - nvkvm_tw_backmap_calls;
    uint64_t d_ovn = nvkvm_t_fbrd_ov_ns    - nvkvm_tw_fbrd_ov_ns;
    uint64_t d_ovc = nvkvm_t_fbrd_ov_calls - nvkvm_tw_fbrd_ov_calls;
    uint64_t d_pgn = nvkvm_t_fbrd_pg_ns    - nvkvm_tw_fbrd_pg_ns;
    uint64_t d_pgc = nvkvm_t_fbrd_pg_calls - nvkvm_tw_fbrd_pg_calls;
    uint64_t inside = d_db + d_cx + d_ev;
    /* M5.11c: emit to stderr (NOT qemu_log) so this measurement survives running with `-d` removed
     * — that lets us A/B the per-doorbell DIAG log spew (qemu_log off) vs on using the
     * variance-robust db-us/doorbell metric instead of the ±40%-noisy end-to-end t/s. */
    fprintf(stderr, "nvkvm-gpu[%s] NVKVM-TWIN (%s): wall=%llums dbells=%llu ce=%llu | "
             "INSIDE_qemu=%.1f%% (db=%llums[cpu=%llums]/%lluus_per chan_exec=%llums event=%llums/%lluc) "
             "sweep=%llums | resolve=%llums/%lluc fbrd=%lluc overlay_iters=%llu(%llu/fbrd) "
             "gpga_n=%d idx_mismatch=%llu audit_left=%llu "
             "vaseen=%lluit/%lluc(%llu/call,n=%d) backmap=%llums/%lluc "
             "fbrd_BAR=%llums/%lluc(%lluns) fbrd_RAM=%llums/%lluc(%lluns) "
             "winrd=%lluc winwr=%lluc | OUTSIDE(guest+spin)=%.1f%%\n",
             s->chip->name, why,
             (unsigned long long)(wall/1000000ull), (unsigned long long)d_dbc,
             (unsigned long long)d_ce,
             (double)inside*100.0/(double)wall,
             (unsigned long long)(d_db/1000000ull),
             (unsigned long long)(d_dbcpu/1000000ull),
             (unsigned long long)(d_dbc ? d_db/1000ull/d_dbc : 0),
             (unsigned long long)(d_cx/1000000ull),
             (unsigned long long)(d_ev/1000000ull), (unsigned long long)d_evc,
             (unsigned long long)(d_sw/1000000ull),
             (unsigned long long)(d_rsn/1000000ull), (unsigned long long)d_rsc,
             (unsigned long long)d_fbr, (unsigned long long)d_ovl,
             (unsigned long long)(d_fbr ? d_ovl/d_fbr : 0),
             s->m2_gpga_n, (unsigned long long)s->m2_gpga_idx_mismatch,
             (unsigned long long)s->m2_gpga_idx_audit,
             (unsigned long long)d_vsi, (unsigned long long)d_vsc,
             (unsigned long long)(d_vsc ? d_vsi/d_vsc : 0), s->m2_mapped_va_n,
             (unsigned long long)(d_bmn/1000000ull), (unsigned long long)d_bmc,
             (unsigned long long)(d_ovn/1000000ull), (unsigned long long)d_ovc,
             (unsigned long long)(d_ovc ? d_ovn/d_ovc : 0),
             (unsigned long long)(d_pgn/1000000ull), (unsigned long long)d_pgc,
             (unsigned long long)(d_pgc ? d_pgn/d_pgc : 0),
             (unsigned long long)d_wrd, (unsigned long long)d_wwr,
             100.0 - (double)inside*100.0/(double)wall);
    nvkvm_tw_last_ns = now;
    nvkvm_tw_db_ns = nvkvm_t_doorbell_ns; nvkvm_tw_db_calls = nvkvm_t_doorbell_calls;
    nvkvm_tw_db_cpu_ns = nvkvm_t_doorbell_cpu_ns;
    nvkvm_tw_cx_ns = nvkvm_t_chan_exec_ns; nvkvm_tw_sweep_ns = nvkvm_t_sweep_ns;
    nvkvm_tw_event_ns = nvkvm_t_event_ns; nvkvm_tw_event_calls = nvkvm_t_event_calls;
    nvkvm_tw_winrd_calls = nvkvm_t_win_rd_calls; nvkvm_tw_winwr_calls = nvkvm_t_win_wr_calls;
    nvkvm_tw_ce_calls = nvkvm_dp_ce_launchdma_calls;
    nvkvm_tw_resolve_ns = nvkvm_t_resolve_ns; nvkvm_tw_resolve_calls = nvkvm_t_resolve_calls;
    nvkvm_tw_fbrd_calls = nvkvm_t_fbrd_calls; nvkvm_tw_overlay_iters = nvkvm_t_overlay_iters;
    nvkvm_tw_vaseen_iters = nvkvm_t_vaseen_iters; nvkvm_tw_vaseen_calls = nvkvm_t_vaseen_calls;
    nvkvm_tw_backmap_ns = nvkvm_t_backmap_ns; nvkvm_tw_backmap_calls = nvkvm_t_backmap_calls;
    nvkvm_tw_fbrd_ov_ns = nvkvm_t_fbrd_ov_ns; nvkvm_tw_fbrd_ov_calls = nvkvm_t_fbrd_ov_calls;
    nvkvm_tw_fbrd_pg_ns = nvkvm_t_fbrd_pg_ns; nvkvm_tw_fbrd_pg_calls = nvkvm_t_fbrd_pg_calls;
}

/* CE-fwd map-on-touch: lazily CPU-map a gpu_only object on its first guest CPU touch.
 * Sets m2_objs[oi].cpu_qva on success (obj.promote 1->0) or gives up (1->2). fwd-decl
 * because the overlay (below) is the first-touch hook. */
static bool nvkvm_m2_promote_gpu_only(NvkvmGpuEmul *s, int oi);
/* M5.10 PERF: GR PT-page set membership — fwd-decl (nvkvm_fb_write, below, is the dirty hook). */
static bool nvkvm_m2_gr_pt_contains(NvkvmGpuEmul *s, uint64_t addr);

/* M5.11c PERF: (re)build the sorted-by-base index over m2_gpga[]. Insertion sort — n is small
 * (~430) and rebuilds happen only when an object is allocated (load-time), never during steady
 * generation (the lookup-heavy phase). */
static void nvkvm_m2_gpga_index_rebuild(NvkvmGpuEmul *s)
{
    int n = s->m2_gpga_n;
    if (n > (int)ARRAY_SIZE(s->m2_gpga_sorted)) { n = (int)ARRAY_SIZE(s->m2_gpga_sorted); }
    for (int i = 0; i < n; i++) { s->m2_gpga_sorted[i] = (uint16_t)i; }
    for (int i = 1; i < n; i++) {
        uint16_t key = s->m2_gpga_sorted[i];
        uint64_t kb = s->m2_gpga[key].gpga_base;
        int j = i - 1;
        while (j >= 0 && s->m2_gpga[s->m2_gpga_sorted[j]].gpga_base > kb) {
            s->m2_gpga_sorted[j + 1] = s->m2_gpga_sorted[j];
            j--;
        }
        s->m2_gpga_sorted[j + 1] = key;
    }
    s->m2_gpga_sorted_n = n;
    s->m2_gpga_idx_dirty = false;
}

/* Return the m2_gpga[] index whose range contains fb_addr, or -1. Ranges are non-overlapping
 * (one object per GPGA range), so the only entry that can contain fb_addr is the one with the
 * largest gpga_base <= fb_addr — a binary search, exact. Replaces the O(n~430) linear scan that
 * dominated exec_doorbell (60M fruitless iters/window). A self-disabling audit (m2_gpga_idx_audit)
 * cross-checks against the old linear scan for the first N lookups and counts any disagreement. */
static int nvkvm_m2_gpga_find(NvkvmGpuEmul *s, uint64_t fb_addr)
{
    if (s->m2_gpga_idx_dirty || s->m2_gpga_sorted_n != s->m2_gpga_n) {
        nvkvm_m2_gpga_index_rebuild(s);
    }
    int lo = 0, hi = s->m2_gpga_sorted_n - 1, cand = -1;
    while (lo <= hi) {
        int mid = (lo + hi) >> 1;
        int gi = s->m2_gpga_sorted[mid];
        if (s->m2_gpga[gi].gpga_base <= fb_addr) { cand = gi; lo = mid + 1; }
        else { hi = mid - 1; }
    }
    if (cand >= 0 && fb_addr >= s->m2_gpga[cand].gpga_base + s->m2_gpga[cand].size) {
        cand = -1;
    }
    if (s->m2_gpga_idx_audit) {
        s->m2_gpga_idx_audit--;
        int lin = -1;
        for (int i = 0; i < s->m2_gpga_n; i++) {
            if (fb_addr >= s->m2_gpga[i].gpga_base &&
                fb_addr <  s->m2_gpga[i].gpga_base + s->m2_gpga[i].size) { lin = i; break; }
        }
        /* Equivalent if both miss, or both hit the SAME gpga_base (allow a different array index
         * that maps an identical range — never observed, but harmless). */
        bool same = (lin < 0 && cand < 0) ||
                    (lin >= 0 && cand >= 0 &&
                     s->m2_gpga[lin].gpga_base == s->m2_gpga[cand].gpga_base &&
                     s->m2_gpga[lin].size == s->m2_gpga[cand].size);
        if (!same) {
            s->m2_gpga_idx_mismatch++;
            if (s->m2_gpga_idx_mismatch <= 20) {
                qemu_log("nvkvm-gpu[%s] M5.11c GPGA-INDEX MISMATCH fb=0x%llx lin=%d(base=0x%llx) "
                         "bin=%d(base=0x%llx) — OVERLAP, binary search UNSAFE\n", s->chip->name,
                         (unsigned long long)fb_addr, lin,
                         (unsigned long long)(lin >= 0 ? s->m2_gpga[lin].gpga_base : 0), cand,
                         (unsigned long long)(cand >= 0 ? s->m2_gpga[cand].gpga_base : 0));
            }
            return lin;   /* trust the legacy scan while auditing */
        }
    }
    return cand;
}

static uint8_t *nvkvm_fb_host_overlay(NvkvmGpuEmul *s, uint64_t fb_addr)
{
    /* M5.44: m2_fbback is consulted FIRST. An fbback entry is a channel-bound AUTHORITATIVE
     * backing (e.g. a COPY channel's real USERD object handed to the host channel as
     * hUserdMemory[0], or a back_and_map host placement); a populate_cvas GPGA run is a
     * blanket blank-vidmem shadow that may legitimately COVER such pages (the 2 MiB COPY
     * channel-pool run holds all 16 GPFIFOs+USERDs). With GPGA-first, the shadow stole the
     * guest's GP_PUT writes into a page the host GPU never reads; with fbback-first the
     * USERD pages stay authoritative while the rest of the GPGA run (GPFIFO entries the
     * host GPU fetches by VA) is still served by the gpu_memory_object. */
    for (int i = 0; i < s->m2_fbback_n; i++) {
        nvkvm_t_overlay_iters++;
        if (fb_addr >= s->m2_fbback[i].fb_base &&
            fb_addr <  s->m2_fbback[i].fb_base + s->m2_fbback[i].size) {
            return (uint8_t *)s->m2_fbback[i].host_qva +
                   (fb_addr - s->m2_fbback[i].fb_base);
        }
    }
    /* M7 REFACTOR: fb_addr is a GPGA; resolve it to its backing gpu_memory_object's CPU
     * mapping. Empty => fall through (in the caller) to the local fb_pages. */
    int i = nvkvm_m2_gpga_find(s, fb_addr);   /* M5.11c: binary search (was O(n~430) linear scan) */
    if (i >= 0) {
        int oi = s->m2_gpga[i].obj_idx;
        if (oi < 0 || oi >= s->m2_objs_n) {
            return NULL;                     /* GPGA known but no object -> fb_pages */
        }
        /* CE-fwd map-on-touch: a gpu_only object (real host vidmem + GPU-side map_dma,
         * NO CPU view -> zero host BAR1) has no cpu_qva yet. The guest CPU is touching it
         * NOW (this is the trap) -> promote: RM_MAP_MEMORY the SAME hMem (coherent) +
         * replay any pre-promotion fb_pages writes, then serve from the real bytes. Skip
         * during the GMMU walk (m2_in_walk) to avoid re-entering backing mid-PTE-read. */
        if (!s->m2_objs[oi].cpu_qva && s->m2_objs[oi].promote == 1 && !s->m2_in_walk) {
            nvkvm_m2_promote_gpu_only(s, oi);   /* sets cpu_qva on success */
        }
        if (!s->m2_objs[oi].cpu_qva) {
            return NULL;                     /* no CPU backing (gpu_only/given-up) -> fb_pages */
        }
        return (uint8_t *)s->m2_objs[oi].cpu_qva + s->m2_gpga[i].off +
               (fb_addr - s->m2_gpga[i].gpga_base);
    }
    return NULL;
}

/* M5.48: a CE fill/copy/scrub must NEVER overwrite a LIVE channel's USERD page —
 * the page that holds the ring GP_GET(+0x88)/GP_PUT(+0x8C) cursors.  PyTorch's
 * caching allocator zero-fills a fresh 2 MiB pool (a single CE LAUNCH_DMA with
 * SET_REMAP_CONST_A=0) whose PHYSICAL FB destination happens to span a registered
 * channel's USERD (observed: fb 0x4202000 sits inside a 0x4200000 + 2 MiB fill).
 * Zeroing it makes the next GP_PUT read return 0, the ring goes permanently idle
 * (get==put==0), the channel's genuinely-pending work never executes, and the
 * guest's cuStreamSynchronize / cuCtxSynchronize blocks forever — the PyTorch
 * CUDA hang.  Return true if [fb_phys, fb_phys+4) lies within any registered
 * channel USERD so the CE write loops SKIP that span.  Spans are already clamped
 * to a single 4 KiB page, so this protects exactly the USERD page(s) and writes
 * all surrounding data normally.  (Follow-up correctness refinement: UNregister a
 * channel's USERD from m2_chanbuf[]/chans[] on GSP_RM_FREE of the channel object,
 * so a freed-then-recycled region is no longer protected; until then a still-
 * registered USERD is treated as live and preserved, which is the safe default.) */
static bool nvkvm_fb_is_live_userd(NvkvmGpuEmul *s, uint64_t fb_phys)
{
    for (int k = 0; k < s->m2_chanbuf_n; k++) {
        uint64_t b = s->m2_chanbuf[k].fb_base;
        uint64_t e = b + (s->m2_chanbuf[k].size ? s->m2_chanbuf[k].size : 0x1000ull);
        if (b && fb_phys >= b && fb_phys < e) {
            return true;
        }
    }
    return false;
}

/* Aligned reg accesses never straddle a 4 KiB page. */
static uint64_t nvkvm_fb_read(NvkvmGpuEmul *s, uint64_t fb_addr, unsigned size)
{
    nvkvm_t_fbrd_calls++;
    /* M5.14 DIAG: satisfy the guest-kernel post-PROMOTE_CTX completion poll. The fake-GSP model
     * never runs the real golden-image/ctx-init work, so the vidmem status word the guest RM
     * busy-polls stays 0 forever. Inject a sentinel (host owns real ctx-switch; guest userspace
     * never observes this kernel-internal word). One read per offset within the page is served. */
    if (s->m2semval && (fb_addr & ~0xfffull) == (s->m2sempage & ~0xfffull)) {
        uint64_t v = (size >= 8) ? s->m2semval : (s->m2semval & ((1ull << (size * 8)) - 1));
        if (s->m2_crashwin && s->m2_crashwin_reads < 100000) {
            s->m2_crashwin_reads++;
            qemu_log("nvkvm-gpu[GA106] M5.14 SEM-INJECT fb=0x%llx sz=%u -> 0x%llx\n",
                     (unsigned long long)fb_addr, size, (unsigned long long)v);
        }
        return v;
    }
    uint8_t *hp = ((s->m2_fbback_n || s->m2_gpga_n) ? nvkvm_fb_host_overlay(s, fb_addr) : NULL);
    if (hp) {                            /* M5.3: served from real host GPU memory */
        uint64_t v;
        uint64_t t0ov = nvkvm_now_ns();  /* M5.11d: isolate the host-overlay READ latency (uncached
                                          * PCIe if cpu_qva is a BAR1 map of host vidmem) vs fb_page RAM */
        switch (size) {
        case 1: v = *hp; break;
        case 2: v = lduw_le_p(hp); break;
        case 4: v = ldl_le_p(hp); break;
        case 8: v = ldq_le_p(hp); break;
        default: v = 0; break;
        }
        nvkvm_t_fbrd_ov_ns += nvkvm_now_ns() - t0ov; nvkvm_t_fbrd_ov_calls++;
        if (s->m2_crashwin && !s->m2_in_walk && s->m2_crashwin_reads < 100000) {
            s->m2_crashwin_reads++;
            qemu_log("nvkvm-gpu[GA106] CRASHWIN RD fb=0x%llx sz=%u = 0x%llx "
                     "(HOST-BACKED) gva=0x%llx\n", (unsigned long long)fb_addr, size,
                     (unsigned long long)v, (unsigned long long)s->m2_cur_gva);
        }
        return v;
    }
    uint8_t *p = nvkvm_fb_page(s, fb_addr, false);
    uint32_t o = fb_addr & 0xfffu;
    uint64_t v;
    uint64_t t0pg = nvkvm_now_ns();
    if (!p) {
        v = 0;
    } else {
        switch (size) {
        case 1: v = p[o]; break;
        case 2: v = lduw_le_p(p + o); break;
        case 4: v = ldl_le_p(p + o); break;
        case 8: v = ldq_le_p(p + o); break;
        default: v = 0; break;
        }
    }
    nvkvm_t_fbrd_pg_ns += nvkvm_now_ns() - t0pg; nvkvm_t_fbrd_pg_calls++;
    /* M5.3 DIAG: crash-window probe — log FB reads after the 0xc7c0 alloc. A read
     * returning 0 from an UN-backed page (p==NULL) is a prime suspect for the value
     * that corrupts libcuda's frame; its fb_addr identifies the buffer to back. */
    if (s->m2_crashwin && !s->m2_in_walk && s->m2_crashwin_reads < 100000) {
        s->m2_crashwin_reads++;
        qemu_log("nvkvm-gpu[GA106] CRASHWIN RD fb=0x%llx sz=%u = 0x%llx%s gva=0x%llx\n",
                 (unsigned long long)fb_addr, size, (unsigned long long)v,
                 p ? "" : " (UNBACKED-ZERO)", (unsigned long long)s->m2_cur_gva);
    }
    return v;
}

static void nvkvm_fb_write(NvkvmGpuEmul *s, uint64_t fb_addr, uint64_t val,
                           unsigned size)
{
    /* M5.10 PERF: if this write lands on a tracked GR-VAS page-table page, a mapping changed ->
     * flag a re-sweep for the next doorbell. The lo/hi range rejects the common data-plane write
     * in two compares; the hash confirms. Once dirty, skip until the next sweep consumes it. */
    if (s->m2_gr_pt_n && !s->m2_gr_vas_dirty &&
        fb_addr >= s->m2_gr_pt_lo && fb_addr <= s->m2_gr_pt_hi + 0xfffull &&
        nvkvm_m2_gr_pt_contains(s, fb_addr)) {
        s->m2_gr_vas_dirty = true;
    }
    /* M5.31 DIAG: log guest writes into the GR-VAS page-table region the cuCtxCreate
     * poll re-walks (the stuck small-page PDE @0x2efbc5000 + its PTE table @0x2efbc6xxx).
     * Tells us whether the CPU-RM WRITES these (per gmmu_walk.c memmgrMemWrite -> a
     * read/write-aperture asymmetry if they read back 0) or never writes them (the
     * mapping is awaited from elsewhere). Gated on crashwin so it only fires post-0xc7c0. */
    if (s->m2_crashwin && fb_addr >= 0x2efbc0000ull && fb_addr < 0x2efbd0000ull) {
        qemu_log("nvkvm-gpu[GA106] M5.31 GRPT-WR fb=0x%llx sz=%u val=0x%llx\n",
                 (unsigned long long)fb_addr, size, (unsigned long long)val);
    }
    /* M5.44 TRACE: any write to a registered channel USERD's GP_GET(+0x88)/GP_PUT(+0x8C)
     * word — log the value and WHICH overlay branch serves this address, so a stolen/diverted
     * GP_PUT is directly visible. Rare (ring-control words only), unbounded is fine. */
    if (s->m2_trace && ((fb_addr & 0xfffu) == 0x88u || (fb_addr & 0xfffu) == 0x8Cu)) {
        for (int k = 0; k < s->m2_chanbuf_n; k++) {
            if ((fb_addr & ~0xfffull) == s->m2_chanbuf[k].fb_base) {
                const char *br = "fb_pages"; int bi = -1;
                for (int i = 0; i < s->m2_fbback_n; i++) {
                    if (fb_addr >= s->m2_fbback[i].fb_base &&
                        fb_addr < s->m2_fbback[i].fb_base + s->m2_fbback[i].size) {
                        br = "fbback"; bi = i; break;
                    }
                }
                if (bi < 0) {
                    for (int i = 0; i < s->m2_gpga_n; i++) {
                        if (fb_addr >= s->m2_gpga[i].gpga_base &&
                            fb_addr < s->m2_gpga[i].gpga_base + s->m2_gpga[i].size) {
                            br = "gpga"; bi = i; break;
                        }
                    }
                }
                qemu_log("nvkvm-gpu[%s] M5.44 USERD-WR fb=0x%llx %s=0x%llx via %s[%d] "
                         "(chan 0x%08x hostqva=%p)\n", s->chip->name,
                         (unsigned long long)fb_addr,
                         ((fb_addr & 0xfffu) == 0x8Cu) ? "GP_PUT" : "GP_GET",
                         (unsigned long long)val, br, bi, s->m2_chanbuf[k].chan,
                         s->m2_chanbuf[k].qva);
                break;
            }
        }
    }
    uint8_t *hp = ((s->m2_fbback_n || s->m2_gpga_n) ? nvkvm_fb_host_overlay(s, fb_addr) : NULL);
    if (hp) {                            /* M5.3: written through to real host GPU memory */
        nvkvm_dp_overlay_real_write_bytes += size;   /* NVKVM-DPLANE (ii): real backing */
        switch (size) {
        case 1: *hp = (uint8_t)val; break;
        case 2: stw_le_p(hp, (uint16_t)val); break;
        case 4: stl_le_p(hp, (uint32_t)val); break;
        case 8: stq_le_p(hp, val); break;
        default: break;
        }
        return;
    }
    nvkvm_dp_fbpage_write_bytes += size;             /* NVKVM-DPLANE (ii): fake fb_page */
    uint8_t *p = nvkvm_fb_page(s, fb_addr, true);
    uint32_t o = fb_addr & 0xfffu;
    switch (size) {
    case 1: p[o] = (uint8_t)val; break;
    case 2: stw_le_p(p + o, (uint16_t)val); break;
    case 4: stl_le_p(p + o, (uint32_t)val); break;
    case 8: stq_le_p(p + o, val); break;
    default: break;
    }
}

/* M0: identity registers answered; everything else reads 0.  M1/M2 extend this
 * switch into the fake-the-boot state machine (GFW_BOOT, HWCFG2, RISCV_STATUS,
 * FWSEC/Booter mailboxes). */
static uint64_t nvkvm_reg_read(NvkvmGpuEmul *s, hwaddr off, unsigned size)
{
    /* M6: BAR0 PRAMIN window -> sparse FB backing. */
    if (off >= NVKVM_PRAMIN_BASE && off < NVKVM_PRAMIN_BASE + NVKVM_PRAMIN_SIZE) {
        nvkvm_t_mark_start();
        uint64_t t0 = nvkvm_now_ns();
        uint64_t v = nvkvm_fb_read(s, nvkvm_pramin_fb_addr(s, off), size);
        nvkvm_t_win_rd_ns += nvkvm_now_ns() - t0; nvkvm_t_win_rd_calls++;
        return v;
    }
    if (off == NVKVM_BAR0_WINDOW) {
        return s->bar0_window;
    }
    /* M7 — CPU interrupt tree reads (the ISR reads TOP to find pending subtrees,
     * then LEAF for the vectors). */
    if (off == NVKVM_VF_INTR_TOP0)        { return s->intr_top; }
    if (off == NVKVM_VF_INTR_TOP_EN_SET0 || off == NVKVM_VF_INTR_TOP_EN_CLR0) {
        return s->intr_top_en;
    }
    if (off >= NVKVM_VF_INTR_LEAF0 && off < NVKVM_VF_INTR_LEAF0 + NVKVM_VF_INTR_NLEAF*4) {
        return s->intr_leaf[(off - NVKVM_VF_INTR_LEAF0)/4];
    }
    if (off >= NVKVM_VF_INTR_LEAF_EN_SET0 && off < NVKVM_VF_INTR_LEAF_EN_SET0 + NVKVM_VF_INTR_NLEAF*4) {
        return s->intr_leaf_en[(off - NVKVM_VF_INTR_LEAF_EN_SET0)/4];
    }
    if (off >= NVKVM_VF_INTR_LEAF_EN_CLR0 && off < NVKVM_VF_INTR_LEAF_EN_CLR0 + NVKVM_VF_INTR_NLEAF*4) {
        return s->intr_leaf_en[(off - NVKVM_VF_INTR_LEAF_EN_CLR0)/4];
    }
    switch (off) {
    case NV_PMC_BOOT_0:  return s->chip->pmc_boot_0;
    case NV_PMC_BOOT_42: return s->chip->pmc_boot_42;
    case NV_PMC_BOOT_1:  return 0; /* VGPU=REAL, no virtualization advertised */

    /* M1 — fake the GFW boot.  PLM "fully lowered" (all privilege levels
     * granted: bit0 READ_PROTECTION_LEVEL0 must be ENABLE); GFW_BOOT progress
     * COMPLETED so gpuWaitForGfwBootComplete_TU102 succeeds. */
    case NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK: return 0xFFFFFFFFu;
    case NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT:
        return NV_PGC6_GFW_BOOT_PROGRESS_COMPLETED;

    /* M2 — GSP falcon already halted (FWSEC/Booter "finished"). */
    case NV_PGSP_FALCON_CPUCTL: return NV_PFALCON_FALCON_CPUCTL_HALTED_TRUE;
    /* M2 — GSP falcon has a RISC-V core, memory scrubbing done. */
    case NV_PGSP_FALCON_HWCFG2: return NV_PFALCON_FALCON_HWCFG2_RISCV_ENABLE_VAL;

    /* M3 — PTIMER (GPU ns clock). Real monotonic counter from QEMU's virtual
     * clock so RM timeout loops actually elapse (constant value => infinite
     * spin). TIME_0 low 32 (5-bit aligned), TIME_1 high 32. */
    /* #90: stash the exact sample so the Clock record carries the ns the served
     * value was derived from, not a second (later) sample.  One store; the
     * served value is bit-for-bit what it was before. */
    case NV_PTIMER_TIME_0_GA10X:
        g_nvkvm_rec_ptimer_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
        return (uint32_t)g_nvkvm_rec_ptimer_ns & 0xFFFFFFE0u;
    case NV_PTIMER_TIME_1_GA10X:
        g_nvkvm_rec_ptimer_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
        return (uint32_t)(g_nvkvm_rec_ptimer_ns >> 32);
    /* PTIMER PLM fully lowered: tmrSetCurrentTime_GV100 needs WRITE_PROTECTION
     * _LEVEL0=ENABLE (bit4) or it NV_ASSERT(0)s (timer_gv100.c:80). */
    case NV_PTIMER_TIME_PRIV_LEVEL_MASK: return 0xFFFFFFFFu;

    /* Report display fused-off => compute-only displayless GPU.  The driver's
     * gpuFuseSupportsDisplay_HAL gives NV_ERR_NOT_SUPPORTED in display
     * StatePreInit, skipping all display engine init (inst-mem/heads/channels). */
    case NV_FUSE_STATUS_OPT_DISPLAY: return NVKVM_FUSE_OPT_DISPLAY_DISABLED;

    /* M3 — Falcon DMA always idle+not-full (FWSEC on GSP, Booter on SEC2). */
    case NV_PGSP_FALCON_DMATRFCMD:
    case NV_PSEC_FALCON_DMATRFCMD: return NV_PFALCON_DMATRFCMD_IDLE_VAL;
    /* SEC2 falcon halted (Booter "finished"); SEC2 has no RISC-V advertised. */
    case NV_PSEC_FALCON_CPUCTL:    return NV_PFALCON_FALCON_CPUCTL_HALTED_TRUE;
    case NV_PSEC_FALCON_HWCFG2:    return 0;

    /* M3 — usable FB size (MiB) for the GSP WprMeta computation. */
    case NV_USABLE_FB_SIZE_IN_MB: return NVKVM_FB_SIZE_MB;

    /* M3 — GSP RISC-V core active once the Booter "started" it (post-FWSEC). */
    case NV_PGSP_RISCV_CPUCTL:
        return s->fwsec_ran ? NV_PRISCV_RISCV_CPUCTL_ACTIVE_STAT_VAL : 0;

    /* M3 — WPR2 is stateful: DOWN (0) until FWSEC "runs" (GSP STARTCPU), then
     * UP.  The driver requires WPR2 down before FWSEC, up after. */
    case NV_PFB_PRI_MMU_WPR2_ADDR_LO: return s->fwsec_ran ? NVKVM_WPR2_LO_VAL : 0;
    case NV_PFB_PRI_MMU_WPR2_ADDR_HI: return s->fwsec_ran ? NVKVM_WPR2_HI_VAL : 0;

    /* GSP falcon MAILBOX0: on teardown (kgspUnloadRm) the driver polls this for
     * INTERRUPT_PROCESSOR_SUSPENDED_VALUE (0x80000000).  Report suspended once
     * the UNLOADING RPC arrived so close() doesn't hang 4s. */
    case NV_PGSP_FALCON_MAILBOX0: return s->gsp_suspended ? 0x80000000u : 0;

    /* M5/M7 — GSP falcon interrupt registers for os-event delivery.
     * kgspService_TU102 calls kflcnGetPendingHostInterrupts = IRQSTAT & IRQMASK
     * & IRQDEST (legacy) or IRQSTAT & RISCV_IRQMASK & RISCV_IRQDEST (riscv).  We
     * advertise SWGEN0 (bit6) enabled in all mask/dest regs and latch it in
     * IRQSTAT when an event is pending; cleared via IRQSCLR write.  Offsets:
     * NV_PGSP base 0x110000; FALCON IRQSTAT +0x08, IRQMASK +0x18, IRQDEST +0x1c;
     * RISCV base 0x111000; RISCV_IRQMASK +0x528, RISCV_IRQDEST +0x52c. */
    case 0x00110008u: return s->gsp_swgen0_pending ? (1u << 6) : 0; /* FALCON IRQSTAT */
    case 0x00110018u: return (1u << 6);                             /* FALCON IRQMASK */
    case 0x0011001cu: return (1u << 6);                             /* FALCON IRQDEST */
    case 0x00111528u: return (1u << 6);                             /* RISCV  IRQMASK */
    case 0x0011152cu: return (1u << 6);                             /* RISCV  IRQDEST */

    /* NV_VIRTUAL_FUNCTION_PRIV_ACCESS_COUNTER_NOTIFY_BUFFER_SIZE (VF 0xB80000 +
     * 0x3110): UVM_REGISTER_GPU's uvmGetAccessCounterBufferSize reads this and
     * multiplies by 32 for the notify-buffer byte size; 0 => memdescCreate(0) =>
     * NV_ERR_INVALID_ARGUMENT (access_cntr_buffer.c:72) => UVM register fails =>
     * cuInit bails.  Report 256 entries (8 KiB buffer). */
    case 0x00B83110u: return 256u;

    default:             return 0;
    }
}

/* M5.64: refresh the GSP-falcon rom-device RAM buffer (BAR0 page 0x110000) with the values
 * nvkvm_reg_read would compute, so the driver's poll-READs hit RAM (no vmexit) yet see correct
 * data. Called at init and after any source-state change. Cheap (a handful of stores). The page
 * holds ONLY GSP falcon status regs (the GSP cmd/status queue DATA lives in sysmem, not here), and
 * none have read side-effects, so RAM-serving is safe. WRITES still trap via the rom-device ops. */
static void nvkvm_gsp_falcon_sync(NvkvmGpuEmul *s)
{
    uint8_t *p = s->gsp_falcon_ram;
    if (!p) { return; }
    memset(p, 0, 0x1000);                                         /* default 0 (DEBUGINFO 0x94, ...) */
    stl_le_p(p + 0x008, s->gsp_swgen0_pending ? (1u << 6) : 0u);  /* FALCON IRQSTAT (SWGEN0 bit6) */
    stl_le_p(p + 0x018, (1u << 6));                               /* FALCON IRQMASK  */
    stl_le_p(p + 0x01c, (1u << 6));                               /* FALCON IRQDEST  */
    stl_le_p(p + 0x040, s->gsp_suspended ? 0x80000000u : 0u);     /* MAILBOX0 (suspend poll) */
    stl_le_p(p + 0x0f4, NV_PFALCON_FALCON_HWCFG2_RISCV_ENABLE_VAL);
    stl_le_p(p + 0x100, NV_PFALCON_FALCON_CPUCTL_HALTED_TRUE);
    stl_le_p(p + 0x118, NV_PFALCON_DMATRFCMD_IDLE_VAL);           /* DMATRFCMD idle */
    /* DEBUGINFO (0x94) stays 0 — same as the old default-return; the win is that it no longer traps. */

    /* #90 / §6.2: with m2romregs=on this page is served from RAM, so the guest's
     * reads of IRQSTAT / MAILBOX0 / CPUCTL / DMATRFCMD — the most-read registers
     * in the system — NEVER TRAP and can never appear as MmioRead.  Snapshot the
     * page every time it is re-synced so the trace at least contains what those
     * reads would have returned.  Has no TraceEvent counterpart; see
     * NVKVM_REC_OVERLAY in nvkvm_m2_rec.h.  Moot when m2romregs=off. */
    nvkvm_rec_emit(NVKVM_REC_M_OVERLAY, NVKVM_REC_OVERLAY, 0, 0,
                   0x00110000ull, 0, p, 0x1000);
}

/* PROM window: return VBIOS bytes (little-endian dword at the aligned offset).
 * Not traced per-access — the driver streams the whole ~1 MiB image. */
static bool nvkvm_prom_read(NvkvmGpuEmul *s, hwaddr off, unsigned size,
                            uint64_t *out)
{
    if (!s->vbios || off < NV_PROM_DATA_BASE ||
        off >= NV_PROM_DATA_BASE + NV_PROM_DATA_SIZE) {
        return false;
    }
    hwaddr p = off - NV_PROM_DATA_BASE;
    uint64_t v = 0;
    for (unsigned i = 0; i < size && p + i < NV_PROM_DATA_SIZE; i++) {
        v |= (uint64_t)s->vbios[p + i] << (8 * i);
    }
    s->prom_reads++;
    *out = v;
    return true;
}

static uint64_t nvkvm_bar0_read_inner(void *opaque, hwaddr off, unsigned size)
{
    NvkvmGpuEmul *s = opaque;
    uint64_t prom;
    if (nvkvm_prom_read(s, off, size, &prom)) {
        return prom;
    }
    /* NV_PCFG config mirror in BAR0 (DEVICE_BASE(NV_PCFG)=0x88000).  The kernel
     * BIF reads the PCIe link registers here via GPU_BUS_CFG_RD32 (NOT real PCI
     * config space), and UVM's getPCIELinkRateMBps reads LINK_CAPABILITIES for
     * BUS_INFO PCIE_GPU_LINK_CAPS — 0 => "Unknown PCIe speed" => NV_ERR_INVALID_
     * STATE => UVM_REGISTER_GPU fails => cuInit bails.  Report Gen4 x16.
     *   0x88084 NV_XVE_LINK_CAPABILITIES: MAX_SPEED[3:0]=4, MAX_WIDTH[9:4]=16
     *   0x88088 NV_XVE_LINK_CONTROL_STATUS: CUR_SPEED[19:16]=4, WIDTH[25:20]=16 */
    if (off == 0x88084u) {
        return 4u | (16u << 4);
    }
    if (off == 0x88088u) {
        return (4u << 16) | (16u << 20);
    }
    uint64_t val = nvkvm_reg_read(s, off, size);

    /* Don't trace PTIMER reads (RM timeout loops poll millions of times) or the
     * PRAMIN window (BAR2/page-table setup hammers it). */
    if (s->m2_trace && s->trace && off != NV_PTIMER_TIME_0_GA10X &&
        off != NV_PTIMER_TIME_1_GA10X &&
        !(off >= NVKVM_PRAMIN_BASE && off < NVKVM_PRAMIN_BASE + NVKVM_PRAMIN_SIZE)) {
        const char *nm = nvkvm_reg_name(off);
        qemu_log("nvkvm-gpu[%s] #%llu BAR0 RD  off=0x%06llx sz=%u -> 0x%08llx%s%s\n",
                 s->chip->name, (unsigned long long)s->access_count++,
                 (unsigned long long)off, size, (unsigned long long)val,
                 nm ? "  " : "", nm ? nm : "");
    }
    return val;
}

/* #90 GAP-I1: the ONE place a BAR0 read is recorded, with the value ACTUALLY
 * RETURNED.
 *
 * The body above has three early returns that bypassed every existing trace —
 * the PROM/VBIOS window and the two NV_XVE LINK_* registers — so ~1 MiB of
 * streamed VBIOS and the value that gates UVM_REGISTER_GPU have never appeared
 * in any C log.  Wrapping is what makes "the value served" true by
 * construction, rather than true at three of four return sites. */
static uint64_t nvkvm_bar0_read(void *opaque, hwaddr off, unsigned size)
{
    uint64_t val = nvkvm_bar0_read_inner(opaque, off, size);
    if (nvkvm_rec_on()) {
        uint64_t region = nvkvm_rec_bar0_region(off);
        /* PTIMER: emit the clock sample the value came from FIRST, so the
         * stream reads "here is the time, and here is what the guest was told
         * about it". */
        if (region == NVKVM_REC_M_PTIMER &&
            (nvkvm_rec_mask() & NVKVM_REC_M_PTIMER)) {
            nvkvm_rec_emit(NVKVM_REC_M_CLOCK, NVKVM_REC_CLOCK, 0xFF, 0,
                           g_nvkvm_rec_ptimer_ns, 0, NULL, 0);
        }
        nvkvm_rec_mmio_rd(0, region, off, size, val);
    }
    return val;
}

/* GSP msgq checksum: u64 XOR-fold over the element, returned folded to 32 bits
 * (mirrors _checkSum32 in message_queue_priv.h). The driver requires the whole
 * element to fold to 0, so the sender stores checkSum = fold(element|checkSum=0). */
static uint32_t nvkvm_msgq_checksum32(const uint8_t *p, uint32_t len)
{
    uint64_t cs = 0;
    for (uint32_t i = 0; i < len; i += 8) {   /* matches "while (p < pEnd)" */
        cs ^= ldq_le_p(p + i);
    }
    return (uint32_t)(cs >> 32) ^ (uint32_t)cs;
}

/* M3 keystone step 2: post a GSP_INIT_DONE event into the GSP->CPU status queue
 * so the driver's kgspWaitForRmInitDone -> rpcRecvPoll returns NV_OK and
 * RmInitAdapter succeeds.  statusBase = shmem+statoff; entry slot 0 at
 * statusBase+entryOff; element = GSP_MSG_QUEUE_ELEMENT (authTag[16]/aad[16]/
 * checkSum@32/seqNum@36/elemCount@40/rpc@48) with rpc.function=GSP_INIT_DONE. */
/* Post one element to the GSP->CPU status queue with seqNum == stat_writeptr.
 * If src!=NULL it is a 4096-byte template (echo a received command): we keep
 * its rpc header/body and just override function + rpc_result.  Else build a
 * minimal header (used for GSP_INIT_DONE event).  Recomputes checksum, writes
 * the element to the ring slot, bumps the status tx writePtr. */
/* Post a SINGLE-element message to the GSP->CPU status queue.  The guest reads
 * the element-count from the elemCount field (@40), so a single 4096-byte
 * element with elemCount=1 keeps the status-queue seqNum in lockstep with the
 * guest's rxSeqNum.  Responses must therefore fit one element (params <= ~3976
 * bytes); larger captured controls are echoed instead (see service_cmdq).
 * `el` is a 4096-byte buffer already populated with the response (element
 * header + rpc header + body + params). */
static void nvkvm_m3_post_status(NvkvmGpuEmul *s, const uint8_t *src,
                                 uint32_t function, uint32_t rpc_result)
{
    PCIDevice *pdev = &s->parent_obj;
    /* A status message may span MULTIPLE queue elements when the rpc payload
     * (e.g. GET_DEVICE_INFO_TABLE paramsSize=24580) exceeds one element.  The
     * guest's GspMsgQueueReceiveStatus reads the first element, derives
     * nElements = ceil((hdrSize48 + rpc.length) / queueElementSizeMin), then
     * reads that many CONTIGUOUS slots and checksums (48 + rpc.length) bytes.
     * So we build the full message zero-padded to nElements*msgsize, set
     * elemCount, fold the checksum over the real message length, and write
     * each slot, advancing the write pointer by nElements. */
    static uint8_t el[NVKVM_RESP_MAX]; /* device emu is single-threaded */
    memset(el, 0, sizeof(el));
    if (src) {
        /* copy at most one element's worth of header+body from the seed; the
         * caller's resp buffer already holds the full payload, so copy it all */
        uint32_t copylen = 48u + ldl_le_p(src + 56);
        if (copylen > sizeof(el)) {
            copylen = sizeof(el);
        }
        memcpy(el, src, copylen);
    } else {
        stl_le_p(el + 48, 0x03000000u);  /* header_version MAJOR=3 MINOR=0 */
        stl_le_p(el + 52, 0x43505256u);  /* NV_VGPU_MSG_SIGNATURE_VALID */
        stl_le_p(el + 56, 36u);          /* length = sizeof(rpc_message_header) */
    }
    stl_le_p(el + 60, function);         /* rpc.function */
    stl_le_p(el + 64, rpc_result);       /* rpc.rpc_result */
    stl_le_p(el + 68, rpc_result);       /* rpc.rpc_result_private (RmRpc reads this) */
    stl_le_p(el + 36, s->stat_seqnum);   /* per-message seqNum (NOT element ptr) */

    uint32_t msgsize = s->q_msgsize ? s->q_msgsize : 4096u;
    uint32_t len = 48u + ldl_le_p(el + 56);          /* hdr48 + rpc.length */
    if (len > sizeof(el)) {
        len = sizeof(el);
    }
    uint32_t nelems = (len + msgsize - 1u) / msgsize; /* bytesToElements */
    if (nelems == 0) {
        nelems = 1;
    }
    stl_le_p(el + 40, nelems);           /* elemCount */
    stl_le_p(el + 32, 0);                /* zero checksum field before folding */
    /* zero-pad to an 8-byte boundary for the XOR fold (guest does the same) */
    stl_le_p(el + 32, nvkvm_msgq_checksum32(el, (len + 7u) & ~7u));

    for (uint32_t i = 0; i < nelems; i++) {
        uint32_t slot = s->q_msgcount
            ? ((s->stat_writeptr + i) % s->q_msgcount) : 0;
        uint64_t gpa = s->q_shmem + s->q_stat_base + s->q_stat_entryoff +
                       (uint64_t)slot * msgsize;
        nvkvm_dmaw(pdev, gpa, el + (uint64_t)i * msgsize, msgsize);
    }

    s->stat_writeptr = (s->stat_writeptr + nelems) % s->q_msgcount; /* modulo ring */
    s->stat_seqnum++;                    /* per-message seqNum is ABSOLUTE (no wrap) */
    uint8_t wp[4];
    stl_le_p(wp, s->stat_writeptr);
    nvkvm_dmaw(pdev, s->q_shmem + s->q_stat_base + 16, wp, sizeof(wp));
}

/* M3 keystone: post GSP_INIT_DONE (seqNum 0). */
static void nvkvm_m3_post_init_done(NvkvmGpuEmul *s)
{
    uint32_t seq = s->stat_seqnum; /* captured before post_status advances it */
    nvkvm_m3_post_status(s, NULL, 0x1001u /* GSP_INIT_DONE */, 0 /* NV_OK */);
    qemu_log("nvkvm-gpu[%s] M3: posted GSP_INIT_DONE (seqNum %u) -> "
             "RmInitAdapter should pass kgspWaitForRmInitDone\n",
             s->chip->name, seq);
}

/* M5/M7 — post a GSP NV_VGPU_MSG_EVENT_POST_EVENT (0x1003).  The body is
 * rpc_post_event_v17_00 {NvHandle hClient@0; NvHandle hEvent@4; NvU32
 * notifyIndex@8; NvU32 data@12; NvU16 info16@16; NvU32 status@20; NvU32
 * eventDataSize@24; NvBool bNotifyList@28; NvU8 eventData[]@29} placed at the
 * rpc params offset.  The GSP message element is {48-byte element header,
 * 32-byte rpc_message_header, params...}, so params (rpc_message_data) live at
 * el+48+32 = el+80 — the SAME base the working GSP_RM_CONTROL reply uses
 * (rpc.length = 32 + 40-byte gsp_rm_control header + psize -> control params at
 * el+120).  bNotifyList=0 => _kgspRpcPostEvent does CliGetEventInfo(hClient,
 * hEvent) then osNotifyEvent on the matching event (wakes the os-event fd). */
static void nvkvm_m3_post_event(NvkvmGpuEmul *s, uint32_t hclient,
                                uint32_t hevent, uint32_t notify_index,
                                uint32_t data)
{
    static uint8_t el[256];               /* device emu is single-threaded */
    memset(el, 0, sizeof(el));
    stl_le_p(el + 48, 0x03000000u);       /* rpc header_version MAJOR=3 */
    stl_le_p(el + 52, 0x43505256u);       /* NV_VGPU_MSG_SIGNATURE_VALID */
    stl_le_p(el + 80 +  0, hclient);      /* hClient */
    stl_le_p(el + 80 +  4, hevent);       /* hEvent  */
    stl_le_p(el + 80 +  8, notify_index); /* notifyIndex */
    stl_le_p(el + 80 + 12, data);         /* data */
    /* info16@16, status@20, eventDataSize@24=0, bNotifyList@28=0 stay zero */
    stl_le_p(el + 56, 32u + 32u);         /* rpc.length = hdr(32) + body(32) */
    nvkvm_m3_post_status(s, el, 0x1003u /* NV_VGPU_MSG_EVENT_POST_EVENT */, 0);
}

/* M5/M7 — latch the GSP falcon SWGEN0 interrupt and raise the GSP engine's
 * stall vector (155 = 0x9b for MC_ENGINE_IDX_GSP=50 on GA106, from the captured
 * INTERNAL_INTR_GET_KERNEL_TABLE).  Mirrors the INTR_LEAF_TRIGGER path: set the
 * leaf+top pending bits and notify MSI-X so the guest ISR -> kgspServiceInterrupt
 * -> kgspService reads SWGEN0 and drains the GSP message queue. */
static void nvkvm_gsp_raise_swgen0(NvkvmGpuEmul *s)
{
    s->gsp_swgen0_pending = true;
    nvkvm_gsp_falcon_sync(s);                 /* M5.64: reflect IRQSTAT bit6 into the rom-device RAM */
    uint32_t vec = 155u, leaf = vec / 32u, bit = vec % 32u, subtree = leaf / 2u;
    if (leaf < NVKVM_VF_INTR_NLEAF) {
        s->intr_leaf[leaf] |= (1u << bit);
        s->intr_top        |= (1u << subtree);
        PCIDevice *pd = &s->parent_obj;
        if (msix_enabled(pd)) {
            msix_notify(pd, 0);
            nvkvm_rec_irq_msix(0);          /* #90 */
        } else {
            pci_set_irq(pd, 1);
            nvkvm_rec_irq_intx(1);          /* #90 */
        }
    }
}

/* M5/M7 — deliver completion to every registered os-event, then raise SWGEN0
 * once (the guest drains all queued POST_EVENTs in one service pass). */
static void nvkvm_gsp_deliver_events(NvkvmGpuEmul *s)
{
    if (s->osevent_n <= 0) {
        return;
    }
    /* CRITICAL: the status queue is SHARED with RPC responses and has strictly
     * monotonic per-message seqNums in a small ring.  Posting an event batch on
     * every doorbell (hundreds of times) overflows the ring before the guest
     * drains it -> the guest's rpcRecvPoll sees a seqNum gap ("Bad sequence
     * number") and the whole RPC path breaks.  Gate on the previous batch being
     * drained: only post when SWGEN0 was already cleared by the guest's
     * kgspService (IRQSCLR write), bounding outstanding messages to one batch. */
    if (s->gsp_swgen0_pending) {
        return;
    }
    for (int i = 0; i < s->osevent_n; i++) {
        nvkvm_m3_post_event(s, s->osevents[i].hclient, s->osevents[i].hevent,
                            s->osevents[i].notify_index, 0);
    }
    nvkvm_gsp_raise_swgen0(s);
    if (s->trace) {
        qemu_log("nvkvm-gpu[%s] M7: delivered %d os-event(s) + raised GSP SWGEN0 "
                 "(vec 155)\n", s->chip->name, s->osevent_n);
    }
}

/* M7 fix (2026-06-16): drop os-event entries when the guest frees the event object
 * (GSP_RM_FREE of the NV01_EVENT_OS_EVENT) or its owning client (root).  Without
 * this, nvkvm_gsp_deliver_events keeps POSTing POST_EVENT to dead (hClient,hEvent)
 * pairs -> guest _kgspRpcPostEvent's CliGetEventInfo returns OBJECT_NOT_FOUND, the
 * SHARED status queue's seqNum desyncs ("Bad sequence number"), and the whole
 * RPC/event path wedges.  Reproduced THREE independent ways on bare-metal .32:
 * PyTorch CUDA-init hang, 2-process concurrent compute hang, and nvidia-smi-then-
 * cup8 (nvidia-smi frees its events on exit, leaving stale entries that poison the
 * next process's completion delivery).  freeing the client itself (fClient==fObj)
 * tears down all of its events. */
static void nvkvm_m2_osevent_drop(NvkvmGpuEmul *s, uint32_t fClient, uint32_t fObj)
{
    for (int i = 0; i < s->osevent_n; ) {
        bool ev  = (s->osevents[i].hclient == fClient && s->osevents[i].hevent == fObj);
        bool cli = (fClient == fObj && s->osevents[i].hclient == fObj);
        if (ev || cli) {
            if (s->trace) {
                qemu_log("nvkvm-gpu[%s] M7: drop stale os-event hClient=0x%08x "
                         "hEvent=0x%08x (freed 0x%08x) %d->%d\n", s->chip->name,
                         s->osevents[i].hclient, s->osevents[i].hevent, fObj,
                         s->osevent_n, s->osevent_n - 1);
            }
            s->osevents[i] = s->osevents[s->osevent_n - 1];   /* swap-with-last */
            s->osevent_n--;
            continue;                                          /* re-check slot i */
        }
        i++;
    }
}

/* M5.49 (2026-06-16): drop per-context channel/VASpace bookkeeping on GSP_RM_FREE so a
 * SECOND CUDA context (a fresh process after the first cleanly exits) starts clean.
 * Without this, ctx1's freed channels/VASes linger in chans[]/m2_chanbuf[]/chan_vas[]/
 * m2_devvas[]/m2_cvas[]; ctx2 reuses the same compute-channel VA region (gpfifo in
 * 0x2002xxxxx) but its gpfifo then FAULTs on every snooped VAS (the stale ctx1 PDBs no
 * longer map it and ctx2's real root is masked / the content-probe is misled) -> ctx2's
 * work never executes -> cuStreamSynchronize spins forever (task #12, sequential case).
 * Mirrors nvkvm_m2_osevent_drop: free of a specific channel/VASpace handle drops that
 * object; free of the client root (fClient==fObj) purges everything owned by the client.
 * Only fires for genuinely-freed objects, so removal is safe (cup8/LLM/single-proc PyTorch
 * never free a still-in-use object mid-run). */
/* #12-L3c: record a (client, pdb) VAS root in the STICKY table (never dropped on
 * free), deduped on (client,pdb).  Consulted only by the sema-write resolver so a
 * GSP-managed channel whose VAS handle is gone still resolves its completion sema
 * under its OWN client's address space (see the m2_cli_vas[] comment). */
static void nvkvm_m2_cli_vas_add(NvkvmGpuEmul *s, uint32_t client, uint64_t pdb, bool root_sys)
{
    if (!client || !pdb) { return; }
    for (int i = 0; i < s->m2_cli_vas_n; i++) {
        if (s->m2_cli_vas[i].client == client && s->m2_cli_vas[i].pdb == pdb) { return; }
    }
    if (s->m2_cli_vas_n >= (int)ARRAY_SIZE(s->m2_cli_vas)) { return; }
    int k = s->m2_cli_vas_n++;
    s->m2_cli_vas[k].client = client;
    s->m2_cli_vas[k].pdb = pdb;
    s->m2_cli_vas[k].root_sys = root_sys;
}

/* #12-L3c: record / test a GPA page base the backdoor (0xFFF508) authoritatively
 * writes, so the CE_SEM_RELEASE parser defers on it (see m2_bd_pages[] comment). */
static void nvkvm_m2_bd_page_add(NvkvmGpuEmul *s, uint64_t page)
{
    for (int i = 0; i < s->m2_bd_pages_n; i++) {
        if (s->m2_bd_pages[i] == page) { return; }
    }
    if (s->m2_bd_pages_n >= (int)ARRAY_SIZE(s->m2_bd_pages)) { return; }
    s->m2_bd_pages[s->m2_bd_pages_n++] = page;
}
static bool nvkvm_m2_bd_page_has(NvkvmGpuEmul *s, uint64_t page)
{
    for (int i = 0; i < s->m2_bd_pages_n; i++) {
        if (s->m2_bd_pages[i] == page) { return true; }
    }
    return false;
}

static void nvkvm_m2_host_rmfree(NvkvmGpuEmul *s, uint32_t client, uint32_t parent,
                                 uint32_t hobj); /* #12 cont.34 fwd-decl */
static bool nvkvm_m2_is_gr_client(NvkvmGpuEmul *s, uint32_t client);   /* P0 reap fwd-decl */
static bool nvkvm_m2_is_user_client(NvkvmGpuEmul *s, uint32_t client); /* P0 reap fwd-decl */
static int  nvkvm_m2_proc_get(NvkvmGpuEmul *s, uint32_t anchor);          /* P1 fwd-decl */
static int  nvkvm_m2_proc_find_by_client(NvkvmGpuEmul *s, uint32_t client);/* P1 fwd-decl */
static void nvkvm_m2_proc_add_client(NvkvmGpuEmul *s, int pi, uint32_t client);/* P1 */
static void nvkvm_m2_proc_add_pdb(NvkvmGpuEmul *s, uint32_t owner, uint64_t pdb);/* P1 */
static void nvkvm_m2_proc_drop_client(NvkvmGpuEmul *s, uint32_t client);  /* P1 fwd-decl */
static void nvkvm_m2_ctx_free_drop(NvkvmGpuEmul *s, uint32_t fClient, uint32_t fObj)
{
    bool root = (fClient == fObj);            /* client-root free: purge all of fClient */
    /* #14 P0: capture BEFORE the drops below mutate the client lists — is this root
     * free a user compute process's exit?  Gates the m2_objs/m2_gpga reap (see below). */
    bool user_root = root && (nvkvm_m2_is_gr_client(s, fClient) ||
                              nvkvm_m2_is_user_client(s, fClient));
    /* #14 P1: drop this client from the per-process registry (anchor free reaps the
     * proc).  Registry-only; nothing keys on it yet, so this is a no-op for behavior. */
    if (root) { nvkvm_m2_proc_drop_client(s, fClient); }
    int dropped = 0;
    bool freed_compute_chan = false;          /* #12 cont.34: a compute-aperture chan was freed */
    /* chans[]: match the freed channel handle, or any channel of the freed client. */
    for (int i = 0; i < s->chan_n; ) {
        bool hit = (root && s->chans[i].client == fClient) ||
                   (s->chans[i].client == fClient && s->chans[i].hobject == fObj);
        if (hit) {
            if (s->chans[i].gpfifo_va >= 0x200000000ull) { freed_compute_chan = true; }
            /* #12-L3c DIAG: a channel free is the (a)-vs-(b) hinge — log the freed
             * channel's gpfifo VA + client so the bench timeline shows whether the
             * 0x8a-climber's channel is FREED before the payload-9 writer appears
             * (→ page-reuse, case b) or stays live (→ same-VA alias, case a). */
            qemu_log("nvkvm-gpu[%s] #12-L3c CHAN-FREE gpfifo_va=0x%llx client=0x%08x "
                     "hobj=0x%08x (fClient=0x%08x fObj=0x%08x root=%d)\n", s->chip->name,
                     (unsigned long long)s->chans[i].gpfifo_va, s->chans[i].client,
                     s->chans[i].hobject, fClient, fObj, root);
            s->chans[i] = s->chans[s->chan_n - 1];
            s->chan_n--; dropped++; continue;
        }
        i++;
    }
    /* m2_chanbuf[]: the channel's host-USERD registration.
     * #12 cont.31 (proven via CE-INSTR pageA!=pageB): each chanbuf USERD also has a
     * paired m2_fbback overlay entry (same fb_base, host_qva) that makes the guest's
     * BAR1 GP_PUT write land in the host USERD object.  That overlay was NEVER removed
     * on channel-free.  Because the guest RE-USES the SAME USERD FB addresses across a
     * teardown (2nd cuCtxCreate), CTX2's fresh back_channel_userd APPENDS a new fbback
     * at a HIGHER index while the STALE CTX1 fbback (same fb_base, dead host object)
     * survives at a LOWER index.  nvkvm_fb_host_overlay scans fbback in order and hits
     * the stale entry FIRST -> CTX2's guest GP_PUT is diverted into the dead object,
     * while the real host USERD object (the fresh m2_chanbuf qva the host GPU reads)
     * stays GP_PUT=0 -> the host never fetches the GPFIFO, never runs the CE
     * SET_SEMAPHORE, and libcuda's wait-ALL on the 16 completion semaphores spins
     * forever = the #12 hang.  Fix: when a chanbuf USERD is removed, remove its paired
     * m2_fbback overlay (matched by fb_base) so CTX2's fresh registration is the ONLY
     * overlay for that USERD page.  (The dead host USERD object itself is freed by the
     * fn=10 FREE shadow-forward; this only drops our stale overlay bookkeeping.) */
    for (int i = 0; i < s->m2_chanbuf_n; ) {
        bool hit = (root && s->m2_chanbuf[i].client == fClient) ||
                   (s->m2_chanbuf[i].client == fClient && s->m2_chanbuf[i].chan == fObj);
        if (hit) {
            uint64_t ufb = s->m2_chanbuf[i].fb_base;
            for (int j = 0; ufb && j < s->m2_fbback_n; ) {
                if (s->m2_fbback[j].fb_base == ufb) {
                    qemu_log("nvkvm-gpu[%s] #12 drop stale USERD overlay fbback[%d] "
                             "fb_base=0x%llx host_qva=%p (chan 0x%08x freed)\n",
                             s->chip->name, j, (unsigned long long)ufb,
                             s->m2_fbback[j].host_qva, s->m2_chanbuf[i].chan);
                    s->m2_fbback[j] = s->m2_fbback[s->m2_fbback_n - 1];
                    s->m2_fbback_n--; continue;
                }
                j++;
            }
            s->m2_chanbuf[i] = s->m2_chanbuf[s->m2_chanbuf_n - 1];
            s->m2_chanbuf_n--; dropped++; continue;
        }
        i++;
    }
    /* m2_devvas[]: client -> {dev,vas}.  Collect the freed VAS handles so we can also drop
     * their page-directory roots from chan_vas[] (which has no client key). */
    uint32_t drop_hvas[32]; int drop_hvas_n = 0;
    for (int i = 0; i < s->m2_devvas_n; ) {
        bool hit = (root && s->m2_devvas[i].client == fClient) ||
                   (s->m2_devvas[i].client == fClient && s->m2_devvas[i].vas == fObj);
        if (hit) {
            if (drop_hvas_n < (int)ARRAY_SIZE(drop_hvas)) {
                drop_hvas[drop_hvas_n++] = s->m2_devvas[i].vas;
            }
            s->m2_devvas[i] = s->m2_devvas[s->m2_devvas_n - 1];
            s->m2_devvas_n--; dropped++; continue;
        }
        i++;
    }
    /* m2_cvas[]: per-(client,tsg) fresh VAS state. Purge this client's entries so the next
     * context mints a FRESH host VAS. #12 cont.34: minting fresh (vs reusing) is the correct
     * choice — reusing the old fvas keeps the guest's re-allocated-at-same-VA/new-GPA sysmem
     * colliding (st=0x51) with stale CTX1 maps and thrashes the STALE-SYS re-back (4000+
     * host-rmfree/run, opening unmap windows the host GR channel faults into). A fresh empty
     * VAS + the compute-aperture va_seen flush below (so the full working set re-backs cleanly
     * with NO collision) is stable. */
    for (int i = 0; i < s->m2_cvas_n; ) {
        if (s->m2_cvas[i].client == fClient) {     /* root or specific: both scope to client */
            s->m2_cvas[i] = s->m2_cvas[s->m2_cvas_n - 1];
            s->m2_cvas_n--; dropped++; continue;
        }
        i++;
    }
    /* #12 cont.34: on a COMPUTE-aperture channel teardown (CTX1's cuCtxDestroy frees all
     * 16 compute channels), FLUSH this client's compute-aperture (VA >= 0x2_0000_0000)
     * SYSMEM host pins so the NEXT context re-backs its pushbuffer/pool working set CLEANLY
     * into the REUSED cvas (see nvkvm_m2_cvas_get #12-cont34).  Without this, CTX2's fresh
     * PT walk coalesces the same VAs at DIFFERENT run boundaries than CTX1 left mapped, so
     * back_and_map_sys hits st=0x51 ALREADY-MAPPED against CTX1's stale pin — the host GR/CE
     * channel then faults FAULT_PTE on its own pushbuffer (host Xid 31 @0x2_02400000) and
     * never runs, so the 16 completion semaphores (pool VA 0x20440ff00..fff0) never advance
     * and libcuda's wait-ALL spins forever.  Freeing the host pin makes RM cascade-unmap its
     * FIXED map_dma from the cvas, vacating the VA for a clean re-place.  Scoped to the freed
     * client + compute aperture + only when a compute channel was actually freed, so single-
     * context paths (cup2/cup8/LLM: no mid-run compute-channel free) are untouched. */
    if (freed_compute_chan) {
        uint32_t fdev = 0;
        for (int d = 0; d < s->m2_devvas_n; d++) {
            if (s->m2_devvas[d].client == fClient) { fdev = s->m2_devvas[d].dev; break; }
        }
        int flushed = 0;
        for (int i = 0; i < s->m2_mapped_va_n; ) {
            if (s->m2_mapped_va[i].client == fClient &&
                s->m2_mapped_va[i].va >= 0x200000000ull) {
                if (s->m2_mapped_va[i].hmem && fdev) {
                    nvkvm_m2_host_rmfree(s, fClient, fdev, s->m2_mapped_va[i].hmem);
                }
                s->m2_mapped_va[i] = s->m2_mapped_va[s->m2_mapped_va_n - 1];
                s->m2_mapped_va_n--; flushed++; continue;
            }
            i++;
        }
        if (flushed) {
            qemu_log("nvkvm-gpu[%s] #12 cont.34 compute-aperture flush: freed+forgot %d "
                     "sysmem pins (client=0x%08x) for clean next-context re-back\n",
                     s->chip->name, flushed, fClient);
        }
    }
    /* #14 m2_dup[]: drop dup edges naming the freed client (root free) or the freed
     * object on either side.  RM reuses handle values across process lifetimes, so a
     * stale edge could alias a later process's fresh client/VAS onto the wrong owner. */
    for (int i = 0; i < s->m2_dup_n; ) {
        bool hit = root ? (s->m2_dup[i].dst_client == fClient ||
                           s->m2_dup[i].src_client == fClient)
                        : ((s->m2_dup[i].dst_client == fClient && s->m2_dup[i].dst_obj == fObj) ||
                           (s->m2_dup[i].src_client == fClient && s->m2_dup[i].src_obj == fObj));
        if (hit) {
            s->m2_dup[i] = s->m2_dup[s->m2_dup_n - 1];
            s->m2_dup_n--; dropped++; continue;
        }
        i++;
    }
    /* #14 m2_gr_clients[] / m2_tsg_sched[]: drop the freed client's entries — RM reuses
     * both client and TSG handle values across process lifetimes, so a stale entry would
     * alias a later process onto the dead one's sweep/schedule state. */
    if (root) {
        for (int i = 0; i < s->m2_gr_clients_n; ) {
            if (s->m2_gr_clients[i] == fClient) {
                s->m2_gr_clients[i] = s->m2_gr_clients[s->m2_gr_clients_n - 1];
                s->m2_gr_clients_n--; dropped++; continue;
            }
            i++;
        }
        /* #14 EARLY-ARM list: same lifecycle as m2_gr_clients[]. */
        for (int i = 0; i < s->m2_user_clients_n; ) {
            if (s->m2_user_clients[i] == fClient) {
                s->m2_user_clients[i] = s->m2_user_clients[s->m2_user_clients_n - 1];
                s->m2_user_clients_n--; dropped++; continue;
            }
            i++;
        }
    }
    for (int i = 0; i < s->m2_tsg_sched_n; ) {
        bool hit = root ? (s->m2_tsg_sched[i].client == fClient)
                        : (s->m2_tsg_sched[i].client == fClient &&
                           s->m2_tsg_sched[i].tsg == fObj);
        if (hit) {
            s->m2_tsg_sched[i] = s->m2_tsg_sched[s->m2_tsg_sched_n - 1];
            s->m2_tsg_sched_n--; dropped++; continue;
        }
        i++;
    }
    /* chan_vas[] (no client key): drop the directly-freed VASpace handle, plus any whose
     * handle was referenced by a just-removed devvas entry of this client. */
    for (int i = 0; i < s->chan_vas_n; ) {
        bool hit = (s->chan_vas[i].hvas == fObj);
        for (int j = 0; !hit && j < drop_hvas_n; j++) {
            if (s->chan_vas[i].hvas == drop_hvas[j]) { hit = true; }
        }
        if (hit) {
            s->chan_vas[i] = s->chan_vas[s->chan_vas_n - 1];
            s->chan_vas_n--; dropped++; continue;
        }
        i++;
    }
    /* ── #14 P0 reap hygiene (plan §2 rows 4,5,13,17,18,19,22,28,29): on a client-ROOT
     * free, purge the freed client's entries from the never-reaped client-keyed tables so
     * process churn cannot leak slots or alias a later process that reuses the same RM
     * handle VALUE (RM reuses client handle values across process lifetimes).  Root-free
     * only: mid-run handle frees (#12's sequential 2-context case keeps ONE client alive)
     * never take these paths, so single-process behavior is byte-identical. */
    if (root) {
        int reaped = 0;
        /* rows 4/5 (m2_cli_vas, va_map) + 28/29 (m2_objs, m2_gpga) are RESOLUTION/
         * BACKING state: reaping them AT the root free hangs the dying context's own
         * residual polls (bench-proven, cupctx2_min CTX2 destroy).  Enqueue the client
         * for the DEFERRED reap at the next GSP queue re-handshake instead. */
        if (s->m2_reap_pend_n < (int)ARRAY_SIZE(s->m2_reap_pend)) {
            bool pend_seen = false;
            for (int i = 0; i < s->m2_reap_pend_n; i++) {
                if (s->m2_reap_pend[i] == fClient) { pend_seen = true; break; }
            }
            if (!pend_seen) { s->m2_reap_pend[s->m2_reap_pend_n++] = fClient; }
        }
        /* row 17 — m2_tsgeng[]: keyed by the P0 client field (lookups stay tsg-keyed). */
        for (int i = 0; i < s->m2_tsgeng_n; ) {
            if (s->m2_tsgeng[i].client == fClient) {
                s->m2_tsgeng[i] = s->m2_tsgeng[s->m2_tsgeng_n - 1];
                s->m2_tsgeng_n--; reaped++; continue;
            }
            i++;
        }
        /* row 18 — m2_subdev[]. */
        for (int i = 0; i < s->m2_subdev_n; ) {
            if (s->m2_subdev[i].client == fClient) {
                s->m2_subdev[i] = s->m2_subdev[s->m2_subdev_n - 1];
                s->m2_subdev_n--; reaped++; continue;
            }
            i++;
        }
        /* row 19 — m2_grmap[] (bookkeeping only; the host-side virtmem object was/is
         * freed by the forwarded root free itself). */
        for (int i = 0; i < s->m2_grmap_n; ) {
            if (s->m2_grmap[i].client == fClient) {
                s->m2_grmap[i] = s->m2_grmap[s->m2_grmap_n - 1];
                s->m2_grmap_n--; reaped++; continue;
            }
            i++;
        }
        /* row 22 — m2_user_ce_clients[]. */
        for (int i = 0; i < s->m2_user_ce_n; ) {
            if (s->m2_user_ce_clients[i] == fClient) {
                s->m2_user_ce_clients[i] = s->m2_user_ce_clients[s->m2_user_ce_n - 1];
                s->m2_user_ce_n--; reaped++; continue;
            }
            i++;
        }
        /* row 13 — m2_cmap[] guest->host client remap: LAST (the shadow-forwarded host
         * free of this root already consumed the mapping earlier in service_cmdq).  A
         * later process reusing the guest value then mints a FRESH host client instead
         * of aliasing the dead one (mint is monotonic, so no handle reuse). */
        for (int i = 0; i < s->m2_cmap_n; ) {
            if (s->m2_cmap[i].g == fClient) {
                s->m2_cmap[i] = s->m2_cmap[s->m2_cmap_n - 1];
                s->m2_cmap_n--; reaped++; continue;
            }
            i++;
        }
        if (reaped) {
            qemu_log("nvkvm-gpu[%s] #14 P0 root-free reap client=0x%08x (user=%d): "
                     "%d entries (tsgeng=%d subdev=%d grmap=%d cmap=%d; heavy DEFERRED "
                     "pend=%d)\n", s->chip->name, fClient, user_root, reaped,
                     s->m2_tsgeng_n, s->m2_subdev_n, s->m2_grmap_n, s->m2_cmap_n,
                     s->m2_reap_pend_n);
        }
    }
    /* #12 NOTE (bench-disproven 2026-06-18): a naive "release this client's GPGA
     * overlays on its root-free" is UNSAFE and does NOT fix the hang.  The CeUtils scrub
     * channel (client 0xc1e00007) reads its ring/finishPayload from an emulated-FB phys
     * (e.g. 0x3130000) OWNED by a different UVM client (0xc1d00003); releasing the owner's
     * overlay on its free yanks the backing out from under the still-polling scrub — the
     * exact cross-client SHARING the address-table model forbids without a refcount over
     * ALL referencing clients/VAS.  And it's moot anyway: the forge writes the correct
     * monotonic value to finFB but the guest reads finishPayload through a non-trapping
     * KVM memslot whose backing is not coherent with fb_write, so ce_utils.c:349 still
     * fires.  Real fix = give the scrub its own coherent backing+memslot, OR execute the
     * scrub CE on the host so the real SET_SEMAPHORE writes the page the guest reads.
     * See docs/design/mode2_2nd_context_hang.md (UPDATE cont. 3). */
    if (s->trace && dropped) {
        qemu_log("nvkvm-gpu[%s] M5.49 ctx-free drop %s fClient=0x%08x fObj=0x%08x: %d entries "
                 "(chans=%d chanbuf=%d devvas=%d cvas=%d chanvas=%d)\n", s->chip->name,
                 root ? "ROOT" : "obj", fClient, fObj, dropped, s->chan_n, s->m2_chanbuf_n,
                 s->m2_devvas_n, s->m2_cvas_n, s->chan_vas_n);
    }
}

/* #14 P0 DEFERRED reap (see m2_reap_pend[]): purge the resolution/backing state of
 * clients whose ROOT was freed, at the GSP queue RE-HANDSHAKE — the next context/
 * process boot, after the fn-47 idle-release, when no guest poller can reference
 * these entries (immediate reap at the free hung cupctx2_min's residual teardown
 * polls; and the 2026-06-18 disproof shows kernel channels can read a freed
 * client's overlays mid-run — both impossible here: the GPU was idle-released).
 * Bookkeeping-only: host-side objects were freed by the forwarded frees / will be
 * owned by the per-proc isolate teardown (P2).  m2_gpga[].obj_idx indexes
 * m2_objs[], so obj removal re-points the swapped-in last entry + dirties the
 * sorted index. */
static void nvkvm_m2_reap_dead(NvkvmGpuEmul *s)
{
    if (!s->m2_reap_pend_n) { return; }
    int reaped = 0;
    for (int c = 0; c < s->m2_reap_pend_n; c++) {
        uint32_t cl = s->m2_reap_pend[c];
        for (int i = 0; i < s->m2_cli_vas_n; ) {              /* row 4 */
            if (s->m2_cli_vas[i].client == cl) {
                s->m2_cli_vas[i] = s->m2_cli_vas[s->m2_cli_vas_n - 1];
                s->m2_cli_vas_n--; reaped++; continue;
            }
            i++;
        }
        for (int i = 0; i < s->va_map_n; ) {                  /* row 5 */
            if (s->va_map[i].client == cl) {
                s->va_map[i] = s->va_map[s->va_map_n - 1];
                s->va_map_n--; reaped++; continue;
            }
            i++;
        }
        for (int i = 0; i < s->m2_objs_n; ) {                 /* rows 28/29 */
            if (s->m2_objs[i].client != cl) { i++; continue; }
            for (int g = 0; g < s->m2_gpga_n; ) {
                if (s->m2_gpga[g].obj_idx == i) {
                    s->m2_gpga[g] = s->m2_gpga[s->m2_gpga_n - 1];
                    s->m2_gpga_n--; reaped++; continue;
                }
                g++;
            }
            int last = s->m2_objs_n - 1;
            s->m2_objs[i] = s->m2_objs[last];
            s->m2_objs_n--; reaped++;
            for (int g = 0; g < s->m2_gpga_n; g++) {
                if (s->m2_gpga[g].obj_idx == last) { s->m2_gpga[g].obj_idx = i; }
            }
            /* re-check slot i (swapped-in entry) */
        }
    }
    s->m2_gpga_idx_dirty = true;
    qemu_log("nvkvm-gpu[%s] #14 P0 deferred reap @re-handshake: %d clients, %d entries "
             "(cli_vas=%d va_map=%d objs=%d gpga=%d)\n", s->chip->name,
             s->m2_reap_pend_n, reaped, s->m2_cli_vas_n, s->va_map_n, s->m2_objs_n,
             s->m2_gpga_n);
    s->m2_reap_pend_n = 0;
}

/* ── DIAG (address-virtualization bring-up, removable) ──────────────────────
 * Decode the alloc/control RPCs so we can build the GPU-VA -> physical side
 * table from the GSP_RM_ALLOC memory descriptors and GSP_RM_CONTROL map cmds.
 * fn=103 (GSP_RM_ALLOC) body: hClient@80, hParent@84, hObject@88, hClass@92,
 * paramsSize@100, params@112.  fn=76 (GSP_RM_CONTROL) body: hClient@80,
 * hObject@84, cmd@88, status@92, paramsSize@96, params@120. */
static void nvkvm_diag_hex(const char *tag, const char *chip, uint32_t key,
                           const uint8_t *p, int n)
{
    char line[256]; int o = 0;
    o += snprintf(line + o, sizeof(line) - o, "nvkvm-gpu[%s] DIAG %s key=0x%x:",
                  chip, tag, key);
    for (int i = 0; i < n && o < (int)sizeof(line) - 4; i++) {
        o += snprintf(line + o, sizeof(line) - o, "%s%02x",
                      (i % 8 == 0) ? " " : "", p[i]);
    }
    qemu_log("%s\n", line);
}

/* Scan a params blob for any 64-bit value within [base, base+span) and log the
 * offset + value.  Used to find which RPC carries the GPFIFO GPU-VA so we learn
 * the op that establishes the mapping (no struct-layout guessing). */
static void nvkvm_diag_scan_va(NvkvmGpuEmul *s, const char *what, uint32_t fn,
                               uint32_t cmd_or_class, const uint8_t *params,
                               int psize, uint64_t base, uint64_t span)
{
    int lim = psize < 1024 ? psize : 1024;
    for (int o = 0; o + 8 <= lim; o += 4) {
        uint64_t v = ldq_le_p(params + o);
        if (v >= base && v < base + span) {
            qemu_log("nvkvm-gpu[%s] DIAG %s fn=%u cc=0x%08x VAhit@+%d val=0x%llx\n",
                     s->chip->name, what, fn, cmd_or_class, o,
                     (unsigned long long)v);
        }
    }
}

/* Broad scan: log any 64-bit value that looks like a GPU VA (0x1.2-5.xx_xxxx)
 * or a sysmem GPA near the channel-semaphore region (0x1.0-1.8_xxxx_xxxx).  This
 * reveals EVERY VA<->phys association the guest communicates, so we can find
 * where the UVM channel's GPFIFO/pushbuffer/semaphore sysmem GPA is conveyed. */
static void nvkvm_diag_broad(NvkvmGpuEmul *s, const char *what, uint32_t cc,
                             const uint8_t *params, int psize)
{
    static uint32_t budget = 600;
    int lim = psize < 1024 ? psize : 1024;
    for (int o = 0; o + 8 <= lim; o += 4) {
        uint64_t v = ldq_le_p(params + o);
        bool va  = (v >= 0x120000000ull && v < 0x500000000ull);
        bool gpa = (v >= 0x100000000ull && v < 0x180000000ull);
        bool uvmsema = (v >= 0x121000000ull && v < 0x121100000ull);
        if ((va || gpa || uvmsema) && budget-- > 0) {
            qemu_log("nvkvm-gpu[%s] DIAG SCAN %s cc=0x%08x +%d = 0x%llx%s\n",
                     s->chip->name, what, cc, o, (unsigned long long)v,
                     uvmsema ? " [UVM-VA]" : va ? " [VA]" : " [GPA]");
        }
    }
}

static void nvkvm_diag_rpc(NvkvmGpuEmul *s, const uint8_t *cmd, uint32_t fn)
{
    if (!s->trace) {
        return;                 /* verbose bring-up decode — gated behind -trace */
    }
    if (fn == 103) {                                  /* GSP_RM_ALLOC */
        uint32_t hClient = ldl_le_p(cmd + 80), hParent = ldl_le_p(cmd + 84);
        uint32_t hObject = ldl_le_p(cmd + 88), hClass = ldl_le_p(cmd + 92);
        uint32_t psize   = ldl_le_p(cmd + 100);
        const uint8_t *params = cmd + 112;
        qemu_log("nvkvm-gpu[%s] DIAG ALLOC class=0x%04x hClient=0x%08x "
                 "hParent=0x%08x hObject=0x%08x psize=%u\n", s->chip->name,
                 hClass, hClient, hParent, hObject, psize);
        /* Memory classes: dump the descriptor head (base/size/aperture live here
         * for OS_DESC/SYSTEM/LOCAL_USER/VIRTUAL). */
        if (hClass == 0x003eu || hClass == 0x0040u || hClass == 0x0071u ||
            hClass == 0x50a0u || hClass == 0x90f1u || hClass == 0x00deu ||
            hClass == 0x007eu || hClass == 0x0070u) {
            nvkvm_diag_hex("ALLOCMEM", s->chip->name, hClass, params,
                           psize < 64 ? psize : 64);
        }
        nvkvm_diag_broad(s, "ALLOC", hClass, params, (int)psize);
        /* Full channel-params dump: reveals hVASpace + all memory descriptors
         * (instance/userd/ramfc/mthdbuf) so we see where the GPFIFO/sema live. */
        if (hClass == 0xc56fu || hClass == 0xc36fu) {
            int n = psize < 384 ? (int)psize : 384;
            for (int o = 0; o < n; o += 32) {
                nvkvm_diag_hex("CHANPARAMS", s->chip->name, (uint32_t)o,
                               params + o, (n - o) < 32 ? (n - o) : 32);
            }
        }
        /* Scan any alloc params for a reference to a known channel's GPFIFO VA. */
        for (int i = 0; i < s->chan_n; i++) {
            nvkvm_diag_scan_va(s, "ALLOC", fn, hClass, params, (int)psize,
                               s->chans[i].gpfifo_va & ~0xFFFFFull, 0x100000);
        }
    } else if (fn == 76) {                            /* GSP_RM_CONTROL */
        uint32_t hObject = ldl_le_p(cmd + 84), ctrl = ldl_le_p(cmd + 88);
        uint32_t psize   = ldl_le_p(cmd + 96);
        const uint8_t *params = cmd + 120;
        nvkvm_diag_broad(s, "CTRL", ctrl, params, (int)psize);
        /* Scan control params for any known channel GPFIFO VA neighborhood. */
        for (int i = 0; i < s->chan_n; i++) {
            uint64_t b = s->chans[i].gpfifo_va & ~0xFFFFFull;
            int lim = psize < 1024 ? (int)psize : 1024;
            for (int o = 0; o + 8 <= lim; o += 4) {
                uint64_t v = ldq_le_p(params + o);
                if (v >= b && v < b + 0x100000) {
                    qemu_log("nvkvm-gpu[%s] DIAG CTRL cmd=0x%08x hObject=0x%08x "
                             "VAhit@+%d val=0x%llx psize=%u\n", s->chip->name,
                             ctrl, hObject, o, (unsigned long long)v, psize);
                    nvkvm_diag_hex("CTRLwin", s->chip->name, ctrl,
                                   params + (o > 16 ? o - 16 : 0), 64);
                    break;
                }
            }
        }
    }
}

/* Record (or update) a GPU-VA -> physical mapping in the #2 side-table.  Keyed
 * by (client, va): a re-promote of the same VA replaces the entry. */
static void nvkvm_record_va_map(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                uint64_t phys, uint64_t size, bool sys)
{
    if (!va || !size) {
        return;
    }
    for (int i = 0; i < s->va_map_n; i++) {
        struct nvkvm_va_map *m = &s->va_map[i];
        if (m->client == client && m->va == va) {
            m->phys = phys; m->size = size; m->sys = sys;
            return;
        }
    }
    if (s->va_map_n >= NVKVM_MAX_MAPS) {
        return;                 /* table full — DoS-bounded; oldest stay */
    }
    struct nvkvm_va_map *m = &s->va_map[s->va_map_n++];
    m->client = client; m->va = va; m->phys = phys; m->size = size; m->sys = sys;
    qemu_log("nvkvm-gpu[%s] M5: va_map[%d] client=0x%08x va=0x%llx -> %s "
             "phys=0x%llx size=0x%llx\n", s->chip->name, s->va_map_n - 1, client,
             (unsigned long long)va, sys ? "SYS" : "FB",
             (unsigned long long)phys, (unsigned long long)size);
}

/* Parse NV2080_CTRL_CMD_GPU_PROMOTE_CTX (0x2080012b) and fold its context-buffer
 * entries into the #2 side-table.  Params @cmd+120 (GSP_RM_CONTROL body):
 *   hChanClient@+12, entryCount@+40, promoteEntry[]@+48 (32B each:
 *   gpuPhysAddr@0, gpuVirtAddr@8, size@16, physAttr@24, bufferId@28,
 *   bInitialize@30, bNonmapped@31).  physAttr[1:0]: 0=VIDMEM, 1/2=SYSMEM. */
static void nvkvm_snoop_promote_ctx(NvkvmGpuEmul *s, const uint8_t *cmd)
{
    const uint8_t *p = cmd + 120;
    uint32_t client = ldl_le_p(p + 12);
    uint32_t ec     = ldl_le_p(p + 40);
    if (ec > 64) {              /* NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES=20;
                                 * clamp generously, never trust guest count. */
        ec = 64;
    }
    for (uint32_t i = 0; i < ec; i++) {
        const uint8_t *e = p + 48 + (uint64_t)i * 32;
        uint64_t phys = ldq_le_p(e + 0), va = ldq_le_p(e + 8), sz = ldq_le_p(e + 16);
        uint32_t physAttr = ldl_le_p(e + 24);
        uint32_t bufferId = ldl_le_p(e + 28);   /* NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_* */
        uint8_t  bNonmapped = e[31];
        /* M5.31 DIAG: name each PROMOTE_CTX buffer (MAIN=0/PM/PATCH/.. per ctx0080) so the
         * page the cuCtxCreate poll walks to (FB ~0x2efa6xxx) is identified by type. */
        qemu_log("nvkvm-gpu[GA106] M5.31 PROMOTE entry client=0x%08x bufId=%u va=0x%llx "
                 "phys=0x%llx sz=0x%llx %s%s\n", client, bufferId,
                 (unsigned long long)va, (unsigned long long)phys, (unsigned long long)sz,
                 (physAttr & 0x3u) ? "SYS" : "FB", bNonmapped ? " NONMAPPED" : "");
        if (!va || !sz || bNonmapped) {
            continue;           /* unmapped/phys-only entries don't enter the VAS */
        }
        nvkvm_record_va_map(s, client, va, phys, sz, (physAttr & 0x3u) != 0);
    }
}

/* M4: service the CPU->GSP command queue.  Called when the driver rings the cmd
 * QUEUE_HEAD doorbell (0x110c00).  For each new command, echo a response
 * (same function, rpc_result=NV_OK) onto the status queue so _issueRpcAndWait
 * returns.  Init RPCs are mostly SET_* and accept an NV_OK echo. */
static void nvkvm_m2_shadow_fwd(NvkvmGpuEmul *s, const uint8_t *cmd, uint32_t fn); /* M5.1 fwd-decl */
static int nvkvm_m2_control1(NvkvmGpuEmul *s, uint32_t hClient, uint32_t hObject,
                             uint32_t cmd, void *params, uint32_t psize, uint32_t *st); /* M5.3 fwd-decl */
static void nvkvm_m2_back_channel_userd(NvkvmGpuEmul *s, uint32_t hClient,
                                        uint32_t chanObj, uint8_t *auxbuf,
                                        uint32_t psize); /* M5.4 fwd-decl */
static void nvkvm_m2_mapdma_selftest(NvkvmGpuEmul *s, uint32_t hClient); /* M5.5 fwd-decl */
static uint64_t nvkvm_m2_gpa_to_stub_va(NvkvmGpuEmul *s, uint64_t gpa); /* M6.2 fwd-decl */
static int nvkvm_m2_os_descriptor(NvkvmGpuEmul *s, uint32_t client, uint32_t device,
                                  uint32_t hMem, uint64_t stub_va, uint64_t size,
                                  uint32_t *st); /* M6.2 fwd-decl */
static void nvkvm_m2_osdesc_selftest(NvkvmGpuEmul *s, uint32_t hClient); /* M6.2 fwd-decl */
static uint32_t nvkvm_m2_grmapper(NvkvmGpuEmul *s, uint32_t client); /* M5.7 fwd-decl */
/* M5.49b: is `client` one of libcuda's CE-copy clients (the user-observable data
 * path)?  These hit the grmapper FRESH-VAS fallback; UVM/init clients do not. */
static inline bool nvkvm_m2_is_user_ce(NvkvmGpuEmul *s, uint32_t client)
{
    if (!client) { return false; }
    for (int i = 0; i < s->m2_user_ce_n; i++) {
        if (s->m2_user_ce_clients[i] == client) { return true; }
    }
    return false;
}
static int nvkvm_m2_cvas_get(NvkvmGpuEmul *s, uint32_t client, uint32_t tsg); /* M5.28 fwd-decl */
static bool nvkvm_m2_populate_cvas(NvkvmGpuEmul *s, struct nvkvm_chan_entry *c); /* M5.28 fwd-decl */
static int nvkvm_m2_map_dma(NvkvmGpuEmul *s, uint32_t hClient, uint32_t hDevice,
                            uint32_t hVas, uint32_t hMemory, uint64_t offset,
                            uint64_t length, bool fixed, uint64_t va,
                            uint32_t *st, uint64_t *out_va); /* M5.5 fwd-decl */
static bool nvkvm_m2_back_and_map(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                  uint64_t phys, uint64_t size, bool copy_content,
                                  const char *label); /* M5.7 */
static void nvkvm_m2_doorbell_setup(NvkvmGpuEmul *s, uint32_t client); /* M5.8 fwd-decl */
static void nvkvm_m2_exec_doorbell(NvkvmGpuEmul *s); /* M5.9 fwd-decl */
static void nvkvm_m2_forward_promote_ctx(NvkvmGpuEmul *s, const uint8_t *cmd); /* M6.4 fwd-decl */
static bool nvkvm_m2_back_and_map_sys(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                      uint64_t gpa, uint64_t size); /* M6.5 fwd-decl */
static void nvkvm_m2_enum_gr_sysmem(NvkvmGpuEmul *s, uint32_t client); /* M6.5 fwd-decl */
static int nvkvm_m2_gpga_obj(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                             uint64_t gpga, uint64_t size); /* M7 R2 fwd-decl (Phase A) */
static int nvkvm_m2_gpga_obj_ex(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                uint64_t gpga, uint64_t size, bool gpu_only); /* CE-fwd P1 */
static void nvkvm_m2_capture_devinfo(NvkvmGpuEmul *s); /* M14 fwd-decl */
static bool nvkvm_chan_sem_wr32(NvkvmGpuEmul *s, uint64_t va, uint32_t payload,
                                uint64_t *out_redir); /* M5.18 fwd-decl */
static bool nvkvm_m2_va_seen(NvkvmGpuEmul *s, uint32_t client, uint64_t va); /* M5.19 fwd-decl */
/* #14 fwd-decls: dup-edge (fn=21) ownership helpers — see definitions near
 * nvkvm_chan_own_pdb_rs. */
static bool nvkvm_m2_vas_client_match(NvkvmGpuEmul *s, uint32_t vas_client,
                                      uint32_t vas_hobj, uint32_t client);
static uint32_t nvkvm_m2_vas_dup_owner(NvkvmGpuEmul *s, uint32_t vas_client,
                                       uint32_t vas_hobj);
static bool nvkvm_m2_dup_src_client(NvkvmGpuEmul *s, uint32_t client);
static bool nvkvm_m2_vas_foreign(NvkvmGpuEmul *s, int v, uint32_t client);
/* nvkvm_m2_is_gr_client / _is_user_client are already forward-declared above
 * (the P0-reap block).  Re-declaring them here is a -Werror=redundant-decls
 * build failure under the bench's QEMU 9.2 configure — which is why every
 * emulator source from 3710b8e onward has never been compiled on it. */
static bool nvkvm_m2_multiproc(NvkvmGpuEmul *s);
static void nvkvm_bar0_write_inner(void *opaque, hwaddr off, uint64_t val,
                                   unsigned size, bool from_guest); /* #14 poll-kick fwd-decl */
static bool nvkvm_m2_tsg_sched_check(NvkvmGpuEmul *s, uint32_t client, uint32_t tsg);
static void nvkvm_m2_tsg_sched_mark(NvkvmGpuEmul *s, uint32_t client, uint32_t tsg);
static uint32_t nvkvm_m2_pdb_gr_owner(NvkvmGpuEmul *s, uint64_t pdb);
static uint64_t nvkvm_chan_own_pdb_rs(NvkvmGpuEmul *s, bool *out_root_sys); /* #12 L3b fwd-decl */
static void nvkvm_m2_ce_fb_write_hook(NvkvmGpuEmul *s, uint64_t dst, uint64_t len); /* #13 fwd-decl */
static void nvkvm_m2_cpt_sync_at_release(NvkvmGpuEmul *s); /* #13 fwd-decl */
static uint32_t nvkvm_phys_rd32(NvkvmGpuEmul *s, uint64_t phys, bool sys);  /* #12 L3b fwd-decl */
static void nvkvm_phys_wr32(NvkvmGpuEmul *s, uint64_t phys, bool sys, uint32_t v); /* #12 L3b fwd-decl */

static void nvkvm_m3_service_cmdq(NvkvmGpuEmul *s)
{
    PCIDevice *pdev = &s->parent_obj;
    if (!s->q_ready || !s->q_msgcount) {
        return;
    }
    uint8_t wpb[4];
    if (nvkvm_dmar(pdev, s->q_shmem + s->q_cmd_base + 16, wpb, 4) != MEMTX_OK) {
        return;
    }
    uint32_t cmd_writeptr = ldl_le_p(wpb);
    if (s->trace && s->cmd_readptr != cmd_writeptr) {
        qemu_log("nvkvm-gpu[%s] M4: cmdq enter cmd_wp=%u cmd_rp=%u (inflight=%d) "
                 "stat_wp=%u stat_seq=%u\n", s->chip->name, cmd_writeptr,
                 s->cmd_readptr, (int)(cmd_writeptr - s->cmd_readptr),
                 s->stat_writeptr, s->stat_seqnum);
    }
    /* msgq pointers are MODULO msgCount (msgq.c wraps writePtr/readPtr at
     * msgCount), not absolute.  Bound the loop by msgCount so a desync can never
     * spin forever (pending elements < msgCount by construction). */
    uint32_t guard = 0;
    while (s->cmd_readptr != cmd_writeptr && guard++ < s->q_msgcount) {
        uint32_t slot = s->cmd_readptr % s->q_msgcount;
        uint8_t cmd[4096];
        uint64_t gpa = s->q_shmem + s->q_cmd_base + s->q_cmd_entryoff +
                       (uint64_t)slot * s->q_msgsize;
        if (nvkvm_dmar(pdev, gpa, cmd, sizeof(cmd)) != MEMTX_OK) {
            break;
        }
        /* ── #90 GSP-D6: WITNESS the continuation elements (RECORDER ONLY) ──
         *
         * A GSP_MSG_QUEUE message spans ceil((48 + rpc.length) / 4096) queue
         * elements.  The C acts on element 0 ONLY and consumes the rest
         * silently (see the elemCount advance at the bottom of this loop), so
         * it never READS them — and what is never read is never recorded.  A
         * replay of a *correct* implementation, which does read them, then
         * cannot be answered from this trace: it stops dead at the first
         * multi-element command (`GSP_RM_CONTROL` rpc.length=8276, elemCount=3
         * — record 141976 of `cap1_coldboot_hermetic`), because the capture
         * holds no observation of the continuation slots while they were live.
         *
         * So read them here purely so nvkvm_dmar's recorder chokepoint
         * witnesses them, and then THROW THE BYTES AWAY.  This is deliberately
         * NOT a fix for GSP-D6: the C still acts on element 0 alone and still
         * produces byte-identical replies.  GSP-D6 remains a real, recorded
         * divergence; it is merely now an *observable* one.
         *
         * Why the guest cannot tell:
         *   - gated on nvkvm_rec_on() (the m2rec property), so a non-capture
         *     run behaves exactly as before.  The cost when off is a load of a
         *     file-static bool and one predictable branch — the same guard every
         *     other recorder call site in this file uses;
         *   - pci_dma_read is a pure read of guest RAM: no dirty bits, no
         *     queue pointer moves, no reply, no status, no state of `s`
         *     touched.  `cont` is a dead local;
         *   - the addresses are (cmd_readptr + i) % q_msgcount, i.e. strictly
         *     inside the same ring the guest itself allocated and whose slot 0
         *     we just read — the read can never escape into an MMIO region
         *     that element 0's own read could not already have hit;
         *   - a failing read is ignored rather than breaking the loop, so even
         *     a bad address changes no control flow.
         * The only guest-visible effect would be timing, which the guest has no
         * architected way to observe here (the RPC is completed by our reply,
         * not by a deadline). */
        if (nvkvm_rec_on() && (nvkvm_rec_mask() & NVKVM_REC_M_GUEST_RD)) {
            uint64_t wmsglen = 48ull + (uint64_t)ldl_le_p(cmd + 56);
            uint64_t welems  = (wmsglen + 4095ull) / 4096ull;
            if (welems > s->q_msgcount) {
                welems = s->q_msgcount;   /* a garbage rpc.length must not run away */
            }
            for (uint64_t wi = 1; wi < welems; wi++) {
                uint8_t cont[4096];
                uint32_t cslot = (uint32_t)((s->cmd_readptr + wi) % s->q_msgcount);
                uint64_t cgpa = s->q_shmem + s->q_cmd_base + s->q_cmd_entryoff +
                                (uint64_t)cslot * s->q_msgsize;
                /* return value deliberately ignored; bytes deliberately dropped */
                (void)nvkvm_dmar(pdev, cgpa, cont, sizeof(cont));
            }
        }
        uint32_t fn = ldl_le_p(cmd + 60);
        /* Async one-way init RPCs expect NO response — echoing them shows up in
         * the driver as "Unexpected RPC event" and desyncs the seqNum.  Consume
         * them silently.  (72=GSP_SET_SYSTEM_INFO, 73=SET_REGISTRY, sent by
         * kgspSendInitRpcs before GSP-RM is up.)  Grow this list as the trace
         * reveals more one-way functions. */
        bool async = (fn == 72 || fn == 73);
        if (s->trace) {
            qemu_log("nvkvm-gpu[%s] M4: cmd fn=%u seq=%u -> %s\n",
                     s->chip->name, fn, ldl_le_p(cmd + 36),
                     async ? "async (no response)" : "echo NV_OK");
        }
        nvkvm_diag_rpc(s, cmd, fn);   /* DIAG: decode alloc/control for side-table */
        /* #2 side-table: capture GPU-VA -> physical from PROMOTE_CTX (the GSP-RM
         * map op).  This is what makes GSP-managed-VAS channels (UVM) resolvable
         * without leaf PTEs in our FB. */
        if (fn == 76 && ldl_le_p(cmd + 88) == 0x2080012bu) {
            /* Keep the side-table snoop: GPU-VA->physical capture is legitimate
             * state recovery (makes GSP-managed/UVM VASes resolvable). */
            nvkvm_snoop_promote_ctx(s, cmd);
            /* M6.5: do NOT replay PROMOTE_CTX on the host.  It is a ROUTE_TO_PHYSICAL /
             * GSP-internal (Case-2) control with no userspace equivalent — an unprivileged
             * stub issuing it gets NV_ERR_INSUFFICIENT_PERMISSIONS (0x1b).  Its EFFECT (the
             * host channel's GR context being promoted) is already achieved by the Case-1
             * forwarding: shadow_fwd allocated the host channel + NVC7C0 compute object, and
             * the HOST kernel-RM promoted that host channel's context itself.  So this is
             * ack-only to the guest (the post-PROMOTE_CTX completion poll is satisfied
             * elsewhere).  See docs/design/mode2_forwarding_model.md (the M6.4 forward was a
             * wrong-layer replay; removed). */
        }
        /* M5.1a: shadow-forward the guest's actual RM alloc stream to the real
         * host GPU (gated; non-disruptive — guest still uses the faked response). */
        if (s->m2fwd) {
            nvkvm_m2_shadow_fwd(s, cmd, fn);
        }
        /* fn=47 UNLOADING_GUEST_DRIVER: the guest is tearing down.  On real HW the
         * teardown runs Booter Unload, which brings WPR2 back DOWN.  We don't run
         * Booter Unload, so mirror its effect: clear the GSP-boot state (WPR2,
         * RISCV-active, FWSEC-ran).  Without this, a re-insmod (no QEMU restart)
         * sees WPR2 still up and _kgspBootGspRm bails with NV_ERR_INVALID_STATE
         * ("unexpected WPR2 already up") — a false cascade that masks the real
         * init failure and forces a full VM/QEMU restart between iterations. */
        if (fn == 47) {
            /* UNLOADING_GUEST_DRIVER.  TWO distinct triggers share this RPC: a real
             * driver unload (rmmod; a later insmod re-runs the full GSP boot), AND a
             * GPU-idle release when the last client/context exits while the kernel
             * module stays loaded (the next process's context re-acquires the GPU).  In
             * BOTH cases the guest re-runs the queue handshake (re-writes the status-
             * queue tx header), so reset here ONLY the boot state that gates re-detection:
             * WPR2 down + bootargs/q_ready.  Without that, bootargs_dumped/q_ready stay
             * set, the tx header is never re-detected, and GspStatusQueueInit/msgqRxLink
             * times out (kernel_gsp_tu102.c:570) — the original reload bug.
             *
             * M5.50 (2026-06-16): do NOT reset the queue COUNTERS (stat_seqnum/
             * stat_writeptr/cmd_readptr) here.  The guest sent THIS fn-47 at the current
             * rxSeqNum and polls (rpcRecvPoll) for its ack at that seqNum.  Zeroing
             * stat_seqnum before the ack is posted (the !async response path below) sends
             * the ack at seqNum 0 -> guest sees "Bad sequence number", never accepts it,
             * times out (Xid 119) — and that corrupts teardown so the NEXT context
             * inherits a broken GPU (the sequential/multi-process #12 hang).  The counters
             * are reset at the re-handshake (the tx-header-write path) where BOTH sides
             * reset rxSeqNum=0 together — the only moment that keeps them in lockstep. */
            s->fwsec_ran = false;       /* WPR2 down (booter-unload effect)      */
            s->gsp_suspended = true;    /* MAILBOX0 -> SUSPENDED for the close poll */
            nvkvm_gsp_falcon_sync(s);   /* M5.64: reflect MAILBOX0 suspend value into rom-device RAM */
            s->bootargs_dumped = false; /* re-dump bootargs on the next boot         */
            s->q_ready         = false; /* re-detect the queue handshake on re-init  */
            qemu_log("nvkvm-gpu[%s] M4: UNLOADING -> WPR2 down + GSP suspended "
                     "(queue counters preserved for the in-flight fn-47 ack)\n",
                     s->chip->name);
        }
        /* M5: snoop GSP_RM_ALLOC (fn 103) for a *_CHANNEL_GPFIFO_A alloc so we can
         * locate the GPFIFO ring when the doorbell rings.  rpc_gsp_rm_alloc body
         * @cmd+80: hClass@+12 (cmd+92), paramsSize@+20 (cmd+100), params@+32
         * (cmd+112).  NV_CHANNEL_ALLOC_PARAMS: gpFifoOffset@+8 (cmd+120, u64),
         * gpFifoEntries@+16 (cmd+128). Classes: PASCAL..BLACKWELL _GPFIFO_A all
         * end in 0x6F with the family nibble (C0/C3/C4/C5/C8/C9). */
        /* M5: snoop VASPACE_COPY_SERVER_RESERVED_PDES (0x90f10106) — the CPU
         * hands GSP its page-directory level phys addrs for a VA range.  Record
         * levels[0].physAddress (root PDB) + [virtAddrLo,virtAddrHi] so the
         * doorbell can root the channel GMMU walk (the GSP-managed instblk is
         * empty in our FB).  body @cmd+80: control cmd@+88 (cmd+88); params@cmd+120;
         * virtAddrLo@cmd+136, virtAddrHi@cmd+144, levels[0].physAddress@cmd+160. */
        if (fn == 76 && ldl_le_p(cmd + 88) == 0x90f10106u) {
            if (s->chan_vas_n < (int)ARRAY_SIZE(s->chan_vas)) {
                int k = s->chan_vas_n++;
                s->chan_vas[k].hvas = ldl_le_p(cmd + 84);   /* control hObject = VASpace */
                s->chan_vas[k].client = ldl_le_p(cmd + 80); /* GSP_RM_CONTROL hClient (#12-L3) */
                s->chan_vas[k].pdb  = ldq_le_p(cmd + 160);  /* levels[0].physAddress */
                s->chan_vas[k].root_sys = false;            /* FB-rooted (GSP-client) */
                s->chan_vas[k].uvm = false;                 /* RESERVED_PDES, not the UVM handover */
                nvkvm_m2_cli_vas_add(s, s->chan_vas[k].client, s->chan_vas[k].pdb, false);
                /* #14: a VAS captured under a dup handle also belongs to the dup's
                 * SOURCE client — key the sticky sema-resolution root under the true
                 * owner so that owner's channels resolve their semas in their OWN
                 * address space (two processes share the capture client). */
                {
                    uint32_t own14 = nvkvm_m2_vas_dup_owner(s, s->chan_vas[k].client,
                                                            s->chan_vas[k].hvas);
                    if (own14) { nvkvm_m2_cli_vas_add(s, own14, s->chan_vas[k].pdb, false); }
                    /* #14 P1: attribute this PDB to the owning process (registry-only). */
                    nvkvm_m2_proc_add_pdb(s, own14 ? own14 : s->chan_vas[k].client,
                                          s->chan_vas[k].pdb);
                }
                qemu_log("nvkvm-gpu[%s] M5: VAS hObject=0x%08x client=0x%08x PDB=0x%llx\n",
                         s->chip->name, s->chan_vas[k].hvas, s->chan_vas[k].client,
                         (unsigned long long)s->chan_vas[k].pdb);
            }
        }
        /* M5.30 PRODUCTION UVM CAPTURE: snoop NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY
         * (0x801813).  This is how the guest's UVM driver (nvUvmInterfaceSetPage
         * Directory) hands RM/GSP the page-directory ROOT of a UVM-managed VAS — the
         * VAS that backs cuMemAlloc device pointers.  Unlike the GSP-client GR VASes
         * (FB-rooted, snooped above from 0x90f10106), the UVM root is typically in
         * SYSMEM (guest RAM).  Capturing {physAddress, aperture, hVASpace} lets the
         * doorbell/CE resolver WALK the guest's own UVM page tables to resolve a UVM
         * device VA -> guest GPA, with NO uprobe bridge and NO guest-userspace read
         * (the tables live in guest RAM, reachable via the GPA window).  This is the
         * production replacement for the debug uprobe-bridge UVM-shadow path.
         * NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS @ params(cmd+120):
         *   physAddress u64 @+0 (cmd+120); numEntries @+8; flags @+12 (cmd+132);
         *   hVASpace @+16 (cmd+136).  flags[1:0] aperture: 0=VIDMEM,1=SYS_COH,2=SYS_NONCOH. */
        if (fn == 76 && ldl_le_p(cmd + 88) == 0x00801813u) {
            uint64_t phys   = ldq_le_p(cmd + 120);
            uint32_t flags  = ldl_le_p(cmd + 132);
            uint32_t hvas   = ldl_le_p(cmd + 136);
            bool     rsys   = (flags & 0x3u) != 0u;   /* non-VIDMEM aperture => sysmem root */
            /* APPEND a candidate root (do NOT overwrite the RESERVED_PDES root for
             * the same hVASpace — they can differ: e.g. hVASpace 0xcaf00005 has
             * 0x3114000 from RESERVED_PDES vs 0x3400000 from SET_PAGE_DIRECTORY.
             * The chan_translate resolver tries every VAS candidate and uses the
             * first that yields a valid leaf, so adding the SET_PAGE_DIRECTORY root
             * as an extra candidate is non-destructive.  Dedup exact (hvas,pdb)
             * repeats so the 16-slot table doesn't fill on re-sets. */
            bool dup = false;
            for (int i = 0; i < s->chan_vas_n; i++) {
                if (s->chan_vas[i].hvas == hvas && s->chan_vas[i].pdb == phys) {
                    dup = true; break;
                }
            }
            if (!dup && phys && s->chan_vas_n < (int)ARRAY_SIZE(s->chan_vas)) {
                int k = s->chan_vas_n++;
                s->chan_vas[k].hvas = hvas;
                s->chan_vas[k].client = ldl_le_p(cmd + 80); /* GSP_RM_CONTROL hClient (#12-L3) */
                s->chan_vas[k].pdb = phys;
                s->chan_vas[k].root_sys = rsys;
                s->chan_vas[k].uvm = true;                  /* #13: UVM-managed user VAS */
                nvkvm_m2_cli_vas_add(s, s->chan_vas[k].client, phys, rsys);
                /* #14: also key under the dup-source (owning compute) client. */
                {
                    uint32_t own14 = nvkvm_m2_vas_dup_owner(s, s->chan_vas[k].client, hvas);
                    if (own14) { nvkvm_m2_cli_vas_add(s, own14, phys, rsys); }
                    /* #14 P1: attribute this UVM PDB to the owning process (registry-only). */
                    nvkvm_m2_proc_add_pdb(s, own14 ? own14 : s->chan_vas[k].client, phys);
                }
                qemu_log("nvkvm-gpu[%s] M5.30 SET_PAGE_DIR UVM-VAS hVASpace=0x%08x "
                         "PDB=0x%llx aperture=%u root=%s (candidate %d)\n",
                         s->chip->name, hvas, (unsigned long long)phys,
                         flags & 0x3u, rsys ? "SYS" : "FB", k);
            }
        }
        /* #14: snoop DUP_OBJECT (fn=21) — the handle-graph edge that attributes a
         * VAS captured under one RM client (typically UVM's per-process gpu-ops
         * client) to the compute client that owns it.  rpc_dup_object body =
         * NVOS55_PARAMETERS @cmd+80: hClient(dst)@+0, hParent@+4, hObject(new dup
         * handle)@+8, hClientSrc@+12, hObjectSrc@+16 (g_sdk-structures.h v03_00).
         * Pure bookkeeping: the RPC itself is echoed/forwarded exactly as before. */
        if (fn == 21) {
            uint32_t d_cli = ldl_le_p(cmd + 80);
            uint32_t d_obj = ldl_le_p(cmd + 88);
            uint32_t s_cli = ldl_le_p(cmd + 92);
            uint32_t s_obj = ldl_le_p(cmd + 96);
            if (d_cli && d_obj && s_cli && s_obj) {
                bool seen = false;
                for (int i = 0; i < s->m2_dup_n; i++) {
                    if (s->m2_dup[i].dst_client == d_cli && s->m2_dup[i].dst_obj == d_obj &&
                        s->m2_dup[i].src_client == s_cli && s->m2_dup[i].src_obj == s_obj) {
                        seen = true; break;
                    }
                }
                if (!seen && s->m2_dup_n < (int)ARRAY_SIZE(s->m2_dup)) {
                    int k = s->m2_dup_n++;
                    s->m2_dup[k].dst_client = d_cli;
                    s->m2_dup[k].dst_obj   = d_obj;
                    s->m2_dup[k].src_client = s_cli;
                    s->m2_dup[k].src_obj   = s_obj;
                    qemu_log("nvkvm-gpu[%s] #14 DUP_OBJECT edge[%d]: dst=0x%08x/0x%08x "
                             "<- src=0x%08x/0x%08x\n", s->chip->name, k,
                             d_cli, d_obj, s_cli, s_obj);
                }
                /* #14 EARLY-ARM: the dup SRC is by construction a user compute
                 * client (kernel-internal clients — UVM gpu-ops, CeUtils — only
                 * ever appear on the DST side; bench-verified).  Register it NOW,
                 * at cuCtxCreate's UVM-registration step, so a 2nd process arms
                 * nvkvm_m2_multiproc() BEFORE its channels/working set exist —
                 * not after its 0xc7c0 GR alloc, by which time the shared state
                 * has already aliased the two processes (the round-3 wall).
                 * Single process: exactly one src client ever appears (#12's two
                 * contexts REUSE one client), so the gate stays off. */
                if (!nvkvm_m2_is_user_client(s, s_cli) &&
                    s->m2_user_clients_n < (int)ARRAY_SIZE(s->m2_user_clients)) {
                    s->m2_user_clients[s->m2_user_clients_n++] = s_cli;
                    qemu_log("nvkvm-gpu[%s] #14 user compute client[%d] = 0x%08x "
                             "(dup-src early-arm)%s\n", s->chip->name,
                             s->m2_user_clients_n - 1, s_cli,
                             s->m2_user_clients_n > 1 ? "  -> MULTIPROC ARMED" : "");
                }
                /* #14 P1: the dup SRC anchors a process; the dup DST (its UVM gpu-ops
                 * client) joins that process.  Registry-only (nothing keys on it yet). */
                {
                    int pi = nvkvm_m2_proc_get(s, s_cli);
                    nvkvm_m2_proc_add_client(s, pi, d_cli);
                }
            }
        }
        if (fn == 103) {
            uint32_t hclass = ldl_le_p(cmd + 92);
            /* M5/M7 — record NV01_EVENT_OS_EVENT (0x0079) allocations so we can
             * post a GSP POST_EVENT on channel completion (the blocking-sync
             * wakeup).  GSP_RM_ALLOC body: hClient@80, hObject@88(=hEvent),
             * hClass@92, params@112 = NV0005_ALLOC_PARAMETERS {hParentClient@0,
             * hSrcResource@4, hClass@8, notifyIndex@12, data@16}.  _kgspRpcPostEvent
             * matches by (hClient,hEvent), so those are the load-bearing fields. */
            if (hclass == 0x0079u &&
                s->osevent_n < (int)ARRAY_SIZE(s->osevents)) {
                uint32_t hcli = ldl_le_p(cmd + 80);
                uint32_t hev  = ldl_le_p(cmd + 88);
                /* de-dup (the same event may be re-seen on replay) */
                bool seen = false;
                for (int i = 0; i < s->osevent_n; i++) {
                    if (s->osevents[i].hclient == hcli &&
                        s->osevents[i].hevent  == hev) { seen = true; break; }
                }
                if (!seen) {
                    s->osevents[s->osevent_n].hclient      = hcli;
                    s->osevents[s->osevent_n].hevent       = hev;
                    s->osevents[s->osevent_n].notify_index = ldl_le_p(cmd + 124);
                    s->osevent_n++;
                    if (s->trace) {
                        qemu_log("nvkvm-gpu[%s] M7: recorded os-event hClient=0x%08x "
                                 "hEvent=0x%08x notifyIndex=%u (#%d)\n", s->chip->name,
                                 hcli, hev, ldl_le_p(cmd + 124), s->osevent_n);
                    }
                }
            }
            if ((hclass & 0xFFFFu) >= 0xC06Fu && (hclass & 0xFFu) == 0x6Fu &&
                (hclass & 0xF000u) == 0xC000u) {
                s->chan_class      = hclass;
                s->chan_gpfifo_va  = ldq_le_p(cmd + 120);
                s->chan_gpfifo_ent = ldl_le_p(cmd + 128);
                s->chan_inst_block = ldq_le_p(cmd + 256);   /* instanceMem.base */
                s->chan_inst_sys   = (ldl_le_p(cmd + 272) == 1u); /* ADDR_SYSMEM */
                s->chan_hvaspace   = ldl_le_p(cmd + 140);   /* hVASpace handle */
                s->chan_userd      = ldq_le_p(cmd + 280);   /* userdMem.base */
                s->chan_userd_sys  = (ldl_le_p(cmd + 296) == 1u);
                s->chan_gp_get     = 0;
                /* Register in the multi-channel table (dedup by (hClient, gpFifoVA)).
                 * #14: the key MUST include the client — two concurrent processes get
                 * IDENTICAL GPFIFO VAs (both pools at 0x2002xxxxx), so a VA-only dedup
                 * made process B's registrations OVERWRITE process A's entries in
                 * place (client/hobject/tsg swapped, cursors reset): half the live
                 * channels simply vanished from the table and were never scheduled,
                 * rung, or completion-forged.  Same-process replays still dedup. */
                if (s->chan_gpfifo_va && s->chan_userd) {
                    uint32_t ccli = ldl_le_p(cmd + 80);            /* hClient */
                    int cslot = -1;
                    for (int i = 0; i < s->chan_n; i++) {
                        if (s->chans[i].gpfifo_va == s->chan_gpfifo_va &&
                            s->chans[i].client == ccli) { cslot = i; break; }
                    }
                    if (cslot < 0 && s->chan_n < NVKVM_MAX_CHANS) { cslot = s->chan_n++; }
                    if (cslot >= 0) {
                        s->chans[cslot].gpfifo_va  = s->chan_gpfifo_va;
                        s->chans[cslot].userd      = s->chan_userd;
                        s->chans[cslot].gpfifo_ent = s->chan_gpfifo_ent;
                        s->chans[cslot].userd_sys  = s->chan_userd_sys;
                        s->chans[cslot].hvaspace   = s->chan_hvaspace;
                        s->chans[cslot].client     = ldl_le_p(cmd + 80); /* hClient */
                        s->chans[cslot].hobject    = ldl_le_p(cmd + 88); /* channel handle */
                        s->chans[cslot].tsg        = ldl_le_p(cmd + 84); /* M5.25: parent TSG */
                        s->chans[cslot].scheduled  = false;
                        s->chans[cslot].gp_get     = 0;
                        s->chans[cslot].payload    = 0;
                        s->chans[cslot].fin_payload = 0;   /* #12 */
                        s->chans[cslot].fin_fb      = 0;   /* #12: re-resolve on (re)alloc */
                        s->chans[cslot].fin_sys     = false; /* #12 L3b: re-resolve aperture */
                        s->chans[cslot].token_valid = false;
                        /* #14 P1: recover the guest vChid from the alloc flags.  A GSP-client
                         * CPU-RM encodes its already-decided ChID into USERD_INDEX so the
                         * physical RMAPI reuses it (kernel_channel.c:2688):
                         *   chid = flags[20:12]*8 + flags[10:8]
                         * (numChannelsPerUserd = 1<<DRF_SIZE(USERD_INDEX_VALUE=10:8) = 8).
                         * E0: doorbell token[11:0] == this vChid; it's the demux key (P1: log). */
                        uint32_t cflags = ldl_le_p(cmd + 132);   /* NV_CHANNEL_ALLOC_PARAMS.flags */
                        s->chans[cslot].vchid = ((cflags >> 12) & 0x1ffu) * 8u +
                                                ((cflags >> 8) & 0x7u);
                        s->chans[cslot].vchid_valid = true;
                    }
                }
                qemu_log("nvkvm-gpu[%s] M5: channel alloc class=0x%04x gpFifoVA="
                         "0x%llx ent=%u instblk=0x%llx(%s)\n",
                         s->chip->name, hclass,
                         (unsigned long long)s->chan_gpfifo_va, s->chan_gpfifo_ent,
                         (unsigned long long)s->chan_inst_block,
                         s->chan_inst_sys ? "sys" : "fb");
            }
        }
        /* M7 fix: on GSP_RM_FREE (fn==10) drop os-events for the freed event/client
         * so we never POST_EVENT to a dead (hClient,hEvent) — see nvkvm_m2_osevent_drop.
         * Free body == alloc body: hClient@80, hObject(freed)@88. */
        if (fn == 10 && s->osevent_n > 0) {
            nvkvm_m2_osevent_drop(s, ldl_le_p(cmd + 80), ldl_le_p(cmd + 88));
        }
        /* M5.49: also drop freed channel/VASpace bookkeeping so a 2nd CUDA context
         * (next process) doesn't inherit ctx1's stale VAS routing (task #12 seq case). */
        if (fn == 10) {
            nvkvm_m2_ctx_free_drop(s, ldl_le_p(cmd + 80), ldl_le_p(cmd + 88));
        }
        if (!async) {
            /* Build the response in a large buffer — GSP_RM_CONTROL responses can
             * span multiple queue elements (paramsSize up to ~34 KB).  Seed it
             * from the command element (element header + 32B rpc header + 40B
             * rpc_gsp_rm_control body), then fill the control response.
             * Body@80: hClient@80, hObject@84, cmd@88, status@92, paramsSize@96,
             * ..., params@120.  RmRpc control reads body.status (@92). */
            static uint8_t resp[NVKVM_RESP_MAX];
            memset(resp, 0, sizeof(resp));
            memcpy(resp, cmd, 4096);
            /* M5.3 FIX (cuCtxCreate SIGSEGV): GR-object allocs (compute/3D, e.g.
             * AMPERE_COMPUTE_B 0xc7c0) register NV_GR_ALLOCATION_PARAMETERS (16B)
             * as RS_OPTIONAL.  libcuda passes a non-NULL pAllocParms backed by only
             * an 8-byte stack slot with paramsSize=0; a real GSP returns paramsSize=0
             * (no params writeback) so the host driver copies 0 bytes back.  Our echo
             * reply returned the full 16B params, and the guest's GSP-client deserialize
             * copied 16B into libcuda's 8-byte buffer — clobbering a saved rbp on the
             * stack -> NULL-rbp deref -> SIGSEGV in cuCtxCreate (proven byte-exact,
             * host-vs-guest, 2026-06-04).  Match the real GSP: drop the params from the
             * reply for GR objects so the guest copies nothing back.  GR object classes:
             * low byte 0xC0 (compute) / 0x97 (3D), family >= 0xB0; excludes DMA-copy
             * (0xB5), subdevice (0x2080), channel (0x..6F). */
            if (fn == 103) {
                uint32_t hc = ldl_le_p(resp + 92);
                uint32_t lb = hc & 0xffu, fam = (hc >> 8) & 0xffu;
                uint32_t opsize = ldl_le_p(resp + 100);
                uint32_t robj = ldl_le_p(resp + 88);
                /* M8.2 DIAG (rbp-clobber hunt): the cuCtxCreate SIGSEGV is a ~368B zeros
                 * writeback overflowing libcuda's stack params buffer (368 == c56f channel
                 * alloc psize). Log every alloc's reply paramsSize so we can see if our reply
                 * exceeds libcuda's request (the overflow) — esp. the c56f channel alloc. */
                qemu_log("nvkvm-gpu[%s] M8.2 alloc-reply class=0x%04x obj=0x%08x "
                         "reply_paramsSize=%u\n", s->chip->name, hc, robj, opsize);
                /* M8 (cuCtxCreate SIGSEGV — root cause refined 2026-06-05): host-vs-guest gdb
                 * proved the crash function (libcuda+0x466560) is reached IDENTICALLY on host
                 * and guest (same control flow, vtable call returns NV_OK), but rbp is a VALID
                 * frame pointer on host and 0 on guest.  rbp is callee-saved; a callee corrupted
                 * the saved-rbp slot with zeros on the guest only.  strace pins the last ioctl
                 * before SIGSEGV as this RM_ALLOC.  => the guest kernel's params copy_to_user
                 * OVERRUNS libcuda's pAllocParms by exactly the amount our reply's paramsSize
                 * exceeds the host's.  Fix: forward the host's RETURNED alloc_parms_size verbatim
                 * (resp+100) AND its params bytes (resp+112).  If the real RM returns paramsSize=0
                 * (no writeback), the guest copies 0 bytes -> no overrun -> rbp preserved.
                 * "Forward, don't emulate" — match the host reply byte-for-byte. */
                if (fam >= 0xb0u && (lb == 0xc0u || lb == 0x97u)) {
                    /* GR compute/3D object (e.g. AMPERE_COMPUTE_B 0xc7c0): NV_GR_ALLOCATION_PARAMETERS
                     * is RS_OPTIONAL; libcuda passes pAllocParms backed by only a tiny stack slot with
                     * paramsSize=0, and the REAL GSP returns paramsSize=0 (no params writeback).
                     * HOST-PROVEN 2026-06-06: c7c0 psz=0, reply bytes are libcuda's untouched stack.
                     * gdb-PROVEN: our reply's 16B params -> guest open-driver copy_to_user overruns
                     * the slot, zeroing a saved rbp -> NULL-rbp deref -> cuCtxCreate SIGSEGV at
                     * ioctl 129. Force the reply paramsSize to the host's value (0 when no shadow
                     * capture) so the guest copies 0 bytes back. UNCONDITIONAL: the previous version
                     * gated this on a shadow-forward capture (m2_gr_reply_valid) that often didn't
                     * fire -> overrun -> crash (why "force-paramsSize-0" looked moot before). */
                    /* M8.1 (2026-06-07, rbp-clobber REGRESSION fix): FORCE reply paramsSize=0
                     * for GR objects. libcuda's RM_ALLOC for NV_GR_ALLOCATION_PARAMETERS does
                     * NOT want params written back; a correct GSP/driver copies min(req,reply)
                     * = 0 bytes. Commit 1443793 proved paramsSize=0 makes cuCtxCreate SUCCEED
                     * (2-day wall broken). The later M7 "forward host's real 16B caps" change
                     * (m2_gr_reply_valid -> psize=16) RE-OPENED the clobber: the guest copies
                     * 16B into libcuda's stack slot, zeroing a saved rbp -> SIGSEGV at libcuda
                     * +0x300560 (mov -0x38(%rbp),%rax, rbp=0), exactly after c7c0. The captured
                     * caps are NOT needed for cuCtxCreate. Keep the M7 capture only as a diag. */
                    uint32_t cap_psize = (s->m2_gr_reply_valid && s->m2_gr_reply_obj == robj)
                                             ? s->m2_gr_reply_psize : 0u;
                    /* M8.4 (ported from oracle 7fb47f1 — rbp-RESTORE, supersedes M8.1):
                     * KEEP the request params bytes in the response payload (resp+112) AND
                     * extend the rpc element length (resp+56) so the response actually
                     * transports them.  The guest's GSP-client deserialize then fills its
                     * local rpc_params buffer from the element, and the guest RM's
                     * unavoidable class-size copy_to_user (it derives the 16B
                     * NV_GR_ALLOCATION_PARAMETERS size and ignores reply paramsSize)
                     * writes libcuda's OWN bytes back — restoring the saved rbp on its
                     * stack instead of zeroing it.  M8.1 set only paramsSize=0, leaving the
                     * element short -> the deserialize zero-padded the local buffer -> the
                     * copyout cleared rbp -> cuCtxCreate SIGSEGV (libcuda+0x300560, rbp=0).
                     * Confirmed byte-exact host-vs-guest (host preserves c7c0 params bytes
                     * 8-15; guest zeroed them). Report semantic paramsSize=0 like native RM. */
                    uint32_t req_psize = opsize;
                    if (req_psize > NVKVM_RESP_MAX - 112u) {
                        req_psize = NVKVM_RESP_MAX - 112u;
                    }
                    if (req_psize) {
                        memcpy(resp + 112, cmd + 112, req_psize);
                        stl_le_p(resp + 56, 32u + 32u + req_psize);
                    }
                    stl_le_p(resp + 100, 0u);          /* semantic paramsSize=0 */
                    s->m2_gr_reply_valid = false;
                    qemu_log("nvkvm-gpu[%s] M8.4 GR-obj 0x%04x: paramsSize=0 len-preserve=%u "
                             "host_caps_psize %u dropped [rbp-restore]\n",
                             s->chip->name, hc, req_psize, cap_psize);
                }
            }
            uint32_t ctrl = (fn == 76) ? ldl_le_p(resp + 88) : 0;
            /* #14 piece-2 (per-process completion delivery): MC_SERVICE_INTERRUPTS
             * (0x20801702) is the guest RM's explicit completion POLL — at the #14
             * hang both starved processes spin exactly here.  In multiproc mode a
             * user channel's execution/sema resolution may have been DEFERRED (the
             * pass-1 refusal: its VAS capture raced the doorbell), and the retry
             * only re-ran on the NEXT doorbell — but a process busy-polling
             * cuCtxCreate submits nothing, so once the other process finishes (no
             * more doorbells from anyone) the deferred work never retried and the
             * poller starved forever.  Kick the doorbell service after this RPC
             * completes (flag consumed at the end of service_cmdq, once the
             * response is posted + acked): the service is idempotent (channels
             * with no pending work bail at GP_PUT==gp_get).  multiproc()-gated —
             * single-process behavior (incl. #12/#13 timing) is untouched. */
            if (fn == 76 && ctrl == 0x20801702u && nvkvm_m2_multiproc(s)) {
                s->m2_poll_kick = true;
            }
            if (fn == 76) {
                stl_le_p(resp + 92, 0); /* body.status = NV_OK (default) */
                /* M9: capture the caller's REQUEST paramsSize (the buffer libcuda/CPU-RM
                 * allocated) BEFORE any handler overwrites resp+96, so we can clamp the
                 * reply size and never overrun the caller's buffer (see clamp below). */
                uint32_t req_psize = ldl_le_p(resp + 96);
                const nvkvm_ctrl_resp_t *cr = NULL;
                for (uint32_t i = 0; i < NVKVM_CTRL_RESP_COUNT; i++) {
                    if (nvkvm_ctrl_resps[i].cmd == ctrl) {
                        cr = &nvkvm_ctrl_resps[i];
                        break;
                    }
                }
                if (s->m2fwd && (ctrl == 0x906f0101u || ctrl == 0x0080170du)) {
                    /* item-3 (cuCtxCreate crash hunt): FORWARD these GET controls to
                     * the real host GPU instead of replying NV_OK+zeros.  A faked
                     * count/size here can shift libcuda's stack layout so the later
                     * 0xc7c0 GR-alloc copy_to_user overruns pAllocParms (proven crash
                     * mechanism).  Candidates: 0x906f0101 GET_CLASS_ENGINEID (returns
                     * engineID — libcuda sizes per-engine arrays from it), 0x0080170d
                     * FIFO_GET_CHANNELLIST (returns a channel COUNT that may size a
                     * libcuda stack buffer).  nvkvm_m2_control1 does guest->host handle
                     * translation and writes the host reply back into the buffer. */
                    uint32_t ps = ldl_le_p(resp + 96);
                    if (ps && (120u + ps) <= NVKVM_RESP_MAX) {
                        uint8_t cbuf[4096];
                        uint32_t cn = ps < sizeof(cbuf) ? ps : (uint32_t)sizeof(cbuf);
                        memcpy(cbuf, resp + 120, cn);
                        uint32_t st = 0xffff;
                        int rc = nvkvm_m2_control1(s, ldl_le_p(resp + 80),
                                                   ldl_le_p(resp + 84), ctrl,
                                                   cbuf, cn, &st);
                        if (rc == 0) {
                            memcpy(resp + 120, cbuf, cn);
                            stl_le_p(resp + 92, st);
                        } else {
                            stl_le_p(resp + 92, 0); /* fall back to NV_OK echo */
                        }
                        stl_le_p(resp + 56, 32u + 40u + ps);
                        qemu_log("nvkvm-gpu[%s] item-3 FWD ctrl=0x%08x ps=%u rc=%d "
                                 "st=0x%x reply[0..7]=%02x%02x%02x%02x%02x%02x%02x%02x\n",
                                 s->chip->name, ctrl, ps, rc, st,
                                 resp[120], resp[121], resp[122], resp[123],
                                 resp[124], resp[125], resp[126], resp[127]);
                    }
                } else if (ctrl == 0x2080012fu) {
                    /* M10 (cuCtxCreate rbp=0 ROOT CAUSE — host-vs-guest ioctl diff, 2026-06-06):
                     * NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS (0x2080012f, params 1464B).  A GeForce
                     * GA106 has NO ECC, so the REAL driver returns NV_ERR_NOT_SUPPORTED (0x56) and
                     * the RM sets SKIP_COPYOUT -> libcuda's 1464B ECC buffer is left untouched.  Our
                     * fake NV_OK instead copied 1464B of echoed/zero garbage over that buffer, so
                     * libcuda acted on bogus ECC state and later crashed (saved-rbp clobber ->
                     * rbp=0 SIGSEGV at libcuda+0x466560).  Match the host: report NOT_SUPPORTED.
                     * (Found via the nvioctl_trace LD_PRELOAD host/guest diff: this was the ONLY
                     * control whose status diverged — host 0x56 vs guest 0x0.) */
                    stl_le_p(resp + 92, 0x56u);   /* NV_ERR_NOT_SUPPORTED */
                } else if (ctrl == 0x20803002u) {
                    /* M10: NV2080_CTRL_CMD_NVLINK_GET_NVLINK_STATUS (0x20803002).  The
                     * params struct is huge (NV2080_CTRL_NVLINK_LINK_STATUS_INFO
                     * linkInfo[MAX_ARR_SIZE], ~13 KB).  On a no-NVLink GeForce (GA106) the
                     * real driver returns NV_ERR_NOT_SUPPORTED, on which the RM sets
                     * RMAPI_PARAM_COPY_FLAGS_SKIP_COPYOUT and copies NOTHING back to libcuda.
                     * Our fake NV_OK echo instead forced a full ~13 KB param copy_to_user into
                     * libcuda's stack buffer.  Match the host: report NOT_SUPPORTED so the guest
                     * skips the copyout.  (No NVLink on this part, so this is the correct status.) */
                    stl_le_p(resp + 92, 0x56u);   /* NV_ERR_NOT_SUPPORTED */
                } else if (ctrl == 0x20800a5cu) {
                    /* INTERNAL_INTR_GET_KERNEL_TABLE: the real GSP supplies the
                     * interrupt table via boot static-info so the host CPU-RM
                     * never issues this control; the guest's fake GSP forces the
                     * fallback.  Replay the captured GA106 table (tableLen@120 +
                     * 24 entries of 16B).  Full struct = 4 + 128*16 = 2052B. */
                    memset(resp + 120, 0, INTRTABLE_GA106_PSIZE);
                    memcpy(resp + 120, intrtable_ga106, sizeof(intrtable_ga106));
                    /* subtreeMap[7] of NvU64 (per intrInitSubtreeMap_TU102, which
                     * the GSP mirrors into this control's reply): UVM_OWNED must
                     * map to subtree 1 (mask 0x2) so it equals the access-counter
                     * vector's subtree, else intrCacheIntrFields_TU102 asserts.
                     * idx: 0 DEFAULT, 1 ESCHED(stall subtree3=0x8), 2 ESCHED_NOTIF
                     * (subtree0=0x1), 3 RUNLIST, 4 RUNLIST_NOTIF, 5 UVM_OWNED
                     * (subtree1=0x2), 6 UVM_SHARED (subtree2=0x4). */
                    {
                        static const uint64_t subtree_map[7] = {
                            0x0ull, 0x8ull, 0x1ull, 0x0ull, 0x0ull, 0x2ull, 0x4ull
                        };
                        for (int k = 0; k < 7; k++)
                            stq_le_p(resp + 120 + INTRTABLE_GA106_SUBTREEMAP_OFF + k * 8,
                                     subtree_map[k]);
                    }
                    stl_le_p(resp + 96, INTRTABLE_GA106_PSIZE);
                    stl_le_p(resp + 56, 32u + 40u + INTRTABLE_GA106_PSIZE);
                } else if (ctrl == 0x20801112u) {
                    /* FIFO_GET_DEVICE_INFO_TABLE: the engine enumeration that drives the guest's
                     * classDB + KernelCE objects. M14 (user direction — forward, don't hardcode):
                     * this control is ROUTE_TO_PHYSICAL but NOT privileged/internal (flags 0x5c040),
                     * so FORWARD it to the host's real GSP and return THIS GPU's actual engine table
                     * dynamically — no per-GPU baked blob (which was captured truncated: 10 entries,
                     * no video engines -> guest GET_CLASSLIST_V2 numClasses 97 vs host 107). Fall back
                     * to the captured GA106 blob only if the forward isn't serviceable yet. */
                    uint32_t base = ldl_le_p(resp + 120);
                    if (s->m2fwd) {
                        nvkvm_m2_capture_devinfo(s);   /* one-shot live host capture */
                    }
                    if (s->m2_devinfo_n > 0) {
                        /* serve from the live host capture, paginated (32/page) */
                        uint32_t psize = 12u + 32u * 100u; /* 3212 */
                        memset(resp + 120, 0, psize);
                        stl_le_p(resp + 120, base);
                        uint32_t n_this = 0;
                        if (base < s->m2_devinfo_n) {
                            n_this = s->m2_devinfo_n - base;
                            if (n_this > 32u) n_this = 32u;
                            memcpy(resp + 132, s->m2_devinfo + (uint64_t)base * 100,
                                   (uint64_t)n_this * 100);
                        }
                        stl_le_p(resp + 124, n_this);
                        stl_le_p(resp + 128, (base + n_this < s->m2_devinfo_n) ? 1u : 0u);
                        stl_le_p(resp + 96, psize);
                        stl_le_p(resp + 92, 0);
                        stl_le_p(resp + 56, 32u + 40u + psize);
                    } else {
                        /* fallback: captured GA106 blob (only if live capture unavailable) */
                        uint32_t psize = 12u + 32u * DEVINFO_GA106_ENTRY_SIZE; /* 3212 */
                        memset(resp + 120, 0, psize);
                        stl_le_p(resp + 120, base);
                        if (base == 0) {
                            stl_le_p(resp + 124, DEVINFO_GA106_NUM_ENTRIES);
                            memcpy(resp + 132, devinfo_ga106_entries,
                                   sizeof(devinfo_ga106_entries));
                        }
                        stl_le_p(resp + 96, psize);
                        stl_le_p(resp + 56, 32u + 40u + psize);
                    }
                } else if (ctrl == 0x20802a08u) {
                    /* CE_GET_FAULT_METHOD_BUFFER_SIZE: { NvU32 size }.  Our
                     * capture truncated the 4B payload (size replayed as 0) ->
                     * kchangrpInit_gv100 asserts bufSizeInBytes>0 when CPU-RM
                     * allocates the CE fault method buffer.  Synthesize one page
                     * (the buffer lives in sysmem and only our emulated CE uses
                     * it, so any non-zero page-aligned size satisfies it). */
                    stl_le_p(resp + 120, 0x1000u); /* size = 4 KiB */
                    stl_le_p(resp + 92, 0);
                    stl_le_p(resp + 96, 4u);
                    stl_le_p(resp + 56, 32u + 40u + 4u);
                } else if (ctrl == 0x20800102u || ctrl == 0x20801303u) {
                    /* GPU_GET_INFO_V2 / FB_GET_INFO_V2 (Phase-B compute caps):
                     * inline list {NvU32 count; {NvU32 index, NvU32 value}[]}.
                     * The guest requests a set of indices; fill each value from
                     * the captured GA106 map (real RTX 3060 ground truth), default
                     * 0 for any index we didn't capture.  This is a cuInit=100 fix
                     * (libcuda reads these for compute-cap/device validation). */
                    const nvkvm_idxval_t *map = (ctrl == 0x20800102u)
                        ? gpu_get_info_v2_map : fb_get_info_v2_map;
                    uint32_t mapn = (ctrl == 0x20800102u)
                        ? GPU_GET_INFO_V2_MAP_N : FB_GET_INFO_V2_MAP_N;
                    uint32_t ps = ldl_le_p(resp + 96);
                    uint32_t cnt = ldl_le_p(resp + 120);
                    if (cnt > 128) cnt = 128;
                    for (uint32_t e = 0; e < cnt; e++) {
                        uint32_t eoff = 124u + e * 8u;
                        if (eoff + 8u > 120u + ps) break;
                        uint32_t idx = ldl_le_p(resp + eoff);
                        if (ctrl == 0x20800102u) {
                            /* Match host RM: bit 31 is reserved in NV2080_CTRL_GPU_INFO_INDEX
                             * and is stripped from the returned list entry.  The guest request
                             * currently arrives as 0x80000011; leaving that bit set is the
                             * remaining non-gpuId control divergence in the cuCtxCreate trace. */
                            idx &= 0x7fffffffu;
                            stl_le_p(resp + eoff, idx);
                        }
                        uint32_t val = 0;
                        for (uint32_t k = 0; k < mapn; k++) {
                            if (map[k].index == idx) { val = map[k].value; break; }
                        }
                        stl_le_p(resp + eoff + 4, val);
                    }
                    stl_le_p(resp + 92, 0);              /* NV_OK */
                    stl_le_p(resp + 56, 32u + 40u + ps); /* psize unchanged */
                } else if (ctrl == 0x20802a07u) {
                    /* CE_GET_PHYSICAL_CAPS V2 {u32 ceEngineType; u8 capsTbl[2]}.
                     * UVM channel-manager ces_validate requires each usable CE
                     * to advertise SYSMEM + P2P; a zero capsTbl -> "Failed to
                     * initialize the channel manager: NV_ERR_NOT_SUPPORTED"
                     * (uvm_gpu.c init_gpu) -> UVM_REGISTER_GPU fails -> cuInit
                     * bails.  Set SYSMEM_READ(0x04)|SYSMEM_WRITE(0x08)|
                     * SYSMEM(0x20)|P2P(0x40) in capsTbl[0]. */
                    uint32_t ps = ldl_le_p(resp + 96);
                    if (ps >= 6) {
                        resp[124] = 0x6Cu;
                        resp[125] = 0x00u;
                    }
                    stl_le_p(resp + 92, 0);
                    stl_le_p(resp + 56, 32u + 40u + ps);
                } else if (ctrl == 0x20801823u) {
                    /* BUS_GET_INFO_V2: inline {count; {index,data}[]}.  Fill the
                     * PCIe link entries so the driver's getPCIELinkRateMBps()
                     * succeeds; otherwise it returns NV_ERR_INVALID_STATE
                     * ("Unknown PCIe speed"), which propagates out of
                     * UVM_REGISTER_GPU (rmStatus=0x40) and makes cuInit bail.
                     * idx 0x2D PCIE_GEN_INFO: LINK_CAP_GEN[15:12]=3 (GEN4),
                     * CURR_LEVEL[19:16]=3; idx 0x07 LINK_CTRL_STATUS:
                     * LINK_SPEED[19:16]=4 (16GT/s), LINK_WIDTH[25:20]=16 (x16). */
                    uint32_t ps = ldl_le_p(resp + 96);
                    uint32_t cnt = ldl_le_p(resp + 120);
                    if (cnt > 256) cnt = 256;
                    for (uint32_t e = 0; e < cnt; e++) {
                        uint32_t eoff = 124u + e * 8u;
                        if (eoff + 8u > 120u + ps) break;
                        uint32_t idx = ldl_le_p(resp + eoff);
                        if (idx == 0x03u) {
                            /* PCIE_GPU_LINK_CAPS: MAX_SPEED[3:0]=4 (16000MBPS),
                             * MAX_WIDTH[9:4]=16.  UVM's getPCIELinkRateMBps reads
                             * exactly this; 0 -> "Unknown PCIe speed" INVALID_STATE. */
                            stl_le_p(resp + eoff + 4, 4u | (16u << 4));
                        } else if (idx == 0x2Du) {
                            stl_le_p(resp + eoff + 4, (3u << 12) | (3u << 16));
                        } else if (idx == 0x07u) {
                            stl_le_p(resp + eoff + 4, (4u << 16) | (16u << 20));
                        }
                    }
                    stl_le_p(resp + 92, 0);
                    stl_le_p(resp + 56, 32u + 40u + ps);
                } else if (ctrl == 0x20803801u) {
                    /* GRMGR_GET_GR_FS_INFO: replay captured GA106 floorsweep blob
                     * (GPC/TPC/PES enable masks).  Capture is a 256B prefix; the
                     * leading query results are what cuInit reads. */
                    uint32_t ps = ldl_le_p(resp + 96);
                    uint32_t n = GRMGR_GR_FS_INFO_BLOB_N;
                    if (n > ps) n = ps;
                    memcpy(resp + 120, grmgr_gr_fs_info_blob, n);
                    stl_le_p(resp + 92, 0);
                    stl_le_p(resp + 56, 32u + 40u + ps);
                } else if (ctrl == 0x20803601u) {
                    /* M5.3 forge gap (host-vs-guest control diff): version-ish struct
                     * {u32@0=1; u8@4=1; u8@5=1; char ver[]@6}. libcuda reads the
                     * driver/GSP version; an all-zero reply (CTRL-UNFILLED) may steer it
                     * into a bad path during cuCtxCreate. Replay the host 580.159.04. */
                    uint32_t ps = ldl_le_p(resp + 96);
                    memset(resp + 120, 0, ps);
                    if (ps >= 6) { stl_le_p(resp + 120, 1u); resp[124] = 1u; resp[125] = 1u; }
                    if (ps >= 16) { memcpy(resp + 126, "580.159.04", 10); }
                    stl_le_p(resp + 92, 0);
                    stl_le_p(resp + 56, 32u + 40u + ps);
                } else if (ctrl == 0x20802a0au) {
                    /* M5.3 forge gap: host returns four u16 {0x3e3,0x3e3,0x3e2,0x3e2}@0
                     * and 0x0f@128 (looks like per-unit caps/clocks). Replay. */
                    uint32_t ps = ldl_le_p(resp + 96);
                    memset(resp + 120, 0, ps);
                    if (ps >= 8) {
                        stw_le_p(resp + 120, 0x03e3); stw_le_p(resp + 122, 0x03e3);
                        stw_le_p(resp + 124, 0x03e2); stw_le_p(resp + 126, 0x03e2);
                    }
                    if (ps >= 129) { resp[120 + 128] = 0x0f; }
                    stl_le_p(resp + 92, 0);
                    stl_le_p(resp + 56, 32u + 40u + ps);
                } else if (ctrl == 0x20802a0bu) {
                    /* M13 (host-vs-guest ptrace diff, 2026-06-06): CE_GET_ALL_PHYSICAL_CAPS.
                     * The guest kernel's subdeviceCtrlCmdCeGetAllCaps builds the user-visible
                     * CE_GET_ALL_CAPS reply from THIS GSP physical-caps RPC (kernel_ce_shared.c).
                     * We weren't answering it -> base caps stayed 0 -> guest CE_GET_ALL_CAPS
                     * capsTbl=0 vs host 0x03e3 (the nvtrace/nvdecode diff caught this). Struct =
                     * { NvU8 capsTbl[64][2]; NvU64 present@128 }. Replay the host GA106 values:
                     * CE0/CE1=0x03e3 (GRCE|SHARED|SYSMEM|P2P|BL>64K), CE2/CE3=0x03e2, present=0xf. */
                    uint32_t ps = ldl_le_p(resp + 96);
                    memset(resp + 120, 0, ps);
                    if (ps >= 8) {
                        stw_le_p(resp + 120, 0x03e3); stw_le_p(resp + 122, 0x03e3);
                        stw_le_p(resp + 124, 0x03e2); stw_le_p(resp + 126, 0x03e2);
                    }
                    if (ps >= 136) { stq_le_p(resp + 120 + 128, 0x0full); }
                    stl_le_p(resp + 92, 0);
                    stl_le_p(resp + 56, 32u + 40u + ps);
                } else if (ctrl == 0x20808162u) {
                    /* M5.3 forge gap: host returns bool=1. */
                    uint32_t ps = ldl_le_p(resp + 96);
                    if (ps >= 1) { resp[120] = 1u; }
                    stl_le_p(resp + 92, 0);
                    stl_le_p(resp + 56, 32u + 40u + ps);
                } else if (ctrl == 0x20809009u || ctrl == 0x20809001u ||
                           ctrl == 0x20809064u) {
                    /* cudart (CUDA-runtime) init gate cluster — THE LLM/cudart blocker
                     * (memory mode2_execfwd_layer2 CORRECTION 3).  libcudart issues
                     * 0x20809009/0x20809001/0x20809064 (NV2080 subdevice controls) near
                     * the end of its lazy device-enumeration init; the driver API never
                     * issues them (so cup4/dvp pass without them).  They are serviced
                     * ENTIRELY by GSP firmware: no kernel #define in the 580/610 open
                     * source AND absent from gVisor nvproxy's allowlist even on the 580
                     * branch — i.e. closed libcuda<->GSP controls, no documented struct.
                     * Our default CTRL-UNFILLED echo returned all-zeros, so cudart read 0
                     * where it expects real data and aborted with
                     * cudaErrorInitializationError(3) — silently (the reject is in the
                     * reply PAYLOAD, not an errno/dmesg line).
                     *   PRIMARY: FORWARD to the host GPU, which authoritatively services
                     *   these (proven: native host cudart succeeds).  They are flat
                     *   params (no embedded pointers) so a raw pass-through is safe —
                     *   the same shape gVisor nvproxy uses for rmControlSimple controls;
                     *   nvkvm_m2_control1 only adds guest->host hClient/hObject xlation.
                     *   Forwarding is driver-version-robust (no hard-coded GPU values)
                     *   and de-risks sibling cudart controls.
                     *   FALLBACK (if the forward fails — e.g. the subdevice handle has no
                     *   host twin yet): replay the NATIVE host capture for GA106/580.159.04
                     *   (oracle: ioctl_trace.so on native cudart via
                     *   scripts/mode2_diag/m555_cudart_payload_host.sh + tests/mode2/rtp.c):
                     *     0x20809009 -> {0x00000000, 0x0000000d}  (0x0d=13 = CUDA major)
                     *     0x20809001 -> {0x03fc007f, 0x00000000}  (capability mask)
                     *     0x20809064 -> 520B, leading 10 u32s {0,2,1,1,1,0x64,4,0x10,1,0x64} */
                    uint32_t ps = ldl_le_p(resp + 96);
                    bool ok = false;
                    if (s->m2fwd && ps && (120u + ps) <= NVKVM_RESP_MAX) {
                        uint8_t cbuf[1024];
                        if (ps <= sizeof(cbuf)) {
                            uint32_t st = 0xffff;
                            memcpy(cbuf, resp + 120, ps);
                            int rc = nvkvm_m2_control1(s, ldl_le_p(resp + 80),
                                                       ldl_le_p(resp + 84), ctrl,
                                                       cbuf, ps, &st);
                            if (rc == 0 && st == 0) {
                                memcpy(resp + 120, cbuf, ps);
                                ok = true;
                            }
                            qemu_log("nvkvm-gpu[%s] cudart-ctrl FWD 0x%08x ps=%u rc=%d "
                                     "st=0x%x ok=%d\n", s->chip->name, ctrl, ps, rc,
                                     st, ok);
                        }
                    }
                    if (!ok) {
                        if (ctrl == 0x20809009u && ps >= 8) {
                            stl_le_p(resp + 120, 0u);
                            stl_le_p(resp + 124, 0x0du);
                        } else if (ctrl == 0x20809001u && ps >= 8) {
                            stl_le_p(resp + 120, 0x03fc007fu);
                            stl_le_p(resp + 124, 0u);
                        } else if (ctrl == 0x20809064u && ps >= 40 &&
                                   (120u + ps) <= NVKVM_RESP_MAX) {
                            static const uint32_t v064[10] = {
                                0u, 2u, 1u, 1u, 1u, 0x64u, 4u, 0x10u, 1u, 0x64u };
                            memset(resp + 120, 0, ps);
                            for (int i = 0; i < 10; i++) {
                                stl_le_p(resp + 120 + 4 * i, v064[i]);
                            }
                        }
                    }
                    stl_le_p(resp + 92, 0);
                    stl_le_p(resp + 56, 32u + 40u + ps);
                } else if (ctrl == 0x20800a01u && cr) {
                    /* INTERNAL_DISPLAY_GET_STATIC_INFO: replay captured 32B but
                     * SYNTHESIZE numDispChannels (struct off 32, params+120 =>
                     * resp+152).  Our capture truncated the 36B struct's tail, so
                     * the field replayed as 0 -> kdispStateInitLocked's
                     * portMemAllocNonPaged(sizeof*0)=NULL -> "Could not allocate
                     * clientChannelTable".  Compute-only never allocates a display
                     * channel, so any value that bounds dispChannelNum works; 128
                     * comfortably covers GA10x's core/window/cursor channel space
                     * with a negligible (~3 KiB) table.  (generate-to-satisfy) */
                    memset(resp + 120, 0, cr->psize);
                    memcpy(resp + 120, cr->data, cr->dlen);
                    stl_le_p(resp + 152, 128u); /* numDispChannels */
                    stl_le_p(resp + 92, cr->status);
                    stl_le_p(resp + 96, cr->psize);
                    stl_le_p(resp + 56, 32u + 40u + cr->psize);
                } else if (cr && (120u + cr->psize) <= NVKVM_RESP_MAX) {
                    /* general replay: captured GA106 init-control response
                     * (ROUTE_TO_PHYSICAL GET controls the echo can't fabricate).
                     * Multi-element capable now — nvkvm_m3_post_status splits the
                     * message across queue elements, so large controls (e.g.
                     * GET_DEVICE_INFO_TABLE 0x20800a40 psize=24580) replay too.
                     * Any captured tail beyond cr->dlen is zero (fine where the
                     * meaningful prefix is numEntries + entries). */
                    memset(resp + 120, 0, cr->psize);
                    memcpy(resp + 120, cr->data, cr->dlen);
                    stl_le_p(resp + 92, cr->status);
                    stl_le_p(resp + 96, cr->psize);
                    stl_le_p(resp + 56, 32u + 40u + cr->psize);
                } else if (ctrl == 0x208001b0u) {
                    /* GET_CONSTRUCTED_FALCON_INFO: empty list is valid */
                    stl_le_p(resp + 96, 1284u);
                    memset(resp + 120, 0, 1284);
                    stl_le_p(resp + 56, 32u + 40u + 1284u);
                }
                /* else: void/SET control — echo with status=NV_OK.
                 * NOTE (M11, 2026-06-06): a blanket "default-forward instead of fake NV_OK" was
                 * tried and REVERTED — it regresses cuInit.  GSP-INTERNAL controls (e.g. the
                 * NV2080 interface-0x0a / 0x2a "_INTERNAL_*" series like 0x20802a0a/0x20802a12)
                 * are serviced by the real host's GSP and return NV_OK there, but when re-issued
                 * by our unprivileged forwarded client via the userspace RM_CONTROL ioctl they
                 * return 0x1b (INSUFFICIENT_PERMISSIONS) or 0x56 (NOT_SUPPORTED) — so status alone
                 * cannot classify them (0x56 is legit for ECC/NVLINK but wrong here).  Correct fix
                 * = a CURATED allowlist of forwardable controls (host-native trace as oracle:
                 * scripts/mode2_diag), forwarding those and REPLAYING captured data for the
                 * GSP-internal rest.  See [[mode2-control-forward-vs-replay]]. */

                /* M9 (cuCtxCreate SIGSEGV root cause — proven 2026-06-05): a control reply
                 * must NEVER write more bytes than the caller's params buffer (= the REQUEST
                 * paramsSize the caller allocated).  Several handlers above force a fixed,
                 * larger reply paramsSize than was requested; the guest kernel's params
                 * copyout then overruns the caller's (often stack) buffer.  Watchpoint +
                 * host-vs-guest gdb pinned the crash: a kernel copy_to_user during a control
                 * inside cuCtxCreate's GR-object setup zeroes libcuda's saved-rbp stack slot
                 * -> rbp=0 -> SIGSEGV at libcuda+0x466560.  Clamp reply paramsSize to the
                 * request.  (The caller always allocates its full struct, so legitimate large
                 * controls — INTR_GET_KERNEL_TABLE, DEVICE_INFO_TABLE — request that size and
                 * are unaffected; the clamp only fires on a genuine over-size, which is a bug
                 * and a guest-OOB-write hazard.) */
                if (req_psize > 0) {
                    uint32_t out_psize = ldl_le_p(resp + 96);
                    if (out_psize > req_psize) {
                        qemu_log("nvkvm-gpu[%s] M9 CTRL-CLAMP cmd=0x%08x reply psize %u -> %u "
                                 "(caller buffer)\n", s->chip->name, ctrl, out_psize, req_psize);
                        stl_le_p(resp + 96, req_psize);
                        stl_le_p(resp + 56, 32u + 40u + req_psize);
                    }
                }
                /* DIAG(B-compute): log controls we did NOT fill with real data
                 * (cr==NULL and not special-cased) that REQUEST a non-zero
                 * response (a GET) — these return NV_OK+zeros and are the
                 * cuInit=100 compute-cap suspects. */
                if (!cr && ctrl != 0x20800a5cu && ctrl != 0x20801112u &&
                    ctrl != 0x20802a08u && ctrl != 0x208001b0u &&
                    ctrl != 0x20800102u && ctrl != 0x20801303u &&
                    ctrl != 0x20803801u) {
                    uint32_t reqpsize = ldl_le_p(resp + 96);
                    if (reqpsize > 0) {
                        qemu_log("nvkvm-gpu[%s] CTRL-UNFILLED cmd=0x%08x psize=%u "
                                 "-> echoed NV_OK+zeros\n",
                                 s->chip->name, ctrl, reqpsize);
                    }
                }
            }
            if (fn == 65) {
                /* GET_GSP_STATIC_INFO (fn 65): NOT a control — the GSP returns the
                 * full GspStaticConfigInfo struct directly at the rpc body
                 * (rpc_message->get_gsp_static_info_v14_00.data = element+80,
                 * right after the 32B rpc header).  Replay the captured GA106
                 * struct (1792B incl. fbRegionInfoParams numFBRegions=5).
                 * rpc.length = 32(hdr) + sizeof(struct); single element. */
                memcpy(resp + 80, gspstaticinfo_ga106, sizeof(gspstaticinfo_ga106));
                stl_le_p(resp + 56, 32u + GSPSTATICINFO_GA106_SIZE);
                /* GspStaticConfigInfo.bar2PdeBase @ offset 1672 (verified by a
                 * host printk) = the GSP-chosen BAR2 page-dir base.  The guest
                 * reads this and roots its BAR2 page tables here; use it as our
                 * GMMU walk root + enable VIRTUAL translation. */
                if (GSPSTATICINFO_GA106_SIZE >= NVKVM_GSPSTATIC_BAR2PDEBASE_OFF + 8) {
                    s->bar2_pdb = ldq_le_p(gspstaticinfo_ga106 + NVKVM_GSPSTATIC_BAR2PDEBASE_OFF);
                    s->bar2_virtual = (s->bar2_pdb != 0);
                    /* bar1PdeBase precedes bar2PdeBase (consecutive NvU64). */
                    s->bar1_pdb = ldq_le_p(gspstaticinfo_ga106 +
                                           NVKVM_GSPSTATIC_BAR2PDEBASE_OFF - 8);
                    qemu_log("nvkvm-gpu[%s] M6: BAR2 root PDB (GSP static) = 0x%llx "
                             "BAR1 root PDB = 0x%llx\n", s->chip->name,
                             (unsigned long long)s->bar2_pdb,
                             (unsigned long long)s->bar1_pdb);
                }
            }
            if (fn == 70) {
                /* UPDATE_BAR_PDE (fn 70): on bare-metal GSP-client the GSP binds
                 * BAR2; the kernel sends GSP the BAR2 root PDE to write into the
                 * bound root page directory (kern_bus.c:880).  Body @ element+80:
                 * barType@80 (1=BAR2), entryValue@88 (the root PDE -> kernel's
                 * next-level table), entryLevelShift@96.  We emulate the bind by
                 * writing entryValue into our FB backing at the GSP root PDB
                 * (bar2_pdb) index 0 — the GMMU walk then follows it into the
                 * kernel's page tables (already in FB via PRAMIN). */
                uint32_t bartype = ldl_le_p(cmd + 80);
                uint64_t entryval = ldq_le_p(cmd + 88);
                uint64_t lvlshift = ldq_le_p(cmd + 96);
                if (bartype == 1 /* NV_RPC_UPDATE_PDE_BAR_2 */ && s->bar2_pdb) {
                    nvkvm_fb_write(s, s->bar2_pdb, entryval, 8);
                    qemu_log("nvkvm-gpu[%s] M6: UPDATE_BAR_PDE BAR2 root[0] @ "
                             "0x%llx <- 0x%llx (shift=%llu)\n", s->chip->name,
                             (unsigned long long)s->bar2_pdb,
                             (unsigned long long)entryval,
                             (unsigned long long)lvlshift);
                } else if (bartype == 0 /* NV_RPC_UPDATE_PDE_BAR_1 */ && s->bar1_pdb) {
                    nvkvm_fb_write(s, s->bar1_pdb, entryval, 8);
                    qemu_log("nvkvm-gpu[%s] M6: UPDATE_BAR_PDE BAR1 root[0] @ "
                             "0x%llx <- 0x%llx (shift=%llu)\n", s->chip->name,
                             (unsigned long long)s->bar1_pdb,
                             (unsigned long long)entryval,
                             (unsigned long long)lvlshift);
                }
            }
            /* DIAG(init-stall): log every serviced RPC so we can see the last
             * one before the 4s _threadNodeCheckTimeout.  fn=76 controls also
             * print their ctrl cmd. */
            qemu_log("nvkvm-gpu[%s] M4: RPC fn=%u cmd=0x%x reqPsize=%u -> "
                     "respPsize=%u status=0x%x rpclen=%u\n", s->chip->name, fn,
                     (fn == 76 ? ctrl : 0), ldl_le_p(cmd + 96), ldl_le_p(resp + 96),
                     ldl_le_p(resp + 92), ldl_le_p(resp + 56));
            nvkvm_m3_post_status(s, resp, fn, 0 /* rpc_result NV_OK */);
        }
        /* Advance by the command's ELEMENT COUNT, not by 1.  A GSP_MSG_QUEUE
         * message spans ceil((HDR_SIZE(48) + rpc.length) / SIZE_MIN(4096))
         * queue elements; large controls (e.g. 0x20800a41 paramsSize=8204 =>
         * 3 elements) occupy continuation elements that carry raw payload, NOT
         * rpc headers.  Reading them as separate commands posted bogus fn=0
         * len=0 responses that later failed the driver's
         * GspMsgQueueReceiveStatus ("Incorrect message length 0", msgLen <
         * sizeof(GSP_MSG_QUEUE_ELEMENT)=80) -> NV_ERR_INVALID_PARAM_STRUCT 0x3a.
         * The continuation elements are consumed silently (one response per
         * logical command). */
        {
            uint32_t msglen = 48u + ldl_le_p(cmd + 56);
            uint32_t elems = (msglen + 4095u) / 4096u;
            if (elems == 0) {
                elems = 1;
            }
            s->cmd_readptr = (s->cmd_readptr + elems) % s->q_msgcount; /* wrap */
        }
    }
    /* ack consumption: advance the cmd-queue read pointer.  The GSP queues are
     * created with MSGQ_FLAGS_SWAP_RX (message_queue_cpu.c:180), so the readPtr
     * is SWAPPED into the OTHER queue's backing store: as the cmd-queue consumer
     * our pReadOutgoing = &pOurRxHdr->readPtr, and pOurRxHdr lives in the queue
     * WE created (the status queue).  So write the cmd readPtr to the STATUS
     * queue's rx header (stat_base + rxHdrOff 0x20) — the guest-producer reads
     * it there via its pReadIncoming.  Writing it to cmd_base+0x20 (no-swap
     * location) left the guest seeing 0 frees -> "buffer is full" once init
     * accumulated ~msgCount(63) command elements. */
    uint8_t rp[4];
    stl_le_p(rp, s->cmd_readptr);
    nvkvm_dmaw(pdev, s->q_shmem + s->q_stat_base + 0x20, rp, sizeof(rp));
    /* #14 piece-2: consume the MC_SERVICE_INTERRUPTS completion-retry kick — now
     * that this batch's responses are posted and the read pointer is acked, replay
     * the work-submit doorbell service so any DEFERRED user-channel execution/sema
     * delivery retries and the polling process's completion advances (write sema
     * THEN the guest's poll re-reads it).  Same entry point as a real doorbell
     * ring; idempotent for channels with no pending work.  Only ever set in
     * multiproc mode (see the fn=76 hook). */
    if (s->m2_poll_kick) {
        s->m2_poll_kick = false;
        /* #90 ORD-3: this doorbell is FABRICATED — the guest never wrote it.
         * from_guest=false, so the recorder does not inject a phantom
         * MmioWrite into a stream a replay is supposed to be able to feed
         * back in. */
        nvkvm_bar0_write_inner(s, NVKVM_VF_DOORBELL,
                               s->m2_last_db_valid ? s->m2_last_db_token : 0, 4,
                               false);
    }
}

/* M3-step-1: read the LibOS init-args region array from guest RAM at the GPA
 * the driver programmed into the mailboxes, log each region, and (for the
 * SYSMEM message-queue region) dump the command-queue msgqTxHeader.  This
 * proves the GPA path and gives ground truth before we synthesize responses. */
static void nvkvm_m3_dump_bootargs(NvkvmGpuEmul *s)
{
    uint64_t gpa = ((uint64_t)s->mbox1 << 32) | s->mbox0;
    PCIDevice *pdev = &s->parent_obj;

    qemu_log("nvkvm-gpu[%s] M3: LibOS boot-args GPA = 0x%016llx\n",
             s->chip->name, (unsigned long long)gpa);
    if (gpa == 0) {
        return;
    }

    for (int i = 0; i < 16; i++) {
        uint8_t e[LIBOS_REGION_STRIDE];
        if (nvkvm_dmar(pdev, gpa + (uint64_t)i * LIBOS_REGION_STRIDE,
                         e, sizeof(e)) != MEMTX_OK) {
            qemu_log("nvkvm-gpu[%s] M3:  region[%d] read failed\n",
                     s->chip->name, i);
            break;
        }
        uint64_t id8  = ldq_le_p(e + 0);
        uint64_t pa   = ldq_le_p(e + 8);
        uint64_t sz   = ldq_le_p(e + 16);
        uint8_t  kind = e[24];
        uint8_t  loc  = e[25];
        if (id8 == 0 && pa == 0 && sz == 0) {
            break; /* end of array */
        }
        qemu_log("nvkvm-gpu[%s] M3:  region[%d] id8=0x%016llx pa=0x%016llx "
                 "size=0x%llx kind=%u loc=%u\n", s->chip->name, i,
                 (unsigned long long)id8, (unsigned long long)pa,
                 (unsigned long long)sz, kind, loc);

        /* RMARGS region (id8 "RMARGS") holds GSP_ARGUMENTS_CACHED, which begins
         * with MESSAGE_QUEUE_INIT_ARGUMENTS { u64 sharedMemPhysAddr; u32
         * pageTableEntryCount; NvLength cmdQueueOffset; NvLength statQueueOffset }.
         * The CPU<->GSP message-queue shared region is at sharedMemPhysAddr; the
         * GSP->CPU status queue (whose msgqTxHeader we must init for msgqRxLink)
         * is at sharedMemPhysAddr + statQueueOffset. */
        if (id8 == 0x0000524d41524753ULL /* "RMARGS" */) {
            uint8_t a[32];
            if (nvkvm_dmar(pdev, pa, a, sizeof(a)) == MEMTX_OK) {
                uint64_t shmem = ldq_le_p(a + 0);
                uint32_t ptec  = ldl_le_p(a + 8);
                uint64_t cmdoff = ldq_le_p(a + 16);
                uint64_t statoff = ldq_le_p(a + 24);
                qemu_log("nvkvm-gpu[%s] M3:   RMARGS msgq: sharedMemPA=0x%llx "
                         "pteCount=%u cmdQOff=0x%llx statQOff=0x%llx "
                         "=> statusQueue@0x%llx\n", s->chip->name,
                         (unsigned long long)shmem, ptec,
                         (unsigned long long)cmdoff, (unsigned long long)statoff,
                         (unsigned long long)(shmem + statoff));

                /* M3 keystone step 1: init the GSP->CPU status-queue tx header
                 * so the driver's msgqRxLink links (it polls forever otherwise).
                 * The status queue is structurally identical to the cmd queue
                 * (same size/msgSize/align), so copy the driver's known-good
                 * cmd-queue tx header verbatim, with writePtr=0 (no messages
                 * yet).  GSP is the TX side of the status queue. */
                if (shmem && statoff) {
                    uint8_t txh[32];
                    if (nvkvm_dmar(pdev, shmem + cmdoff, txh, sizeof(txh))
                            == MEMTX_OK) {
                        stl_le_p(txh + 16, 0); /* writePtr = 0 */
                        if (nvkvm_dmaw(pdev, shmem + statoff, txh,
                                          sizeof(txh)) == MEMTX_OK) {
                            qemu_log("nvkvm-gpu[%s] M3:   wrote status-queue tx "
                                     "header @0x%llx (ver=%u size=0x%x msgSize=%u "
                                     "msgCount=%u rxHdrOff=0x%x entryOff=0x%x) "
                                     "-> msgqRxLink should link\n", s->chip->name,
                                     (unsigned long long)(shmem + statoff),
                                     ldl_le_p(txh+0), ldl_le_p(txh+4),
                                     ldl_le_p(txh+8), ldl_le_p(txh+12),
                                     ldl_le_p(txh+24), ldl_le_p(txh+28));
                            /* cache the queue layout for the M4 RPC shim */
                            s->q_shmem        = shmem;
                            s->q_cmd_base     = (uint32_t)cmdoff;
                            s->q_stat_base    = (uint32_t)statoff;
                            s->q_msgsize      = ldl_le_p(txh + 8);
                            s->q_msgcount     = ldl_le_p(txh + 12);
                            s->q_cmd_entryoff = ldl_le_p(txh + 28);
                            s->q_stat_entryoff= ldl_le_p(txh + 28);
                            /* #14 P0: the re-handshake = the quiesced point (GPU was
                             * idle-released; next context boots).  Purge dead-client
                             * resolution/backing state now — never at the free. */
                            nvkvm_m2_reap_dead(s);
                            /* #12 L3 (2026-06-20): RESET the status-queue WRITE
                             * position but PRESERVE the seqNums across a GSP
                             * re-acquire.  The driver's MESSAGE_QUEUE_INFO (and its
                             * rx/txSeqNum) is built in kgspConstructEngine and torn
                             * down only in kgspDestruct (module unload) — NOT on the
                             * cuCtxDestroy-of-last-ctx idle-release — so it PERSISTS
                             * across the re-boot.  Per boot the driver re-links the
                             * status queue (GspStatusQueueInit -> msgqRxLink), which
                             * resets only the POSITION (rxReadPtr=0), never the
                             * seqNum (no rxSeqNum= reset exists anywhere in the gsp
                             * tree — it is only ++'d).  So on re-acquire the guest
                             * still expects the next status element at seqNum N, and
                             * its cmd-queue writePtr continues from N too.  Resetting
                             * stat_seqnum/cmd_readptr to 0 here (as the old one-shot
                             * boot did) makes the re-posted INIT_DONE arrive at seqNum
                             * 0 << N -> msgq treats it as an old package and ignores
                             * it (message_queue_cpu.c:762,768) -> the 2nd context
                             * hangs in kgspWaitForRmInitDone.  On the FIRST boot all
                             * three are already 0 (realize), so preserving is a no-op
                             * there; only stat_writeptr is reset (the per-boot RX
                             * re-link zeroes the guest read pointer, so our write
                             * pointer must match). */
                            s->stat_writeptr  = 0;
                            s->q_ready        = true;
                            /* step 2: re-post GSP_INIT_DONE at the PRESERVED seqNum
                             * (== guest rxSeqNum); first boot posts it at 0. */
                            nvkvm_m3_post_init_done(s);
                        }
                    }
                }
            }
        }
    }
}

static uint64_t nvkvm_chan_translate(NvkvmGpuEmul *s, uint64_t va, bool *out_sys);
static void nvkvm_chan_execute(NvkvmGpuEmul *s);
static uint64_t nvkvm_walk_pdb(NvkvmGpuEmul *s, uint64_t pdb, uint64_t va,
                               bool *out_sys);
static uint64_t nvkvm_walk_pdb_root(NvkvmGpuEmul *s, uint64_t pdb, uint64_t va,
                                    bool root_sys, bool *out_sys);

/* #90 ORD-2 + ORD-3.
 *
 * ORD-2: this function used to log itself LAST, at the bottom of a ~800-line
 * body, AFTER every side effect — which inverts causality in the trace: the
 * doorbell's consequences (queue service, DMA reads, DMA writes, IRQ) appeared
 * BEFORE the write that caused them.  The recorder's MmioWrite is therefore
 * emitted at the TOP, before anything happens.
 *
 * ORD-3: this function is also RE-ENTERED INTERNALLY with a fabricated doorbell
 * (the #14 poll-kick) that the guest never wrote.  `from_guest` distinguishes
 * the two; only a real guest write is recorded, and exactly once.  The
 * m2romregs rom-device thunk (nvkvm_gsp_falcon_write) IS a real guest write, so
 * it comes in through the wrapper below with from_guest=true. */
static void nvkvm_bar0_write_inner(void *opaque, hwaddr off, uint64_t val,
                                   unsigned size, bool from_guest)
{
    NvkvmGpuEmul *s = opaque;

    if (from_guest && nvkvm_rec_on()) {
        nvkvm_rec_mmio_wr(0, nvkvm_rec_bar0_region(off), off, size, val);
    }

    /* M6: BAR0 PRAMIN window write -> sparse FB backing; window-base register. */
    if (off >= NVKVM_PRAMIN_BASE && off < NVKVM_PRAMIN_BASE + NVKVM_PRAMIN_SIZE) {
        uint64_t fa = nvkvm_pramin_fb_addr(s, off);
        nvkvm_t_mark_start();
        uint64_t t0w = nvkvm_now_ns();
        nvkvm_fb_write(s, fa, val, size);
        nvkvm_t_win_wr_ns += nvkvm_now_ns() - t0w; nvkvm_t_win_wr_calls++;
        /* Snoop the BAR2 instance-block PAGE_DIR_BASE.  On the GSP-client path the
         * CPU never writes NV_PBUS_BAR2_BLOCK (0x1714) — the GSP (which we fake)
         * binds BAR2 from the instance block the CPU builds in FB.  The instblk's
         * NV_RAMIN_PAGE_DIR_BASE is word128 (byte 0x200): TARGET[1:0]=VID_MEM(0),
         * PDB_LO[31:12].  Instance blocks are 4 KiB-aligned, so a 4-byte write at
         * (fb&0xFFF)==0x200 with a non-zero VID_MEM page-dir base is an instblk
         * bind; the most-recent one before kbusVerifyBar2 is BAR2's. */
        if (size == 4 && (fa & 0xFFFu) == 0x200u &&
            (val & 0x3u) == 0u && (val & 0xFFFFF000u) != 0u) {
            s->bar2_inst_block = fa - 0x200u;
            if (s->trace) {
                qemu_log("nvkvm-gpu[%s] M6: snooped BAR2 instblk @ FB 0x%llx "
                         "(PDB_LO word=0x%08x)\n", s->chip->name,
                         (unsigned long long)s->bar2_inst_block, (uint32_t)val);
            }
        }
        return;
    }
    if (off == NVKVM_BAR0_WINDOW) {
        s->bar0_window = (uint32_t)val;
        return;
    }
    /* DEBUG-PROOF backdoor: the patched guest UVM reports a tracking-semaphore
     * GPA (lo@0xFFF500, hi@0xFFF504) then writes the payload@0xFFF508 to commit;
     * QEMU forges the GPU's CE SEM_RELEASE by DMA-writing the payload to that
     * guest-RAM GPA, unblocking the UVM channel busy-poll that no observable RPC
     * lets us resolve. Bring-up proof only (see struct comment). */
    if (off == 0xFFF500u) { s->dbg_gpa_lo = (uint32_t)val; return; }
    if (off == 0xFFF504u) { s->dbg_gpa_hi = (uint32_t)val; return; }
    if (off == 0xFFF508u) {
        /* M5.38: AUTHORITATIVE kernel-internal CE-completion simulation.  The
         * patched guest UVM reports its CE tracking-sema GPA (lo@0xFFF500,
         * hi@0xFFF504) and the EXACT payload it is releasing; QEMU writes that
         * value to the guest's sema page.  Per the governing principle this is
         * "simulate exactly" (the CE scrubber is guest-KERNEL-internal CeUtils,
         * never exposed to guest userspace) — NOT a guess: the guest is telling
         * us its own release value, so the write mirrors the guest faithfully
         * (including the legitimate low-value reset UVM does on pool-slot
         * realloc, which resets UVM's own wrap baseline — so no false wrap).
         *
         * This is the RELIABLE writer: the QEMU pushbuffer parser
         * (CE_SEM_RELEASE) flaps/faults on the UVM CE pushbuffers
         * (pb_read=FAULT) and stalls the sema mid-climb, so it cannot carry this
         * alone.  The DESTRUCTIVE writer was never the forge — it was the
         * LAGGING bridged host channel writing stale entry-0/1 payloads (1,2)
         * over the live value 0x1e ~40s late, tripping UVM's 32->64-bit wrap
         * detector; that writer is removed separately (M5.38: sema fwd-map gated
         * on m2hostsem in nvkvm_chan_sem_wr32). */
        uint64_t gpa = ((uint64_t)s->dbg_gpa_hi << 32) | s->dbg_gpa_lo;
        uint8_t b[4]; stl_le_p(b, (uint32_t)val);
        /* M5.49b: the 0xFFF508 backdoor is patched EXCLUSIVELY into uvm_channel.c's
         * CE-push path (docs/kernel_patches/mode2_uvm_complete_proof.patch), so it
         * only ever carries UVM kernel-internal tracking semas (page-table scrubs,
         * CeUtils).  Per the governing map-vs-stub rule those are kernel-only +
         * content-irrelevant and stay SIMULATED even under m2hostsem.  "Narrow
         * host-only completion" forces only the USER-OBSERVABLE CE round-trip
         * (the compute client's CE channels) host-written — handled at the
         * CE_SEM_RELEASE / SEM_EXECUTE parser sites, NOT here.  So always forge. */
        if (gpa) {
            nvkvm_dmaw(&s->parent_obj, gpa, b, 4);
            /* #12-L3c: claim this EXACT sema GPA for the backdoor so the
             * CE_SEM_RELEASE parser never software-writes it via (unreliable) VAS
             * translation.  Per-slot (not per-page): the UVM kernel sema pool packs
             * several channels' tracking semas onto one page, and only the channels
             * whose uvm_channel.c reports via 0xFFF508 are backdoor-owned.  A page-
             * granular claim wrongly suppressed co-located non-backdoor slots
             * (e.g. CeUtils @ ce_utils.c:349), starving their completion. */
            nvkvm_m2_bd_page_add(s, gpa);
            qemu_log("nvkvm-gpu[%s] M5: DBG-FORGE uvm sema GPA=0x%llx <- payload=%u\n",
                     s->chip->name, (unsigned long long)gpa, (uint32_t)val);
        }
        return;
    }
    /* M5 — work-submit doorbell.  Detect the channel submission (the guest wrote
     * the work-submit token).  TODO(M5): execute the channel — walk its GPFIFO ->
     * pushbuffer -> CE semaphore release and write the payload so the driver's
     * channelWaitForFinishPayload poll completes (currently times out at
     * ce_utils.c:349).  For now, log it so the doorbell offset/token are
     * confirmed against the GA100 HAL. */
    if (off == NVKVM_VF_DOORBELL) {
        /* M5.11 (doorbell-demux observability): the guest writes its work-submit TOKEN here
         * (val). On Ampere the token encodes the guest vChid+runlist (NVC36F GET_WORK_SUBMIT_TOKEN).
         * Today we ignore it and ring the host GR token unconditionally — wrong for multi-channel.
         * Log each distinct token (deduped) so we can map guest-token -> channel for the real
         * vChid->sChid demux. No behavior change. */
        if (s->m2_crashwin && (!s->m2_last_db_valid || s->m2_last_db_token != (uint32_t)val)) {
            s->m2_last_db_token = (uint32_t)val; s->m2_last_db_valid = true;
            /* #14 P1 E0 demux: token[11:0] == guest vChid (E0 result).  Resolve it to a
             * channel (chans[].vchid) and thence to the owning process (registry).  P1 =
             * logging only; the DEMUX itself (ring only this channel's token) is P4.  A
             * distinct (vchid -> single channel -> single proc) mapping across 2x cup8 is
             * the acceptance signal that PDB+vChid keying needs no CR3 (plan §1.4). */
            uint32_t vchid = (uint32_t)val & 0xfffu;
            int mch = -1;
            for (int i = 0; i < s->chan_n; i++) {
                if (s->chans[i].vchid_valid && s->chans[i].vchid == vchid) { mch = i; break; }
            }
            int mpi = (mch >= 0) ? nvkvm_m2_proc_find_by_client(s, s->chans[mch].client) : -1;
            qemu_log("nvkvm-gpu[%s] M5.11 DOORBELL token=0x%08x vChid=%u -> chan[%d] "
                     "client=0x%08x proc=%d (chan_n=%d proc_n=%d)\n", s->chip->name,
                     (uint32_t)val, vchid, mch,
                     mch >= 0 ? s->chans[mch].client : 0, mpi, s->chan_n, s->m2_proc_n);
        }
        /* M5.6 EXECUTION-PLANE INVENTORY: once cuCtxCreate has built the GR context
         * (crashwin armed) and starts submitting work, dump the EXACT working set the
         * execution path must back+FIXED-map: va_map = the #2 side-table (PROMOTE_CTX
         * GPU-VA->guest-FB ctx buffers) and chans[] = the channel rings (GPFIFO/USERD).
         * One-shot; logging only. */
        if (s->m2_crashwin && !s->m2_inventory_done) {
            s->m2_inventory_done = true;
            qemu_log("nvkvm-gpu[%s] M5.6 INVENTORY @doorbell va_map_n=%d chan_n=%d:\n",
                     s->chip->name, s->va_map_n, s->chan_n);
            for (int i = 0; i < s->va_map_n; i++) {
                qemu_log("nvkvm-gpu[%s] M5.6   va_map[%d] client=0x%08x VA=0x%llx -> "
                         "%s phys=0x%llx size=0x%llx\n", s->chip->name, i,
                         s->va_map[i].client, (unsigned long long)s->va_map[i].va,
                         s->va_map[i].sys ? "SYS" : "FB",
                         (unsigned long long)s->va_map[i].phys,
                         (unsigned long long)s->va_map[i].size);
            }
            for (int i = 0; i < s->chan_n; i++) {
                qemu_log("nvkvm-gpu[%s] M5.6   chan[%d] client=0x%08x gpfifo_va=0x%llx "
                         "ent=%u userd=0x%llx(%s) hvas=0x%08x\n", s->chip->name, i,
                         s->chans[i].client, (unsigned long long)s->chans[i].gpfifo_va,
                         s->chans[i].gpfifo_ent, (unsigned long long)s->chans[i].userd,
                         s->chans[i].userd_sys ? "sys" : "fb", s->chans[i].hvaspace);
            }
        }
        /* M5.7 EXECUTION PLANE (gated m2exec, default off): back the GR working set with
         * real host GPU memory and FIXED-map it into the GR channel's VASpace at the guest
         * VAs, so the host channel's MMU resolves the guest's submitted work. One-shot.
         * The va_map (PROMOTE_CTX) entries carry VA<->guest-FB<->size for the ctx buffers
         * (under sibling RM clients that share the GR address space); we map them under the
         * GR compute client's GR vaspace (0x5c000007), proven mappable in M5.5 P2. This is
         * the FIRST execution-path increment; doorbell-forward + GPFIFO/pushbuffer phys
         * resolution follow. */
        if (s->m2exec && !s->m2_exec_done && s->m2_gr_client) {
            s->m2_exec_done = true;
            uint32_t grc = s->m2_gr_client;
            int mapped = 0;
            for (int i = 0; i < s->va_map_n; i++) {
                if (s->va_map[i].sys) {
                    continue;                 /* sysmem leaf: GPU->CPU DMA path, not here */
                }
                char lbl[24];
                snprintf(lbl, sizeof(lbl), "ctx%d", i);
                if (nvkvm_m2_back_and_map(s, grc, s->va_map[i].va, s->va_map[i].phys,
                                          s->va_map[i].size, false, lbl)) {
                    mapped++;
                }
            }
            qemu_log("nvkvm-gpu[%s] M5.7 EXEC: backed %d/%d FB working-set buffers into GR "
                     "client 0x%08x VASpace\n", s->chip->name, mapped, s->va_map_n, grc);
            /* The GR channel's GPFIFO VA is a UVM mapping NOT forwarded to the host, so it
             * is FREE in 0x5c000007 (probe: FIXED map there SUCCEEDS, unlike the already-
             * host-mapped ctx VAs). For real operation we must DOUBLE-mmap it: resolve its
             * guest-FB phys by walking the guest GR page tables (try each snooped VAS PDB),
             * so the guest's submitted GP entries (written via BAR->fb_write at that phys)
             * land in the SAME host memory the host channel's GPFIFO reads. */
            for (int i = 0; i < s->chan_n; i++) {
                if (s->chans[i].client != grc || !s->chans[i].gpfifo_va) {
                    continue;
                }
                uint64_t gva = s->chans[i].gpfifo_va, gphys = 0;
                for (int v = 0; v < s->chan_vas_n; v++) {
                    bool sy = false;
                    uint64_t p = nvkvm_walk_pdb(s, s->chan_vas[v].pdb, gva, &sy);
                    if (p != NVKVM_GMMU_FAULT && !sy) {
                        gphys = p & ~0xfffull;
                        qemu_log("nvkvm-gpu[%s] M5.7 GPFIFO VA=0x%llx resolved via VAS[%d] "
                                 "pdb=0x%llx -> FB phys=0x%llx\n", s->chip->name,
                                 (unsigned long long)gva, v,
                                 (unsigned long long)s->chan_vas[v].pdb,
                                 (unsigned long long)gphys);
                        break;
                    }
                }
                if (!gphys) {
                    qemu_log("nvkvm-gpu[%s] M5.7 GPFIFO VA=0x%llx phys UNRESOLVED "
                             "(chan_vas_n=%d) — VA-only map\n", s->chip->name,
                             (unsigned long long)gva, s->chan_vas_n);
                }
                nvkvm_m2_back_and_map(s, grc, gva, gphys, 0x10000, true, "gpfifo");
                break;
            }
            /* M5.8: set up doorbell-forward primitives (map host USERMODE + fetch the GR
             * channel work-submit token). NOT rung yet — ringing before pushbuffers are
             * mapped + the channel scheduled would fault/wedge the host GPU. */
            nvkvm_m2_doorbell_setup(s, grc);
        }
        /* M5.9/M5.22: real execution forward — map this doorbell's new GR pushbuffers and RING
         * the host doorbell (per-channel token, unconditional) so the HOST GPU runs the work.
         * The chan_execute Phase-B semaphore write below still runs as a fallback until the host
         * GPFIFO/USERD are bridged; we also deliver the os-event so the guest's blocking-sync
         * poll wakes and re-reads the semaphore the HOST GPU (or Phase-B) wrote. */
        nvkvm_t_mark_start();
        { uint64_t t0db = nvkvm_now_ns(); uint64_t t0cpu = nvkvm_now_cpu_ns();
          nvkvm_m2_exec_doorbell(s);
          nvkvm_t_doorbell_ns += nvkvm_now_ns() - t0db; nvkvm_t_doorbell_calls++;
          nvkvm_t_doorbell_cpu_ns += nvkvm_now_cpu_ns() - t0cpu; }
        /* Work submitted on SOME channel.  The doorbell token's chid would name
         * it, but during init multiple GPFIFO channels coexist (CeUtils scrubber
         * + its self-verify channel + the host/compute channel) and tracking only
         * the last-allocated one dropped the scrubber's work.  Instead walk EVERY
         * registered channel's pending GPFIFO: a channel with no new work has
         * GP_PUT==gp_get so nvkvm_chan_execute() bails harmlessly.  For each that
         * advanced, honor an explicit CE/NVC56F semaphore release from its
         * pushbuffer; else fall back to the implicit finish-payload semaphore at
         * gpFifoVA + GPFIFO_SIZE(0x8000) + HOST_SEMA(4) = +0x8004 with a
         * per-channel incrementing payload (channelWaitForFinishPayload polls
         * exactly that). */
        bool any_completed = false;
        for (int i = 0; i < s->chan_n; i++) {
            struct nvkvm_chan_entry *c = &s->chans[i];
            /* Load this channel into the chan_* working set chan_execute reads. */
            s->chan_gpfifo_va  = c->gpfifo_va;
            s->chan_userd      = c->userd;
            s->chan_gpfifo_ent = c->gpfifo_ent;
            s->chan_userd_sys  = c->userd_sys;
            s->chan_hvaspace   = c->hvaspace;
            s->chan_client     = c->client;
            s->chan_gp_get     = c->gp_get;
            /* M5.28: route this channel's working-set maps into ITS per-channel fresh VAS
             * (matched by parent TSG). On first touch, mirror the channel's whole guest PDB
             * into the fresh VAS so every guest VA places into a VAS we own (no host-RM
             * ctx self-promote collision). m2_cur_cvas stays set for chan_execute's reactive
             * maps below, then is reset after the iteration. */
            s->m2_cur_cvas = -1;
            for (int ci = 0; ci < s->m2_cvas_n; ci++) {
                if (s->m2_cvas[ci].client == c->client && s->m2_cvas[ci].tsg == c->tsg) {
                    s->m2_cur_cvas = ci; break;
                }
            }
            if (s->m2_cur_cvas >= 0 && !s->m2_cvas[s->m2_cur_cvas].populated) {
                /* M5.32 Step-1b: only latch populated when the walk actually resolved the
                 * PDB + ran; else retry on the next doorbell (deterministic, not one-shot). */
                if (nvkvm_m2_populate_cvas(s, c)) {
                    s->m2_cvas[s->m2_cur_cvas].populated = true;
                }
            }
            /* CE-fwd STEP 0 (gated, one-shot per channel): observe how each user-CE channel
             * is routed BEFORE we touch its data path. Reports COPY engineType, whether it
             * shares a per-channel VAS (m2_cvas, populated?), token validity, and whether a
             * USERD double-mmap (m2_chanbuf qva) exists for it. No side effects. */
            if (s->m2cefwd && !c->ce_route_logged && nvkvm_m2_is_user_ce(s, c->client)) {
                c->ce_route_logged = true;
                uint32_t r_eng = 0; bool r_eng_found = false;
                for (int e = 0; e < s->m2_tsgeng_n; e++) {
                    if (s->m2_tsgeng[e].tsg == c->tsg) {
                        r_eng = s->m2_tsgeng[e].engine; r_eng_found = true; break;
                    }
                }
                int r_cvas = -1;
                for (int ci = 0; ci < s->m2_cvas_n; ci++) {
                    if (s->m2_cvas[ci].client == c->client && s->m2_cvas[ci].tsg == c->tsg) {
                        r_cvas = ci; break;
                    }
                }
                bool r_userd = false;
                for (int k = 0; k < s->m2_chanbuf_n; k++) {
                    if (s->m2_chanbuf[k].client == c->client &&
                        s->m2_chanbuf[k].chan == c->hobject && s->m2_chanbuf[k].qva) {
                        r_userd = true; break;
                    }
                }
                bool r_is_copy = r_eng_found && r_eng >= 0x9u && r_eng <= 0x12u;
                qemu_log("nvkvm-gpu[%s] NVKVM-CEFWD-ROUTE ch[%d] client=0x%08x chid/hObj=0x%08x "
                         "tsg=0x%08x engineType=0x%x%s%s | cvas=%s%s token_valid=%d userd_qva=%s\n",
                         s->chip->name, i, c->client, c->hobject, c->tsg,
                         r_eng, r_eng_found ? "" : "(none)", r_is_copy ? " COPY" : "",
                         r_cvas >= 0 ? "yes" : "no",
                         r_cvas >= 0 ? (s->m2_cvas[r_cvas].populated ? "(populated)" : "(unpopulated)") : "",
                         c->token_valid, r_userd ? "yes" : "no");
            }
            /* M5.41 (deterministic, moved out of the advance-gated M5.25 path): bind +
             * GPFIFO_SCHEDULE the compute client's COPY TSGs ONCE, on first sight — NOT
             * gated on this channel having advanced (that made it fire only flakily, the
             * oracle's 2/4). The CE engine context must be bound to a runlist on the host
             * before the GSP runlist-commit can resolve it (GR self-binds at its 0xc7c0
             * alloc; CE has no equivalent we forward) — without the bind GPFIFO_SCHEDULE
             * returns st=0x57 OBJECT_NOT_FOUND. NVA06C_CTRL_CMD_BIND (0xa06c0102,
             * {engineType}) then GPFIFO_SCHEDULE (0xa06c0101, {bEnable=1}). Skip the GR
             * TSG (M5.8 already scheduled it) + the guest-kernel CE scrubber (simulated). */
            if (s->m2exec && c->tsg && !c->scheduled && c->tsg != s->m2_gr_tsg &&
                c->client != 0xc1d00001u) {
                uint32_t teng = 0;
                for (int e = 0; e < s->m2_tsgeng_n; e++) {
                    if (s->m2_tsgeng[e].tsg == c->tsg) { teng = s->m2_tsgeng[e].engine; break; }
                }
                if (teng >= 0x9u && teng <= 0x12u) {        /* NV2080_ENGINE_TYPE_IS_COPY */
                    uint8_t bp[4]; stl_le_p(bp, teng);
                    uint32_t bst = 0xffff;
                    int brc = nvkvm_m2_control1(s, c->client, c->tsg, 0xa06c0102u,
                                                bp, sizeof(bp), &bst);
                    uint8_t sp[3]; memset(sp, 0, sizeof(sp)); sp[0] = 1;
                    uint32_t sst = 0xffff;
                    int src = nvkvm_m2_control1(s, c->client, c->tsg, 0xa06c0101u,
                                                sp, sizeof(sp), &sst);
                    c->scheduled = true;
                    qemu_log("nvkvm-gpu[%s] M5.41 COPY TSG bind+sched ch[%d] TSG=0x%08x "
                             "engineType=0x%x -> bind rc=%d st=0x%x | sched rc=%d st=0x%x%s\n",
                             s->chip->name, i, c->tsg, teng, brc, bst, src, sst,
                             (sst == 0) ? "  OK SCHEDULED" : "  <-- sched err");
                    /* M5.42 CURSOR ALIGNMENT (user-directed): the host channel's USERD
                     * GP_GET was reset to 0 by the host RM at channel create, but the
                     * guest channel is mid-stream — its real produce index (GP_PUT) is
                     * already in the SHARED USERD page (host_qva via the m2_fbback overlay).
                     * If the host runs with GP_GET=0 it re-fetches the already-consumed
                     * entries [0, our-consume-cursor) -> Xid 31 (MMU fault on a stale
                     * pushbuffer @ the scrubber region 0x121000000) / Xid 32 (corrupted
                     * pushbuffer). ALIGN the host consume cursor ONCE at takeover to our
                     * consumed index (c->gp_get) so the host fetches only [c->gp_get,
                     * GP_PUT) = the genuinely-new work. The host owns GP_GET after this. */
                    for (int k = 0; k < s->m2_chanbuf_n; k++) {
                        if (s->m2_chanbuf[k].client == c->client &&
                            s->m2_chanbuf[k].chan == c->hobject && s->m2_chanbuf[k].qva) {
                            uint32_t cur = ldl_le_p((uint8_t *)s->m2_chanbuf[k].qva + 0x88);
                            stl_le_p((uint8_t *)s->m2_chanbuf[k].qva + 0x88, c->gp_get);
                            qemu_log("nvkvm-gpu[%s] M5.42 align host GP_GET ch[%d] "
                                     "0x%x -> %u (USERD %p)\n", s->chip->name, i, cur,
                                     c->gp_get, s->m2_chanbuf[k].qva);
                            break;
                        }
                    }
                    /* M5.46: fetch THIS channel's host work-submit token NOW — only
                     * valid after the TSG is BIND+SCHEDULE'd (runlist-assigned);
                     * 0xc36f0108 returns 0x40 INVALID_STATE before BIND. Without
                     * this the token-fetch in exec_doorbell's M5.12 pass (which runs
                     * BEFORE this bind) failed, token_valid stayed false, and the
                     * ring fell back to the GR token — telling the host to fetch the
                     * GR channel for this COPY channel's work (ESCHED never ran it).
                     * With a valid per-channel token the M5.22(b) ring below is
                     * correct. */
                    if (!c->token_valid && c->hobject) {
                        uint8_t tp[4]; memset(tp, 0, sizeof(tp)); uint32_t tst = 0xffff;
                        int trc = nvkvm_m2_control1(s, c->client, c->hobject,
                                                    0xc36f0108u, tp, 4, &tst);
                        if (trc == 0 && tst == 0) {
                            c->host_token = ldl_le_p(tp); c->token_valid = true;
                        }
                        qemu_log("nvkvm-gpu[%s] M5.46 post-bind token ch[%d] hObj=0x%08x "
                                 "-> rc=%d st=0x%x token=0x%08x valid=%d (rl=%u chid=%u)\n",
                                 s->chip->name, i, c->hobject, trc, tst, c->host_token,
                                 c->token_valid, (c->host_token >> 16) & 0xffff,
                                 c->host_token & 0xffff);
                    }
                }
            }
            uint32_t before = c->gp_get;
            nvkvm_t_mark_start();
            { uint64_t t0cx = nvkvm_now_ns();
              nvkvm_chan_execute(s);
              nvkvm_t_chan_exec_ns += nvkvm_now_ns() - t0cx; nvkvm_t_chan_exec_calls++; }
            c->gp_get = s->chan_gp_get;          /* save consumed index */
            if (c->gp_get == before) {
                continue;                        /* no new work on this channel */
            }
            /* #12: track lastSubmittedPayload == cumulative GPFIFO entries submitted on
             * this channel (CeUtils does exactly one entry per memset/memcopy and
             * payload=lastSubmitted+1, ce_utils.c:611). Monotonic; handles ring wrap. */
            c->fin_payload += (c->gp_get >= before)
                            ? (c->gp_get - before)
                            : (c->gpfifo_ent - before + c->gp_get);
            any_completed = true;
            /* RESUME-INSTR (host-CE forward, 2026-06-13): disambiguate the CE
             * pool put=0 BEFORE coding. Fires for EVERY advanced channel (NOT
             * gated on token_valid/usermode), so it also catches hypothesis (b)
             * — a copy channel that advanced but never reaches the M5.22 ring.
             *   page A = the fb-backed overlay page chan_execute + the guest's
             *            BAR1 GP_PUT write hit (nvkvm_fb_read @ c->userd).
             *   page B = the m2_chanbuf qva the host GPU's USERD object reads.
             * A!=B  => page-identity divergence = the M5.44/M5.47 keystone, now
             *          for the CE pool (extend the overlay/fbback-first guard).
             * Also reports the gpfifo VA + whether an m2_gpga[] entry shadows
             * c->userd (a GPGA mirror would feed reads from page B not A). */
            if (s->m2_trace) {
                uint32_t a_put = (uint32_t)nvkvm_fb_read(s, c->userd + 0x8C, 4);
                uint32_t a_get = (uint32_t)nvkvm_fb_read(s, c->userd + 0x88, 4);
                void *bqva = NULL;
                for (int k = 0; k < s->m2_chanbuf_n; k++) {
                    if (s->m2_chanbuf[k].client == c->client &&
                        s->m2_chanbuf[k].chan == c->hobject) {
                        bqva = s->m2_chanbuf[k].qva; break;
                    }
                }
                uint32_t b_put = bqva ? ldl_le_p((uint8_t *)bqva + 0x8C) : 0xffffffffu;
                uint32_t b_get = bqva ? ldl_le_p((uint8_t *)bqva + 0x88) : 0xffffffffu;
                int gpga_ov = -1;
                for (int g = 0; g < s->m2_gpga_n; g++) {
                    if (c->userd >= s->m2_gpga[g].gpga_base &&
                        c->userd <  s->m2_gpga[g].gpga_base + s->m2_gpga[g].size) {
                        gpga_ov = g; break;
                    }
                }
                qemu_log("nvkvm-gpu[%s] CE-INSTR ch[%d] client=0x%08x tsg=0x%08x "
                         "userd=0x%llx%s gpfifo=0x%llx gp_get %u->%u | pageA(fb)put=%u "
                         "get=%u | pageB(qva%s)put=%u get=%u | gpga_ov=%d%s\n",
                         s->chip->name, i, c->client, c->tsg,
                         (unsigned long long)c->userd, c->userd_sys ? "(sys)" : "",
                         (unsigned long long)c->gpfifo_va, before, c->gp_get,
                         a_put, a_get, bqva ? "" : " NONE", b_put, b_get, gpga_ov,
                         (bqva && a_put != b_put) ? "  <-- A!=B PAGE-IDENTITY DIVERGENCE" : "");
            }
            /* M5.22 (b): RING this channel's HOST doorbell with ITS own work-submit
             * token so the real host GPU executes the channel's work and writes the
             * real completion.  The working set is mapped into the host VAS at the
             * matching guest VAs (M5.19/M5.21).  Unconditional (the m2ring gate was
             * removed): until the host GPFIFO/USERD are bridged the host sees
             * GP_PUT==GP_GET so a stale ring is a harmless no-op; once bridged this
             * is the real submission.  The Phase-B sema write below still runs as a
             * fallback. */
            /* M5.39: never ring/schedule the host for the guest-KERNEL-internal CE
             * scrubber (client 0xc1d00001 = guest-RM CeUtils).  Per the governing
             * principle a kernel-only path is SIMULATED, not forwarded: its
             * completion sema is already produced by the parsed CE_SEM_RELEASE +
             * the authoritative forge.  Ringing the host only made its CE2 channel
             * execute stale work and FAULT (Xid 31 writing the now-unmapped sema,
             * Xid 32 corrupted pushbuffer) and exhausted its host VAS
             * (dmaAllocMapping) — pure dead weight. */
            if (s->m2_usermode_qva && c->token_valid && c->client != 0xc1d00001u) {
                /* M5.25: the host channel's TSG must be GPFIFO_SCHEDULE'd (on a runlist)
                 * before a ring runs — the guest's schedule control isn't forwarded, so an
                 * unscheduled host TSG is idle and the ring is a no-op (GPU stays 0%).
                 * Schedule once, per channel's parent TSG. NVA06C_CTRL_CMD_GPFIFO_SCHEDULE
                 * (0xa06c0101), params {bEnable=1,bSkipSubmit,bSkipEnable}. */
                /* M5.33: the GR TSG is already GPFIFO_SCHEDULE'd (M5.8 in ctx1, or the
                 * #12 cont.34 GR-TSG schedule in exec_doorbell for a 2nd context); re-
                 * scheduling it here per-channel re-binds it with transient/freed init
                 * channels -> st=0x57. Skip whichever GR TSG is already scheduled.
                 * #14: the check is (client, tsg)-keyed — two processes reuse identical
                 * TSG handle values, so a value-only compare aliased them. */
                if (c->tsg && !c->scheduled &&
                    !nvkvm_m2_tsg_sched_check(s, c->client, c->tsg)) {
                    uint8_t sp[3]; memset(sp, 0, sizeof(sp)); sp[0] = 1;
                    uint32_t sst = 0xffff;
                    int src = nvkvm_m2_control1(s, c->client, c->tsg, 0xa06c0101u,
                                                sp, sizeof(sp), &sst);
                    c->scheduled = true;
                    qemu_log("nvkvm-gpu[%s] M5.25 GPFIFO_SCHEDULE ch[%d] TSG=0x%08x "
                             "client=0x%08x -> rc=%d st=0x%x%s\n", s->chip->name, i,
                             c->tsg, c->client, src, sst,
                             (src == 0 && sst == 0) ? "  OK SCHEDULED" : "  <-- ERR");
                }
                /* M5.26 DIAG: read the HOST USERD GP_PUT/GP_GET (the double-mmapped
                 * page the host GPU reads) to verify (a) the guest's GP_PUT actually
                 * propagated to the host channel and (b) whether the host GPU consumes
                 * it (GP_GET advancing across rings). USERD: GP_GET@0x88, GP_PUT@0x8C. */
                void *uqva = NULL;
                for (int k = 0; k < s->m2_chanbuf_n; k++) {
                    if (s->m2_chanbuf[k].client == c->client &&
                        s->m2_chanbuf[k].chan == c->hobject) {
                        uqva = s->m2_chanbuf[k].qva; break;
                    }
                }
                uint32_t hput = uqva ? ldl_le_p((uint8_t *)uqva + 0x8C) : 0xffffffffu;
                uint32_t hget = uqva ? ldl_le_p((uint8_t *)uqva + 0x88) : 0xffffffffu;
                /* M5.42: do NOT bridge GP_PUT here. The old M5.33 wrote the CONSUME
                 * cursor (c->gp_get) into the PRODUCE offset (+0x8C), clobbering the
                 * guest's real GP_PUT — which is already present in the SHARED USERD
                 * page (host_qva via the m2_fbback overlay; the guest's BAR1 write lands
                 * there directly, proven by the STEP1 trap probe). The host consume
                 * cursor (GP_GET @ +0x88) is aligned ONCE at takeover by M5.42 above;
                 * the host owns it thereafter. So just ring the doorbell — the host sees
                 * the guest's real GP_PUT > aligned GP_GET and fetches the new work. */
                stl_le_p((uint8_t *)s->m2_usermode_qva + 0x90, c->host_token);
                if (s->m2_trace)
                qemu_log("nvkvm-gpu[%s] M5.22 RANG host doorbell ch[%d] token=0x%08x "
                         "(client=0x%08x gpfifo=0x%llx) hostUSERD put=%u get=%u%s\n",
                         s->chip->name, i, c->host_token, c->client,
                         (unsigned long long)c->gpfifo_va, hput, hget,
                         uqva ? "" : " [no host USERD qva]");
            }
            /* #12 L3b (2026-06-21): complete THIS channel's finishPayload semaphore
             * (gpFifoVA + GPFIFO_SIZE(0x8000) + HOST_SEMA(4) = +0x8004) for the
             * GSP-managed *kernel* CeUtils channels (the PMA/heap memory scrubber AND
             * the cuCtxDestroy CeUtils), resolved PER-CHANNEL through the channel's OWN
             * VAS.  This GENERALISES the L1 forge and removes the two reasons the
             * scrubber's finishPayload was never written — the root cause of the
             * scrubberDestruct timeout (ce_utils.c:349) that is the #12 wall:
             *
             *  (1) WRONG GATE — the old forge fired ONLY when the M5.16 BAR1 MRU scan
             *      had pinned a VIDMEM ring (s->chan_gpfifo_phys != 0).  The scrubber
             *      channels (client 0xc1d00001) resolve cleanly through their OWN VAS
             *      (chan_pdb != 0), so M5.16 never runs for them, chan_gpfifo_phys stays
             *      0, the gate failed, and they were never forged (proven: FORGE-RESOLVE
             *      only ever fired for 0xc1e00007, the cuCtxDestroy CeUtils).  Fix:
             *      resolve the finishPayload phys via nvkvm_chan_own_pdb_rs() — the same
             *      authoritative per-channel root chan_translate uses — by walking
             *      gpFifoVA+0x8004 in the channel's address space (VIDMEM *or* SYSMEM;
             *      the kernel CeUtils channel buffer is SYSMEM by default).  The M5.16
             *      bar1off shortcut stays as a fallback for the VIDMEM-ring channel
             *      whose own VAS we don't otherwise hold.
             *
             *  (2) WRONG SKIP — the finishPayload is the channel-HOST semaphore released
             *      per GP entry (channelWaitForFinishPayload polls exactly it), NOT a
             *      pushbuffer CE method.  So honouring a parsed CE SEM_RELEASE (the
             *      scrubber's *tracking* sema at 0x121000010, a different address) does
             *      NOT advance the finishPayload — yet the old `if (chan_sem_released)
             *      continue;` ran FIRST and skipped the completion for any scrubber whose
             *      tracking-sema release we parsed.  Fix: run this completion BEFORE that
             *      continue for kernel CeUtils channels.
             *
             * The scrub is a no-op for our backing (sparse-zero / host pre-zeroed), so
             * per the address-table rule "complete now if no real work" we write the
             * channel's TRUE monotonic submit count (c->fin_payload == lastSubmittedPayload,
             * 1 GPFIFO entry == 1 op) forward-only (cur < fin_payload, never a backward
             * jump that would trip uvm_gpu_semaphore's wrap detector), into the
             * per-channel finishPayload, pinned ONCE (one forward-populated entry, never
             * reverse-resolved — docs/design/mode2_address_table.md).  User-CE / GR
             * channels are excluded (the host executes + releases those for real).
             *
             * cont.27→28: the finishPayload forge is a real-but-SECONDARY completion (cwfp
             * only fires at teardown; cont.28 proved the actual 2nd-ctx hang is a userspace
             * memset over a BAR1-mapped vidmem buffer, an MMIO-write perf wall — NOT this).
             * Re-gated behind m2trace (unvalidated for cup8/LLM as default-on); revisit if the
             * completion is needed once the memset wall is fixed.  See mode2_2nd_context_hang.md. */
            /* #12 cont.33 (2026-07-05): ENABLE by default for kernel CeUtils channels.
             * This forge was previously m2_trace-gated ("unvalidated as default-on")
             * because the tracking-sema-pool collapse (fixed above in nvkvm_chan_sem_wr32
             * — CeUtils' tracking sema no longer lands on UVM's page) fired FIRST and
             * masked it.  With that collapse gone, the CeUtils scrubberDestruct
             * (ce_utils.c:349 "Timed out waiting for the scrub") is now the terminal #12
             * block: it polls THIS channel's finishPayload (gpfifo_va+0x8004), which no
             * parsed CE SEM_RELEASE ever advances (the finishPayload is the channel-HOST
             * semaphore, not a CE method).  The scrub is a no-op on our sparse/pre-zeroed
             * backing, so completing it now (forward-only monotonic submit count) is
             * correct per the address-table rule.  SCOPE: kernel CeUtils only — user-CE
             * and GR channels are excluded (the host executes + releases those for real),
             * so cup8/LLM's compute round-trip is untouched. */
            /* #13: the finishPayload forge below is ANOTHER completion the guest may
             * order compute against.  If this channel's just-executed pushes wrote a
             * compute VAS's page tables and carried no parsed semaphore (the in-parse
             * sync sites never fired), back the dirtied VAS before forging. */
            nvkvm_m2_cpt_sync_at_release(s);
            /* #14: the kernel-CeUtils finishPayload forge must exclude EVERY
             * process's GR/user channels, not just the first client's — the host
             * executes + releases those for real; forging a 2nd process's GR
             * channel would fake-complete work the host hasn't run (correctness)
             * and mask its real starvation.  Single process: is_gr_client is
             * exactly {m2_gr_client} and multiproc() is false — byte-identical. */
            if (!nvkvm_m2_is_gr_client(s, c->client) &&
                !nvkvm_m2_is_user_ce(s, c->client) &&
                !(nvkvm_m2_multiproc(s) && nvkvm_m2_is_user_client(s, c->client))) {
                /* #12 cont.24: forge THIS kernel channel's finishPayload — the channel-HOST
                 * semaphore at gpFifoVA + GPFIFO_SIZE(0x8000) + HOST_SEMA(4) = +0x8004 that
                 * channelWaitForFinishPayload polls (channel_utils.c:344; the GPU releases it via
                 * NVC8B5_SET_SEMAPHORE_A/B at pbGpuVA+finishPayloadOffset == gpfifo_va+0x8004).
                 * Route through the hardened M5.18 writer nvkvm_chan_sem_wr32 — the SAME primitive
                 * the CE SET_SEMAPHORE pushbuffer parser uses, so the forge and the real-CE release
                 * agree on WHERE.  It resolves the VA under the writing client's OWN (client-keyed
                 * m2_cli_vas) VAS — NOT the foreign-alias-prone content-probe the bespoke forge used
                 * (cont.23 proved own-VAS-content-probe resolved to a sysmem page the guest never
                 * read; cont.23's "BAR1→FB 0x31f8004" was itself a scrubber's page via the stomped
                 * global bar1off) — AND mirrors it to the BAR1-relative page the guest polls for a
                 * GSP-managed vidmem channel.  fin_payload is the monotonic submit count
                 * (lastSubmittedPayload), advanced once per GPFIFO entry above; the writer is
                 * forward-only for kernel semas (nothing else advances this one). */
                {
                    uint64_t fin_va = c->gpfifo_va + 0x8004ull;
                    uint64_t redir = 0;
                    nvkvm_chan_sem_wr32(s, fin_va, c->fin_payload, &redir);
                    /* #12 cont.25: the GROUND-TRUTHED case — a bUseBar1 CeUtils channel reads its
                     * finishPayload through BAR1 into VIDMEM (channel_utils.c:272; kprobe proved
                     * the 2nd-ctx hang channel polls a BAR1 GPA, value stuck at 0).  sem_wr32 above
                     * resolves to SYSMEM/own-VAS (correct for bUseBar1=0 channels like CTX1) but the
                     * guest never reads sysmem for a bUseBar1 channel.  So ALSO write the FB page the
                     * guest's BAR1 poll resolves to: bar1_pdb walk of THIS channel's ring BAR1 offset
                     * (captured by M5.16 just above, even without a VAS) + (gpfifo_va&0xfff) + 0x8004.
                     * Forward-only; harmless for bUseBar1=0 channels (chan_fin_ring_found stays false
                     * — their ring isn't BAR1-written).  The two writes are non-conflicting: each
                     * channel reads exactly one aperture; the other write lands on a page nobody reads. */
                    uint64_t fin_fb = NVKVM_GMMU_FAULT;
                    if (s->chan_fin_ring_found && s->bar1_pdb) {
                        uint64_t fin_b1 = s->chan_fin_ring_off + (c->gpfifo_va & 0xfffull) + 0x8004ull;
                        bool fsys = false;
                        uint64_t fb = nvkvm_walk_pdb(s, s->bar1_pdb, fin_b1, &fsys);
                        if (fb != NVKVM_GMMU_FAULT && !fsys) {
                            uint32_t cur = (uint32_t)nvkvm_fb_read(s, fb, 4);
                            if (cur < c->fin_payload) {     /* forward-only, never rewind */
                                nvkvm_fb_write(s, fb, c->fin_payload, 4);
                            }
                            fin_fb = fb;
                        }
                    }
                    if (s->m2_trace) {
                        qemu_log("nvkvm-gpu[%s] #12 FORGE finishPayload ch[%d] gpfifo=0x%llx "
                                 "fin_va=0x%llx payload=%u sysredir=0x%llx ring_off=0x%llx "
                                 "barFB=0x%llx client=0x%08x\n", s->chip->name, i,
                                 (unsigned long long)c->gpfifo_va, (unsigned long long)fin_va,
                                 c->fin_payload, (unsigned long long)redir,
                                 s->chan_fin_ring_found ? (unsigned long long)s->chan_fin_ring_off : 0ull,
                                 (unsigned long long)fin_fb, c->client);
                    }
                }
            }
            if (s->chan_sem_released) {
                continue;                        /* explicit release already done */
            }
            (void)c->payload;
            continue;
        }
        s->m2_cur_cvas = -1;    /* M5.28: clear per-channel VAS routing after the loop */
        /* M5/M7 — a channel finished: deliver the os-event completion so
         * libcuda's blocking-sync poll() wakes (write sema THEN signal, so the
         * payload is already visible when the guest re-checks).  Posting per
         * completed-doorbell is bounded by the queue ring; the guest drains all
         * queued POST_EVENTs in one SWGEN0 service. */
        if (any_completed) {
            uint64_t t0ev = nvkvm_now_ns();
            nvkvm_gsp_deliver_events(s);
            nvkvm_t_event_ns += nvkvm_now_ns() - t0ev; nvkvm_t_event_calls++;
        }
        /* M5.11b: per-window timeshare sample, keyed to doorbell count so steady-state
         * generation is sampled evenly (every ~400 doorbells ~= a handful of tokens). */
        if (nvkvm_t_doorbell_calls && (nvkvm_t_doorbell_calls % 400u) == 0u) {
            nvkvm_timeshare_window_dump(s, "doorbell-window");
        }
        return;
    }
    /* M7 — CPU interrupt tree writes. */
    if (off == NVKVM_VF_INTR_LEAF_TRIGGER) {
        /* The driver triggers an interrupt by writing its vector here: set the
         * leaf+top pending bits and raise the MSI so the ISR fires (this is what
         * _osVerifyInterrupts polls for). */
        uint32_t vec = (uint32_t)val & 0xFFFu;
        uint32_t leaf = vec / 32u, bit = vec % 32u, subtree = leaf / 2u;
        if (leaf < NVKVM_VF_INTR_NLEAF) {
            s->intr_leaf[leaf] |= (1u << bit);
            s->intr_top |= (1u << subtree);
            PCIDevice *pd = &s->parent_obj;
            if (msix_enabled(pd)) {
                msix_notify(pd, 0);   /* single stall vector; ISR demuxes via TOP/LEAF */
                nvkvm_rec_irq_msix(0);      /* #90 */
            } else {
                pci_set_irq(pd, 1);
                nvkvm_rec_irq_intx(1);      /* #90 */
            }
            qemu_log("nvkvm-gpu[%s] M7: INTR trigger vec=%u -> leaf[%u] bit%u "
                     "subtree%u, MSI raised\n", s->chip->name, vec, leaf, bit, subtree);
        }
        return;
    }
    if (off >= NVKVM_VF_INTR_LEAF0 && off < NVKVM_VF_INTR_LEAF0 + NVKVM_VF_INTR_NLEAF*4) {
        uint32_t i = (off - NVKVM_VF_INTR_LEAF0) / 4;     /* LEAF(i): write-1-to-clear */
        s->intr_leaf[i] &= ~(uint32_t)val;
        uint32_t st = i / 2u;
        if (s->intr_leaf[st*2] == 0 && (st*2+1 >= NVKVM_VF_INTR_NLEAF ||
            s->intr_leaf[st*2+1] == 0)) {
            s->intr_top &= ~(1u << st);
        }
        if (s->intr_top == 0 && !msix_enabled(&s->parent_obj)) {
            pci_set_irq(&s->parent_obj, 0);
            nvkvm_rec_irq_intx(0);          /* #90: deassert is observable too */
        }
        return;
    }
    if (off >= NVKVM_VF_INTR_LEAF_EN_SET0 && off < NVKVM_VF_INTR_LEAF_EN_SET0 + NVKVM_VF_INTR_NLEAF*4) {
        s->intr_leaf_en[(off - NVKVM_VF_INTR_LEAF_EN_SET0)/4] |= (uint32_t)val; return;
    }
    if (off >= NVKVM_VF_INTR_LEAF_EN_CLR0 && off < NVKVM_VF_INTR_LEAF_EN_CLR0 + NVKVM_VF_INTR_NLEAF*4) {
        s->intr_leaf_en[(off - NVKVM_VF_INTR_LEAF_EN_CLR0)/4] &= ~(uint32_t)val; return;
    }
    if (off == NVKVM_VF_INTR_TOP_EN_SET0) { s->intr_top_en |= (uint32_t)val; return; }
    if (off == NVKVM_VF_INTR_TOP_EN_CLR0) { s->intr_top_en &= ~(uint32_t)val; return; }
    /* M6: NV_PBUS_BAR2_BLOCK (0x1714) PTR[27:0] = BAR2 instance-block FB addr
     * (in NV_RAMIN_BASE_SHIFT=12 units).  Caches the page-dir base source for
     * the BAR2 GMMU walk. */
    /* BAR2 bind register: NV_PBUS_BAR2_BLOCK (0x1714) on Maxwell, OR the
     * Turing/Ampere VF variant NV_VIRTUAL_FUNCTION_PRIV_BAR2_BLOCK at BAR0
     * 0xB80F48 (NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET 0xB80000 + 0xF48).  PTR
     * [27:0]<<12 = instblk FB addr; MODE bit31 = 1 VIRTUAL / 0 PHYSICAL. */
    if (off == NVKVM_PBUS_BAR2_BLOCK || off == NVKVM_VF_BAR2_BLOCK) {
        s->bar2_inst_block = (uint64_t)(val & 0x0FFFFFFFu) << NVKVM_BAR2_BLOCK_PTR_SHIFT;
        s->bar2_virtual    = (val & NVKVM_BAR2_BLOCK_MODE_VIRTUAL) != 0;
        qemu_log("nvkvm-gpu[%s] M6: BAR2_BLOCK@0x%llx -> instblk FB 0x%llx mode=%s\n",
                 s->chip->name, (unsigned long long)off,
                 (unsigned long long)s->bar2_inst_block,
                 s->bar2_virtual ? "VIRTUAL" : "PHYSICAL");
        return;
    }

    /* M5/M7 — GSP falcon IRQSCLR (0x110004): write-1-to-clear SWGEN0 (bit6).
     * kgspService clears the edge before draining the queue. */
    if (off == 0x00110004u) {
        if (val & (1u << 6)) {
            s->gsp_swgen0_pending = false;
            nvkvm_gsp_falcon_sync(s);     /* M5.64: clear IRQSTAT bit6 in the rom-device RAM */
        }
        return;
    }

    /* #12 (2026-06-20): a GSP-falcon DMA transfer (DMATRFCMD) issued WHILE suspended
     * means the guest is re-loading the GSP image to RE-BOOT it for a new context
     * (cuCtxDestroy of the last ctx sent fn-47 UNLOADING -> gsp_suspended; the next
     * cuCtxCreate reloads the falcon then STARTCPUs).  Latch it so the STARTCPU below
     * is recognised as a genuine re-boot (raise WPR2) rather than a bare trailing-
     * teardown STARTCPU (keep WPR2 down).  Without this the 2nd context hangs forever
     * waiting for a GSP_INIT_DONE that never comes (WPR2 never re-raised). */
    if (off == NV_PGSP_FALCON_DMATRFCMD && s->gsp_suspended &&
        val != NV_PFALCON_DMATRFCMD_IDLE_VAL) {   /* any transfer while suspended = reload */
        s->gsp_reloaded = true;
    }

    /* #12 L3 (2026-06-20): SEC2 falcon.  The driver runs the SEC2 Booter for both
     * LOAD (kgspExecuteBooterLoad, raises WPR2) and UNLOAD
     * (kgspExecuteBooterUnloadIfNeeded, lowers WPR2).  On a context teardown/re-init
     * it runs Booter Unload and then ASSERTS that WPR2 reads down
     * (kernel_gsp_booter_tu102.c "WPR2 is still up" -> osinit.c:2363).  From BAR0
     * alone Load and Unload differ only by the mailbox args written before the SEC2
     * STARTCPU: a NORMAL Unload writes MAILBOX0/1 = 0xff (Load writes 0 or the WprMeta
     * GPA; GC6 Unload writes 0xdeaddead, not our path).  Latch SEC2 MAILBOX0; on the
     * SEC2 STARTCPU (CPUCTL bit1), if it is 0xff this is a Booter Unload -> drop WPR2. */
    if (off == NV_PSEC_FALCON_MAILBOX0) {
        s->sec_mbox0 = (uint32_t)val;
    }
    if (off == NV_PSEC_FALCON_CPUCTL && (val & 0x2u) && s->sec_mbox0 == 0xffu) {
        if (s->fwsec_ran && s->trace) {
            qemu_log("nvkvm-gpu[%s] M4: SEC2 Booter Unload (mbox0=0xff) -> WPR2 down\n",
                     s->chip->name);
        }
        s->fwsec_ran = false;   /* WPR2 lowered, matching kgspIsWpr2Up post-Unload */
    }

    /* M3: GSP falcon STARTCPU => FWSEC "executes" => WPR2 becomes initialized.
     * (CPUCTL bit1 STARTCPU, or via CPUCTL_ALIAS 0x110130.) */
    if ((off == NV_PGSP_FALCON_CPUCTL || off == 0x00110130u) && (val & 0x2u)) {
        /* A GSP-falcon STARTCPU here is a FRESH FWSEC/GSP boot => WPR2 comes UP — but
         * ONLY when we are not in a teardown/suspended phase.  On driver teardown the
         * guest sends fn=47 UNLOADING (sets gsp_suspended + WPR2 down) and then issues a
         * trailing STARTCPU as part of the unload sequence; that one must NOT re-raise
         * WPR2, or the NEXT adapter init reads WPR2 up, runs Booter Unload (a separate
         * SEC2 path we don't model), finds it still up, and asserts "WPR2 still up" —
         * which is why only the first init per QEMU lifetime used to succeed.  Gating the
         * raise on !gsp_suspended leaves WPR2 down after teardown, so a reload / reopen
         * boots cleanly.  The teardown STARTCPU still clears gsp_suspended, so the next
         * genuine boot STARTCPU (suspended already false) raises WPR2 as normal. */
        /* A post-UNLOADING STARTCPU is a GENUINE re-boot iff the guest re-loaded the
         * GSP image first (gsp_reloaded) — a context RE-ACQUIRE (cuCtxDestroy ->
         * fn-47 -> cuCtxCreate) does exactly that, and must raise WPR2 or the 2nd
         * context hangs forever on GSP_INIT_DONE (#12 next-layer, 2026-06-20).  A bare
         * trailing-teardown STARTCPU (no reload) must NOT re-raise WPR2 (the original
         * reload-cascade fix). */
        bool was_suspended = s->gsp_suspended;
        bool teardown = s->gsp_suspended && !s->gsp_reloaded;
        s->gsp_suspended = false;       /* any STARTCPU => GSP active, not suspended */
        s->gsp_reloaded  = false;       /* consume the reload latch                  */
        if (!teardown && !s->fwsec_ran) {
            s->fwsec_ran = true;
            if (s->trace) {
                qemu_log("nvkvm-gpu[%s] M3: GSP STARTCPU -> FWSEC ran, WPR2 up\n",
                         s->chip->name);
            }
            /* #12 L3 (2026-06-20): a genuine GSP RE-acquire (cuCtxDestroy of the
             * last ctx -> fn-47 idle-release -> next cuCtxCreate) reuses the
             * existing boot-args + GSP message queue and does NOT re-write the
             * boot-args mailbox, so the mailbox-keyed dump below never re-runs and
             * GSP_INIT_DONE is never re-posted -> the guest polls the status queue
             * in sysmem forever.  This is the real 2nd-context hang: the SEC2 Booter
             * Load and WPR2 re-raise both complete cleanly UPSTREAM of this (verified
             * in trace: MAILBOX0=0, WPR2_HI up), then the guest goes quiet on BAR0
             * waiting on INIT_DONE.  Re-post from the cached boot-args GPA: the queue
             * allocations persist across the idle-release and mbox0/mbox1 still hold
             * the right GPA (the guest never rewrote them).  was_suspended (true only
             * on a re-acquire; the first boot was never suspended) distinguishes this
             * from the first boot, whose own mailbox write drives the dump as usual. */
            if (was_suspended && (s->mbox0 | s->mbox1)) {
                nvkvm_m3_dump_bootargs(s);
                s->bootargs_dumped = true; /* don't double-post if mailbox is rewritten */
            }
        } else if (s->trace) {
            qemu_log("nvkvm-gpu[%s] M4: teardown-phase STARTCPU off=0x%llx -> WPR2 "
                     "stays %s (no spurious re-raise)\n", s->chip->name,
                     (unsigned long long)off, s->fwsec_ran ? "up" : "down");
        }
    }

    /* M4: cmd-queue doorbell — the driver wrote NV_PGSP_QUEUE_HEAD(0) to notify
     * the GSP of new command(s).  Service the cmd queue (echo NV_OK responses). */
    if (off == NVKVM_GSP_QUEUE_HEAD0 && s->q_ready) {
        nvkvm_m3_service_cmdq(s);
    }

    /* M3: capture the LibOS boot-args GPA from the GSP falcon mailboxes. */
    if (off == NV_PGSP_FALCON_MAILBOX0) {
        s->mbox0 = (uint32_t)val;
    } else if (off == NV_PGSP_FALCON_MAILBOX1) {
        s->mbox1 = (uint32_t)val;
        if (!s->bootargs_dumped && (s->mbox0 | s->mbox1)) {
            s->bootargs_dumped = true;
            nvkvm_m3_dump_bootargs(s);
        }
    }

    if (s->m2_trace && s->trace) {
        const char *nm = nvkvm_reg_name(off);
        qemu_log("nvkvm-gpu[%s] #%llu BAR0 WR  off=0x%06llx sz=%u <- 0x%08llx%s%s\n",
                 s->chip->name, (unsigned long long)s->access_count++,
                 (unsigned long long)off, size, (unsigned long long)val,
                 nm ? "  " : "", nm ? nm : "");
    }
    /* M0: writes are observed only.  M1/M2 add the state machine. */
}

/* The MemoryRegionOps entry point: every write arriving here came from the
 * guest (#90 ORD-3). */
static void nvkvm_bar0_write(void *opaque, hwaddr off, uint64_t val,
                             unsigned size)
{
    nvkvm_bar0_write_inner(opaque, off, val, size, true);
}

static const MemoryRegionOps nvkvm_bar0_ops = {
    .read       = nvkvm_bar0_read,
    .write      = nvkvm_bar0_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .impl       = { .min_access_size = 4, .max_access_size = 4 },
    .valid      = { .min_access_size = 1, .max_access_size = 8 },
};

/* M5.64: rom-device write thunk for the GSP-falcon page. Reads are served from the region's RAM
 * (kept current by nvkvm_gsp_falcon_sync — zero vmexit); writes carry the SIDE EFFECTS (QUEUE_HEAD
 * doorbell, CPUCTL STARTCPU, IRQSCLR W1C) so they must still reach nvkvm_bar0_write. The subregion
 * is at BAR0 offset 0x110000, so re-absolutize the offset. */
static void nvkvm_gsp_falcon_write(void *opaque, hwaddr off, uint64_t val, unsigned size)
{
    nvkvm_bar0_write(opaque, off + 0x00110000u, val, size);
}
static const MemoryRegionOps nvkvm_gsp_falcon_ops = {
    .write      = nvkvm_gsp_falcon_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .impl       = { .min_access_size = 4, .max_access_size = 4 },
    .valid      = { .min_access_size = 1, .max_access_size = 8 },
};

/* BAR1 — RM "BAR1" FB aperture.  The driver maps channel USERD, pushbuffers and
 * semaphores (in FB) into BAR1 and the CPU reads/writes them through it — most
 * importantly GP_PUT @ USERD+0x8C, the channel work-submit.  A no-op stub here
 * silently dropped those writes (GP_PUT never reached the FB backing → the CE
 * scrubber's channel never advanced → ce_utils.c:349 timeout).  BAR1 is a GPU
 * virtual aperture (its own page tables in FB, root = bar1_pdb from
 * GspStaticConfigInfo.bar1PdeBase + the UPDATE_BAR_PDE(BAR_1) root entry), so a
 * BAR1 offset is a GPU VA: GMMU-VER2-walk it to FB/sysmem (nvkvm_walk_pdb). */
static uint64_t nvkvm_baraperture_read_inner(void *opaque, hwaddr off, unsigned size)
{
    NvkvmGpuEmul *s = opaque;
    if (!s->bar1_pdb) {
        return 0;
    }
    bool sys = false;
    uint64_t pa = nvkvm_walk_pdb(s, s->bar1_pdb, off, &sys);
    if (pa == NVKVM_GMMU_FAULT) {
        if (s->m2_crashwin) {                /* M6.6 DIAG: does libcuda read BAR1 but FAULT? */
            static uint32_t fcnt;
            if (fcnt++ < 200) {
                qemu_log("nvkvm-gpu[GA106] M6.6 BAR1 RD off=0x%llx -> WALK-FAULT (returns 0)\n",
                         (unsigned long long)off);
            }
        }
        return 0;
    }
    uint64_t rv;
    if (sys) {
        uint8_t b[8] = {0};
        if (nvkvm_dmar(&s->parent_obj, pa, b, size) != MEMTX_OK) return 0;
        rv = ldn_le_p(b, size);
    } else {
        s->m2_cur_gva = off ? off : 0;       /* CRASHWIN: report the guest GPU VA */
        rv = nvkvm_fb_read(s, pa, size);
        s->m2_cur_gva = 0;
    }
    /* #12 DIAG: a finishPayload poll spins on ONE address thousands of times.  The
     * earlier LOFB-windowed detector fired 0× — so either the poll reads a page
     * OUTSIDE the [3M,3.3M) window, or it is memslot-served (no trap).  Drop the
     * window and fire only on a genuine spin (same addr read ≥2000× consecutively)
     * so this pinpoints the TRUE finishPayload FB page (compare against the forge's
     * resolved finFB) with negligible noise.  If this also fires 0×, the read is
     * memslot-served and the fix must target that backing. */
    if (s->trace && !sys) {
        static uint64_t last_pa; static uint32_t rep; static uint32_t total;
        if (pa != last_pa) { last_pa = pa; rep = 0; }
        else if (++rep == 2000 && total++ < 200) {   /* 2000× spin on one addr = a poll */
            qemu_log("nvkvm-gpu[GA106] #12 DIAG BAR1 POLL-SPIN off=0x%llx -> FB 0x%llx "
                     "= 0x%llx\n", (unsigned long long)off,
                     (unsigned long long)pa, (unsigned long long)rv);
        }
    }
    /* M5.10 DIAG: after the GR compute object constructs (crashwin), log ALL BAR1 reads
     * (off -> resolved FB/SYS + value + whether m2_fbback-backed) to settle the
     * cuCtxCreate access path: do libcuda's pre-crash reads go via BAR1 (trapped here)
     * and to which FB, and is that FB backed? Capped. */
    if (s->m2_crashwin) {
        static uint32_t bcnt;
        if (bcnt++ < 400) {
            bool backed = (s->m2_fbback_n && nvkvm_fb_host_overlay(s, pa) != NULL);
            qemu_log("nvkvm-gpu[GA106] M5.10 BAR1 RD off=0x%llx -> %s 0x%llx = 0x%llx %s\n",
                     (unsigned long long)off, sys ? "SYS" : "FB", (unsigned long long)pa,
                     (unsigned long long)rv, backed ? "[BACKED]" : "[unbacked]");
        }
    }
    return rv;
}

static void nvkvm_baraperture_write_inner(void *opaque, hwaddr off, uint64_t val,
                                    unsigned size)
{
    NvkvmGpuEmul *s = opaque;
    if (!s->bar1_pdb) {
        return;
    }
    /* BAR1-TRAP-INSTR (2026-06-13): settle memslot-vs-MMIO for the cup5 bulk copy.
     * If the guest's HtoD writes route through here, BAR1 is MMIO-trapping every
     * access (fix = install a RAM memslot/double-mmap so writes stop trapping);
     * if this stays ~0 during a 64MB copy, dp is already a memslot (fix = WB
     * cacheability only). Log cumulative call/byte counts each 16 MiB crossed. */
    {
        static uint64_t bw_calls, bw_bytes, bw_next = (16ull << 20);
        bw_calls++; bw_bytes += size;
        if (bw_bytes >= bw_next) {
            qemu_log("nvkvm-gpu[GA106] BAR1-TRAP-INSTR cumulative writes: calls=%llu "
                     "bytes=%llu (%.1f MiB)\n", (unsigned long long)bw_calls,
                     (unsigned long long)bw_bytes, (double)bw_bytes / (1024*1024));
            bw_next += (16ull << 20);
        }
    }
    bool sys = false;
    uint64_t pa = nvkvm_walk_pdb(s, s->bar1_pdb, off, &sys);
    if (pa == NVKVM_GMMU_FAULT) {
        return;
    }
    if (sys) {
        uint8_t b[8];
        stn_le_p(b, size, val);
        nvkvm_dmaw(&s->parent_obj, pa, b, size);
    } else {
        nvkvm_fb_write(s, pa, val, size);
        /* M5.16: remember this vidmem page as a guest-CPU-written backing (the
         * authoritative ring location; see bar1_wpg comment).  MRU-ordered. */
        uint64_t pg = pa & ~0xFFFull;
        int hit = -1;
        for (int i = 0; i < s->bar1_wpg_n; i++) {
            if (s->bar1_wpg[i].page == pg) { hit = i; break; }
        }
        if (hit < 0) {
            if (s->bar1_wpg_n < NVKVM_MAX_BAR1PG) {
                hit = s->bar1_wpg_n++;
            } else {                         /* evict LRU (smallest seq) */
                hit = 0;
                for (int i = 1; i < s->bar1_wpg_n; i++) {
                    if (s->bar1_wpg[i].seq < s->bar1_wpg[hit].seq) { hit = i; }
                }
            }
            s->bar1_wpg[hit].page = pg;
        }
        s->bar1_wpg[hit].seq = ++s->bar1_wpg_seq;
        s->bar1_wpg[hit].off = off & ~0xFFFull;   /* #12: BAR1 page-offset of this FB page */
    }
    /* DIAG: BAR1 writes into the low-FB region reveal where the guest CPU lays
     * down the UVM channel's GPFIFO entry, pushbuffer, and inits the semaphore. */
    if (s->trace && !sys && pa >= NVKVM_DIAG_LOFB_LO && pa < NVKVM_DIAG_LOFB_HI) {
        static uint32_t total;
        if (total++ < 2000) {
            qemu_log("nvkvm-gpu[GA106] DIAG BAR1 WR off=0x%llx -> FB 0x%llx "
                     "<- 0x%llx sz=%u\n", (unsigned long long)off,
                     (unsigned long long)pa, (unsigned long long)val, size);
        }
    }
    /* M5.16 DIAG: the LOFB-only filter above is blind to the COMPUTE channel's
     * GPFIFO/pushbuffer, which resolve to HIGH FB (e.g. 0x2eee10000 via the CPU-
     * built device-default VAS).  cuCtxCreate's GP-entry never showed up because
     * its write was never logged — not because it never happened.  Log EVERY
     * vidmem BAR1 write outside LOFB too (hard-capped), so the next run settles
     * whether the guest lays the GP entry into the page our GMMU walk resolves
     * (FB 0x2eee1xxxx) — if it does, the data-capture path is correct and the
     * content-pick "require non-zero" heuristic is the only thing rejecting it;
     * if it doesn't, the GPFIFO is reached via a different aperture/backing. */
    if (s->trace && !sys && !(pa >= NVKVM_DIAG_LOFB_LO && pa < NVKVM_DIAG_LOFB_HI)) {
        static uint32_t htotal;
        if (htotal++ < 4000) {
            qemu_log("nvkvm-gpu[GA106] M5.16 BAR1 WR off=0x%llx -> FB 0x%llx "
                     "<- 0x%llx sz=%u\n", (unsigned long long)off,
                     (unsigned long long)pa, (unsigned long long)val, size);
        }
    }
}

/* #90: BAR1 (the FB aperture) — wrapped so the recorded read carries the value
 * ACTUALLY SERVED across all of the body's early returns (walk faults return 0
 * from three separate places). */
static uint64_t nvkvm_baraperture_read(void *opaque, hwaddr off, unsigned size)
{
    uint64_t val = nvkvm_baraperture_read_inner(opaque, off, size);
    nvkvm_rec_mmio_rd(1, NVKVM_REC_M_BAR1, off, size, val);
    return val;
}

static void nvkvm_baraperture_write(void *opaque, hwaddr off, uint64_t val,
                                    unsigned size)
{
    nvkvm_rec_mmio_wr(1, NVKVM_REC_M_BAR1, off, size, val);
    nvkvm_baraperture_write_inner(opaque, off, val, size);
}

static const MemoryRegionOps nvkvm_aperture_ops = {
    .read       = nvkvm_baraperture_read,
    .write      = nvkvm_baraperture_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid      = { .min_access_size = 1, .max_access_size = 8 },
};

/* ── M6: RM BAR2 aperture — GA10x GMMU VER2 page-walk over the FB backing ─────
 * BAR2 (PCI BAR3, 32 MiB) is a GPU virtual aperture: the guest programs page
 * tables (in FB) and an instance block (NV_PBUS_BAR2_BLOCK 0x1714) holding the
 * page-directory base, then accesses VRAM through BAR2.  We walk those tables to
 * translate a BAR2 offset (== GPU VA) to an FB phys addr, then hit fb_pages.
 * VER2 levels (kern_gmmu_fmt_gp10x.c): PD3 VA[48:47], PD2 [46:38], PD1 [37:29],
 * PD0 [28:21] (16B dual PDE), PT_small [20:12] (4 KiB) / PT_big [20:bigShift].
 * Entry addr = field<<shift; PTE/PDE ADDRESS_VID = bits 32:8 (<<12). */

static uint64_t nvkvm_fb_rd64(NvkvmGpuEmul *s, uint64_t fb_addr)
{
    return nvkvm_fb_read(s, fb_addr, 8);
}

/* Translate a BAR2 GPU VA to an FB physical address (VID_MEM path only — the
 * page tables and BAR2-mapped surfaces during init live in FB).  Returns
 * NVKVM_GMMU_FAULT on an unmapped/SYSMEM path. */
static uint64_t nvkvm_bar2_translate(NvkvmGpuEmul *s, uint64_t va)
{
    uint64_t tbl;
    if (s->bar2_pdb != 0) {
        /* Page-dir base reported by GSP in GspStaticConfigInfo.bar2PdeBase
         * (offset 1672), which we replay for GET_GSP_STATIC_INFO.  The guest
         * reads the same value and builds its BAR2 page tables (on demand, via
         * the PRAMIN window into our FB backing) rooted here, so this is the
         * walk root. */
        tbl = s->bar2_pdb;
    } else if (s->bar2_inst_block != 0) {
        /* Fallback: read PDB from an instance block (word128 @ +0x200 LO[31:12],
         * word129 @ +0x204 HI[31:0]). */
        uint64_t w128 = nvkvm_fb_read(s, s->bar2_inst_block + NVKVM_RAMIN_PDB_LO_OFF, 4);
        uint64_t w129 = nvkvm_fb_read(s, s->bar2_inst_block + NVKVM_RAMIN_PDB_HI_OFF, 4);
        tbl = (w128 & 0xFFFFF000ull) | (w129 << 32);
    } else {
        return NVKVM_GMMU_FAULT;
    }

    /* PD3 -> PD2 -> PD1 : single 8B PDEs. */
    static const struct { int hi, lo; } lvl[3] = {
        {48, 47}, {46, 38}, {37, 29}
    };
    for (int i = 0; i < 3; i++) {
        uint32_t idx = (uint32_t)((va >> lvl[i].lo) &
                                  ((1ull << (lvl[i].hi - lvl[i].lo + 1)) - 1));
        uint64_t pde = nvkvm_fb_rd64(s, tbl + (uint64_t)idx * 8);
        tbl = NVKVM_VER2_ADDR_VID(pde);
        if (tbl == 0) {
            return NVKVM_GMMU_FAULT;
        }
    }

    /* PD0: 16B dual PDE (big-page + small-page sub-tables). */
    uint32_t idx0 = (uint32_t)((va >> 21) & 0xFF);            /* [28:21], 8 bits */
    uint64_t lo = nvkvm_fb_rd64(s, tbl + (uint64_t)idx0 * 16);
    uint64_t hi = nvkvm_fb_rd64(s, tbl + (uint64_t)idx0 * 16 + 8);
    uint64_t small_tbl = (((hi >> 8) & ((1ull << 25) - 1)) << 12); /* SMALL bits 96:72, <<12 */
    uint64_t big_tbl   = (((lo >> 4) & ((1ull << 28) - 1)) << 8);  /* BIG  bits 32:4,  <<8  */

    uint64_t pte, page;
    if (small_tbl != 0) {                       /* 4 KiB pages: PT VA[20:12] */
        uint32_t idx = (uint32_t)((va >> 12) & 0x1FF);
        pte  = nvkvm_fb_rd64(s, small_tbl + (uint64_t)idx * 8);
        if (!(pte & 1)) {
            return NVKVM_GMMU_FAULT;            /* PTE VALID bit 0 */
        }
        page = NVKVM_VER2_ADDR_VID(pte);
        return page + (va & 0xFFFull);
    }
    if (big_tbl != 0) {                          /* 64 KiB pages: PT VA[20:16] */
        uint32_t idx = (uint32_t)((va >> 16) & 0x1F);
        pte  = nvkvm_fb_rd64(s, big_tbl + (uint64_t)idx * 8);
        if (!(pte & 1)) {
            return NVKVM_GMMU_FAULT;
        }
        page = NVKVM_VER2_ADDR_VID(pte);
        return page + (va & 0xFFFFull);
    }
    return NVKVM_GMMU_FAULT;
}

/* M5.10 PERF: open-addressing hash set of vidmem PT-page bases (4 KiB-aligned). 0 = empty slot;
 * page base 0 is never a real PT page so the sentinel is safe. */
#define NVKVM_GR_PT_SLOTS 8192
static void nvkvm_m2_gr_pt_record(NvkvmGpuEmul *s, uint64_t addr)
{
    uint64_t base = addr & ~0xfffull;
    if (!base || base == s->m2_gr_pt_last) { return; }   /* sequential entry reads share a page */
    s->m2_gr_pt_last = base;
    if (s->m2_gr_pt_n >= NVKVM_GR_PT_SLOTS * 3 / 4) { return; }  /* near full -> stop (lo/hi still bound) */
    uint32_t h = (uint32_t)((base >> 12) * 2654435761u) & (NVKVM_GR_PT_SLOTS - 1);
    for (int p = 0; p < NVKVM_GR_PT_SLOTS; p++) {
        if (s->m2_gr_pt_set[h] == base) { return; }       /* already present */
        if (s->m2_gr_pt_set[h] == 0) {
            s->m2_gr_pt_set[h] = base; s->m2_gr_pt_n++;
            if (base < s->m2_gr_pt_lo) { s->m2_gr_pt_lo = base; }
            if (base > s->m2_gr_pt_hi) { s->m2_gr_pt_hi = base; }
            return;
        }
        h = (h + 1) & (NVKVM_GR_PT_SLOTS - 1);
    }
}
static bool nvkvm_m2_gr_pt_contains(NvkvmGpuEmul *s, uint64_t addr)
{
    uint64_t base = addr & ~0xfffull;
    uint32_t h = (uint32_t)((base >> 12) * 2654435761u) & (NVKVM_GR_PT_SLOTS - 1);
    for (int p = 0; p < NVKVM_GR_PT_SLOTS; p++) {
        if (s->m2_gr_pt_set[h] == base) { return true; }
        if (s->m2_gr_pt_set[h] == 0) { return false; }
        h = (h + 1) & (NVKVM_GR_PT_SLOTS - 1);
    }
    return false;
}
static void nvkvm_m2_gr_pt_reset(NvkvmGpuEmul *s)
{
    if (s->m2_gr_pt_n) { memset(s->m2_gr_pt_set, 0, sizeof(s->m2_gr_pt_set)); }
    s->m2_gr_pt_n = 0; s->m2_gr_pt_last = 0;
    s->m2_gr_pt_lo = ~0ull; s->m2_gr_pt_hi = 0;
    /* #13: the compute-PT metadata set shares the sweep lifecycle (rebuilt by the same
     * recorded walk) — stale entries for freed/re-pointed tables must not linger, and
     * the pending dirty-index list refers into it so it resets too. */
    if (s->m2_cpt_n) { memset(s->m2_cpt, 0, sizeof(s->m2_cpt)); }
    s->m2_cpt_n = 0; s->m2_cpt_dirty_n = 0;
    s->m2_cpt_lo = ~0ull; s->m2_cpt_hi = 0;
}

/* Read 8 bytes from a page-table entry in FB (vidmem) or sysmem (GPA). */
static uint64_t nvkvm_pt_rd64(NvkvmGpuEmul *s, uint64_t addr, bool sys)
{
    if (sys) {
        uint8_t b[8];
        if (nvkvm_dmar(&s->parent_obj, addr, b, 8) != MEMTX_OK) return 0;
        return ldq_le_p(b);
    }
    if (s->m2_recording_gr_pt) {          /* M5.10 PERF: this vidmem addr is a GR-VAS PT page */
        nvkvm_m2_gr_pt_record(s, addr);
    }
    s->m2_in_walk = true;                /* exclude this PTE read from the CRASHWIN probe */
    uint64_t v = nvkvm_fb_rd64(s, addr);
    s->m2_in_walk = false;
    return v;
}

/* M5 — translate a CHANNEL GPU VA to a physical address, rooted at the channel's
 * own PDB (read from its instance block), and report whether the leaf page is in
 * sysmem (GPA) or vidmem (FB) via *out_sys.  Unlike BAR2, channel buffers (the
 * scrubber's pushbuffer/semaphore) live in SYSMEM, so the leaf PTE APERTURE
 * (NV_MMU_VER2_PTE_APERTURE bits 2:1: 0=VID, 2/3=SYS) selects ADDRESS_VID
 * (bits32:8) vs ADDRESS_SYS (bits53:8), both <<12.  The page-table hierarchy
 * itself is assumed to live in FB (the GSP-client RM allocates the page directory
 * from FB, as for BAR2) — read via the PRAMIN/FB backing.  Returns
 * NVKVM_GMMU_FAULT on any miss (caller then does nothing — safe). */
/* Walk VER2 from an explicit page-directory base `pdb`.  `root_sys` selects the
 * aperture of the page-directory ROOT (false = FB/vidmem, true = sysmem/guest
 * RAM, as for a UVM-managed VAS rooted via SET_PAGE_DIRECTORY); each PDE/PTE
 * aperture bit then selects where the next level lives.  Returns the physical
 * address (and *out_sys = leaf in sysmem) or NVKVM_GMMU_FAULT. */
static uint64_t nvkvm_walk_pdb_root(NvkvmGpuEmul *s, uint64_t pdb, uint64_t va,
                                    bool root_sys, bool *out_sys)
{
    *out_sys = false;
    uint64_t tbl = pdb;
    if (tbl == 0) {
        return NVKVM_GMMU_FAULT;
    }
    /* PD3->PD2->PD1 (8B PDEs), then PD0 (16B dual PDE or 2 MiB PTE); aperture per level. */
    bool tsys = root_sys;  /* root aperture; each PDE aperture says where next lives */
    static const struct { int hi, lo; } lvl[3] = { {48,47}, {46,38}, {37,29} };
    for (int i = 0; i < 3; i++) {
        uint32_t idx = (uint32_t)((va >> lvl[i].lo) &
                                  ((1ull << (lvl[i].hi - lvl[i].lo + 1)) - 1));
        uint64_t pde = nvkvm_pt_rd64(s, tbl + (uint64_t)idx * 8, tsys);
        /* #13 FIX: on GA10x, a PD1 entry can itself be a 512 MiB LEAF PTE
         * (kern_gmmu_fmt_ga10x.c: pLevels[2].bPageTable = NV_TRUE; bit0 =
         * NV_MMU_VER2_PTE_VALID, aperture 2:1 with PTE encoding 0=VID,2/3=SYS).
         * The guest kernel-RM's CeUtils channel (bUseVasForCeCopy,
         * channel_utils.c) identity-maps the WHOLE FB heap into its own VAS at
         * the largest page size — 512 MiB — and then issues its page-table
         * writes as VIRTUAL-dst CE copies (dstAddr = fbPhys + fbAliasVA -
         * startFbOffset = fbPhys).  Without this case the walker decoded the
         * 512M vidmem PTE as PDE_APERTURE_INVALID -> FAULT, chan_execute
         * silently DROPPED every such PT write, the compute VAS's rebuilt
         * subtree never reached our FB shadow, its re-mapped buffers were
         * never backed into the persistent host GR VAS, and the host CE
         * FAULT_PDE'd one page past the last-backed leaf (#13, Xid 31). */
        if ((pde & 1) && i == 2) {                   /* PD1-level 512 MiB leaf PTE */
            uint32_t lap = (uint32_t)((pde >> 1) & 0x3);
            uint64_t pg;
            if (lap == 0) { pg = ((pde >> 8) & ((1ull << 25) - 1)) << 12; *out_sys = false; }
            else if (lap == 2 || lap == 3) { pg = ((pde >> 8) & ((1ull << 46) - 1)) << 12; *out_sys = true; }
            else { return NVKVM_GMMU_FAULT; }        /* PEER unsupported */
            return pg + (va & 0x1FFFFFFFull);
        }
        uint32_t ap = (uint32_t)((pde >> 1) & 0x3);  /* PDE APERTURE: 1=VID,2/3=SYS */
        if (ap == 1) { tbl = ((pde >> 8) & ((1ull << 25) - 1)) << 12; tsys = false; }
        else if (ap == 2 || ap == 3) { tbl = ((pde >> 8) & ((1ull << 46) - 1)) << 12; tsys = true; }
        else { return NVKVM_GMMU_FAULT; }            /* INVALID */
        if (tbl == 0) {
            return NVKVM_GMMU_FAULT;
        }
    }
    /* PD0: 16B dual PDE.  BIG aperture lo bits2:1, SMALL aperture hi bits2:1.
     * SMALL addr VID hi32:8 / SYS hi53:8 (<<12); BIG addr VID lo32:4 / SYS lo53:4 (<<8). */
    uint32_t idx0 = (uint32_t)((va >> 21) & 0xFF);
    uint64_t lo = nvkvm_pt_rd64(s, tbl + (uint64_t)idx0 * 16, tsys);
    uint64_t hi = nvkvm_pt_rd64(s, tbl + (uint64_t)idx0 * 16 + 8, tsys);
    uint32_t big_ap = (uint32_t)((lo >> 1) & 0x3), small_ap = (uint32_t)((hi >> 1) & 0x3);
    /* A PD0 entry with VALID(bit0)=1 is itself a 2 MiB LEAF PTE (NV_MMU_VER2_PTE),
     * not a dual PDE pointing to 4K/64K sub-tables.  APERTURE bits2:1 (0=VID,
     * 2/3=SYS); VA[20:0] is the 2 MiB page offset. */
    if (lo & 1) {
        uint32_t lap = (uint32_t)((lo >> 1) & 0x3);
        uint64_t pg;
        if (lap == 0) { pg = ((lo >> 8) & ((1ull << 25) - 1)) << 12; *out_sys = false; }
        else if (lap == 2 || lap == 3) { pg = ((lo >> 8) & ((1ull << 46) - 1)) << 12; *out_sys = true; }
        else { return NVKVM_GMMU_FAULT; }
        return pg + (va & 0x1FFFFFull);
    }
    uint64_t pte; uint32_t pgshift; bool stsys;
    if (small_ap == 1 || small_ap == 2 || small_ap == 3) {
        stsys = (small_ap != 1);
        uint64_t st = stsys ? (((hi >> 8) & ((1ull << 46) - 1)) << 12)
                            : (((hi >> 8) & ((1ull << 25) - 1)) << 12);
        if (st == 0) { return NVKVM_GMMU_FAULT; }
        pte = nvkvm_pt_rd64(s, st + (uint64_t)((va >> 12) & 0x1FF) * 8, stsys);
        pgshift = 12;
    } else if (big_ap == 1 || big_ap == 2 || big_ap == 3) {
        stsys = (big_ap != 1);
        uint64_t bt = stsys ? (((lo >> 4) & ((1ull << 50) - 1)) << 8)
                            : (((lo >> 4) & ((1ull << 29) - 1)) << 8);
        if (bt == 0) { return NVKVM_GMMU_FAULT; }
        pte = nvkvm_pt_rd64(s, bt + (uint64_t)((va >> 16) & 0x1F) * 8, stsys);
        pgshift = 16;
    } else {
        return NVKVM_GMMU_FAULT;
    }
    if (!(pte & 1)) {                       /* PTE VALID bit0 */
        return NVKVM_GMMU_FAULT;
    }
    uint32_t aperture = (uint32_t)((pte >> 1) & 0x3);  /* PTE APERTURE: 0=VID,2/3=SYS */
    uint64_t page;
    if (aperture == 0) {
        page = ((pte >> 8) & ((1ull << 25) - 1)) << 12;
        *out_sys = false;
    } else if (aperture == 2 || aperture == 3) {
        page = ((pte >> 8) & ((1ull << 46) - 1)) << 12;
        *out_sys = true;
    } else {
        return NVKVM_GMMU_FAULT;
    }
    return page + (va & ((1ull << pgshift) - 1));
}

/* FB-rooted convenience wrapper (the common case: GSP-client RM allocates the
 * page directory from FB).  UVM-managed VASes use nvkvm_walk_pdb_root(...,true). */
static uint64_t nvkvm_walk_pdb(NvkvmGpuEmul *s, uint64_t pdb, uint64_t va,
                               bool *out_sys)
{
    return nvkvm_walk_pdb_root(s, pdb, va, false, out_sys);
}

/* #14: does a snooped VAS root (captured under RM client `vas_client`, VASpace
 * handle `vas_hobj`) BELONG to the executing channel's client `client`?  TRUE when
 * captured directly under that client, or when an observed DUP_OBJECT edge links
 * the two: UVM's per-process gpu-ops client dups the user's VASpace out of the
 * compute client (so the user VAS is captured under UVM's client — the miss that
 * made the client-keyed pass-0 degrade to the blind content-pick and cross two
 * concurrent processes' identical guest VAs, #14).  One hop, either direction —
 * the dup edge is a transport-observed handle-graph fact, not a heuristic. */
static bool nvkvm_m2_vas_client_match(NvkvmGpuEmul *s, uint32_t vas_client,
                                      uint32_t vas_hobj, uint32_t client)
{
    if (!client || !vas_client) {
        return false;
    }
    if (vas_client == client) {
        return true;
    }
    for (int i = 0; i < s->m2_dup_n; i++) {
        /* VAS captured under the dup DST (UVM's dup handle): owner = the src client. */
        if (s->m2_dup[i].dst_client == vas_client && s->m2_dup[i].dst_obj == vas_hobj &&
            s->m2_dup[i].src_client == client) {
            return true;
        }
        /* VAS captured under the SRC side; `client` holds a dup of it. */
        if (s->m2_dup[i].src_client == vas_client && s->m2_dup[i].src_obj == vas_hobj &&
            s->m2_dup[i].dst_client == client) {
            return true;
        }
    }
    return false;
}

/* #14: if (vas_client, vas_hobj) is the DST of an observed dup edge, return the edge's
 * SOURCE client — the compute client that owns the dupped object (UVM dups the user's
 * VASpace out of libcuda's client, so the SET_PAGE_DIRECTORY capture under the dup
 * handle really belongs to that source client).  0 = no edge (not dup-created). */
static uint32_t nvkvm_m2_vas_dup_owner(NvkvmGpuEmul *s, uint32_t vas_client,
                                       uint32_t vas_hobj)
{
    for (int i = 0; i < s->m2_dup_n; i++) {
        if (s->m2_dup[i].dst_client == vas_client && s->m2_dup[i].dst_obj == vas_hobj) {
            return s->m2_dup[i].src_client;
        }
    }
    return 0;
}

/* #14: is `client` a USER COMPUTE client — i.e. did it dup one of its objects (its
 * VASpace/TSG/channel) out to another client (the UVM handover)?  Kernel-internal
 * clients (CeUtils, UVM gpu-ops) appear only on the DST side, never as SRC.  This is
 * the transport-observed definition of "a process's libcuda client"; with one process
 * the set is exactly {m2_gr_client}, so every consumer below is single-process-neutral. */
static bool nvkvm_m2_dup_src_client(NvkvmGpuEmul *s, uint32_t client)
{
    if (!client) { return false; }
    for (int i = 0; i < s->m2_dup_n; i++) {
        if (s->m2_dup[i].src_client == client) { return true; }
    }
    return false;
}

/* #14: is chan_vas[v] PROVABLY another user process's VAS (relative to `client`)?
 * TRUE only on positive transport evidence: (a) it is the DST of a dup edge whose
 * source client is a different user client, or (b) it was captured directly under a
 * different user compute client.  Kernel/UVM-internal roots (no dup linkage) are
 * NEVER foreign — they stay visible to every client exactly as before, so with a
 * single process this predicate is constant-false (no behavior change).  Used to
 * stop the walk-all-roots paths (enum_gr_sysmem / resolve_fb / the blind pick
 * passes) from crossing two processes' IDENTICAL guest VAs. */
static bool nvkvm_m2_vas_foreign(NvkvmGpuEmul *s, int v, uint32_t client)
{
    if (nvkvm_m2_vas_client_match(s, s->chan_vas[v].client, s->chan_vas[v].hvas,
                                  client)) {
        return false;
    }
    uint32_t owner = nvkvm_m2_vas_dup_owner(s, s->chan_vas[v].client,
                                            s->chan_vas[v].hvas);
    if (owner && owner != client) {
        return true;            /* (a) dup handle of another user's object */
    }
    if (s->chan_vas[v].client != client &&
        nvkvm_m2_dup_src_client(s, s->chan_vas[v].client)) {
        return true;            /* (b) captured under another user compute client */
    }
    return false;
}

/* #14: is `client` one of the user GR compute clients (a 0xc7c0 allocator)?  With a
 * single process the list is exactly {m2_gr_client}, so every consumer is single-
 * process-neutral. */
static bool nvkvm_m2_is_gr_client(NvkvmGpuEmul *s, uint32_t client)
{
    if (!client) { return false; }
    for (int i = 0; i < s->m2_gr_clients_n; i++) {
        if (s->m2_gr_clients[i] == client) { return true; }
    }
    return false;
}

/* #14 EARLY-ARM: is `client` a user compute client by EITHER signal — the early
 * dup-src registration (available from cuCtxCreate's UVM-registration step) or
 * the late 0xc7c0 registration.  Used by the multiproc()-gated divergences
 * (pass-1 refusal, per-owner backing) so a 2nd process's channels are separated
 * from their FIRST execution, before its 0xc7c0 exists.  Single process:
 * multiproc() is false, so every consumer is byte-identical. */
static bool nvkvm_m2_is_user_client(NvkvmGpuEmul *s, uint32_t client)
{
    if (!client) { return false; }
    for (int i = 0; i < s->m2_user_clients_n; i++) {
        if (s->m2_user_clients[i] == client) { return true; }
    }
    return nvkvm_m2_is_gr_client(s, client);
}

/* #14: is more than one user GR compute client live (two concurrent guest processes
 * with compute)?  The per-process VAS-separation divergences below (blind-pass refusal,
 * foreign-VAS skip) activate ONLY here — with a single compute client (incl. #12's
 * sequential 2-context single process, which REUSES one RM client) they are disabled so
 * behavior is byte-identical to the single-process baseline (task single-process-safety
 * mandate).  Ownership keying that is inherently identical for one client (chan/tsg
 * client-keys, per-owner backing) stays always-on. */
static bool nvkvm_m2_multiproc(NvkvmGpuEmul *s)
{
    /* #14 EARLY-ARM: the dup-src user-client list arms this at the 2nd process's
     * cuCtxCreate UVM-registration (fn=21), BEFORE any of its channels/mappings
     * exist; the 0xc7c0 list is kept as a belt-and-braces late signal. */
    return s->m2_gr_clients_n > 1 || s->m2_user_clients_n > 1;
}

/* ── #14 P1: the per-process registry (plan §3) ─────────────────────────────────
 * A process is anchored by its user COMPUTE client (the dup SRC — kernel/UVM clients
 * only ever appear as dup DST, bench-verified §1.3).  A process may accrue several
 * clients (its UVM gpu-ops dup-dst client, CE-copy clients) and several PDBs (a
 * process holds several VASes).  P1 builds+logs this; nothing keys on it yet, so
 * single-process behavior is byte-identical (one anchor client → one proc).  The
 * SYSTEM class (kernel/GSP/scrubber) has NO proc — it is the implicit remainder. */
static int nvkvm_m2_proc_find_by_client(NvkvmGpuEmul *s, uint32_t client)
{
    if (!client) { return -1; }
    for (int i = 0; i < s->m2_proc_n; i++) {
        if (!s->m2_proc[i].live) { continue; }
        for (int c = 0; c < s->m2_proc[i].clients_n; c++) {
            if (s->m2_proc[i].clients[c] == client) { return i; }
        }
    }
    return -1;
}

/* Get-or-create the proc anchored by user compute client `anchor`. */
static int nvkvm_m2_proc_get(NvkvmGpuEmul *s, uint32_t anchor)
{
    int pi = nvkvm_m2_proc_find_by_client(s, anchor);
    if (pi >= 0) { return pi; }
    if (s->m2_proc_n >= NVKVM_MAX_PROCS) { return -1; }
    pi = s->m2_proc_n++;
    struct nvkvm_proc *p = &s->m2_proc[pi];
    memset(p, 0, sizeof(*p));
    p->live = true;
    p->clients[p->clients_n++] = anchor;
    qemu_log("nvkvm-gpu[%s] #14 P1 PROC[%d] created anchor-client=0x%08x (proc_n=%d)\n",
             s->chip->name, pi, anchor, s->m2_proc_n);
    return pi;
}

/* Associate an additional client (dup-dst UVM/CE) with an existing proc. */
static void nvkvm_m2_proc_add_client(NvkvmGpuEmul *s, int pi, uint32_t client)
{
    if (pi < 0 || pi >= s->m2_proc_n || !client) { return; }
    struct nvkvm_proc *p = &s->m2_proc[pi];
    for (int c = 0; c < p->clients_n; c++) {
        if (p->clients[c] == client) { return; }
    }
    if (p->clients_n >= NVKVM_PROC_MAX_CLIENTS) { return; }
    p->clients[p->clients_n++] = client;
    qemu_log("nvkvm-gpu[%s] #14 P1 PROC[%d] += client=0x%08x (clients_n=%d)\n",
             s->chip->name, pi, client, p->clients_n);
}

/* Associate a PDB with the proc owning `owner_client` (its GR/UVM VAS root). */
static void nvkvm_m2_proc_add_pdb(NvkvmGpuEmul *s, uint32_t owner_client, uint64_t pdb)
{
    if (!pdb) { return; }
    int pi = nvkvm_m2_proc_find_by_client(s, owner_client);
    if (pi < 0) { return; }
    struct nvkvm_proc *p = &s->m2_proc[pi];
    for (int i = 0; i < p->pdbs_n; i++) {
        if (p->pdbs[i] == pdb) { return; }
    }
    if (p->pdbs_n >= NVKVM_PROC_MAX_VAS) { return; }
    p->pdbs[p->pdbs_n++] = pdb;
    qemu_log("nvkvm-gpu[%s] #14 P1 PROC[%d] += PDB=0x%llx (via client=0x%08x, pdbs_n=%d)\n",
             s->chip->name, pi, (unsigned long long)pdb, owner_client, p->pdbs_n);
}

/* Drop a client (and, on the anchor, the whole proc) at root-free. */
static void nvkvm_m2_proc_drop_client(NvkvmGpuEmul *s, uint32_t client)
{
    int pi = nvkvm_m2_proc_find_by_client(s, client);
    if (pi < 0) { return; }
    struct nvkvm_proc *p = &s->m2_proc[pi];
    if (p->clients[0] == client) {
        qemu_log("nvkvm-gpu[%s] #14 P1 PROC[%d] reaped (anchor-client=0x%08x freed)\n",
                 s->chip->name, pi, client);
        p->live = false; p->clients_n = 0; p->pdbs_n = 0;
        return;
    }
    for (int c = 0; c < p->clients_n; c++) {
        if (p->clients[c] == client) {
            p->clients[c] = p->clients[p->clients_n - 1];
            p->clients_n--; break;
        }
    }
}

/* #14: (client, tsg)-keyed GPFIFO_SCHEDULE tracking — see the m2_tsg_sched struct
 * comment (two processes reuse identical TSG handle VALUES, so a value-keyed scalar
 * aliased them). */
static bool nvkvm_m2_tsg_sched_check(NvkvmGpuEmul *s, uint32_t client, uint32_t tsg)
{
    for (int i = 0; i < s->m2_tsg_sched_n; i++) {
        if (s->m2_tsg_sched[i].client == client && s->m2_tsg_sched[i].tsg == tsg) {
            return true;
        }
    }
    return false;
}
static void nvkvm_m2_tsg_sched_mark(NvkvmGpuEmul *s, uint32_t client, uint32_t tsg)
{
    if (nvkvm_m2_tsg_sched_check(s, client, tsg)) { return; }
    if (s->m2_tsg_sched_n < (int)ARRAY_SIZE(s->m2_tsg_sched)) {
        s->m2_tsg_sched[s->m2_tsg_sched_n].client = client;
        s->m2_tsg_sched[s->m2_tsg_sched_n].tsg    = tsg;
        s->m2_tsg_sched_n++;
    }
}

/* #14: the user GR compute client that OWNS the VAS rooted at `pdb` — direct capture
 * or via a DUP_OBJECT edge (the UVM-managed user VAS is captured under UVM's client;
 * the dup edge names the true owner).  0 = no user-compute owner derivable (kernel/
 * CeUtils roots).  Single process: the only derivable owner IS m2_gr_client. */
static uint32_t nvkvm_m2_pdb_gr_owner(NvkvmGpuEmul *s, uint64_t pdb)
{
    for (int i = 0; i < s->chan_vas_n; i++) {
        if (!pdb || s->chan_vas[i].pdb != pdb) { continue; }
        uint32_t cl = s->chan_vas[i].client;
        if (nvkvm_m2_is_gr_client(s, cl)) { return cl; }
        uint32_t own = nvkvm_m2_vas_dup_owner(s, cl, s->chan_vas[i].hvas);
        if (own && nvkvm_m2_is_gr_client(s, own)) { return own; }
        /* #14 EARLY-ARM (multiproc only, single-process byte-identical): a 2nd
         * process's VAS is owned by its dup-src user client BEFORE that client
         * allocs 0xc7c0 — without this, the caller's `own = m2_gr_client`
         * fallback would back the 2nd process's pages under the FIRST process's
         * host VAS during the window (cross-process aliasing). */
        if (nvkvm_m2_multiproc(s)) {
            if (nvkvm_m2_is_user_client(s, cl)) { return cl; }
            if (own && nvkvm_m2_is_user_client(s, own)) { return own; }
        }
    }
    return 0;
}

/* M5.21: the executing channel's OWN VAS PDB, derived from its client's GR
 * VASpace (chan_client -> m2_devvas vas handle -> chan_vas pdb).  This is the
 * AUTHORITATIVE address space for the channel's pushbuffer/sema — unlike the
 * content-pick heuristic below, which scans ALL snooped VASes and can land on a
 * FOREIGN client's VAS that merely aliases the same guest VA (the confirmed
 * wrong-channel bug: the compute channel's working set resolved through the
 * probe client 0xc1d0000a's VAS 0x2efa4c000 instead of its own 0x3114000).
 * Returns 0 if the client's VAS or its PDB isn't known yet (caller falls back). */
static uint64_t nvkvm_chan_own_pdb_rs(NvkvmGpuEmul *s, bool *out_root_sys)
{
    if (out_root_sys) {
        *out_root_sys = false;       /* default: FB-rooted (GSP-client common case) */
    }
    if (!s->chan_client) {
        return 0;
    }
    /* (a) Existing: client's device VASpace (m2_devvas[client] -> vas -> chan_vas pdb).
     * These are GSP-client device VASes -> FB-rooted (root_sys stays false). */
    uint32_t hvas = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == s->chan_client) {
            hvas = s->m2_devvas[i].vas;
            break;
        }
    }
    if (hvas) {
        for (int i = 0; i < s->chan_vas_n; i++) {
            if (s->chan_vas[i].hvas == hvas) {
                if (out_root_sys) { *out_root_sys = s->chan_vas[i].root_sys; }
                return s->chan_vas[i].pdb;
            }
        }
    }
    /* M5.32 (Step 1): the GR compute channel commonly uses a context-share VAS NOT
     * allocated under its own client (so m2_devvas[client] misses).  Fall back to the
     * channel's OWN hVASpace handle directly against chan_vas[] — which M5.30 now
     * populates from SET_PAGE_DIRECTORY (0x801813) as well as RESERVED_PDES.  This is
     * the same authoritative key nvkvm_chan_translate prefers; chan_own_pdb just never
     * tried it.  Then fall back to the instblk PDB (RAMIN+0x200) if fake-GSP has
     * written it (chan_pdb).  This is what un-bails populate_cvas for the GR channel. */
    if (s->chan_hvaspace) {
        for (int i = 0; i < s->chan_vas_n; i++) {
            if (s->chan_vas[i].hvas == s->chan_hvaspace && s->chan_vas[i].pdb) {
                if (out_root_sys) { *out_root_sys = s->chan_vas[i].root_sys; }
                return s->chan_vas[i].pdb;
            }
        }
    }
    if (s->chan_pdb) {
        return s->chan_pdb;             /* authoritative instblk root (RAMIN+0x200), FB */
    }
    /* (d) M5.36: PROBE every captured root against the channel's OWN GPFIFO VA — the one
     * VA guaranteed mapped in the channel's VAS. Handle-keyed lookups (a)/(b) MISS when the
     * channel's true root was captured only under UVM's dup handle (e.g. 0xcaf00005) and is
     * unlinkable to the channel's RM client (libcuda's compute channels: chan_hvas=0,
     * gpfifo in the UVM-managed 0x200200000 region). Content-validate instead: the first
     * captured root whose walk resolves the gpfifo VA IS the channel's VAS. Returns its
     * root_sys so populate_cvas enumerates with the correct root aperture (UVM roots are
     * sys-rooted; the hardcoded false mis-walked them even when the PDB was found). */
    if (s->chan_gpfifo_va) {
        /* #12-L3c: TWO PASSES.  Pass 0 prefers a root whose snooped owning CLIENT
         * matches the executing channel's client (chan_vas[].client, the L3a key) —
         * the content-probe alone is NOT client-keyed, so when DISTINCT clients put
         * their gpfifo at VAs that both validate under a shared/foreign root it
         * collapses their per-client VASes onto one (the CeUtils 0xc1d00001 vs UVM
         * 0xc1e00007 completion-sema-at-va-0x121000010 collision → uvm_gpu_semaphore.c
         * :776 backward jump → 2nd-context hang).  Pass 1 = the original blind probe,
         * for channels whose true root was captured only under a foreign dup handle
         * (e.g. UVM's 0xcaf00005, client-unlinkable) — unchanged behavior there. */
        for (int pass = 0; pass < 2; pass++) {
            /* #14: a USER compute client's channel resolves ONLY in a VAS it owns
             * (direct or dup-linked) — miss means its PTs aren't populated yet, so
             * DEFER (return 0; populate_cvas/chan_execute retry next doorbell) per
             * the address-table rule (miss = fault, never heuristic-resolve).  The
             * blind pass-1 probe validates by WALK SUCCESS alone, and the kernel
             * gpu-ops whole-FB alias (12 GiB identity root) resolves ANY VA — with
             * two concurrent processes, process B's first-doorbell probe raced its
             * own PT population, pass-1 "hit" that alias, and populate_cvas latched
             * B's fvas populated with the aliased garbage space (B hung forever).
             * Kernel/CeUtils channels (no user-client linkage) keep pass 1.
             * EARLY-ARM: keyed on is_user_client (dup-src, known from process
             * start) so the 2nd process's channels are refused the blind pass
             * from their FIRST execution — before its 0xc7c0 exists. */
            if (pass == 1 && nvkvm_m2_multiproc(s) &&
                nvkvm_m2_is_user_client(s, s->chan_client)) {
                break;
            }
            for (int i = 0; i < s->chan_vas_n; i++) {
                if (!s->chan_vas[i].pdb) {
                    continue;
                }
                /* #14: pass-0 ownership is dup-edge aware — the user working-set
                 * VASes are captured under UVM's per-process gpu-ops client, so the
                 * plain client==chan_client test always missed them and degraded to
                 * the blind pass-1 probe, which crossed two concurrent processes'
                 * identical guest VAs (B's channel picked A's PDB -> B faulted). */
                if (pass == 0 &&
                    !nvkvm_m2_vas_client_match(s, s->chan_vas[i].client,
                                               s->chan_vas[i].hvas, s->chan_client)) {
                    continue;   /* pass 0: own-client (direct or dup-linked) roots only */
                }
                bool sy = false;
                if (nvkvm_walk_pdb_root(s, s->chan_vas[i].pdb, s->chan_gpfifo_va,
                                        s->chan_vas[i].root_sys, &sy) != NVKVM_GMMU_FAULT) {
                    if (out_root_sys) { *out_root_sys = s->chan_vas[i].root_sys; }
                    if (s->m2_own_pdb_diag < 8) {
                        qemu_log("nvkvm-gpu[%s] M5.36 own_pdb PROBE hit (pass%d): client=0x%08x "
                                 "vas_client=0x%08x hvas=0x%08x pdb=0x%llx root_sys=%d maps gpfifo=0x%llx\n",
                                 s->chip->name, pass, s->chan_client, s->chan_vas[i].client,
                                 s->chan_vas[i].hvas, (unsigned long long)s->chan_vas[i].pdb,
                                 s->chan_vas[i].root_sys,
                                 (unsigned long long)s->chan_gpfifo_va);
                    }
                    return s->chan_vas[i].pdb;
                }
            }
        }
    }
    /* Step-1 DIAG: dump what's available so we can see WHY no root resolved
     * (bounded to avoid spam). */
    if (s->m2_own_pdb_diag++ < 8) {
        qemu_log("nvkvm-gpu[%s] M5.32 own_pdb MISS: chan_client=0x%08x chan_hvas=0x%08x "
                 "chan_pdb=0x%llx devvas_n=%d chan_vas_n=%d\n", s->chip->name,
                 s->chan_client, s->chan_hvaspace, (unsigned long long)s->chan_pdb,
                 s->m2_devvas_n, s->chan_vas_n);
        for (int i = 0; i < s->m2_devvas_n; i++) {
            qemu_log("nvkvm-gpu[%s] M5.32   devvas[%d] client=0x%08x dev=0x%08x vas=0x%08x\n",
                     s->chip->name, i, s->m2_devvas[i].client, s->m2_devvas[i].dev,
                     s->m2_devvas[i].vas);
        }
        for (int i = 0; i < s->chan_vas_n; i++) {
            qemu_log("nvkvm-gpu[%s] M5.32   chan_vas[%d] hvas=0x%08x pdb=0x%llx root_sys=%d\n",
                     s->chip->name, i, s->chan_vas[i].hvas,
                     (unsigned long long)s->chan_vas[i].pdb, s->chan_vas[i].root_sys);
        }
    }
    return 0;
}

/* Back-compat wrapper: callers that don't need the root aperture. */
static uint64_t nvkvm_chan_own_pdb(NvkvmGpuEmul *s)
{
    return nvkvm_chan_own_pdb_rs(s, NULL);
}

/* Translate a channel GPU VA by trying every snooped VAS PDB (from
 * VASPACE_COPY_SERVER_RESERVED_PDES) and returning the first that resolves.  The
 * channel's pushbuffer/sema live in its own VAS, but the scrubber's vid/sys test
 * surfaces may be in a different VAS, so match by which PD actually maps the VA
 * (a valid leaf PTE) rather than by the channel's hVASpace handle. */
static uint64_t nvkvm_chan_translate(NvkvmGpuEmul *s, uint64_t va, bool *out_sys)
{
    /* #2 side-table (PROMOTE_CTX) — authoritative and required for GSP-managed
     * VASes whose leaf PTEs never land in our FB (so the PDB walk below FAULTs).
     * Scoped to the executing channel's RM client so VAs can't collide across
     * processes.  Longest-prefix not needed: PROMOTE_CTX ranges are disjoint. */
    for (int i = 0; i < s->va_map_n; i++) {
        struct nvkvm_va_map *m = &s->va_map[i];
        if (m->client == s->chan_client && va >= m->va && va < m->va + m->size) {
            *out_sys = m->sys;
            return m->phys + (va - m->va);
        }
    }
    /* Authoritative: the executing channel's own PDB (from its instance block).
     * Correct even for hVASpace=0 (device-default) channels that don't match any
     * snooped VAS handle.  0 = instblk empty -> fall through to the heuristic. */
    if (s->chan_pdb) {
        uint64_t p = nvkvm_walk_pdb(s, s->chan_pdb, va, out_sys);
        if (p != NVKVM_GMMU_FAULT) { return p; }
    }
    /* Prefer the channel's own hVASpace first (fast path / disambiguation). */
    for (int i = 0; i < s->chan_vas_n; i++) {
        if (s->chan_vas[i].hvas == s->chan_hvaspace) {
            uint64_t p = nvkvm_walk_pdb_root(s, s->chan_vas[i].pdb, va,
                                             s->chan_vas[i].root_sys, out_sys);
            if (p != NVKVM_GMMU_FAULT) { return p; }
            break;
        }
    }
    /* #12-L3: prefer a VAS owned by the EXECUTING channel's RM client before the
     * blind any-client fallback below.  Host VASpaces are per-client, so VA X in
     * client A's VAS and VA X in client B's VAS are DIFFERENT physical pages.  The
     * blind fallback returns the first snooped VAS that resolves X regardless of
     * owner — which collapsed two distinct kernel semaphores (CeUtils' completion
     * sema and a UVM channel's tracking sema, both at VA 0x121000010 in their own
     * VASes) onto ONE phys page: CeUtils' release climbed the UVM page to 0x8a, then
     * UVM's own low release looked like a 2^32 backward jump (uvm_gpu_semaphore.c:776
     * + ce_utils.c:349) → the 2nd-context matmul hang.  Same-client-first stops the
     * cross-client collision; the blind pass stays as last resort so every VA that
     * legitimately resolves only under another client's VAS (e.g. a scrubber reading
     * a shared surface, or a channel with no snooped own VAS) behaves exactly as
     * before. */
    if (s->chan_client) {
        for (int i = 0; i < s->chan_vas_n; i++) {
            /* #14: ownership is dup-edge aware (user VASes are captured under UVM's
             * per-process gpu-ops client), so concurrent processes' identical VAs
             * resolve under their OWN process's VAS here, not the blind pass below. */
            if (!nvkvm_m2_vas_client_match(s, s->chan_vas[i].client,
                                           s->chan_vas[i].hvas, s->chan_client)) {
                continue;
            }
            uint64_t p = nvkvm_walk_pdb_root(s, s->chan_vas[i].pdb, va,
                                             s->chan_vas[i].root_sys, out_sys);
            if (p != NVKVM_GMMU_FAULT) { return p; }
        }
    }
    for (int i = 0; i < s->chan_vas_n; i++) {
        uint64_t p = nvkvm_walk_pdb_root(s, s->chan_vas[i].pdb, va,
                                         s->chan_vas[i].root_sys, out_sys);
        if (p != NVKVM_GMMU_FAULT) { return p; }
    }
    *out_sys = false;
    return NVKVM_GMMU_FAULT;
}

/* M5 — read/write a 32-bit word at a PHYSICAL address in either aperture. */
static uint32_t nvkvm_phys_rd32(NvkvmGpuEmul *s, uint64_t phys, bool sys)
{
    if (sys) {
        uint8_t b[4];
        if (nvkvm_dmar(&s->parent_obj, phys, b, 4) != MEMTX_OK) return 0;
        return ldl_le_p(b);
    }
    return (uint32_t)nvkvm_fb_read(s, phys, 4);
}
static void nvkvm_phys_wr32(NvkvmGpuEmul *s, uint64_t phys, bool sys, uint32_t v)
{
    if (sys) {
        uint8_t b[4]; stl_le_p(b, v);
        nvkvm_dmaw(&s->parent_obj, phys, b, 4);
    } else {
        nvkvm_fb_write(s, phys, v, 4);
    }
}

/* M5.12 PERF: raw host-CPU backing pointer for an FB (vidmem) address, valid for at least the
 * rest of its 4 KiB page (overlay objects and fb_pages are both page-contiguous). Mirrors the
 * fb_read/fb_write overlay-then-fb_page resolution but returns the pointer so a CE copy can
 * memcpy a whole page-span instead of paying the per-4-byte translate + O(n) overlay-scan that
 * dominated ce_emul (m569: 42% of generation). for_write allocates a backing fb_page on miss
 * (so a dst page exists); read can return NULL (sparse-zero) -> caller treats as zeros. The
 * caller must clamp the span to the page boundary. Returns NULL only on a read miss. */
static uint8_t *nvkvm_fb_host_ptr(NvkvmGpuEmul *s, uint64_t fb_addr, bool for_write)
{
    uint8_t *hp = ((s->m2_fbback_n || s->m2_gpga_n) ? nvkvm_fb_host_overlay(s, fb_addr) : NULL);
    if (hp) { return hp; }
    return nvkvm_fb_page(s, fb_addr, for_write);   /* write: alloc dst page; read: NULL => sparse zero */
}

/* M5.18 — write a completion-semaphore payload at a CHANNEL GPU VA, redirected to
 * the location the guest CPU actually READS.  For a GSP-managed vidmem channel the
 * channel-VAS walk gives a stale aliasing FB page (libcuda never reads it); the
 * guest accesses the channel buffer through BAR1, which the earlier trace showed is
 * FB-contiguous from chan_gpfifo_phys (the page where the guest wrote the GP entry:
 * GPFIFO BAR1 0xa0000->FB 0x3130000, USERD 0xb0000->0x3140000).  So when the sema
 * VA lies inside the channel's GPFIFO buffer window, ALSO write it at
 * chan_gpfifo_phys + (va - chan_gpfifo_va) — the page libcuda polls.  We write both
 * the channel-VAS page (harmless, may be the real one for sysmem semas) AND the
 * BAR1-relative page, so sysmem semas (other channels) are unaffected. */
#define NVKVM_CHAN_BUF_WINDOW 0x20000ull   /* GPFIFO + USERD + sema slack (128 KiB) */
static bool nvkvm_chan_sem_wr32(NvkvmGpuEmul *s, uint64_t va, uint32_t payload,
                                uint64_t *out_redir)
{
    bool wrote = false;
    /* M5.49b: HOST-ONLY for the user CE-copy clients — do NOT write the sema in
     * software; instead fwd-map it so the REAL host GPU writes the completion.  The
     * fwd-map MUST happen on this same call path (it lives here), so the parser sites
     * always call us; we decide here whether to write locally or defer to the host. */
    bool hostonly = s->m2exec && s->m2hostsem && nvkvm_m2_is_user_ce(s, s->chan_client);
    /* #12-L3b: resolve the completion-sema VA under the channel's OWN content-validated
     * VAS when chan_execute could NOT pin chan_pdb (the GSP-managed-channel case: the
     * ring lives in vidmem read via BAR1, so every VAS-walk of the ring entry reads 0 and
     * the value-gated pin is skipped → chan_pdb==0).  Without this the sema VA falls to
     * nvkvm_chan_translate's blind last-resort fallback, which resolves it under whatever
     * snooped VAS maps it first — collapsing DISTINCT channels' completion semaphores onto
     * one phys page (a live UVM channel then reads a backward payload → uvm_gpu_semaphore.c
     * :776 / ce_utils.c:349 → the 2nd-context hang).  nvkvm_chan_own_pdb_rs is content-
     * validated (returns a PDB only if it maps THIS channel's gpfifo VA), so it is the
     * channel's real VAS even when the ring reads 0.  CONFINED to the sema write: the
     * pushbuffer/gpfifo translation is untouched (pinning own globally into chan_pdb
     * regressed single-context init). */
    bool sy; uint64_t p = NVKVM_GMMU_FAULT;
    uint64_t dbg_own = 0; const char *dbg_res = "translate";   /* #12-L3c DIAG */
    /* #12-L3c: for a KERNEL completion sema (NOT the user-CE hostonly path — that
     * resolves correctly via chan_pdb and is cup8's hot path, leave it untouched),
     * resolve under the WRITING client's OWN VAS from the sticky table FIRST.  This
     * overrides the stale global chan_pdb a foreign channel left behind, which is the
     * CeUtils(0xc1d00001)<->UVM(0xc1e00007) sema-at-va-0x121000010 collapse: the UVM
     * CE channel is GSP-managed (empty instblk, hvas=0) and its VAS handle was freed,
     * so without this its sema resolves under CeUtils' 0x3114000 → one phys → backward
     * jump (uvm_gpu_semaphore.c:776) → 2nd-context hang.  Client-keyed ⇒ each client's
     * sema lands in its own address space. */
    if (!hostonly && s->chan_client) {
        for (int i = 0; i < s->m2_cli_vas_n; i++) {
            if (s->m2_cli_vas[i].client != s->chan_client) { continue; }
            uint64_t cp = nvkvm_walk_pdb_root(s, s->m2_cli_vas[i].pdb, va,
                                              s->m2_cli_vas[i].root_sys, &sy);
            if (cp != NVKVM_GMMU_FAULT) { p = cp; dbg_res = "cli_vas"; break; }
        }
    }
    {
        bool own_rs = false;
        dbg_own = nvkvm_chan_own_pdb_rs(s, &own_rs);            /* probe for DIAG always */
        /* #12 cont.33 (2026-07-05): resolve the completion-sema VA under the VAS that
         * is CONTENT-VALIDATED to map THIS channel's gpfifo VA — NOT chan_translate's
         * blind foreign-VAS fallback, and NOT nvkvm_chan_own_pdb_rs() (which short-
         * circuits to the global s->chan_pdb before it reaches its own gpfifo probe).
         *
         * The bug (trace-proven): at the CeUtils scrubber's sema write (client
         * 0xc1e00007, sema VA 0x121000010, gpfifo VA 0x120064000) the global chan_pdb
         * held a FOREIGN root (0x3114000, client 0xc1d00001).  Both chan_translate and
         * own_pdb's chan_pdb short-circuit therefore walked 0x121000010 under that
         * foreign VAS and landed on phys ..3482a010 = UVM's PERSISTENT per-CE-channel
         * tracking-sema page (guest CPU VA 0xffff..3482a010).  CeUtils' low completion
         * payload (0x1b..) then overwrote UVM's live tracking value (0x72) → a 32-bit
         * BACKWARD jump → uvm_gpu_semaphore.c:776 UVM_ASSERT_MSG_RELEASE MAX_JUMP → UVM
         * global fatal → CTX2's CE channels aborted before they ran → 2nd cuCtxCreate
         * hangs (#12).  The channel's OWN VAS (pdb 0x2efa6c000, which the gpfifo probe
         * validates because it maps gpfifo 0x120064000) resolves 0x121000010 → phys
         * ..1000010 = CeUtils' REAL, DISTINCT sema page — no collapse.
         *
         * So: when cli_vas missed, directly re-run the gpfifo content probe (pass 0 =
         * same snooped client first, pass 1 = blind) to find the root that maps this
         * channel's gpfifo, and walk the sema VA under it.  Confined to the sema write;
         * the gpfifo/pushbuffer translation and the global chan_pdb are untouched
         * (pinning a probed root globally regressed single-context init — L3b note). */
        if (p == NVKVM_GMMU_FAULT && s->chan_gpfifo_va) {
            for (int pass = 0; pass < 2 && p == NVKVM_GMMU_FAULT; pass++) {
                /* #14: user compute clients never take the blind pass (see
                 * nvkvm_chan_own_pdb_rs) — a sema WRITE through the kernel
                 * whole-FB alias would corrupt unrelated FB.  EARLY-ARM: keyed
                 * on is_user_client (dup-src, known from process start). */
                if (pass == 1 && nvkvm_m2_multiproc(s) &&
                    nvkvm_m2_is_user_client(s, s->chan_client)) {
                    break;
                }
                for (int i = 0; i < s->chan_vas_n; i++) {
                    if (!s->chan_vas[i].pdb) { continue; }
                    if (pass == 0 &&
                        !nvkvm_m2_vas_client_match(s, s->chan_vas[i].client,
                                                   s->chan_vas[i].hvas,
                                                   s->chan_client)) {
                        continue;   /* pass 0: own-client (direct or dup-linked) only (#14) */
                    }
                    bool gsy = false;
                    if (nvkvm_walk_pdb_root(s, s->chan_vas[i].pdb, s->chan_gpfifo_va,
                                            s->chan_vas[i].root_sys, &gsy)
                            == NVKVM_GMMU_FAULT) {
                        continue;   /* this root does not map our gpfifo */
                    }
                    /* content-validated: this root IS the channel's VAS */
                    bool ssy = false;
                    uint64_t sp = nvkvm_walk_pdb_root(s, s->chan_vas[i].pdb, va,
                                                      s->chan_vas[i].root_sys, &ssy);
                    if (sp != NVKVM_GMMU_FAULT) {
                        p = sp; sy = ssy; dbg_res = "gpfifo-own"; break;
                    }
                }
            }
        }
        if (p == NVKVM_GMMU_FAULT && s->chan_pdb == 0 && dbg_own) {
            p = nvkvm_walk_pdb_root(s, dbg_own, va, own_rs, &sy);
            if (p != NVKVM_GMMU_FAULT) { dbg_res = "own"; }
        }
    }
    if (p == NVKVM_GMMU_FAULT) { p = nvkvm_chan_translate(s, va, &sy); }
    if (p != NVKVM_GMMU_FAULT && !hostonly) {
        /* #12-L3 DIAG: a CE completion-sema write whose payload goes BACKWARDS vs
         * the value already at that phys page is the exact event that trips UVM's
         * 32->64-bit wrap detector (uvm_gpu_semaphore.c:776) across a ctx teardown/
         * re-init.  Log it with full attribution (phys, old, new, writing client) so
         * one bench run distinguishes (a) a single channel rewound by re-init from
         * (b) two distinct channels' semas ALIASING one phys page.  Rare event ->
         * unconditional, low-volume. */
        uint32_t sem_old = nvkvm_phys_rd32(s, p, sy);
        if (sem_old != 0 && payload < sem_old) {
            qemu_log("nvkvm-gpu[%s] #12-L3 CE-SEM BACKWARD va=0x%llx phys=0x%llx(%s) "
                     "old=0x%x new=0x%x client=0x%08x chan_gpfifo_va=0x%llx\n",
                     s->chip->name, (unsigned long long)va, (unsigned long long)p,
                     sy ? "sys" : "fb", sem_old, payload, s->chan_client,
                     (unsigned long long)s->chan_gpfifo_va);
            /* #12-L3c PROBE: on the collision, resolve va under EVERY captured VAS
             * (chan_vas[] live + m2_cli_vas[] sticky) to find whether UVM's OWN sema
             * page exists in our captures at all (distinct phys) or is entirely
             * uncaptured (→ must snoop the sema-pool alloc).  Bounded: backward is rare. */
            for (int i = 0; i < s->chan_vas_n; i++) {
                bool fs = false;
                uint64_t fp = s->chan_vas[i].pdb ?
                    nvkvm_walk_pdb_root(s, s->chan_vas[i].pdb, va, s->chan_vas[i].root_sys, &fs)
                    : NVKVM_GMMU_FAULT;
                qemu_log("nvkvm-gpu[%s] #12-L3c PROBE chan_vas[%d] client=0x%08x pdb=0x%llx "
                         "rs=%d -> %s0x%llx\n", s->chip->name, i, s->chan_vas[i].client,
                         (unsigned long long)s->chan_vas[i].pdb, s->chan_vas[i].root_sys,
                         fp == NVKVM_GMMU_FAULT ? "FAULT " : "", (unsigned long long)fp);
            }
            for (int i = 0; i < s->m2_cli_vas_n; i++) {
                bool fs = false;
                uint64_t fp = nvkvm_walk_pdb_root(s, s->m2_cli_vas[i].pdb, va,
                                                  s->m2_cli_vas[i].root_sys, &fs);
                qemu_log("nvkvm-gpu[%s] #12-L3c PROBE cli_vas[%d] client=0x%08x pdb=0x%llx "
                         "rs=%d -> %s0x%llx\n", s->chip->name, i, s->m2_cli_vas[i].client,
                         (unsigned long long)s->m2_cli_vas[i].pdb, s->m2_cli_vas[i].root_sys,
                         fp == NVKVM_GMMU_FAULT ? "FAULT " : "", (unsigned long long)fp);
            }
        }
        /* #12-L3c DIAG: full sema-write timeline (M2TRACE). Pairs every writer's
         * (client, gpfifo_va) with the resolved (va, phys) it lands on, so one run
         * shows whether the 0x8a-climber and the payload-9 writer have DISTINCT VAs
         * collapsed to one phys (translation bug → fix VAS disambiguation) or the
         * SAME VA under the shared UVM VAS (page-reuse → fix free/realloc lifecycle). */
        if (s->trace) {
            qemu_log("nvkvm-gpu[%s] #12-L3c SEMW va=0x%llx phys=0x%llx(%s) old=0x%x new=0x%x "
                     "client=0x%08x gpfifo_va=0x%llx res=%s chan_pdb=0x%llx own_pdb=0x%llx\n",
                     s->chip->name, (unsigned long long)va, (unsigned long long)p,
                     sy ? "sys" : "fb", sem_old, payload, s->chan_client,
                     (unsigned long long)s->chan_gpfifo_va, dbg_res,
                     (unsigned long long)s->chan_pdb, (unsigned long long)dbg_own);
        }
        /* #12-L3c FIX: on a backdoor-owned sema slot (0xFFF508 — the patched guest
         * UVM reports its kernel tracking-sema GPA+payload at CE-PUSH/submit time),
         * suppress ONLY the parser writes that go BACKWARD.  Rationale:
         *  - The backdoor reports the SUBMITTED payload; the parser's CE_SEM_RELEASE
         *    reports COMPLETION.  In steady state the backdoor races ahead, so the
         *    parser lags and writes a LOWER value → that backward write is exactly
         *    what trips UVM's 32→64 wrap detector (uvm_gpu_semaphore.c:776) across a
         *    2nd context.  Suppress it; the backdoor's monotonic value stands.
         *  - But the backdoor MISSES some submits (e.g. CeUtils scrubberDestruct's
         *    final scrub), leaving the slot below lastSubmittedPayload → ce_utils.c:349
         *    "scrub timed out".  A FORWARD parser write is the legitimate completion
         *    the backdoor never reported — let it through so the scrub signals done.
         * Combined with per-slot (exact-GPA) ownership, co-located non-backdoor slots
         * still get all their parser writes. */
        if (sy && nvkvm_m2_bd_page_has(s, p) && sem_old != 0 && payload < sem_old) {
            if (s->trace) {
                qemu_log("nvkvm-gpu[%s] #12-L3c SEMW-DEFER backdoor owns phys=0x%llx "
                         "(client=0x%08x) — parser write suppressed\n", s->chip->name,
                         (unsigned long long)p, s->chan_client);
            }
        } else {
            nvkvm_phys_wr32(s, p, sy, payload); wrote = true;
        }
    }
    if (hostonly && p != NVKVM_GMMU_FAULT) {
        qemu_log("nvkvm-gpu[%s] M5.49b host-only sema VA=0x%llx payload=%u client=0x%08x "
                 "— host GPU writes this (sim suppressed)\n", s->chip->name,
                 (unsigned long long)va, payload, s->chan_client);
    }
    /* M5.19 — REAL forward prep: if the completion sema is SYSMEM, map it into the
     * host GR VAS so the REAL host GPU writes the payload here (guest GPA -> shared
     * memfd -> OS_DESCRIPTOR WB -> FIXED map at the matching VA).  Guest then reads
     * the host GPU's write coherently (WB snooped).  Idempotent; m2exec-gated. */
    /* M5.38: gate the sema fwd-map on m2hostsem (single-writer rule).  With
     * software completion active (!m2hostsem, the default) the LAGGING bridged
     * host channel must NOT have write access to the guest's tracking-sema
     * page: the host ran GPFIFO entry 0 ~40s late (hostUSERD get 0->1/2) and
     * DMA'd its stale payload=1 over the software value 0x1e, tripping UVM's
     * 32->64-bit wrap detector (uvm_gpu_semaphore.c:776 jump 0x1e ->
     * 0x100000001) and wedging CE2 (completed 0x100000054 > queued 0x54) ->
     * cuCtxCreate hang.  When m2hostsem=true the map happens and the software
     * writers are already gated off (see the per-client predicate at the release
     * sites) — exactly one writer in either mode.  M5.49b: arm the host-write map
     * ONLY for the compute client's CE channels (chan_client == m2_gr_client) —
     * the user-observable CE round-trip.  m2_gr_client is 0 until the 0xc7c0 GR
     * obj alloc, so RmInitAdapter and all UVM-client (kernel-internal) scrubs keep
     * their simulated completion; only the user round-trip is forced host-written. */
    if (hostonly && p != NVKVM_GMMU_FAULT && sy &&
        !nvkvm_m2_va_seen(s, s->chan_client, va & ~0xfffull)) {
        uint64_t gbase = p & ~0xfffull;
        bool mok = nvkvm_m2_back_and_map_sys(s, s->chan_client, va & ~0xfffull, gbase, 0x1000);
        qemu_log("nvkvm-gpu[%s] M5.19 fwd-map sema VA=0x%llx gpa=0x%llx -> %s\n",
                 s->chip->name, (unsigned long long)(va & ~0xfffull),
                 (unsigned long long)gbase, mok ? "MAPPED (host GPU writes completion, WB)"
                                                : "map-FAILED");
    }
    /* The BAR1 redir page is libcuda's locally-polled mirror; under host-only the
     * host GPU writes the real (fwd-mapped, WB) sema page directly, so skip it. */
    if (!hostonly && s->chan_gpfifo_phys && va >= s->chan_gpfifo_va &&
        va <  s->chan_gpfifo_va + NVKVM_CHAN_BUF_WINDOW) {
        uint64_t rp = s->chan_gpfifo_phys + (va - s->chan_gpfifo_va);
        nvkvm_fb_write(s, rp, payload, 4);     /* the page libcuda actually polls */
        if (out_redir) { *out_redir = rp; }
        wrote = true;
    } else if (out_redir) {
        *out_redir = 0;
    }
    return wrote;   /* host-only => false: caller skips its sim-log + high-word write */
}
/* Read one 32-bit word at a CHANNEL GPU VA (translate then phys read). */
static bool nvkvm_chan_rd32(NvkvmGpuEmul *s, uint64_t va, uint32_t *out)
{
    bool sys; uint64_t p = nvkvm_chan_translate(s, va, &sys);
    if (p == NVKVM_GMMU_FAULT) return false;
    *out = nvkvm_phys_rd32(s, p, sys);
    return true;
}

/* M5 — EXECUTE the copy-engine work submitted on the doorbell-rung channel.
 * Walk the GPFIFO [chan_gp_get, GP_PUT) (GP_PUT read from USERD @ +0x8C), and for
 * each pushbuffer parse the FERMI method stream (header: SEC_OP[31:29],
 * METHOD_ADDR[11:0]<<2, COUNT[28:16]) for the NVB0B5/NVC7B5 copy class.  On
 * LAUNCH_DMA (0x300) perform the op for real: REMAP_ENABLE(bit10) => fill
 * OFFSET_OUT with SET_REMAP_CONST_A (memset); else copy OFFSET_IN->OFFSET_OUT for
 * LINE_LENGTH_IN bytes (x LINE_COUNT).  All addresses are channel VAs translated
 * per-word.  This makes the scrubber's CE self-verify (mem_mgr.c:469) see real
 * data.  Bounded + fault-safe (bail on any miss). */
static void nvkvm_chan_execute(NvkvmGpuEmul *s)
{
    if (!s->chan_gpfifo_va || !s->chan_userd || !s->chan_gpfifo_ent) {
        return;
    }
    uint32_t gp_put = s->chan_userd_sys
        ? nvkvm_phys_rd32(s, s->chan_userd + 0x8C, true)
        : (uint32_t)nvkvm_fb_read(s, s->chan_userd + 0x8C, 4);
    if (s->m2_trace)
    qemu_log("nvkvm-gpu[%s] M5: chan_exec gpfifo=0x%llx userd=0x%llx(%s) "
             "gp_get=%u gp_put=%u ent=%u\n", s->chip->name,
             (unsigned long long)s->chan_gpfifo_va,
             (unsigned long long)s->chan_userd, s->chan_userd_sys ? "sys" : "fb",
             s->chan_gp_get, gp_put, s->chan_gpfifo_ent);
    /* Pick the channel's VAS by CONTENT, not by handle.  The instance block is
     * empty (GSP-managed) so it gives no PDB, and hVASpace=0 (device-default)
     * channels match no snooped VAS handle -> the try-all fallback picks a wrong
     * VAS that maps gpFifoVA to a stale/zero page.  Instead, among the snooped
     * VAS PDBs, choose the one under which the pending GPFIFO entry reads
     * NON-ZERO (a valid pushbuffer pointer) — that is the VAS that actually owns
     * this channel's ring.  Pin it in chan_pdb so every translate in this walk
     * (entry/pushbuffer/sema) uses the same correct VAS. */
    s->chan_pdb = 0;
    s->chan_gpfifo_phys = 0;
    s->chan_gpfifo_bar1off = 0;
    s->chan_fin_ring_off = 0;          /* #12 cont.25: re-resolve per channel */
    s->chan_fin_ring_found = false;
    if (gp_put < s->chan_gpfifo_ent && gp_put != s->chan_gp_get) {
        uint64_t eva = s->chan_gpfifo_va + (uint64_t)s->chan_gp_get * 8;
        /* M5.21: prefer the channel's OWN client VAS — authoritative, avoids the
         * cross-client aliasing the content-pick below falls into. */
        uint64_t own = nvkvm_chan_own_pdb(s);
        if (own) {
            bool sy = false;
            uint64_t p = nvkvm_walk_pdb(s, own, eva, &sy);
            if (p != NVKVM_GMMU_FAULT && nvkvm_phys_rd32(s, p, sy) != 0) {
                s->chan_pdb = own;
            }
        }
        /* #14: TWO passes.  Pass 0 restricts the content-pick to VASes owned by the
         * executing channel's client (directly or via an observed DUP_OBJECT edge —
         * the user VAS is captured under UVM's per-process gpu-ops client).  With two
         * concurrent processes the stock driver hands out IDENTICAL guest VAs, so the
         * blind first-non-zero scan below landed on process A's PDB while executing
         * process B's channel -> B's pushbuffer walked under A's VAS -> FAULT -> both
         * cuCtxCreate spun.  Pass 1 = the original blind probe, kept for channels
         * whose true root has no client linkage (single-process legacy behavior). */
        for (int pass = 0; s->chan_pdb == 0 && pass < 2; pass++) {
            /* #14: user compute clients NEVER take the blind pass — their channel
             * either resolves in an owned VAS or defers to the next doorbell (the
             * kernel whole-FB alias walk-succeeds for ANY VA and would be parsed as
             * a garbage pushbuffer).  Kernel channels keep the legacy blind probe.
             * EARLY-ARM: keyed on is_user_client (dup-src, known from process
             * start) — see nvkvm_chan_own_pdb_rs. */
            if (pass == 1 && nvkvm_m2_multiproc(s) &&
                nvkvm_m2_is_user_client(s, s->chan_client)) {
                break;
            }
            for (int i = 0; i < s->chan_vas_n; i++) {
                if (pass == 0 &&
                    !nvkvm_m2_vas_client_match(s, s->chan_vas[i].client,
                                               s->chan_vas[i].hvas, s->chan_client)) {
                    continue;   /* pass 0: own-client (direct or dup-linked) roots only */
                }
                bool sy = false;
                uint64_t p = nvkvm_walk_pdb(s, s->chan_vas[i].pdb, eva, &sy);
                if (p == NVKVM_GMMU_FAULT) { continue; }
                if (nvkvm_phys_rd32(s, p, sy) != 0) {   /* valid GP_ENTRY0 (pb low) */
                    s->chan_pdb = s->chan_vas[i].pdb;
                    break;
                }
            }
        }
        qemu_log("nvkvm-gpu[%s] M5: chan_exec hvas=0x%08x picked_pdb=0x%llx "
                 "gpfifoVA=0x%llx\n", s->chip->name, s->chan_hvaspace,
                 (unsigned long long)s->chan_pdb,
                 (unsigned long long)s->chan_gpfifo_va);
        /* M5.14: content-pick failed -> device-default-VAS channel (hVASpace=0), whose VAS is never
         * snooped into chan_vas[] (only explicit VASpace objects are). Derive the channel's PDB
         * from its INSTANCE BLOCK (RAMIN PAGE_DIR_BASE @0x200 LO / @0x204 HI) — the HW-authoritative
         * VAS root for THIS channel (libcuda's compute USERMODE channel gpfifo 0x121010000). Use it
         * only if it resolves the pending GPFIFO entry to a non-zero pushbuffer pointer. */
        if (s->chan_pdb == 0 && s->chan_inst_block) {
            bool isys = s->chan_inst_sys;
            uint32_t plo = isys ? nvkvm_phys_rd32(s, s->chan_inst_block + NVKVM_RAMIN_PDB_LO_OFF, true)
                                : (uint32_t)nvkvm_fb_read(s, s->chan_inst_block + NVKVM_RAMIN_PDB_LO_OFF, 4);
            uint32_t phi = isys ? nvkvm_phys_rd32(s, s->chan_inst_block + NVKVM_RAMIN_PDB_HI_OFF, true)
                                : (uint32_t)nvkvm_fb_read(s, s->chan_inst_block + NVKVM_RAMIN_PDB_HI_OFF, 4);
            uint64_t ipdb = ((uint64_t)phi << 32) | ((uint64_t)plo & 0xFFFFF000ull);
            if (ipdb) {
                bool sy = false;
                uint64_t p = nvkvm_walk_pdb(s, ipdb, eva, &sy);
                uint32_t v = (p != NVKVM_GMMU_FAULT) ? nvkvm_phys_rd32(s, p, sy) : 0;
                qemu_log("nvkvm-gpu[%s] M5.14: instblk=0x%llx(%s) PDB=0x%llx; gpfifo eva=0x%llx -> "
                         "%s val=0x%08x\n", s->chip->name, (unsigned long long)s->chan_inst_block,
                         isys ? "sys" : "fb", (unsigned long long)ipdb, (unsigned long long)eva,
                         (p == NVKVM_GMMU_FAULT) ? "FAULT" : (sy ? "SYS" : "FB"), v);
                if (p != NVKVM_GMMU_FAULT && v != 0) { s->chan_pdb = ipdb; }
            } else {
                qemu_log("nvkvm-gpu[%s] M5.14: instblk=0x%llx PDB empty (GSP-managed)\n",
                         s->chip->name, (unsigned long long)s->chan_inst_block);
            }
        }
        /* DIAG: when content-pick fails, show what EACH snooped VAS resolves the
         * GPFIFO entry VA to (fault / phys+aperture) and the value read there. */
        if (s->trace && s->chan_pdb == 0) {
            for (int i = 0; i < s->chan_vas_n; i++) {
                bool sy = false;
                uint64_t p = nvkvm_walk_pdb(s, s->chan_vas[i].pdb, eva, &sy);
                if (p == NVKVM_GMMU_FAULT) {
                    qemu_log("nvkvm-gpu[%s] DIAG vas[%d] hvas=0x%08x pdb=0x%llx "
                             "eva=0x%llx -> FAULT\n", s->chip->name, i,
                             s->chan_vas[i].hvas, (unsigned long long)s->chan_vas[i].pdb,
                             (unsigned long long)eva);
                } else {
                    qemu_log("nvkvm-gpu[%s] DIAG vas[%d] hvas=0x%08x pdb=0x%llx "
                             "eva=0x%llx -> %s phys=0x%llx val=0x%08x\n", s->chip->name,
                             i, s->chan_vas[i].hvas, (unsigned long long)s->chan_vas[i].pdb,
                             (unsigned long long)eva, sy ? "SYS" : "FB",
                             (unsigned long long)p, nvkvm_phys_rd32(s, p, sy));
                }
            }
        }
    }
    /* M5.16 — last-resort GPFIFO resolution for GSP-managed channel VASes.  When
     * the channel-VAS walk above failed to find a PDB under which the pending GP
     * entry reads non-zero (chan_pdb==0), the ring's leaf PTE isn't reliable in
     * our FB — but the guest's OWN CPU mapping wrote the entry through BAR1, and
     * that landed in our FB at the TRUE backing page (recorded in bar1_wpg).
     * Try each guest-written vidmem page (MRU first) as the GPFIFO base: if the
     * pending entry there decodes to a plausible pushbuffer (non-zero, sane len,
     * pb VA resolves), pin it as chan_gpfifo_phys so the entry read below reads
     * the real ring instead of the stale aliased page.  This is the data-plane
     * keystone: stop trusting the channel-VAS walk for GSP-managed rings. */
    if (s->chan_pdb == 0 && gp_put < s->chan_gpfifo_ent && gp_put != s->chan_gp_get) {
        bool visited[NVKVM_MAX_BAR1PG] = { false };
        uint64_t off = (uint64_t)s->chan_gp_get * 8;
        for (int scan = 0; scan < s->bar1_wpg_n && off + 8 <= 0x1000; scan++) {
            int best = -1;                       /* pick MRU (highest seq) unvisited */
            for (int i = 0; i < s->bar1_wpg_n; i++) {
                if (visited[i] || !s->bar1_wpg[i].page) { continue; }
                if (best < 0 || s->bar1_wpg[i].seq > s->bar1_wpg[best].seq) { best = i; }
            }
            if (best < 0) { break; }
            visited[best] = true;
            uint64_t cand = s->bar1_wpg[best].page;
            uint32_t e0 = (uint32_t)nvkvm_fb_read(s, cand + off, 4);
            uint32_t e1 = (uint32_t)nvkvm_fb_read(s, cand + off + 4, 4);
            if (e0 == 0 && e1 == 0) { continue; }
            uint64_t pb = (uint64_t)(e0 & 0xFFFFFFFCu) | ((uint64_t)(e1 & 0xFFu) << 32);
            uint32_t pblen = (e1 >> 10) & 0x1FFFFFu;
            if (!pb || pblen == 0 || pblen > 0x40000) { continue; }
            /* #12 cont.27: capture THIS channel's ring BAR1 page-offset for the
             * finishPayload forge BEFORE the pushbuffer-VAS validation below — which
             * FAILS for a GSP-managed bUseBar1 CeUtils (no VAS we hold), so chan_pdb
             * stays 0 and chan_gpfifo_bar1off is never pinned, leaving the forge's
             * BAR1 path dark (cont.24 redir=0x0).  The forge resolves finishPayload FB
             * = walk_pdb(bar1_pdb, off+0x8004), so we need the ONE gpfifo RING page.
             *
             * cont.26's single-pb check flip-flopped onto PUSHBUFFER pages: a bUseBar1
             * channel buffer's pushbuffer is ~100 BAR1-tracked pages whose method bytes
             * can coincidentally decode as one in-range GP entry.  Deterministic fix:
             * read the candidate page AS A GP-ENTRY ARRAY over the pending window
             * [gp_get, gp_put) — the true ring has EVERY non-zero entry decoding to a
             * pushbuffer pointer in THIS channel's buffer [gpfifo_va-0x100000, gpfifo_va)
             * (each scrub op's pb lives in [pbGpuVA, gpfifo_va), pbGpuVA = gpfifo_va -
             * channelPbSize); a pushbuffer page has method words that decode OUT of range
             * -> disqualified.  Require >=2 in-range and 0 out-of-range.  Channels sit
             * 256MB+ apart so no cross-channel collision in the window.  cand =
             * s->bar1_wpg[best].page (read above); off = chan_gp_get*8 (page base). */
            if (!s->chan_fin_ring_found) {
                int rgood = 0, rbad = 0;
                for (uint32_t k = 0; k < 16; k++) {
                    uint64_t eo = off + (uint64_t)k * 8;
                    if (eo + 8 > 0x1000) { break; }
                    uint32_t f0 = (uint32_t)nvkvm_fb_read(s, cand + eo, 4);
                    uint32_t f1 = (uint32_t)nvkvm_fb_read(s, cand + eo + 4, 4);
                    if (!f0 && !f1) { continue; }   /* unwritten slot */
                    uint64_t fpb = (uint64_t)(f0 & 0xFFFFFFFCu) | ((uint64_t)(f1 & 0xFFu) << 32);
                    if (fpb && fpb < s->chan_gpfifo_va &&
                        (s->chan_gpfifo_va - fpb) <= 0x100000ull) { rgood++; }
                    else { rbad++; }
                }
                if (rgood >= 2 && rbad == 0) {
                    s->chan_fin_ring_off = s->bar1_wpg[best].off;
                    s->chan_fin_ring_found = true;
                }
            }
            /* The decoded pushbuffer must resolve to REAL content (a valid method
             * header), not just any non-faulting page.  chan_translate's try-all
             * fallback picks the FIRST VAS that maps pb — often a wrong aliasing
             * VAS whose page reads 0 (proven: vas[0] pdb 0x2efba5000 -> empty,
             * vs the channel's real device-default vas pdb 0x2efa4c000 -> the
             * pushbuffer 0x20016000).  So content-pick the VAS under which pb's
             * first word is non-zero and PIN it as chan_pdb — that VAS owns the
             * whole channel working set (pushbuffer + sema), so every subsequent
             * translate uses it.  Skip this GPFIFO candidate if NO VAS yields a
             * non-zero pb word (the page was a stale/foreign ring). */
            uint64_t pb_pdb = 0;
            /* M5.21: prefer the channel's OWN client VAS so pb (and the whole
             * working set) resolves through the right address space and mirrors
             * under the right client — not a foreign client's aliasing VAS. */
            uint64_t own = nvkvm_chan_own_pdb(s);
            if (own) {
                bool sy = false;
                uint64_t pp = nvkvm_walk_pdb(s, own, pb, &sy);
                if (pp != NVKVM_GMMU_FAULT && nvkvm_phys_rd32(s, pp, sy) != 0) {
                    pb_pdb = own;
                }
            }
            for (int v = 0; pb_pdb == 0 && v < s->chan_vas_n; v++) {
                bool sy = false;
                uint64_t pp = nvkvm_walk_pdb(s, s->chan_vas[v].pdb, pb, &sy);
                if (pp == NVKVM_GMMU_FAULT) { continue; }
                if (nvkvm_phys_rd32(s, pp, sy) != 0) { pb_pdb = s->chan_vas[v].pdb; break; }
            }
            if (pb_pdb == 0) { continue; }   /* pb has no real backing in any VAS */
            s->chan_gpfifo_phys = cand;
            s->chan_gpfifo_bar1off = s->bar1_wpg[best].off;   /* #12: BAR1 offset of the ring base */
            s->chan_pdb = pb_pdb;            /* pin the channel's true VAS for pb/sema */
            if (s->m2_trace)
            qemu_log("nvkvm-gpu[%s] M5.16: GPFIFO resolved via BAR1-written page "
                     "FB 0x%llx (seq %llu) -> entry pb=0x%llx len=%u; pinned VAS "
                     "pdb=0x%llx (pushbuffer-backed) [VAS-walk gave wrong page]\n",
                     s->chip->name, (unsigned long long)cand,
                     (unsigned long long)s->bar1_wpg[best].seq,
                     (unsigned long long)pb, pblen, (unsigned long long)pb_pdb);
            break;
        }
    }
    if (gp_put >= s->chan_gpfifo_ent) {
        return;                                  /* implausible -> bail */
    }
    /* M5.24 GPFIFO double-mmap (host-channel bridge step 2): the host channel
     * expects its GPFIFO ring at gpFifoOffset (gpfifo_va) in its VAS — client-
     * allocated, NOT RM-allocated — but we never mapped it, so the rung host channel
     * fetched empty entries.  The guest wrote its GP entries (vidmem) via BAR1 to
     * chan_gpfifo_phys (M5.16-resolved).  back_and_map: alloc host GPU mem, seed-copy
     * the current entries, double-mmap at chan_gpfifo_phys (future guest BAR1 GP
     * writes land in host mem), and map_dma FIXED at gpfifo_va into the channel's VAS
     * (via the client grmapper — same VAS the host channel runs in, M5.20/M5.21).
     * Then the rung host channel fetches the REAL GP entries -> the pushbuffers
     * (already zero-copy-mapped, M5.19) -> runs + writes the completion.  Gated on
     * m2exec + a resolved GSP-managed ring (chan_gpfifo_phys); idempotent per VA. */
    if (s->m2exec && s->chan_gpfifo_phys && s->chan_gpfifo_va &&
        !nvkvm_m2_va_seen(s, s->chan_client, s->chan_gpfifo_va)) {
        uint64_t gsz = ((uint64_t)s->chan_gpfifo_ent * 8 + 0xfffull) & ~0xfffull;
        if (gsz == 0 || gsz > 0x10000) { gsz = 0x10000; }
        bool gok = nvkvm_m2_back_and_map(s, s->chan_client, s->chan_gpfifo_va,
                                         s->chan_gpfifo_phys, gsz, true, "gpfifo-bridge");
        qemu_log("nvkvm-gpu[%s] M5.24 GPFIFO double-mmap va=0x%llx phys=0x%llx sz=0x%llx "
                 "client=0x%08x -> %s\n", s->chip->name,
                 (unsigned long long)s->chan_gpfifo_va,
                 (unsigned long long)s->chan_gpfifo_phys, (unsigned long long)gsz,
                 s->chan_client, gok ? "MAPPED (host channel fetches guest GP entries)"
                                     : "map-FAILED");
    }
    /* NVC56F host-channel semaphore-release tracking (methods 0x5c..0x6c).  The
     * golden-image / watchdog / scrubber channels append a SEM_EXECUTE RELEASE
     * after their engine work to signal completion; channelWaitForFinishPayload
     * polls that semaphore.  We honor the EXPLICIT release here (translate the
     * SEM addr, write the payload) WITHOUT running the GR/compute methods — per
     * the Phase-B design we never emulate GR, we only signal completion.  Tracked
     * at function scope so addr/payload set in one method group apply to a later
     * SEM_EXECUTE.  ADDR is 64-bit: LO bits[31:2] | HI<<32. */
    uint64_t sem_addr = 0;
    uint32_t sem_pay_lo = 0, sem_pay_hi = 0;
    /* CE-class (NVC8B5/NVB0B5) completion semaphore — the one
     * channelWaitForFinishPayload() polls (pbGpuVA + finishPayloadOffset); the
     * CeUtils memory scrubber waits on it (ce_utils.c:349).  Released by
     * LAUNCH_DMA when SEMAPHORE_TYPE != NONE.  Distinct from the NVC56F host
     * semaphore (sem_addr) which the same scrub pushbuffer ALSO releases at
     * semaOffset — honoring only the host one left finishPayload unwritten,
     * so the scrubber timed out. */
    uint64_t ce_sem_addr = 0;
    uint32_t ce_sem_pay = 0;
    /* COMPUTE/3D-class (NVC7C0 AMPERE_COMPUTE_B etc) report-semaphore release.
     * cuCtxCreate's compute init work releases a dedicated completion semaphore
     * via SET_REPORT_SEMAPHORE_A/B(addr)+C(payload)+D(trigger,OPERATION=RELEASE);
     * libcuda's blocking-sync poll spins on THAT semaphore.  The parser
     * previously only honored CE + NVC56F host releases, so the compute
     * completion sema was never written and the wait hung (the dataless os-event
     * wake fired but the decisive bit stayed 0).  ADDR_UPPER[7:0]@0x1b00,
     * ADDR_LOWER[31:0]@0x1b04, PAYLOAD@0x1b08, D@0x1b0c. */
    uint64_t cr_sem_addr = 0;
    uint32_t cr_sem_pay = 0;
    s->chan_sem_released = false;
    uint32_t guard = 0;
    for (uint32_t idx = s->chan_gp_get; idx != gp_put &&
         guard < s->chan_gpfifo_ent; idx = (idx + 1) % s->chan_gpfifo_ent, guard++) {
        uint32_t e0, e1;
        uint64_t eva = s->chan_gpfifo_va + (uint64_t)idx * 8;
        if (s->chan_gpfifo_phys && (uint64_t)idx * 8 + 8 <= 0x1000) {
            /* M5.16: read the entry from the BAR1-resolved true ring page (the
             * channel-VAS walk gives a stale page for GSP-managed VASes). */
            e0 = (uint32_t)nvkvm_fb_read(s, s->chan_gpfifo_phys + (uint64_t)idx * 8, 4);
            e1 = (uint32_t)nvkvm_fb_read(s, s->chan_gpfifo_phys + (uint64_t)idx * 8 + 4, 4);
        } else if (!nvkvm_chan_rd32(s, eva, &e0) || !nvkvm_chan_rd32(s, eva + 4, &e1)) {
            qemu_log("nvkvm-gpu[%s] M5: chan_exec GPFIFO entry[%u] @VA 0x%llx "
                     "FAULTED (no VAS maps it)\n", s->chip->name, idx,
                     (unsigned long long)eva);
            break;
        }
        uint64_t pb   = (uint64_t)(e0 & 0xFFFFFFFCu) | ((uint64_t)(e1 & 0xFFu) << 32);
        uint32_t pblen = (e1 >> 10) & 0x1FFFFFu;   /* GP_ENTRY1_LENGTH: # method words */
        /* M5.39: an all-zero GP entry is NOT real work.  This fires when our
         * saved chan_gp_get (e.g. 1) is stale relative to a rezeroed/host-backed
         * USERD whose GP_PUT reads 0: the [gp_get,gp_put) walk then wraps the
         * entire 1023-entry ring with every entry zero, each paying a 6-level VA
         * walk + pb_read fault ON THE vCPU's SYNCHRONOUS MMIO-exit path (this was
         * ~45% of the QEMU log and the dominant cuCtxCreate-era stall).  A real
         * pending entry is never all-zero (pb=0 is an invalid pushbuffer addr).
         * Resync the cursor to gp_put and bail. */
        if (!e0 && !e1) {
            s->chan_gp_get = gp_put;
            break;
        }
        /* M5.19 — REAL forward prep: make the host GPU able to read this pushbuffer
         * DIRECTLY from guest sysmem.  The pushbuffer is SYSMEM (resolved via the
         * pinned chan_pdb -> SYS guest GPA).  Map guest VA -> GPA -> shared-memfd
         * stub VA -> OS_DESCRIPTOR(COHERENCY_CACHED=WB) -> FIXED-map at the matching
         * VA in the host GR VAS.  The host channel (shadow_fwd, same handles) then
         * reads the exact bytes the guest wrote — no trap, WB-coherent.  Gated on
         * m2exec; idempotent (m2_va_seen). */
        if (s->m2exec && pb && pblen) {
            bool psy = false; uint64_t pgpa = nvkvm_chan_translate(s, pb, &psy);
            if (pgpa != NVKVM_GMMU_FAULT && psy) {        /* sysmem pushbuffer only */
                uint64_t pbbase = pb & ~0xfffull;
                uint64_t gbase  = pgpa - (pb - pbbase);   /* GPA of the page base */
                uint64_t msz    = (((pb + (uint64_t)pblen * 4) - pbbase) + 0xfffull) & ~0xfffull;
                if (!nvkvm_m2_va_seen(s, s->chan_client, pbbase)) {
                    bool mok = nvkvm_m2_back_and_map_sys(s, s->chan_client, pbbase, gbase, msz);
                    qemu_log("nvkvm-gpu[%s] M5.19 fwd-map pushbuffer VA=0x%llx gpa=0x%llx "
                             "sz=0x%llx client=0x%08x -> %s\n", s->chip->name,
                             (unsigned long long)pbbase, (unsigned long long)gbase,
                             (unsigned long long)msz, s->chan_client,
                             mok ? "MAPPED (host GPU reads guest sysmem, WB)" : "map-FAILED");
                }
            }
        }
        { uint32_t w0 = 0; bool pbok = nvkvm_chan_rd32(s, pb, &w0);
          qemu_log("nvkvm-gpu[%s] M5: chan_exec entry[%u] pb=0x%llx pblen=%u "
                   "pb_read=%s w0=0x%08x\n", s->chip->name, idx,
                   (unsigned long long)pb, pblen, pbok ? "ok" : "FAULT", w0);
          /* M5.17 DIAG: when the userspace pushbuffer's first word reads 0 (no valid
           * method header), the VAS-selection picked a wrong/aliasing page.  Dump how
           * pb resolves under EACH snooped VAS (phys+aperture+value), and the SYS read
           * at the same numeric addr — to find which VAS owns the compute pushbuffer
           * and whether it's a sysmem-aperture miss.  Capped one-shot. */
          if (s->trace && w0 == 0) {
              static uint32_t pbd;
              if (pbd++ < 12) {
                  for (int v = 0; v < s->chan_vas_n; v++) {
                      bool sy = false;
                      uint64_t pp = nvkvm_walk_pdb(s, s->chan_vas[v].pdb, pb, &sy);
                      if (pp == NVKVM_GMMU_FAULT) {
                          qemu_log("nvkvm-gpu[%s] M5.17 pb=0x%llx vas[%d] hvas=0x%08x "
                                   "pdb=0x%llx -> FAULT\n", s->chip->name,
                                   (unsigned long long)pb, v, s->chan_vas[v].hvas,
                                   (unsigned long long)s->chan_vas[v].pdb);
                      } else {
                          qemu_log("nvkvm-gpu[%s] M5.17 pb=0x%llx vas[%d] hvas=0x%08x "
                                   "pdb=0x%llx -> %s phys=0x%llx fbval=0x%08x sysval=0x%08x\n",
                                   s->chip->name, (unsigned long long)pb, v,
                                   s->chan_vas[v].hvas, (unsigned long long)s->chan_vas[v].pdb,
                                   sy ? "SYS" : "FB", (unsigned long long)pp,
                                   (uint32_t)nvkvm_fb_read(s, pp, 4),
                                   nvkvm_phys_rd32(s, pp, true));
                      }
                  }
              }
          } }
        /* method-stream parse */
        uint64_t off_in = 0, off_out = 0;
        uint32_t llen = 0, lcount = 1, remapA = 0;
        uint32_t src_pm = 0, dst_pm = 0;   /* SET_SRC/DST_PHYS_MODE target */
        for (uint32_t w = 0; w < pblen; ) {
            uint32_t hdr;
            if (!nvkvm_chan_rd32(s, pb + (uint64_t)w * 4, &hdr)) { break; }
            w++;
            uint32_t secop = (hdr >> 29) & 0x7;
            uint32_t maddr = (hdr & 0xFFFu) << 2;
            uint32_t cnt   = (hdr >> 16) & 0x1FFFu;
            if (secop != 1 && secop != 3 && secop != 5) { continue; } /* INC/NON_INC/ONE_INC */
            for (uint32_t j = 0; j < cnt && w < pblen; j++, w++) {
                uint32_t d;
                if (!nvkvm_chan_rd32(s, pb + (uint64_t)w * 4, &d)) { w = pblen; break; }
                uint32_t m = (secop == 3) ? maddr : maddr + j * 4; /* NON_INC holds */
                /* M5.22 (a) INSTRUMENT: dump every decoded method so we can see
                 * EXACTLY what cuCtxCreate submits (esp. any completion-signalling
                 * method the parser doesn't yet honor, and the compute QMD launch).
                 * Capped + trace-gated; compiled in for bring-up. */
                if (s->trace) {
                    static uint32_t m22n;
                    if (s->m2_trace && m22n++ < 1200) {
                        qemu_log("nvkvm-gpu[%s] M5.22 method client=0x%08x gpfifo=0x%llx "
                                 "m=0x%04x d=0x%08x\n", s->chip->name, s->chan_client,
                                 (unsigned long long)s->chan_gpfifo_va, m, d);
                    }
                }
                switch (m) {
                case 0x400: off_in  = (off_in  & 0xFFFFFFFFull) | ((uint64_t)d << 32); break; /* IN_UPPER  */
                case 0x404: off_in  = (off_in  & ~0xFFFFFFFFull) | d; break;                  /* IN_LOWER  */
                case 0x408: off_out = (off_out & 0xFFFFFFFFull) | ((uint64_t)d << 32); break; /* OUT_UPPER */
                case 0x40C: off_out = (off_out & ~0xFFFFFFFFull) | d; break;                  /* OUT_LOWER */
                case 0x418: llen   = d; break;                                               /* LINE_LENGTH_IN */
                case 0x41C: lcount = d ? d : 1; break;                                        /* LINE_COUNT */
                case 0x260: src_pm = d & 3; break;                                           /* SET_SRC_PHYS_MODE */
                case 0x264: dst_pm = d & 3; break;                                           /* SET_DST_PHYS_MODE */
                case 0x700: remapA = d; break;                                               /* SET_REMAP_CONST_A */
                /* CE-class completion semaphore (NVC8B5_SET_SEMAPHORE_A/B/PAYLOAD).
                 * A=upper[24:0], B=lower[31:0], PAYLOAD=release value. */
                case 0x240: ce_sem_addr = (ce_sem_addr & 0xFFFFFFFFull) | ((uint64_t)(d & 0x01FFFFFFu) << 32); break;
                case 0x244: ce_sem_addr = (ce_sem_addr & ~0xFFFFFFFFull) | d; break;
                case 0x248: ce_sem_pay = d; break;
                case 0x300: {                                                                /* LAUNCH_DMA */
                    bool remap    = (d >> 10) & 1;
                    bool mscrub   = (d >> 23) & 1;   /* MEMORY_SCRUB_ENABLE [23] */
                    bool src_phys = (d >> 12) & 1;   /* SRC_TYPE PHYSICAL */
                    bool dst_phys = (d >> 13) & 1;   /* DST_TYPE PHYSICAL */
                    uint32_t sem_type = (d >> 3) & 0x3; /* SEMAPHORE_TYPE [4:3], !=0 => release */
                    /* #13: DATA_TRANSFER_TYPE [1:0] (NVC7B5, == NVB0B5): 0 = NONE — a
                     * semaphore-release-only launch that moves NO data.  UVM/RM emit these
                     * as the completion release right after a real copy; re-executing the
                     * copy with the STALE offset/length state (what we did before decoding
                     * this field) issued phantom duplicate copies — and phantom translate
                     * FAULTs for VIRT addresses.  EXCEPT: a CE MEMSET (REMAP_ENABLE[10] /
                     * MEMORY_SCRUB_ENABLE[23]) also has DATA_TRANSFER_TYPE==NONE but DOES
                     * write via remap — zeroing bytes for those killed the scrubber's
                     * zero-fill and hung cuCtxCreate (the round-4 regression).  Only zero
                     * for a GENUINE no-op (no transfer, no remap, no scrub). */
                    bool xfer_none = ((d & 0x3u) == 0u);
                    uint64_t bytes = (uint64_t)llen * lcount;
                    if (bytes > (16u << 20)) bytes = 16u << 20;  /* safety cap */
                    if (xfer_none && !remap && !mscrub) { bytes = 0; }
                    /* M5.60 (Phase A — user-CE dst real-backing, the cup5/LLM-hang fix
                     * prerequisite): the copy DEST must be a REAL host vidmem object
                     * mapped in the GR fvas (not a fake fb_page) — both so the CPU copy
                     * writes live vidmem now, and so a real host CE can DMA it later
                     * (Phase B). The dst buffer lives in the COMPUTE client's VAS, which
                     * was last walked at ctx-create; a buffer allocated afterwards (e.g.
                     * LLM weights) is missed -> verdict=fbpage. When we observe an
                     * un-backed FB dst, re-run the proven compute-VAS backing walk ONCE
                     * (enum_gr_sysmem is idempotent via the global m2_va_seen dedup, so it
                     * backs only genuinely-new leaves and is cheap once the hole is filled
                     * -> the next copy classifies gpga and skips the re-walk). Gated to the
                     * forwarded user-CE path; never the kernel scrubber/MEMSET or a sysmem
                     * (DtoH) dst. A total cap bounds any never-backable pathological leaf. */
                    if (s->m2cefwd && !mscrub && !remap && !xfer_none && s->m2_gr_client &&
                        nvkvm_m2_is_user_ce(s, s->chan_client)) {
                        uint64_t full = (uint64_t)llen * lcount;
                        if (full >= 4 && dst_phys && dst_pm == 0) {
                            /* PHYSICAL-FB dst: off_out IS the guest-FB-phys and a CE physical
                             * copy bypasses the MMU, so the page-table walk can NEVER discover
                             * this dst (no PTE). Back the contiguous dst range DIRECTLY as one
                             * real host vidmem object (gpga_obj) when it still resolves to a
                             * fake fb_page. The CPU copy then writes live vidmem, host mem.used
                             * reflects it, and Phase B's host CE gets a real dst object.
                             * classify-gated -> backs once; total cap bounds any failure loop. */
                            if (nvkvm_dp_classify_fb(s, off_out) == 0 ||
                                nvkvm_dp_classify_fb(s, off_out + full - 4) == 0) {
                                static uint32_t m560p;
                                if (m560p < 4096) {
                                    m560p++;
                                    /* CE-fwd map-on-touch: back GPU-ONLY (zero host BAR1). A
                                     * physical CE copy has no PTE, so the GMMU walk can never find
                                     * this dst — back it here directly. gpga_obj_ex keeps it
                                     * gpu_only if still blank (the usual case for a CE dst) and the
                                     * guest gets a CPU view lazily on first touch (DtoH read). */
                                    int oi = nvkvm_m2_gpga_obj_ex(s, s->m2_gr_client, off_out,
                                                                  off_out, full, true);
                                    qemu_log("nvkvm-gpu[%s] M5.60 user-CE PHYS dst back[gpu_only] "
                                             "gpga=0x%llx full=%llu -> obj=%d (#%u)\n",
                                             s->chip->name, (unsigned long long)off_out,
                                             (unsigned long long)full, oi, m560p);
                                }
                            }
                        } else if (full >= 4 && !dst_phys) {
                            /* VIRTUAL dst: it has PTEs -> re-walk the compute VAS so the proven
                             * coalescing path backs its (possibly non-contiguous) leaves. */
                            bool dsy = false;
                            uint64_t dp0 = nvkvm_chan_translate(s, off_out, &dsy);
                            if (dp0 != NVKVM_GMMU_FAULT && !dsy &&
                                nvkvm_dp_classify_fb(s, dp0) == 0) {
                                static uint32_t m560v;
                                if (m560v < 4096) {
                                    m560v++;
                                    /* CE-fwd map-on-touch: re-walk the compute VAS so the proven
                                     * coalescing path backs this VIRTUAL dst's leaves. leaf_flush
                                     * now requests gpu_only by default for compute clients, so the
                                     * (blank) dst leaves land off-BAR1 and promote on first touch. */
                                    qemu_log("nvkvm-gpu[%s] M5.60 user-CE VIRT dst un-backed "
                                             "va=0x%llx dphys=0x%llx -> re-walk compute VAS "
                                             "clients=%d (#%u)\n", s->chip->name,
                                             (unsigned long long)off_out,
                                             (unsigned long long)dp0, s->m2_gr_clients_n, m560v);
                                    /* #14: re-walk EVERY process's compute VAS (each sweep is
                                     * owner-scoped, so this backs the dst under whichever
                                     * process owns it).  Single process: one client = the
                                     * old single enum call. */
                                    for (int gk = 0; gk < s->m2_gr_clients_n; gk++) {
                                        nvkvm_m2_enum_gr_sysmem(s, s->m2_gr_clients[gk]);
                                    }
                                }
                            }
                        }
                    }
                    /* Resolve a CE address: PHYSICAL -> the offset IS the phys addr
                     * (aperture from PHYS_MODE: 0=FB else sysmem); VIRTUAL ->
                     * translate via the channel VAS (leaf PTE picks FB/sys). */
                    #define NVKVM_CE_RESOLVE(off, phys, pm, sysv) \
                        ((phys) ? ((sysv) = ((pm) != 0), (off)) \
                                : nvkvm_chan_translate(s, (off), &(sysv)))
                    nvkvm_t_mark_start();
                    /* CE-EXEC fwd (A): if the host CE will run this LAUNCH_DMA for real (m2cexec +
                     * user-CE channel, fully VIRTUAL so its src/dst resolve in the host CE VAS at
                     * identity VAs), skip BOTH the CPU byte-copy and the CPU completion-sema below —
                     * the forwarded host CE pushbuffer writes the data and releases the sema. A
                     * PHYSICAL copy (guest-fb-phys, no host meaning) stays CPU-emulated. */
                    bool host_ce = s->m2cexec && !mscrub && !remap && !src_phys && !dst_phys &&
                                   nvkvm_m2_is_user_ce(s, s->chan_client);
                    uint64_t t0ce = nvkvm_now_ns();    /* M5.11: emulated-CE byte-copy time-share */
                    if (host_ce) {
                        /* host CE owns the copy + the sema release; CPU does nothing here */
                    } else if (mscrub) {
                        /* MEMORY_SCRUB: zero the dst region.  Our FB backing is
                         * sparse-zero (unwritten reads return 0), so the data
                         * write is a no-op; the completion semaphore below is
                         * what unblocks the CeUtils scrubber.  No src is set. */
                    } else if (remap) {
                        /* M5.44 TRACE: a simulated CE fill whose PHYS dst covers a
                         * registered channel USERD page writes THROUGH the fbback
                         * overlay into the REAL host USERD object — i.e. a guest
                         * free+scrub wipes GP_PUT on the host channel. Make that
                         * visible: it explains post-mortem put=0 reads. */
                        if (dst_phys && dst_pm == 0) {
                            for (int k = 0; k < s->m2_chanbuf_n; k++) {
                                if (s->m2_chanbuf[k].fb_base >= off_out &&
                                    s->m2_chanbuf[k].fb_base < off_out + bytes) {
                                    qemu_log("nvkvm-gpu[%s] M5.44 CE-FILL WIPES USERD "
                                             "fb=0x%llx (chan 0x%08x) dst=0x%llx bytes=%llu "
                                             "const=0x%x\n", s->chip->name,
                                             (unsigned long long)s->m2_chanbuf[k].fb_base,
                                             s->m2_chanbuf[k].chan,
                                             (unsigned long long)off_out,
                                             (unsigned long long)bytes, remapA);
                                }
                            }
                        }
                        for (uint64_t b = 0; b + 4 <= bytes; ) {
                            bool sy; uint64_t p = NVKVM_CE_RESOLVE(off_out + b, dst_phys, dst_pm, sy);
                            if (p == NVKVM_GMMU_FAULT) break;
                            uint64_t span = bytes - b, dpg = 0x1000ull - (p & 0xfffull);
                            if (span > dpg) span = dpg;
                            span &= ~3ull; if (span == 0) span = 4;
                            /* M5.48: never let a fill zero a live channel's USERD page (its
                             * GP_PUT/GP_GET ring cursors) — that idles the ring forever and
                             * hangs the guest's sync.  Skip the span (it's one 4 KiB page). */
                            if (!sy && nvkvm_fb_is_live_userd(s, p)) { b += span; continue; }
                            uint8_t *dhp = sy ? NULL : nvkvm_fb_host_ptr(s, p, true);  /* M5.12 PERF */
                            if (!sy && dhp && remapA == 0) {          /* zero-fill: bulk */
                                memset(dhp, 0, span);
                            } else if (!sy && dhp) {                  /* pattern-fill: per-word into ptr */
                                for (uint64_t k = 0; k < span; k += 4) { stl_le_p(dhp + k, remapA); }
                            } else {                                  /* sysmem dst: per-word DMA */
                                for (uint64_t k = 0; k < span; k += 4) { nvkvm_phys_wr32(s, p + k, sy, remapA); }
                            }
                            /* #13: a fill can write PTEs too (UVM's pte_batch flushes a run of
                             * IDENTICAL PTE values as a CE memset_8) — same hook as the copy dst. */
                            if (!sy) { nvkvm_m2_ce_fb_write_hook(s, p, span); }
                            b += span;
                        }
                    } else {
                        /* M5.12 PERF: copy in PAGE-SPANS, not per-4-byte. The old loop called the
                         * GMMU translate + O(n) overlay-scan TWICE per 4 bytes (m569: 42% of gen).
                         * Translate src+dst ONCE per span, resolve host pointers ONCE, bulk-copy the
                         * contiguous run clamped to the 4 KiB page boundary on BOTH ends (overlay /
                         * fb_page backing is only guaranteed page-contiguous). fb<->fb = memcpy; a
                         * sysmem end falls back to a per-word DMA loop over the span (translate still
                         * amortized). Byte-IDENTICAL to the per-word path — just far fewer walks. */
                        bool logged0 = false;
                        for (uint64_t b = 0; b + 4 <= bytes; ) {
                            bool ssy, dsy;
                            uint64_t sp = NVKVM_CE_RESOLVE(off_in + b,  src_phys, src_pm, ssy);
                            uint64_t dp = NVKVM_CE_RESOLVE(off_out + b, dst_phys, dst_pm, dsy);
                            if (sp == NVKVM_GMMU_FAULT || dp == NVKVM_GMMU_FAULT) {
                                /* #13 DIAG: a CE copy DROPPED mid-stream on a translate fault —
                                 * the remaining [b, bytes) is silently never written.  #13's
                                 * root cause was exactly this class: the CeUtils VIRTUAL-dst
                                 * (512 MiB FB-alias identity map) page-table writes all faulted
                                 * here before the walker grew PD1-leaf support.  Keep the trace
                                 * to catch any other dropped-transport class early. */
                                static uint32_t dbgcd13;
                                if (dbgcd13 < 256) {
                                    dbgcd13++;
                                    qemu_log("nvkvm-gpu[%s] #13 CE-DROP client=0x%08x in=0x%llx(%s) "
                                             "out=0x%llx(%s) bytes=0x%llx b=0x%llx %s-FAULT\n",
                                             s->chip->name, s->chan_client,
                                             (unsigned long long)off_in, src_phys ? "phys" : "virt",
                                             (unsigned long long)off_out, dst_phys ? "phys" : "virt",
                                             (unsigned long long)bytes, (unsigned long long)b,
                                             (sp == NVKVM_GMMU_FAULT) ? "SRC" : "DST");
                                }
                                break;
                            }
                            uint64_t span = bytes - b;
                            uint64_t spg = 0x1000ull - (sp & 0xfffull);  /* src page tail */
                            uint64_t dpg = 0x1000ull - (dp & 0xfffull);  /* dst page tail */
                            if (span > spg) span = spg;
                            if (span > dpg) span = dpg;
                            span &= ~3ull;
                            if (span == 0) span = 4;                      /* progress guarantee */
                            /* M5.48: protect a live channel's USERD page from a CE copy dst
                             * landing on it (same hazard as the fill path above). */
                            if (!dsy && nvkvm_fb_is_live_userd(s, dp)) { b += span; continue; }
                            if (!logged0) {
                                logged0 = true;
                                qemu_log("nvkvm-gpu[%s] M5:   COPY[0] src 0x%llx(%s)=0x%08x -> "
                                  "dst 0x%llx(%s) (page-batched)\n", s->chip->name,
                                  (unsigned long long)sp, ssy?"sys":"fb",
                                  nvkvm_phys_rd32(s, sp, ssy), (unsigned long long)dp, dsy?"sys":"fb");
                            }
                            uint8_t *shp = ssy ? NULL : nvkvm_fb_host_ptr(s, sp, false);
                            uint8_t *dhp = dsy ? NULL : nvkvm_fb_host_ptr(s, dp, true);
                            if (!ssy && !dsy && dhp) {                    /* fb -> fb: bulk */
                                if (shp) { memcpy(dhp, shp, span); }
                                else     { memset(dhp, 0, span); }        /* sparse-zero src page */
                                nvkvm_dp_overlay_real_write_bytes += span;
                            } else {                                      /* a sysmem end: per-word */
                                for (uint64_t k = 0; k < span; k += 4) {
                                    nvkvm_phys_wr32(s, dp + k, dsy, nvkvm_phys_rd32(s, sp + k, ssy));
                                }
                            }
                            /* #13 THE TRIGGER: the guest kernel writes page tables via CE copies
                             * (VIRTUAL dst through the CeUtils 512 MiB FB alias); these land here
                             * through fb_host_ptr and BYPASS nvkvm_fb_write, so neither the M5.10
                             * dirty-arm nor any sweep sees them.  If this span just changed a
                             * COMPUTE VAS's page table, back the newly-mapped VA range NOW —
                             * before this push's completion release un-gates the host GR channel
                             * into the new mapping. */
                            if (!dsy) { nvkvm_m2_ce_fb_write_hook(s, dp, span); }
                            b += span;
                        }
                    }
                    if (!host_ce) { nvkvm_t_ce_emul_ns += nvkvm_now_ns() - t0ce; nvkvm_t_ce_emul_calls++; }
                    #undef NVKVM_CE_RESOLVE
                    /* NVKVM-DPLANE (cup6): per-call attribution of the bulk copy DEST.
                     * Resolve off_out's backing the SAME way the overlay resolver does
                     * (PHYSICAL FB dest -> off_out is the fb addr; for VIRTUAL we can't
                     * cheaply classify here, so tag "virt"). Answers (i): does the 64MB
                     * DEST hit real host vidmem (fbback/gpga) or a fake fb_page? */
                    {
                        nvkvm_dp_ce_launchdma_calls++;
                        nvkvm_dp_ce_bytes_total += bytes;
                        /* M5.11: snapshot the time-share buckets every 128 LAUNCH_DMA calls
                         * (decoupled from copy size, so a decode-bound phase still samples) —
                         * a killed/hung run thus still leaves periodic readings in the log. */
                        if ((nvkvm_dp_ce_launchdma_calls & 127u) == 0u) {
                            nvkvm_timeshare_dump(s, "periodic");
                        }
                        const char *verdict;
                        bool dst_fb_phys = dst_phys && (dst_pm == 0); /* PHYS + FB aperture */
                        if (dst_fb_phys) {
                            int c = nvkvm_dp_classify_fb(s, off_out);
                            if (c == 1)      { nvkvm_dp_ce_dst_fbback_hits++;     verdict = "fbback"; }
                            else if (c == 2) { nvkvm_dp_ce_dst_gpga_hits++;       verdict = "gpga";   }
                            else             { nvkvm_dp_ce_dst_fbpage_fallback++; verdict = "fbpage"; }
                        } else {
                            verdict = dst_phys ? "phys-sys" : "virt";
                        }
                        /* Rate-limit: log every call but cap total volume. */
                        static uint32_t dp_logged;
                        if (dp_logged++ < 4000) {
                            qemu_log("nvkvm-gpu[%s] NVKVM-DPLANE CE-LAUNCHDMA client=0x%08x "
                                     "off_out=0x%llx off_in=0x%llx bytes=%llu dst_phys=%d "
                                     "dst_pm=%u verdict=%s\n", s->chip->name, s->chan_client,
                                     (unsigned long long)off_out, (unsigned long long)off_in,
                                     (unsigned long long)bytes, dst_phys ? 1 : 0,
                                     dst_pm, verdict);
                        }
                        /* Emit running totals right after any big copy so cup6's 64MB
                         * shows up. Answers (ii): which counter scales with the size. */
                        if (bytes >= (1u << 20)) {
                            nvkvm_dplane_summary(s, "ce>=1MB");
                        }
                    }
                    qemu_log("nvkvm-gpu[%s] M5: CE %s in=0x%llx(%s) out=0x%llx(%s) "
                             "bytes=%llu const=0x%x [client=0x%08x gpfifo=0x%llx]\n",
                             s->chip->name,
                             mscrub ? "SCRUB" : remap ? "MEMSET" : "COPY",
                             (unsigned long long)off_in,
                             src_phys ? "phys" : "virt", (unsigned long long)off_out,
                             dst_phys ? "phys" : "virt", (unsigned long long)bytes, remapA,
                             s->chan_client, (unsigned long long)s->chan_gpfifo_va);
                    /* CE-class completion semaphore release: LAUNCH_DMA with
                     * SEMAPHORE_TYPE != NONE writes ce_sem_pay to
                     * (pbGpuVA+finishPayloadOffset).  This is what the CeUtils
                     * scrubber's channelWaitForFinishPayload polls — the fast-
                     * scrub pushbuffer ALSO emits an NVC56F SEM_EXECUTE (host
                     * sema at semaOffset), so honoring only that left this one
                     * unwritten and the scrubber timed out (ce_utils.c:349). */
                    /* #13: this launch releases a completion semaphore — the commit point of
                     * any compute-VAS page-table writes earlier in this push.  Back the
                     * dirtied VAS BEFORE the release is observable (see cpt_sync_at_release). */
                    if (sem_type != 0) { nvkvm_m2_cpt_sync_at_release(s); }
                    if (!host_ce && sem_type != 0 && ce_sem_addr) {
                        /* CE-EXEC fwd (A): when host_ce, the forwarded host CE pushbuffer's own
                         * SEM_RELEASE writes ce_sem_pay — the CPU must NOT also write it (double
                         * writer races / stale value). For all other copies (and PHYSICAL ones):
                         * M5.49b: nvkvm_chan_sem_wr32 internally forces HOST-only for the user
                         * CE-copy clients (fwd-maps the sema, skips the sim write, returns false);
                         * UVM/init scrubs still write+return true here. */
                        uint64_t redir = 0;                    /* M5.18: also write the BAR1 page libcuda polls */
                        if (nvkvm_chan_sem_wr32(s, ce_sem_addr, ce_sem_pay, &redir)) {
                            s->chan_sem_released = true;
                            qemu_log("nvkvm-gpu[%s] M5: CE_SEM_RELEASE addr=0x%llx "
                                     "payload=%u redir=0x%llx\n", s->chip->name,
                                     (unsigned long long)ce_sem_addr, ce_sem_pay,
                                     (unsigned long long)redir);
                        }
                    }
                    break;
                }
                /* NVC56F host-channel semaphore methods. */
                case 0x5c: sem_addr = (sem_addr & ~0xFFFFFFFFull) | (d & 0xFFFFFFFCu); break; /* SEM_ADDR_LO[31:2] */
                case 0x60: sem_addr = (sem_addr & 0xFFFFFFFFull) | ((uint64_t)d << 32); break;/* SEM_ADDR_HI */
                case 0x64: sem_pay_lo = d; break;                                            /* SEM_PAYLOAD_LO */
                case 0x68: sem_pay_hi = d; break;                                            /* SEM_PAYLOAD_HI */
                case 0x6c: {                                                                 /* SEM_EXECUTE */
                    if ((d & 0x7u) == 0x1u && sem_addr) {   /* OPERATION == RELEASE */
                        /* #13: commit point — back any dirtied compute VAS first. */
                        nvkvm_m2_cpt_sync_at_release(s);
                        bool sz64 = (d >> 24) & 1;           /* PAYLOAD_SIZE: 0=16B(64-bit val), 1=4B */
                        uint64_t redir = 0;                  /* M5.18: also write the BAR1 page libcuda polls */
                        /* M5.49b: wr32 returns false for the host-only user CE-copy path
                         * (host writes it) -> the high-word + sim-log below are skipped too. */
                        if (nvkvm_chan_sem_wr32(s, sem_addr, sem_pay_lo, &redir)) {
                            if (!sz64) {                     /* 64-bit value: high word too */
                                bool sy2 = false;
                                uint64_t p2 = nvkvm_chan_translate(s, sem_addr + 4, &sy2);
                                if (p2 != NVKVM_GMMU_FAULT) { nvkvm_phys_wr32(s, p2, sy2, sem_pay_hi); }
                                if (redir) { nvkvm_fb_write(s, redir + 4, sem_pay_hi, 4); }
                            }
                            s->chan_sem_released = true;
                            qemu_log("nvkvm-gpu[%s] M5: SEM_RELEASE addr=0x%llx "
                                     "payload=%u redir=0x%llx\n", s->chip->name,
                                     (unsigned long long)sem_addr, sem_pay_lo,
                                     (unsigned long long)redir);
                        }
                    }
                    break;
                }
                /* COMPUTE/3D-class (NVC7C0+) report-semaphore release. */
                case 0x1b00: cr_sem_addr = (cr_sem_addr & 0xFFFFFFFFull) | ((uint64_t)(d & 0xFFu) << 32); break; /* ADDR_UPPER */
                case 0x1b04: cr_sem_addr = (cr_sem_addr & ~0xFFFFFFFFull) | d; break;                          /* ADDR_LOWER */
                case 0x1b08: cr_sem_pay = d; break;                                                            /* PAYLOAD */
                case 0x1b0c: {                                                                                 /* D: trigger */
                    if ((d & 0x3u) == 0x0u && cr_sem_addr) {   /* OPERATION == RELEASE */
                        bool one_word = (d >> 28) & 1;         /* STRUCTURE_SIZE: 1=ONE_WORD(4B), 0=FOUR_WORDS(16B w/ ts) */
                        uint64_t redir = 0;                    /* M5.18: also write the BAR1 page libcuda polls */
                        bool ok = nvkvm_chan_sem_wr32(s, cr_sem_addr, cr_sem_pay, &redir);
                        if (ok) {
                            if (!one_word) {                   /* 4-word: also zero the 12B timestamp */
                                bool sy2 = false;
                                uint64_t p2 = nvkvm_chan_translate(s, cr_sem_addr + 4, &sy2);
                                if (p2 != NVKVM_GMMU_FAULT) {
                                    nvkvm_phys_wr32(s, p2, sy2, 0);
                                    nvkvm_phys_wr32(s, p2 + 4, sy2, 0);
                                    nvkvm_phys_wr32(s, p2 + 8, sy2, 0);
                                }
                                if (redir) { nvkvm_fb_write(s, redir + 4, 0, 4);
                                             nvkvm_fb_write(s, redir + 8, 0, 4);
                                             nvkvm_fb_write(s, redir + 12, 0, 4); }
                            }
                            s->chan_sem_released = true;
                            qemu_log("nvkvm-gpu[%s] M5: COMPUTE_REPORT_SEM addr=0x%llx "
                                     "payload=%u redir=0x%llx awaken=%d\n", s->chip->name,
                                     (unsigned long long)cr_sem_addr, cr_sem_pay,
                                     (unsigned long long)redir, (int)((d >> 20) & 1));
                        }
                    }
                    break;
                }
                default: break;
                }
            }
        }
    }
    s->chan_gp_get = gp_put;
}

static uint64_t nvkvm_bar2_read_inner(void *opaque, hwaddr off, unsigned size)
{
    NvkvmGpuEmul *s = opaque;
    /* PHYSICAL mode (or not yet bound) = identity FB access; VIRTUAL = GMMU walk.
     * During bootstrap the driver accesses the BAR2 page tables via BAR2-physical
     * before binding it virtual, so identity must work then. */
    uint64_t pa = s->bar2_virtual ? nvkvm_bar2_translate(s, off) : off;
    if (pa == NVKVM_GMMU_FAULT) {
        return 0;
    }
    return nvkvm_fb_read(s, pa, size);
}

static void nvkvm_bar2_write_inner(void *opaque, hwaddr off, uint64_t val,
                             unsigned size)
{
    NvkvmGpuEmul *s = opaque;
    uint64_t pa = s->bar2_virtual ? nvkvm_bar2_translate(s, off) : off;
    if (pa == NVKVM_GMMU_FAULT) {
        return;
    }
    nvkvm_fb_write(s, pa, val, size);
}

/* #90: PCI BAR3 == RM "BAR2", the 32 MiB GPU-virtual GMMU window. */
static uint64_t nvkvm_bar2_read(void *opaque, hwaddr off, unsigned size)
{
    uint64_t val = nvkvm_bar2_read_inner(opaque, off, size);
    nvkvm_rec_mmio_rd(3, NVKVM_REC_M_BAR2, off, size, val);
    return val;
}

static void nvkvm_bar2_write(void *opaque, hwaddr off, uint64_t val,
                             unsigned size)
{
    nvkvm_rec_mmio_wr(3, NVKVM_REC_M_BAR2, off, size, val);
    nvkvm_bar2_write_inner(opaque, off, val, size);
}

static const MemoryRegionOps nvkvm_bar2_ops = {
    .read       = nvkvm_bar2_read,
    .write      = nvkvm_bar2_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid      = { .min_access_size = 1, .max_access_size = 8 },
};

/* ── realize / unrealize ───────────────────────────────────────────────────*/

/* M5.1: lazily stand up the per-guest host isolate + open the real GPU control
 * and device nodes. Persists for the guest's lifetime (the host-side context the
 * forwarded RM stream builds on). Returns true once ready; on failure disables
 * forwarding cleanly so the guest keeps running on the faked path. */
/* M6.1 (item-4 step 2): share the guest-RAM memfd into the stub and MAP_FIXED it at a
 * reserved stub VA window, so the host nvidia driver (stub process) can address any guest
 * GPA — the prerequisite for OS_DESCRIPTOR-registering the guest's sysmem GR buffers (item-4
 * step 3) so the host GPU DMA-fills them. Reuses the Mode-1 handle_table + send_handle +
 * isolate_mmap (the user's foreseen handle-reuse, [[mode2-per-proc-isolate-handle-reuse]]).
 * One-shot; idempotent. */
#define NVKVM_M2_STUB_RAMWIN 0x7e0000000000ull   /* 126 TiB: free in the fresh stub process */
static bool nvkvm_m2_share_guest_ram(NvkvmGpuEmul *s)
{
    if (s->m2_ram_shared) {
        return true;
    }
    if (s->m2_guest_ram_fd < 0 || s->m2_guest_ram_size == 0) {
        qemu_log("nvkvm-gpu[%s] M6.1 share guest-RAM: no memfd (fd=%d) — need "
                 "memory-backend-memfd,share=on\n", s->chip->name, s->m2_guest_ram_fd);
        return false;
    }
    /* The stub keys handles in ONE id space shared with the isolate's device handles
     * (ctl=1, gpu=2) and the data-plane map fds (m2_maph_next, 16+). Seed a high id so the
     * guest-RAM handle can't collide (id 1 clobbered the stub's /dev/nvidiactl -> ENOTTY). */
    s->m2_ht.next_id = 0x8000u;
    uint32_t hid = 0;
    if (nvkvm_handle_alloc_pending(&s->m2_ht, 1 /*session*/, 0, &hid) != 0) {
        qemu_log("nvkvm-gpu[%s] M6.1 share guest-RAM: alloc_pending failed\n", s->chip->name);
        return false;
    }
    if (nvkvm_handle_attach_fd(&s->m2_ht, hid, s->m2_guest_ram_fd) != 0) {
        nvkvm_handle_abort_open(&s->m2_ht, hid);
        qemu_log("nvkvm-gpu[%s] M6.1 share guest-RAM: attach_fd failed\n", s->chip->name);
        return false;
    }
    int sr = nvkvm_isolate_send_handle(&s->m2_iso, &s->m2_ht, s->m2_iso_id, hid);
    int mr = nvkvm_isolate_mmap(&s->m2_iso, s->m2_iso_id, hid, NVKVM_M2_STUB_RAMWIN,
                                s->m2_guest_ram_size, 0, PROT_READ | PROT_WRITE, MAP_SHARED);
    s->m2_guest_ram_handle = hid;
    s->m2_stub_ram_base    = NVKVM_M2_STUB_RAMWIN;
    s->m2_ram_shared       = (sr == 0 && mr == 0);
    qemu_log("nvkvm-gpu[%s] M6.1 share guest-RAM: handle=%u send=%d mmap=%d stub_base=0x%llx "
             "size=0x%llx -> %s\n", s->chip->name, hid, sr, mr,
             (unsigned long long)NVKVM_M2_STUB_RAMWIN,
             (unsigned long long)s->m2_guest_ram_size,
             s->m2_ram_shared ? "SHARED (stub can address guest RAM)" : "FAILED");
    return s->m2_ram_shared;
}

static bool nvkvm_m2_iso_ensure(NvkvmGpuEmul *s)
{
    if (s->m2_iso_ready) {
        return true;
    }
    nvkvm_isolate_table_init(&s->m2_iso);
    s->m2_iso.abi_profile = NVKVM_ABI_580;   /* host is 580.159.04 */
    uint32_t id = 0;
    if (nvkvm_isolate_create(&s->m2_iso, 1 /*session*/, NULL, &id) != 0 || id == 0) {
        qemu_log("nvkvm-gpu[%s] M5.1: isolate_create FAILED — forwarding OFF\n",
                 s->chip->name);
        s->m2fwd = false;
        return false;
    }
    int ctlfd = -1, gpufd = -1;
    int r1 = nvkvm_isolate_open_device(&s->m2_iso, id, 1, NVKVM_DEV_CTL,    O_RDWR, &ctlfd);
    int r2 = nvkvm_isolate_open_device(&s->m2_iso, id, 2, NVKVM_DEV_GPU(0), O_RDWR, &gpufd);
    s->m2_gpu_fd = gpufd;   /* the stub's /dev/nvidia0 fd in QEMU's process (SCM_RIGHTS) */
    if (r1 != 0 || r2 != 0) {
        qemu_log("nvkvm-gpu[%s] M5.1: open ctl/gpu FAILED r1=%d r2=%d — forwarding OFF\n",
                 s->chip->name, r1, r2);
        nvkvm_isolate_kill(&s->m2_iso, id);
        s->m2fwd = false;
        return false;
    }
    s->m2_iso_id = id; s->m2_ctl_h = 1; s->m2_gpu_h = 2; s->m2_iso_ready = true;
    s->m2_cur_cvas = -1;        /* M5.28: no per-channel VAS active until the doorbell loop sets it */
    qemu_log("nvkvm-gpu[%s] M5.1: host isolate %u ready (pid=%d, ctl+gpu0 open)\n",
             s->chip->name, id, (int)nvkvm_isolate_host_pid(&s->m2_iso, id));
    /* M6.1 (item-4 step 2): share guest RAM into the stub so it can OS_DESCRIPTOR guest GPAs. */
    nvkvm_m2_share_guest_ram(s);
    return true;
}

/* M5.1b: map a guest RM client handle -> a host (synthetic, non-colliding)
 * handle, minting a fresh 0xdeadNNNN on first sight. Host clients live in
 * 0xc1xxxxxx, so 0xdeadNNNN never collides. */
static uint32_t nvkvm_m2_client(NvkvmGpuEmul *s, uint32_t g)
{
    for (int i = 0; i < s->m2_cmap_n; i++) {
        if (s->m2_cmap[i].g == g) {
            return s->m2_cmap[i].h;
        }
    }
    if (s->m2_cmap_n >= (int)(sizeof(s->m2_cmap) / sizeof(s->m2_cmap[0]))) {
        return g;                        /* table full -> verbatim (may collide) */
    }
    uint32_t h = 0xdead0001u + (s->m2_cmap_next++ & 0xffffu);  /* P0: monotonic (reap-safe) */
    s->m2_cmap[s->m2_cmap_n].g = g;
    s->m2_cmap[s->m2_cmap_n].h = h;
    s->m2_cmap_n++;
    return h;
}
static bool nvkvm_m2_client_known(NvkvmGpuEmul *s, uint32_t g)
{
    for (int i = 0; i < s->m2_cmap_n; i++) {
        if (s->m2_cmap[i].g == g) {
            return true;
        }
    }
    return false;
}

/* M5.1a SHADOW-forward: replay the guest's RM alloc on the real host GPU in
 * PARALLEL — the guest still proceeds on the faked GSP response, so this is
 * non-disruptive. It validates the real alloc stream forwards and reveals where
 * the two-RM GPU-phys reconciliation first breaks (expected at the channel alloc,
 * whose instanceMem.base is the guest CPU-RM PMA's FB offset, meaningless to the
 * host RM's PMA). GSP_RM_ALLOC body @cmd: hClient@80,hParent@84,hObject@88,
 * hClass@92,paramsSize@100,params@112. -> NV_ESC_RM_ALLOC (NVOS64), params as aux. */
static void nvkvm_m2_shadow_fwd(NvkvmGpuEmul *s, const uint8_t *cmd, uint32_t fn)
{
    if (fn != 103 && fn != 10) {
        return;                          /* allocs (103) + frees (10) */
    }
    if (!nvkvm_m2_iso_ensure(s)) {
        return;
    }
    /* M5.1c: forward FREE so host objects/channels don't accumulate (the
     * un-freed channels exhausted the host's channel-ID heap). */
    if (fn == 10) {
        uint32_t fClient = ldl_le_p(cmd + 80), fParent = ldl_le_p(cmd + 84);
        uint32_t fObj = ldl_le_p(cmd + 88);
        struct nvos00_parameters f;
        memset(&f, 0, sizeof(f));
        f.h_root = nvkvm_m2_client(s, fClient);
        f.h_object_parent = nvkvm_m2_client_known(s, fParent) ? nvkvm_m2_client(s, fParent)
                                                              : fParent;
        f.h_object_old = nvkvm_m2_client_known(s, fObj) ? nvkvm_m2_client(s, fObj) : fObj;
        unsigned int fc = (3u << 30) | ((unsigned int)sizeof(f) << 16) |
                          ((unsigned int)'F' << 8) | NV_ESC_RM_FREE;
        uint32_t fst = 0; uint64_t ff = 0;
        nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, fc,
                            &f, sizeof(f), NULL, 0, 0, &fst, &ff);
        return;
    }
    static uint8_t auxbuf[16384];
    uint32_t hClient = ldl_le_p(cmd + 80), hParent = ldl_le_p(cmd + 84);
    uint32_t hObject = ldl_le_p(cmd + 88), hClass = ldl_le_p(cmd + 92);
    uint32_t psize   = ldl_le_p(cmd + 100);
    if (psize > sizeof(auxbuf)) {
        psize = sizeof(auxbuf);
    }
    memcpy(auxbuf, cmd + 112, psize);
    /* M5.3 DIAG: compare libcuda's working VASpace (0xcaf00005) vs the UVM RM-internal
     * one (0x5c000007) that resolves to a NULL OBJVASPACE on the host. NV_VASPACE_
     * ALLOCATION_PARAMETERS: index@0, flags@4, vaSize@8(u64), vaStartInternal@16(u64),
     * vaLimitInternal@24(u64), bigPageSize@32, vaBase@40(u64). */
    if (hClass == 0x90f1u && psize >= 4) {
        char hex[256]; int n = (int)(psize < 48 ? psize : 48); int o = 0;
        for (int i = 0; i + 4 <= n; i += 4) {
            o += snprintf(hex + o, sizeof(hex) - o, "%s@%d=0x%08x",
                          i ? " " : "", i, ldl_le_p(auxbuf + i));
        }
        qemu_log("nvkvm-gpu[%s] M5.3 DIAG 90f1 VAS obj=0x%08x client=0x%08x "
                 "psize=%u: %s\n", s->chip->name, hObject, hClient, psize, hex);
    }
    /* M5.3 EXPERIMENT: the UVM GR VASpace is forwarded with flags IS_EXTERNALLY_OWNED
     * (BIT3) | ENABLE_PAGE_FAULTING (BIT6) = 0x48, whose page tables the guest UVM
     * manages — unforwardable, so the host gets an unmanaged shell (NULL OBJVASPACE)
     * -> ctxshare INVALID_STATE. Hypothesis: the host doesn't need UVM management of
     * THIS vaspace; it only needs a functional RM-managed VASpace to build its shadow
     * GR context (buffer CONTENTS are matched later via the data plane). So strip
     * EXTERNALLY_OWNED + PAGE_FAULTING (0x48) on the host copy → host RM owns the page
     * tables → real OBJVASPACE → ctxshare/channel/compute can construct. */
    if (hClass == 0x90f1u && psize >= 8) {
        uint32_t vflags = ldl_le_p(auxbuf + 4);
        if (vflags & 0x48u) {
            stl_le_p(auxbuf + 4, vflags & ~0x48u);
            qemu_log("nvkvm-gpu[%s] M5.3 90f1 strip EXT_OWNED|PAGE_FAULT flags 0x%x->0x%x\n",
                     s->chip->name, vflags, vflags & ~0x48u);
        }
    }
    /* M5.3 data-plane: remember the NV20_SUBDEVICE_0 (0x2080) handle per GR client —
     * GR_GET_CTX_BUFFER_INFO is issued on the subdevice to enumerate the host shadow
     * context's real buffers (the data to mirror into the guest's BAR-backed buffers). */
    if (hClass == 0x2080u && s->m2_subdev_n < (int)ARRAY_SIZE(s->m2_subdev)) {
        s->m2_subdev[s->m2_subdev_n].client = hClient;
        s->m2_subdev[s->m2_subdev_n].subdev = hObject;
        s->m2_subdev_n++;
    }
    /* M5.3: remember each FERMI_VASPACE_A (0x90f1) forwarded under a (client,device)
     * so the GR channelgroup can be given an explicit hVASpace below. */
    if (hClass == 0x90f1u && s->m2_devvas_n < 32) {
        s->m2_devvas[s->m2_devvas_n].client = hClient;
        s->m2_devvas[s->m2_devvas_n].dev    = hParent;   /* VASpace parent = device */
        s->m2_devvas[s->m2_devvas_n].vas    = hObject;
        s->m2_devvas_n++;
    }
    /* M5.3 FIX: the GR-engine channelgroup (KEPLER_CHANNEL_GROUP_A 0xa06c,
     * engineType@12 == NV2080_ENGINE_TYPE_GRAPHICS=1) leaves hVASpace@8 == 0
     * (device-default), which fails NV_ERR_INVALID_OBJECT_HANDLE (0x33) on the
     * forwarded host device (no default VAS). The COPY-engine TSGs pass an explicit
     * handle and construct fine. Substitute the first VASpace forwarded under the
     * same (client,device) so the GR TSG — the compute object's parent chain — can
     * construct and the host RM self-promotes its GR context.
     * NV_CHANNEL_GROUP_ALLOCATION_PARAMS: hObjectError@0,hObjectEccError@4,
     * hVASpace@8, engineType@12. */
    if (hClass == 0xa06cu && psize >= 16) {
        uint32_t cur_vas = ldl_le_p(auxbuf + 8);
        uint32_t engine  = ldl_le_p(auxbuf + 12);
        uint32_t sub = 0;
        for (int i = 0; i < s->m2_devvas_n; i++) {
            if (s->m2_devvas[i].client == hClient && s->m2_devvas[i].dev == hParent) {
                sub = s->m2_devvas[i].vas;
                break;                       /* first VASpace under this device */
            }
        }
        /* M5.28 PER-CHANNEL VAS: a GR/compute TSG (engineType GRAPHICS=1) gets a FRESH
         * nvkvm-owned VAS, ALWAYS — replacing whatever VAS libcuda passed (the compute TSGs
         * reference the guest's forwarded VAS explicitly, the one the host RM self-promotes GR
         * ctx into -> st=0x51 collisions / Xid 32). cvas is keyed by the TSG handle (hObject);
         * the ctxshare + channel below inherit/reference it. Other engine TSGs keep the legacy
         * forwarded VAS — NOTE: giving COPY engines (0x9..0x12) their OWN fresh VAS via
         * cvas_get was tried and HUNG the guest (each COPY TSG got a NEW empty VAS, redirecting
         * the copy channels off the main guest VAS 0xcaf00005; faulted -> PMC_BOOT_0 reset
         * spin). The M5.40 branch below is DIFFERENT: it binds the compute client's COPY TSG
         * to the GR TSG's EXISTING, ALREADY-POPULATED fvas (the unified CUDA VAS mirror) —
         * never allocating a new VAS — so the COPY channels see the same populated address
         * space the GR channel runs in (fixes GPFIFO_SCHEDULE st=0x57 / 0x2002xxxxx faults). */
        if (engine == 1u) {
            int ci = nvkvm_m2_cvas_get(s, hClient, hObject);
            if (ci >= 0) {
                stl_le_p(auxbuf + 8, s->m2_cvas[ci].fvas);
                qemu_log("nvkvm-gpu[%s] M5.28 a06c GR TSG hVASpace 0x%08x -> 0x%08x "
                         "[per-chan fresh] (engineType=%u tsg=0x%08x)\n", s->chip->name,
                         cur_vas, s->m2_cvas[ci].fvas, engine, hObject);
            } else if (cur_vas == 0u && sub) {
                stl_le_p(auxbuf + 8, sub);
                qemu_log("nvkvm-gpu[%s] M5.28 a06c GR TSG cvas alloc FAILED; fallback "
                         "forwarded VAS 0x%08x (engineType=%u)\n", s->chip->name, sub, engine);
            } else {
                qemu_log("nvkvm-gpu[%s] M5.28 a06c GR TSG cvas FAILED + no fallback "
                         "(cur_vas=0x%08x client=0x%08x)\n", s->chip->name, cur_vas, hClient);
            }
        } else if (engine >= 0x9u && engine <= 0x12u &&  /* NV2080_ENGINE_TYPE_IS_COPY */
                   hClient != 0xc1d00001u &&             /* not the guest-RM CeUtils client */
                   ((hObject & 0xffff0000u) != 0xbaba0000u) &&
                   ((hObject & 0xffffff00u) != 0x31415900u) &&
                   s->m2_gr_tsg) {
            /* M5.40 SHARED-VAS COPY TSG: the compute client's COPY TSG must run in the SAME
             * populated VAS as its GR TSG (libcuda's unified CUDA VAS, here mirrored into the
             * GR fvas). Rewrite hVASpace@8 UNCONDITIONALLY (libcuda may pass 0 OR the explicit
             * guest VAS handle) to the GR cvas entry's fvas, and register a cvas entry keyed
             * by THIS TSG so the doorbell loop routes/populates the COPY working set into the
             * shared fvas (va_seen dedup makes the re-populate idempotent). Do NOT cvas_get
             * (fresh VAS) here — that was the PMC_BOOT_0 hang. */
            int gi = -1;
            for (int i = 0; i < s->m2_cvas_n; i++) {
                if (s->m2_cvas[i].client == hClient && s->m2_cvas[i].tsg == s->m2_gr_tsg) {
                    gi = i; break;               /* the GR TSG's entry (preferred) */
                }
            }
            if (gi < 0) {
                for (int i = 0; i < s->m2_cvas_n; i++) {
                    if (s->m2_cvas[i].client == hClient) { gi = i; break; }
                }
            }
            int dup = -1;
            for (int i = 0; i < s->m2_cvas_n; i++) {
                if (s->m2_cvas[i].client == hClient && s->m2_cvas[i].tsg == hObject) {
                    dup = i; break;              /* already registered (replayed alloc) */
                }
            }
            if (gi >= 0 && (dup >= 0 || s->m2_cvas_n < (int)ARRAY_SIZE(s->m2_cvas))) {
                stl_le_p(auxbuf + 8, s->m2_cvas[gi].fvas);
                if (dup < 0) {
                    int idx = s->m2_cvas_n++;
                    s->m2_cvas[idx].client    = hClient;
                    s->m2_cvas[idx].tsg       = hObject;          /* the COPY TSG */
                    s->m2_cvas[idx].hdev      = s->m2_cvas[gi].hdev;
                    s->m2_cvas[idx].fvas      = s->m2_cvas[gi].fvas;
                    s->m2_cvas[idx].fvirt     = s->m2_cvas[gi].fvirt;
                    s->m2_cvas[idx].populated = false;  /* doorbell populates COPY working set */
                }
                qemu_log("nvkvm-gpu[%s] M5.40 a06c COPY TSG hVASpace 0x%08x -> 0x%08x "
                         "[GR-shared fvas, gr_tsg=0x%08x] (engineType=0x%x tsg=0x%08x "
                         "client=0x%08x)\n", s->chip->name, cur_vas, s->m2_cvas[gi].fvas,
                         s->m2_gr_tsg, engine, hObject, hClient);
            } else if (cur_vas == 0u && sub) {
                stl_le_p(auxbuf + 8, sub);
                qemu_log("nvkvm-gpu[%s] M5.40 a06c COPY TSG: no GR cvas for client 0x%08x; "
                         "legacy fallback forwarded VAS 0x%08x (engineType=0x%x)\n",
                         s->chip->name, hClient, sub, engine);
            } else {
                qemu_log("nvkvm-gpu[%s] M5.40 a06c COPY TSG: no GR cvas + no fallback "
                         "(cur_vas=0x%08x client=0x%08x cvas_n=%d)\n", s->chip->name,
                         cur_vas, hClient, s->m2_cvas_n);
            }
        } else if (cur_vas == 0u && sub) {
            stl_le_p(auxbuf + 8, sub);
            qemu_log("nvkvm-gpu[%s] M5.3 a06c non-GR TSG hVASpace 0 -> 0x%08x "
                     "(engineType=%u)\n", s->chip->name, sub, engine);
        }
    }
    /* M5.3: record TSG (0xa06c) handle -> engineType@12 for the channel engineType fix. */
    if (hClass == 0xa06cu && psize >= 16 &&
        s->m2_tsgeng_n < (int)ARRAY_SIZE(s->m2_tsgeng)) {
        s->m2_tsgeng[s->m2_tsgeng_n].tsg    = hObject;
        s->m2_tsgeng[s->m2_tsgeng_n].engine = ldl_le_p(auxbuf + 12);
        s->m2_tsgeng[s->m2_tsgeng_n].client = hClient;   /* P0: reap key only */
        s->m2_tsgeng_n++;
    }
    /* M5.3: FERMI_CONTEXT_SHARE_A (0x9067) NV_CTXSHARE_ALLOCATION_PARAMETERS has
     * hVASpace@0. The GR context share (under the GR TSG) leaves it 0 (device
     * default) → NV_ERR_INVALID_STATE (0x40) on the host, and it must match the
     * TSG/channel VASpace. Substitute the same first-VASpace-for-client. */
    if (hClass == 0x9067u && psize >= 12) {
        uint32_t cvas = ldl_le_p(auxbuf), cfl = ldl_le_p(auxbuf + 4),
                 csub = ldl_le_p(auxbuf + 8);
        uint32_t sub = 0;
        for (int i = 0; i < s->m2_devvas_n; i++) {
            if (s->m2_devvas[i].client == hClient) { sub = s->m2_devvas[i].vas; break; }
        }
        /* M5.28: the ctxshare is parented to the GR TSG (hParent). If that TSG was given a
         * per-channel fresh VAS above, the ctxshare's hVASpace@0 must reference the SAME
         * fresh VAS (RM requires the share's VAS == the TSG's VAS), else the host channel
         * runs in a different VAS than the one we populate. */
        int ci = -1;
        for (int i = 0; i < s->m2_cvas_n; i++) {
            if (s->m2_cvas[i].client == hClient && s->m2_cvas[i].tsg == hParent) { ci = i; break; }
        }
        if (ci >= 0) {
            stl_le_p(auxbuf, s->m2_cvas[ci].fvas);
            qemu_log("nvkvm-gpu[%s] M5.28 9067 ctxshare hVASpace@0 0x%08x -> 0x%08x "
                     "[per-chan fresh, tsg=0x%08x]\n", s->chip->name, cvas,
                     s->m2_cvas[ci].fvas, hParent);
        } else {
            qemu_log("nvkvm-gpu[%s] M5.3 DIAG 9067 ctxshare hVASpace@0=0x%08x flags@4=0x%x "
                     "subctxId@8=0x%x hClient=0x%08x trackedVAS=0x%08x (devvas_n=%d cvas=%d)\n",
                     s->chip->name, cvas, cfl, csub, hClient, sub, s->m2_devvas_n, ci);
        }
    }
    /* M5.1c experiment: for channel classes, drop hObjectError (params+0) — its
     * error-notifier memory object isn't forwarded yet, so RM's notifier lookup
     * fails (kchannelGetNotifierInfo OBJECT_NOT_FOUND). Zeroing it lets the
     * channel construct without a notifier; revisit when memory objects forward. */
    if ((hClass == 0xc56fu || hClass == 0xc36fu) && psize >= 4) {
        stl_le_p(auxbuf, 0u);                        /* hObjectError = 0 */
        /* hUserdMemory[0] @ params+32: "ignored if 0" -> the host CPU-RM allocates
         * USERD itself (kernel_channel.c:309); instance memory is RM-allocated on
         * the normal host-RM path too. So zeroing the client-USERD handle lets the
         * channel fully construct with RM-managed memory (M5.3a).
         * M5.4: for the GR channel (the one whose completion poll hangs cuCtxCreate,
         * CRASHWIN fb=0x420208c), instead BACK its USERD with real host GPU memory
         * (double-mmap) so the host GPU's GP_GET is visible to the guest's poll.
         * Identify it by its parent TSG's engine (GRAPHICS=1). Other channels keep
         * hUserdMemory[0]=0 (host RM allocates) until they're backed too. */
        if (psize >= 36) {
            stl_le_p(auxbuf + 32, 0u);               /* default: host RM allocates USERD */
            /* M5.4: back the GR channel's USERD with real host GPU memory (double-mmap).
             * Identify it by parent TSG engine (GRAPHICS=1). Backing ALL channels was
             * tried (incl. sentinel-handle probe channels 0xbaba0045/0x31415900) and
             * introduced status=0x51/0x33 errors on those probe channels, so restrict to
             * the GR channel (the cuCtxCreate context) — proven clean. The libcuda COPY
             * channels keep RM-allocated USERD. NOTE: USERD-backing alone does NOT clear
             * the cuCtxCreate hang — the dominant wait (CRASHWIN fb=0x2efbaf000, 331x,
             * PRAMIN-accessed gva=0) is a non-USERD FB semaphore the host channel must
             * EXECUTE to write (M5.4 steps 2-3: GPFIFO+pushbuffer double-mmap + doorbell). */
            bool is_gr = false;
            for (int i = 0; i < s->m2_tsgeng_n; i++) {
                if (s->m2_tsgeng[i].tsg == hParent && s->m2_tsgeng[i].engine == 1u) {
                    is_gr = true; break;
                }
            }
            /* M5.23 USERD double-mmap (host-channel bridge step 1): back the USERD of
             * EVERY real forwarded channel with host GPU memory + double-mmap at the
             * guest USERD FB addr, so the guest's GP_PUT (userd+0x8C) lands where the
             * host GPU reads GP_PUT/GP_GET — the prerequisite for the rung host channel
             * to actually run.  EXCLUDE libcuda PROBE/sentinel channels (0xbaba.. /
             * 0x31415..): M5.4 proved backing those returns 0x51/0x33 (they aren't real
             * runnable channels).  No-copy: the double-mmap shares the page, not a copy. */
            bool is_sentinel = ((hObject & 0xffff0000u) == 0xbaba0000u) ||
                               ((hObject & 0xffffff00u) == 0x31415900u);
            if (!is_sentinel) {
                nvkvm_m2_back_channel_userd(s, hClient, hObject, auxbuf, psize);
            }
            if (is_gr) {
                s->m2_gr_channel = hObject;  /* M5.8: track for work-submit-token */
                s->m2_gr_tsg     = hParent;  /* M5.8: GR TSG (for GPFIFO_SCHEDULE) */
            }
        }
        /* M5.3: NV_CHANNEL_ALLOC_PARAMS hVASpace@28 (alloc_channel.h). Like the GR
         * channelgroup, the GR channel leaves it 0 (device default) which won't
         * resolve on the forwarded host device -> OBJECT_NOT_FOUND. Substitute the
         * VASpace forwarded under the same (client,device-of-the-channelgroup). The
         * channel's hParent is the TSG, so look up by the TSG's device — track via
         * the a06c we already saw. Simplest: substitute the first VASpace tracked for
         * this client (the GR VAS). Also dump for diagnosis. */
        if (psize >= 64) {
            uint32_t hctxshare = ldl_le_p(auxbuf + 24);
            uint32_t hvas      = ldl_le_p(auxbuf + 28);
            qemu_log("nvkvm-gpu[%s] M5.3 DIAG c56f obj=0x%08x hParent=0x%08x "
                     "hContextShare@24=0x%08x hVASpace@28=0x%08x gpFifoOff@8=0x%llx "
                     "hUserd[0]@32=0x%08x userdOffset[0]@64=0x%llx psize=%u\n", s->chip->name,
                     hObject, hParent, hctxshare, hvas,
                     (unsigned long long)ldq_le_p(auxbuf + 8), ldl_le_p(auxbuf + 32),
                     (unsigned long long)ldq_le_p(auxbuf + 64), psize);
            /* memory descriptors region (NV_MEMORY_DESC_PARAMS @144/168/192/216 for
             * instanceMem/userdMem/ramfcMem/mthdbufMem; base@+0,addrSpace@+16) — the
             * suspected two-RM reconciliation point (guest-FB bases). Dump u64s. */
            if (psize >= 240) {
                qemu_log("nvkvm-gpu[%s] M5.3 DIAG c56f memdescs: inst.base@144=0x%llx "
                         "as@160=0x%x userd.base@168=0x%llx ramfc.base@192=0x%llx "
                         "mthd.base@216=0x%llx engineType@128=0x%x\n", s->chip->name,
                         (unsigned long long)ldq_le_p(auxbuf + 144), ldl_le_p(auxbuf + 160),
                         (unsigned long long)ldq_le_p(auxbuf + 168),
                         (unsigned long long)ldq_le_p(auxbuf + 192),
                         (unsigned long long)ldq_le_p(auxbuf + 216), ldl_le_p(auxbuf + 128));
            }
            /* NOTE: do NOT substitute hVASpace for TSG channels (parented to a 0xa06c
             * group); the host RM rejects any explicit vaspace on a TSG channel ("TSG
             * channels can't use an explicit vaspace", kernel_channel.c) — they inherit
             * the TSG's vaspace. (void to silence unused.) */
            (void)hctxshare;
            /* M5.50 (REVERTED — see mode2_dataplane_architecture.md Addendum 2026-06-12c):
             * substituting a fresh cvas into the CE-copy client's BARE channel hVASpace@28
             * moved its alloc 0x33->0x1f (still fails — the whole separate-client bare-channel
             * stack needs forwarding) AND broke the M5.49b USER-CE identification (grmapper no
             * longer hit the FRESH-VAS fallback), so cup2 passed via simulation, not host-only.
             * The CE-copy host-only path is an orthogonal detour; pivoted to matmul (GR compute
             * channels already construct host-side). hVASpace stays 0 for bare channels here. */
            /* M5.3: NV_CHANNEL_ALLOC_PARAMS engineType@128. The GR channel passes 0
             * (NULL/inherit); on the host give it the parent TSG's engine explicitly. */
            if (psize >= 132 && ldl_le_p(auxbuf + 128) == 0u) {
                for (int i = 0; i < s->m2_tsgeng_n; i++) {
                    if (s->m2_tsgeng[i].tsg == hParent && s->m2_tsgeng[i].engine != 0) {
                        stl_le_p(auxbuf + 128, s->m2_tsgeng[i].engine);
                        qemu_log("nvkvm-gpu[%s] M5.3 c56f engineType 0 -> 0x%x "
                                 "(from TSG 0x%08x)\n", s->chip->name,
                                 s->m2_tsgeng[i].engine, hParent);
                        break;
                    }
                }
            }
        }
    }
    struct nvos64_parameters p;
    memset(&p, 0, sizeof(p));
    /* M5.1b: translate client refs. h_root is always the owning client; register
     * + remap it. h_object_parent is a client for device allocs (== hClient) but
     * an object for deeper allocs — translate only if it's a known client. */
    uint32_t h_root = nvkvm_m2_client(s, hClient);
    uint32_t h_parent = nvkvm_m2_client_known(s, hParent) ? nvkvm_m2_client(s, hParent)
                                                          : hParent;
    p.h_root = h_root; p.h_object_parent = h_parent; p.h_object_new = hObject;
    p.h_class = hClass; p.alloc_parms_size = psize;
    unsigned int ic = (3u << 30) | ((unsigned int)sizeof(p) << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_ALLOC;
    uint32_t nvstatus = 0xdeadbeefu;
    uint64_t fault = 0;
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, ic,
                                 &p, sizeof(p), auxbuf, psize, 0, &nvstatus, &fault);
    s->m2_fwd_n++;
    qemu_log("nvkvm-gpu[%s] M5.1 SHADOW[%u] alloc class=0x%04x hParent=0x%08x "
             "hObj=0x%08x -> rc=%d status=0x%x%s\n", s->chip->name, s->m2_fwd_n,
             hClass, hParent, hObject, rc, p.status,
             (rc == 0 && p.status == 0) ? "  OK" : "  <-- ERR/MISMATCH");

    /* M7 (cuCtxCreate fix): for GR-object allocs (fam>=0xb0, lowbyte 0xc0 compute / 0x97 3D —
     * NV_GR_ALLOCATION_PARAMETERS, 16B with a GSP-filled `caps` output), capture the HOST's real
     * reply params (auxbuf is in/out; the host RM wrote them) so the GSP-RPC reply builder can
     * pass them through to the guest instead of echoing the request (caps=0). */
    {
        uint32_t lb = hClass & 0xffu, fam = (hClass >> 8) & 0xffu;
        if (fam >= 0xb0u && (lb == 0xc0u || lb == 0x97u) && rc == 0 && p.status == 0) {
            uint32_t n = psize < sizeof(s->m2_gr_reply) ? psize : (uint32_t)sizeof(s->m2_gr_reply);
            memcpy(s->m2_gr_reply, auxbuf, n);
            s->m2_gr_reply_obj = hObject;
            s->m2_gr_reply_psize = p.alloc_parms_size; /* host's RETURNED paramsSize */
            s->m2_gr_reply_valid = true;
            qemu_log("nvkvm-gpu[%s] M7 captured host GR-alloc reply 0x%04x obj=0x%08x "
                     "caps@12=0x%08x host_ret_psize=%u (req_psize=%u)\n", s->chip->name, hClass, hObject,
                     n >= 16 ? ldl_le_p(auxbuf + 12) : 0, p.alloc_parms_size, psize);
        }
    }

    /* M5.3 DATA-PLANE step 1 (enumerate): once the compute object (AMPERE_COMPUTE_B
     * 0xc7c0) constructs on the host shadow context, query GR_GET_CTX_BUFFER_INFO on the
     * subdevice to read the REAL host context-buffer set (size/physAddr/aperture/type).
     * RESULT (2026-06-04): this control is PRIVILEGED -> returns st=0x1b
     * (NV_ERR_INSUFFICIENT_PERMISSIONS) for the unprivileged stub, exactly like
     * GET_SURFACE_PHYS_ATTR. Per the hard security constraint (QEMU stays unprivileged in
     * prod), the "read/mirror host GR context buffers" data-plane approach is BLOCKED on
     * the unprivileged path. Kept as a documented probe; the data plane must instead be
     * solved by forging the guest-side GSP state so libcuda's context buffers are
     * self-consistent (unprivileged), or via an unprivileged host-content path TBD.
     * hParent of the compute object IS the channel. */
    if (hClass == 0xc7c0u && rc == 0 && p.status == 0) {
        /* M5.3 DIAG: arm the crash-window FB-read probe. libcuda now reads GR-context
         * GPU memory and crashes (rbp=0); the reads logged from here pin the buffer. */
        /* #14: record EVERY user GR compute client (one per guest process).  The
         * legacy m2_gr_client scalar stays the first; the sweep/ring/backing paths
         * iterate the list so a 2nd process's compute client is a peer. */
        if (!nvkvm_m2_is_gr_client(s, hClient) &&
            s->m2_gr_clients_n < (int)ARRAY_SIZE(s->m2_gr_clients)) {
            s->m2_gr_clients[s->m2_gr_clients_n++] = hClient;
            qemu_log("nvkvm-gpu[%s] #14 GR compute client[%d] = 0x%08x\n",
                     s->chip->name, s->m2_gr_clients_n - 1, hClient);
        }
        if (!s->m2_gr_client) {
            s->m2_gr_client = hClient;        /* M5.7: the GR compute client (load-bearing, always) */
            /* M5.63: crashwin drives ONLY the high-volume per-fb/per-doorbell DIAG logs (M5.31/M5.15/
             * CRASHWIN-RD/DMAW/M5.6/M5.11/M6.6/M5.10) — arm it only when tracing. m2_gr_client above
             * is the real side effect and stays unconditional. */
            s->m2_crashwin = s->m2_trace;
            if (s->m2_crashwin) {
                qemu_log("nvkvm-gpu[GA106] CRASHWIN ARMED (after 0xc7c0 compute obj "
                         "0x%08x) client=0x%08x — logging subsequent FB reads\n",
                         hObject, hClient);
            }
        }
        /* M5.5: validate the RM_MAP_MEMORY_DMA-FIXED primitive once, on the GR client's
         * real host VASpace. Proves we can place a mapping at a VA we choose — the
         * irreducible step for forwarding the guest's working set into the host VAS. */
        if (!s->m2_mapdma_tested) {
            s->m2_mapdma_tested = true;
            nvkvm_m2_mapdma_selftest(s, hClient);
            nvkvm_m2_osdesc_selftest(s, hClient);   /* M6.2: OS_DESCRIPTOR guest RAM (item-4 step 3) */
        }
        /* M6.5 (item-4 step 4): DISCOVERY sweep — walk the GR VAS page tables, enumerate every
         * sysmem leaf, and OS_DESCRIPTOR+map_dma each into the host GR VASpace so the host GPU
         * can DMA into the guest's actual NVOS32-local sysmem GR buffers (the crash buffers).
         * Idempotent; re-run on each later compute-obj alloc to catch mappings built afterward. */
        if (s->m2exec) {
            nvkvm_m2_enum_gr_sysmem(s, hClient);
        }
        uint32_t subdev = 0;
        for (int i = 0; i < s->m2_subdev_n; i++) {
            if (s->m2_subdev[i].client == hClient) { subdev = s->m2_subdev[i].subdev; break; }
        }
        if (!subdev) {
            qemu_log("nvkvm-gpu[%s] M5.3 ctxbuf: no subdevice tracked for client 0x%08x\n",
                     s->chip->name, hClient);
        } else {
            /* params: hUserClient@0, hChannel@4, bufferCount@8, ctxBufferInfo[64]@16
             * (each 80B: alignment@0,size@8,bufferHandle@16,pageCount@24,physAddr@32,
             * bufferType@40,aperture@44,kind@48,pageSize@52,flags@56,uuid@60). */
            static uint8_t cb[16 + 64 * 80];
            memset(cb, 0, sizeof(cb));
            stl_le_p(cb + 0, nvkvm_m2_client(s, hClient)); /* hUserClient = host client */
            stl_le_p(cb + 4, hParent);                     /* hChannel = the GR channel  */
            uint32_t st = 0xffff;
            int crc = nvkvm_m2_control1(s, hClient, subdev, 0x20801219u, cb, sizeof(cb), &st);
            uint32_t cnt = ldl_le_p(cb + 8);
            qemu_log("nvkvm-gpu[%s] M5.3 GR_GET_CTX_BUFFER_INFO chan=0x%08x sub=0x%08x "
                     "-> crc=%d st=0x%x bufferCount=%u\n", s->chip->name, hParent, subdev,
                     crc, st, cnt);
            if (crc == 0 && st == 0 && cnt <= 64) {
                for (uint32_t i = 0; i < cnt; i++) {
                    const uint8_t *e = cb + 16 + (uint64_t)i * 80;
                    qemu_log("nvkvm-gpu[%s]   ctxbuf[%u] type=%u aperture=%u size=0x%llx "
                             "physAddr=0x%llx align=0x%llx pageSize=%u\n", s->chip->name, i,
                             ldl_le_p(e + 40), ldl_le_p(e + 44),
                             (unsigned long long)ldq_le_p(e + 8),
                             (unsigned long long)ldq_le_p(e + 32),
                             (unsigned long long)ldq_le_p(e + 0), ldl_le_p(e + 52));
                }
            }
        }
    }
}

/* M5.3 helper: forward one RM_ALLOC (NVOS64) with client remap; returns nvstatus. */
static int nvkvm_m2_alloc1(NvkvmGpuEmul *s, uint32_t hClient, uint32_t hParent,
                           uint32_t hObject, uint32_t hClass,
                           void *aux, uint32_t auxlen, uint32_t *st)
{
    struct nvos64_parameters p;
    memset(&p, 0, sizeof(p));
    p.h_root = nvkvm_m2_client(s, hClient);
    p.h_object_parent = nvkvm_m2_client_known(s, hParent) ? nvkvm_m2_client(s, hParent)
                                                          : hParent;
    p.h_object_new = hObject; p.h_class = hClass; p.alloc_parms_size = auxlen;
    unsigned int ic = (3u << 30) | ((unsigned int)sizeof(p) << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_ALLOC;
    uint32_t nv = 0; uint64_t f = 0;
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, ic,
                                 &p, sizeof(p), aux, auxlen, 0, &nv, &f);
    if (st) { *st = p.status; }
    return rc;
}

/* M5.3 helper: forward one RM_CONTROL (NVOS54) with client remap; params marshalled
 * as aux (the stub relocates the params@16 pointer). h_object is an object handle,
 * not a client, so it is passed verbatim unless it names a known client. */
static int nvkvm_m2_control1(NvkvmGpuEmul *s, uint32_t hClient, uint32_t hObject,
                             uint32_t cmd, void *params, uint32_t psize, uint32_t *st)
{
    struct nvos54_parameters p;
    memset(&p, 0, sizeof(p));
    p.h_client = nvkvm_m2_client(s, hClient);
    p.h_object = nvkvm_m2_client_known(s, hObject) ? nvkvm_m2_client(s, hObject) : hObject;
    p.cmd = cmd; p.params_size = psize;
    unsigned int ic = (3u << 30) | ((unsigned int)sizeof(p) << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_CONTROL;
    uint32_t nv = 0; uint64_t f = 0;
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, ic,
                                 &p, sizeof(p), params, psize, 0, &nv, &f);
    if (st) { *st = p.status; }
    return rc;
}

/* M5.3: query the host GPU-physical (FB) address of a vidmem object via
 * NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR (0x410103). Returns the FB phys offset in
 * *phys (the value PROMOTE_CTX needs to point the host GPU at this buffer). The
 * 48B params: memOffset@0 (in: offset, out: phys), memAperture@20 (0=VIDMEM). */
static bool nvkvm_m2_host_phys(NvkvmGpuEmul *s, uint32_t hClient, uint32_t hMem,
                               uint64_t *phys, uint32_t *aperture)
{
    uint8_t pa[48];
    memset(pa, 0, sizeof(pa));
    uint32_t st = 0xffff;
    int rc = nvkvm_m2_control1(s, hClient, hMem, 0x410103u, pa, sizeof(pa), &st);
    if (rc != 0 || st != 0) {
        qemu_log("nvkvm-gpu[%s] M5.3: GET_SURFACE_PHYS_ATTR 0x%x rc=%d st=0x%x\n",
                 s->chip->name, hMem, rc, st);
        return false;
    }
    if (phys)     { *phys = ldq_le_p(pa); }
    if (aperture) { *aperture = ldl_le_p(pa + 20); }
    return true;
}

/* Result of the M5.3 data-plane primitive: real host GPU memory mapped into QEMU. */
struct nvkvm_host_map {
    void    *qva;       /* QEMU VA of the host GPU memory (NULL on failure)    */
    int      mapfd;     /* QEMU's fd for the fresh device open (SCM_RIGHTS)    */
    uint32_t h_mem;     /* RM memory handle on the host                        */
    uint32_t maph;      /* isolate-fd handle of the fresh device fd            */
    uint64_t size;
};

/* M5.3 DATA-PLANE PRIMITIVE (proven 651d860). Allocate a host GPU vidmem object of
 * `size` under (hClient,hDevice) on the real GPU, RM_MAP_MEMORY it on the CONTROL
 * device (NV_CTL_DEVICE_ONLY), and mmap QEMU's copy of a fresh device fd at offset 0
 * (vm_pgoff must be 0; per-fd mmap_context). Returns true with `out` filled, so QEMU
 * holds a real host-GPU-memory VA — the host half of the context-buffer double-mmap.
 * Caller owns the unique RM memory handle `hMem`. */
static bool nvkvm_m2_host_alloc_map_vidmem(NvkvmGpuEmul *s, uint32_t hClient,
                                           uint32_t hDevice, uint32_t hMem,
                                           uint64_t size, struct nvkvm_host_map *out)
{
    memset(out, 0, sizeof(*out));
    if (!nvkvm_m2_iso_ensure(s)) {
        return false;
    }
    struct nv_memory_allocation_params_v545 mp;
    memset(&mp, 0, sizeof(mp));
    mp.owner     = hClient;
    mp.type      = 0;                            /* NVOS32_TYPE_IMAGE */
    mp.attr      = (2u << 27) | (0u << 25);      /* CONTIGUOUS | LOCATION_VIDMEM */
    mp.size      = size;
    mp.alignment = 0x10000;
    uint32_t st = 0xffff;
    nvkvm_m2_alloc1(s, hClient, hDevice, hMem, 0x0040u, &mp, sizeof(mp), &st);
    if (st != 0) {
        qemu_log("nvkvm-gpu[%s] M5.3: host vidmem alloc 0x%x size=0x%llx failed st=0x%x\n",
                 s->chip->name, hMem, (unsigned long long)size, st);
        return false;
    }
    /* Fresh /dev/nvidia0 fd — nvidia binds exactly one CPU mapping per device fd. */
    if (s->m2_maph_next < 16) {
        s->m2_maph_next = 16;
    }
    uint32_t maph = s->m2_maph_next++;
    int mapfd = -1;
    if (nvkvm_isolate_open_device(&s->m2_iso, s->m2_iso_id, maph,
                                  NVKVM_DEV_GPU(0), O_RDWR, &mapfd) != 0 || mapfd < 0) {
        qemu_log("nvkvm-gpu[%s] M5.3: map-fd open failed (maph=%u)\n",
                 s->chip->name, maph);
        return false;
    }
    struct nv_ioctl_nvos33_parameters_with_fd mm;
    memset(&mm, 0, sizeof(mm));
    mm.h_client = nvkvm_m2_client(s, hClient);
    mm.h_device = hDevice;
    mm.h_memory = hMem;
    mm.length   = size;
    mm.fd       = (int32_t)maph;                 /* device fd to mmap (stub translates) */
    unsigned int mc = (3u << 30) | ((unsigned int)sizeof(mm) << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_MAP_MEMORY;
    uint32_t mnv = 0; uint64_t mf = 0;
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, mc,
                                 &mm, sizeof(mm), NULL, 0, 0, &mnv, &mf);
    if (rc != 0 || mm.status != 0) {
        qemu_log("nvkvm-gpu[%s] M5.3: RM_MAP_MEMORY 0x%x failed rc=%d st=0x%x\n",
                 s->chip->name, hMem, rc, mm.status);
        return false;
    }
    void *qva = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, mapfd, 0);
    if (qva == MAP_FAILED) {
        qemu_log("nvkvm-gpu[%s] M5.3: mmap host mem 0x%x failed: %s\n",
                 s->chip->name, hMem, strerror(errno));
        return false;
    }
    out->qva = qva; out->mapfd = mapfd; out->h_mem = hMem; out->maph = maph;
    out->size = size;
    return true;
}

/* CE-forward P1: allocate a host GPU vidmem object WITHOUT the RM_MAP_MEMORY+mmap CPU view.
 * The CPU mapping is what consumes the host's 256 MiB BAR1 (the proven D2 wall); a CE-forward
 * dst is PROT_NONE to the guest CPU, so it needs only a GPU-side map_dma into a channel VAS.
 * Returns the alloc'd object in `out` with qva=NULL/mapfd=-1 — caller does the map_dma. Zero
 * host BAR1 consumed. Mirrors the alloc half of nvkvm_m2_host_alloc_map_vidmem. */
static bool nvkvm_m2_host_alloc_vidmem_gpu_only(NvkvmGpuEmul *s, uint32_t hClient,
                                                uint32_t hDevice, uint32_t hMem,
                                                uint64_t size, struct nvkvm_host_map *out)
{
    memset(out, 0, sizeof(*out));
    out->mapfd = -1;
    if (!nvkvm_m2_iso_ensure(s)) {
        return false;
    }
    struct nv_memory_allocation_params_v545 mp;
    memset(&mp, 0, sizeof(mp));
    mp.owner     = hClient;
    mp.type      = 0;                            /* NVOS32_TYPE_IMAGE */
    mp.attr      = (2u << 27) | (0u << 25);      /* CONTIGUOUS | LOCATION_VIDMEM */
    mp.size      = size;
    mp.alignment = 0x10000;
    uint32_t st = 0xffff;
    nvkvm_m2_alloc1(s, hClient, hDevice, hMem, 0x0040u, &mp, sizeof(mp), &st);
    if (st != 0) {
        qemu_log("nvkvm-gpu[%s] CE-fwd: gpu_only vidmem alloc 0x%x size=0x%llx failed st=0x%x\n",
                 s->chip->name, hMem, (unsigned long long)size, st);
        return false;
    }
    out->qva = NULL; out->mapfd = -1; out->h_mem = hMem; out->maph = 0;
    out->size = size;
    return true;
}

/* CE-fwd map-on-touch: add a CPU view to an ALREADY-ALLOCATED host vidmem object (the map
 * half of nvkvm_m2_host_alloc_map_vidmem, factored out). Used to promote a gpu_only object
 * on its first guest CPU touch: RM_MAP_MEMORY the SAME hMem -> mmap. Returns the qva or NULL
 * (e.g. host BAR1 exhausted -> RM_MAP_MEMORY st!=0). Consumes host BAR1 only now, lazily. */
static void *nvkvm_m2_host_map_existing_vidmem(NvkvmGpuEmul *s, uint32_t hClient,
                                               uint32_t hDevice, uint32_t hMem,
                                               uint64_t size)
{
    if (s->m2_maph_next < 16) {
        s->m2_maph_next = 16;
    }
    uint32_t maph = s->m2_maph_next++;
    int mapfd = -1;
    if (nvkvm_isolate_open_device(&s->m2_iso, s->m2_iso_id, maph,
                                  NVKVM_DEV_GPU(0), O_RDWR, &mapfd) != 0 || mapfd < 0) {
        qemu_log("nvkvm-gpu[%s] CE-fwd promote: map-fd open failed (maph=%u)\n",
                 s->chip->name, maph);
        return NULL;
    }
    struct nv_ioctl_nvos33_parameters_with_fd mm;
    memset(&mm, 0, sizeof(mm));
    mm.h_client = nvkvm_m2_client(s, hClient);
    mm.h_device = hDevice;
    mm.h_memory = hMem;
    mm.length   = size;
    mm.fd       = (int32_t)maph;
    unsigned int mc = (3u << 30) | ((unsigned int)sizeof(mm) << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_MAP_MEMORY;
    uint32_t mnv = 0; uint64_t mf = 0;
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, mc,
                                 &mm, sizeof(mm), NULL, 0, 0, &mnv, &mf);
    if (rc != 0 || mm.status != 0) {
        qemu_log("nvkvm-gpu[%s] CE-fwd promote: RM_MAP_MEMORY 0x%x failed rc=%d st=0x%x "
                 "(host BAR1 likely full)\n", s->chip->name, hMem, rc, mm.status);
        return NULL;
    }
    void *qva = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, mapfd, 0);
    if (qva == MAP_FAILED) {
        qemu_log("nvkvm-gpu[%s] CE-fwd promote: mmap host mem 0x%x failed: %s\n",
                 s->chip->name, hMem, strerror(errno));
        return NULL;
    }
    return qva;
}

/* CE-fwd map-on-touch promotion. The guest CPU is touching gpu_only object `oi` for the first
 * time (via the overlay hot path). Give it a coherent CPU view of the SAME host object, then
 * replay any bytes the guest already wrote to its GPGA range through the local fb_pages BEFORE
 * promotion (mirrors the M5.44 copy-preserve that gpu_only skipped at alloc). After this the
 * overlay resolves to the real object -> guest CPU and host GPU share the SAME bytes. On BAR1
 * exhaustion the object is marked given-up (promote=2): we serve fb_pages and never retry (so
 * a tight-BAR1 DtoH read degrades to stale-but-no-hang rather than an RM_MAP_MEMORY storm). */
static bool nvkvm_m2_promote_gpu_only(NvkvmGpuEmul *s, int oi)
{
    if (oi < 0 || oi >= s->m2_objs_n) {
        return false;
    }
    if (s->m2_objs[oi].promote != 1 || s->m2_objs[oi].cpu_qva) {
        return s->m2_objs[oi].cpu_qva != NULL;   /* already promoted, or not promotable */
    }
    uint32_t client = s->m2_objs[oi].client;
    uint32_t hMem   = s->m2_objs[oi].hMemory;
    uint64_t size   = s->m2_objs[oi].size;
    uint32_t hDev = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == client) { hDev = s->m2_devvas[i].dev; break; }
    }
    void *qva = hDev ? nvkvm_m2_host_map_existing_vidmem(s, client, hDev, hMem, size) : NULL;
    if (!qva) {
        s->m2_objs[oi].promote = 2;          /* give up: serve fb_pages, no retry storm */
        qemu_log("nvkvm-gpu[%s] CE-fwd map-on-touch GIVE-UP obj=%d hMem=0x%08x size=0x%llx "
                 "(host BAR1 full) -> fb_pages fallback\n", s->chip->name, oi, hMem,
                 (unsigned long long)size);
        return false;
    }
    /* Replay pre-promotion guest writes: find this object's GPGA range and copy any written
     * fb_pages into the fresh CPU view (unwritten pages stay as the host CE left them). */
    uint64_t replayed = 0;
    for (int g = 0; g < s->m2_gpga_n; g++) {
        if (s->m2_gpga[g].obj_idx != oi) { continue; }
        uint64_t gbase = s->m2_gpga[g].gpga_base;
        uint64_t goff  = s->m2_gpga[g].off;
        uint64_t glen  = s->m2_gpga[g].size;
        for (uint64_t off = 0; off < glen; off += 4096) {
            uint8_t *gp = nvkvm_fb_page(s, gbase + off, false);
            if (gp) { memcpy((uint8_t *)qva + goff + off, gp, 4096); replayed += 4096; }
        }
    }
    s->m2_objs[oi].cpu_qva = qva;
    s->m2_objs[oi].promote = 0;
    qemu_log("nvkvm-gpu[%s] CE-fwd map-on-touch PROMOTED obj=%d hMem=0x%08x size=0x%llx "
             "cpu_qva=%p replayed=0x%llx (coherent CPU+GPU view now)\n", s->chip->name,
             oi, hMem, (unsigned long long)size, qva, (unsigned long long)replayed);
    return true;
}

/* M6.2 (item-4 step 3): translate a guest GPA to the stub VA where the guest-RAM memfd is
 * MAP_FIXED'd. Uses pci_dma_map to get QEMU's host VA for the GPA (hole-safe across the q35
 * PCI hole), then stub_va = stub_base + (hva - ram_base_hva) since the stub mmapped the SAME
 * memfd at m2_stub_ram_base. Returns 0 if not shared / GPA outside the main RAM block. */
static uint64_t nvkvm_m2_gpa_to_stub_va(NvkvmGpuEmul *s, uint64_t gpa)
{
    if (!s->m2_ram_shared || !s->m2_guest_ram_hva) {
        return 0;
    }
    dma_addr_t len = 0x1000;
    void *p = pci_dma_map(&s->parent_obj, gpa, &len, DMA_DIRECTION_TO_DEVICE);
    if (!p) {
        return 0;
    }
    uint64_t off = (uint64_t)((uintptr_t)p - (uintptr_t)s->m2_guest_ram_hva);
    pci_dma_unmap(&s->parent_obj, p, len, DMA_DIRECTION_TO_DEVICE, 0);
    if (off >= s->m2_guest_ram_size) {
        return 0;                                /* GPA not in the main (memfd) RAM block */
    }
    return s->m2_stub_ram_base + off;
}

/* M6.2 (item-4 step 3): OS_DESCRIPTOR-register guest RAM (at stub VA) as a host sysmem object,
 * so the host nvidia driver pins the guest pages and the host GPU can DMA into them — the fix
 * for libcuda's un-backed sysmem GR buffers ([[mode2-cuctxcreate-pagetable-poll]]). NVOS02
 * (NV_ESC_RM_ALLOC_MEMORY, NR 0x27), hClass=NV01_MEMORY_SYSTEM_OS_DESCRIPTOR (0x0071),
 * p_memory = the descriptor (stub VA the kernel pin_user_pages walks), limit = size-1, flags =
 * PHYSICALITY_NONCONTIGUOUS | LOCATION_PCI | COHERENCY_CACHED. */
static int nvkvm_m2_os_descriptor(NvkvmGpuEmul *s, uint32_t client, uint32_t device,
                                  uint32_t hMem, uint64_t stub_va, uint64_t size, uint32_t *st)
{
    /* The GR client lives in the ctl-fd session; OS_DESCRIPTOR runs on the /dev/nvidia0 device
     * fd, which must be REGISTER_FD'd to that ctl session or RM returns 0x23 INVALID_CLIENT
     * (real host libcuda does NV_ESC_REGISTER_FD(nvidia0, ctl_fd) before using the device fd). */
    if (!s->m2_gpu_registered) {
        struct nv_ioctl_register_fd rf; memset(&rf, 0, sizeof(rf));
        rf.ctl_fd = (int32_t)s->m2_ctl_h;   /* handle; stub translates @off 0 -> real ctl fd */
        unsigned int rc2 = (3u << 30) | ((unsigned int)sizeof(rf) << 16) |
                           ((unsigned int)'F' << 8) | NV_ESC_REGISTER_FD;
        uint32_t rnv = 0; uint64_t rf2 = 0;
        int rr = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_gpu_h, rc2,
                                     &rf, sizeof(rf), NULL, 0, 0, &rnv, &rf2);
        s->m2_gpu_registered = (rr == 0);
        qemu_log("nvkvm-gpu[%s] M6.2 REGISTER_FD(gpu0, ctl) rc=%d -> %s\n", s->chip->name,
                 rr, s->m2_gpu_registered ? "registered" : "FAILED");
    }
    struct nv_ioctl_nvos02_parameters_with_fd p;
    memset(&p, 0, sizeof(p));
    p.h_root          = nvkvm_m2_client(s, client);
    p.h_object_parent = nvkvm_m2_client_known(s, device) ? nvkvm_m2_client(s, device) : device;
    p.h_object_new    = hMem;
    p.h_class         = 0x00000071u;             /* NV01_MEMORY_SYSTEM_OS_DESCRIPTOR */
    /* Flags captured from a real host CUDA OS_DESCRIPTOR (cuMemHostAlloc/Register on driver
     * 580.159.04): 0x40001010 = NONCONTIG(0x10) | LOCATION_PCI(0) | COHERENCY_CACHED(0x1000) |
     * MAPPING_NO_MAP(0x40000000, bits31:30=1). MAPPING_NO_MAP is required — without it the
     * driver returns EINVAL (it tried to auto-map a describe-only allocation). */
    p.flags           = 0x40001010u;
    p.p_memory        = stub_va;                 /* [IN] descriptor: stub VA of the guest RAM */
    p.limit           = size ? (size - 1) : 0;
    p.fd              = -1;
    unsigned int ic = (3u << 30) | ((unsigned int)sizeof(p) << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_ALLOC_MEMORY;
    uint32_t nv = 0; uint64_t f = 0;
    /* OS_DESCRIPTOR must run on the GPU DEVICE fd (/dev/nvidia0, m2_gpu_h), NOT the ctl fd —
     * a real host CUDA app does it on /dev/nvidia0 (captured). ctl fd -> EINVAL. */
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_gpu_h, ic,
                                 &p, sizeof(p), NULL, 0, 0, &nv, &f);
    if (st) { *st = p.status; }
    return rc;
}

/* M6.2 selftest: OS_DESCRIPTOR the first real GR sysmem buffer (va_map sys=true entry) to prove
 * the shared-memfd -> stub-VA -> OS_DESCRIPTOR chain works (host RM pins guest RAM). One-shot. */
static void nvkvm_m2_osdesc_selftest(NvkvmGpuEmul *s, uint32_t hClient)
{
    uint32_t hDev = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == hClient) { hDev = s->m2_devvas[i].dev; break; }
    }
    int idx = -1;
    for (int i = 0; i < s->va_map_n; i++) {
        if (s->va_map[i].sys && s->va_map[i].size) { idx = i; break; }
    }
    if (!hDev || idx < 0) {
        qemu_log("nvkvm-gpu[%s] M6.2 osdesc-selftest: no dev (0x%08x) or no sysmem va_map "
                 "entry (va_map_n=%d)\n", s->chip->name, hDev, s->va_map_n);
        return;
    }
    uint64_t gpa = s->va_map[idx].phys, sz = s->va_map[idx].size;
    uint64_t sva = nvkvm_m2_gpa_to_stub_va(s, gpa);
    if (!sva) {
        qemu_log("nvkvm-gpu[%s] M6.2 osdesc-selftest: GPA 0x%llx -> no stub VA (shared=%d)\n",
                 s->chip->name, (unsigned long long)gpa, s->m2_ram_shared);
        return;
    }
    uint32_t hMem = 0xdd000000u | (s->m2_databuf_next++ & 0xffffu);
    uint32_t st = 0xffff;
    int rc = nvkvm_m2_os_descriptor(s, hClient, hDev, hMem, sva, sz, &st);
    qemu_log("nvkvm-gpu[%s] M6.2 osdesc-selftest: GR sysmem buf GPA=0x%llx size=0x%llx -> "
             "stub_va=0x%llx OS_DESCRIPTOR hMem=0x%08x rc=%d st=0x%x %s\n", s->chip->name,
             (unsigned long long)gpa, (unsigned long long)sz, (unsigned long long)sva, hMem,
             rc, st, (rc == 0 && st == 0) ? "  OK — host RM pinned guest RAM!"
                                          : "  <-- ERR (tune flags/descriptor)");
    /* M6.3 (item-4 step 4): map the pinned guest RAM into the GR VASpace at the guest's GR VA,
     * so the host GPU's MMU resolves that VA to the guest's sysmem buffer (host GPU then
     * DMA-reads/writes the SAME memory the guest CPU sees). Reuses the M5.5 map_dma primitive +
     * the per-client GR virtmem mapper. */
    if (rc == 0 && st == 0) {
        uint64_t va = s->va_map[idx].va;
        uint32_t hVirt = nvkvm_m2_grmapper(s, hClient);
        uint32_t mst = 0xffff; uint64_t outva = 0;
        int mrc = hVirt ? nvkvm_m2_map_dma(s, hClient, hDev, hVirt, hMem, 0, sz, true, va,
                                           &mst, &outva) : -1;
        qemu_log("nvkvm-gpu[%s] M6.3 map pinned guest RAM into GR VAS: hVirt=0x%08x va=0x%llx "
                 "-> rc=%d st=0x%x outva=0x%llx %s\n", s->chip->name, hVirt,
                 (unsigned long long)va, mrc, mst, (unsigned long long)outva,
                 (mrc == 0 && mst == 0 && outva == va)
                     ? "  OK — host GPU can now reach the guest's sysmem GR buffer!"
                     : (mst == 0x51u ? "  ALREADY-MAPPED" : "  <-- ERR"));
        /* M6.3b (the user's "can we fix ANY GR VA?" for SYSMEM): map the SAME OS_DESCRIPTOR'd
         * guest RAM at a VA WE choose (free, not host-pre-mapped) into the GR VAS. st=0 proves
         * we control the sysmem GR-VA layout end-to-end (guest RAM placeable at any chosen GR VA
         * = the item-4 step-4 placement primitive, validated). Distinct from M6.3 which reuses
         * the guest's own (often host-occupied) VA. */
        if (hVirt) {
            uint64_t freeva = 0x300000000ull;        /* well clear of GR ctx (0x120xxxxxx) + UVM */
            uint32_t fst = 0xffff; uint64_t fova = 0;
            int frc = nvkvm_m2_map_dma(s, hClient, hDev, hVirt, hMem, 0, sz, true, freeva,
                                       &fst, &fova);
            qemu_log("nvkvm-gpu[%s] M6.3b place guest-RAM sysmem at CHOSEN free GR VA=0x%llx -> "
                     "rc=%d st=0x%x outva=0x%llx %s\n", s->chip->name,
                     (unsigned long long)freeva, frc, fst, (unsigned long long)fova,
                     (frc == 0 && fst == 0 && fova == freeva)
                         ? "  OK — we OWN the sysmem GR-VA layout (item-4 step-4 primitive proven)"
                         : (fst == 0x51u ? "  ALREADY-MAPPED (pick another VA)" : "  <-- ERR"));
        }
    }
}

/* M6.4 (item-4): forward the guest's PROMOTE_CTX to the host with each sysmem buffer's
 * gpuPhysAddr substituted to OUR backing's host physical. For each promote entry that's
 * sysmem + mapped: OS_DESCRIPTOR the guest RAM at its GPA -> host hMem -> GET_SURFACE_PHYS_ATTR
 * -> host phys; write that into the entry's gpuPhysAddr. Then forward the (substituted)
 * PROMOTE_CTX control (0x2080012b) on the GR subdevice. Effect: the host GR context maps the
 * guest's GR VAs onto the guest's actual sysmem -> host GPU DMA-fills what libcuda reads.
 * (Reframe: we don't replay the guest's calls — we reproduce the GR-VA->backing EFFECT.) */
/* M6.5: retained for reference only — NOT called (PROMOTE_CTX is a Case-2
 * privileged control we no longer replay; see mode2_forwarding_model.md). */
G_GNUC_UNUSED static void nvkvm_m2_forward_promote_ctx(NvkvmGpuEmul *s, const uint8_t *cmd)
{
    uint32_t hClient = ldl_le_p(cmd + 80), hObject = ldl_le_p(cmd + 84);
    uint32_t psize   = ldl_le_p(cmd + 96);
    if (psize < 48 || psize > 8192) {
        return;
    }
    static uint8_t pc[8192];
    memcpy(pc, cmd + 120, psize);                 /* the PROMOTE_CTX params */
    uint32_t ec = ldl_le_p(pc + 40);
    if (ec > 20) { ec = 20; }
    uint32_t dev = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == hClient) { dev = s->m2_devvas[i].dev; break; }
    }
    int subst = 0;
    for (uint32_t i = 0; i < ec; i++) {
        uint8_t *e = pc + 48 + (uint64_t)i * 32;
        uint64_t phys = ldq_le_p(e + 0), va = ldq_le_p(e + 8), sz = ldq_le_p(e + 16);
        uint32_t physAttr = ldl_le_p(e + 24);
        uint8_t  bNonmapped = e[31];
        if (!va || !sz || bNonmapped || (physAttr & 0x3u) == 0) {
            continue;                             /* skip vidmem / unmapped / phys-only */
        }
        uint64_t sva = nvkvm_m2_gpa_to_stub_va(s, phys);
        if (!sva || !dev) { continue; }
        uint32_t hMem = 0xde000000u | (s->m2_databuf_next++ & 0xffffu);
        uint32_t ost = 0xffff;
        if (nvkvm_m2_os_descriptor(s, hClient, dev, hMem, sva, sz, &ost) != 0 || ost != 0) {
            continue;
        }
        uint64_t hphys = 0; uint32_t aper = 0xff;
        if (!nvkvm_m2_host_phys(s, hClient, hMem, &hphys, &aper) || !hphys) {
            continue;
        }
        stq_le_p(e + 0, hphys);                   /* substitute gpuPhysAddr -> our backing */
        subst++;
    }
    uint32_t st = 0xffff;
    int rc = nvkvm_m2_control1(s, hClient, hObject, 0x2080012bu, pc, psize, &st);
    qemu_log("nvkvm-gpu[%s] M6.4 forward PROMOTE_CTX: client=0x%08x subdev=0x%08x entries=%u "
             "subst=%d -> rc=%d st=0x%x %s\n", s->chip->name, hClient, hObject, ec, subst,
             rc, st, (rc == 0 && st == 0) ? "  OK — host GR ctx mapped onto guest RAM!"
                                          : "  <-- ERR");
}

/* M5.5 EXECUTION-PLANE PRIMITIVE: map a host memory object into a host VASpace at a
 * FIXED GPU VA (the guest's chosen VA), via NV_ESC_RM_MAP_MEMORY_DMA. This is the
 * irreducible primitive the whole data plane rests on: it puts the guest's working-set
 * buffers (GPFIFO / pushbuffer / semaphores / ctx buffers) into the host channel's VAS
 * at the guest VAs, so when the host GPU runs the channel its MMU resolves the same VAs
 * the guest submitted — i.e. the host executes the guest's real work (no faking).
 *   hDma  = the VASpace handle (FERMI_VASPACE_A); for a VASpace (non-CTXDMA) target,
 *           dmaOffset is [IN] when DMA_OFFSET_FIXED_TRUE, [OUT] otherwise.
 *   NVOS46 V580 layout (nvkvm_abi: size 64, status@56 — flags2+kindOverride pushed it
 *   past the 535/56-byte base): hClient@0 hDevice@4 hDma@8 hMemory@12 offset@16(u64)
 *   length@24(u64) flags@32 flags2@36 kindOverride@40 dmaOffset@48(u64) status@56.
 * Returns the ioctl rc; *st gets the RM status; *out_va gets the resulting GPU VA. */
static int nvkvm_m2_map_dma(NvkvmGpuEmul *s, uint32_t hClient, uint32_t hDevice,
                            uint32_t hVas, uint32_t hMemory, uint64_t offset,
                            uint64_t length, bool fixed, uint64_t va,
                            uint32_t *st, uint64_t *out_va)
{
    uint8_t p[64];
    memset(p, 0, sizeof(p));
    stl_le_p(p + 0,  nvkvm_m2_client(s, hClient));         /* hClient (remapped)        */
    stl_le_p(p + 4,  nvkvm_m2_client_known(s, hDevice) ? nvkvm_m2_client(s, hDevice)
                                                       : hDevice);   /* hDevice         */
    stl_le_p(p + 8,  nvkvm_m2_client_known(s, hVas) ? nvkvm_m2_client(s, hVas)
                                                    : hVas);         /* hDma = VASpace  */
    stl_le_p(p + 12, nvkvm_m2_client_known(s, hMemory) ? nvkvm_m2_client(s, hMemory)
                                                       : hMemory);   /* hMemory         */
    stq_le_p(p + 16, offset);
    stq_le_p(p + 24, length);
    /* flags: ACCESS_READ_WRITE(0) | (DMA_OFFSET_FIXED_TRUE bit15 = 0x8000 if fixed) */
    stl_le_p(p + 32, fixed ? 0x00008000u : 0u);
    if (fixed) {
        stq_le_p(p + 48, va);                              /* dmaOffset [IN] = FIXED VA */
    }
    unsigned int ic = (3u << 30) | ((unsigned int)64 << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_MAP_MEMORY_DMA;
    uint32_t nv = 0; uint64_t f = 0;
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, ic,
                                 p, sizeof(p), NULL, 0, 0, &nv, &f);
    if (st)     { *st = ldl_le_p(p + 56); }
    if (out_va) { *out_va = ldq_le_p(p + 48); }
    return rc;
}

/* M5.5: allocate an NV01_MEMORY_VIRTUAL (class 0x0070) mapper spanning a VASpace. RM's
 * RM_MAP_MEMORY_DMA mapper (hDma) must be a VirtualMemory resource — virtual_mem.c is the
 * only class that implements MapTo; vaspace_api.c does NOT — so a raw FERMI_VASPACE_A or
 * Device handle as hDma returns INVALID_OBJECT_HANDLE. NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS
 * (cl0070.h, 24B): offset@0(u64), limit@8(u64), hVASpace@16(u32) (NULL => device default,
 * else a FERMI_VASPACE_A). One mapper per vaspace, then many FIXED map_dma into it. */
static int nvkvm_m2_alloc_virtmem(NvkvmGpuEmul *s, uint32_t hClient, uint32_t hDevice,
                                  uint32_t hVirt, uint32_t hVASpace, uint32_t *st)
{
    uint8_t p[24];
    memset(p, 0, sizeof(p));
    stq_le_p(p + 0, 0);            /* offset = 0 */
    stq_le_p(p + 8, 0);            /* limit  = 0 (=> max) */
    stl_le_p(p + 16, hVASpace);    /* hVASpace (0 = device default) */
    return nvkvm_m2_alloc1(s, hClient, hDevice, hVirt, 0x0070u, p, sizeof(p), st);
}

/* M5.28 PER-CHANNEL VAS: get (allocating on first use) the fresh nvkvm-owned VAS context
 * for a (client, tsg). The fresh FERMI_VASPACE_A is allocated under the channel's FORWARDED
 * device (the TSG is parented to it, so RM requires the VAS share that device); a virtmem
 * mapper (NV01_MEMORY_VIRTUAL) spans it for FIXED map_dma. We substitute fvas into the GR
 * TSG's hVASpace (and its ctxshare) in shadow_fwd, and route the channel's working-set maps
 * here (m2_cur_cvas) instead of the guest's forwarded VAS — so every guest VA places into a
 * VAS WE fully control, killing the host-RM-self-promote collision (st=0x51 / Xid 32).
 * Returns index into m2_cvas[], or -1. */
static int nvkvm_m2_cvas_get(NvkvmGpuEmul *s, uint32_t client, uint32_t tsg)
{
    for (int i = 0; i < s->m2_cvas_n; i++) {
        if (s->m2_cvas[i].client == client && s->m2_cvas[i].tsg == tsg) { return i; }
    }
    if (s->m2_cvas_n >= (int)ARRAY_SIZE(s->m2_cvas)) { return -1; }
    uint32_t hDev = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == client) { hDev = s->m2_devvas[i].dev; break; }
    }
    if (!hDev) {
        qemu_log("nvkvm-gpu[%s] M5.28 cvas_get: no forwarded device for client 0x%08x\n",
                 s->chip->name, client);
        return -1;
    }
    uint32_t fVas  = 0xce200000u | (s->m2_databuf_next++ & 0xffffu);
    uint32_t fVirt = 0xce300000u | (s->m2_databuf_next++ & 0xffffu);
    uint8_t vasp[56]; memset(vasp, 0, sizeof(vasp));
    uint32_t vst = 0xffff, vmst = 0xffff;
    nvkvm_m2_alloc1(s, client, hDev, fVas, 0x90f1u, vasp, sizeof(vasp), &vst);
    if (vst == 0) {
        nvkvm_m2_alloc_virtmem(s, client, hDev, fVirt, fVas, &vmst);
    }
    qemu_log("nvkvm-gpu[%s] M5.28 cvas_get: client=0x%08x tsg=0x%08x dev=0x%08x -> "
             "fresh vas=0x%08x(st=0x%x) virtmem=0x%08x(st=0x%x)%s\n", s->chip->name,
             client, tsg, hDev, fVas, vst, fVirt, vmst,
             (vst == 0 && vmst == 0) ? "  OK" : "  <-- ERR");
    if (vst != 0 || vmst != 0) { return -1; }
    int idx = s->m2_cvas_n++;
    s->m2_cvas[idx].client    = client;
    s->m2_cvas[idx].tsg       = tsg;
    s->m2_cvas[idx].hdev      = hDev;
    s->m2_cvas[idx].fvas      = fVas;
    s->m2_cvas[idx].fvirt     = fVirt;
    s->m2_cvas[idx].populated = false;
    return idx;
}

/* M5.7 EXECUTION PLANE: get (allocating once) the NV01_MEMORY_VIRTUAL mapper spanning a
 * client's GR VASpace. Returns the virtmem handle (0 on failure). The mapper is the hDma
 * for all FIXED map_dma into that vaspace. */
static uint32_t nvkvm_m2_grmapper(NvkvmGpuEmul *s, uint32_t client)
{
    /* M5.28: when a per-channel VAS is active for this client, route ALL FIXED map_dma
     * into ITS fresh nvkvm-owned virtmem mapper (not the guest's forwarded VAS). */
    if (s->m2_cur_cvas >= 0 && s->m2_cur_cvas < s->m2_cvas_n &&
        s->m2_cvas[s->m2_cur_cvas].client == client) {
        return s->m2_cvas[s->m2_cur_cvas].fvirt;
    }
    /* M5.48: even OUTSIDE the doorbell loop (m2_cur_cvas unset), a client whose TSGs were
     * re-homed into an nvkvm-owned cvas (M5.28 GR / M5.40 COPY-shared) RUNS its channels in
     * that fvas — so EVERY FIXED map for that client must target the SAME fvas. The out-of-
     * loop map sites (M6.5 sweeps at the 0xc7c0 alloc + M5.10 re-sweeps, M5.7 ctx/gpfifo,
     * M5.9 pushbufs) previously fell through to the per-client grmap virtmem over the guest
     * forwarded VAS (0x5c000007): the GPFIFO pool (VA 0x200200000, GPGA 0x4200000) landed
     * THERE while the host channel runs in fvas 0xce20002a -> host PBDMA fetch faulted
     * Xid 31 FAULT_PDE @ the GPFIFO VA (and poisoned va_seen so populate_cvas backed=0).
     * Prefer the GR TSG's entry; all of a client's entries share one fvas after M5.40. */
    {
        int any = -1;
        for (int i = 0; i < s->m2_cvas_n; i++) {
            if (s->m2_cvas[i].client != client) { continue; }
            if (s->m2_cvas[i].tsg == s->m2_gr_tsg) { any = i; break; }
            if (any < 0) { any = i; }
        }
        if (any >= 0) {
            return s->m2_cvas[any].fvirt;
        }
    }
    for (int i = 0; i < s->m2_grmap_n; i++) {
        if (s->m2_grmap[i].client == client) {
            return s->m2_grmap[i].hvirt;
        }
    }
    uint32_t hDev = 0, hVas = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == client) {
            hDev = s->m2_devvas[i].dev; hVas = s->m2_devvas[i].vas; break;
        }
    }
    if (!hDev || !hVas || s->m2_grmap_n >= (int)ARRAY_SIZE(s->m2_grmap)) {
        qemu_log("nvkvm-gpu[%s] M5.7 grmapper: no dev/vas for client 0x%08x\n",
                 s->chip->name, client);
        return 0;
    }
    uint32_t hVirt = 0xdb000000u | (s->m2_databuf_next++ & 0xffffu);
    uint32_t st = 0xffff;
    nvkvm_m2_alloc_virtmem(s, client, hDev, hVirt, hVas, &st);
    if (st != 0) {
        /* M5.20 FRESH-VAS FALLBACK: the guest's forwarded VAS (0xcaf00000) for the
         * COMPUTE client is parented to a libcuda PROBE device (0x3141590x) and is an
         * index=3 / EXTERNALLY_OWNED-class VASpace; the host RM rejects NV01_MEMORY_
         * VIRTUAL over it with 0x57 (INSUFFICIENT_PERMISSIONS). The self-contained
         * selftest path (fresh client->device->vaspace->virtmem) succeeds, so allocate
         * a FRESH, normal RM-managed device+VASpace under THIS client and map into it.
         * The host compute channel is pointed at this fresh VAS in shadow_fwd (M5.21)
         * so its working set (pushbuffer/sema mapped here) resolves when it runs.
         * Contained to the failure path — the CeUtils clients (st==0 above) are
         * untouched. */
        uint32_t fDev  = 0xdf100000u | (s->m2_databuf_next++ & 0xffffu);
        uint32_t fVas  = 0xdf200000u | (s->m2_databuf_next++ & 0xffffu);
        uint32_t fVirt = 0xdf300000u | (s->m2_databuf_next++ & 0xffffu);
        uint8_t devp[56]; memset(devp, 0, sizeof(devp));
        uint8_t vasp[56]; memset(vasp, 0, sizeof(vasp));
        uint32_t dst = 0xffff, vst = 0xffff, st2 = 0xffff;
        nvkvm_m2_alloc1(s, client, client, fDev, 0x0080u, devp, sizeof(devp), &dst);
        nvkvm_m2_alloc1(s, client, fDev, fVas, 0x90f1u, vasp, sizeof(vasp), &vst);
        if (dst == 0 && vst == 0) {
            nvkvm_m2_alloc_virtmem(s, client, fDev, fVirt, fVas, &st2);
        }
        qemu_log("nvkvm-gpu[%s] M5.20 grmapper: guest-VAS 0x%08x virtmem st=0x%x -> "
                 "FRESH dev=0x%08x(st=0x%x) vas=0x%08x(st=0x%x) virtmem=0x%08x(st=0x%x)\n",
                 s->chip->name, hVas, st, fDev, dst, fVas, vst, fVirt, st2);
        if (!(dst == 0 && vst == 0 && st2 == 0)) {
            return 0;
        }
        hVirt = fVirt; hVas = fVas; hDev = fDev;
        /* M5.49b: this client hit the FRESH-VAS fallback => it is a libcuda
         * compute-side (CE-copy) client, the user-observable data path.  Record it
         * (dedup) so its CE completion is forced host-written under m2hostsem. */
        /* #14: never record a GR compute client (ANY process's, incl. a 2nd
         * process's dup-src client pre-0xc7c0) as a user-CE client — that would
         * route its GR channels through the CE-forward/hostsem paths.  Single
         * process: is_gr_client == {m2_gr_client}, multiproc false — identical. */
        if (!nvkvm_m2_is_gr_client(s, client) &&
            !(nvkvm_m2_multiproc(s) && nvkvm_m2_is_user_client(s, client))) {
            bool known = false;
            for (int i = 0; i < s->m2_user_ce_n; i++) {
                if (s->m2_user_ce_clients[i] == client) { known = true; break; }
            }
            if (!known && s->m2_user_ce_n < (int)ARRAY_SIZE(s->m2_user_ce_clients)) {
                s->m2_user_ce_clients[s->m2_user_ce_n++] = client;
                qemu_log("nvkvm-gpu[%s] M5.49b USER-CE client 0x%08x recorded "
                         "(host-only completion target)\n", s->chip->name, client);
            }
        }
    }
    s->m2_grmap[s->m2_grmap_n].client = client;
    s->m2_grmap[s->m2_grmap_n].hvirt  = hVirt;
    s->m2_grmap[s->m2_grmap_n].hvas   = hVas;
    s->m2_grmap[s->m2_grmap_n].hdev   = hDev;
    s->m2_grmap_n++;
    qemu_log("nvkvm-gpu[%s] M5.7 grmapper: client 0x%08x -> virtmem 0x%08x over VAS "
             "0x%08x (dev 0x%08x)\n", s->chip->name, client, hVirt, hVas, hDev);
    return hVirt;
}

/* M5.7 EXECUTION PLANE unit op: back a guest working-set buffer with real host GPU vidmem
 * and place it in the GR channel's address space at the guest's VA.
 *  (1) alloc host vidmem(size) under the GR client+device;
 *  (2) double-mmap it into m2_fbback at the guest-FB phys, so guest CPU access (BAR/PRAMIN
 *      -> nvkvm_fb_read/write) and the host GPU share the SAME bytes (no faking);
 *  (3) map_dma FIXED at the guest VA into the client's GR virtmem mapper, so the host GPU's
 *      MMU resolves that VA to this memory when it runs the channel.
 * Returns true on success. CONTENT DIRECTION: `copy_content`=true for GUEST-written buffers
 * (GPFIFO/pushbuffers) -> copy the guest's current FB bytes into the host vidmem so the host
 * GPU reads the real commands; false for GPU-written buffers (completion semaphore) which the
 * host fills. The FB overlay is registered ONLY on a successful PLACE (st=0): a 0x51
 * (already-host-mapped, e.g. ctx) must NOT be overlaid or we'd shadow the host's real buffer
 * with zeroed memory. phys==0 => VA-only mapping (no overlay). */
static bool nvkvm_m2_back_and_map_inner(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                  uint64_t phys, uint64_t size, bool copy_content,
                                  const char *label);
static bool nvkvm_m2_back_and_map(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                  uint64_t phys, uint64_t size, bool copy_content,
                                  const char *label)
{
    uint64_t t0 = nvkvm_now_ns();
    bool r = nvkvm_m2_back_and_map_inner(s, client, va, phys, size, copy_content, label);
    nvkvm_t_backmap_ns += nvkvm_now_ns() - t0; nvkvm_t_backmap_calls++;
    return r;
}
static bool nvkvm_m2_back_and_map_inner(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                  uint64_t phys, uint64_t size, bool copy_content,
                                  const char *label)
{
    uint32_t hVirt = nvkvm_m2_grmapper(s, client);
    if (!hVirt) {
        return false;
    }
    uint32_t hDev = 0;
    for (int i = 0; i < s->m2_grmap_n; i++) {
        if (s->m2_grmap[i].client == client) { hDev = s->m2_grmap[i].hdev; break; }
    }
    if (!hDev) {            /* M5.48: cvas-routed client never minted a grmap entry */
        for (int i = 0; i < s->m2_devvas_n; i++) {
            if (s->m2_devvas[i].client == client) { hDev = s->m2_devvas[i].dev; break; }
        }
    }
    uint64_t asize = (size + 0xffff) & ~0xffffull;     /* round to 64 KiB */
    if (s->m2_fbback_n >= (int)ARRAY_SIZE(s->m2_fbback)) {
        qemu_log("nvkvm-gpu[%s] M5.7 back_and_map: m2_fbback full\n", s->chip->name);
        return false;
    }
    uint32_t hMem = 0xdc000000u | (s->m2_databuf_next++ & 0xffffu);
    struct nvkvm_host_map hm;
    if (!nvkvm_m2_host_alloc_map_vidmem(s, client, hDev, hMem, asize, &hm)) {
        qemu_log("nvkvm-gpu[%s] M5.7 back_and_map[%s]: host vidmem alloc failed\n",
                 s->chip->name, label);
        return false;
    }
    uint32_t st = 0xffff; uint64_t outva = 0;
    int rc = nvkvm_m2_map_dma(s, client, hDev, hVirt, hMem, 0, asize, true, va, &st, &outva);
    /* st=0x51 (NV_ERR_NO_MEMORY) on a FIXED map => the VA is ALREADY mapped in the host
     * VASpace (host RM self-promoted its GR ctx at the same VAs). Desired for ctx buffers —
     * host already has them; do NOT overlay. Only genuinely-unmapped buffers get placed. */
    bool already = (st == 0x51u);
    bool ok = (rc == 0 && st == 0 && outva == va);
    if (phys && ok) {                            /* overlay ONLY a buffer we actually placed */
        if (copy_content) {                      /* preserve guest-written bytes (cmds) */
            for (uint64_t off = 0; off < asize; off += 4096) {
                uint8_t *gp = nvkvm_fb_page(s, phys + off, false);
                if (gp) { memcpy((uint8_t *)hm.qva + off, gp, 4096); }
            }
        }
        s->m2_fbback[s->m2_fbback_n].fb_base  = phys;
        s->m2_fbback[s->m2_fbback_n].size     = asize;
        s->m2_fbback[s->m2_fbback_n].host_qva = hm.qva;
        s->m2_fbback_n++;
    }
    qemu_log("nvkvm-gpu[%s] M5.7 back_and_map[%s] VA=0x%llx phys=0x%llx size=0x%llx copy=%d -> "
             "hMem=0x%08x qva=%p map rc=%d st=0x%x va=0x%llx%s\n", s->chip->name, label,
             (unsigned long long)va, (unsigned long long)phys, (unsigned long long)asize,
             copy_content, hMem, hm.qva, rc, st, (unsigned long long)outva,
             ok ? "  OK PLACED" : already ? "  ALREADY-HOST-MAPPED" : "  <-- ERR");
    return ok || already;
}

/* M5.8 DOORBELL-FORWARD setup (no ring): alloc the host AMPERE_USERMODE_A (0xc561) doorbell
 * register page under the GR client's subdevice, RM_MAP_MEMORY + mmap it into QEMU, and fetch
 * the host GR channel's work-submit token (NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN
 * 0xc36f0108). To RING (later, once pushbuffers are mapped + channel scheduled) we write the
 * token to usermode_qva + NVC361_NOTIFY_CHANNEL_PENDING (0x90) -> the HOST GPU runs the
 * channel. NOT rung here: ringing before the working set is mapped/scheduled would fault the
 * host GPU (wedge). This validates the two new primitives (usermode map + token). */
static void nvkvm_m2_doorbell_setup(NvkvmGpuEmul *s, uint32_t client)
{
    if (s->m2_doorbell_ready || !s->m2_gr_channel) {
        return;
    }
    uint32_t hDev = 0, subdev = 0;
    for (int i = 0; i < s->m2_grmap_n; i++) {
        if (s->m2_grmap[i].client == client) { hDev = s->m2_grmap[i].hdev; break; }
    }
    if (!hDev) {            /* M5.48: cvas-routed client never minted a grmap entry */
        for (int i = 0; i < s->m2_devvas_n; i++) {
            if (s->m2_devvas[i].client == client) { hDev = s->m2_devvas[i].dev; break; }
        }
    }
    for (int i = 0; i < s->m2_subdev_n; i++) {
        if (s->m2_subdev[i].client == client) { subdev = s->m2_subdev[i].subdev; break; }
    }
    if (!hDev || !subdev) {
        qemu_log("nvkvm-gpu[%s] M5.8 doorbell: no dev/subdev for client 0x%08x\n",
                 s->chip->name, client);
        return;
    }
    uint32_t hUM = 0xde900001u, st = 0xffff;
    nvkvm_m2_alloc1(s, client, subdev, hUM, 0xc561u, NULL, 0, &st);   /* AMPERE_USERMODE_A */
    qemu_log("nvkvm-gpu[%s] M5.8 doorbell: AMPERE_USERMODE_A alloc st=0x%x\n",
             s->chip->name, st);
    if (st != 0) {
        return;
    }
    if (s->m2_maph_next < 16) { s->m2_maph_next = 16; }
    uint32_t maph = s->m2_maph_next++;
    int mapfd = -1;
    if (nvkvm_isolate_open_device(&s->m2_iso, s->m2_iso_id, maph, NVKVM_DEV_GPU(0),
                                  O_RDWR, &mapfd) != 0 || mapfd < 0) {
        qemu_log("nvkvm-gpu[%s] M5.8 doorbell: usermode map-fd open failed\n", s->chip->name);
        return;
    }
    struct nv_ioctl_nvos33_parameters_with_fd mm;
    memset(&mm, 0, sizeof(mm));
    mm.h_client = nvkvm_m2_client(s, client);
    mm.h_device = hDev;
    mm.h_memory = hUM;
    mm.length   = 0x10000;                        /* NVC361_NV_USERMODE__SIZE = 64 KiB */
    mm.fd       = (int32_t)maph;
    unsigned int mc = (3u << 30) | ((unsigned int)sizeof(mm) << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_MAP_MEMORY;
    uint32_t mnv = 0; uint64_t mf = 0;
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, mc,
                                 &mm, sizeof(mm), NULL, 0, 0, &mnv, &mf);
    if (rc != 0 || mm.status != 0) {
        qemu_log("nvkvm-gpu[%s] M5.8 doorbell: usermode RM_MAP_MEMORY rc=%d st=0x%x\n",
                 s->chip->name, rc, mm.status);
        return;
    }
    void *qva = mmap(NULL, 0x10000, PROT_READ | PROT_WRITE, MAP_SHARED, mapfd, 0);
    if (qva == MAP_FAILED) {
        qemu_log("nvkvm-gpu[%s] M5.8 doorbell: usermode mmap failed: %s\n",
                 s->chip->name, strerror(errno));
        return;
    }
    s->m2_usermode_qva = qva;
    uint8_t tp[4]; memset(tp, 0, sizeof(tp));
    uint32_t tst = 0xffff;
    int trc = nvkvm_m2_control1(s, client, s->m2_gr_channel, 0xc36f0108u, tp, 4, &tst);
    s->m2_gr_token = ldl_le_p(tp);
    s->m2_doorbell_ready = (trc == 0 && tst == 0);
    qemu_log("nvkvm-gpu[%s] M5.8 doorbell: usermode qva=%p GR chan=0x%08x WORK_SUBMIT_TOKEN "
             "trc=%d st=0x%x token=0x%08x -> %s\n", s->chip->name, qva, s->m2_gr_channel,
             trc, tst, s->m2_gr_token,
             s->m2_doorbell_ready ? "READY (ring deferred until pushbuffers mapped+scheduled)"
                                  : "TOKEN-FAILED");
    /* M5.8: schedule the host GR TSG so a future doorbell ring actually runs it. The guest's
     * GPFIFO_SCHEDULE is a control (not forwarded by shadow_fwd), so the host TSG is idle
     * until we schedule it. NVA06C_CTRL_CMD_GPFIFO_SCHEDULE (0xa06c0101) on the TSG,
     * NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS{bEnable,bSkipSubmit,bSkipEnable}. Safe (no ring). */
    if (s->m2_gr_tsg) {
        uint8_t sp[3]; memset(sp, 0, sizeof(sp)); sp[0] = 1;   /* {bEnable=1,bSkipSubmit,bSkipEnable} */
        uint32_t sst = 0xffff;
        int src = nvkvm_m2_control1(s, client, s->m2_gr_tsg, 0xa06c0101u, sp, sizeof(sp), &sst);
        if (src == 0 && sst == 0) {
            nvkvm_m2_tsg_sched_mark(s, client, s->m2_gr_tsg);   /* #12 cont.34 / #14 pair-keyed */
        }
        qemu_log("nvkvm-gpu[%s] M5.8 doorbell: GPFIFO_SCHEDULE TSG=0x%08x rc=%d st=0x%x%s\n",
                 s->chip->name, s->m2_gr_tsg, src, sst,
                 (src == 0 && sst == 0) ? "  OK SCHEDULED" : "  <-- ERR");
    }
}

/* M5.9: resolve a GR-VAS guest VA -> guest-FB phys by trying each snooped VAS PDB (FB leaf
 * only; sysmem leaves are the GPU->CPU DMA path, handled elsewhere). 0 on miss.
 * #14: `client` scopes the probe to VASes NOT provably another process's (two concurrent
 * processes use identical guest VAs, so a client-blind first-hit walk resolved process B's
 * GPFIFO/pushbuffer to A's phys).  Kernel roots are never foreign; single process:
 * identical to the old blind walk. */
static uint64_t nvkvm_m2_resolve_fb_inner(NvkvmGpuEmul *s, uint32_t client, uint64_t va)
{
    for (int v = 0; v < s->chan_vas_n; v++) {
        if (client && nvkvm_m2_multiproc(s) && nvkvm_m2_vas_foreign(s, v, client)) { continue; }
        bool sy = false;
        uint64_t p = nvkvm_walk_pdb(s, s->chan_vas[v].pdb, va, &sy);
        if (p != NVKVM_GMMU_FAULT && !sy) { return p; }
    }
    return 0;
}
static uint64_t nvkvm_m2_resolve_fb(NvkvmGpuEmul *s, uint32_t client, uint64_t va)
{
    uint64_t t0 = nvkvm_now_ns();
    uint64_t r = nvkvm_m2_resolve_fb_inner(s, client, va);
    nvkvm_t_resolve_ns += nvkvm_now_ns() - t0; nvkvm_t_resolve_calls++;
    return r;
}
/* M5.9: has this VA already been backed+mapped? (dedup repeated pushbuffers). Adds if new. */
static bool nvkvm_m2_va_seen(NvkvmGpuEmul *s, uint32_t client, uint64_t va)
{
    nvkvm_t_vaseen_calls++;
    for (int i = 0; i < s->m2_mapped_va_n; i++) {
        nvkvm_t_vaseen_iters++;
        if (s->m2_mapped_va[i].va == va && s->m2_mapped_va[i].client == client) {
            return true;
        }
    }
    if (s->m2_mapped_va_n < NVKVM_MAX_MAPPED_VA) {
        s->m2_mapped_va[s->m2_mapped_va_n].client = client;
        s->m2_mapped_va[s->m2_mapped_va_n].va     = va;
        s->m2_mapped_va[s->m2_mapped_va_n].gpa    = 0;
        s->m2_mapped_va[s->m2_mapped_va_n].hmem   = 0;
        s->m2_mapped_va[s->m2_mapped_va_n].reback = 0;
        s->m2_mapped_va_n++;
    }
    return false;
}

/* M5.51: PURE check (no mark) + explicit mark. The legacy nvkvm_m2_va_seen() above is
 * check-AND-mark, which POISONS a VA whose backing then FAILS: it's marked seen but never
 * backed, so every later sweep skips it (backed=0 forever, coverage ending exactly at that
 * buffer — the recurring cup2-dp / matmul-d_out fault). For backing sites that can fail,
 * use va_check() first and only va_mark() on SUCCESS, so a failed/transient back is retried. */
static bool nvkvm_m2_va_check(NvkvmGpuEmul *s, uint32_t client, uint64_t va)
{
    nvkvm_t_vaseen_calls++;
    for (int i = 0; i < s->m2_mapped_va_n; i++) {
        nvkvm_t_vaseen_iters++;
        if (s->m2_mapped_va[i].va == va && s->m2_mapped_va[i].client == client) {
            return true;
        }
    }
    return false;
}
static void nvkvm_m2_va_mark(NvkvmGpuEmul *s, uint32_t client, uint64_t va)
{
    if (nvkvm_m2_va_check(s, client, va)) { return; }
    if (s->m2_mapped_va_n < NVKVM_MAX_MAPPED_VA) {
        s->m2_mapped_va[s->m2_mapped_va_n].client = client;
        s->m2_mapped_va[s->m2_mapped_va_n].va     = va;
        s->m2_mapped_va[s->m2_mapped_va_n].gpa    = 0;
        s->m2_mapped_va[s->m2_mapped_va_n].hmem   = 0;
        s->m2_mapped_va[s->m2_mapped_va_n].reback = 0;
        s->m2_mapped_va_n++;
    }
}

/* #12 cont.31: GPA-aware find/mark for the SYSMEM backing path.  The table entry is the
 * forward-populated VA->GPA truth for host-pinned guest sysmem: find returns the entry
 * index (or -1), mark records the resolved GPA + the pinning OS-descriptor handle so a
 * staleness re-back can free the old pin. */
static int nvkvm_m2_va_find(NvkvmGpuEmul *s, uint32_t client, uint64_t va)
{
    nvkvm_t_vaseen_calls++;
    for (int i = 0; i < s->m2_mapped_va_n; i++) {
        nvkvm_t_vaseen_iters++;
        if (s->m2_mapped_va[i].va == va && s->m2_mapped_va[i].client == client) {
            return i;
        }
    }
    return -1;
}
static void nvkvm_m2_va_mark_gpa(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                 uint64_t gpa, uint32_t hmem)
{
    int i = nvkvm_m2_va_find(s, client, va);
    if (i < 0) {
        if (s->m2_mapped_va_n >= NVKVM_MAX_MAPPED_VA) { return; }
        i = s->m2_mapped_va_n++;
        s->m2_mapped_va[i].client = client;
        s->m2_mapped_va[i].va     = va;
        s->m2_mapped_va[i].reback = 0;
    }
    s->m2_mapped_va[i].gpa  = gpa;
    s->m2_mapped_va[i].hmem = hmem;
}

/* #12 cont.31: free a host RM object created for a Mode-2 backing (e.g. a stale
 * OS-descriptor pin).  Mirrors the shadow_fwd fn=10 path.  RM cascades the free to the
 * object's DMA mappings, so freeing the hMem also unmaps its FIXED map_dma from the GR
 * VAS — the precondition for re-backing the same VA at a new GPA (else st=0x51). */
static void nvkvm_m2_host_rmfree(NvkvmGpuEmul *s, uint32_t client, uint32_t parent,
                                 uint32_t hobj)
{
    struct nvos00_parameters f;
    memset(&f, 0, sizeof(f));
    f.h_root = nvkvm_m2_client(s, client);
    f.h_object_parent = nvkvm_m2_client_known(s, parent) ? nvkvm_m2_client(s, parent)
                                                         : parent;
    f.h_object_old = hobj;
    unsigned int fc = (3u << 30) | ((unsigned int)sizeof(f) << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_FREE;
    uint32_t fst = 0; uint64_t ff = 0;
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, fc,
                                 &f, sizeof(f), NULL, 0, 0, &fst, &ff);
    qemu_log("nvkvm-gpu[%s] #12 host-rmfree client=0x%08x obj=0x%08x rc=%d st=0x%x\n",
             s->chip->name, client, hobj, rc, f.status);
}

/* M6.5 (item-4 DISCOVERY+backing): place a contiguous guest-RAM SYSMEM run at its GR VA in
 * the host GR VASpace, so the host GPU can DMA into the guest's actual buffer. Reuses the
 * M6.2/M6.3b primitive chain: gpa->stub VA (memfd, 1:1) -> OS_DESCRIPTOR (host RM pins guest
 * RAM) -> per-client GR virtmem mapper -> FIXED map_dma at the guest VA. (st=0x51 = the VA is
 * already host-resident, e.g. self-promoted GR ctx — treated as success, do not re-place.)
 * out_hmem (optional): the OS-descriptor handle on PLACED success, 0 on ALREADY/failure. */
static bool nvkvm_m2_back_and_map_sys_ex(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                         uint64_t gpa, uint64_t size, uint32_t *out_hmem)
{
    if (out_hmem) { *out_hmem = 0; }
    uint32_t hDev = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == client) { hDev = s->m2_devvas[i].dev; break; }
    }
    if (!hDev || !size) { return false; }
    uint64_t sva = nvkvm_m2_gpa_to_stub_va(s, gpa);
    if (!sva) { return false; }
    uint32_t hMem = 0xdf000000u | (s->m2_databuf_next++ & 0xffffu);
    uint32_t ost = 0xffff;
    if (nvkvm_m2_os_descriptor(s, client, hDev, hMem, sva, size, &ost) != 0 || ost != 0) {
        return false;
    }
    uint32_t hVirt = nvkvm_m2_grmapper(s, client);
    if (!hVirt) { return false; }
    uint32_t mst = 0xffff; uint64_t outva = 0;
    int mrc = nvkvm_m2_map_dma(s, client, hDev, hVirt, hMem, 0, size, true, va, &mst, &outva);
    bool ok = (mrc == 0 && mst == 0 && outva == va);
    bool already = (mst == 0x51u);
    qemu_log("nvkvm-gpu[%s] M6.5 back_sys VA=0x%llx gpa=0x%llx size=0x%llx -> hMem=0x%08x "
             "os_st=0x%x map rc=%d st=0x%x %s\n", s->chip->name, (unsigned long long)va,
             (unsigned long long)gpa, (unsigned long long)size, hMem, ost, mrc, mst,
             ok ? "  PLACED" : already ? "  ALREADY-MAPPED" : "  <-- ERR");
    if (out_hmem) { *out_hmem = ok ? hMem : 0; }
    return ok || already;
}
static bool nvkvm_m2_back_and_map_sys(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                      uint64_t gpa, uint64_t size)
{
    return nvkvm_m2_back_and_map_sys_ex(s, client, va, gpa, size, NULL);
}

/* M7 R2: the unified gpu_memory_object backing primitive (replaces back_and_map's split
 * FB-overlay-vs-map). Allocates ONE blank host vidmem object and double-mmaps it:
 *   CPU view  — cpu_qva, registered in the GPGA table so guest BAR1/PRAMIN reads of `gpga`
 *               resolve (nvkvm_fb_host_overlay) to this object (replaces dead fb_pages);
 *   GPU view  — FIXED map_dma at the guest VA into the host GR VAS (the host GPU sees the
 *               same bytes). 0x51 = host self-promoted its own object at this VA -> our GPU
 *               view isn't placed (gr_va=0); R3 makes the host adopt OURS instead.
 * One nvkvm/RM handle backs both views = coherent. Returns obj_idx, or -1. Idempotent caller
 * (dedup by va via m2_va_seen). */
static int nvkvm_m2_gpga_obj_ex(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                                uint64_t gpga, uint64_t size, bool gpu_only)
{
    if (s->m2_objs_n >= 1024 || s->m2_gpga_n >= 2048) {
        return -1;
    }
    uint32_t hDev = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == client) { hDev = s->m2_devvas[i].dev; break; }
    }
    if (!hDev || !size) {
        return -1;
    }
    uint64_t asize = (size + 0xffffu) & ~0xffffull;        /* 64 KiB granular (host alloc/map) */
    uint64_t tsize = (size + 0xfffu)  & ~0xfffull;          /* 4 KiB true run length (overlay) */
    /* M5.44 (supersedes the M5.43 whole-run SKIP): an m2_fbback[] entry inside this run (e.g.
     * a COPY channel's real USERD) stays authoritative because nvkvm_fb_host_overlay now scans
     * fbback FIRST. Rejecting the whole run here was WRONG: the 2 MiB COPY channel-pool run
     * (GPGA 0x4200000) holds all 16 GPFIFOs+USERDs, and skipping it left the GPFIFO entries
     * with no host backing and NO GPU-view map at the guest VA — the host channel had nothing
     * to fetch. Register the run; just log the overlap for observability. */
    for (int i = 0; i < s->m2_fbback_n; i++) {
        uint64_t fb0 = s->m2_fbback[i].fb_base, fb1 = fb0 + s->m2_fbback[i].size;
        if (gpga < fb1 && fb0 < gpga + tsize) {
            qemu_log("nvkvm-gpu[%s] M5.44 gpga_obj OVERLAP-OK: gpga=0x%llx size=0x%llx covers "
                     "fbback[%d] [0x%llx,0x%llx) (fbback wins in the overlay)\n",
                     s->chip->name, (unsigned long long)gpga, (unsigned long long)tsize, i,
                     (unsigned long long)fb0, (unsigned long long)fb1);
        }
    }
    uint32_t hMem = 0xda000000u | (s->m2_databuf_next++ & 0xffffu);
    /* CE-fwd map-on-touch: gpu_only is only SAFE when the run is still BLANK at walk time (a
     * pure CE-copy dst / freshly-alloc'd-but-unwritten user buffer). If the guest already wrote
     * ANY page (a pushbuffer/GPFIFO/src laid down before the doorbell), those bytes are needed by
     * the host NOW -> fall back to the eager CPU-mapped + copy-preserve path. This blank-vs-written
     * test (not a window or m2_gr_client gate, both of which lost the ordering race in m565) is
     * what makes ordering irrelevant: written => eager-map+copy, blank => lazy promote-on-touch. */
    if (gpu_only) {
        for (uint64_t off = 0; off < tsize; off += 4096) {
            if (nvkvm_fb_page(s, gpga + off, false)) { gpu_only = false; break; }
        }
    }
    struct nvkvm_host_map hm;
    if (gpu_only) {
        /* CE-forward P1: real host vidmem object, NO CPU view (zero host BAR1). The dst is a
         * user-CE copy destination the guest CPU never reads through the GPA window. */
        if (!nvkvm_m2_host_alloc_vidmem_gpu_only(s, client, hDev, hMem, asize, &hm)) {
            return -1;
        }
    } else if (!nvkvm_m2_host_alloc_map_vidmem(s, client, hDev, hMem, asize, &hm)) {
        return -1;
    }
    /* M5.44: preserve any bytes the guest already wrote to this GPGA range via the local
     * fb_pages BEFORE this object existed (back_and_map's copy_content equivalent) — e.g.
     * GPFIFO entries laid down pre-doorbell. Unwritten pages are sparse-zero = blank.
     * Skipped for gpu_only (no CPU view to copy into; the host CE writes it in Phase B). */
    if (!gpu_only) {
        for (uint64_t off = 0; off < tsize; off += 4096) {
            uint8_t *gp = nvkvm_fb_page(s, gpga + off, false);
            if (gp) { memcpy((uint8_t *)hm.qva + off, gp, 4096); }
        }
    }
    /* GPU view: FIXED-map into the host GR VAS at the guest VA. */
    uint32_t hVirt = nvkvm_m2_grmapper(s, client);
    uint32_t mst = 0xffff; uint64_t outva = 0; int mrc = -1;
    if (hVirt) {
        mrc = nvkvm_m2_map_dma(s, client, hDev, hVirt, hm.h_mem, 0, asize, true, va, &mst, &outva);
    }
    bool gpu_mapped = (mrc == 0 && mst == 0 && outva == va);
    int oi = s->m2_objs_n++;
    s->m2_objs[oi].mode = 0;                                /* physical (FB-backed general) */
    s->m2_objs[oi].cpu_qva = hm.qva;
    s->m2_objs[oi].size = asize;
    s->m2_objs[oi].client = client;
    s->m2_objs[oi].hMemory = hm.h_mem;
    s->m2_objs[oi].gr_va = gpu_mapped ? va : 0;
    s->m2_objs[oi].promote = gpu_only ? 1 : 0;   /* 1 => CPU view deferred to first touch */
    int gi = s->m2_gpga_n++;
    s->m2_gpga[gi].gpga_base = gpga;
    s->m2_gpga[gi].size = asize;
    s->m2_gpga[gi].obj_idx = oi;
    s->m2_gpga[gi].off = 0;
    s->m2_gpga[gi].readable = true;
    s->m2_gpga[gi].writable = true;
    s->m2_gpga_idx_dirty = true;   /* M5.11c: stale the sorted index (rebuilt lazily on next lookup) */
    qemu_log("nvkvm-gpu[%s] M7 R2 gpga_obj%s: va=0x%llx gpga=0x%llx size=0x%llx hMem=0x%08x "
             "cpu_qva=%p gpu_mapped=%d(st=0x%x) obj=%d gpga_n=%d\n", s->chip->name,
             gpu_only ? "[gpu_only]" : "", (unsigned long long)va, (unsigned long long)gpga,
             (unsigned long long)asize, hm.h_mem, hm.qva, gpu_mapped, mst, oi, s->m2_gpga_n);
    return oi;
}

/* Default (CPU-mapped) gpga_obj — the working-milestone path. */
static int nvkvm_m2_gpga_obj(NvkvmGpuEmul *s, uint32_t client, uint64_t va,
                             uint64_t gpga, uint64_t size)
{
    return nvkvm_m2_gpga_obj_ex(s, client, va, gpga, size, false);
}

/* M6.5 leaf accumulator: coalesce contiguous (VA,GPA,sys) leaf pages into runs, back each
 * SYSMEM run via the primitive above. (Vidmem leaves are host-resident already — M6.4.) */
struct nvkvm_leaf_acc { NvkvmGpuEmul *s; uint32_t client; uint64_t va0, gpa0, len;
                        int sys, runs, backed; uint64_t sysbytes, vidbytes;
                        /* #13: when nonzero, this walk is over a COMPUTE VAS rooted at this
                         * PDB — record every visited vidmem table page's {page,level,vabase}
                         * into m2_cpt[] so nvkvm_m2_cpt_sync_at_release can later decode a
                         * CE-written page against it.  0 = don't record (kernel VASes,
                         * dry-run probes, per-channel cvas mirrors). */
                        uint64_t cpt_pdb;
                        /* M5.13 DRY-RUN probe: when dry, do NOT back anything; just log which
                         * leaf (VA range) maps the `target` guest-phys. Used to definitively
                         * identify the completion-semaphore page's owning PDB + GR-VA before we
                         * commit to backing it (blindly backing e.g. a BAR2 walk would overlay
                         * the guest's own page tables with blank objects -> wedge). */
                        bool dry; uint64_t target; const char *tag; bool found; };

static void nvkvm_m2_leaf_flush(struct nvkvm_leaf_acc *a)
{
    if (a->len == 0) { return; }
    a->runs++;
    /* (M5.48b "skip top-of-heap" was tried here and REVERTED: nvkprobe proved the host RM's
     * internal allocator is BOTTOM-UP (~0x120000000) — the top region is guest-only, and the
     * guest's high leaves (its GR-ctx pool, e.g. 0x78dd76000000) ARE referenced by the
     * executing channel (FE faulted VIRT_WRITE at pool+0xe00000), so they MUST be mirrored. */
    if (a->dry) {
        if (a->target >= a->gpa0 && a->target < a->gpa0 + a->len) {
            uint64_t hit_va = a->va0 + (a->target - a->gpa0);
            a->found = true;
            qemu_log("nvkvm-gpu[GA106] M5.13 PROBE[%s] *** target gpa=0x%llx FOUND: %s "
                     "run VA=0x%llx gpa=0x%llx len=0x%llx -> sem GR-VA=0x%llx ***\n",
                     a->tag ? a->tag : "?", (unsigned long long)a->target,
                     a->sys ? "SYS" : "VID", (unsigned long long)a->va0,
                     (unsigned long long)a->gpa0, (unsigned long long)a->len,
                     (unsigned long long)hit_va);
        }
        a->len = 0;
        return;
    }
    if (a->sys) {
        a->sysbytes += a->len;
        /* M5.52: dedup PER 2-MiB-ALIGNED CHUNK, not per whole-run va0 (mirrors the M5.48d
         * vidmem fix; the sysmem path was never updated). A guest sysmem run GROWS as the
         * app maps more of a contiguous allocation; re-coalesced it keeps the SAME va0, so
         * the old whole-run va_check(va0) skipped the grown TAIL forever -> a residency hole
         * at the old run end -> host CE/GR FAULT_PTE there. THIS is the cup4 cuCtxCreate hang:
         * a backed run [0x7877074e0000,+0x120000) ending at 0x787707600000, then a 0x80000
         * tail in the NEXT 2-MiB chunk [0x787707600000,..) that the va0-keyed skip never
         * backed -> Xid 31 CE2 FAULT_PTE @0x787707600000. Per-chunk keys are stable across
         * re-walks so only genuinely-new chunks back; back_and_map_sys is idempotent
         * (st=0x51 ALREADY-MAPPED) for any head re-touch. M5.51 mark-on-success preserved:
         * an unbacked chunk stays unmarked so a later sweep retries. (Sub-2-MiB runs back
         * identically to before -> no regression to the cup3 path, whose sysmem runs are all
         * <2 MiB.) The >=1 GiB single-key path is kept for any giant sysmem alias. */
        if (a->len < 0x40000000ull) {
            for (uint64_t off = 0; off < a->len; ) {
                uint64_t cva  = a->va0 + off;
                uint64_t cgpa = a->gpa0 + off;
                uint64_t next = (cva + 0x200000ull) & ~0x1fffffull;
                uint64_t clen = (next - cva < a->len - off) ? next - cva : a->len - off;
                int ei = nvkvm_m2_va_find(a->s, a->client, cva);
                if (ei < 0) {
                    uint32_t newh = 0;
                    if (nvkvm_m2_back_and_map_sys_ex(a->s, a->client, cva, cgpa, clen,
                                                     &newh)) {
                        nvkvm_m2_va_mark_gpa(a->s, a->client, cva, cgpa, newh);
                        a->backed++;
                    }
                } else if (a->s->m2_mapped_va[ei].gpa &&
                           a->s->m2_mapped_va[ei].gpa != cgpa) {
                    /* #12 cont.31 STALE MAPPING: this {client,VA} was backed when it
                     * resolved to a DIFFERENT guest page — the guest tore the mapping
                     * down and re-created it (2nd cuCtxCreate re-allocs channels and
                     * the 16-slot completion-semaphore pool at the SAME VAs but fresh
                     * pages).  The host GR VAS still targets the OLD page, so every
                     * host completion write lands in stale memory: libcuda's wait-ALL
                     * on the pool semaphores (VA 0x20440f000) spins forever = the #12
                     * hang, and the stray writes corrupt whoever now owns the old page
                     * (guest UVM MAX_JUMP "semaphore jumped backwards" asserts).
                     * Fix per the address-table directive: the walk IS the forward-
                     * populated truth — free the stale host pin (RM cascades the free
                     * to its map_dma, vacating the VA) and re-back at the new GPA. */
                    if (a->s->m2_mapped_va[ei].reback >= 64) {
                        qemu_log("nvkvm-gpu[GA106] #12 STALE-SYS va=0x%llx reback cap "
                                 "hit (ping-pong?) old_gpa=0x%llx new_gpa=0x%llx\n",
                                 (unsigned long long)cva,
                                 (unsigned long long)a->s->m2_mapped_va[ei].gpa,
                                 (unsigned long long)cgpa);
                    } else {
                        uint64_t ogpa = a->s->m2_mapped_va[ei].gpa;
                        if (a->s->m2_mapped_va[ei].hmem) {
                            uint32_t fdev = 0;
                            for (int d = 0; d < a->s->m2_devvas_n; d++) {
                                if (a->s->m2_devvas[d].client == a->client) {
                                    fdev = a->s->m2_devvas[d].dev; break;
                                }
                            }
                            nvkvm_m2_host_rmfree(a->s, a->client, fdev,
                                                 a->s->m2_mapped_va[ei].hmem);
                            a->s->m2_mapped_va[ei].hmem = 0;
                        }
                        uint32_t newh = 0;
                        bool ok = nvkvm_m2_back_and_map_sys_ex(a->s, a->client, cva,
                                                               cgpa, clen, &newh);
                        qemu_log("nvkvm-gpu[GA106] #12 STALE-SYS re-back client=0x%08x "
                                 "va=0x%llx gpa 0x%llx -> 0x%llx len=0x%llx hMem=0x%08x "
                                 "%s (reback=%u)\n", a->client,
                                 (unsigned long long)cva, (unsigned long long)ogpa,
                                 (unsigned long long)cgpa, (unsigned long long)clen,
                                 newh, ok ? "OK" : "<-- ERR",
                                 a->s->m2_mapped_va[ei].reback + 1);
                        if (ok) {
                            a->s->m2_mapped_va[ei].gpa  = cgpa;
                            a->s->m2_mapped_va[ei].hmem = newh;
                            a->s->m2_mapped_va[ei].reback++;
                            a->backed++;
                        }
                    }
                }
                off += clen;
            }
        } else if (!nvkvm_m2_va_check(a->s, a->client, a->va0)) {
            if (nvkvm_m2_back_and_map_sys(a->s, a->client, a->va0, a->gpa0, a->len)) {
                nvkvm_m2_va_mark(a->s, a->client, a->va0);
                a->backed++;
            }
        }
    } else {
        /* M6.6 (user direction): vidmem leaf — back with a BLANK host vidmem object,
         * double-mmapped (back_and_map: m2_fbback CPU side at the FB addr + FIXED map_dma
         * GPU side at the guest VA). The buffer is OPAQUE: the guest manages its contents
         * and the host GPU fills the golden ctx on execution — both sides share ONE
         * coherent host object. Replaces the dead malloc'd fb_pages backing that the host
         * GPU can't touch (the cuCtxCreate crash = libcuda reads that dead vidmem as zero).
         * 0x51 from map_dma = the host self-promoted its own object at this VA (no overlay;
         * needs the avoid-self-promotion path). copy_content=false (blank). */
        a->vidbytes += a->len;
        /* M5.48d: dedup PER 2-MiB-ALIGNED CHUNK, not per run. The guest GROWS its vid
         * mappings (GR-ctx pool leaves appear submission-by-submission); a grown run
         * re-coalesces to the SAME start VA, so the old whole-run va_seen check skipped
         * the new tail forever — the host CE then faulted VIRT_WRITE at exactly
         * old-run-end (0x717ccc000000+0xe00000). Aligned chunk keys are stable across
         * re-walks, so only genuinely-new chunks get backed. Giant runs (e.g. the
         * kernel's whole-FB linear alias, 12 GiB) keep the legacy single-key skip. */
        if (a->len < 0x40000000ull) {   /* chunk everything < 1 GiB; the 12-GiB whole-FB
                                         * linear alias keeps the legacy single-key skip */
            for (uint64_t off = 0; off < a->len; ) {
                uint64_t cva  = a->va0 + off;
                uint64_t next = (cva + 0x200000ull) & ~0x1fffffull;
                uint64_t clen = (next - cva < a->len - off) ? next - cva : a->len - off;
                /* M5.51: check-then-back-then-MARK-ON-SUCCESS. The old check-and-mark
                 * va_seen() poisoned a chunk whose gpga_obj failed (marked seen, never
                 * backed -> backed=0 forever, coverage ending exactly at it = the cup2-dp /
                 * matmul-d_out fault). Now an unbacked chunk stays unmarked so a later
                 * sweep retries; on persistent failure the diag below names why. */
                if (!nvkvm_m2_va_check(a->s, a->client, cva)) {
                    /* CE-fwd map-on-touch: REQUEST gpu_only backing for a compute client's
                     * vidmem leaf (the GR compute client + libcuda's CE-copy clients). This is
                     * the DEFAULT now (no window, no m2_gr_client-at-decode gate — both lost the
                     * ordering race in m565). gpga_obj_ex keeps it gpu_only ONLY if the run is
                     * blank at walk time (real host vidmem + GPU-side map_dma, zero host BAR1);
                     * a written run eager-maps+copies as before. A blank gpu_only leaf gets its
                     * CPU view lazily on the first guest touch (promote-on-touch). Non-compute
                     * clients keep the exact milestone path (go=false). Only under m2cefwd. */
                    bool go = a->s->m2cefwd &&
                              (nvkvm_m2_is_gr_client(a->s, a->client) ||   /* #14: any process */
                               nvkvm_m2_is_user_ce(a->s, a->client));
                    if (nvkvm_m2_gpga_obj_ex(a->s, a->client, cva, a->gpa0 + off, clen, go) >= 0) {
                        nvkvm_m2_va_mark(a->s, a->client, cva);
                        a->backed++;  /* M7 R2: unified gpu_memory_object (GPGA + GR-VAS) */
                    } else {
                        qemu_log("nvkvm-gpu[%s] M5.51 gpga_obj FAILED (retry next sweep) "
                                 "client=0x%08x cva=0x%llx gpa=0x%llx clen=0x%llx "
                                 "objs_n=%d gpga_n=%d\n", a->s->chip->name, a->client,
                                 (unsigned long long)cva, (unsigned long long)(a->gpa0 + off),
                                 (unsigned long long)clen, a->s->m2_objs_n, a->s->m2_gpga_n);
                    }
                }
                off += clen;
            }
        } else if (!nvkvm_m2_va_seen(a->s, a->client, a->va0) &&
                   nvkvm_m2_gpga_obj(a->s, a->client, a->va0, a->gpa0, a->len) >= 0) {
            a->backed++;
        }
    }
    a->len = 0;
}

static void nvkvm_m2_leaf_add(struct nvkvm_leaf_acc *a, uint64_t va, uint64_t gpa,
                              int sys, uint64_t pgsz)
{
    if (a->len && a->sys == sys && va == a->va0 + a->len && gpa == a->gpa0 + a->len) {
        a->len += pgsz;                          /* extend run (VA+GPA both contiguous) */
        return;
    }
    nvkvm_m2_leaf_flush(a);
    a->va0 = va; a->gpa0 = gpa; a->sys = sys; a->len = pgsz;
}

/* #13: compute-VAS page-table page ownership (m2_cpt[]) — see the struct comment.
 * A page is recorded for every table the compute-VAS walk visits, at any level
 * (0..2 = PD3/PD2/PD1, 3 = PD0, 4 = small (4 KiB) leaf PT, 5 = big (64 KiB) leaf
 * PT; big PTs are only 256 B, so their 4 KiB page is what's keyed).  `vabase` is
 * the VA that this level covers, so the exact table base can be re-derived from
 * the page and decoded without a root walk. */
#define NVKVM_M2_CPT_SLOTS 4096
static void nvkvm_m2_cpt_record(NvkvmGpuEmul *s, uint64_t pdb, uint64_t tbl,
                                int level, uint64_t vabase)
{
    uint64_t base = tbl & ~0xfffull;
    if (!base) { return; }
    uint32_t h = (uint32_t)((base >> 12) * 2654435761u) & (NVKVM_M2_CPT_SLOTS - 1);
    for (int p = 0; p < NVKVM_M2_CPT_SLOTS; p++) {
        if (s->m2_cpt[h].page == 0) {
            if (s->m2_cpt_n >= NVKVM_M2_CPT_SLOTS * 3 / 4) { return; }  /* full: stop (bounded) */
            s->m2_cpt_n++;
            break;
        }
        if (s->m2_cpt[h].page == base) { break; }  /* re-record: update in place */
        h = (h + 1) & (NVKVM_M2_CPT_SLOTS - 1);
    }
    s->m2_cpt[h].page = base; s->m2_cpt[h].pdb = pdb;
    s->m2_cpt[h].vabase = vabase; s->m2_cpt[h].level = (uint8_t)level;
    if (base < s->m2_cpt_lo) { s->m2_cpt_lo = base; }
    if (base > s->m2_cpt_hi) { s->m2_cpt_hi = base; }
}
static int nvkvm_m2_cpt_find(NvkvmGpuEmul *s, uint64_t addr)
{
    uint64_t base = addr & ~0xfffull;
    uint32_t h = (uint32_t)((base >> 12) * 2654435761u) & (NVKVM_M2_CPT_SLOTS - 1);
    for (int p = 0; p < NVKVM_M2_CPT_SLOTS; p++) {
        if (s->m2_cpt[h].page == 0) { return -1; }
        if (s->m2_cpt[h].page == base) { return (int)h; }
        h = (h + 1) & (NVKVM_M2_CPT_SLOTS - 1);
    }
    return -1;
}
/* #13: is `pdb` the page-directory root of a USER-COMPUTE address space?  True for
 * a chan_vas[] root that is either (a) UVM-managed — handed over via SET_PAGE_
 * DIRECTORY, the transport that exists exactly to register a user process's GPU VA
 * space (cuMemAlloc pointers live there; issued under UVM's session client, so the
 * hClient can't discriminate) — or (b) registered by the GR compute client itself /
 * one of libcuda's user-CE clients.  The kernel scrubber / CeUtils internal VASes
 * qualify under neither — their transient tables must never trigger backing (that
 * was round-4 part2's cuCtxCreate breaker).  Re-checked at trigger time so a freed
 * VAS's stale m2_cpt entries (pruned from chan_vas[] at teardown) are inert. */
static bool nvkvm_m2_pdb_is_compute(NvkvmGpuEmul *s, uint64_t pdb)
{
    if (!pdb || !s->m2_gr_client) { return false; }
    for (int i = 0; i < s->chan_vas_n; i++) {
        if (s->chan_vas[i].pdb != pdb) { continue; }
        uint32_t cl = s->chan_vas[i].client;
        if (s->chan_vas[i].uvm) { return true; }
        if (cl && (nvkvm_m2_is_gr_client(s, cl) ||   /* #14: any process's compute client */
                   nvkvm_m2_is_user_ce(s, cl))) { return true; }
    }
    return false;
}

/* Decode ONE leaf PTE -> accumulate its page.  Shared by the full pt_enum walk and
 * the #13 CE-PT-write trigger (which decodes only the entries a write covered). */
static void nvkvm_m2_pt_enum_pte(NvkvmGpuEmul *s, uint64_t pte, uint64_t va,
                                 uint64_t pgsz, struct nvkvm_leaf_acc *a)
{
    if (!(pte & 1)) { return; }
    uint32_t apt = (uint32_t)((pte >> 1) & 0x3); uint64_t pg; int sys;
    if (apt == 0) { pg = ((pte >> 8) & ((1ull << 25) - 1)) << 12; sys = 0; }
    else if (apt == 2 || apt == 3) { pg = ((pte >> 8) & ((1ull << 46) - 1)) << 12; sys = 1; }
    else { return; }
    nvkvm_m2_leaf_add(a, va, pg, sys, pgsz);
}

/* Decode ONE PD0 dual entry {lo,hi} covering VA `pdva`: a 2 MiB leaf PTE, else
 * descend its small (4 KiB) / big (64 KiB) leaf PTs.  Shared as above. */
static void nvkvm_m2_pt_enum_pd0e(NvkvmGpuEmul *s, uint64_t lo, uint64_t hi,
                                  uint64_t pdva, struct nvkvm_leaf_acc *a, int *budget)
{
    if (lo & 1) {                                          /* 2 MiB leaf PTE */
        nvkvm_m2_pt_enum_pte(s, lo, pdva, 0x200000ull, a);
        return;
    }
    uint32_t big_ap = (uint32_t)((lo >> 1) & 0x3), small_ap = (uint32_t)((hi >> 1) & 0x3);
    if (small_ap == 1 || small_ap == 2 || small_ap == 3) {
        bool stsys = (small_ap != 1);
        uint64_t st = stsys ? (((hi >> 8) & ((1ull << 46) - 1)) << 12)
                            : (((hi >> 8) & ((1ull << 25) - 1)) << 12);
        if (st && a->cpt_pdb && !stsys) { nvkvm_m2_cpt_record(s, a->cpt_pdb, st, 4, pdva); }
        for (uint32_t j = 0; st && j < 512 && *budget > 0; j++) {
            uint64_t pte = nvkvm_pt_rd64(s, st + (uint64_t)j * 8, stsys); (*budget)--;
            nvkvm_m2_pt_enum_pte(s, pte, pdva | ((uint64_t)j << 12), 0x1000ull, a);
        }
    }
    if (big_ap == 1 || big_ap == 2 || big_ap == 3) {
        bool btsys = (big_ap != 1);
        uint64_t bt = btsys ? (((lo >> 4) & ((1ull << 50) - 1)) << 8)
                            : (((lo >> 4) & ((1ull << 29) - 1)) << 8);
        if (bt && a->cpt_pdb && !btsys) { nvkvm_m2_cpt_record(s, a->cpt_pdb, bt, 5, pdva); }
        for (uint32_t j = 0; bt && j < 32 && *budget > 0; j++) {
            uint64_t pte = nvkvm_pt_rd64(s, bt + (uint64_t)j * 8, btsys); (*budget)--;
            nvkvm_m2_pt_enum_pte(s, pte, pdva | ((uint64_t)j << 16), 0x10000ull, a);
        }
    }
}

/* Recursive GMMU-VER2 descent: PD3->PD2->PD1 (8B PDEs), PD0 (16B dual-PDE / 2 MiB leaf),
 * small (4K) / big (64K) PTs. Mirrors nvkvm_walk_pdb's decode but ENUMERATES every valid
 * leaf instead of resolving one VA. `budget` bounds total entries visited (sparse tables). */
static void nvkvm_m2_pt_enum(NvkvmGpuEmul *s, uint64_t tbl, bool tsys, int level,
                             uint64_t vabase, struct nvkvm_leaf_acc *a, int *budget)
{
    if (*budget <= 0 || tbl == 0) { return; }
    static const struct { int lo, n; } L[3] = { {47, 2}, {38, 512}, {29, 512} };
    if (a->cpt_pdb && !tsys) { nvkvm_m2_cpt_record(s, a->cpt_pdb, tbl, level, vabase); }
    if (level < 3) {
        for (uint32_t i = 0; i < (uint32_t)L[level].n && *budget > 0; i++) {
            uint64_t pde = nvkvm_pt_rd64(s, tbl + (uint64_t)i * 8, tsys);
            /* #13 (matches the walker's PD1-leaf case): a GA10x PD1 entry with bit0 set
             * is itself a 512 MiB LEAF PTE, not a PDE.  Its only known producer is the
             * CeUtils whole-FB identity alias — never DESCEND it as if it pointed at a
             * table (a SYS-aperture leaf would walk garbage) and never BACK it (a
             * 512 MiB alias of the whole heap is not a compute working-set leaf). */
            if (level == 2 && (pde & 1)) { continue; }
            uint32_t ap = (uint32_t)((pde >> 1) & 0x3);
            uint64_t nt; bool ntsys;
            if (ap == 1) { nt = ((pde >> 8) & ((1ull << 25) - 1)) << 12; ntsys = false; }
            else if (ap == 2 || ap == 3) { nt = ((pde >> 8) & ((1ull << 46) - 1)) << 12; ntsys = true; }
            else { continue; }
            nvkvm_m2_pt_enum(s, nt, ntsys, level + 1,
                             vabase | ((uint64_t)i << L[level].lo), a, budget);
        }
        return;
    }
    for (uint32_t i = 0; i < 256 && *budget > 0; i++) {       /* PD0: 256 dual-PDEs */
        uint64_t e = tbl + (uint64_t)i * 16;
        uint64_t lo = nvkvm_pt_rd64(s, e, tsys), hi = nvkvm_pt_rd64(s, e + 8, tsys);
        (*budget)--;
        nvkvm_m2_pt_enum_pd0e(s, lo, hi, vabase | ((uint64_t)i << 21), a, budget);
    }
}

/* #13 THE FIX — forward-populate host GR-VAS backing from the OBSERVED page-table-
 * write transport, keyed by PDB and honoring the guest's own commit point
 * (docs/design/mode2_address_table.md §4.2/§5).
 *
 * The guest (re)maps a compute buffer by writing the compute VAS's page tables via
 * VIRTUAL-dst CE copies through the CeUtils 512 MiB FB alias (resolvable since the
 * walker's PD1-leaf support, part 1 of this fix), then releases the map push's
 * completion semaphore.  The ALREADY-RUNG host GR channel sits in a SEM ACQUIRE on
 * exactly that release and dereferences the new mapping immediately after — and the
 * guest's LAST doorbell precedes the map push, so no doorbell-driven sweep can ever
 * run in between (bench-proven in cup8_iter iter-2: sweeps #20-23 all fire before
 * the remap lands; host CE then Xid-31 FAULT_PDEs one page past the last-backed
 * leaf).  Two halves:
 *
 *  - nvkvm_m2_ce_fb_write_hook (at each CE FB write): if the span landed on a tracked
 *    compute-VAS page-table PAGE, LATCH that page's index (O(1)).  No decode/backing
 *    in the hot loop — a big scrub can hit a PD page thousands of times, and decoding
 *    its subtree per span livelocks (bench-proven: the first per-write attempt hung
 *    with State=R busy-poll and no CTX OK).
 *  - nvkvm_m2_cpt_sync_at_release (BEFORE the push's semaphore release is written):
 *    decode each dirtied page DIRECTLY — from the page itself, NOT a root walk — and
 *    back its leaves with the same coalescing/va_seen-dedup'd sweep primitives.
 *    Direct decode is load-bearing: the guest fills a leaf PT page and links it under
 *    the root a SEPARATE push later, so at this release a root walk of the PDB can't
 *    yet reach the page (bench-proven: root re-walk read runs=0 while the host CE
 *    faulted one page past the last-backed leaf), but the page itself already holds
 *    committed PTEs.  The release is the guest's own commit point for those PTEs, and
 *    the CPU emulation is synchronous, so backing completes before the release is
 *    observable: the §5.1 membar/fence ordering by construction.
 *
 * Scope — why this is cuCtxCreate-safe where round-4's part2 pre-sema sweep was
 * not: part2 swept EVERY chan_vas VAS (kernel scrubber's transient tables, the
 * 12 GiB whole-FB alias) on EVERY kernel-CE sema release.  This decodes ONLY pages a
 * CE write actually landed on, only for compute-VAS PTs (nvkvm_m2_pdb_is_compute,
 * re-checked at decode time so a freed VAS's stale m2_cpt entries are inert), and
 * only committed values the guest just wrote — never a whole-tree walk of possibly-
 * mid-update state (the §6 torn-walk hazard). */

/* Per-level table geometry, indexed by m2_cpt level (0=PD3..2=PD1, 3=PD0, 4=small
 * leaf PT, 5=big leaf PT).  `shift` = VA bits this level's entries stride; `nent` =
 * entries per 4 KiB page; PD0 (level 3) has 16 B dual entries, the rest 8 B. */
static const struct { int shift; uint32_t nent; } nvkvm_m2_cpt_lvl[6] = {
    { 47, 512 }, { 38, 512 }, { 29, 512 }, { 21, 256 }, { 12, 512 }, { 16, 32 },
};
static void nvkvm_m2_cpt_decode_page(NvkvmGpuEmul *s, int ci,
                                     struct nvkvm_leaf_acc *a, int *budget)
{
    uint64_t tbl = s->m2_cpt[ci].page, vabase = s->m2_cpt[ci].vabase;
    int lvl = s->m2_cpt[ci].level;
    uint32_t nent = nvkvm_m2_cpt_lvl[lvl].nent;
    int shft = nvkvm_m2_cpt_lvl[lvl].shift;
    for (uint32_t i = 0; i < nent && *budget > 0; i++) {
        uint64_t va = vabase | ((uint64_t)i << shft);
        if (lvl == 3) {                            /* PD0: 16 B dual PDE / 2 MiB leaf */
            uint64_t lo = nvkvm_pt_rd64(s, tbl + (uint64_t)i * 16, false);
            uint64_t hi = nvkvm_pt_rd64(s, tbl + (uint64_t)i * 16 + 8, false);
            (*budget)--;
            nvkvm_m2_pt_enum_pd0e(s, lo, hi, va, a, budget);
        } else if (lvl >= 4) {                     /* small/big leaf PT: one PTE */
            uint64_t pte = nvkvm_pt_rd64(s, tbl + (uint64_t)i * 8, false);
            (*budget)--;
            nvkvm_m2_pt_enum_pte(s, pte, va, lvl == 4 ? 0x1000ull : 0x10000ull, a);
        } else {                                   /* PD3/PD2/PD1: a PDE (sub-table) */
            uint64_t pde = nvkvm_pt_rd64(s, tbl + (uint64_t)i * 8, false);
            (*budget)--;
            if (lvl == 2 && (pde & 1)) { continue; }   /* 512 MiB leaf (FB alias): never back */
            uint32_t ap = (uint32_t)((pde >> 1) & 0x3);
            uint64_t nt; bool ntsys;
            if (ap == 1) { nt = ((pde >> 8) & ((1ull << 25) - 1)) << 12; ntsys = false; }
            else if (ap == 2 || ap == 3) { nt = ((pde >> 8) & ((1ull << 46) - 1)) << 12; ntsys = true; }
            else { continue; }
            nvkvm_m2_pt_enum(s, nt, ntsys, lvl + 1, va, a, budget);
        }
    }
}
static void nvkvm_m2_cpt_sync_at_release(NvkvmGpuEmul *s)
{
    if (!s->m2_cpt_dirty_n) { return; }
    int ndirty = s->m2_cpt_dirty_n;
    struct nvkvm_leaf_acc a; memset(&a, 0, sizeof(a));
    a.s = s; a.client = s->m2_gr_client;
    int budget = 300000;
    bool rec = s->m2_recording_gr_pt;
    s->m2_recording_gr_pt = true;      /* fresh sub-tables join the tracked sets */
    for (int k = 0; k < s->m2_cpt_dirty_n; k++) {
        int ci = s->m2_cpt_dirty[k];
        s->m2_cpt[ci].dirty = false;
        /* Re-check ownership at decode time: a page freed/re-pointed since the latch
         * (chan_vas[] pruned at teardown) must not back garbage. */
        if (!nvkvm_m2_pdb_is_compute(s, s->m2_cpt[ci].pdb)) { continue; }
        /* #14: back under the compute client that OWNS this PDB's VAS (dup-edge
         * derived), not blanket m2_gr_client — two processes' identical guest VAs
         * must land in their OWN host VASes.  Single process: owner IS m2_gr_client.
         * Flush the run accumulator across an owner change (runs never span it). */
        uint32_t own = nvkvm_m2_pdb_gr_owner(s, s->m2_cpt[ci].pdb);
        if (!own) { own = s->m2_gr_client; }
        if (own != a.client) {
            nvkvm_m2_leaf_flush(&a);
            a.client = own;
        }
        a.cpt_pdb = s->m2_cpt[ci].pdb;   /* newly-descended sub-tables join m2_cpt too */
        nvkvm_m2_cpt_decode_page(s, ci, &a, &budget);
    }
    s->m2_cpt_dirty_n = 0;
    s->m2_recording_gr_pt = rec;
    nvkvm_m2_leaf_flush(&a);
    if (a.backed || budget <= 0) {
        qemu_log("nvkvm-gpu[%s] #13 PT-SYNC@release: dirty_pages=%d runs=%d backed=%d "
                 "(budget_left=%d)\n", s->chip->name, ndirty, a.runs, a.backed, budget);
    }
}

/* #13: one hook for every CPU-emulated CE write that lands in FB.  These writes go
 * through nvkvm_fb_host_ptr / phys_wr32 and BYPASS nvkvm_fb_write — so (a) mirror
 * fb_write's M5.10 tracked-PT-page dirty-arm (a CE-written PTE re-arms the doorbell
 * re-sweep), and (b) latch the touched compute-VAS PT page(s) for the #13 release-
 * time decode above.  A span can cross a page boundary, so latch every page it
 * covers (spans are page-clamped in the copy loop, but the fill path is not). */
static void nvkvm_m2_ce_fb_write_hook(NvkvmGpuEmul *s, uint64_t dst, uint64_t len)
{
    if (s->m2_gr_pt_n && !s->m2_gr_vas_dirty &&
        dst >= s->m2_gr_pt_lo && dst <= s->m2_gr_pt_hi + 0xfffull &&
        nvkvm_m2_gr_pt_contains(s, dst)) {
        s->m2_gr_vas_dirty = true;
    }
    if (!s->m2exec || !s->m2_gr_client || !s->m2_cpt_n || !len) { return; }
    if (dst > s->m2_cpt_hi + 0xfffull || dst + len <= s->m2_cpt_lo) { return; }
    for (uint64_t p = dst & ~0xfffull; p < dst + len; p += 0x1000ull) {
        int ci = nvkvm_m2_cpt_find(s, p);
        if (ci < 0 || s->m2_cpt[ci].dirty) { continue; }
        if (!nvkvm_m2_pdb_is_compute(s, s->m2_cpt[ci].pdb)) { continue; }
        if (s->m2_cpt_dirty_n >= (int)ARRAY_SIZE(s->m2_cpt_dirty)) {
            /* Overflow (unbounded fill hitting many PT pages): drain now so nothing
             * is lost — the current values are as committed as this write makes them. */
            nvkvm_m2_cpt_sync_at_release(s);
        }
        s->m2_cpt[ci].dirty = true;
        s->m2_cpt_dirty[s->m2_cpt_dirty_n++] = ci;
    }
}

/* M6.5 (item-4 step 4, the DISCOVERY sweep): the crash buffers are NVOS32-local sysmem GR
 * buffers with NO GSP-RPC, so QEMU only learns their GR-VA->guest-GPA mapping by WALKING the
 * GR VAS page tables (the guest RM builds them in guest-RAM-as-vidmem). Walk each snooped VAS
 * PDB, enumerate every sysmem leaf, coalesce runs, and OS_DESCRIPTOR+map_dma each into the host
 * GR VASpace (host GPU can then DMA into the guest's real sysmem working set). Idempotent
 * (re-runs only back NEW VAs). Gated by the m2exec caller. Bounded by `budget`. */
static void nvkvm_m2_enum_gr_sysmem(NvkvmGpuEmul *s, uint32_t client)
{
    int budget = 300000;                          /* total PT entries to visit (sparse) */
    for (int v = 0; v < s->chan_vas_n && budget > 0; v++) {
        uint64_t pdb = s->chan_vas[v].pdb;
        if (!pdb) { continue; }
        /* #14 (the ALREADY-MAPPED aliasing root cause): never walk ANOTHER process's
         * VAS under this sweep client.  Two concurrent processes get IDENTICAL guest
         * VAs (both 0x2024xxxxx/0x2002xxxxx), so backing process B's leaves (different
         * GPAs!) into A's host VAS hit st=0x51 against A's own maps — and worse, B's
         * unique VAs landed A's fvas maps first, so B's OWN backing then collided in
         * B's fvas and each process read the other's bytes (aliased completion sema =
         * both cuCtxCreate spin).  Ownership is the transport-observed dup-edge fact;
         * kernel/CeUtils roots have no user owner and stay visible to every client
         * exactly as before, so a single process's sweep is byte-identical. */
        if (nvkvm_m2_multiproc(s) && nvkvm_m2_vas_foreign(s, v, client)) { continue; }
        struct nvkvm_leaf_acc a; memset(&a, 0, sizeof(a));
        a.s = s; a.client = client;
        /* #13: a compute-client VAS's walk also records its table pages into m2_cpt[]
         * so the CE-PT-write trigger can decode writes against them. */
        a.cpt_pdb = nvkvm_m2_pdb_is_compute(s, pdb) ? pdb : 0;
        nvkvm_m2_pt_enum(s, pdb, false, 0, 0, &a, &budget);
        nvkvm_m2_leaf_flush(&a);
        qemu_log("nvkvm-gpu[%s] M6.5 enum_gr_sysmem: vas=0x%08x pdb=0x%llx comp=%d runs=%d "
                 "sysbytes=0x%llx vidbytes=0x%llx backed=%d (budget_left=%d)\n", s->chip->name,
                 s->chan_vas[v].hvas, (unsigned long long)pdb, a.cpt_pdb ? 1 : 0, a.runs,
                 (unsigned long long)a.sysbytes, (unsigned long long)a.vidbytes,
                 a.backed, budget);
    }
    /* M5.10 PERF: if we ran out of budget the walk was incomplete (unvisited PT pages aren't
     * tracked) -> keep sweeping eagerly next doorbell rather than trust the dirty flag. */
    s->m2_gr_pt_trunc = (budget <= 0);
}

/* M5.28 PER-CHANNEL VAS population: mirror the guest channel's ENTIRE address space into its
 * fresh nvkvm-owned VAS. Walk the channel's OWN guest PDB (chan_own_pdb, derived from the
 * channel's client -> forwarded VAS -> snooped PDB), enumerate every valid leaf, and FIXED
 * map_dma each into the fresh VAS at the same GPU VA (sysmem -> OS_DESCRIPTOR guest RAM WB;
 * vidmem -> blank host vidmem object via the GPGA table). m2_cur_cvas MUST be set by the caller
 * so grmapper routes the maps into THIS channel's fvas (not the guest forwarded VAS). Because
 * the VAS is one WE own (no host-RM ctx self-promote), every guest VA places without st=0x51 —
 * the Xid-32 collision class. Idempotent via the global m2_va_seen dedup. */
static bool nvkvm_m2_populate_cvas(NvkvmGpuEmul *s, struct nvkvm_chan_entry *c)
{
    bool root_sys = false;                         /* M5.36: walk with the resolved root aperture */
    uint64_t pdb = nvkvm_chan_own_pdb_rs(s, &root_sys); /* uses s->chan_client (caller set it) */
    if (!pdb) {
        /* M5.32 Step-1b: return FALSE so the caller does NOT mark this CVAS populated —
         * the GR-VAS root is captured asynchronously (RESERVED_PDES / SET_PAGE_DIRECTORY)
         * and may not have arrived by the first doorbell.  Retrying on subsequent doorbells
         * makes resolution deterministic instead of one-shot-flaky. */
        qemu_log("nvkvm-gpu[%s] M5.28 populate_cvas: client=0x%08x tsg=0x%08x — no own PDB "
                 "(VAS not snooped yet); will retry next doorbell\n", s->chip->name,
                 c->client, c->tsg);
        return false;
    }
    int budget = 300000;
    struct nvkvm_leaf_acc a; memset(&a, 0, sizeof(a));
    a.s = s; a.client = c->client;
    /* M5.36: pass the resolved root aperture — UVM-managed roots are sys-rooted; the
     * previously-hardcoded false mis-walked them (enumerated 0 leaves) even when found. */
    nvkvm_m2_pt_enum(s, pdb, root_sys, 0, 0, &a, &budget);
    nvkvm_m2_leaf_flush(&a);
    qemu_log("nvkvm-gpu[%s] M5.28 populate_cvas: client=0x%08x tsg=0x%08x pdb=0x%llx -> "
             "cvas[%d] fvas=0x%08x runs=%d sysbytes=0x%llx vidbytes=0x%llx backed=%d "
             "(budget_left=%d)\n", s->chip->name, c->client, c->tsg,
             (unsigned long long)pdb, s->m2_cur_cvas,
             s->m2_cur_cvas >= 0 ? s->m2_cvas[s->m2_cur_cvas].fvas : 0,
             a.runs, (unsigned long long)a.sysbytes, (unsigned long long)a.vidbytes,
             a.backed, budget);
    return true;
}

/* M5.13 DRY-RUN: locate which page-directory maps a target guest-phys (the completion
 * semaphore 0x2efbaf000), at what GR-VA, WITHOUT backing anything. Walks every candidate root
 * — the snooped GR VASes (chan_vas[]) plus the BAR1/BAR2 aperture PDBs — so we can see whether
 * the semaphore lives in a GR channel VAS (mappable into the host GR VAS at the same GR-VA) or
 * only in a kernel aperture (which would need a different bridge). Pure diagnostic; one-shot. */
static void nvkvm_m2_probe_sem_pdb(NvkvmGpuEmul *s, uint32_t client, uint64_t target)
{
    struct { const char *tag; uint64_t pdb; } roots[16 + 2];
    int nr = 0;
    for (int v = 0; v < s->chan_vas_n && nr < 16; v++) {
        if (s->chan_vas[v].pdb) {
            roots[nr].tag = "chan_vas"; roots[nr].pdb = s->chan_vas[v].pdb; nr++;
        }
    }
    if (s->bar1_pdb) { roots[nr].tag = "bar1_pdb"; roots[nr].pdb = s->bar1_pdb; nr++; }
    if (s->bar2_pdb) { roots[nr].tag = "bar2_pdb"; roots[nr].pdb = s->bar2_pdb; nr++; }
    qemu_log("nvkvm-gpu[%s] M5.13 PROBE start: target gpa=0x%llx across %d roots "
             "(chan_vas_n=%d bar1=0x%llx bar2=0x%llx)\n", s->chip->name,
             (unsigned long long)target, nr, s->chan_vas_n,
             (unsigned long long)s->bar1_pdb, (unsigned long long)s->bar2_pdb);
    for (int r = 0; r < nr; r++) {
        int budget = 300000;
        struct nvkvm_leaf_acc a; memset(&a, 0, sizeof(a));
        a.s = s; a.client = client; a.dry = true; a.target = target; a.tag = roots[r].tag;
        nvkvm_m2_pt_enum(s, roots[r].pdb, false, 0, 0, &a, &budget);
        nvkvm_m2_leaf_flush(&a);
        qemu_log("nvkvm-gpu[%s] M5.13 PROBE root[%d] %s pdb=0x%llx runs=%d sysB=0x%llx "
                 "vidB=0x%llx found=%d (budget_left=%d)\n", s->chip->name, r, roots[r].tag,
                 (unsigned long long)roots[r].pdb, a.runs, (unsigned long long)a.sysbytes,
                 (unsigned long long)a.vidbytes, a.found, budget);
    }
}

/* M5.9 EXECUTION FORWARD (per doorbell): map the GR channel's newly-submitted pushbuffers
 * into the host GR VASpace (double-mmap + copy the guest's command bytes) so the host GPU's
 * MMU resolves them, then RING the host doorbell (per-channel token, unconditional since M5.22)
 * so the HOST GPU actually runs the guest's work and writes the completion semaphore
 * for real ([[mode2-real-forward-not-fake]]). USERD (GP_PUT) + GPFIFO are already double-
 * mmapped; here we add the pushbuffers each entry points at. Idempotent via the mapped set. */
static void nvkvm_m2_exec_doorbell(NvkvmGpuEmul *s)
{
    if (!s->m2exec) { return; }
    uint32_t grc = s->m2_gr_client;
    /* M5.10: re-sweep the GR VAS at the doorbell (decoupled from doorbell_ready / channel-client
     * match — the semaphore-releasing channel may be under a different client than m2_gr_client).
     * The guest maps its working set (pushbuffers, data, completion semaphore 0x2efbaf000) only
     * WHEN it submits work, AFTER the one-shot M6.5 sweep at the 0xc7c0 alloc — so those leaves are
     * still UNBACKED at this point. Re-sweep (idempotent: only NEW VAs backed) so the full working
     * set, incl. the semaphore the host must write, is FIXED-mapped into the host VAS before any
     * ring (else a ring faults the host GPU on the SEM_RELEASE target -> cuInit=999). Bounded. */
    /* M5.48c: ALSO re-sweep whenever a compute-client channel has NEW submitted work (GP_PUT
     * advanced) — the guest maps late working-set leaves (its GR-ctx pool / local-memory
     * backing at top-of-heap VAs, the buffers its init pushbuffer references) only right
     * before submitting, typically AFTER the first-8 boot-time sweeps are exhausted. Without
     * this the host GR FE faulted VIRT_WRITE at guest-pool+0xe00000 (leaf never mirrored).
     * Bounded by a higher total cap; idempotent via va_seen. */
    /* M5.49b: residency must cover the GR client AND libcuda's CE-copy clients.
     * Previously the sweep was keyed ONLY on grc=m2_gr_client, so the CE-copy
     * client's late working-set leaves (its copy/sema buffers in the high
     * 0x78301xxxxxx band) never became resident in the host CE channel's VAS ->
     * host CE2 FAULT_PTE on a VIRT_WRITE once the host actually had to complete
     * (the simulated completion used to mask it).  Sweep every forwarded compute
     * client. */
    uint32_t sweepc[(int)ARRAY_SIZE(s->m2_gr_clients) +
                    (int)ARRAY_SIZE(s->m2_user_ce_clients)]; int nsweep = 0;
    /* #14: sweep EVERY user GR compute client (one per guest process), not just the
     * first — a 2nd process's late-mapped working set (completion sema, pushbuffers)
     * must back into ITS OWN host VAS (the enum walk is owner-scoped now).  Single
     * process: the list is exactly {m2_gr_client} = the old behavior. */
    for (int i = 0; i < s->m2_gr_clients_n; i++) { sweepc[nsweep++] = s->m2_gr_clients[i]; }
    /* M5.51: only sweep the CE-copy clients under the (abandoned) host-only-sema path (m2hostsem).
     * NOT under m2cexec (approach A): the m570 probe proved a full CE-client VAS sweep RE-BACKS the
     * weight buffers the GR client already owns at the same gpga (1498 gpga FAILED + 1024-object cap
     * exhaustion -> model-load hang). The CE channel shares those buffers; if its VAS is the GR VAS
     * the GR sweep already covers it, and if it is a separate VAS the fix is a TARGETED map_dma of
     * the EXISTING host object into the CE VAS, never a fresh per-leaf alloc. So do NOT sweep here. */
    if (s->m2hostsem) {
        for (int i = 0; i < s->m2_user_ce_n; i++) { sweepc[nsweep++] = s->m2_user_ce_clients[i]; }
    }
    bool m548_newwork = false;
    for (int i = 0; i < s->chan_n; i++) {
        struct nvkvm_chan_entry *nc = &s->chans[i];
        bool match = false;
        for (int k = 0; k < nsweep; k++) { if (nc->client == sweepc[k]) { match = true; break; } }
        if (!match || !nc->gpfifo_va || !nc->userd) { continue; }
        uint32_t np = (uint32_t)nvkvm_fb_read(s, nc->userd + 0x8C, 4);
        if (np != nc->sweep_put && np <= nc->gpfifo_ent) {
            m548_newwork = true;
            nc->sweep_put = np;          /* latch: one sweep per submission, not per doorbell */
        }
    }
    /* M5.10 PERF: the sweep walks the WHOLE GR VAS (the LLM's was 7777 runs) and on a real LLM
     * 99.97% of walks (m568: 91932/91960) backed NOTHING — pure waste that dropped it to ~0.1
     * tok/s. Re-sweep ONLY when something that could need backing changed:
     *   (a) bootstrap: the first 8 sweeps (populate the PT-page set + initial working set);
     *   (b) m2_gr_vas_dirty: the guest wrote a tracked GR PT page (a mapping changed) — every
     *       PTE/PDE edit writes a tracked page or an ancestor of one (the root PDB is always
     *       tracked), so this catches all real mapping changes BEFORE the engine uses them;
     *   (c) a new GR VAS appeared (chan_vas_n grew) — its PT pages aren't tracked yet;
     *   (d) the last walk was budget-truncated (incomplete coverage -> stay eager);
     *   (e) a sparse periodic net (every 64th submission) as belt-and-suspenders.
     * m548_newwork (a submission actually advanced) still gates so we never sweep on idle
     * doorbells. Each sweep RESETS + rebuilds the PT-page set so it tracks the current tables. */
    if (m548_newwork) { s->m2_db_submits++; }
    bool new_vas = (s->chan_vas_n != s->m2_last_swept_vas_n);
    bool periodic = (m548_newwork && (s->m2_db_submits & 255u) == 0u);  /* sparse insurance net */
    bool want = (s->m2_exec_sweeps < 8) ||
                (m548_newwork && s->m2_exec_sweeps < 200000 &&
                 (s->m2_gr_vas_dirty || new_vas || s->m2_gr_pt_trunc || periodic));
    if (nsweep && want) {
        s->m2_exec_sweeps++;
        s->m2_gr_vas_dirty = false;        /* consume; a later PT write re-arms it */
        s->m2_last_swept_vas_n = s->chan_vas_n;
        nvkvm_m2_gr_pt_reset(s);           /* rebuild the PT-page set from this walk */
        s->m2_recording_gr_pt = true;
        uint64_t t0sw = nvkvm_now_ns();    /* M5.11: re-sweep walk time-share */
        for (int k = 0; k < nsweep; k++) {
            qemu_log("nvkvm-gpu[%s] M5.10 doorbell re-sweep #%u (client 0x%08x)%s — back newly-mapped "
                     "working set incl. completion semaphore\n", s->chip->name, s->m2_exec_sweeps,
                     sweepc[k], m548_newwork ? " [M5.48c new-work]" : "");
            nvkvm_m2_enum_gr_sysmem(s, sweepc[k]);
        }
        nvkvm_t_sweep_ns += nvkvm_now_ns() - t0sw; nvkvm_t_sweep_calls++;
        s->m2_recording_gr_pt = false;
        /* M5.62: a mapping changed (a sweep just (re)backed working-set leaves) — re-arm the walk on
         * every channel so the next doorbell rediscovers any new pushbuffers before going opaque. */
        if (s->m2opaque) {
            for (int ci = 0; ci < s->chan_n; ci++) {
                s->chans[ci].resident = false; s->chans[ci].stable_subs = 0;
            }
        }
    }
    /* M5.13: one-shot DRY-RUN locate of the completion semaphore (0x2efbaf000, the page the
     * guest RM busy-polls during cuCtxCreate) so we learn its owning PDB + GR-VA before backing.
     * No side effects. */
    if (grc && !s->m2_sem_probe_done) {
        s->m2_sem_probe_done = true;
        nvkvm_m2_probe_sem_pdb(s, grc, 0x2efbaf000ull);
    }
    if (!s->m2_doorbell_ready) { return; }
    /* M5.12 (chid/token table): fetch each forwarded channel's HOST work-submit token once.
     * shadow_fwd creates the host channel with the SAME hObject, so 0xc36f0108 on the guest's
     * channel handle hits the host channel. The GP_PUT-driven demux rings THIS token for whichever
     * channel advanced (vs. decoding vChid from the guest token). M5.22: the ring is now
     * unconditional (per-channel token). */
    for (int i = 0; i < s->chan_n; i++) {
        struct nvkvm_chan_entry *c = &s->chans[i];
        if (c->token_valid || !c->hobject || !c->gpfifo_va) { continue; }
        uint8_t tp[4]; memset(tp, 0, sizeof(tp)); uint32_t tst = 0xffff;
        int trc = nvkvm_m2_control1(s, c->client, c->hobject, 0xc36f0108u, tp, 4, &tst);
        if (trc == 0 && tst == 0) {
            c->host_token = ldl_le_p(tp); c->token_valid = true;
            qemu_log("nvkvm-gpu[%s] M5.12 chan[%d] hObj=0x%08x gpfifo=0x%llx -> HOST token=0x%08x "
                     "(rl=%u chid=%u)\n", s->chip->name, i, c->hobject,
                     (unsigned long long)c->gpfifo_va, c->host_token,
                     (c->host_token >> 16) & 0xffff, c->host_token & 0xffff);
        }
    }
    for (int i = 0; i < s->chan_n; i++) {
        struct nvkvm_chan_entry *c = &s->chans[i];
        /* CE-EXEC fwd (A): besides the GR client's channels, also forward+ring the user-CE
         * channels so the HOST CE executes their LAUNCH_DMA for real. Residency (the pre-ring
         * sweep below) + the M5.60 dst real-backing must have made the copy's src/dst leaves
         * resident in THIS channel's VAS first, else the host CE faults (the old CE2 Xid). */
        bool fwd_ce = s->m2cexec && nvkvm_m2_is_user_ce(s, c->client);
        /* #14: forward+ring the GR channels of EVERY user compute client (one per guest
         * process), not just the first — single process: identical (list = {grc}). */
        bool is_grc_chan = nvkvm_m2_is_gr_client(s, c->client);
        if ((!is_grc_chan && !fwd_ce) || !c->gpfifo_va || !c->gpfifo_ent) { continue; }
        uint32_t cc = c->client;   /* key residency on the channel's OWN client (CE VAS != GR VAS) */
        uint64_t gpf_phys = nvkvm_m2_resolve_fb(s, cc, c->gpfifo_va);
        if (!gpf_phys) { continue; }
        uint32_t gp_put = (uint32_t)nvkvm_fb_read(s, c->userd + 0x8C, 4);
        if (gp_put == c->gp_get || gp_put > c->gpfifo_ent || gp_put < c->gp_get) {
            continue;                            /* no new (non-wrapping) work */
        }
        int newmaps = 0;
        /* M5.62 OPAQUE fast-path: a fully-resident userspace channel re-references only
         * already-mapped pushbuffers (the dirty-sweep keeps the working set backed; newpushbufs has
         * been 0 for K subs). Skip the per-entry GPFIFO walk + va_seen (uncached host-vidmem reads)
         * entirely — just ring (gp_get advances to gp_put at ring time). Re-armed on any sweep. */
        bool skip_walk = s->m2opaque && c->resident;
        if (!skip_walk) {
            for (uint32_t idx = c->gp_get; idx < gp_put && idx < c->gpfifo_ent; idx++) {
                uint64_t epa = gpf_phys + (uint64_t)idx * 8;
                uint32_t e0 = (uint32_t)nvkvm_fb_read(s, epa, 4);
                uint32_t e1 = (uint32_t)nvkvm_fb_read(s, epa + 4, 4);
                uint64_t pb = (uint64_t)(e0 & 0xFFFFFFFCu) | ((uint64_t)(e1 & 0xFFu) << 32);
                uint32_t pblen = (e1 >> 10) & 0x1FFFFFu;
                if (!pb) { continue; }
                uint64_t pbbase = pb & ~0xfffull;
                if (nvkvm_m2_va_seen(s, cc, pbbase)) { continue; }
                uint64_t pbphys = nvkvm_m2_resolve_fb(s, cc, pbbase);
                uint64_t sz = ((pb - pbbase) + (uint64_t)pblen * 4 + 0xfff) & ~0xfffull;
                if (!sz) { sz = 0x1000; }
                nvkvm_m2_back_and_map(s, cc, pbbase, pbphys, sz, true, "pushbuf");
                newmaps++;
            }
            /* Promote to resident after K consecutive zero-newmap submissions; reset the instant
             * anything new is mapped (working set grew -> keep walking until stable again). */
            if (s->m2opaque) {
                if (newmaps == 0) {
                    if (++c->stable_subs >= 8u && !c->resident) {
                        c->resident = true;
                        qemu_log("nvkvm-gpu[%s] M5.62 OPAQUE promote ch[%d] client=0x%08x "
                                 "(resident — skip GPFIFO walk)\n", s->chip->name, i, cc);
                    }
                } else {
                    c->stable_subs = 0; c->resident = false;
                }
            }
        }
        if (s->m2_trace)
        qemu_log("nvkvm-gpu[%s] M5.9 exec_doorbell %s gp_get=%u->%u newpushbufs=%d (client=0x%08x)%s\n",
                 s->chip->name, is_grc_chan ? "GR" : "CE", c->gp_get, gp_put, newmaps, cc,
                 skip_walk ? " [OPAQUE skip-walk]" : "");
        /* M5.46: ring THIS channel's own host token. The GR channel (token from M5.8
         * doorbell_setup) may legitimately use s->m2_gr_token; EVERY other channel
         * must use its OWN per-channel token. NEVER fall back to the GR token for a
         * non-GR channel — that told the host to fetch the GR channel's USERD for a
         * COPY channel's work, so ESCHED never ran the COPY work (put=1>get=0, 0%
         * util, no Xid; proven by the M5.45 host self-test). A non-GR channel with no
         * token yet is not runlist-assigned (BIND happens in the per-channel loop
         * AFTER this exec_doorbell call) — DEFER: leave gp_get unadvanced so the
         * per-channel M5.22(b) ring (after the M5.46 post-bind token fetch) submits it
         * correctly. Advance gp_get ONLY on a real ring. */
        bool is_gr = (c->hobject == s->m2_gr_channel);
        bool can_ring = c->token_valid || (is_gr && s->m2_gr_token);
        /* #12 cont.34: GPFIFO_SCHEDULE the GR TSG here if it is NOT the one M5.8 already
         * scheduled.  A 2nd cuCtxCreate mints a FRESH GR TSG, but doorbell_setup early-
         * returns on the sticky m2_doorbell_ready and never schedules it — so CTX2's 8
         * rl=0 GR channels rang but the host never consumed (gp_get stuck 0), leaving 8
         * of the 16 pool-completion semaphores at 0 and libcuda's wait-ALL spinning
         * forever.  This is THE #12 residual.  Schedule the fresh GR TSG exactly once
         * (tracked by m2_tsg_sched); the M5.33 st=0x57 noise the old skip avoided was
         * RE-scheduling an already-scheduled TSG, which this one-shot guard prevents.
         * #14: track (client, tsg) pairs and schedule THIS channel's own TSG — two
         * concurrent processes mint IDENTICAL TSG handle values (both 0x5c000012), so
         * the old value-keyed scalar made process B's GR TSG look already-scheduled
         * and B's GR channels rang forever off-runlist. */
        if (is_gr && c->tsg && !nvkvm_m2_tsg_sched_check(s, c->client, c->tsg)) {
            uint8_t sp[3]; memset(sp, 0, sizeof(sp)); sp[0] = 1;
            uint32_t sst = 0xffff;
            int src = nvkvm_m2_control1(s, c->client, c->tsg, 0xa06c0101u,
                                        sp, sizeof(sp), &sst);
            if (src == 0 && sst == 0) { nvkvm_m2_tsg_sched_mark(s, c->client, c->tsg); }
            qemu_log("nvkvm-gpu[%s] #12 cont.34 GR-TSG GPFIFO_SCHEDULE TSG=0x%08x "
                     "client=0x%08x -> rc=%d st=0x%x%s\n", s->chip->name, c->tsg,
                     c->client, src, sst,
                     (src == 0 && sst == 0) ? "  OK SCHEDULED" : "  <-- ERR");
        }
        if (!s->m2_usermode_qva || !can_ring) {
            if (s->m2_usermode_qva) {
                qemu_log("nvkvm-gpu[%s] M5.46 DEFER ring ch[%d] hObj=0x%08x: no per-channel "
                         "token yet (TSG not bound) — per-channel loop will ring\n",
                         s->chip->name, i, c->hobject);
            }
            continue;                            /* do NOT advance gp_get; retry path rings it */
        }
        c->gp_get = gp_put;                      /* consumed — advance only on a real ring */
        {
            uint32_t tok = c->token_valid ? c->host_token : s->m2_gr_token;
            /* M5.44: read the REAL host USERD (chanbuf qva, no overlay) at ring time. */
            uint32_t hput = 0xffffffffu, hget = 0xffffffffu;
            for (int k = 0; k < s->m2_chanbuf_n; k++) {
                if (s->m2_chanbuf[k].client == c->client &&
                    s->m2_chanbuf[k].chan == c->hobject && s->m2_chanbuf[k].qva) {
                    hput = ldl_le_p((uint8_t *)s->m2_chanbuf[k].qva + 0x8C);
                    hget = ldl_le_p((uint8_t *)s->m2_chanbuf[k].qva + 0x88);
                    break;
                }
            }
            stl_le_p((uint8_t *)s->m2_usermode_qva + 0x90, tok);
            if (s->m2_trace)
            qemu_log("nvkvm-gpu[%s] M5.9 *** RANG host doorbell token=0x%08x (USERMODE+0x90) "
                     "— host GPU should now run gpfifo=0x%llx *** hostUSERD put=%u get=%u%s\n",
                     s->chip->name, tok, (unsigned long long)c->gpfifo_va, hput, hget,
                     is_gr ? " [GR]" : "");
        }
    }
}

/* M5.5 one-shot validation of the RM_MAP_MEMORY_DMA primitive via the CORRECT mapper
 * (NV01_MEMORY_VIRTUAL). P1: fully private client->device->vaspace->virtmem->memory,
 * map NON-FIXED then FIXED. P2: against the forwarded GR VASpace (alloc a virtmem mapper
 * referencing 0x5c000007, map a fresh host vidmem FIXED at a guest VA). Pure validation —
 * private handles, never the live forward chain — cannot regress GR build. */
static void nvkvm_m2_mapdma_selftest(NvkvmGpuEmul *s, uint32_t hClient)
{
    uint64_t sz = 0x10000;                        /* 64 KiB (PMA granularity) */

    /* ---- Part 1: fully self-contained tuple ---- */
    const uint32_t C = 0xc1ee0011u, DEV = 0xde110001u, VAS = 0xde110002u,
                   VIRT = 0xde110004u, MEM = 0xde110003u;
    uint32_t st = 0xffff;
    uint32_t c0 = C;
    nvkvm_m2_alloc1(s, C, 0, 0, 0x0u, &c0, sizeof(c0), &st);
    uint8_t devp[56]; memset(devp, 0, sizeof(devp));
    nvkvm_m2_alloc1(s, C, C, DEV, 0x0080u, devp, sizeof(devp), &st);
    uint32_t dst = st;
    uint8_t vasp[56]; memset(vasp, 0, sizeof(vasp));   /* NV_VASPACE_ALLOCATION_PARAMETERS, default */
    nvkvm_m2_alloc1(s, C, DEV, VAS, 0x90f1u, vasp, sizeof(vasp), &st);
    uint32_t vst = st;
    uint32_t vmst = 0xffff;
    nvkvm_m2_alloc_virtmem(s, C, DEV, VIRT, VAS, &vmst);
    qemu_log("nvkvm-gpu[%s] M5.5 selftest P1: dev st=0x%x vaspace st=0x%x virtmem(0x0070) "
             "st=0x%x\n", s->chip->name, dst, vst, vmst);
    struct nvkvm_host_map hm;
    if (nvkvm_m2_host_alloc_map_vidmem(s, C, DEV, MEM, sz, &hm)) {
        uint32_t s1 = 0xffff, s2 = 0xffff; uint64_t v1 = 0, v2 = 0;
        int r1 = nvkvm_m2_map_dma(s, C, DEV, VIRT, MEM, 0, sz, false, 0, &s1, &v1);
        qemu_log("nvkvm-gpu[%s] M5.5 [P1a] map hDma=VIRTMEM NON-FIXED -> rc=%d st=0x%x "
                 "va=0x%llx%s\n", s->chip->name, r1, s1, (unsigned long long)v1,
                 (r1 == 0 && s1 == 0) ? "  OK" : "  <-- ERR");
        uint64_t want = 0x7f0000000000ull;
        int r2 = nvkvm_m2_map_dma(s, C, DEV, VIRT, MEM, 0, sz, true, want, &s2, &v2);
        qemu_log("nvkvm-gpu[%s] M5.5 [P1b] map hDma=VIRTMEM FIXED@0x%llx -> rc=%d st=0x%x "
                 "va=0x%llx%s\n", s->chip->name, (unsigned long long)want, r2, s2,
                 (unsigned long long)v2,
                 (r2 == 0 && s2 == 0 && v2 == want) ? "  OK FIXED-PLACEMENT-WORKS"
                                                    : "  <-- ERR");
        munmap(hm.qva, hm.size);
    } else {
        qemu_log("nvkvm-gpu[%s] M5.5 selftest P1: private vidmem alloc failed\n",
                 s->chip->name);
    }

    /* ---- Part 2: against the forwarded GR client's real VASpace ---- */
    uint32_t hDev = 0, hVas = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == hClient) {
            hDev = s->m2_devvas[i].dev; hVas = s->m2_devvas[i].vas; break;
        }
    }
    qemu_log("nvkvm-gpu[%s] M5.5 selftest P2: GR client 0x%08x -> host 0x%08x dev=0x%08x "
             "vas=0x%08x\n", s->chip->name, hClient, nvkvm_m2_client(s, hClient), hDev, hVas);
    if (hDev && hVas) {
        uint32_t hVirt = 0xdb000000u | (s->m2_databuf_next & 0xffffu);
        uint32_t hMem  = 0xda100000u | (s->m2_databuf_next++ & 0xffffu);
        uint32_t gvm = 0xffff;
        nvkvm_m2_alloc_virtmem(s, hClient, hDev, hVirt, hVas, &gvm);
        struct nvkvm_host_map gm;
        if (gvm == 0 && nvkvm_m2_host_alloc_map_vidmem(s, hClient, hDev, hMem, sz, &gm)) {
            uint64_t want = 0x200000000ull;       /* a guest-style VA in the GR vaspace */
            uint32_t gs = 0xffff; uint64_t gv = 0;
            int gr = nvkvm_m2_map_dma(s, hClient, hDev, hVirt, hMem, 0, sz, true, want, &gs, &gv);
            qemu_log("nvkvm-gpu[%s] M5.5 [P2] virtmem(0x%08x) over GR VAS 0x%08x map "
                     "FIXED@0x%llx -> rc=%d st=0x%x va=0x%llx%s\n", s->chip->name, hVirt,
                     hVas, (unsigned long long)want, gr, gs, (unsigned long long)gv,
                     (gr == 0 && gs == 0) ? "  OK GR-VAS-MAP-WORKS" : "  <-- ERR");
            munmap(gm.qva, gm.size);
        } else {
            qemu_log("nvkvm-gpu[%s] M5.5 [P2] virtmem alloc st=0x%x (skip map)\n",
                     s->chip->name, gvm);
        }
    }
}

/* M5.4 DATA-PLANE: back a forwarded channel's USERD with REAL host GPU memory.
 * The guest's NV_CHANNEL_ALLOC_PARAMS carries a userd memdesc (base@168, size@176,
 * addressSpace@184) naming a guest-FB address where the guest driver maps USERD via
 * BAR1. We allocate a host vidmem object, hand it to the host channel as
 * hUserdMemory[0] (@auxbuf+32) so the host channel USES it, mmap it into QEMU, and
 * register the guest-FB userd.base range in m2_fbback. Then guest reads/writes of its
 * USERD (GP_PUT/GP_GET) go through the BAR-aperture->FB path to the SAME host memory
 * the host GPU uses — the double-mmap that makes GP_GET observable to the guest poll.
 * Defensive: skips (leaving hUserdMemory[0] for the caller to zero) on any anomaly. */
static void nvkvm_m2_back_channel_userd(NvkvmGpuEmul *s, uint32_t hClient,
                                        uint32_t chanObj, uint8_t *auxbuf,
                                        uint32_t psize)
{
    if (psize < 192) {
        return;                              /* no userd memdesc present */
    }
    uint64_t ubase = ldq_le_p(auxbuf + 168);
    uint64_t usize = ldq_le_p(auxbuf + 176);
    uint32_t uas   = ldl_le_p(auxbuf + 184); /* addressSpace: 2=FBMEM, 1=SYSMEM */
    if (ubase == 0) {
        return;                              /* guest didn't place USERD — let RM alloc */
    }
    if (s->m2_chanbuf_n >= (int)ARRAY_SIZE(s->m2_chanbuf) ||
        s->m2_fbback_n >= (int)ARRAY_SIZE(s->m2_fbback)) {
        return;
    }
    /* Find the channel's device (VASpace parent) tracked for this client. */
    uint32_t hDev = 0;
    for (int i = 0; i < s->m2_devvas_n; i++) {
        if (s->m2_devvas[i].client == hClient) { hDev = s->m2_devvas[i].dev; break; }
    }
    if (!hDev) {
        qemu_log("nvkvm-gpu[%s] M5.4 USERD-back: no device for client 0x%08x — skip\n",
                 s->chip->name, hClient);
        return;
    }
    uint64_t asize = usize ? ((usize + 0xfff) & ~0xfffull) : 0x1000;
    uint32_t hUserd = 0xda000000u | (s->m2_databuf_next++ & 0xffffu);
    struct nvkvm_host_map hm;
    if (!nvkvm_m2_host_alloc_map_vidmem(s, hClient, hDev, hUserd, asize, &hm)) {
        qemu_log("nvkvm-gpu[%s] M5.4 USERD-back: host alloc failed (chan 0x%08x) — skip\n",
                 s->chip->name, chanObj);
        return;
    }
    stl_le_p(auxbuf + 32, hUserd);           /* hUserdMemory[0] = host USERD handle */
    /* M5.47 ROOT-CAUSE FIX (silent forwarded no-fetch): the guest pools all its channel
     * USERDs into ONE memory object and addresses each via a NONZERO userdOffset[0]@64
     * (chid * 0x3000). We replace hUserdMemory[0] with a FRESH per-channel object whose
     * USERD belongs at offset 0, but the host channel's USERD = hUserdMemory[0] +
     * userdOffset[0]. Left nonzero, the host GPU reads USERD at our_object + 0x2000.. (past
     * our 0x1000 object) while the guest's GP_PUT lands (via the fbback overlay) at our
     * object offset 0 -> host sees GP_PUT==GP_GET, never fetches the GPFIFO (zero util,
     * zero Xid -- the exact morph-confirmed symptom). Zero userdOffset[0] so the host reads
     * USERD where we write it. (NV_CHANNEL_ALLOC_PARAMS userdOffset[NV_MAX_SUBDEVICES]@64.) */
    stq_le_p(auxbuf + 64, 0u);
    s->m2_fbback[s->m2_fbback_n].fb_base = ubase;
    s->m2_fbback[s->m2_fbback_n].size    = asize;
    s->m2_fbback[s->m2_fbback_n].host_qva = hm.qva;
    s->m2_fbback_n++;
    s->m2_chanbuf[s->m2_chanbuf_n].client = hClient;
    s->m2_chanbuf[s->m2_chanbuf_n].chan   = chanObj;
    s->m2_chanbuf[s->m2_chanbuf_n].h_userd = hUserd;
    s->m2_chanbuf[s->m2_chanbuf_n].qva    = hm.qva;
    s->m2_chanbuf[s->m2_chanbuf_n].fb_base = ubase;
    s->m2_chanbuf[s->m2_chanbuf_n].size   = asize;
    s->m2_chanbuf_n++;
    qemu_log("nvkvm-gpu[%s] M5.4 USERD-back: chan 0x%08x USERD guest-FB 0x%llx "
             "(memdesc sz=0x%llx as=%u) -> host hUserd=0x%08x qva=%p asize=0x%llx "
             "[DOUBLE-MMAP]\n", s->chip->name, chanObj, (unsigned long long)ubase,
             (unsigned long long)usize, uas, hUserd, hm.qva,
             (unsigned long long)asize);
}

/* M14: capture THIS host GPU's real device-info-table (engine enumeration) once, via a
 * private QEMU-owned client→device→subdevice on the host. The guest's classDB and KernelCE
 * objects are built from this table, so serving the host's real one makes the guest advertise
 * exactly the engines/classes the physical GPU has — no hardcoded per-GPU blob (which was
 * captured truncated: 10 entries, missing video engines → numClasses 97 vs host 107).
 * GPU-AGNOSTIC: contains zero GA106-specific data; works for any NVIDIA GPU the host exposes.
 *
 * FINDING (2026-06-06): 0x20801112 flags=0x5c040 has NEITHER _PRIVILEGED(0x4) NOR
 * _NON_PRIVILEGED(0x8) -> defaults to KERNEL_PRIVILEGED, so an UNPRIVILEGED isolate client
 * gets NV_ERR_NOT_SUPPORTED (0x1b) here. This code therefore falls back to the captured blob.
 * The unprivileged dynamic route (TODO if numClasses ever matters for correctness — it does
 * NOT for compute; the 10 missing classes are video engines NVENC/NVDEC/NVJPG/OFA, and the GR
 * compute class 0xc7c0 IS already advertised) is to SYNTHESIZE the table from GET_ENGINES_V2
 * (0x20800170, flags=0x48 = NON_PRIVILEGED), which an unprivileged client CAN issue.
 * Paginated: 32 entries/call, 100B/entry; bMore drives the loop. */
static void nvkvm_m2_capture_devinfo(NvkvmGpuEmul *s)
{
    if (s->m2_devinfo_tried) {
        return;
    }
    s->m2_devinfo_tried = true;
    if (!nvkvm_m2_iso_ensure(s)) {
        return;
    }
    const uint32_t C = 0xc1ee0011u, DEV = 0xde100011u, SUB = 0xde100012u;
    uint32_t st = 0xffff;
    uint32_t c0 = C;
    nvkvm_m2_alloc1(s, C, 0, 0, 0x0u, &c0, sizeof(c0), &st);           /* NV01_ROOT */
    if (st != 0) {
        qemu_log("nvkvm-gpu[%s] M14 devinfo: client alloc st=0x%x\n", s->chip->name, st);
        return;
    }
    uint8_t dev[56]; memset(dev, 0, sizeof(dev));
    nvkvm_m2_alloc1(s, C, C, DEV, 0x0080u, dev, sizeof(dev), &st);     /* NV01_DEVICE_0 */
    if (st != 0) {
        qemu_log("nvkvm-gpu[%s] M14 devinfo: device alloc st=0x%x\n", s->chip->name, st);
        return;
    }
    uint32_t sub = 0;
    nvkvm_m2_alloc1(s, C, DEV, SUB, 0x2080u, &sub, sizeof(sub), &st);  /* NV20_SUBDEVICE_0 */
    if (st != 0) {
        qemu_log("nvkvm-gpu[%s] M14 devinfo: subdevice alloc st=0x%x\n", s->chip->name, st);
        return;
    }
    static uint8_t buf[12 + 32 * 100];
    uint32_t base = 0, total = 0; int pages = 0;
    for (;;) {
        memset(buf, 0, sizeof(buf));
        stl_le_p(buf + 0, base);                  /* baseIndex */
        st = 0xffff;
        int rc = nvkvm_m2_control1(s, C, SUB, 0x20801112u, buf, sizeof(buf), &st);
        if (rc != 0 || st != 0) {
            qemu_log("nvkvm-gpu[%s] M14 devinfo: control base=%u rc=%d st=0x%x\n",
                     s->chip->name, base, rc, st);
            break;
        }
        uint32_t num = ldl_le_p(buf + 4), bMore = ldl_le_p(buf + 8);
        if (num > 32u) num = 32u;
        if (total + num > 256u) num = 256u - total;
        memcpy(s->m2_devinfo + (uint64_t)total * 100, buf + 12, (uint64_t)num * 100);
        total += num;
        if (++pages > 16 || !bMore || total >= 256u) break;
        base += num ? num : 32u;
    }
    s->m2_devinfo_n = total;
    qemu_log("nvkvm-gpu[%s] M14 devinfo CAPTURED %u engine entries LIVE from host GPU "
             "(no blob)\n", s->chip->name, total);
}

/* M5.3 DATA-PLANE PROOF: validate that QEMU can put REAL host GPU memory into its
 * own address space via the isolate's /dev/nvidia0 fd (received over SCM_RIGHTS).
 * Build a minimal client→device→subdevice on the host GPU, then exercise the
 * reusable primitive helper and write+read a pattern. */
static void nvkvm_m2_memtest(NvkvmGpuEmul *s)
{
    if (!nvkvm_m2_iso_ensure(s)) {
        return;
    }
    const uint32_t C = 0xc1ee0001u, DEV = 0xde100001u, SUB = 0xde100002u,
                   MEM = 0xde100003u;
    uint32_t st = 0xffff;
    /* client (NV01_ROOT): aux = NV0000_ALLOC_PARAMS {hClient} */
    uint32_t c0 = C;
    nvkvm_m2_alloc1(s, C, 0, 0, 0x0u, &c0, sizeof(c0), &st);
    qemu_log("nvkvm-gpu[%s] MEMTEST client    -> 0x%x\n", s->chip->name, st);
    /* device NV01_DEVICE_0 (0x0080): NV0080_ALLOC_PARAMS (deviceId + handles), 56B zeros */
    uint8_t dev[56]; memset(dev, 0, sizeof(dev));
    nvkvm_m2_alloc1(s, C, C, DEV, 0x0080u, dev, sizeof(dev), &st);
    qemu_log("nvkvm-gpu[%s] MEMTEST device    -> 0x%x\n", s->chip->name, st);
    /* subdevice NV20_SUBDEVICE_0 (0x2080): NV2080_ALLOC_PARAMS {subDeviceId}, 4B */
    uint32_t sub = 0;
    nvkvm_m2_alloc1(s, C, DEV, SUB, 0x2080u, &sub, sizeof(sub), &st);
    qemu_log("nvkvm-gpu[%s] MEMTEST subdevice -> 0x%x\n", s->chip->name, st);
    /* Exercise the reusable data-plane primitive: alloc + map real host GPU vidmem. */
    struct nvkvm_host_map hm;
    if (!nvkvm_m2_host_alloc_map_vidmem(s, C, DEV, MEM, 0x10000, &hm)) {
        qemu_log("nvkvm-gpu[%s] MEMTEST: host alloc+map primitive FAILED\n",
                 s->chip->name);
        return;
    }
    volatile uint32_t *p = (volatile uint32_t *)hm.qva;
    p[0] = 0xc0ffee01u; p[1] = 0xdeadbeefu;
    uint32_t r0 = p[0], r1 = p[1];
    qemu_log("nvkvm-gpu[%s] MEMTEST: *** mmap OK hva=%p  wrote/read 0x%08x 0x%08x "
             "-> %s ***  DATA PLANE PRIMITIVE WORKS\n", s->chip->name, hm.qva, r0, r1,
             (r0 == 0xc0ffee01u && r1 == 0xdeadbeefu) ? "PASS" : "MISMATCH");
    /* M5.3: query the host GPU-phys of this buffer — the value PROMOTE_CTX rewrite
     * will point the host GPU at, so the host GPU operates on the SAME memory the
     * guest sees through the (future) memslot backing. */
    uint64_t hphys = 0; uint32_t aper = 0xff;
    if (nvkvm_m2_host_phys(s, C, MEM, &hphys, &aper)) {
        qemu_log("nvkvm-gpu[%s] MEMTEST: host GPU-phys=0x%llx aperture=%u (0=VIDMEM)"
                 "  <- PROMOTE_CTX target\n", s->chip->name,
                 (unsigned long long)hphys, aper);
    }
    munmap(hm.qva, hm.size);
}

/* M5.45 HOST-CHANNEL SELF-TEST (gated: env NVKVM_SELFTEST=1, never in normal boots).
 * Decisively splits the open blocker — "host never fetches forwarded channels" — in half:
 * with NO guest involvement, build a FULLY HOST-SIDE channel out of the SAME primitives the
 * forwarding path uses (private client→device→subdevice→VAS→virtmem; GPFIFO+pushbuffer+sem
 * in host vidmem mmapped into QEMU; client-provided USERD via hUserdMemory[0]; an
 * AMPERE_USERMODE_A doorbell page; NVA06C BIND+GPFIFO_SCHEDULE; GP_PUT=1; ring the channel's
 * NVC36F work-submit token), submit ONE inline NVC56F host-FIFO semaphore release (no engine
 * object — ESCHED/PBDMA executes host methods), and poll the sem word through QEMU's mmap.
 *   SEM LANDS (and host GP_GET -> 1): host doorbell/schedule/USERD mechanics are GOOD; the
 *     blocker is purely the guest↔host bridging of FORWARDED channels.
 *   SEM NEVER LANDS: the host-side schedule/doorbell sequence itself is broken — dig there.
 * Engine = COPY0 (no GR golden-ctx dependency; mirrors the host CE scrubber pattern).
 * Runs once from realize (stub is up, guest hasn't booted) — fully deterministic. */
static void nvkvm_m2_channel_selftest(NvkvmGpuEmul *s)
{
    if (!nvkvm_m2_iso_ensure(s)) {
        qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST: no isolate — abort\n", s->chip->name);
        return;
    }
    const uint32_t C = 0xc1ee0077u, DEV = 0xde770001u, SUB = 0xde770002u,
                   VAS = 0xde770003u, VIRT = 0xde770004u, BUF = 0xde770005u,
                   USERD = 0xde770006u, TSG = 0xde770007u, CHAN = 0xde770008u,
                   UM = 0xde770009u;
    const uint64_t VAB = 0x500000000ull;          /* private VAS layout base          */
    const uint64_t PB_VA  = VAB + 0x0000;         /* pushbuffer                       */
    const uint64_t GPF_VA = VAB + 0x1000;         /* GPFIFO ring                      */
    const uint64_t SEM_VA = VAB + 0x2000;         /* semaphore word the GPU writes    */
    const uint32_t PAYLOAD = 0xCAFEF00Du;
    uint32_t st = 0xffff;

    /* 1) private hierarchy: client -> device -> subdevice -> VAS -> virtmem mapper */
    uint32_t c0 = C;
    nvkvm_m2_alloc1(s, C, 0, 0, 0x0u, &c0, sizeof(c0), &st);
    uint32_t cst = st;
    uint8_t devp[56]; memset(devp, 0, sizeof(devp));
    nvkvm_m2_alloc1(s, C, C, DEV, 0x0080u, devp, sizeof(devp), &st);
    uint32_t dst = st;
    uint32_t sub0 = 0;
    nvkvm_m2_alloc1(s, C, DEV, SUB, 0x2080u, &sub0, sizeof(sub0), &st);
    uint32_t sst = st;
    uint8_t vasp[56]; memset(vasp, 0, sizeof(vasp));
    nvkvm_m2_alloc1(s, C, DEV, VAS, 0x90f1u, vasp, sizeof(vasp), &st);
    uint32_t vst = st;
    uint32_t vmst = 0xffff;
    nvkvm_m2_alloc_virtmem(s, C, DEV, VIRT, VAS, &vmst);
    qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST hierarchy: client=0x%x dev=0x%x subdev=0x%x "
             "vas=0x%x virtmem=0x%x\n", s->chip->name, cst, dst, sst, vst, vmst);
    if (cst | dst | sst | vst | vmst) {
        return;
    }

    /* 2) host vidmem: one 64 KiB working buffer (pushbuf+GPFIFO+sem) FIXED-mapped at VAB
     *    in the private VAS, plus a 64 KiB USERD object handed to the channel. */
    struct nvkvm_host_map buf, ud;
    if (!nvkvm_m2_host_alloc_map_vidmem(s, C, DEV, BUF, 0x10000, &buf) ||
        !nvkvm_m2_host_alloc_map_vidmem(s, C, DEV, USERD, 0x10000, &ud)) {
        qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST: vidmem alloc/map failed\n", s->chip->name);
        return;
    }
    uint32_t mst = 0xffff; uint64_t outva = 0;
    int mrc = nvkvm_m2_map_dma(s, C, DEV, VIRT, BUF, 0, 0x10000, true, VAB, &mst, &outva);
    qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST map buf FIXED@0x%llx -> rc=%d st=0x%x va=0x%llx\n",
             s->chip->name, (unsigned long long)VAB, mrc, mst, (unsigned long long)outva);
    if (mrc != 0 || mst != 0 || outva != VAB) {
        return;
    }

    /* 3) TSG (KEPLER_CHANNEL_GROUP_A): engineType COPY0, our VAS.
     *    NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS {hObjectError,hObjectEccError,
     *    hVASpace@8,engineType@12,bIsCallingContextVgpuPlugin@16}. */
    uint8_t tsgp[20]; memset(tsgp, 0, sizeof(tsgp));
    stl_le_p(tsgp + 8, VAS);
    stl_le_p(tsgp + 12, 9u);                      /* NV2080_ENGINE_TYPE_COPY0 */
    nvkvm_m2_alloc1(s, C, DEV, TSG, 0xa06cu, tsgp, sizeof(tsgp), &st);
    uint32_t tst = st;

    /* 4) channel (AMPERE_CHANNEL_GPFIFO_A 0xc56f) under the TSG. 368B = the 580 guest's
     *    exact NV_CHANNEL_ALLOC_PARAMS size (psize=368 in the c56f DIAG): gpFifoOffset@8,
     *    gpFifoEntries@16, hUserdMemory[0]@32, userdOffset[0]@64, engineType@128.
     *    hContextShare=0/hVASpace=0 (TSG channel inherits; matches the forwarded allocs). */
    uint8_t cp[368]; memset(cp, 0, sizeof(cp));
    stq_le_p(cp + 8, GPF_VA);
    stl_le_p(cp + 16, 64u);                       /* gpFifoEntries */
    stl_le_p(cp + 32, USERD);                     /* hUserdMemory[0] */
    stl_le_p(cp + 128, 9u);                       /* engineType = COPY0 */
    nvkvm_m2_alloc1(s, C, TSG, CHAN, 0xc56fu, cp, sizeof(cp), &st);
    uint32_t chst = st;
    qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST tsg(a06c)=0x%x chan(c56f)=0x%x\n",
             s->chip->name, tst, chst);
    if (tst || chst) {
        return;
    }

    /* 5) AMPERE_USERMODE_A doorbell page under OUR subdevice, mmapped (mirror M5.8). */
    nvkvm_m2_alloc1(s, C, SUB, UM, 0xc561u, NULL, 0, &st);
    if (st != 0) {
        qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST: usermode alloc st=0x%x\n", s->chip->name, st);
        return;
    }
    if (s->m2_maph_next < 16) { s->m2_maph_next = 16; }
    uint32_t maph = s->m2_maph_next++;
    int mapfd = -1;
    if (nvkvm_isolate_open_device(&s->m2_iso, s->m2_iso_id, maph, NVKVM_DEV_GPU(0),
                                  O_RDWR, &mapfd) != 0 || mapfd < 0) {
        qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST: usermode map-fd open failed\n", s->chip->name);
        return;
    }
    struct nv_ioctl_nvos33_parameters_with_fd mm;
    memset(&mm, 0, sizeof(mm));
    mm.h_client = nvkvm_m2_client(s, C);
    mm.h_device = DEV;
    mm.h_memory = UM;
    mm.length   = 0x10000;
    mm.fd       = (int32_t)maph;
    unsigned int mc = (3u << 30) | ((unsigned int)sizeof(mm) << 16) |
                      ((unsigned int)'F' << 8) | NV_ESC_RM_MAP_MEMORY;
    uint32_t mnv = 0; uint64_t mf = 0;
    int rc = nvkvm_isolate_ioctl(&s->m2_iso, s->m2_iso_id, s->m2_ctl_h, mc,
                                 &mm, sizeof(mm), NULL, 0, 0, &mnv, &mf);
    if (rc != 0 || mm.status != 0) {
        qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST: usermode RM_MAP_MEMORY rc=%d st=0x%x\n",
                 s->chip->name, rc, mm.status);
        return;
    }
    void *um_qva = mmap(NULL, 0x10000, PROT_READ | PROT_WRITE, MAP_SHARED, mapfd, 0);
    if (um_qva == MAP_FAILED) {
        qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST: usermode mmap failed: %s\n",
                 s->chip->name, strerror(errno));
        return;
    }

    /* 6) BIND(engineType) + GPFIFO_SCHEDULE(bEnable=1) on the TSG (mirrors M5.41/M5.8).
     *    BIND must precede the token fetch: kfifoGenerateWorkSubmitTokenHal_GA100 returns
     *    NV_ERR_INVALID_STATE (0x40) until the channel is assigned a runlist, which is
     *    what NVA06C_CTRL_CMD_BIND does (ogkm kernel_fifo_ga100.c "not assigned to
     *    runlist yet"). */
    uint32_t bp = 9u; uint32_t bst = 0xffff;
    int brc = nvkvm_m2_control1(s, C, TSG, 0xa06c0102u, &bp, sizeof(bp), &bst);
    uint8_t sp[3]; memset(sp, 0, sizeof(sp)); sp[0] = 1;
    uint32_t scst = 0xffff;
    int src = nvkvm_m2_control1(s, C, TSG, 0xa06c0101u, sp, sizeof(sp), &scst);
    qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST BIND rc=%d st=0x%x | SCHEDULE rc=%d st=0x%x\n",
             s->chip->name, brc, bst, src, scst);
    if (brc != 0 || bst != 0 || src != 0 || scst != 0) {
        return;
    }

    /* 7) the channel's host work-submit token (NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN). */
    uint8_t tp[4]; memset(tp, 0, sizeof(tp));
    uint32_t tkst = 0xffff;
    int trc = nvkvm_m2_control1(s, C, CHAN, 0xc36f0108u, tp, 4, &tkst);
    uint32_t token = ldl_le_p(tp);
    qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST token rc=%d st=0x%x token=0x%08x (rl=%u chid=%u)\n",
             s->chip->name, trc, tkst, token, (token >> 16) & 0xffff, token & 0xffff);
    if (trc != 0 || tkst != 0) {
        return;
    }

    /* 8) pushbuffer: ONE incrementing method run SEM_ADDR_LO(0x5c)..SEM_EXECUTE(0x6c) =
     *    inline FIFO semaphore RELEASE of PAYLOAD to SEM_VA (32-bit, no WFI, no timestamp).
     *    GPFIFO entry 0 points at it (entry0=addr[31:2], entry1=addr[39:32]|len_dwords<<10). */
    uint32_t *pb = (uint32_t *)((uint8_t *)buf.qva + (PB_VA - VAB));
    pb[0] = (1u << 29) | (5u << 16) | (0u << 13) | (0x5cu >> 2);  /* INC, count=5, subch=0 */
    pb[1] = (uint32_t)(SEM_VA & 0xfffffffcu);                     /* SEM_ADDR_LO            */
    pb[2] = (uint32_t)((SEM_VA >> 32) & 0xffu);                   /* SEM_ADDR_HI            */
    pb[3] = PAYLOAD;                                              /* SEM_PAYLOAD_LO         */
    pb[4] = 0;                                                    /* SEM_PAYLOAD_HI         */
    pb[5] = 0x1u;                                                 /* SEM_EXECUTE: RELEASE   */
    volatile uint32_t *sem = (volatile uint32_t *)((uint8_t *)buf.qva + (SEM_VA - VAB));
    *sem = 0;                                                     /* sentinel               */
    uint8_t *gpf = (uint8_t *)buf.qva + (GPF_VA - VAB);
    stl_le_p(gpf + 0, (uint32_t)(PB_VA & 0xfffffffcu));
    stl_le_p(gpf + 4, (uint32_t)((PB_VA >> 32) & 0xffu) | (6u << 10));
    __sync_synchronize();
    (void)*(volatile uint32_t *)gpf;                              /* flush WC writes        */

    /* 9) submit: GP_PUT=1 in OUR host USERD, then ring the doorbell with the token. */
    stl_le_p((uint8_t *)ud.qva + 0x8C, 1u);                       /* USERD GP_PUT = 1       */
    __sync_synchronize();
    (void)ldl_le_p((uint8_t *)ud.qva + 0x8C);
    stl_le_p((uint8_t *)um_qva + 0x90, token);                    /* NOTIFY_CHANNEL_PENDING */
    qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST *** RANG token=0x%08x GP_PUT=1 — polling sem ***\n",
             s->chip->name, token);

    /* 10) poll the sem word (host GPU writes it) + host GP_GET, up to 5 s. */
    uint32_t semv = 0, gpget = 0;
    int ms = 0;
    for (ms = 0; ms <= 5000; ms += 50) {
        semv  = *sem;
        gpget = ldl_le_p((uint8_t *)ud.qva + 0x88);
        if (semv == PAYLOAD) {
            break;
        }
        g_usleep(50 * 1000);
    }
    qemu_log("nvkvm-gpu[%s] M5.45 SELFTEST VERDICT after %dms: sem=0x%08x (want 0x%08x) "
             "hostUSERD get=%u put=%u -> %s\n", s->chip->name, ms, semv, PAYLOAD, gpget,
             ldl_le_p((uint8_t *)ud.qva + 0x8C),
             (semv == PAYLOAD)
                 ? "*** SEM LANDED — host doorbell+schedule+USERD mechanics GOOD; blocker "
                   "is forwarded-channel guest<->host bridging ***"
                 : "*** SEM NEVER LANDED — host-side schedule/doorbell sequence itself is "
                   "broken (token/usermode/runlist/enable) ***");
}

/* M6.0: RAMBlock iterator — record the largest fd-backed (memfd) block as guest RAM. */
static int nvkvm_m2_find_guest_ram(RAMBlock *rb, void *opaque)
{
    NvkvmGpuEmul *s = opaque;
    int fd = qemu_ram_get_fd(rb);
    uint64_t len = qemu_ram_get_used_length(rb);
    if (fd >= 0 && len > s->m2_guest_ram_size) {
        s->m2_guest_ram_fd   = fd;
        s->m2_guest_ram_hva  = qemu_ram_get_host_addr(rb);
        s->m2_guest_ram_size = len;
    }
    return 0;
}

/* #90: flush on ANY QEMU exit path, not only device unrealize — a killed QEMU
 * must still leave a usable DENSE PREFIX (the only kind of truncation that is
 * not fatal for a positional differential). */
static void nvkvm_rec_exit_notify(Notifier *n, void *opaque)
{
    nvkvm_rec_close();
}

static void nvkvm_gpu_emul_realize(PCIDevice *pci_dev, Error **errp)
{
    NvkvmGpuEmul *s = NVKVM_GPU_EMUL(pci_dev);
    const NvkvmGpuChip *chip = &nvkvm_chip_ga106;
    uint8_t *cfg = pci_dev->config;

    s->chip = chip;
    g_nvkvm_dma_s = s;                    /* M5.15 DIAG: enable DMA-write logging hook */
    /* M5.11c PERF: arm the GPGA binary-search-index audit — cross-check the new sorted lookup
     * against the legacy linear scan for the first N lookups (covers load + early gen) to PROVE
     * semantic equivalence (m2_gpga_idx_mismatch must stay 0). Self-disabling; ~free once spent. */
    /* Proven idx_mismatch=0 over millions of lookups (non-overlap invariant holds) -> default the
     * cross-check OFF (it re-runs the O(n) legacy scan); re-enable for re-verification via m2trace. */
    s->m2_gpga_idx_audit = s->m2_trace ? 3000000 : 0;
    s->m2_gpga_idx_mismatch = 0;
    s->m2_gpga_idx_dirty = true;
    s->access_count = 0;

    /* ── #90: bring the §6 replay-trace recorder up BEFORE any BAR exists ──
     * so that not one access can precede the stream.  The header carries the
     * exact property vector, the declared filter, and free-text provenance
     * (NVKVM_M2REC_PROV, filled in by scripts/run_mode2_vm.sh with the guest
     * kernel vermagic, the host driver version and an nvidia-smi summary).  An
     * oracle whose provenance is not in the artefact stops being an oracle the
     * moment the bench dies. */
    if (s->m2rec) {
        uint64_t props =
            (s->trace      ? NVKVM_REC_P_TRACE     : 0) |
            (s->m2fwd      ? NVKVM_REC_P_M2FWD     : 0) |
            (s->m2exec     ? NVKVM_REC_P_M2EXEC    : 0) |
            (s->m2hostsem  ? NVKVM_REC_P_M2HOSTSEM : 0) |
            (s->m2cefwd    ? NVKVM_REC_P_M2CEFWD   : 0) |
            (s->m2cexec    ? NVKVM_REC_P_M2CEXEC   : 0) |
            (s->m2opaque   ? NVKVM_REC_P_M2OPAQUE  : 0) |
            (s->m2_trace   ? NVKVM_REC_P_M2TRACE   : 0) |
            (s->m2romregs  ? NVKVM_REC_P_M2ROMREGS : 0);
        /* ★ Hermeticity is a property of the RUN, and it is not ours to assume:
         * with m2fwd/m2exec on, nvkvm_m2_share_guest_ram MAP_FIXEDs guest RAM
         * into the stub and the HOST GPU DMAs into it directly — bytes that are
         * guest-visible and pass through neither nvkvm_dmaw nor nvkvm_dmar nor
         * any QEMU path.  Such a trace cannot be closed over by a replay.  Say
         * so in the artefact rather than leaving a reader to infer it. */
        bool hermetic = !s->m2fwd && !s->m2exec;
        if (!hermetic) {
            props |= NVKVM_REC_P_NONHERMETIC;
        }
        const char *extern_prov = getenv("NVKVM_M2REC_PROV");
        g_autofree char *prov = g_strdup_printf(
            "nvkvm mode-2 §6 replay trace\n"
            "chip=%s\n"
            "props: trace=%d m2fwd=%d m2exec=%d m2hostsem=%d m2cefwd=%d "
            "m2cexec=%d m2opaque=%d m2trace=%d m2romregs=%d\n"
            "mask=0x%016llx\n"
            "hermetic=%s%s\n"
            "vbios=%s\n"
            "---\n%s\n",
            chip->name, s->trace, s->m2fwd, s->m2exec, s->m2hostsem,
            s->m2cefwd, s->m2cexec, s->m2opaque, s->m2_trace, s->m2romregs,
            (unsigned long long)s->m2recmask,
            hermetic ? "yes" : "NO",
            hermetic ? ""
                     : "  (m2fwd/m2exec on: the HOST GPU writes guest RAM behind "
                       "this recorder; NOT replayable, decision planes only)",
            s->vbios_path ? s->vbios_path : "(none)",
            extern_prov ? extern_prov : "(no NVKVM_M2REC_PROV in the environment)");
        const char *path = s->m2recfile && s->m2recfile[0]
                         ? s->m2recfile : "/tmp/m0_rec.bin";
        if (nvkvm_rec_open(path, props, s->m2recmask, prov,
                           qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL))) {
            s->m2rec_exit.notify = nvkvm_rec_exit_notify;
            qemu_add_exit_notifier(&s->m2rec_exit);
            info_report("nvkvm-rec: recording to %s (mask=0x%016llx, %s)",
                        path, (unsigned long long)s->m2recmask,
                        hermetic ? "hermetic" : "NON-HERMETIC");
        } else {
            /* Fail loudly: a capture campaign that silently produced no file is
             * worse than one that did not start. */
            error_setg(errp, "nvkvm-rec: could not open trace sink %s", path);
            return;
        }
    }

    s->prom_reads = 0;
    s->mbox0 = 0;
    s->mbox1 = 0;
    s->bootargs_dumped = false;
    s->fwsec_ran = false;
    s->gsp_reloaded = false;
    s->q_ready = false;
    s->stat_writeptr = 0;
    s->stat_seqnum = 0;
    s->cmd_readptr = 0;

    /* M6: sparse FB backing for the BAR0 PRAMIN window (value = g_malloc0'd 4 KiB). */
    s->bar0_window = 0;
    s->bar2_inst_block = 0;
    s->bar2_virtual = false;
    s->bar2_pdb = 0;
    s->fb_pages = g_hash_table_new_full(g_direct_hash, g_direct_equal,
                                        NULL, g_free);

    /* M2: load the VBIOS image for the PROM window (if a path was given). */
    s->vbios = NULL;
    if (s->vbios_path && s->vbios_path[0]) {
        FILE *f = fopen(s->vbios_path, "rb");
        if (!f) {
            error_setg(errp, "nvkvm-gpu-emul: cannot open vbios '%s'",
                       s->vbios_path);
            return;
        }
        s->vbios = g_malloc0(NV_PROM_DATA_SIZE);
        size_t n = fread(s->vbios, 1, NV_PROM_DATA_SIZE, f);
        fclose(f);
        if (s->vbios[0] != 0x55 || s->vbios[1] != 0xAA) {
            warn_report("nvkvm-gpu-emul: vbios '%s' lacks 0x55AA signature "
                        "(read %zu bytes)", s->vbios_path, n);
        }
    }

    /* Class 0x030000 = VGA-compatible 3D controller, as a real GeForce reports.
     * (Mode-2's "display for free" rides on this.) */
    pci_set_word(cfg + PCI_SUBSYSTEM_VENDOR_ID, chip->sub_vendor_id);
    pci_set_word(cfg + PCI_SUBSYSTEM_ID,        chip->sub_device_id);

    /* BAR0: REGS, 32-bit non-prefetchable MMIO (matches real GA10x). */
    /* BAR0 is a CONTAINER: the trapped MMIO ops fill all of it at priority 0; the M5.64 GSP-falcon
     * rom-device overlays page 0x110000 at priority 1 so its poll-READS hit RAM (no vmexit) while
     * WRITES fall through the rom-device thunk to the side-effect handler. (A plain leaf-with-
     * subregion overlay did NOT take effect for KVM — m582; a container renders subregions reliably.) */
    memory_region_init(&s->bar0, OBJECT(s), "nvkvm-gpu-bar0", chip->bar0_size);
    memory_region_init_io(&s->bar0_io, OBJECT(s), &nvkvm_bar0_ops, s,
                          "nvkvm-gpu-regs", chip->bar0_size);
    memory_region_add_subregion_overlap(&s->bar0, 0, &s->bar0_io, 0);
    if (s->m2romregs) {
        memory_region_init_rom_device(&s->gsp_falcon, OBJECT(s), &nvkvm_gsp_falcon_ops, s,
                                      "nvkvm-gsp-falcon", 0x1000, &error_fatal);
        s->gsp_falcon_ram = memory_region_get_ram_ptr(&s->gsp_falcon);
        nvkvm_gsp_falcon_sync(s);
        memory_region_add_subregion_overlap(&s->bar0, 0x00110000u, &s->gsp_falcon, 1);
    }
    pci_register_bar(pci_dev, 0, PCI_BASE_ADDRESS_SPACE_MEMORY, &s->bar0);

    /* BAR1: FB aperture, 64-bit prefetchable (occupies BAR1+BAR2). */
    memory_region_init_io(&s->bar1, OBJECT(s), &nvkvm_aperture_ops, s,
                          "nvkvm-gpu-fb", chip->bar1_size);
    pci_register_bar(pci_dev, 1,
                     PCI_BASE_ADDRESS_SPACE_MEMORY |
                     PCI_BASE_ADDRESS_MEM_TYPE_64 |
                     PCI_BASE_ADDRESS_MEM_PREFETCH, &s->bar1);

    /* BAR3: usermode/IMEM aperture, 64-bit prefetchable (occupies BAR3+BAR4).
     * Driver assigns FB then IMEM to the next valid 64-bit BARs after REGS. */
    /* BAR3 == RM "BAR2": the 32 MiB GPU-virtual instance/PTE aperture.  GMMU-VER2
     * walked to the FB backing (M6).  (PCI BAR0=regs, BAR1=RM BAR1 FB window.) */
    memory_region_init_io(&s->bar3, OBJECT(s), &nvkvm_bar2_ops, s,
                          "nvkvm-gpu-bar2", chip->bar3_size);
    pci_register_bar(pci_dev, 3,
                     PCI_BASE_ADDRESS_SPACE_MEMORY |
                     PCI_BASE_ADDRESS_MEM_TYPE_64 |
                     PCI_BASE_ADDRESS_MEM_PREFETCH, &s->bar3);

    /* MSI-X (BAR5): the driver expects message interrupts (GSP->CPU doorbell,
     * engine completion).  Table lives in its own BAR; we raise vectors from
     * host eventfd/poll later (mode2_interrupt_delivery). */
    memory_region_init(&s->msix, OBJECT(s), "nvkvm-gpu-msix", 0x1000);
    pci_register_bar(pci_dev, 5, PCI_BASE_ADDRESS_SPACE_MEMORY, &s->msix);
    if (msix_init(pci_dev, NVKVM_GPU_MSIX_VECTORS,
                  &s->msix, 5, 0x0,
                  &s->msix, 5, 0x800,
                  0x00, errp) < 0) {
        /* Non-fatal for M0 observation: fall back to INTx so the device still
         * enumerates and we capture the BAR0 trace. */
        error_free(*errp);
        *errp = NULL;
    } else {
        for (int v = 0; v < NVKVM_GPU_MSIX_VECTORS; v++) {
            msix_vector_use(pci_dev, v);
        }
    }

    /* PCIe capability so the driver sees an Express endpoint, not legacy PCI.
     * Only valid when actually on a PCIe bus (q35 root complex / root port);
     * on a conventional PCI bus we enumerate as a plain PCI device.  A real
     * GeForce is express, so prefer plugging this behind a pcie-root-port. */
    /* This device is a "hybrid" — it declares BOTH INTERFACE_CONVENTIONAL_PCI_
     * DEVICE and INTERFACE_PCIE_DEVICE, so QEMU does NOT auto-set
     * QEMU_PCI_CAP_EXPRESS (see do_pci_register_device: only pure-PCIe devices
     * get it).  Behind a pcie-root-port the bus IS express, so set it manually
     * — otherwise pci_is_express() is false, the Express cap is never added,
     * and the driver's link-rate read at config 0x88 returns 0. */
    if (pci_bus_is_express(pci_get_bus(pci_dev))) {
        pci_dev->cap_present |= QEMU_PCI_CAP_EXPRESS;
    }
    if (pci_is_express(pci_dev)) {
        /* Place the PCIe Express capability at config 0x78 — EXACTLY where a real
         * GA10x / RTX 3060 puts it ("Capabilities: [78] Express").  The NVIDIA
         * driver reads its link via GPU_BUS_CFG_RD32 at the ABSOLUTE config
         * offset NV_XVE_LINK_CONTROL_STATUS=0x88, which is only the cap's
         * LINK_CONTROL_STATUS when the cap base is 0x78 (0x78+0x10).  With the
         * cap elsewhere (QEMU default 0x60) the driver read garbage ->
         * calculatePCIELinkRateMBps "Unknown PCIe speed" -> NV_ERR_INVALID_STATE.
         * MSI-X auto-placed near 0x40, so 0x78..0xB4 is free. */
        uint8_t exp = 0x78;
        pcie_endpoint_cap_init(pci_dev, exp);
        /* LINK_CAP @ 0x84 (cap+0xC): MAX_LINK_SPEED[3:0]=4 (16GT/s), width[9:4]=16
         * — matches the real card's LnkCap 0x00453d04. */
        uint32_t lnkcap = pci_get_long(cfg + exp + PCI_EXP_LNKCAP);
        lnkcap = (lnkcap & ~0x3FFu) | 4u | (16u << 4);
        pci_set_long(cfg + exp + PCI_EXP_LNKCAP, lnkcap);
        /* LINK_CONTROL_STATUS @ 0x88: LNKSTA @ 0x8A -> dword 0x88[31:16].
         * CURRENT_LINK_SPEED[19:16]=4, NEG_LINK_WIDTH[25:20]=16. */
        pci_set_word(cfg + exp + PCI_EXP_LNKSTA, (uint16_t)(4u | (16u << 4)));
    }

    /* M6.0 (item-4 prereq): locate the guest-RAM memfd so the stub can later mmap any guest
     * GPA + OS_DESCRIPTOR it for host-GPU DMA. Pick the largest fd-backed RAMBlock (the guest
     * RAM when run with -object memory-backend-memfd,share=on). fd=-1 if anon RAM. */
    s->m2_guest_ram_fd = -1;
    nvkvm_handle_table_init(&s->m2_ht);   /* M6.1: fd registry for sharing guest RAM to stub */
    qemu_ram_foreach_block(nvkvm_m2_find_guest_ram, s);
    qemu_log("nvkvm-gpu[%s] M6.0 guest-RAM memfd: fd=%d hva=%p size=0x%llx %s\n",
             chip->name, s->m2_guest_ram_fd, s->m2_guest_ram_hva,
             (unsigned long long)s->m2_guest_ram_size,
             s->m2_guest_ram_fd >= 0 ? "[shareable to stub for item-4]"
                                     : "[anon RAM — add memory-backend-memfd,share=on]");

    /* M5.1: forwarding (m2fwd) is lazy — the per-guest host isolate is created on
     * the first forwarded alloc in the cmdq path (nvkvm_m2_shadow_fwd). */
    /* M5.3: one-shot data-plane proof (QEMU mmaps real host GPU vidmem). */
    if (s->m2fwd) {
        nvkvm_m2_memtest(s);
        /* M5.45: host-channel self-test — OFF unless NVKVM_SELFTEST=1 in the env
         * (diagnostic only; never runs in normal boots). */
        const char *selftest = getenv("NVKVM_SELFTEST");
        if (selftest && selftest[0] == '1') {
            nvkvm_m2_channel_selftest(s);
        }
    }
}

static void nvkvm_gpu_emul_exit(PCIDevice *pci_dev)
{
    NvkvmGpuEmul *s = NVKVM_GPU_EMUL(pci_dev);
    if (s->m2rec) {
        info_report("nvkvm-rec: %llu records",
                    (unsigned long long)nvkvm_rec_count());
        qemu_remove_exit_notifier(&s->m2rec_exit);
        nvkvm_rec_close();          /* #90 */
    }
    msix_unuse_all_vectors(pci_dev);
    msix_uninit(pci_dev, &s->msix, &s->msix);
    g_free(s->vbios);
    if (s->fb_pages) {
        g_hash_table_destroy(s->fb_pages);
    }
}

/* ── QOM boilerplate ───────────────────────────────────────────────────────*/

static Property nvkvm_gpu_emul_props[] = {
    DEFINE_PROP_BOOL("trace", NvkvmGpuEmul, trace, true),
    /* Host-GPU forwarding is the ONLY supported Mode-2 operating mode (there is no
     * pure-emulation-without-host-GPU path).  Default ON; the props remain solely as a
     * DEBUG off-switch for the no-host-GPU fake-the-boot bring-up (M0-M3). */
    DEFINE_PROP_BOOL("m2fwd", NvkvmGpuEmul, m2fwd, true), /* M5: host-GPU forwarding (always on) */
    DEFINE_PROP_BOOL("m2exec", NvkvmGpuEmul, m2exec, true), /* M5.7: execution-plane backing (always on) */
    DEFINE_PROP_BOOL("m2hostsem", NvkvmGpuEmul, m2hostsem, false), /* M5.35: host owns completion sema */
    DEFINE_PROP_BOOL("m2cefwd", NvkvmGpuEmul, m2cefwd, false), /* CE-fwd: Step-0 route probe + Phase-A (M5.60) real-back the user-CE copy dst in the GR fvas (completion/CPU-copy unchanged) */
    DEFINE_PROP_BOOL("m2cexec", NvkvmGpuEmul, m2cexec, false), /* CE-EXEC fwd (A): host CE runs the user-CE LAUNCH_DMA; suppress CPU copy+sema. Sub-flag of m2cefwd. Default OFF (A/B) */
    DEFINE_PROP_BOOL("m2opaque", NvkvmGpuEmul, m2opaque, false), /* M5.62: skip GPFIFO walk when a userspace channel is fully resident (ring-only). Default OFF (perf experiment) */
    DEFINE_PROP_BOOL("m2trace", NvkvmGpuEmul, m2_trace, false), /* M5.63: high-volume per-doorbell/fb-access DIAG qemu_log. Default OFF (perf: synchronous log I/O overhead) */
    DEFINE_PROP_BOOL("m2romregs", NvkvmGpuEmul, m2romregs, false), /* M5.64: GSP-falcon rom-device overlay (reads from RAM, no vmexit) — 0x110094 poll-storm fix. Default OFF (A/B) */
    /* #90: the §6 replay-trace recorder.  A NEW property on purpose — m2trace
     * is NOT observationally neutral (it arms the GPGA index audit, sets
     * m2_crashwin, and adds two nvkvm_fb_read calls), so it changes what the
     * device DOES.  m2rec only observes. */
    DEFINE_PROP_BOOL("m2rec", NvkvmGpuEmul, m2rec, false),
    DEFINE_PROP_STRING("m2recfile", NvkvmGpuEmul, m2recfile),
    /* The DECLARED FILTER (NVKVM_REC_M_*), written verbatim into the file
     * header.  Default = everything.  Never sample, never cap: the consumer's
     * differential is positional, so a drop corrupts every later position; only
     * a filter both recorders can apply identically is legitimate. */
    DEFINE_PROP_UINT64("m2recmask", NvkvmGpuEmul, m2recmask, NVKVM_REC_M_ALL),
    DEFINE_PROP_UINT64("m2semval", NvkvmGpuEmul, m2semval, 0), /* M5.14 DIAG: ctx-poll sentinel */
    DEFINE_PROP_UINT64("m2sempage", NvkvmGpuEmul, m2sempage, 0x2efbaf000ull), /* M5.14 page */
    DEFINE_PROP_STRING("vbios", NvkvmGpuEmul, vbios_path),
    DEFINE_PROP_END_OF_LIST(),
};

static void nvkvm_gpu_emul_class_init(ObjectClass *klass, void *data)
{
    DeviceClass    *dc = DEVICE_CLASS(klass);
    PCIDeviceClass *k  = PCI_DEVICE_CLASS(klass);
    const NvkvmGpuChip *chip = &nvkvm_chip_ga106;

    k->realize   = nvkvm_gpu_emul_realize;
    k->exit      = nvkvm_gpu_emul_exit;
    k->vendor_id = chip->vendor_id;
    k->device_id = chip->device_id;
    k->revision  = chip->revision;
    k->class_id  = PCI_CLASS_DISPLAY_VGA;   /* 0x0300; we override prog-if=0 */

    dc->desc = "nvkvm Mode-2 emulated NVIDIA GPU (GA106)";
    device_class_set_props(dc, nvkvm_gpu_emul_props);
    set_bit(DEVICE_CATEGORY_DISPLAY, dc->categories);
}

static const TypeInfo nvkvm_gpu_emul_info = {
    .name          = TYPE_NVKVM_GPU_EMUL,
    .parent        = TYPE_PCI_DEVICE,
    .instance_size = sizeof(NvkvmGpuEmul),
    .class_init    = nvkvm_gpu_emul_class_init,
    .interfaces    = (InterfaceInfo[]) {
        { INTERFACE_CONVENTIONAL_PCI_DEVICE },
        { INTERFACE_PCIE_DEVICE },
        { },
    },
};

static void nvkvm_gpu_emul_register_types(void)
{
    type_register_static(&nvkvm_gpu_emul_info);
}
type_init(nvkvm_gpu_emul_register_types)
