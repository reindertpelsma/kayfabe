/*
 * nvkvm.c — the QOM half of the kayfabe device.  QOM ONLY.
 *
 * `l2_qemu_adapter.md` decision Q2: this file contains the type, class_init, realize and
 * unrealize, the three reset phases, the region table and its callbacks, and the table of
 * primitives the Rust archive is handed.  **No logic.**  Every function below is either a
 * hypervisor call, a field read, or a hand-off across `kayfabe_shim.h`.
 *
 * ══ THE THREE THINGS TO READ FIRST ══
 *
 * 1. ★★★ THE RESERVATION REGISTERS ARE PURE MMIO AND MUST STAY THAT WAY.
 *    `memory_region_init_io` never sets the region's RAM flag, so the accelerator's memory
 *    listener takes an unconditional early return for our registers — before allocating a
 *    slot, in BOTH directions.  The hypervisor therefore never creates, deletes or even looks
 *    up a slot for those guest-physical ranges, in any code path including a base-address
 *    register move, and the archive's own slots over the same range cannot be clobbered by
 *    it.  **That early return is the entire safety argument for installing foreign slots.**
 *    Registering one of these with a RAM-backed constructor instead would give it a
 *    hypervisor-managed slot over the same range as ours, and only one of the two can win.
 *    `nvkvm_op_bar_is_unbacked_reservation` is how the archive asks rather than assumes; keep
 *    it truthful.
 *
 * 2. ★★ THE REGION TABLE IS THE ENUMERATION.  There is one table (`nvkvm_regions`), one
 *    constructor (`nvkvm_region_init_io`), one registration loop (`nvkvm_bars_realize`) and
 *    one realize-time self-check (`nvkvm_regions_selfcheck`).  A grep cannot answer "did we
 *    miss a region?" — a missed region is an OMISSION and an omission has no token to match —
 *    so it is answered structurally.  Do not construct a region anywhere else.
 *
 * 3. ★ THE ARCHIVE IS REALIZED LATE, NOT AT `realize`.  At the moment a PCI device realizes,
 *    its base-address registers are unprogrammed; firmware assigns them afterwards.  The
 *    archive's memory plane needs a base, so it is realized from the configuration-space write
 *    path, the first time every register this device owns has one.  The C artifact reaches the
 *    same shape from the other direction and installs on first use.
 */

#include "qemu/osdep.h"
#include "nvkvm_compat.h"

#include "hw/pci/pci.h"
#include "hw/pci/pci_device.h"
#include "hw/pci/msix.h"
#include "hw/qdev-properties.h"
#include "hw/resettable.h"
#include "migration/blocker.h"
#include "qapi/error.h"
#include "qemu/error-report.h"
#include "qemu/main-loop.h"
#include "qemu/module.h"
#include "qemu/range.h"
#include "qemu/units.h"
#include "qom/object.h"
/* ★ §16.16 — `get_system_memory()`, for the trap-status table. QEMU 10.2 moved this header
 * from `exec/` to `system/`; the bench tree carries only the `system/` spelling. */
#include "system/address-spaces.h"
/* ★ For `RAMBlock::fd_offset`, read as a field in the topology listener. Same justification
 * as `MemoryRegion::rom_device` there: no public accessor answers it, and the alternative
 * is an assumption with nothing to catch it. */
#include "system/ramblock.h"
#include "system/system.h"

#include "kayfabe_shim.h"

#define TYPE_NVKVM "nvkvm-gpu"
OBJECT_DECLARE_SIMPLE_TYPE(NvkvmState, NVKVM)

/* Kept in step with `nvkvm_regions` by a build-time assertion in nvkvm_bars_realize. */
#define NVKVM_N_REGIONS 4

/* The table row the message-signalled-interrupt container occupies.  Named, and asserted
 * against the table in nvkvm_regions_selfcheck, so it cannot drift into a row that is a
 * reservation — which would hand the archive's memslots and the hypervisor's vector table
 * the same guest-physical range. */
#define NVKVM_MSIX_ROW 3

typedef enum NvkvmRegionKind {
    /* Accesses trap to this device. */
    NVKVM_KIND_TRAP = 0,
    /* ★ A pure-MMIO reservation the archive shadows with its own slots.  Its callbacks are
     * not reached in normal operation; if one fires, the shadow is missing. */
    NVKVM_KIND_RESERVATION = 1,
    /*
     * ★★ The message-signalled-interrupt table's own register.
     *
     * A CONTAINER, not an io region: `msix_init` adds the table and the pending-bit array
     * into it as subregions, and a leaf built by `memory_region_init_io` has nowhere to put
     * them.  It is in this table rather than beside it for the reason the table exists —
     * `nvkvm_regions_selfcheck` counts constructor calls against rows, so a region built
     * anywhere else is an omission the count catches.
     */
    NVKVM_KIND_MSIX = 2,
} NvkvmRegionKind;

typedef struct NvkvmRegionSpec {
    const char            *name;
    /*
     * ★★★ TWO indices, and conflating them was a REAL BUG this table now prevents.
     *
     * `port_index` is what the archive calls the register: 0, 1, 2, dense, and the only
     * thing that crosses `kayfabe_shim.h`.  `pci_bar` is the hardware base-address register
     * the region is registered at, and a 64-BIT register CONSUMES TWO of them — so three
     * 64-bit regions live at 0, 2 and 4, never at 0, 1 and 2.
     *
     * The first build of this device used one index for both.  It registered three 64-bit
     * regions at 0, 1 and 2, they overlapped, and the device came up reporting register 0 and
     * register 1 at the SAME guest-physical base with register 2 holding the high half of its
     * neighbour's address.  A reservation was then installed over the wrong register's range
     * — the exact "two descriptions of one range that disagree" this file's own refusals are
     * written to prevent, arriving through the table instead of past it.
     * `nvkvm_regions_selfcheck` now asserts the spacing so it cannot recur silently.
     */
    uint32_t               port_index;
    int                    pci_bar;
    NvkvmRegionKind        kind;
    /*
     * ★★★ 64-BIT-NESS IS PER ROW, and making it so is a CORRECTION, not a feature.
     *
     * Every row used to be registered 64-bit prefetchable, including the register aperture.
     * A real GA10x reports its register aperture as a 32-bit NON-prefetchable region and
     * both halves of that matter: a prefetchable register window tells a guest its reads
     * are side-effect-free, which for a register plane is false, and the C artifact — the
     * only implementation a real driver has ever accepted — registers exactly
     * `PCI_BASE_ADDRESS_SPACE_MEMORY` there (`C: src/qemu/nvkvm_gpu_emul.c:9804`).
     *
     * It also frees a hardware register: three 64-bit regions consume 0+1, 2+3, 4+5 and
     * leave nowhere for the interrupt table.  With the register aperture 32-bit the layout
     * becomes 0, 1+2, 3+4, 5 — which is the C's layout exactly, arrived at from the same
     * constraint.
     */
    bool                   bar64;
    /* Byte offset of the uint64_t size property inside NvkvmState.  A row cannot name a size
     * directly, because the sizes are operator-settable. */
    size_t                 size_off;
    const MemoryRegionOps *ops;
} NvkvmRegionSpec;

/* How many BAR1 accesses nvkvm_bar1_{read,write} record in full.  ⊘ Bounded because the
 * guest chooses how many it issues; the TOTAL is `bar1_touches`, and the two are printed
 * together so a truncation is visible rather than silent. */
#define NVKVM_BAR1_LOG 16u

/* ★★★★★ ITEM 2 / w262 — GP_PUT INSIDE USERD, and WHY IT IS PRINTED LIVE.
 *
 * `kayfabe_abi::submit::USERD_GP_PUT` is 0x8c.  A guest CPU store of the GPFIFO put pointer
 * to its own USERD is therefore a 4-byte BAR1 write whose page offset is exactly 0x8c —
 * `[measured, boot s17_e8fde62 and ~78 boots since]` off=0xa008c, 0xc008c, 0xe008c, 0x10008c.
 *
 * ⊘⊘ THOSE ROWS EXIST ALREADY AND CANNOT ANSWER AN ORDERING QUESTION.  nvkvm_bar1_record
 * STORES them and prints NOTHING; the array is dumped once, from nvkvm_report_audit, at
 * teardown.  So the timestamp on a `BAR1[2] WRITE off=0xa008c` row is the DUMP's, not the
 * guest's, and the row sits after every other line in the file.  `[measured 2026-08-12,
 * traces/boots/w261/run_w261_ring_qemu.log]` the first GR channel birth is at ~05:43:36 and
 * the BAR1 dump is at 05:46:34 — a fixed 178 s that is the distance to power-down and not a
 * fact about the guest.  A reader ordering `GP_PUT` against the engine-object alloc off that
 * file gets the FAVOURABLE answer, on every boot, whatever the truth is.
 *
 * ⇒ This prints AT THE INSTANT OF THE STORE, so it carries QEMU's own -msg timestamp and
 * interleaves with `kayfabe: ENGINE-OBJECT …` in file order.  That is exactly the argument
 * KayfabeRegWrite::doorbell already makes one aperture over: *"a doorbell is a property of
 * ONE WRITE … so the shell logs it as it happens, against QEMU's own -msg timestamp … A
 * per-boot counter cannot be stamped."*
 *
 * ⚠ WHAT IT DOES **NOT** SAY, printed on the line itself.  Nothing here joins a BAR1 offset
 * to a CHANNEL — `kayfabe-mmu`'s gpga.rs forbids reverse-resolution by address in as many
 * words — so this witnesses *when the guest first advances a cursor at all*, never *which
 * channel's*.  And the four recorded pairs are attributable, from the driver's own source
 * (see NvkvmState::bar1_log), to nvidia-uvm's internal_channel_submit_work, whose channels
 * are NOT the GR channel leg B would adopt.
 *
 * ⊘ Bounded prints, UNBOUNDED total: the count is reported at teardown from a counter this
 * cap never touches, so a printed count can never be mistaken for a total. */
#define NVKVM_USERD_GP_PUT 0x8cu
#define NVKVM_GP_PUT_LIVE  8u

/* ★★★★★ w279 MEASURED — THIS DETECTOR HAS FALSE POSITIVES, AND ITS OWN ARTEFACT NAMED THEM.
 *
 * The test above is `page offset == 0x8c && size == 4`, on ANY BAR1 page.  It is exactly as
 * strong as the assumption that every BAR1 page a guest writes is a USERD, and `w278` broke
 * that assumption by introducing a workload that CPU-maps its own vidmem data buffers:
 *
 *   `[measured 2026-08-12, traces/boots/w278/run_w278b_guest_qemu.log.gz]`
 *     BAR1 GP_PUT #1 aperture +0x9008c val=0xc0ffee56
 *     BAR1 GP_PUT #2 aperture +0xa008c val=0x3f0011cc
 *     BAR1[0] WRITE off=0x90000 val=0xc0ffee33   BAR1[1] WRITE off=0xa0000 val=0x3f0011cc
 *
 * Pages +0x90000 and +0xa0000 are the raw CE client's SOURCE and DESTINATION buffers — the
 * same two magic words it prints as its payload — so the two "advances" on them are the
 * client's DATA, sixteen dwords into a 4 KiB buffer.  ⇒ Two of the eight lines that boot
 * printed, and two of the four "distinct USERD page(s)", are not cursors at all.
 *
 * ⊘ THE FIX IS A LABEL, NOT A FILTER.  A put pointer indexes a ring, so it is bounded by the
 * largest GPFIFO this tree has ever seen (4096 entries, the kernel's; the client's is 64).
 * `0xc0ffee56` cannot be one.  The converse does NOT hold — a small data word is
 * indistinguishable from a cursor here — so this marks the rows it can PROVE are not cursors
 * and claims nothing about the rest.  Dropping them silently would delete evidence; leaving
 * the line's positive claim ("the guest advanced a GPFIFO put pointer") on them is a
 * measured falsehood.  ⇒ The claim is now conditional, and the count of disproved rows is
 * reported beside the total so the total can never be read as a cursor count. */
#define NVKVM_GP_PUT_MAX_ENTRIES 4096u

/* ★★★★★ w262 MEASURED, AND IT IS WHY THIS SECOND INSTRUMENT EXISTS.
 *
 * `[measured 2026-08-12, boots w262_off and w262_ring, GA106 / 580.159.04]` the flat cap above
 * printed **8** live cursor advances out of **188**.  The 8 are enough to answer *"when is the
 * guest's FIRST cursor advance"* — they all precede every host channel birth of the walling GR
 * client — and they are **not** enough to answer the question leg B actually needs, which is
 * whether the **GR channel's own** USERD page ever advances, and when.  180 advances were
 * counted and not placed.
 *
 * ⊘ Raising the flat cap to 188 would be the wrong fix: it makes the log 180 lines longer and
 * still answers by eyeball.  The question is about **pages**, because one page is one channel's
 * USERD — so this records the FIRST advance on each distinct page, and counts the rest per
 * page.  `[measured]` the whole workload uses four pages (0xa0000 / 0xc0000 / 0xe0000 /
 * 0x100000, `0x20000` apart, nvidia-uvm's internal channel pool), so 16 is four times the
 * observed need and a fifth page appearing is itself the news.
 *
 * ⚠ AND THE OVERFLOW IS COUNTED SEPARATELY, because the page this table drops is exactly the
 * page a reader would most want: a table that silently stopped recording new pages would answer
 * *"only these four channels ever advanced a cursor"* when it meant *"only these four fit"*. */
#define NVKVM_GP_PUT_PAGES 16u

struct NvkvmState {
    PCIDevice parent_obj;

    /* --- properties ------------------------------------------------------------- */
    uint64_t bar0_size;
    uint64_t bar1_size;
    uint64_t bar2_size;
    uint64_t msix_size;
    uint64_t window_size;
    /* ★★★★ Whether a shadow memslot was actually INSTALLED.  ⊘ §16.18: BAR1 now TRAPS,
     * so a shadow over it is REFUSED at realize — a slot there would answer the guest out
     * of memory the framebuffer store and the page walk cannot see, and a read back through
     * the same slot would agree with it.  This therefore stays false on every supported
     * configuration, and it is kept so that the refusal has something to report against. */
    bool window_installed;
    bool     shareable_ram;
    /* 0 = the chip table's default row.  A hex PCI device id selects another. */
    uint32_t chip_device_id;
    /* ★ PROBE ONLY, default NULL (= empty).  Comma-separated decimal notifier indices to
     * probe-arm.  Parsed STRICTLY by the archive: junk refuses realize by name rather
     * than booting probe-off, and the set in effect comes back in the end-of-run census
     * so the boot's own report proves what it ran with. */
    char    *probe_arm_notifier;

    /* --- regions ---------------------------------------------------------------- */
    MemoryRegion mr[NVKVM_N_REGIONS];
    /* ★ The constructor-call counter.  Its whole job is to disagree with the table's row
     * count if somebody ever builds a region another way. */
    unsigned     io_inits;

    /* --- lifecycle -------------------------------------------------------------- */
    Error   *migrate_blocker;
    bool     discard_disabled;
    bool     want_listener;
    bool     listening;
    MemoryListener listener;

    void    *shim;         /* the archive's memory-plane handle, or NULL */
    bool     shim_refused; /* refused once, loudly; never retried */
    /* ★★ The archive's REGISTER-plane handle.  A second handle with its own lifetime — see
     * kayfabe_shim.h for why it is not the same one.  Created at realize, because unlike
     * the memory plane it needs no base-address register to exist. */
    void    *regs;
    /* ★★ Stage Q5: the register plane has been given the memory plane's guest RAM.  Kept
     * so the teardown order is a fact rather than an assumption, and so the realize report
     * can say which of the two states this device is actually in — the difference is
     * whether the emulated GSP can follow a guest pointer at all. */
    bool     regs_have_ram;
    /* MSI-X was asked for and the hypervisor refused it.  Recorded rather than fatal: the
     * guest driver's own gate is satisfied by the capability OR by a legacy line, and a
     * device that cannot enumerate tells nobody anything. */
    bool     msix_refused;
    /* One-shot, so a guest polling a doorbell cannot fill the log with the same refusal. */
    bool     irq_refusal_reported;
    /* ★ The register audit is printed from BOTH device teardown and process exit, because
     * a plain machine shutdown reaches neither `exit` nor `unrealize` — the device is never
     * unplugged, the process simply ends.  Measured: the first run of this device printed
     * nothing at shutdown and the counters were unobservable in exactly the case an
     * operator cares about most.  `audit_printed` keeps the two paths from double-printing.
     */
    Notifier exit_notifier;
    bool     audit_printed;
    /* How many vectors the chip table says this device offers. */
    uint16_t msix_vectors;
    uint64_t reset_epoch;
    bool     traps_open;

    uint64_t trap_reads;
    uint64_t trap_writes;
    /* ★ §16.18 — every access that reached nvkvm_bar1_{read,write}, whether or not the
     * bounded log had room for it.  `bar1_log_used` alone cannot say whether the log is a
     * sample or the whole of it, and the archive's own bar1_* counters cannot either: they
     * count what the ADDRESS MODEL did, and an access is recorded here before that runs. */
    uint64_t bar1_touches;
    /* ★★★★ §16.17 — THE BAR1 ACCESS LOG, not just its count.
     *
     * `[src] ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:984-1015`
     * `internal_channel_submit_work` writes the GPFIFO entry through a **CPU POINTER**
     * (`channel->channel_info.gpFifoEntries + channel->cpu_put`, dereferenced), then
     * `mb()`, then `write_gpu_put`.  For a VIDMEM ring that CPU mapping is a **BAR1**
     * mapping — and until §16.18 nvkvm_reservation_write DESTROYED the value
     * (`(void)val;`).  It now routes to the archive's GMMU translation, and this log is
     * what says which offsets arrived.
     *
     * ⇒ The count alone cannot test that.  `[measured, boot s16_5fcd259]` it is **3**, and
     * 3 is EXACTLY what the hypothesis predicts (one 8-byte entry store, possibly split,
     * plus a GP_PUT store) — but it is equally what "the guest barely touched BAR1"
     * predicts.  ⊘ A number consistent with two opposite readings decides nothing.  The
     * ADDRESSES and VALUES decide: an 8-byte store of a plausible GPFIFO entry at a BAR1
     * offset, immediately before the doorbell, is a different fact from three stray probes.
     *
     * ⚠ Precondition, printed with it: this records only accesses that REACH THE HANDLER,
     * and it is a complete census only while no shadow memslot covers the range.  §16.18
     * refuses such a shadow outright, so on a supported configuration it is complete — but
     * the condition is stated rather than assumed, because the day it stops holding this
     * log would silently become a sample. */
    struct {
        uint64_t addr;
        uint64_t val;
        unsigned size;
        bool     is_write;
    } bar1_log[NVKVM_BAR1_LOG];
    unsigned bar1_log_used;
    /* ★★★★★ ITEM 2 / w262 — the GP_PUT-shaped BAR1 writes, counted without a cap, and the
     * number of them that were printed live.  TWO numbers for the reason NvkvmState's IRQ
     * pair gives: "we printed none because there were none" and "we printed none because the
     * cap was already spent" are the same absence in a log otherwise. */
    uint64_t gp_put_writes;
    unsigned gp_put_printed;
    /* ★★★★★ w262 — ONE ROW PER USERD PAGE, i.e. per channel.  See NVKVM_GP_PUT_PAGES. */
    struct {
        uint64_t page;      /* the BAR1 aperture page, addr & ~0xfff */
        uint64_t first_val; /* the value of the FIRST advance on it */
        uint64_t writes;    /* how many advances landed on it, uncapped */
    } gp_put_pages[NVKVM_GP_PUT_PAGES];
    unsigned gp_put_pages_used;
    /* Advances on a page the table had no room for.  ⊘ A separate number, never folded into
     * the totals: "no fifth page appeared" and "a fifth page appeared and was dropped" are
     * the two readings a full table cannot otherwise be told apart. */
    uint64_t gp_put_pages_dropped;
    /* ★★★★★ w279 — of `gp_put_writes`, how many carried a value that CANNOT be a put
     * pointer.  See NVKVM_GP_PUT_MAX_ENTRIES.  ⊘ A lower bound on the false positives and
     * never an upper one: a data word that happens to be small is unprovable either way. */
    uint64_t gp_put_implausible;
    uint64_t irq_requests_dropped;
    /* ★★★ #151.  Message-signalled vectors this device actually delivered, and the ones it
     * could not because the guest had not enabled the table.  TWO numbers, because they are
     * two different diagnoses: "we never asked" and "we asked and the guest was not
     * listening" look identical in a boot log otherwise. */
    uint64_t irq_vectors_delivered;
    uint64_t irq_vectors_undeliverable;
    /* ★★★ E2.  How many doorbell arrivals this SHELL printed a timestamped line for.
     *
     * ⊘ Deliberately the shell's own number and not the archive's `doorbells`: the two are
     * counted by different code and a disagreement between them (arrivals that were never
     * logged, beyond the bound) is visible in the report rather than inferred. */
    uint64_t doorbells_logged;
    /* ★★★★ §16.78 — how many REFUSED arrivals got their own timestamped line, counted
     * SEPARATELY from `doorbells_logged`.
     *
     * ⊘ **`w214` is why this field exists, and the defect it fixes is an INSTRUMENT one.**
     * `[measured 2026-08-10, boot `w214_9b65664_ctl`]` that run refused 8 doorbells, every
     * one of them `Route::NotACopyEngineChannel` on a `GrCompute` channel — and **not one
     * of the 8 appears in the log**, because the shared `NVKVM_DOORBELL_LOG_MAX` bound had
     * already been spent on the first 16 arrivals, all of which were CE servings from the
     * device-open phase.  So the boot could say *that* GR doorbells were refused and could
     * never say *when*, and "were they the last thing the guest did before it began to
     * spin?" — the question that decides whether the GR executor is on `cuCtxCreate`'s path
     * at all — had no answer in the evidence.  ⊘ A bound shared between a common event and
     * a rare one is spent by the common one; the rare event is the diagnosis. */
    uint64_t doorbell_refusals_logged;
    /* ★★★ §16.78 — total arrivals SEEN BY THE SHELL, and the heartbeat lines spent on them.
     * Two fields for one fact, on purpose: the modulus needs a monotonic count that the
     * bound never freezes, and the bound needs its own counter. */
    uint64_t doorbell_arrivals;
    uint64_t doorbell_heartbeats_logged;
};

/* ★★ How many doorbell arrivals get a timestamped log line before the device goes quiet.
 *
 * Small, and the smallness is the point: the line is an ATTRIBUTION instrument for an
 * acceptance run that rings a handful of times on purpose, not a trace.  A guest that rings
 * in a loop must be able to move a counter and unable to fill a disk. */
#define NVKVM_DOORBELL_LOG_MAX 16u

/* ★★ The REFUSAL bound, and it is deliberately its OWN budget rather than a larger shared
 * one.  A refusal is the diagnosis; a serving is progress.  Raising the shared bound would
 * buy 8 refusal lines at the cost of hundreds of serving lines nobody asked for, and would
 * change the shape of every committed census that quotes "(16 logged)".  ⊘ Still bounded:
 * a guest that rings a refused channel in a loop moves `doorbells_refused` and cannot fill
 * a disk. */
#define NVKVM_DOORBELL_REFUSAL_LOG_MAX 24u

/* ★★ The heartbeat cap.  64 lines covers ~2048 arrivals at one line per 32; past that the
 * device goes quiet and the counters carry on. */
#define NVKVM_DOORBELL_HEARTBEAT_MAX 64u

/*
 * ═══ ★★★ #151: DELIVERY ════════════════════════════════════════════════════════════════
 *
 * One message-signalled vector, raised.  This is the whole of what "the device can
 * interrupt the guest" means here, and it exists because RmInitAdapter refuses to finish
 * without it: osVerifySystemEnvironment runs a LOOPBACK SELF-TEST — the driver writes
 * CPU_INTR_LEAF_TRIGGER and spins ~4.3 s waiting for its own ISR — and returns
 * NV_ERR_IRQ_NOT_FIRING if nothing arrives (ogkm-580: src/nvidia/src/kernel/os/os_sanity.c
 * :232-249, 292).  Measured as `RmInitAdapter failed! (0x11:0x45:2134)` in
 * /workspace/bench/run_stateload2_dmesg.log:35.
 *
 * ★★ THE LOCK, which is what the previous refusal here was actually about.
 *
 * The old comment said raising from this path "would call the hypervisor's notify path
 * underneath a region this device has taken the global-lock opt-out on".  That opt-out no
 * longer exists — MemoryRegionOps::global_locking was removed, and QEMU 10.2's
 * include/system/memory.h has no such field — so an MMIO write from a vCPU reaches here
 * with the BQL already held.  BQL_LOCK_GUARD() is nonetheless the right thing rather than
 * an assertion, because it is EXACTLY the conditional the concern named: bql_auto_lock()
 * returns NULL and locks nothing when the lock is already ours (include/qemu/main-loop.h
 * :393-401), so this is correct both from a vCPU write and from a future callback that is
 * not one.  ⊘ Not a bare bql_lock(): that would deadlock on the common path.
 *
 * ★ msix_enabled() is checked, and a refusal is COUNTED rather than logged per event.  A
 * guest that has not yet written its own table is not a fault — it is the ordinary state
 * for the microseconds between msix_init and the driver enabling it — but a boot in which
 * every vector fell into that hole would otherwise be indistinguishable from one in which
 * none was ever raised.
 */
static void nvkvm_deliver_vector(NvkvmState *s, unsigned vector)
{
    PCIDevice *pci = PCI_DEVICE(s);

    BQL_LOCK_GUARD();
    if (!msix_enabled(pci) || vector >= s->msix_vectors) {
        s->irq_vectors_undeliverable++;
        return;
    }
    msix_notify(pci, vector);
    s->irq_vectors_delivered++;
}

/*
 * ⊘ ONE VECTOR, and the guest demultiplexes.  The interrupt tree carries the vector number
 * in its own LEAF/TOP pending bits and the guest's ISR reads them to find out what
 * happened (ogkm-580: intr_tu102.c:729-744); the message itself carries no identity that
 * RM consults.  The C artifact makes the same choice at
 * `C: src/qemu/nvkvm_gpu_emul.c:4386-4388` — "single stall vector; ISR demuxes via
 * TOP/LEAF" — and it is the only implementation a real driver has accepted end to end.
 */
#define NVKVM_STALL_VECTOR 0u

/* ===================================================================================
 * Region callbacks
 * =================================================================================== */

/*
 * ═══ ★★★ STAGE Q4: THE REGISTER DISPATCH ═══════════════════════════════════════════════
 *
 * This used to return a constant zero and say so in its own comment.  The archive was
 * already linked into this binary; what was missing was any path that crossed into it, so
 * the emulated GSP had no way for a guest to reach it and a stock driver polled the GSP
 * falcon's HALTED bit until it timed out.
 *
 * ★★ THERE IS NO LOGIC BELOW THIS LINE, and that is the file's own rule (see the header):
 * the routing — which of the chip's read sources owns an offset, what the boot state
 * machine does with a write — is entirely inside the archive.  What is here is the
 * translation of one hypervisor callback into one call, which is the only thing this file
 * is allowed to be.
 *
 * ★ The register plane is deliberately NOT gated on `traps_open`.  That latch belongs to
 * the memory plane's reset dance, where the thing being protected is a reservation being
 * rebuilt; the register plane's reset is a value being rebuilt behind its own lock, and a
 * guest whose registers go silent across a reset learns nothing it can act on.
 */
static uint64_t nvkvm_trap_read(void *opaque, hwaddr addr, unsigned size)
{
    NvkvmState *s = opaque;

    s->trap_reads++;
    return kayfabe_shim_regs_read(s->regs, KAYFABE_BUS_BAR_REGS, (uint64_t)addr, size);
}

static void nvkvm_trap_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    NvkvmState *s = opaque;
    KayfabeRegWrite w;

    s->trap_writes++;
    memset(&w, 0, sizeof(w));
    kayfabe_shim_regs_write(s->regs, KAYFABE_BUS_BAR_REGS, (uint64_t)addr, size, val, &w);

    if (w.fault && w.fault_len) {
        /* ★ Per-message, never fatal: the archive's own rule is that a refused message
         * leaves the register surface answering.  Reported at trace level would be
         * invisible; reported every time would let a polling guest fill the disk — so it is
         * a warning, and the archive counts them for the audit. */
        if (w.fb_why && w.fb_why_len) {
            /* ★★★ #146.  A refused framebuffer write is the one fault whose ONLY other
             * symptom arrives hundreds of operations later, as RmInitAdapter's
             * NV_ERR_MEMORY_ERROR out of kbusVerifyBar2.  Said here, at the instant it
             * happens, with the framebuffer address the moving window resolved to. */
            warn_report("nvkvm: a framebuffer write through the BAR0 moving window at "
                        "+0x%" PRIx64 " DID NOT LAND: %.*s; framebuffer address 0x%" PRIx64
                        " (%" PRIu64 " bytes): %.*s",
                        (uint64_t)addr, (int)w.fault_len, (const char *)w.fault,
                        w.fb_phys, w.fb_refused_len,
                        (int)w.fb_why_len, (const char *)w.fb_why);
        } else if (w.ram_why && w.ram_why_len) {
            /* ★★ A guest-RAM refusal is the one fault with an ADDRESS, and the address is
             * the whole diagnosis: the register offset says which write the guest was
             * making, and this says which of the guest's own pointers we would not
             * follow, and why. */
            warn_report("nvkvm: the emulated GSP refused a register write at +0x%" PRIx64
                        ": %.*s; guest memory at 0x%" PRIx64 " (%" PRIu64 " bytes): %.*s",
                        (uint64_t)addr, (int)w.fault_len, (const char *)w.fault,
                        w.ram_gpa, w.ram_len,
                        (int)w.ram_why_len, (const char *)w.ram_why);
        } else {
            warn_report("nvkvm: the emulated GSP refused a register write at +0x%" PRIx64 ": %.*s",
                        (uint64_t)addr, (int)w.fault_len, (const char *)w.fault);
        }
    }
    /* ★★★ E2 — THE DOORBELL, LOGGED AS IT HAPPENS, and this is the ATTRIBUTION instrument.
     *
     * ⊘ The audit counters at teardown say WHETHER a ring arrived and can never say WHEN, so
     * they cannot answer "did it arrive BECAUSE THE GUEST WROTE ONE?" — a boolean witness
     * cannot attribute, which this project has been bitten by once already (E0's isolate
     * child, sighted 28 seconds before the guest existed and read as the strong claim).
     * This line carries a QEMU -msg timestamp, so a ring can be bracketed by a guest-side
     * command whose own start and end times were recorded by a different writer.
     *
     * ★ BOUNDED, and the bound is why it is safe on a hot path: after
     * NVKVM_DOORBELL_LOG_MAX lines the device stops printing and keeps counting.  A guest
     * that rings in a loop can fill a counter; it cannot fill a disk.  The `s->doorbells_
     * logged` counter is the device's, not the archive's — deliberately, so that "how many
     * did the archive see" and "how many did the shell print" are two numbers and a
     * disagreement between them is visible.
     *
     * ⊘ It is info_report and not warn_report even for a refusal: at E2 a refused doorbell
     * is the EXPECTED answer (no channel has been allocated on this port, and the routing
     * that would find one is increments E4/E5).  A warning would train a reader to ignore
     * the line that matters later. */
    if (w.doorbell != KAYFABE_DOORBELL_NONE) {
        /* ★★★★ §16.78 — EVERY refusal gets a timestamped line, on its own budget, BEFORE
         * the shared bound is consulted.  See `doorbell_refusals_logged`: w214 refused 8
         * and logged 0 of them.  ⊘ This runs first and independently, so a refusal is
         * never hidden by a flood of servings — the exact ordering defect being fixed. */
        if (w.doorbell == KAYFABE_DOORBELL_REFUSED
            && s->doorbell_refusals_logged < NVKVM_DOORBELL_REFUSAL_LOG_MAX) {
            s->doorbell_refusals_logged++;
            info_report("nvkvm: DOORBELL-REFUSED #%" PRIu64 " token 0x%08" PRIx64
                        " at +0x%" PRIx64 " [%.*s]",
                        s->doorbell_refusals_logged, w.doorbell_token, (uint64_t)addr,
                        (int)w.doorbell_kind_len, (const char *)w.doorbell_kind);
        }
        /* ★★★ §16.78 — A HEARTBEAT, so the run has a TIME AXIS beyond its first 16 rings.
         *
         * `w214`'s 16 logged lines all fall inside 21 seconds of a 210-second run, so the
         * evidence cannot say whether the guest went quiet after the refusals or kept
         * ringing — and "did it go quiet?" is what separates *waiting on the refused work*
         * from *waiting on something else*.  One line per 32nd arrival, capped, is a time
         * axis at ~6 lines for a run this size. */
        s->doorbell_arrivals++;
        if ((s->doorbell_arrivals % 32u) == 0
            && s->doorbell_heartbeats_logged < NVKVM_DOORBELL_HEARTBEAT_MAX) {
            s->doorbell_heartbeats_logged++;
            info_report("nvkvm: DOORBELL-HEARTBEAT arrival #%" PRIu64 " token 0x%08" PRIx64,
                        s->doorbell_arrivals, w.doorbell_token);
        }
        if (s->doorbells_logged < NVKVM_DOORBELL_LOG_MAX) {
            s->doorbells_logged++;
            if (w.doorbell == KAYFABE_DOORBELL_REFUSED) {
                info_report("nvkvm: DOORBELL token 0x%08" PRIx64 " at +0x%" PRIx64
                            " REFUSED [%.*s]",
                            w.doorbell_token, (uint64_t)addr,
                            (int)w.doorbell_kind_len, (const char *)w.doorbell_kind);
            } else if (w.doorbell == KAYFABE_DOORBELL_SERVED_LOCAL) {
                /* ★★★ E10e.  The shell's own CPU copy-engine executor did this one; the
                 * per-run detail is in the teardown report, so this line is only the
                 * ATTRIBUTION half — which write, at which instant, and served by whom. */
                info_report("nvkvm: DOORBELL token 0x%08" PRIx64 " at +0x%" PRIx64
                            " SERVED-LOCAL [%.*s]",
                            w.doorbell_token, (uint64_t)addr,
                            (int)w.doorbell_kind_len, (const char *)w.doorbell_kind);
            } else {
                info_report("nvkvm: DOORBELL token 0x%08" PRIx64 " at +0x%" PRIx64 " SERVED",
                            w.doorbell_token, (uint64_t)addr);
            }
        }
    }
    /* ★★★ #151.  The guest's own trigger — delivered. */
    if (w.raise_cpu_intr) {
        nvkvm_deliver_vector(s, NVKVM_STALL_VECTOR);
    }
    if (w.raise_status_irq) {
        s->irq_requests_dropped++;
        if (!s->irq_refusal_reported) {
            s->irq_refusal_reported = true;
            /*
             * ★★ A NAMED REFUSAL, not silence, and the distinction is the whole point.
             *
             * ⚠ #151 CORRECTED THIS ONCE and §16.76 has NARROWED it a second time.  It is
             * narrowed rather than deleted, deliberately: what it says must stay TRUE, and
             * the true statement keeps shrinking as pieces land.
             *
             * What #151 made false: "this device does not deliver vectors yet" — raise_cpu_
             * intr above started delivering them.
             *
             * ⚠ What §16.76 made false: "a guest that BLOCKS on a status-queue event will
             * not be woken by us".  It IS woken now, when it has REGISTERED an os-event: the
             * archive latches MC_ENGINE_IDX_GSP's own stall vector (155 on GA106, read from
             * the captured interrupt table, not written down) into the interrupt tree and
             * asks for delivery through raise_cpu_intr — the same two-step the C does at
             * `C: src/qemu/nvkvm_gpu_emul.c:1828-1843`.
             *
             * ⊘ WHAT IS STILL TRUE, and it is what this now says: this flag is the archive's
             * OWN service cadence, raised on the bind and on every subsequent service pass
             * while anything is queued.  It is NOT demand — nothing is waiting on it — and
             * wiring it to deliver would ship one message per doorbell with no pending bit
             * behind it, sending the guest's ISR looking for an interrupt that is not there.
             * The boot path polls for GSP_INIT_DONE (kgspWaitForRmInitDone) and does not
             * need it; a real WAITER is served by the os-event path instead, and the
             * "os-events" lines in the teardown report are where that is counted.
             */
            warn_report("nvkvm: the emulated GSP asked for its STATUS-QUEUE interrupt and "
                        "this device does not deliver that one. It is the archive's own "
                        "service cadence, not a guest waiting on anything: delivering it "
                        "would send one message per doorbell with nothing pending behind "
                        "it. A guest that BLOCKS on a REGISTERED os-event IS woken — that "
                        "path latches the GSP engine's stall vector and delivers; see the "
                        "\"os-events\" lines in this report for whether it fired.");
        }
    }
}

static const MemoryRegionOps nvkvm_trap_ops = {
    .read       = nvkvm_trap_read,
    .write      = nvkvm_trap_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid      = { .min_access_size = 1, .max_access_size = 8 },
};

/*
 * ═══ ★★★ #149: THE TRANSLATED WINDOW ══════════════════════════════════════════════════
 *
 * The instance/BAR2 window used to be a RESERVATION — a region whose callbacks were not
 * supposed to be reached, backed by nothing, and counted nowhere.  The
 * 2026-08-01 `l2evict1` boot stopped at kbusVerifyBar2_GM107's MMU sub-test, which writes
 * sixteen bytes through this window and reads them back through the BAR0 moving window:
 *
 *     NVRM: kbusVerifyBar2_GM107: MMUTest BAR0 window offset 0x70e000 returned garbage 0x0
 *
 * The write had nowhere to land, so it landed nowhere.  These callbacks are the routing
 * that gives it somewhere, and — as with the register plane — THERE IS NO LOGIC BELOW THIS
 * LINE: the page walk, the root, the refusals and the counters are all inside the archive.
 *
 * ★ The ONLY difference from nvkvm_trap_read/write is the base-address-register index the
 * archive is handed.  It is written as a second pair rather than a shared helper with a
 * parameter because the hypervisor's callback signature carries no index and the opaque is
 * the device — so the index has to come from somewhere, and a literal at one call site is
 * more auditable than an opaque that is sometimes the device and sometimes a row.
 */
static uint64_t nvkvm_bar2_read(void *opaque, hwaddr addr, unsigned size)
{
    NvkvmState *s = opaque;

    s->trap_reads++;
    return kayfabe_shim_regs_read(s->regs, KAYFABE_BUS_BAR_INST, (uint64_t)addr, size);
}

static void nvkvm_bar2_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    NvkvmState *s = opaque;
    KayfabeRegWrite w;

    s->trap_writes++;
    memset(&w, 0, sizeof(w));
    kayfabe_shim_regs_write(s->regs, KAYFABE_BUS_BAR_INST, (uint64_t)addr, size, val, &w);

    if (w.fault && w.fault_len) {
        /*
         * ★★★ A translated write that did not land is the one fault whose ONLY other
         * symptom arrives ninety lines of guest code later, as RmInitAdapter's
         * NV_ERR_MEMORY_ERROR out of kbusVerifyBar2.  Said here, at the instant it happens,
         * with the aperture offset — which for this region IS the virtual address the guest
         * asked the GMMU to translate.
         */
        warn_report("nvkvm: a write through the translated BAR2 window at aperture offset "
                    "+0x%" PRIx64 " DID NOT LAND: %.*s",
                    (uint64_t)addr, (int)w.fault_len, (const char *)w.fault);
    }
}

static const MemoryRegionOps nvkvm_bar2_ops = {
    .read       = nvkvm_bar2_read,
    .write      = nvkvm_bar2_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid      = { .min_access_size = 1, .max_access_size = 8 },
};

/* ★★★★ §16.17 — record ONE BAR1 access in full.  See NvkvmState::bar1_log for why the
 * count was not enough and what the addresses are supposed to decide.
 *
 * ⊘ FIRST accesses, not last: the hypothesis under test is about the GPFIFO entry the guest
 * writes BEFORE it rings, so the earliest accesses are the evidence and a ring buffer that
 * kept the newest would discard exactly them. */
static void nvkvm_bar1_record(NvkvmState *s, uint64_t addr, uint64_t val,
                              unsigned size, bool is_write)
{
    s->bar1_touches++;
    if (s->bar1_log_used >= NVKVM_BAR1_LOG) {
        return;
    }
    s->bar1_log[s->bar1_log_used].addr     = addr;
    s->bar1_log[s->bar1_log_used].val      = val;
    s->bar1_log[s->bar1_log_used].size     = size;
    s->bar1_log[s->bar1_log_used].is_write = is_write;
    s->bar1_log_used++;
}

/*
 * ★★★★ §16.18: THE FRAMEBUFFER APERTURE, AND WHY IT GETS #149'S TREATMENT
 *
 * BAR1 was a RESERVATION whose write callback did `(void)val;`.  MEASURED (boot
 * `s17_e8fde62`, with no shadow installed, so the census was complete): the guest issued
 * exactly THREE accesses to it for the whole boot and we destroyed all three —
 *
 *     BAR1[0] WRITE off=0x90000 size=4 val=0x20000000
 *     BAR1[1] WRITE off=0x90004 size=4 val=0x2801
 *     BAR1[2] WRITE off=0xa008c size=4 val=0x1
 *
 * — which is `internal_channel_submit_work` verbatim (`ogkm-580:
 * kernel-open/nvidia-uvm/uvm_channel.c:984-1015`): a GPFIFO entry written as two dwords
 * through a dereferenced CPU pointer (`gpu_va=0x120000000`, `len=40`), then `GP_PUT = 1` at
 * USERD + 0x8c.  ★ Three accesses read as "the guest barely touched BAR1"; three accesses
 * ARE the entire submission handshake.  A small count is not a small event.
 *
 * ⊘ The old comment here said giving those bytes a home "needs the window's own
 * base/target decoded first".  That framing was wrong in a way worth recording: BAR1 has no
 * base register to decode.  It is GMMU-translated, and its root is the framebuffer address
 * this device itself publishes in GspStaticConfigInfo.bar1PdeBase — see
 * KayfabeRegAudit::bar1_pde_base.  As with BAR0 and BAR2, THERE IS NO LOGIC BELOW THIS
 * LINE: the walk, the root, the refusals and the counters are all inside the archive.
 */
static uint64_t nvkvm_bar1_read(void *opaque, hwaddr addr, unsigned size)
{
    NvkvmState *s = opaque;

    s->trap_reads++;
    nvkvm_bar1_record(s, (uint64_t)addr, 0, size, false);
    return kayfabe_shim_regs_read(s->regs, KAYFABE_BUS_BAR_FB, (uint64_t)addr, size);
}

/* ★★★★★ ITEM 2 / w262 — say it WHEN IT HAPPENS.  See NVKVM_USERD_GP_PUT for why the
 * teardown dump cannot, and for what this line does and does not claim.
 *
 * ⊘ It reads the offset and the value of a write this handler already has in registers, and
 * prints them.  No ring byte is read, no method is decoded, and nothing downstream branches
 * on it — it runs BEFORE kayfabe_shim_regs_write and does not touch `w`. */
static void nvkvm_bar1_gp_put_live(NvkvmState *s, uint64_t addr, uint64_t val, unsigned size)
{
    /* ★★★★★ w279 — CAN this value be a put pointer at all?  See NVKVM_GP_PUT_MAX_ENTRIES.
     * ⊘ Declared before the early return, not beside its first use: this file is built with
     * QEMU's warning set, where a declaration after a statement is an error. */
    bool gp_put_possible;

    if (size != 4 || (addr & 0xfffu) != NVKVM_USERD_GP_PUT) {
        return;
    }
    s->gp_put_writes++;
    gp_put_possible = val < (uint64_t)NVKVM_GP_PUT_MAX_ENTRIES;
    if (!gp_put_possible) {
        s->gp_put_implausible++;
    }
    /* ★★★★★ w262 — PER-PAGE FIRST TOUCH, printed live and uncapped in its count. */
    {
        uint64_t page = addr & ~(uint64_t)0xfff;
        unsigned pi;

        for (pi = 0; pi < s->gp_put_pages_used; pi++) {
            if (s->gp_put_pages[pi].page == page) {
                s->gp_put_pages[pi].writes++;
                break;
            }
        }
        if (pi == s->gp_put_pages_used) {
            if (s->gp_put_pages_used < NVKVM_GP_PUT_PAGES) {
                s->gp_put_pages[pi].page      = page;
                s->gp_put_pages[pi].first_val = val;
                s->gp_put_pages[pi].writes    = 1;
                s->gp_put_pages_used++;
                info_report("nvkvm: BAR1 GP_PUT — FIRST advance on page +0x%" PRIx64
                            " (val=0x%" PRIx64 "), page %u of at most %u.%s ⚠ WHICH channel "
                            "is still not known here: nothing joins a BAR1 offset to a channel.",
                            page, val, s->gp_put_pages_used, NVKVM_GP_PUT_PAGES,
                            gp_put_possible
                            ? " ⊘ This page MAY be one channel's USERD, and this line is the"
                              " instant it first moved — order it against the ENGINE-OBJECT"
                              " births above."
                            : " ⊘⊘ NOT A USERD PAGE: the value cannot index any GPFIFO this"
                              " tree has seen, so this is a 4-byte guest DATA write that"
                              " landed sixteen dwords into some other BAR1-mapped page."
                              " ⇒ EXCLUDE this page from any channel count (w279).");
            } else {
                s->gp_put_pages_dropped++;
            }
        }
    }
    if (s->gp_put_printed >= NVKVM_GP_PUT_LIVE) {
        return;
    }
    s->gp_put_printed++;
    info_report("nvkvm: BAR1 GP_PUT #%" PRIu64 " aperture +0x%" PRIx64 " val=0x%" PRIx64
                " — %s (offset 0x%x). "
                "⊘ WHICH channel is NOT known here: nothing joins a BAR1 offset to a "
                "channel, so this orders the guest's FIRST cursor advance against the host "
                "channel births above, never a particular channel's against its own. "
                "(printed %u of %u; the total is reported at teardown and is not capped)",
                s->gp_put_writes, addr, val,
                gp_put_possible
                ? "the guest MAY have advanced a GPFIFO put pointer in a USERD"
                : "⊘⊘ NOT A PUT POINTER — the value exceeds every GPFIFO size this tree has"
                  " seen, so this is guest DATA at offset 0x8c of a non-USERD page (w279)",
                NVKVM_USERD_GP_PUT,
                s->gp_put_printed, NVKVM_GP_PUT_LIVE);
}

static void nvkvm_bar1_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    NvkvmState *s = opaque;
    KayfabeRegWrite w;

    s->trap_writes++;
    nvkvm_bar1_record(s, (uint64_t)addr, val, size, true);
    nvkvm_bar1_gp_put_live(s, (uint64_t)addr, val, size);
    memset(&w, 0, sizeof(w));
    kayfabe_shim_regs_write(s->regs, KAYFABE_BUS_BAR_FB, (uint64_t)addr, size, val, &w);

    if (w.fault && w.fault_len) {
        /*
         * ★★★ A submission write that did not land has NO other symptom on this device:
         * the guest rings a doorbell for work whose methods were never stored, and what
         * comes back is a channel that never advances.  Said here, at the instant it
         * happens, with the aperture offset — which for this region IS the virtual address
         * the guest asked the GMMU to translate.
         */
        warn_report("nvkvm: a write through the translated BAR1 window at aperture offset "
                    "+0x%" PRIx64 " DID NOT LAND: %.*s",
                    (uint64_t)addr, (int)w.fault_len, (const char *)w.fault);
    }
}

static const MemoryRegionOps nvkvm_bar1_ops = {
    .read       = nvkvm_bar1_read,
    .write      = nvkvm_bar1_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid      = { .min_access_size = 1, .max_access_size = 8 },
};

/* ★★ THE table.  Complete, literal, and the only place a region is named.
 *
 * The hardware indices are 0, 1, 3, 5 and NOT 0, 2, 4: the register aperture is 32-bit (see
 * `NvkvmRegionSpec::bar64`) so it consumes one register, the two windows are 64-bit and
 * consume two each, and the interrupt table lands in the one that is left.  That is a real
 * GA10x's layout and the C artifact's (`C: src/qemu/nvkvm_gpu_emul.c:9804-9831`).
 *
 * ★ The interrupt table's row has no callbacks: it is a container the hypervisor's own
 * message-signalled-interrupt code fills in.  `nvkvm_region_init_io` is not used for it,
 * which is why the constructor counter counts a DIFFERENT number from the row count and
 * `nvkvm_regions_selfcheck` derives the expected value from the table instead of assuming
 * it — an assumption that would have hidden exactly this addition. */
static const NvkvmRegionSpec nvkvm_regions[NVKVM_N_REGIONS] = {
    { "nvkvm-bar0-regs",   0, 0, NVKVM_KIND_TRAP,        false,
      offsetof(NvkvmState, bar0_size), &nvkvm_trap_ops },
    /* ★★★★ §16.18: TRAP, not RESERVATION — for the reason the row below it says, and for
     * one more.  BAR1 is GMMU-translated, so it cannot be shadowed by a flat memslot; and
     * the three writes that ARE the guest's whole submission handshake were reaching a
     * callback that discarded them.  Changing the kind also changes
     * nvkvm_op_bar_is_unbacked_reservation's answer for this register to "no", which is
     * load-bearing: the archive must not install a slot over a range it is trapping. */
    { "nvkvm-bar1-window", 1, 1, NVKVM_KIND_TRAP, true,
      offsetof(NvkvmState, bar1_size), &nvkvm_bar1_ops },
    /* ★★★ #149: TRAP, not RESERVATION.  This window is GMMU-translated — every access to
     * it is a virtual address the archive must walk the guest's own page tables to
     * resolve — so it cannot be shadowed by a memslot the way a flat reservation can.
     * Changing the kind also changes nvkvm_op_bar_is_unbacked_reservation's answer for this
     * register to "no", which is correct and load-bearing: the archive must not install a
     * slot over a range it is trapping. */
    { "nvkvm-bar2-window", 2, 3, NVKVM_KIND_TRAP, true,
      offsetof(NvkvmState, bar2_size), &nvkvm_bar2_ops },
    { "nvkvm-msix",        3, 5, NVKVM_KIND_MSIX,        false,
      offsetof(NvkvmState, msix_size), NULL },
};

static uint64_t nvkvm_row_size(const NvkvmState *s, const NvkvmRegionSpec *row)
{
    return *(const uint64_t *)((const char *)s + row->size_off);
}

/* The one translation from the archive's register name to the hardware's. */
static const NvkvmRegionSpec *nvkvm_row_for_port(uint32_t port_index)
{
    unsigned i;

    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        if (nvkvm_regions[i].port_index == port_index) {
            return &nvkvm_regions[i];
        }
    }
    return NULL;
}

/* ===================================================================================
 * ★★ The ONE region constructor, and the ONE registration loop
 * =================================================================================== */

static void nvkvm_region_init_io(NvkvmState *s, MemoryRegion *mr,
                                 const MemoryRegionOps *ops, const char *name,
                                 uint64_t size)
{
    memory_region_init_io(mr, OBJECT(s), ops, s, name, size);
#if NVKVM_HAVE_LOCKLESS_IO
    memory_region_enable_lockless_io(mr);
#endif
    s->io_inits++;
}

static bool nvkvm_bars_realize(NvkvmState *s, Error **errp)
{
    PCIDevice *pci = PCI_DEVICE(s);
    unsigned i;

    QEMU_BUILD_BUG_ON(ARRAY_SIZE(nvkvm_regions) != NVKVM_N_REGIONS);
    /* ★★★ The two hand-mirrored halves of the publication census, PINNED AT COMPILE TIME.
     *
     * KayfabeRegAudit is written by the Rust archive and read here, and the crate docs say
     * plainly that the `sizeof` handshake does NOT cover this structure — only
     * KAYFABE_SHIM_ABI stands between a field added on one side and a write past the end
     * of this allocation.  That protects against a version SKEW; it does not protect
     * against the two sides declaring the same fields with different padding in the same
     * commit.  crates/kayfabe-qemu-raw/tests/shim_logic.rs pins these exact two numbers on
     * the Rust side; these two lines pin them here, so a layout that drifts is a build
     * failure on the bench rather than a plausible address in a report. */
    QEMU_BUILD_BUG_ON(sizeof(KayfabePdeLevel) != 24);
    QEMU_BUILD_BUG_ON(sizeof(KayfabeGvasPublication) != 200);

    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        const NvkvmRegionSpec *row = &nvkvm_regions[i];
        uint64_t size = nvkvm_row_size(s, row);

        /*
         * ★ Refused, not clamped.  A base-address register's size is a power of two by
         * hardware definition; a request that is not one would be silently rounded by the PCI
         * layer and the archive's region map would then describe a range the guest does not
         * see.  Two descriptions of one range that disagree is the failure this whole device
         * exists to avoid, so it is refused at the only moment an operator can act on it.
         */
        if (size == 0 || (size & (size - 1)) != 0) {
            error_setg(errp,
                       "nvkvm: %s was given size 0x%" PRIx64 ", which is not a power of two; "
                       "a base-address register cannot have that size and rounding it would "
                       "make this device's own map disagree with the guest's",
                       row->name, size);
            return false;
        }
        if (size < 0x1000) {
            error_setg(errp,
                       "nvkvm: %s was given size 0x%" PRIx64 ", which is smaller than a page; "
                       "the archive's memory plane works in whole pages and could never place "
                       "anything in it",
                       row->name, size);
            return false;
        }

        if (row->kind == NVKVM_KIND_MSIX) {
            /* A plain container.  Its contents are the hypervisor's, added by msix_init. */
            memory_region_init(&s->mr[i], OBJECT(s), row->name, size);
        } else {
            nvkvm_region_init_io(s, &s->mr[i], row->ops, row->name, size);
        }
        pci_register_bar(pci, row->pci_bar,
                         PCI_BASE_ADDRESS_SPACE_MEMORY |
                         (row->bar64 ? (PCI_BASE_ADDRESS_MEM_TYPE_64 |
                                        PCI_BASE_ADDRESS_MEM_PREFETCH)
                                     : 0),
                         &s->mr[i]);
    }
    return true;
}

/* How many hardware base-address registers a row consumes. */
static int nvkvm_row_bars(const NvkvmRegionSpec *row)
{
    return row->bar64 ? 2 : 1;
}

/*
 * ★★★ Does this row cross `kayfabe_shim.h`?
 *
 * The archive names the MEMORY plane's registers 0, 1, 2 — densely, and it refuses an index
 * it does not name.  The interrupt table's register is this device's and the hypervisor's;
 * the archive neither backs it nor places anything in it, and handing it over made realize
 * refuse with *"a base-address-register index this port does not name"* — the seam's own
 * check firing correctly on a table that had grown a row the seam does not have a word for.
 *
 * So the table is the enumeration of this device's REGIONS, and this predicate is the
 * enumeration of the subset the archive is told about.  Written as a function rather than a
 * `< 3` because the next row added must be a decision and not an off-by-one.
 */
static bool nvkvm_row_crosses_the_seam(const NvkvmRegionSpec *row)
{
    return row->kind != NVKVM_KIND_MSIX;
}

/* How many rows the archive is told about. */
static unsigned nvkvm_seam_rows(void)
{
    unsigned i, n = 0;

    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        if (nvkvm_row_crosses_the_seam(&nvkvm_regions[i])) {
            n++;
        }
    }
    return n;
}

/* How many rows go through the io constructor — DERIVED from the table, never assumed.
 * See the table's own comment for the addition that made assuming it wrong. */
static unsigned nvkvm_expected_io_inits(void)
{
    unsigned i, n = 0;

    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        if (nvkvm_regions[i].kind != NVKVM_KIND_MSIX) {
            n++;
        }
    }
    return n;
}

/*
 * ★★★ The clause a grep cannot carry.
 *
 * (a) "one call site for the opt-out" and (b) "no hand-rolled unlock around a dispatch" are
 * greps and live in CI.  (c) "EVERY trapped region is marked" is not: a region is missed by
 * omission, and an omission has no token to match.  So it is checked here, three ways, at the
 * only moment the answer is knowable.
 */
static bool nvkvm_regions_selfcheck(NvkvmState *s, Error **errp)
{
    PCIDevice *pci = PCI_DEVICE(s);
    unsigned i;

    if (s->io_inits != nvkvm_expected_io_inits()) {
        error_setg(errp,
                   "nvkvm: the region table has %u rows the io constructor should have built "
                   "but it ran %u times; a region was built somewhere other than "
                   "nvkvm_region_init_io, which is exactly the omission the table exists to "
                   "make impossible",
                   nvkvm_expected_io_inits(), s->io_inits);
        return false;
    }

    /*
     * ★★★ The check that would have caught the aliasing, added because it did not exist and
     * the bug shipped through the gap.  Every row here is registered as a 64-BIT base-address
     * register, so each consumes TWO hardware registers and two rows must be at least two
     * apart.  At 0, 1, 2 the second row's low half lands on the first row's high half, PCI
     * accepts it without complaint, and the device comes up with two registers reporting the
     * same guest-physical base — which the archive then treats as two distinct ranges.
     * Register 0 must also be first, or the archive's dense naming and the hardware's sparse
     * one stop being in the same order.
     */
    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        if (nvkvm_regions[i].port_index != i) {
            error_setg(errp,
                       "nvkvm: the region table's port names are not 0..%u in order; the "
                       "archive addresses registers densely and the table is the only "
                       "translation",
                       NVKVM_N_REGIONS - 1);
            return false;
        }
        /* ★ The rows the archive is told about must be the table's PREFIX, because the
         * archive addresses them densely: a non-crossing row in the middle would make the
         * indices this device sends and the ones it names diverge, silently. */
        if (nvkvm_row_crosses_the_seam(&nvkvm_regions[i]) && i >= nvkvm_seam_rows()) {
            error_setg(errp,
                       "nvkvm: %s crosses the archive seam but sits after a row that does "
                       "not; the archive names its registers densely from zero",
                       nvkvm_regions[i].name);
            return false;
        }
        if (nvkvm_regions[i].pci_bar + nvkvm_row_bars(&nvkvm_regions[i]) > PCI_NUM_REGIONS) {
            error_setg(errp, "nvkvm: %s is registered past the last base-address register",
                       nvkvm_regions[i].name);
            return false;
        }
        /* ★ The spacing rule is now PER ROW, because 64-bit-ness is.  A row that consumes
         * two registers must be two apart from its neighbour; one that consumes one need
         * only be one.  Reading the WIDTH from the table rather than assuming it is the
         * whole reason this check keeps working after the register aperture became 32-bit
         * — a fixed "+2" would have refused the correct layout. */
        if (i > 0 &&
            nvkvm_regions[i].pci_bar <
                nvkvm_regions[i - 1].pci_bar + nvkvm_row_bars(&nvkvm_regions[i - 1])) {
            error_setg(errp,
                       "nvkvm: %s is at base-address register %d and consumes %d, and %s is "
                       "at %d; they overlap. PCI accepts this silently and the device then "
                       "reports two registers at one guest-physical base",
                       nvkvm_regions[i - 1].name, nvkvm_regions[i - 1].pci_bar,
                       nvkvm_row_bars(&nvkvm_regions[i - 1]),
                       nvkvm_regions[i].name, nvkvm_regions[i].pci_bar);
            return false;
        }
    }

    /* ★ NVKVM_MSIX_ROW is a name the vector table is installed through; if it ever pointed
     * at a reservation row, the hypervisor's vector table and the archive's memslots would
     * be handed the same guest-physical range and only one could win. */
    if (nvkvm_regions[NVKVM_MSIX_ROW].kind != NVKVM_KIND_MSIX) {
        error_setg(errp,
                   "nvkvm: NVKVM_MSIX_ROW names row %d, which the table says is a %s and not "
                   "the interrupt table; installing a vector table there would put it over a "
                   "range the archive's own slots claim",
                   NVKVM_MSIX_ROW, nvkvm_regions[NVKVM_MSIX_ROW].name);
        return false;
    }

    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        const NvkvmRegionSpec *row = &nvkvm_regions[i];

        if (pci->io_regions[row->pci_bar].memory != &s->mr[i]) {
            error_setg(errp,
                       "nvkvm: %s is not the region registered at base-address register %d; "
                       "the table and the registration loop disagree",
                       row->name, row->pci_bar);
            return false;
        }
        if (memory_region_size(&s->mr[i]) != nvkvm_row_size(s, row)) {
            error_setg(errp,
                       "nvkvm: %s was constructed at a size the table does not name",
                       row->name);
            return false;
        }
#if NVKVM_HAVE_LOCKLESS_IO
        if (row->kind != NVKVM_KIND_MSIX && !s->mr[i].lockless_io) {
            error_setg(errp,
                       "nvkvm: %s did not get the global-lock opt-out; a device with one "
                       "marked region and one unmarked one keeps BOTH hazards on the "
                       "unmarked one while passing any per-device check",
                       row->name);
            return false;
        }
#endif
    }
    return true;
}

static bool nvkvm_is_ours(const NvkvmState *s, const MemoryRegion *mr)
{
    unsigned i;

    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        if (&s->mr[i] == mr) {
            return true;
        }
    }
    return false;
}

/* ===================================================================================
 * The primitives handed to the archive
 * =================================================================================== */

/*
 * ★ For an in-tree device these ARE the binary's version: there is no supported out-of-tree
 * mechanism, so the shim is compiled into the binary it runs in and the header/binary skew
 * the two-floor rule imagines cannot occur here.  Kept as a primitive anyway, because the
 * archive must not learn the answer at ITS compile time — a second hypervisor's shim answers
 * differently, and that is the whole point of the table.
 */
static uint32_t nvkvm_op_version_major(void *dev)
{
    (void)dev;
    return QEMU_VERSION_MAJOR;
}

static uint32_t nvkvm_op_version_minor(void *dev)
{
    (void)dev;
    return QEMU_VERSION_MINOR;
}

static int32_t nvkvm_op_kvm_enabled(void *dev)
{
    (void)dev;
    return kvm_enabled() ? 1 : 0;
}

static int32_t nvkvm_op_migrate_add_blocker(void *dev, const uint8_t *reason,
                                            uint64_t reason_len, uint64_t *out_id)
{
    NvkvmState *s = dev;
    g_autofree char *txt = g_strndup((const char *)reason, (gsize)reason_len);

    if (s->migrate_blocker) {
        return KAYFABE_E_REFUSED;
    }
    error_setg(&s->migrate_blocker, "nvkvm: %s", txt);
    if (migrate_add_blocker(&s->migrate_blocker, NULL) < 0) {
        /* the hypervisor frees and clears *reasonp on failure */
        s->migrate_blocker = NULL;
        return KAYFABE_E_REFUSED;
    }
    *out_id = 1;
    return KAYFABE_OK;
}

static void nvkvm_op_migrate_del_blocker(void *dev, uint64_t id)
{
    NvkvmState *s = dev;

    (void)id;
    if (s->migrate_blocker) {
        migrate_del_blocker(&s->migrate_blocker);   /* frees and clears */
    }
}

static int32_t nvkvm_op_ram_block_discard_disable(void *dev, int32_t disable)
{
    NvkvmState *s = dev;
    int rc = ram_block_discard_disable(disable != 0);

    if (rc == 0) {
        s->discard_disabled = (disable != 0);
        return KAYFABE_OK;
    }
    /*
     * ★ -EBUSY is reported as its own class, not as an errno, because it is the one refusal
     * an operator can fix from the command line: a device that REQUIRES guest-driven discard
     * is already in this machine.  Flattening it into the generic arm would tell them
     * "refused" and leave them to bisect their own invocation.
     */
    if (rc == -EBUSY) {
        return KAYFABE_E_BUSY;
    }
    return rc < 0 ? rc : KAYFABE_E_REFUSED;
}

/*
 * ★★ Deferred on purpose, and this is an ORDERING fact, not a shortcut.
 *
 * `memory_listener_register` replays the whole existing topology through region_add before it
 * returns.  The archive calls this primitive from INSIDE its realize, so at this instant it
 * has no handle to give those callbacks and every replayed section would be dropped —
 * silently, because a dropped section looks exactly like a machine with no memory in it.  So
 * the primitive records the request and the caller registers for real the moment realize
 * returns a handle, which is a few microseconds later and still inside the same
 * configuration-space write.
 */
static int32_t nvkvm_op_register_listener(void *dev)
{
    NvkvmState *s = dev;

    s->want_listener = true;
    return KAYFABE_OK;
}

static int32_t nvkvm_op_bar_is_unbacked_reservation(void *dev, uint32_t bar)
{
    const NvkvmRegionSpec *row = nvkvm_row_for_port(bar);

    (void)dev;
    /*
     * ★★★ Truthful, not optimistic.  The answer is a property of HOW the region was
     * constructed, and both kinds here are built with the pure-MMIO constructor — so both are
     * in fact unbacked.  The row's kind is what the table INTENDS, and only a reservation row
     * answers yes, so a future edit that RAM-backs a row and forgets this function still gets
     * a "no" and the archive refuses to put a reservation there.  A register this device does
     * not own answers "no" for the same reason: erring toward "the hypervisor backs it" is
     * the safe direction, because the other one puts two slots over one range.
     */
    if (!row) {
        return 0;
    }
    return row->kind == NVKVM_KIND_RESERVATION ? 1 : 0;
}

static int32_t nvkvm_op_bar_base(void *dev, uint32_t bar, uint64_t *out_base)
{
    NvkvmState *s = dev;
    PCIDevice *pci = PCI_DEVICE(s);
    const NvkvmRegionSpec *row = nvkvm_row_for_port(bar);
    pcibus_t addr;

    /* ★ The archive names registers 0, 1, 2; the hardware numbers them 0, 2, 4 because these
     * are 64-bit registers.  The table is the only translation, and a name it does not carry
     * is refused rather than passed through as a hardware index — which is how the two got
     * conflated in the first place. */
    if (!row) {
        return KAYFABE_E_UNSUPPORTED;
    }
    /* One read of this device's own PCI bookkeeping.  No lock, no lookup, no cache. */
    addr = pci->io_regions[row->pci_bar].addr;
    if (addr == PCI_BAR_UNMAPPED) {
        return KAYFABE_E_UNSUPPORTED;
    }
    *out_base = (uint64_t)addr;
    return KAYFABE_OK;
}

static int32_t nvkvm_op_ref_region(void *dev, uint64_t mr)
{
    (void)dev;
    if (mr == 0) {
        return KAYFABE_E_REFUSED;
    }
    memory_region_ref((MemoryRegion *)(uintptr_t)mr);
    return KAYFABE_OK;
}

static void nvkvm_op_unref_region(void *dev, uint64_t mr)
{
    (void)dev;
    if (mr != 0) {
        memory_region_unref((MemoryRegion *)(uintptr_t)mr);
    }
}

/*
 * ★★★ A bounded copy against THIS region's own backing.  Spelling it as `address_space_read`
 * would take the hypervisor's global lock whenever the target is not direct-access,
 * underneath one of the archive's ranked locks.  That inversion is invisible to every gate in
 * the tree, which is why the obligation is written on the primitive.
 */
static int32_t nvkvm_op_read_region(void *dev, uint64_t mr_u, uint64_t off,
                                    uint8_t *dst, uint64_t len)
{
    MemoryRegion *mr = (MemoryRegion *)(uintptr_t)mr_u;
    uint64_t size;
    const uint8_t *base;

    (void)dev;
    if (!mr || !memory_region_is_ram(mr) || memory_region_is_ram_device(mr)) {
        return KAYFABE_E_REFUSED;
    }
    size = memory_region_size(mr);
    if (off > size || len > size - off) {
        return KAYFABE_E_REFUSED;
    }
    base = memory_region_get_ram_ptr(mr);
    if (!base) {
        return KAYFABE_E_REFUSED;
    }
    memcpy(dst, base + off, (size_t)len);
    return KAYFABE_OK;
}

static int32_t nvkvm_op_write_region(void *dev, uint64_t mr_u, uint64_t off,
                                     const uint8_t *src, uint64_t len)
{
    MemoryRegion *mr = (MemoryRegion *)(uintptr_t)mr_u;
    uint64_t size;
    uint8_t *base;

    (void)dev;
    /* ★ The write direction additionally refuses a read-only or rom-device region: a memcpy
     * into one bypasses the owning device's own write path, which is a side effect on
     * hardware and not merely a bad byte. */
    if (!mr || !memory_region_is_ram(mr) || memory_region_is_ram_device(mr) ||
        mr->rom_device || mr->readonly) {
        return KAYFABE_E_REFUSED;
    }
    size = memory_region_size(mr);
    if (off > size || len > size - off) {
        return KAYFABE_E_REFUSED;
    }
    base = memory_region_get_ram_ptr(mr);
    if (!base) {
        return KAYFABE_E_REFUSED;
    }
    memcpy(base + off, src, (size_t)len);
    memory_region_set_dirty(mr, off, len);
    return KAYFABE_OK;
}

/*
 * ★★★ #151.  The memory plane's own interrupt seam, wired to the same one delivery point.
 *
 * ⚠ This op and the register plane's raise_cpu_intr flag are TWO WIRES into one action, and
 * they stay two: this one is called by kayfabe_fwd's completion path (IrqSpec::Msix(0)),
 * which has nothing to do with the guest writing a trigger register.  Sharing
 * nvkvm_deliver_vector means they cannot disagree about the lock or about what "enabled"
 * means; sharing a FLAG would have meant losing which of them fired.
 *
 * ⊘ KAYFABE_E_REFUSED, not OK, when the guest has not enabled its table: the caller is a
 * completion notifier, and telling it a vector went out when none did is precisely the
 * "success without raising anything" this used to refuse to be.
 */
static int32_t nvkvm_op_signal_msix(void *dev, uint16_t vector)
{
    NvkvmState *s = (NvkvmState *)dev;
    uint64_t before = s->irq_vectors_delivered;

    nvkvm_deliver_vector(s, vector);
    return s->irq_vectors_delivered != before ? KAYFABE_OK : KAYFABE_E_REFUSED;
}

static const KayfabeHostOps nvkvm_host_ops = {
    .abi_version                 = KAYFABE_SHIM_ABI,
    .struct_size                 = (uint32_t)sizeof(KayfabeHostOps),
    .version_major               = nvkvm_op_version_major,
    .version_minor               = nvkvm_op_version_minor,
    .kvm_enabled                 = nvkvm_op_kvm_enabled,
    .migrate_add_blocker         = nvkvm_op_migrate_add_blocker,
    .migrate_del_blocker         = nvkvm_op_migrate_del_blocker,
    .ram_block_discard_disable   = nvkvm_op_ram_block_discard_disable,
    .register_listener           = nvkvm_op_register_listener,
    .bar_is_unbacked_reservation = nvkvm_op_bar_is_unbacked_reservation,
    .bar_base                    = nvkvm_op_bar_base,
    .ref_region                  = nvkvm_op_ref_region,
    .unref_region                = nvkvm_op_unref_region,
    .read_region                 = nvkvm_op_read_region,
    .write_region                = nvkvm_op_write_region,
    .signal_msix                 = nvkvm_op_signal_msix,
};

/* ===================================================================================
 * The topology listener
 * =================================================================================== */

static void nvkvm_listener_region_add(MemoryListener *l, MemoryRegionSection *sec)
{
    NvkvmState *s = container_of(l, NvkvmState, listener);
    KayfabeSection w;
    const uint8_t *msg = NULL;
    uint64_t msg_len = 0;
    int32_t rc;

    if (!s->shim || nvkvm_is_ours(s, sec->mr)) {
        return;
    }
    /* A section wider than 64 bits cannot be described; skipped rather than truncated,
     * because a truncated length would be a map that quietly disagrees with reality. */
    if (int128_gethi(sec->size) != 0) {
        return;
    }

    w.mr                   = (uint64_t)(uintptr_t)sec->mr;
    w.gpa                  = (uint64_t)sec->offset_within_address_space;
    w.len                  = int128_get64(sec->size);
    w.offset_within_region = (uint64_t)sec->offset_within_region;
    /* ★★ Five facts, reported unclassified.  The rule that turns them into a verdict lives in
     * one place and it is not this file — including `mr->rom_device`, which is read as a field
     * because no public predicate answers it. */
    w.is_ram        = memory_region_is_ram(sec->mr);
    w.is_ram_device = memory_region_is_ram_device(sec->mr);
    w.is_rom_device = sec->mr->rom_device;
    w.readonly      = sec->readonly;
    w.nonvolatile   = sec->nonvolatile;
    /* ★★★ THE LAYOUT, STATED.  A census of this process's descriptors yields the SIZE of
     * guest RAM and nothing about its shape; the mapping from guest-physical address to
     * byte-of-the-file is a fact only the hypervisor holds, and this is where it says it.
     *
     * ⊘ Deliberately NOT derived from the machine type.  On `-m 2048` q35 the map is the
     * identity and `traces/guest_boots/run_w224m_mtree.log` measured that -- for ONE command
     * line.  With `-m 8G` RAM splits around the 4 GiB PCI hole and the identity stops
     * holding, so a consumer that assumed it would be silently wrong on a boot nobody
     * thought was unusual.
     *
     * `mr` cannot carry this: it is this process's pointer to a region object.  The consumer
     * has to join against a DESCRIPTOR it holds, so the identity reported is the block's --
     * `(st_dev, st_ino)` -- exactly the key the descriptor census was forced onto when fd
     * numbers turned out to move between two physical benches.
     *
     * `fd_offset` is read as a field for the same reason `rom_device` above is: there is no
     * public accessor, and the alternative is to assume it is zero.  It IS zero for every
     * `memory-backend-memfd` this bench has booted -- which is precisely why an assumption
     * would never be caught. */
    w.fd_backed             = 0;
    w.backing_dev           = 0;
    w.backing_ino           = 0;
    w.file_offset_of_region = 0;
    if (memory_region_is_ram(sec->mr) && sec->mr->ram_block) {
        int fd = memory_region_get_fd(sec->mr);
        struct stat st;

        if (fd >= 0 && fstat(fd, &st) == 0) {
            w.fd_backed             = 1;
            w.backing_dev           = (uint64_t)st.st_dev;
            w.backing_ino           = (uint64_t)st.st_ino;
            w.file_offset_of_region = sec->mr->ram_block->fd_offset;
        }
    }

    rc = kayfabe_shim_region_add(s->shim, &w, &msg, &msg_len);
    if (rc != KAYFABE_OK) {
        warn_report("nvkvm: a topology section at 0x%" PRIx64 " was not taken: %.*s",
                    w.gpa, (int)msg_len, (const char *)msg);
    }
}

static void nvkvm_listener_region_del(MemoryListener *l, MemoryRegionSection *sec)
{
    NvkvmState *s = container_of(l, NvkvmState, listener);

    if (!s->shim || nvkvm_is_ours(s, sec->mr)) {
        return;
    }
    if (int128_gethi(sec->size) != 0) {
        return;
    }
    kayfabe_shim_region_del(s->shim,
                            (uint64_t)sec->offset_within_address_space,
                            int128_get64(sec->size));
}

/* ===================================================================================
 * Realizing the archive, once the registers have bases
 * =================================================================================== */


static bool nvkvm_bars_all_mapped(NvkvmState *s)
{
    PCIDevice *pci = PCI_DEVICE(s);
    unsigned i;

    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        if (!nvkvm_row_crosses_the_seam(&nvkvm_regions[i])) {
            continue;
        }
        if (pci->io_regions[nvkvm_regions[i].pci_bar].addr == PCI_BAR_UNMAPPED) {
            return false;
        }
    }
    return true;
}

static void nvkvm_shim_realize(NvkvmState *s)
{
    PCIDevice *pci = PCI_DEVICE(s);
    KayfabeBarCfg bars[NVKVM_N_REGIONS];
    KayfabeRealizeCfg cfg;
    const uint8_t *msg = NULL;
    uint64_t msg_len = 0;
    void *handle = NULL;
    unsigned i, n;
    int32_t rc;

    for (i = 0, n = 0; i < NVKVM_N_REGIONS; i++) {
        if (!nvkvm_row_crosses_the_seam(&nvkvm_regions[i])) {
            continue;
        }
        bars[n].index    = nvkvm_regions[i].port_index;
        bars[n].reserved = 0;
        bars[n].base     = (uint64_t)pci->io_regions[nvkvm_regions[i].pci_bar].addr;
        bars[n].len      = nvkvm_row_size(s, &nvkvm_regions[i]);
        n++;
    }
    cfg.abi_version   = KAYFABE_SHIM_ABI;
    cfg.struct_size   = (uint32_t)sizeof(cfg);
    cfg.shareable_ram = s->shareable_ram ? 1 : 0;
    cfg.n_bars        = n;
    cfg.bars          = bars;

    rc = kayfabe_shim_realize(&nvkvm_host_ops, s, &cfg, &handle, &msg, &msg_len);
    if (rc != KAYFABE_OK) {
        /*
         * ★ Loud, once, and then dead.  The device cannot fail `realize` here — realize
         * finished long ago, at machine build time — so the loudest thing available is a
         * report naming the refusal verbatim and a device that never pretends to work.  A
         * refusal retried on every configuration-space write would bury itself.
         */
        s->shim_refused = true;
        error_report("nvkvm: the memory plane refused to realize (%d): %.*s",
                     (int)rc, (int)msg_len, (const char *)msg);
        return;
    }
    s->shim = handle;

    /* See nvkvm_op_register_listener for why this happens HERE and not there. */
    if (s->want_listener && !s->listening) {
        s->listener = (MemoryListener){
            .name       = "nvkvm",
            .region_add = nvkvm_listener_region_add,
            .region_del = nvkvm_listener_region_del,
        };
        memory_listener_register(&s->listener, pci_get_address_space(pci));
        s->listening = true;
    }

    /*
     * ⊘⊘ §16.18 — AND IT MUST NOT BE INSTALLED OVER A TRAPPING ROW.  A shadow memslot is
     * plain guest RAM with no connection to the framebuffer store; while BAR1 was a
     * RESERVATION that was the whole point, but BAR1 now TRAPS and is GMMU-translated, so a
     * shadow would serve the guest's submission writes out of memory the copy engine and
     * the page walk can never see — and a read back through the same slot would AGREE.
     * That is the self-consistent-wrong-store defect exactly, and read-after-write cannot
     * detect it.  Refused loudly rather than left as a property nobody would think to check.
     */
    if (s->window_size != 0 && nvkvm_regions[1].kind == NVKVM_KIND_TRAP) {
        error_report("nvkvm: the window-size property asks for a shadow memslot over "
                     "'%s', which is a TRAPPING, GMMU-translated row since §16.18. A slot "
                     "there would answer the guest out of memory the framebuffer store and "
                     "the page walk cannot see, and a read back would agree with it. "
                     "REFUSED; no shadow installed.",
                     nvkvm_regions[1].name);
    } else if (s->window_size != 0) {
        uint64_t base = (uint64_t)pci->io_regions[nvkvm_regions[1].pci_bar].addr;

        rc = kayfabe_shim_install_window(s->shim, base, s->window_size, &msg, &msg_len);
        if (rc != KAYFABE_OK) {
            error_report("nvkvm: the realize-time reservation was refused (%d): %.*s",
                         (int)rc, (int)msg_len, (const char *)msg);
        } else {
            KayfabeAudit a;

            /*
             * ★ The counters, not the return code.  "The call said OK" is a claim about the
             * seam; `live_memslots` is a claim about the KERNEL, and only the second one
             * distinguishes a reservation that was installed from a reservation that was
             * accounted for.  `regions_published` is printed beside it because it must be
             * zero forever — it is the whole memory-plane decision as one number, and a
             * message that carried only the good news would never show it moving.
             */
            s->window_installed = true;
            if (kayfabe_shim_audit(s->shim, &a) == KAYFABE_OK) {
                info_report("nvkvm: reservation of 0x%" PRIx64 " bytes installed at 0x%" PRIx64
                            " (kernel slots live=%" PRIu64 " installs=%" PRIu64
                            ", regions the hypervisor backs=%" PRIu64 ")",
                            s->window_size, base, a.live_memslots, a.memslot_installs,
                            a.regions_published);
            } else {
                info_report("nvkvm: reservation of 0x%" PRIx64 " bytes installed at 0x%" PRIx64,
                            s->window_size, base);
            }
        }
    }

    /*
     * ★★★ STAGE Q5 — the two planes are joined HERE, and this is the only place that can
     * do it.  The register plane was created at `realize`, when the base-address registers
     * were still unprogrammed; the memory plane could not exist until one had a base.  So
     * this instant — the memory plane having just realized, the register plane having been
     * answering registers for a while already — is the first moment both exist.
     *
     * ★ Refusal is a warning, not fatal, for the same reason the memory plane's own refusal
     * is: the register surface keeps answering either way, and a guest whose registers go
     * silent learns nothing it can act on.  What it must never be is quiet — without the
     * port, the emulated GSP refuses one specific write thousands of accesses into a boot
     * that otherwise looks completely healthy, and that is a whole debugging session.
     */
    if (s->regs) {
        rc = kayfabe_shim_regs_attach_ram(s->regs, s->shim);
        if (rc != KAYFABE_OK) {
            error_report("nvkvm: the register plane could not be given guest memory (%d); "
                         "the emulated GSP will refuse every guest-memory access by name",
                         (int)rc);
        } else {
            s->regs_have_ram = true;
        }
    }

    info_report("nvkvm: memory plane realized (bar0=0x%" PRIx64 " bar1=0x%" PRIx64
                " bar2=0x%" PRIx64 ", register plane has guest memory=%s)",
                bars[0].base, bars[1].base, bars[2].base,
                s->regs_have_ram ? "yes" : "NO");
}

static void nvkvm_after_bar_update(NvkvmState *s)
{
    PCIDevice *pci = PCI_DEVICE(s);
    unsigned i;

    if (!s->shim) {
        if (!s->shim_refused && nvkvm_bars_all_mapped(s)) {
            nvkvm_shim_realize(s);
        }
        return;
    }
    /* The detector.  The preventer already ran, in nvkvm_config_write. */
    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        pcibus_t addr;

        if (!nvkvm_row_crosses_the_seam(&nvkvm_regions[i])) {
            continue;
        }
        addr = pci->io_regions[nvkvm_regions[i].pci_bar].addr;

        kayfabe_shim_note_bar_mapping(s->shim, nvkvm_regions[i].port_index,
                                      addr == PCI_BAR_UNMAPPED ? 0 : 1,
                                      addr == PCI_BAR_UNMAPPED ? 0 : (uint64_t)addr);
    }
}

/*
 * ★★ The preventer and the detector are BOTH here, and they close different halves.  A device
 * that refuses the move never reaches the inconsistent state; a device that only detects it
 * finds out afterwards.  The C artifact has neither, which is the latent bug this port exists
 * to subtract: it caches the base once and has no hook of any kind, so a guest that moves the
 * register leaves the hypervisor's region following and the slots behind — the old range keeps
 * working AND becomes reassignable, the new one reads zeros, silently.
 */
static void nvkvm_config_write(PCIDevice *pci, uint32_t addr, uint32_t val, int len)
{
    NvkvmState *s = NVKVM(pci);
    unsigned i;

    if (s->shim && ranges_overlap(addr, len, PCI_BASE_ADDRESS_0,
                                  PCI_BASE_ADDRESS_5 + 4 - PCI_BASE_ADDRESS_0)) {
        for (i = 0; i < NVKVM_N_REGIONS; i++) {
            const uint8_t *msg = NULL;
            uint64_t msg_len = 0;

            if (!nvkvm_row_crosses_the_seam(&nvkvm_regions[i])) {
                continue;
            }
            if (kayfabe_shim_bar_move_requested(s->shim, nvkvm_regions[i].port_index,
                                                &msg, &msg_len) != KAYFABE_OK) {
                warn_report("nvkvm: a base-address-register write was refused: %.*s",
                            (int)msg_len, (const char *)msg);
                return;
            }
        }
    }

    pci_default_write_config(pci, addr, val, len);
    nvkvm_after_bar_update(s);
}

/* ===================================================================================
 * Lifecycle
 * =================================================================================== */

static void nvkvm_report_registers(NvkvmState *s)
{
    KayfabeRegAudit a;
    /* ★ §16.16's trap-status table needs this device's own PCI bookkeeping and a loop
     * index. Declared here because this file is built with QEMU's C dialect settings and
     * the rest of the function already declares at the top.
     * ⊘ `ri`, not `i`: THIS function already declares `i` inside five later blocks, and a
     * function-scope `i` shadowed every one of them (-Wshadow=local x6, caught by the
     * bench's own first compile of §16.16). A shadowed loop counter is how an edit to one
     * census block silently starts driving another. */
    PCIDevice *pci = PCI_DEVICE(s);
    unsigned ri;

    /*
     * ★★ `unclaimed_reads` is the number to read.
     *
     * It counts every register this device answered with a DEFAULTED ZERO because no model
     * owns the offset.  That is not an error today and the C artifact does the same — but it
     * is the difference between "the guest booted" and "the guest booted on answers nobody
     * wrote", and without a number nobody can tell which.
     *
     * ★ Called from BOTH device teardown and process exit.  Teardown alone is not enough: a
     * plain machine shutdown never unplugs the device, so `exit` is never reached and the
     * counters were unobservable in exactly the case an operator cares about.  Measured on
     * the first run of this device — the shutdown printed nothing.
     */
    if (s->audit_printed || !s->regs) {
        return;
    }
    s->audit_printed = true;
    if (kayfabe_shim_regs_audit(s->regs, &a) != KAYFABE_OK) {
        return;
    }
    info_report("nvkvm: registers: %" PRIu64 " reads / %" PRIu64 " writes "
                "(chip-constant %" PRIu64 ", rom %" PRIu64 ", gsp %" PRIu64 "r/%" PRIu64 "w, "
                "UNCLAIMED %" PRIu64 "r/%" PRIu64 "w), faults %" PRIu64 ", "
                "guest-RAM refusals %" PRIu64 ", interrupt requests dropped %" PRIu64,
                a.reads, a.writes, a.boot_reg_reads, a.rom_reads, a.gsp_reads,
                a.gsp_writes, a.unclaimed_reads, a.unclaimed_writes,
                a.faults, a.ram_refusals, s->irq_requests_dropped);

    /* ★★★ #128 — THE COUNTER, printed unconditionally for the reason the interrupt line
     * below is.  `ptimer_reads` at zero is a diagnosis all by itself: a guest that never
     * read the free-running counter never reached a driver timeout loop, which is a much
     * earlier failure than whatever else the log is showing.  And `refused-writes` counts
     * the guest's own `tmrSetCurrentTime` being told no — a DECISION this device makes
     * (`kayfabe_device::plane::PTIMER_WRITE_REFUSED`), not a drop, so it must be visible
     * rather than inferable from a gap in the unclaimed count. */
    info_report("nvkvm: timer: %" PRIu64 " counter reads, %" PRIu64 " writes REFUSED "
                "(the guest reads the host GPU's counter and may not move it)",
                a.ptimer_reads, a.ptimer_writes_refused);

    /* ★★★ #151 — THE INTERRUPT LINE, printed unconditionally including when every number is
     * zero, for the reason the framebuffer line below is: a boot that stops at
     * NV_ERR_IRQ_NOT_FIRING and a boot that never reached the self-test are the same silence
     * otherwise.  `delivered` is the one to read: the driver's own loopback test triggers
     * EXACTLY ONCE (measured, the oracle's cap1: one IrqRaise in 359 062 records), so 1 is
     * the healthy value and 0 is the whole diagnosis. */
    info_report("nvkvm: interrupts: %" PRIu64 " vectors delivered, %" PRIu64 " undeliverable "
                "(guest had not enabled the table), %" PRIu64 " status-queue requests dropped",
                s->irq_vectors_delivered, s->irq_vectors_undeliverable,
                s->irq_requests_dropped);

    /* ★★★★★ §16.76.9 — THE INTERRUPT TREE'S OWN NUMBERS, and this line exists to settle ONE
     * ambiguity that `w211` could not.
     *
     * That boot reported `1 would be masked` for the os-event announcement AND `4 of 4` for
     * the CE completions — i.e. EVERY vector this device has ever latched read as masked.
     * Two hypotheses with opposite fixes: (a) the guest genuinely never enables these leaves
     * on a GSP-offload adapter, or (b) our own `leaf_en` bookkeeping never sees the guest's
     * enable writes (the same boot logged 2464 UNCLAIMED register writes).
     *
     * ⊘ `cpu_intr_masked` is the DISCRIMINATOR and it was already on the wire, unprinted.
     * It counts the guest's OWN `LEAF_TRIGGER` writes — overwhelmingly the driver's
     * `_osVerifyInterrupts` loopback on vector 129, which we KNOW succeeded (no
     * NV_ERR_IRQ_NOT_FIRING, and the adapter came up).  `_osVerifyInterrupts` writes
     * `LEAF_EN_SET` immediately before it triggers (ogkm-580: intr_swintr_tu102.c:72-90), so:
     *
     *   masked == 0 while raises > 0  ->  the bookkeeping WORKS; hypothesis (a).
     *   masked == raises              ->  the bookkeeping is BLIND; hypothesis (b), and the
     *                                     "would be masked" numbers above mean nothing.
     *
     * ★ `suspect_the_instrument_first`: this device raises unconditionally by standing
     * decision (kayfabe_device::cpuintr), so a blind enable-tracker costs NOTHING at run time
     * and corrupts EVERY masked reading.  A number that is only ever read as a diagnosis must
     * be able to say that it is the one that is broken. */
    info_report("nvkvm: interrupt tree: %" PRIu64 " register accesses, %" PRIu64
                " guest LEAF_TRIGGER raises, %" PRIu64 " of them would be masked "
                "(⊘ if that equals the raises, the ENABLE BOOKKEEPING is blind and every "
                "\"would be masked\" number in this report is meaningless — the loopback "
                "self-test enables its leaf immediately before triggering)",
                a.cpu_intr_accesses, a.cpu_intr_raises, a.cpu_intr_masked);

    /* ★★★ §14.18 — THE COMPLETION-NOTIFICATION LINE, printed unconditionally for the same
     * reason as the one above.  Serving notifier index 35 is a promise to raise a non-stall
     * vector when the engine's work completes; this line is whether the promise was kept.
     *
     * ⊘ `unvectored` is the one to read and its healthy value is ZERO: each one is a copy
     * this shell really performed and never announced.  `masked` is the second half of a
     * hang diagnosis — the message went out, but the guest's own LEAF_EN would hide the
     * vector from its non-stall scan, which without this number looks exactly like never
     * having raised at all. */
    info_report("nvkvm: completions: %" PRIu64 " announced (non-stall vector raised), %" PRIu64
                " UNVECTORED (work done, nothing told the guest), %" PRIu64
                " would be masked by the guest's own LEAF_EN",
                a.nonstall_raises, a.nonstall_unvectored, a.nonstall_masked);

    /* ★★★★★ §16.76 — THE OS-EVENT WAKEUP LINE, and it answers a DIFFERENT question from the
     * completion line above.  That one is "a copy engine finished"; this one is "a userspace
     * waiter blocked in poll() was told its registered event fired", which is what
     * cuCtxCreate is actually parked on.
     *
     * Printed unconditionally, including all-zero, for every other block's reason: zero
     * registrations and a registry that never got seated are the same silence otherwise.
     *
     * ⊘ THE TWO NUMBERS TO READ, IN THIS ORDER:
     *   `cleared` — the guest's IRQSCLR.  It is the ONLY thing that reopens the flow-control
     *     gate.  0 with raises > 0 means the gate latched shut after one batch and delivery
     *     has stopped silently.  The oracle cannot see this: cap1 has ZERO IRQSCLR writes.
     *   `woke-with-nothing` — batches announced with no newly-served doorbell behind them.
     *     A wakeup is not a completion; on the passthrough data plane the HOST GPU DMAs the
     *     release semaphore into guest RAM, so a batch with nothing executed wakes libcuda
     *     into an unchanged semaphore and it blocks again.  Non-zero names the next rung
     *     (get the guest's channel forwarded and executed), never a payload to fabricate. */
    info_report("nvkvm: os-events: %" PRIu64 " registered / %" PRIu64 " retired / %" PRIu64
                " live (%" PRIu64 " malformed, %" PRIu64 " refused-full); %" PRIu64
                " POST_EVENT posted in %" PRIu64 " batch(es); gate: %" PRIu64 " gated, %"
                PRIu64 " not-running, %" PRIu64 " failed, %" PRIu64 " IRQSCLR cleared",
                a.os_events_registered, a.os_events_retired, a.os_events_live,
                a.os_events_malformed, a.os_events_overflowed, a.os_event_posted,
                a.os_event_batches, a.os_event_gated, a.os_event_not_running,
                a.os_event_failed, a.status_irq_cleared);
    info_report("nvkvm: os-event announce: %" PRIu64 " GSP stall vector(s) raised, %" PRIu64
                " UNVECTORED, %" PRIu64 " would be masked; %" PRIu64
                " batch(es) WOKE WITH NOTHING (last join: %" PRIu64 " doorbells served, %"
                PRIu64 " of them forwarded, %" PRIu64 " new since the previous batch)",
                a.gsp_event_raises, a.gsp_event_unvectored, a.gsp_event_masked,
                a.os_event_woke_with_nothing, a.os_event_last_join_served,
                a.os_event_last_join_forwarded, a.os_event_last_join_advanced);

    /*
     * ★★★ #146 — THE FRAMEBUFFER LINE, printed unconditionally INCLUDING when every number
     * is zero, for the reason every other unconditional block here is: "no line appeared"
     * is what a silently-dead reporter looks like.
     *
     * `fb refusals` is the one to read.  A dropped framebuffer write used to have no
     * symptom at all until kbusVerifyBar2 reported NV_ERR_MEMORY_ERROR hundreds of
     * operations later; this number is that fact, at teardown, in one place.
     *
     * `window drops` counts the two GMMU-TRANSLATED windows, which this port genuinely has
     * no address model for.  They are separate numbers because they are separate findings:
     * one says "we are not there yet", the other says "we are there and we lost bytes".
     */
    info_report("nvkvm: framebuffer: %" PRIu64 " reads / %" PRIu64 " writes served through "
                "the BAR0 moving window (%" PRIu64 " window register reads / %" PRIu64
                " writes), fb refusals %" PRIu64 ", resident %" PRIu64 " bytes",
                a.fb_reads, a.fb_writes, a.bar0_window_reads, a.bar0_window_writes,
                a.fb_refusals, a.fb_resident_bytes);
    /*
     * ★★★★ THE COUNTER THAT COULD NOT MOVE, AND NOW CAN — read its history before its value.
     *
     * This line used to end "translated-window drops %ur/%uw" and its own comment claimed
     * the pair counted "the two GMMU-TRANSLATED windows".  MEASURED 2026-08-09, and the
     * sentence was wrong twice over: it counts ONE window (BAR2's refusals go to
     * `bar2_faults`), and that one was UNREACHABLE — both increment sites sat in the
     * `WindowRefusal::NoAddressModel` arm, returned only for BAR1, and BAR1 registered with
     * nvkvm_reservation_ops and never crossed this seam.  Its `0r/0w` was VACUOUS: true,
     * unfalsifiable, and read by three rungs as evidence that no translated window ever
     * dropped a byte.  Same shape as `pgrep -x qemu-system-x86_64`.
     *
     * ★★★★ §16.18 CHANGED THE PRECONDITION, so read this zero differently now.  BAR1 traps
     * (KAYFABE_BUS_BAR_FB) and `window_phys` translates it whenever the chip row states a
     * `bar1PdeBase`.  The `NoAddressModel` arm survives for a chip row that states NONE, so
     * a non-zero value here no longer means "BAR1 leaked across the seam" — it means **this
     * device is running a chip profile with no framebuffer-aperture address model at all**,
     * and every `bar1_*` number below it is about nothing.  ⊘ That is exactly what
     * `bar1_pde_base` in the BAR1 block is for; the two must be read together.
     */
    if (a.fb_window_reads || a.fb_window_writes) {
        warn_report("nvkvm:   ⚠ windows with NO ADDRESS MODEL: %" PRIu64 "r/%" PRIu64 "w "
                    "dropped. Since §16.18 the only way to reach this is a chip row whose "
                    "bar1PdeBase is 0 (this boot's is 0x%" PRIx64 "), so BAR1 has no root to "
                    "walk and its bytes went nowhere.",
                    a.fb_window_reads, a.fb_window_writes, a.bar1_pde_base);
    } else {
        /*
         * ⊘⊘ THIS ARM USED TO ASSERT ITS OWN PRECONDITION AND COULD NOT CHECK IT.  It read
         * "and since §16.18 this zero is NO LONGER VACUOUS: BAR1 traps and is translated",
         * which is a claim that BAR1 was EXERCISED — unconditional, and false on any boot
         * that never touches BAR1, where `0r/0w` is exactly as vacuous as it was before
         * §16.18.  The same shape as the sentence §16.35 removed two blocks below: a claim
         * frozen into runtime output, read as evidence because it arrives inside a
         * measurement, with no mechanism to expire.  A section number in a log line is the
         * tell.
         *
         * ⇒ It now PRINTS THE PRECONDITION instead of asserting it.  The reader sees how
         * much BAR1 traffic crossed the translated path and decides for themselves whether
         * this zero is a statement; the argument for why it can be one lives in
         * docs/design/bar1_translation, where staleness is expected and dated.
         */
        info_report("nvkvm:   windows with no address model: 0r/0w — read it beside the "
                    "BAR1 traffic that had to exist for this zero to say anything: %"
                    PRIu64 " translated read(s) / %" PRIu64 " write(s), bar1PdeBase = 0x%"
                    PRIx64 ". ⊘ With 0 BAR1 accesses this line reports nothing about the "
                    "guest.",
                    a.bar1_reads, a.bar1_writes, a.bar1_pde_base);
    }
    /*
     * ★★★★ §16.13 — WHICH bytes, not how many.  MEASURED 2026-08-09 (boot bar1_03a679f):
     * the framebuffer page the guest's own page tables name for its GPFIFO ring dumped as
     * `nz0/4096` — not one non-zero byte — while the page tables themselves dumped `nz2`.
     * ⊘ A byte census cannot say WHY: FbStore::read returns zero AND Ok for a page nobody
     * ever wrote, so "never written" and "written with zeros" print identically.  Residency
     * separates them, and the extent says whether the resident set is CLUSTERED (one write
     * path) or SPREAD (several).
     *
     * ⊘ `fb_resident_valid` is the precondition, printed as its own sentence: a device with
     * no framebuffer port has no residency to report, and that is not the same fact as a
     * framebuffer in which nothing is resident.
     */
    if (!a.fb_resident_valid) {
        info_report("nvkvm:   framebuffer residency: ⊘ NO STORE TO ASK — this device has no "
                    "framebuffer port installed, which is NOT the same fact as a framebuffer "
                    "in which nothing is resident.");
    } else if (a.fb_resident_pages == 0) {
        info_report("nvkvm:   framebuffer residency: 0 pages — the store exists and the guest "
                    "has written NOTHING to it.");
    } else {
        info_report("nvkvm:   framebuffer residency: %" PRIu64 " page(s) spanning "
                    "[0x%" PRIx64 "..0x%" PRIx64 "] — %" PRIu64 " page(s) of extent, so the "
                    "resident set is %s. ⊘ A page NOT in this set was never written; a page "
                    "in it that reads zero WAS addressed and given zeros. The byte census "
                    "alone cannot tell those apart.",
                    a.fb_resident_pages, a.fb_resident_lo, a.fb_resident_hi,
                    ((a.fb_resident_hi - a.fb_resident_lo) / 4096u) + 1u,
                    (((a.fb_resident_hi - a.fb_resident_lo) / 4096u) + 1u) == a.fb_resident_pages
                        ? "CONTIGUOUS" : "SPARSE");
    }

    /*
     * ★★★★ §16.16 — WHO CREATED THOSE PAGES, and WHETHER A RING IS ANYWHERE AMONG THEM.
     *
     * Both blocks hang off the same precondition as the extent above, for the same reason:
     * an archive that never wrote this struct leaves all zeros, and zero must be the honest
     * non-claim rather than "no page has an origin" / "no ring-like page exists".
     */
    if (a.fb_resident_valid) {
        /*
         * ⊘ READ THE UNATTRIBUTED SLOT BEFORE THE OTHER FOUR.  MEASURED at tree e394b69:
         * the whole tagging mechanism existed and NOTHING CALLED IT — a repo-wide search
         * for `write_tagged` returned its own definition and its own default impl and
         * nothing else — so every framebuffer write recorded UNATTRIBUTED.  Booting that
         * tree would have printed a census reading 100% UNATTRIBUTED, which by the
         * instrument's own definition means "we did not instrument that path": a
         * non-finding wearing the shape of a measurement.  The arm below SAYS SO when the
         * slot dominates, so the next reader cannot mistake it for a fact about the guest.
         */
        info_report("nvkvm:   framebuffer FIRST-WRITER census: PRAMIN %" PRIu64
                    " / BAR1 %" PRIu64 " / BAR2 %" PRIu64 " / EXEC %" PRIu64
                    " / UNATTRIBUTED %" PRIu64 " page(s) — FIRST writer, not last, so this "
                    "attributes CREATION and not traffic.",
                    a.fb_origin_by_writer[0], a.fb_origin_by_writer[1],
                    a.fb_origin_by_writer[2], a.fb_origin_by_writer[3],
                    a.fb_origin_by_writer[4]);
        if (a.fb_resident_pages != 0 &&
            a.fb_origin_by_writer[4] * 2u >= a.fb_resident_pages) {
            warn_report("nvkvm:   ⊘ MOST RESIDENT PAGES ARE UNATTRIBUTED — this is a "
                        "statement about THIS PORT, not about the guest: some framebuffer "
                        "write path is not passing a writer tag. Do not cite the other four "
                        "counts as a census of where the guest's bytes came from until it "
                        "is.");
        }

        /*
         * ★★★★ THE FORWARD SEARCH.  ⊘ It concludes nothing; the two arms below are worded
         * so that neither reads as a verdict about the guest on its own.  Its whole value
         * is that it is INDEPENDENT of the page-table descent: the descent's answer and
         * this one were produced by disjoint code over the same bytes, so they can
         * genuinely disagree — which is exactly what a second projection of one computation
         * can never do.
         */
        if (a.fb_sweep_ringlike == 0) {
            info_report("nvkvm:   GPFIFO forward search: swept %" PRIu64 " of %" PRIu64
                        " resident page(s), and NO page carries GPFIFO-entry-shaped bytes. "
                        "⊘ This says the ring's bytes are not in THIS store — it does not "
                        "say the guest never wrote them.",
                        a.fb_sweep_swept, a.fb_resident_pages);
        } else {
            static const char *const writers[6] = {
                "NO-ORIGIN-RECORDED", "PRAMIN", "BAR1", "BAR2", "EXEC", "UNATTRIBUTED"
            };
            uint64_t w = a.fb_sweep_best_writer_plus1;

            info_report("nvkvm:   GPFIFO forward search: swept %" PRIu64 " of %" PRIu64
                        " resident page(s); %" PRIu64 " carry GPFIFO-entry-shaped bytes. "
                        "★ BEST 0x%" PRIx64 " with %" PRIu64 " shaped entries, created by "
                        "%s. ⊘ Compare that ADDRESS with the leaf the doorbell probe's walk "
                        "reported: if they differ, the write was caught and the DESCENT is "
                        "aimed at the wrong table.",
                        a.fb_sweep_swept, a.fb_resident_pages, a.fb_sweep_ringlike,
                        a.fb_sweep_best, a.fb_sweep_best_score,
                        writers[w < 6 ? w : 0]);
        }
    }

    /*
     * ★★★★ §16.16 — THE TRAP-STATUS TABLE.  The owner's hypothesis, and it is the one
     * question `docs/design/ring_write_path_map.md` is STRUCTURALLY BLIND TO: that document
     * enumerated five stores we write INTO and concluded "every reachable write path lands
     * in the store the walker reads", which quantifies only over paths WE IMPLEMENT.  A
     * guest write that never reaches this device at all — because the range is served by a
     * directly-mapped slot rather than trapped — produces EXACTLY the observation we have
     * (a never-written page) while the guest writes happily at full speed.
     *
     * ⊘ So this asks the hypervisor, not ourselves: for each of this device's regions, is
     * the memory region actually installed at that guest-physical base OUR region, and is
     * it RAM (served directly, invisible to us) or IO (trapped, so every access is ours)?
     * memory_region_find walks the live address space, so a foreign region overlaying ours
     * — the shape that would hide the writes — shows up as a NAME THAT IS NOT OURS.
     *
     * ⚠ Printed for every row including the ones we are confident about, because the value
     * of the table is the DIFFERENTIAL between rows: BAR1 answering "io" and BAR2 answering
     * "io" while some row answers "ram" is the finding, and a table that omitted the
     * boring rows could not show it.
     */
    for (ri = 0; ri < NVKVM_N_REGIONS; ri++) {
        const NvkvmRegionSpec *row = &nvkvm_regions[ri];
        pcibus_t base = pci->io_regions[row->pci_bar].addr;
        MemoryRegionSection sec;

        if (base == PCI_BAR_UNMAPPED) {
            info_report("nvkvm:   trap-status %s: UNMAPPED — the guest never assigned this "
                        "base-address register, so nothing can have been written through it.",
                        row->name);
            continue;
        }
        sec = memory_region_find(get_system_memory(), (hwaddr)base, 1);
        if (!sec.mr) {
            info_report("nvkvm:   trap-status %s: base 0x%" PRIx64 " resolves to NO REGION "
                        "— ⚠ the guest assigned a base this address space does not serve.",
                        row->name, (uint64_t)base);
            continue;
        }
        info_report("nvkvm:   trap-status %s: base 0x%" PRIx64 " -> region '%s' %s, %s. %s",
                    row->name, (uint64_t)base,
                    memory_region_name(sec.mr),
                    sec.mr == &s->mr[ri] ? "(OURS)" : "⚠ (NOT OURS — it overlays this BAR)",
                    memory_region_is_ram(sec.mr) ? "RAM — served DIRECTLY, every guest access "
                                                   "to it is INVISIBLE to this device"
                                                 : "IO — TRAPPED, every guest access reaches "
                                                   "this device",
                    memory_region_is_ram(sec.mr)
                        ? "⊘ A never-written page behind a RAM row proves nothing about the "
                          "guest: we would not have seen the write."
                        : "⇒ a never-written page behind this row IS a statement about the "
                          "guest, because we would have seen the write.");
        memory_region_unref(sec.mr);
    }

    /*
     * ★★★★ §16.18 — THE DISCARDING RESERVATION IS GONE, AND THIS SAYS SO FROM THE TABLE.
     *
     * What stood here was `reservation_touches`, a counter incremented only by
     * nvkvm_reservation_read/write.  BAR1 was the last row that selected those callbacks;
     * with it a TRAP they are unreferenced, so the counter could never move again and
     * printing it would have been a permanently-true sentence — the same vacuity §16.11
     * caught in the translated-window-drops line, reintroduced one block later.
     *
     * ⊘ A statement that cannot be false is not an instrument.  So this counts the region
     * TABLE instead, which somebody really can change: a row reintroduced as
     * NVKVM_KIND_RESERVATION would appear here, and its accesses would be destroyed exactly
     * the way BAR1's three submission writes were.
     */
    {
        /* ⚠ `rk`, not `ri`: this function already has an `ri` at its top and 418b951 exists
         * because a loop counter here shadowed five inner declarations.  The compiler says
         * so under -Wshadow=compatible-local; this bench builds with werror=false, so the
         * warning is advisory and the name is the whole defence. */
        unsigned rk, nres = 0;

        for (rk = 0; rk < NVKVM_N_REGIONS; rk++) {
            if (nvkvm_regions[rk].kind == NVKVM_KIND_RESERVATION) {
                nres++;
                warn_report("nvkvm: ⚠ region '%s' is a DISCARDING RESERVATION — accesses to "
                            "it reach no store. Since §16.18 no row is supposed to be one.",
                            nvkvm_regions[rk].name);
            }
        }
        if (nres == 0) {
            info_report("nvkvm: discarding reservations: none — every region this device "
                        "presents either traps to the archive or is the interrupt table. "
                        "⊘ This is read off the region table, not off a counter, so it "
                        "cannot go stale the way an unreachable counter did.");
        }
    }
    /*
     * ★★★★ §16.17 — AND THE ACCESSES THEMSELVES.  `[src] ogkm-580: uvm_channel.c:984-1015`
     * writes the GPFIFO entry through a dereferenced CPU POINTER and then `write_gpu_put`;
     * for a vidmem ring that CPU mapping is a BAR1 mapping, and this handler discards the
     * value.  ⊘ The COUNT cannot test that — 3 is equally what "the guest barely touched
     * BAR1" predicts.  An 8-byte store whose value decodes as a plausible GPFIFO entry is a
     * different fact from three stray probes, and only the value can say which.
     */
    if (s->bar1_log_used == 0) {
        info_report("nvkvm:   BAR1 access log: EMPTY — no access reached the handler, so "
                    "there is nothing to attribute. ⊘ Read this beside the trap-status row "
                    "for this BAR: it means 'no access' only while that row says TRAPPED.");
    } else {
        uint64_t k;

        info_report("nvkvm:   BAR1 access log: %u of %" PRIu64 " access(es) recorded in full "
                    "(FIRST ones, because the write under test precedes the doorbell)%s",
                    s->bar1_log_used, s->bar1_touches,
                    s->bar1_log_used < s->bar1_touches
                        ? " — ⊘ BOUNDED-LOG, later accesses are not shown"
                        : " — complete");
        for (k = 0; k < s->bar1_log_used; k++) {
            info_report("nvkvm:     BAR1[%" PRIu64 "] %s off=0x%" PRIx64 " size=%u val=0x%" PRIx64
                        " ⊘ NOT A TIMELINE — this row was RECORDED when it happened and is "
                        "PRINTED now; its timestamp is this dump's. Order it against nothing.",
                        k, s->bar1_log[k].is_write ? "WRITE" : "read ",
                        s->bar1_log[k].addr, s->bar1_log[k].size, s->bar1_log[k].val);
        }
    }
    /*
     * ★★★★★ ITEM 2 / w262 — THE UNCAPPED TOTAL, beside the capped print count.
     *
     * ⊘ Printed unconditionally, zero included.  A boot with no GP_PUT store at all and a
     * boot whose witness was never compiled in are the same absence otherwise — the
     * `dlen=0` shape, and the reason this block states its own zero.
     */
    info_report("nvkvm: BAR1 GP_PUT: %" PRIu64 " write(s) at USERD+0x%x, %u printed LIVE "
                "(cap %u). ⊘ The live lines above carry the guest's own instants and are "
                "the ONLY rows here that may be ordered against anything; this total is "
                "uncapped and the per-row cap never touches it.",
                s->gp_put_writes, NVKVM_USERD_GP_PUT, s->gp_put_printed, NVKVM_GP_PUT_LIVE);
    /* ★★★★★ w279 — beside the total, never folded into it. */
    info_report("nvkvm: BAR1 GP_PUT: %" PRIu64 " of those %" PRIu64 " carried a value that "
                "CANNOT be a put pointer (>= %u, the largest GPFIFO this tree has seen) ⇒ "
                "they are guest DATA at offset 0x%x of a page that is not a USERD. "
                "⊘ A LOWER BOUND on the false positives and never an upper one: a data word "
                "that happens to be small is unprovable either way, so the total above is "
                "an offset census and MUST NOT be read as a cursor count (w279, measured on "
                "w278b where 2 of 8 were the CE client's own payload magics).",
                s->gp_put_implausible, s->gp_put_writes, NVKVM_GP_PUT_MAX_ENTRIES,
                NVKVM_USERD_GP_PUT);
    {
        unsigned pi;

        info_report("nvkvm: BAR1 GP_PUT pages: %u distinct USERD page(s) ever advanced a "
                    "cursor, %" PRIu64 " advance(s) DROPPED because the table was full "
                    "(cap %u). ⊘ A dropped page is the one you would most want; it is counted "
                    "here and nowhere else.",
                    s->gp_put_pages_used, s->gp_put_pages_dropped, NVKVM_GP_PUT_PAGES);
        for (pi = 0; pi < s->gp_put_pages_used; pi++) {
            info_report("nvkvm:   GP_PUT page[%u] +0x%" PRIx64 " first_val=0x%" PRIx64
                        " advances=%" PRIu64,
                        pi, s->gp_put_pages[pi].page, s->gp_put_pages[pi].first_val,
                        s->gp_put_pages[pi].writes);
        }
    }

    /*
     * ★★★ #149 — THE TRANSLATED WINDOW.  Printed unconditionally, all-zeros included, for
     * the same reason as every other block here.
     *
     * Three numbers and they answer three different questions.  `roots published` says
     * whether UPDATE_BAR_PDE ever arrived — the guest ignores that command's status, so
     * this is the ONLY observable.  `bar2 faults` says whether a translated access was
     * refused by name.  `reads/writes` say whether a page walk actually resolved one.
     */
    info_report("nvkvm: BAR2 (translated): %" PRIu64 " reads / %" PRIu64 " writes resolved "
                "through the GMMU, %" PRIu64 " REFUSED by name; roots published %" PRIu64
                " (%" PRIu64 " bodies refused), BAR2 root entry 0x%" PRIx64,
                a.bar2_reads, a.bar2_writes, a.bar2_faults,
                a.bar_pde_updates >> 32, a.bar_pde_updates & 0xffffffffu,
                a.bar2_root_entry);

    /*
     * ★★★★ §16.30 — THE PAGE-DIRECTORY ROOT THE GUEST INSTALLS (0x00801813).
     *
     * Printed UNCONDITIONALLY, all-zeros included, and THE PRECONDITION IS PRINTED FIRST.
     * `hVASpace == 0` is a real handle value — it names the client/device pair's implicit
     * VA space — so "installed a root into VAS 0" and "no SET ever arrived" would print
     * identically without `valid`.  This device has been bitten by an absence decoded as a
     * measurement before (the C oracle's dlen=0 rows), and this is the same shape.
     *
     * ⊘ The handle is REPORTED, never named.  Whether it is the Device's implicit VA space
     * or a user VA space (which is what UVM allocates) is what this line exists to settle
     * from a boot rather than from header semantics — so the printer must not editorialise.
     */
    if (a.set_page_dir_valid) {
        info_report("nvkvm: SET_PAGE_DIRECTORY (0x00801813): %" PRIu64 " ACCEPTED, %" PRIu64
                    " refused; latest hClient 0x%" PRIx64 " hObject 0x%" PRIx64
                    " hVASpace 0x%" PRIx64 " physAddress 0x%" PRIx64 " numEntries %" PRIu64
                    " flags 0x%" PRIx64 " (aperture %" PRIu64 ")",
                    a.set_page_dir_total, a.set_page_dir_refused,
                    a.set_page_dir_client, a.set_page_dir_object,
                    a.set_page_dir_h_vaspace, a.set_page_dir_phys,
                    a.set_page_dir_num_entries, a.set_page_dir_flags,
                    a.set_page_dir_flags & 0x3u);
    } else {
        info_report("nvkvm: SET_PAGE_DIRECTORY (0x00801813): NO RECORD LATCHED "
                    "(%" PRIu64 " accepted, %" PRIu64 " refused) — this is NOT "
                    "\"installed into VA space 0\"; it is \"nothing was installed\". "
                    "A boot that reaches cuInit with 0 accepted did not exercise the rung.",
                    a.set_page_dir_total, a.set_page_dir_refused);
    }

    /*
     * ★★★★ §16.18 — THE FRAMEBUFFER APERTURE, TRANSLATED.  Printed unconditionally,
     * all-zeros included, and the PRECONDITION IS PRINTED FIRST.
     *
     * ⊘ `bar1PdeBase` is not decoration.  BAR1's root is the one root the guest never sends
     * us: MEASURED against ogkm-580, NV_RM_RPC_UPDATE_BAR_PDE has two call sites
     * (kern_bus.c:880, kern_bus_gm107.c:2137) and BOTH pass NV_RPC_UPDATE_PDE_BAR_2, while
     * kbusPatchBar1Pdb_GSPCLIENT (kern_bus.c:755-807) takes GspStaticConfigInfo.bar1PdeBase
     * — OUR number — and re-roots CPU-RM's own walker onto it.  So a 0 here means we never
     * gave the guest anywhere to build its page tables, and `0 reads / 0 writes` beneath it
     * would be a fact about US.
     *
     * ★ `root entry published by the guest` is the standing refutation test for the
     * paragraph above.  It is expected to read NO on every boot; a YES would mean some
     * driver version does send an UPDATE_BAR_PDE(BAR_1) after all, and the address model
     * would then have two candidate roots that could disagree.  Measured every boot rather
     * than argued once from a grep.
     */
    info_report("nvkvm: BAR1 (translated): %" PRIu64 " reads / %" PRIu64 " writes resolved "
                "through the GMMU, %" PRIu64 " REFUSED by name; bar1PdeBase = 0x%" PRIx64
                " (%s), root entry published by the guest: %s",
                a.bar1_reads, a.bar1_writes, a.bar1_faults, a.bar1_pde_base,
                a.bar1_pde_base
                    ? "the framebuffer address WE published and the walk starts from"
                    : "⚠ ZERO — no address model, so every number on this line is about us",
                a.bar1_root_published
                    ? "⚠ YES — this REFUTES the ogkm reading that BAR1 has no such command"
                    : "no, as ogkm-580's two UPDATE_BAR_PDE call sites predict");

    /*
     * ★★★ THE VA-SPACE PAGE-DIRECTORY PUBLICATIONS.  Printed unconditionally, all-zeros
     * included, for the reason every other block here is.
     *
     * MEASURED 2026-08-08 over traces/real_ga106/rpc_transcript_real_ga106.txt (a real
     * 580.159.04 driver on a real GA106): NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY — the ONE
     * control this port turns into a page-directory base — occurs ZERO times in the whole
     * boot, while 0x90f10106 occurs four times and 0x20800a9f once.  So these rows are the
     * only thing a boot can say about its own address spaces, and a report without them
     * was a report in which "the guest never published a page directory" and "the guest
     * published four and we discarded them" printed identically.
     *
     * ⊘ These are OBSERVED, never answered: the recording link declines every command and
     * kayfabe_device::inittables::InitTablePolicy still answers these two ids byte for
     * byte.  A row here is a statement about the GUEST, not about what we did with it —
     * and today we do nothing with it.
     *
     * `levels[0]` is the ROOT (ogkm-580: gpu_vaspace.c:3974-4031 fills top-down from
     * pFmt->pRoot; the receiver consumes bottom-up at :4492), and `aperture` decides
     * whether that address is a framebuffer offset or a guest-physical one
     * (VIDEO=1 -> ADDR_FBMEM, SYS_COH/SYS_NONCOH -> ADDR_SYSMEM, :4503-4511).
     */
    /* ⊘ This line used to end "— SET_PAGE_DIRECTORY is never sent", and that clause was
     * MEASURED FALSE from the moment `0x00801813` was served (§16.30).  `[measured, boots
     * s28_933a709_spd and s31_675af4a_echofix]` the census printed
     *
     *     SET_PAGE_DIRECTORY (0x00801813): 1 ACCEPTED, 0 refused; …
     *     VA-space page-directory publications: … SET_PAGE_DIRECTORY is never sent
     *
     * — two lines apart, in the same report, contradicting each other.  ★ A claim frozen
     * into a log string does not age with the code that made it true, and it is read as
     * evidence precisely because it sits inside a measurement.  So this line now reports
     * only what it counts, and says which sources it counts, without claiming anything
     * about what the guest does NOT do. */
    info_report("nvkvm: VA-space page-directory publications: %" PRIu64 " total, %" PRIu64
                " distinct, %" PRIu64 " UNDECODABLE (counted from 0x90f10106 / 0x20800a9f "
                "ONLY; ⊘ 0x00801813 publishes a root too and is counted on its own line "
                "above, so these totals are NOT every root the guest published)",
                a.gvas_pub_total, a.gvas_pub_len, a.gvas_pub_undecodable);
    /* ★★★★ THE ROOT TABLE'S COMPLETENESS — printed only when it is NOT complete, because
     * a line that is always there is a line nobody reads.  A non-zero value means the
     * lookup that answers "can this channel address anything" stopped seeing publications,
     * so every `CeResolve::NoPublication` refusal in this boot is unsafe to believe. */
    if (a.gvas_pub_roots_refused) {
        warn_report("nvkvm:   ★★★ THE PAGE-DIRECTORY ROOT TABLE IS INCOMPLETE: %" PRIu64
                    " publication(s) REFUSED by its cap. Every NoPublication refusal in "
                    "this boot may be a publication we dropped, not one the guest never "
                    "sent.", a.gvas_pub_roots_refused);
    }
    /* ★★★ §14.23 — and what the OBJECT MODEL made of them, which is a different link's
     * count.  Until 2026-08-08 the line above was the whole story: the port decoded this
     * control, answered NV_OK and dropped the value, so every promote-ctx could only refuse
     * `ContextVasUndeclared`.  `applied` is the number a claim about the page-directory
     * plane may cite; `seen` is its denominator, and `seen == 0` beside a non-zero `total`
     * means the seat that forwards them was never filled. */
    info_report("nvkvm:   of those, %" PRIu64 " reached the object model, %" PRIu64
                " ACCEPTED (Vas::pdb populated from the guest's own publication), %" PRIu64
                " not an event",
                a.gvas_pub_seen, a.gvas_pub_applied, a.gvas_pub_unexpected);
    {
        uint64_t i, j, shown = a.gvas_pub_len;

        if (shown > KAYFABE_GVAS_PUBLICATION_SLOTS) {
            shown = KAYFABE_GVAS_PUBLICATION_SLOTS;
        }
        for (i = 0; i < shown; i++) {
            const KayfabeGvasPublication *g = &a.gvas_pub[i];
            uint64_t nl = g->num_levels;

            info_report("nvkvm:   gvas cmd 0x%08x hClient 0x%08x hObject 0x%08x "
                        "va [0x%016" PRIx64 "..0x%016" PRIx64 "] pageSize 0x%" PRIx64
                        " levels %u subdev %u/%u x%" PRIu64,
                        g->cmd, g->client, g->object, g->virt_addr_lo, g->virt_addr_hi,
                        g->page_size, g->num_levels, g->h_subdevice, g->subdevice_id,
                        g->count);
            if (nl > KAYFABE_GVAS_MAX_LEVELS) {
                nl = KAYFABE_GVAS_MAX_LEVELS;
            }
            for (j = 0; j < nl; j++) {
                const KayfabePdeLevel *lv = &g->levels[j];

                /* ⊘ levels[0] is labelled ROOT in the line itself.  An operator reading
                 * this for the first time must not have to know the top-down convention to
                 * find the one address the whole census exists to carry. */
                info_report("nvkvm:     level[%" PRIu64 "]%s phys 0x%016" PRIx64
                            " size 0x%" PRIx64 " aperture %u pageShift %u",
                            j, j == 0 ? " ROOT" : "", lv->phys_address, lv->size,
                            lv->aperture, lv->page_shift);
            }
        }
    }

    /*
     * ★★★ THE LIST.  Printed unconditionally, INCLUDING when it is empty, because "no line
     * appeared" is exactly what a silently-dead reporter looks like and this device has
     * been bitten by that before (see nvkvm_report_registers' own note on shutdown).
     */
    info_report("nvkvm: commands: %" PRIu64 " decoded, %" PRIu64 " UNSERVICED "
                "(refused by name; the guest logs these quietly, which is why they are here)"
                ", %" PRIu64 " distinct",
                a.commands, a.commands_unserviced, a.unserviced_len);
    {
        uint64_t i, shown = a.unserviced_len;

        if (shown > KAYFABE_UNSERVICED_SLOTS) {
            shown = KAYFABE_UNSERVICED_SLOTS;
        }
        for (i = 0; i < shown; i++) {
            uint32_t fn = (uint32_t)(a.unserviced[i] >> 32);
            uint32_t cmd = (uint32_t)(a.unserviced[i] & 0xffffffffu);

            if (cmd == KAYFABE_UNSERVICED_NO_CMD) {
                info_report("nvkvm:   unserviced fn %u", fn);
            } else {
                info_report("nvkvm:   unserviced fn %u cmd 0x%08x", fn, cmd);
            }
        }
        /*
         * ⊘⊘ THE LINE WHOSE ABSENCE COST A ROOT CAUSE.  A saturated list used to be
         * indistinguishable from a complete one -- §14.31 read a miss from a full 32-slot
         * ledger as "this control never reaches the emulated GSP".  Printed only when it
         * is true, and phrased so the reader cannot conclude anything from an absence.
         */
        if (a.unserviced_len > shown) {
            info_report("nvkvm:   ⊘ unserviced list TRUNCATED: %" PRIu64 " distinct, only "
                        "%" PRIu64 " shown — a command MISSING from the rows above may "
                        "simply not have fit. Absence here is NOT evidence of absence.",
                        a.unserviced_len, shown);
        }
    }

    /*
     * ★★★ THE OTHER HALF OF THE LIST.  Printed unconditionally and INCLUDING when it is
     * zero, for the same reason the block above is: a bridge refusal answers the guest's
     * command, so it reaches no ledger, and boot `alloc1` had to be diagnosed by a
     * function number being ABSENT from six lines.  "bridge refusals: 0" is a POSITIVE
     * statement that the bridge refused nothing; the absence of a line is not.
     */
    info_report("nvkvm: bridge refusals: %" PRIu64 " total, %" PRIu64 " distinct "
                "(these ANSWER the command and so never reach the unserviced list)",
                a.bridge_refusals, a.bridge_refusal_len);
    {
        uint64_t i, shown = a.bridge_refusal_len;

        if (shown > KAYFABE_BRIDGE_REFUSAL_SLOTS) {
            shown = KAYFABE_BRIDGE_REFUSAL_SLOTS;
        }
        for (i = 0; i < shown; i++) {
            const KayfabeBridgeRefusal *r = &a.bridge_refusal[i];
            int len = (int)(r->tag_len > KAYFABE_BRIDGE_REFUSAL_TAG_LEN
                            ? KAYFABE_BRIDGE_REFUSAL_TAG_LEN : r->tag_len);

            /* ★★★★ 16.56 — the ids the tag cannot carry, appended to the SAME line so a
             * grep for the tag returns them.  A row with no ids prints exactly as before. */
            char ids[KAYFABE_REFUSAL_IDS_PER_TAG * 12u + 8u];
            uint64_t nid = r->ids_len > KAYFABE_REFUSAL_IDS_PER_TAG
                           ? KAYFABE_REFUSAL_IDS_PER_TAG : r->ids_len;
            int at = 0;
            uint64_t j;

            ids[0] = '\0';
            for (j = 0; j < nid && at >= 0 && (size_t)at < sizeof(ids); j++) {
                at += snprintf(ids + at, sizeof(ids) - (size_t)at,
                               j == 0 ? " id=0x%08x" : ",0x%08x", r->ids[j]);
            }
            /* %.*s, never %s: the tag is NUL-PADDED and a name that exactly fills the
             * array carries no terminator. */
            info_report("nvkvm:   bridge refusal %.*s x%" PRIu64 "%s%s",
                        len, (const char *)r->tag, r->count, ids,
                        r->ids_len > nid ? " (+more, count is not capped)" : "");
        }
    }

    /*
     * ★★★★ §16.40 — THE FIRST REFUSED GPU_PROMOTE_CTX, WITH THE ADDRESS PLANE'S STATE AS
     * IT STOOD AT THAT INSTANT.
     *
     * Printed UNCONDITIONALLY, including when nothing was latched, for the reason every
     * block in this function is: an absent line and a line saying "none" are different
     * observations, and only one of them is evidence.  ⊘ "no promotion was refused" is a
     * FINDING — read beside `control 0x2080012b result …` in the census below, it
     * separates "every promotion succeeded" from "none arrived".
     *
     * ★★★ WHY IT IS HERE AT ALL.  The per-channel VA-space census this sentence carries
     * has existed since §15 and could be reached ONLY from inside a doorbell-refusal
     * sentence.  MEASURED 2026-08-09: `census[` appears in exactly TWO of the seventeen
     * committed boot logs and in NONE since doorbells began to be served — s35 printed
     * `doorbells: 124 arrived, 124 served, 0 REFUSED by name`, so the refusal that carried
     * the census never fired.  A diagnostic for the ADDRESS plane was gated behind the
     * EXECUTION plane failing; fixing the second silenced the first, and nothing in the
     * report said so.  Three consecutive rungs then recorded "which VA space the failing
     * channel names is unread" and one prescribed a shim ABI bump to add an instrument
     * that was already built and already crossed this ABI.
     */
    if (a.promote_diag_len == 0) {
        info_report("nvkvm: promote-ctx: NO REFUSAL LATCHED — no GPU_PROMOTE_CTX was "
                    "refused this boot. ⊘ Read this against the 0x2080012b rows in the "
                    "control census below: absent there too means none arrived.");
    } else {
        uint64_t i, shown = a.promote_diag_len;

        /*
         * ★★★★ ONE ROW PER REFUSAL KIND, and per-kind is a CORRECTION.  MEASURED, boot
         * `s36_3a0146c_vascensus`: a boot-global "first refusal" latched kernel RM's
         * `UnknownContextObject { client 0xc1d00008, object 0x31415900 }` — refused long
         * before cup2 ran, with a census of the two CE channels alive at that instant —
         * while `ContextVasUndeclared`, the refusal this rung is ABOUT and `x1` in the same
         * boot, was never latched because it was not first.  ⊘ The doorbell precedent that
         * suggested "first" does not transfer: there the flood is identical rings from one
         * guest, here the boot holds several DISTINCT refusals from different callers.
         */
        info_report("nvkvm: promote-ctx refusals: %" PRIu64 " distinct kind(s), each with "
                    "the VA-space census AS IT STOOD AT ITS FIRST refusal",
                    a.promote_diag_len);
        if (shown > KAYFABE_PROMOTE_DIAG_SLOTS) {
            shown = KAYFABE_PROMOTE_DIAG_SLOTS;
        }
        for (i = 0; i < shown; i++) {
            const KayfabePromoteDiag *d = &a.promote_diag[i];
            int tl = (int)(d->tag_len > KAYFABE_BRIDGE_REFUSAL_TAG_LEN
                           ? KAYFABE_BRIDGE_REFUSAL_TAG_LEN : d->tag_len);
            int sl = (int)(d->text_len > KAYFABE_PROMOTE_DIAG_LEN
                           ? KAYFABE_PROMOTE_DIAG_LEN : d->text_len);

            /* %.*s, never %s: NUL-PADDED, and a name or sentence that exactly fills its
             * array carries no terminator.  The archive stamps its own `[CLIPPED …]`. */
            info_report("nvkvm:   promote-ctx %.*s: %.*s",
                        tl, (const char *)d->tag, sl, (const char *)d->text);
        }
    }

    /*
     * ★★★ THE CONTROL CENSUS — the two POSITIVE states the two lists above cannot express.
     * Printed unconditionally, INCLUDING when empty, for the reason every block here is.
     *
     * A row with result 0 is seen-and-served.  A row with a non-zero result is
     * seen-and-REFUSED — a refusal that ANSWERS the command (InitTablePolicy's refuse())
     * reaches neither the unserviced list nor the bridge census, and 0x20800301 was the
     * control named in the guest line that killed a boot while being absent from every
     * line this report printed.  With these rows, an id absent from ALL the lists is
     * finally a control that was never issued.
     */
    /*
     * ★ THE PROBE SET FIRST, unconditionally, empties included: the census rows below are
     * only interpretable against the probe set the boot ran with — an arming of an
     * UNLISTED index served with result 0 means one thing under an empty probe (a
     * defect) and another under a probe naming it (the measurement that was asked for).
     * Three boots ran probe-off while looking armed from the launching shell; this line
     * is what makes that impossible to misread again.
     *
     * ⚠ §14.19 CORRECTED THE SENTENCE THIS PRINTS.  It said an empty probe means "every
     * non-silent notifier arming refused", and that became false the moment
     * kayfabe_abi::eventnotify::DELIVERED_NOTIFIERS existed: index 35 is served in the
     * SHIPPING configuration, because this device now raises the non-stall vector its
     * arming promises (see the `completions:` line above).  A report line that names the
     * wrong reason for a served row is worse than none — an operator reading "every
     * non-silent arming refused" beside `arming event 35 ... result 0x00000000` would
     * diagnose the census, not the boot.
     */
    if (a.probe_arm_len == 0) {
        info_report("nvkvm: probe-arm set: EMPTY (shipping configuration: an arming is "
                    "served only if the index is on SILENT_NOTIFIERS — the event cannot "
                    "occur — or DELIVERED_NOTIFIERS — this device raises it; all others "
                    "refused)");
    } else {
        uint64_t i, pshown = a.probe_arm_len;
        char pbuf[KAYFABE_PROBE_ARM_SLOTS * 12 + 1];
        size_t off = 0;

        if (pshown > KAYFABE_PROBE_ARM_SLOTS) {
            pshown = KAYFABE_PROBE_ARM_SLOTS;
        }
        pbuf[0] = '\0';
        for (i = 0; i < pshown; i++) {
            int n = snprintf(pbuf + off, sizeof(pbuf) - off, "%s%u",
                             i ? "," : "", a.probe_arm[i]);
            if (n < 0 || (size_t)n >= sizeof(pbuf) - off) {
                break;
            }
            off += (size_t)n;
        }
        info_report("nvkvm: probe-arm set: [%s] — PROBE BOOT, reachability only; any "
                    "rung reached under this set is a measurement, NOT a milestone",
                    pbuf);
    }

    info_report("nvkvm: controls: %" PRIu64 " answered, %" PRIu64 " distinct cmd/result "
                "rows (result 0 = served; non-zero = served-but-REFUSED, which reaches "
                "no other list)",
                a.served_total, a.served_len);
    {
        uint64_t i, shown = a.served_len;

        if (shown > KAYFABE_SERVED_CONTROL_SLOTS) {
            shown = KAYFABE_SERVED_CONTROL_SLOTS;
        }
        for (i = 0; i < shown; i++) {
            const KayfabeServedControl *r = &a.served[i];

            info_report("nvkvm:   control 0x%08x result 0x%08x x%" PRIu64 "%s",
                        r->cmd, r->rpc_result, r->count,
                        r->rpc_result ? " REFUSED" : "");
        }
        /* ⊘ The same statement for the other list, for the same reason. */
        if (a.served_len > shown) {
            info_report("nvkvm:   ⊘ control census TRUNCATED: %" PRIu64 " distinct, only "
                        "%" PRIu64 " shown — a control MISSING from the rows above may "
                        "simply not have fit. Absence here is NOT evidence of absence.",
                        a.served_len, shown);
        }

        /*
         * ★★★ §14.42 — THE CE PLANE'S UNBOUGHT HALF, and the fourth sentence of its kind.
         *
         * The three fault-buffer sentences above hang off dedicated counters.  This one is
         * keyed on the CENSUS instead, and the difference is deliberate rather than lazy:
         * those controls needed their SIZES reported, so the audit had to carry fields;
         * here the only question is "was it answered at all", which the census already
         * records exactly.  ⊘ The cost of that choice is named below and is real.
         *
         * ⊘ Why the sentence exists: `control 0x20802a02 result 0x00000000` reads as
         * "handled", and what it actually bought is a DESCRIPTION of four copy engines.
         * `queryCopyEngines` sets ceCaps->supported = NV_TRUE only once BOTH 0x20802a07 and
         * 0x20802a02 answer NV_OK (ogkm-580: nv_gpu_ops.c:8519-8537), and UVM's own
         * ce_is_usable() is `supported && !grce` (kernel-open/nvidia-uvm/uvm_channel.c:
         * 2913-2923) — so answering them is exactly what licenses RM and UVM to build
         * channels on LCE2/LCE3 and push real copies down them.  This port does not
         * complete that work, and the guest does NOT get an error when it isn't completed:
         * it gets a 4000 ms timeout and a blown assertion.  [measured, boot ce1442 at
         * 8ea44dc, the first boot that served these]  A reader who meets only the census row
         * will look for an error and find none — the fault plane's failure shape exactly.
         *
         * ⚠ THE WEAKNESS, STATED: the census TRUNCATES at KAYFABE_SERVED_CONTROL_SLOTS, and
         * this scan can only see the rows that fit.  A boot that served 0x20802a02 and
         * overflowed the census would print the row-truncation warning above and NOT this
         * sentence.  ⊘ So absence of this line is not evidence the controls were unserved —
         * the same caveat that governs the rows themselves, inherited rather than escaped.
         * Promote it to a counter the day that truncation is observed.
         */
        {
            uint64_t i;
            bool ce_described = false;

            for (i = 0; i < shown; i++) {
                if (a.served[i].cmd == 0x20802a02u && a.served[i].rpc_result == 0) {
                    ce_described = true;
                    break;
                }
            }
            if (ce_described) {
                info_report("nvkvm:   ⊘ CE COMPLETION is UNBUILT: serving 0x20802a07 and "
                            "0x20802a02 is what makes queryCopyEngines set "
                            "supported=NV_TRUE and UVM treat LCE2/LCE3 as usable async copy "
                            "engines (ce_is_usable = supported && !grce, "
                            "uvm_channel.c:2913-2923) — but this port DESCRIBES those "
                            "engines and does not complete their work, so submitted CE work "
                            "retires no payload and the guest gets a TIMEOUT plus a blown "
                            "assertion (scrubberDestruct's 4000 ms wait, then "
                            "ce_utils.c:349 lastCompletedPayload == lastSubmittedPayload), "
                            "never an error it can read "
                            "(execution_plane_increments.md)");
            }
        }
    }

    /*
     * ★★ THE NOTIFIER ARMINGS, with the handles they arrived on.  The device's
     * notify_actions[] is device-global while RM's already-armed transition rule is
     * per-subdevice (ogkm-580: subdevice_ctrl_event_kernel.c:126-131), so a second arming
     * of one index on a DIFFERENT subdevice — the aliasing hypothesis — reads as two rows
     * with different object handles, not one line with a count of two.
     */
    info_report("nvkvm: notifier armings (0x20800301): %" PRIu64 " total, %" PRIu64
                " distinct (result 0x%08x = nothing answered)",
                a.arming_total, a.arming_len, KAYFABE_CTRL_NO_REPLY);
    {
        uint64_t i, shown = a.arming_len;

        if (shown > KAYFABE_NOTIFIER_ARMING_SLOTS) {
            shown = KAYFABE_NOTIFIER_ARMING_SLOTS;
        }
        for (i = 0; i < shown; i++) {
            const KayfabeNotifierArming *r = &a.armings[i];

            info_report("nvkvm:   arming event %u action %u client 0x%08x object 0x%08x "
                        "result 0x%08x x%" PRIu64 "%s",
                        r->event, r->action, r->client, r->object, r->rpc_result,
                        r->count, (r->rpc_result != 0
                                   && r->rpc_result != KAYFABE_CTRL_NO_REPLY)
                                  ? " REFUSED" : "");
        }
    }

    /*
     * ★★★ THE CHANNEL BINDS — and specifically WHICH COPY ENGINE the guest named.
     *
     * The scrubber's CE is chosen inside the guest by ceutilsGetFirstAsyncCe and reaches
     * this device only as this control's `engineType` (see KayfabeChannelBind).  Printed
     * unconditionally, zero rows included: "the guest bound no channel" is a diagnosis,
     * and a block that appears only when non-empty cannot state it.
     */
    info_report("nvkvm: channel binds (0xa06f0104): %" PRIu64 " total, %" PRIu64
                " distinct (result 0x%08x = nothing answered)",
                a.bind_total, a.bind_len, KAYFABE_CTRL_NO_REPLY);
    {
        uint64_t i, shown = a.bind_len;

        if (shown > KAYFABE_CHANNEL_BIND_SLOTS) {
            shown = KAYFABE_CHANNEL_BIND_SLOTS;
        }
        for (i = 0; i < shown; i++) {
            const KayfabeChannelBind *r = &a.binds[i];
            char engine[32];

            if (r->ce_index == KAYFABE_BIND_NOT_A_COPY_ENGINE) {
                /* ⊘ Never printed as CE0: not-a-CE and CE0 are different answers, and CE0
                 * is one of the two indices with no non-stall vector. */
                snprintf(engine, sizeof(engine), "not-a-CE");
            } else {
                snprintf(engine, sizeof(engine), "COPY%u", r->ce_index);
            }
            info_report("nvkvm:   bind engineType %u (%s) client 0x%08x object 0x%08x "
                        "result 0x%08x x%" PRIu64 "%s",
                        r->engine_type, engine, r->client, r->object, r->rpc_result,
                        r->count, (r->rpc_result != 0
                                   && r->rpc_result != KAYFABE_CTRL_NO_REPLY)
                                  ? " REFUSED" : "");
        }
    }

    /*
     * ★★★ E0b/E1 — THE ISOLATE PLANE.  Printed unconditionally, all-zeros included, for
     * the reason every other block here is: "no line appeared" is what a silently-dead
     * reporter looks like, and this device has been bitten by that before.
     *
     * Two numbers, two different questions.
     *
     *  - `materialized` — DID THE GUEST CAUSE A SPAWN AT ALL?  Since E0b the isolate is
     *    spawned by an accepted guest RM event and NOT by Gpu::realize, so 0 is a real
     *    diagnosis (the guest never reached an accepted GSP_RM_ALLOC) rather than a blank.
     *    ⊘ It says whether, never why: this device writes it, so it cannot attribute a
     *    spawn to the guest.  scripts/bench/e0_isolate_witness.sh does that, against a
     *    timeline this device does not write.
     *
     *  - `spawn-failed` — DID A PLANE WE ASKED FOR FAIL TO COME UP?  bench_rebuild_notes.md
     *    §5 row 7: a failed real isolate used to be indistinguishable from a deliberately
     *    plane-less build at the seam.  `no-plane` is a configuration; `spawn-failed` means
     *    the host could not do what it was asked, and the sentence below says at which step.
     */
    info_report("nvkvm: isolates: %" PRIu64 " materialized, %" PRIu64 " live, "
                "%" PRIu64 " refusing (%" PRIu64 " no-plane, %" PRIu64 " spawn-failed)",
                a.isolates_materialized, a.isolates_live,
                a.isolates_no_plane + a.isolates_spawn_failed,
                a.isolates_no_plane, a.isolates_spawn_failed);
    /*
     * ★★★ E2 — THE USERMODE DOORBELL APERTURE.  Printed unconditionally, all-zeros
     * included, for the reason every other block here is.
     *
     * `arrived` is the number to read, and it is a statement about the GUEST: it counts
     * writes that landed on NV_VIRTUAL_FUNCTION_DOORBELL, before the core was consulted.
     * `served + refused == arrived` always; neither can absorb the other, so "the transport
     * works and the routing does not" is a readable state and not a silence.
     *
     * ⊘ ZERO IS NOT A FAILURE OF THE TRANSPORT.  At the current wall the guest driver never
     * reaches kfifoUpdateUsermodeDoorbell — the channel SCHEDULE before it fails with 0x56
     * — so a stock boot rings nothing.  See docs/design/execution_plane_increments.md §7,
     * and note that this line ALSO cannot attribute: the device writes it.  The per-write
     * lines above, stamped by -msg timestamp, are the attributing instrument.
     */
    info_report("nvkvm: doorbells: %" PRIu64 " arrived, %" PRIu64 " served, %" PRIu64
                " REFUSED by name; last token %s0x%08" PRIx64 " (%" PRIu64 " logged)",
                a.doorbells, a.doorbells_served, a.doorbells_refused,
                a.doorbell_last_token_valid ? "" : "n/a ",
                a.doorbell_last_token, s->doorbells_logged);
    /*
     * ★★★★ §16.62.3 — WHOSE progress the `served` number is.  Printed on its own line and
     * unconditionally, zeros included, because the two servings are different events with
     * different evidence: `local` means this process moved the bytes and witnessed the end;
     * `forwarded` means a host channel was rung at an instant this device was not standing
     * at.  A boot whose split moved from one to the other would be a change in what
     * `served` MEANS with the number unchanged.
     */
    info_report("nvkvm:   of the served: %" PRIu64 " local (CPU CE, end witnessed), %"
                PRIu64 " forwarded (host channel rung)",
                a.doorbells_served_locally, a.doorbells_served_forwarded);
    /*
     * ★★★★ §16.65 — THE PER-ENGINE DOORBELL CENSUS.
     *
     * ⊘ EVERY bucket, zeros included, on one line.  An empty bucket is a measurement — "no
     * NVENC channel rang" — and a printer that skipped it would make that indistinguishable
     * from "we did not look", which is this campaign's own fifth-limit mistake one plane
     * over.  `unrouted` is last and separate: a channel we did not find, never an engine we
     * do not interpret.
     *
     * ⊘ This is a CENSUS and not a sample.  The 16 per-write doorbell lines above are
     * capped by their own slot count, so they can say "a GR channel was refused" and can
     * never say "the refused population IS the GR population" — which is the only question
     * the routing statement can be falsified by.
     */
    {
        /* KAYFABE_ENGINE_KINDS names of <= 16 chars plus a 20-digit count and a separator,
         * then the unrouted tail.  Sized with room to spare and TRUNCATED rather than
         * overrun — and the truncation is not silent: `n` is checked below. */
        char hist[512];
        int n = snprintf(hist, sizeof(hist), "nvkvm:   by engine:");
        for (unsigned i = 0; i < KAYFABE_ENGINE_KINDS && n > 0 &&
                             (size_t)n < sizeof(hist); i++) {
            n += snprintf(hist + n, sizeof(hist) - (size_t)n, " %s=%" PRIu64,
                          kayfabe_shim_engine_kind_name(i),
                          a.doorbells_by_engine[i]);
        }
        if (n > 0 && (size_t)n < sizeof(hist)) {
            n += snprintf(hist + n, sizeof(hist) - (size_t)n,
                          " unrouted=%" PRIu64, a.doorbells_engine_unrouted);
        }
        /* ⊘ A census that silently lost a bucket would be worse than none — it would read
         * as a complete partition.  So say so, in the line itself. */
        info_report("%s%s", hist,
                    (n < 0 || (size_t)n >= sizeof(hist)) ? " [TRUNCATED]" : "");
    }
    if (a.doorbell_local_serving.present) {
        int len = (int)(a.doorbell_local_serving.len > KAYFABE_DOORBELL_REFUSAL_LEN
                        ? KAYFABE_DOORBELL_REFUSAL_LEN : a.doorbell_local_serving.len);
        info_report("nvkvm:   last CPU-CE serving: %.*s",
                    len, (const char *)a.doorbell_local_serving.text);
    }
    if (a.doorbell_refusal.present) {
        int klen = (int)(a.doorbell_refusal.kind_len > KAYFABE_DOORBELL_KIND_LEN
                         ? KAYFABE_DOORBELL_KIND_LEN : a.doorbell_refusal.kind_len);
        int tlen = (int)(a.doorbell_refusal.len > KAYFABE_DOORBELL_REFUSAL_LEN
                         ? KAYFABE_DOORBELL_REFUSAL_LEN : a.doorbell_refusal.len);

        /* %.*s, never %s: both arrays are NUL-PADDED and a name that exactly fills one
         * carries no terminator. */
        info_report("nvkvm:   first doorbell refusal [%.*s] %.*s",
                    klen, (const char *)a.doorbell_refusal.kind,
                    tlen, (const char *)a.doorbell_refusal.text);
    }

    /*
     * ★★★ §8.2.2 — THE GPFIFO RING ADDRESS THE GUEST DECLARED.  Printed unconditionally,
     * all-zeros included, for the reason every other block here is.
     *
     * `declarations` is the number to read FIRST: it says whether the guest got as far as
     * declaring a ring at all, which is a completely different diagnosis from "it declared
     * one and it looked wrong".  `nonzero` is the validity flag for the address, because
     * gpFifoOffset = 0 is a real declaration (ogkm-580: kernel_graphics.c:2420-2424).
     *
     * ⊘ The address is a GPU VIRTUAL address, and this line does NOT translate it — there
     * is nothing here to translate it with.  Read it against the guest's own RAM layout
     * (the -m size and the machine's PCI hole): that comparison is the measurement.
     */
    info_report("nvkvm: gpfifo rings: %" PRIu64 " declared, %" PRIu64 " with a non-zero "
                "address; first %s0x%016" PRIx64 " (%" PRIu64 " entries) — GPU VIRTUAL, "
                "untranslated",
                a.gpfifo_ring_declarations, a.gpfifo_ring_nonzero,
                a.gpfifo_ring_nonzero ? "" : "n/a ",
                a.gpfifo_ring_va, a.gpfifo_ring_entries);

    /*
     * ★★★ §14.41 — THE REPLAYABLE FAULT BUFFER, and the half we did NOT build.
     *
     * Printed only when the guest actually registered one, because absence is already
     * readable elsewhere: the control census carries 0x20800a9b when it was served and the
     * unserviced list carries it when nothing answered, so a third "none" line here would
     * add no fact.  What this line adds is the one thing neither of those can say —
     * that SERVING it bought registration and nothing else.
     *
     * ⊘ The sentence is not decoration.  `control 0x20800a9b result 0x00000000` reads as
     * "handled"; the failure it hides has no message at all, because a fault that is never
     * delivered is a HANG inside UVM's replayable-fault service loop, not an error.  A reader who meets only
     * the census row will look for an error and find none.
     */
    if (a.fault_buffers_registered) {
        info_report("nvkvm: replayable fault buffer: %" PRIu64 " registration(s) SERVED "
                    "NV_OK; first 0x%" PRIx64 " B = %" PRIu64 " pages, %" PRIu64 " malformed",
                    a.fault_buffers_registered, a.fault_buffer_size, a.fault_buffer_pages,
                    a.fault_buffers_malformed);
        info_report("nvkvm:   ⊘ fault DELIVERY is UNBUILT: this port raises no replayable "
                    "fault and never advances MMU_FAULT_BUFFER_PUT(1), so a fault the guest "
                    "should have been told about becomes a HANG inside UVM's replayable-fault "
                    "service loop, not an "
                    "error (docs/design/resume_from_fault.md §7 steps 5b-5d)");
        if (a.fault_buffers_registered > 1) {
            info_report("nvkvm:   ⚠ MORE THAN ONE registration — real RM refuses the second "
                        "with NV_ERR_NOT_SUPPORTED while one is live (ogkm-580: "
                        "kern_gmmu.c:3117) and this port does not model that, because its "
                        "0x20800a9c UNREGISTER partner is unserved. This is the measurement "
                        "that decision was deferred to");
        }
    }

    /*
     * ★★★ §14.41 rung 2 — THE CLIENT SHADOW QUEUE, and why its sentence is a DIFFERENT one.
     *
     * For the replayable buffer above, the guest polls a BAR0 register this device serves, and
     * "empty" is a statement made on a plane we own.  Here the guest allocates a queue in its
     * OWN sysmem and the GSP — us — is the declared WRITER of it.  Answering NV_OK therefore
     * promises to enqueue fault packets, and on a GSP client this queue is the guest's ONLY
     * route to a non-replayable fault.
     *
     * ⊘ "Unbuilt" alone would be FALSE, which is why the sentence names the substitute: this
     * port reports such a fault as an RC on the channel plus an error notifier, which is
     * simulated_gpu_fault.md 5.2's deliberate choice and IS built.  A reader who is told only
     * that the push is missing would go looking for a silence that is not there.
     */
    if (a.shadow_fault_buffers_registered) {
        info_report("nvkvm: client shadow fault buffer: %" PRIu64 " registration(s) SERVED "
                    "NV_OK; first 0x%" PRIx64 " B = %" PRIu64 " pages, type %" PRIu64
                    " (0=non-replayable), %" PRIu64 " malformed",
                    a.shadow_fault_buffers_registered, a.shadow_fault_buffer_size,
                    a.shadow_fault_buffer_pages, a.shadow_fault_buffer_type,
                    a.shadow_fault_buffers_malformed);
        info_report("nvkvm:   ⊘ shadow-queue PUSH is UNBUILT: on a GSP client the GSP is the "
                    "WRITER of this queue and this port never enqueues a fault packet, so a "
                    "non-replayable fault surfaces as an RC on the channel plus an error "
                    "notifier (simulated_gpu_fault.md 5.2, the deliberate choice) and NEVER "
                    "as a queue entry the guest is polling for");
        if (a.shadow_fault_buffer_type != 0) {
            info_report("nvkvm:   ⚠ shadowFaultBufferType %" PRIu64 " is NOT non-replayable. "
                        "The replayable SHADOW buffer requires Confidential Compute "
                        "(ogkm-580: mmu_fault_buffer_ctrl.c:148), which is off — so this is a "
                        "measurement no boot has produced before, not a configuration",
                        a.shadow_fault_buffer_type);
        }
    }

    /*
     * ★★★ The THIRD buffer, and the only one whose SIZE this device also invents.  BAR0
     * 0xB83110 is served as a deliberate fiction (it read zero, and zero is what killed
     * cuInit); this line is where the number and the caveat are printed together.
     */
    if (a.access_cntr_buffers_registered) {
        info_report("nvkvm: access counter buffer: %" PRIu64 " registration(s) SERVED NV_OK; "
                    "first 0x%" PRIx64 " B = %" PRIu64 " pages, %" PRIu64 " malformed",
                    a.access_cntr_buffers_registered, a.access_cntr_buffer_size,
                    a.access_cntr_buffer_pages, a.access_cntr_buffers_malformed);
        info_report("nvkvm:   ⊘ access-counter NOTIFICATION is UNBUILT: this port advertised "
                    "the buffer's size as a deliberate fiction (BAR0 0xB83110, which read "
                    "zero and killed cuInit) and writes no entry into it and raises no "
                    "notification, so UVM's migration heuristics never fire "
                    "(resume_from_fault.md S2 ruled that acceptable)");
    }

    if (a.isolate_refusal.kind != KAYFABE_ISOLATE_REFUSAL_NONE) {
        const char *kind = a.isolate_refusal.kind == KAYFABE_ISOLATE_REFUSAL_SPAWN_FAILED
                           ? "spawn-failed" : "no-plane";
        int len = (int)(a.isolate_refusal.len > KAYFABE_ISOLATE_REFUSAL_LEN
                        ? KAYFABE_ISOLATE_REFUSAL_LEN : a.isolate_refusal.len);

        /* %.*s, never %s: the sentence is NUL-PADDED and one that exactly fills the array
         * carries no terminator. */
        info_report("nvkvm:   isolate refusal [%s] %.*s",
                    kind, len, (const char *)a.isolate_refusal.text);
    }
}

static void nvkvm_exit_notify(Notifier *n, void *data)
{
    NvkvmState *s = container_of(n, NvkvmState, exit_notifier);

    (void)data;
    nvkvm_report_registers(s);
    /* ★★★ The layout, at the END of the run.  The attach-time report is taken before the
     * guest has enabled bus mastering, so on its own it can only ever say "nothing yet". */
    if (s->regs && s->shim) {
        kayfabe_shim_regs_report_ram_layout(s->regs, s->shim);
    }
}

/*
 * Ask the archive who this device is, and put the answer in configuration space.
 *
 * ★★ Every number below comes from ONE call.  There is no fallback identity and no partial
 * one: an identity assembled half here and half in the archive is the two-descriptions
 * failure this device's whole region table exists to prevent, one plane over.
 */
static bool nvkvm_identity_realize(NvkvmState *s, Error **errp)
{
    PCIDevice *pci = PCI_DEVICE(s);
    KayfabeChipIdentity id;
    const uint8_t *msg = NULL;
    uint64_t msg_len = 0;
    uint8_t *cfg = pci->config;
    int32_t rc;

    memset(&id, 0, sizeof(id));
    rc = kayfabe_shim_chip_identity((uint16_t)s->chip_device_id, &id, &msg, &msg_len);
    if (rc != KAYFABE_OK) {
        error_setg(errp, "nvkvm: the chip table refused an identity (%d): %.*s",
                   (int)rc, (int)msg_len, (const char *)msg);
        return false;
    }
    if (id.abi_version != KAYFABE_SHIM_ABI || id.struct_size != sizeof(id)) {
        error_setg(errp,
                   "nvkvm: the chip identity came back with ABI %u size %u; this shim speaks "
                   "ABI %u size %zu, so the two are from different builds",
                   id.abi_version, id.struct_size, KAYFABE_SHIM_ABI, sizeof(id));
        return false;
    }

    /*
     * ★ The register aperture's size is the chip's, not the operator's.  A property that
     * disagrees is refused rather than clamped: the archive answers registers by offset
     * within the aperture the CHIP declares, so a smaller one would make the guest's map
     * and the archive's map disagree about where the ROM window is — the same class of
     * silent divergence `nvkvm_bars_realize` refuses a non-power-of-two size for.
     */
    if (s->bar0_size != id.regs_aperture_len) {
        error_setg(errp,
                   "nvkvm: bar0-size is 0x%" PRIx64 " but chip %04x:%04x declares a "
                   "0x%" PRIx64 "-byte register aperture; the archive answers registers by "
                   "offset within the chip's aperture, so the two maps would disagree",
                   s->bar0_size, id.vendor_id, id.device_id, id.regs_aperture_len);
        return false;
    }

    /*
     * ★★ AND THE SAME FOR THE TWO WINDOWS, for a sharper reason than the aperture's.
     *
     * The emulated GSP answers NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO out of the chip row,
     * and the guest's resource manager copies barSizeBytes straight into
     * pKernelBus->pciBarSizes[] and sizes its own mappings against it.  If this device
     * registers a 128 MiB window and tells the guest 256 MiB, the guest maps past the end
     * of a region the hypervisor decodes and reads whatever is next — with nothing logged
     * on either side.  Refused at realize, which is the only moment an operator can act.
     */
    if (s->bar1_size != id.fb_window_len) {
        error_setg(errp,
                   "nvkvm: bar1-size is 0x%" PRIx64 " but chip %04x:%04x declares a "
                   "0x%" PRIx64 "-byte framebuffer window, and that is the size the "
                   "emulated GSP tells the guest's resource manager",
                   s->bar1_size, id.vendor_id, id.device_id, id.fb_window_len);
        return false;
    }
    if (s->bar2_size != id.inst_window_len) {
        error_setg(errp,
                   "nvkvm: bar2-size is 0x%" PRIx64 " but chip %04x:%04x declares a "
                   "0x%" PRIx64 "-byte instance window, and that is the size the emulated "
                   "GSP tells the guest's resource manager",
                   s->bar2_size, id.vendor_id, id.device_id, id.inst_window_len);
        return false;
    }

    pci_config_set_vendor_id(cfg, id.vendor_id);
    pci_config_set_device_id(cfg, id.device_id);
    pci_config_set_revision(cfg, id.revision);
    pci_config_set_class(cfg, (uint16_t)(id.class_code >> 8));
    cfg[PCI_CLASS_PROG] = (uint8_t)(id.class_code & 0xff);
    pci_set_word(cfg + PCI_SUBSYSTEM_VENDOR_ID, id.subsystem_vendor_id);
    pci_set_word(cfg + PCI_SUBSYSTEM_ID, id.subsystem_id);

    s->msix_vectors = id.msix_vectors;

    info_report("nvkvm: presenting %04x:%04x class %06x rev %02x subsys %04x:%04x "
                "(%u interrupt vectors)",
                id.vendor_id, id.device_id, id.class_code, id.revision,
                id.subsystem_vendor_id, id.subsystem_id, id.msix_vectors);
    return true;
}

static void nvkvm_realize(PCIDevice *pci, Error **errp)
{
    NvkvmState *s = NVKVM(pci);

    /*
     * ★ The accelerator refusal, first and loud.  The global-lock opt-out is honoured only on
     * the accelerator's dispatch path, so an interpreted machine runs this device's handlers
     * under the hypervisor's global lock on EVERY access — which is not a slow mode to offer
     * quietly, it is the amplification the threading design exists to prevent, and it is
     * invisible from inside.  Refused rather than degraded.
     */
    if (!kvm_enabled()) {
        error_setg(errp,
                   "nvkvm: this device requires the hardware accelerator (-accel kvm). "
                   "Without it every access to this device is dispatched under the "
                   "hypervisor's global lock, which is not a slower mode of the same thing.");
        return;
    }

    if (kayfabe_shim_abi_version() != KAYFABE_SHIM_ABI) {
        error_setg(errp,
                   "nvkvm: this shim speaks wire ABI %u and the archive it was linked "
                   "against speaks %u; they are from different builds",
                   KAYFABE_SHIM_ABI, kayfabe_shim_abi_version());
        return;
    }

    /*
     * ═══ ★★★ THE IDENTITY, AND WHY IT IS NOT A LIE ══════════════════════════════════
     *
     * This device used to present neutral identifiers, on the argument that "this device
     * emulates no vendor's silicon and claiming one would make a guest driver bind to
     * something that cannot serve it".  The second half of that has been measured and it is
     * backwards: a stock NVIDIA driver's own table matches vendor 0x10DE with a display
     * class and the module UNLOADS ITSELF when nothing matches, so a neutral identity does
     * not produce a driver that binds cautiously — it produces one that never binds at all,
     * and there is no force-bind fallback to fall back to.
     *
     * We are emulating an NVIDIA GPU.  Saying so is the accurate statement, and the identity
     * comes from the same table row the ROM this device serves is generated from, so the
     * two cannot disagree.
     */
    if (!nvkvm_identity_realize(s, errp)) {
        return;
    }

    if (!nvkvm_bars_realize(s, errp)) {
        return;
    }
    if (!nvkvm_regions_selfcheck(s, errp)) {
        return;
    }

    /*
     * ★★ The interrupt capability, and it is the CAPABILITY that is load-bearing here.
     *
     * `nv_pci_probe` aborts when the device has neither an interrupt line nor a
     * message-signalled capability, and treats that as an error on an accelerated machine.
     * Delivery is a later stage (see nvkvm_trap_write's named refusal); a capability the
     * guest can find is what lets its driver get far enough to need one.
     *
     * ★ Refusal is recorded, not fatal.  A device that fails to realize because the
     * hypervisor would not give it a vector table tells an operator nothing about the
     * emulation, and the register plane is independently useful.
     */
    if (s->msix_vectors > 0) {
        Error *local = NULL;
        int i;

        if (msix_init(pci, s->msix_vectors,
                      &s->mr[NVKVM_MSIX_ROW], nvkvm_regions[NVKVM_MSIX_ROW].pci_bar, 0x0,
                      &s->mr[NVKVM_MSIX_ROW], nvkvm_regions[NVKVM_MSIX_ROW].pci_bar, 0x800,
                      0x00, &local) < 0) {
            s->msix_refused = true;
            warn_report("nvkvm: this machine refused a %u-vector message-signalled table "
                        "(%s); the device still enumerates, but a guest driver that requires "
                        "one will refuse to bind",
                        s->msix_vectors, error_get_pretty(local));
            error_free(local);
        } else {
            for (i = 0; i < (int)s->msix_vectors; i++) {
                msix_vector_use(pci, i);
            }
        }
    }

    /*
     * ═══ ★★★ THE REGISTER PLANE ═════════════════════════════════════════════════════
     *
     * Built HERE, and not from the configuration-space write path the memory plane uses,
     * because it needs no base-address register: a guest driver's first act is to read
     * chip-identity registers and the answer is a function of the chip table alone.  Two
     * planes, two lifetimes — see kayfabe_shim.h.
     */
    {
        const uint8_t *msg = NULL;
        uint64_t msg_len = 0;
        void *handle = NULL;
        const char *probe = s->probe_arm_notifier ? s->probe_arm_notifier : "";
        int32_t rc = kayfabe_shim_regs_create((uint16_t)s->chip_device_id,
                                              (const uint8_t *)probe,
                                              (uint64_t)strlen(probe),
                                              &handle, &msg, &msg_len);

        if (rc != KAYFABE_OK) {
            error_setg(errp, "nvkvm: the register plane refused to build (%d): %.*s",
                       (int)rc, (int)msg_len, (const char *)msg);
            return;
        }
        s->regs = handle;
    }
    s->exit_notifier.notify = nvkvm_exit_notify;
    qemu_add_exit_notifier(&s->exit_notifier);

    s->traps_open = true;
}

static void nvkvm_exit(PCIDevice *pci)
{
    NvkvmState *s = NVKVM(pci);

    if (s->listening) {
        memory_listener_unregister(&s->listener);
        s->listening = false;
    }
    /* ★★★ STAGE Q5, IN ORDER.  The register plane's guest-RAM port holds a handle onto the
     * memory plane, and the register surface keeps answering across this teardown BY
     * DESIGN — so the port must be withdrawn BEFORE the plane it points into is unrealized.
     * Afterwards the emulated GSP refuses every guest-memory access by name, which is the
     * honest answer for a device whose memory plane is gone. */
    if (s->regs && s->regs_have_ram) {
        kayfabe_shim_regs_detach_ram(s->regs);
        s->regs_have_ram = false;
    }
    /* The archive withdraws the blocker and re-enables discard through the primitives, so the
     * two arms below are backstops for the case where it never realized at all. */
    if (s->shim) {
        kayfabe_shim_unrealize(s->shim);
        s->shim = NULL;
    }
    /* ★ The register plane is destroyed AFTER the listener is unregistered and the memory
     * plane is gone, because a topology callback still in flight can reach neither. */
    nvkvm_report_registers(s);
    if (s->regs) {
        kayfabe_shim_regs_destroy(s->regs);
        s->regs = NULL;
    }
    if (msix_present(pci)) {
        msix_uninit(pci, &s->mr[NVKVM_MSIX_ROW], &s->mr[NVKVM_MSIX_ROW]);
    }
    if (s->discard_disabled) {
        ram_block_discard_disable(false);
        s->discard_disabled = false;
    }
    if (s->migrate_blocker) {
        migrate_del_blocker(&s->migrate_blocker);
    }
}



/*
 * Three-phase reset, latch-and-defer.  `enter` must not have a side effect on any other
 * object, so it does the one thing that cannot: it bumps an epoch.  `hold` stops accepting
 * traps.  `exit` re-opens them and does nothing else — reclamation belongs to a worker, not to
 * a phase that runs with the hypervisor's global lock held.
 */
static void nvkvm_reset_enter(Object *obj, ResetType type)
{
    NvkvmState *s = NVKVM(obj);

    (void)type;
    s->reset_epoch++;
}

static void nvkvm_reset_hold(Object *obj, ResetType type)
{
    NvkvmState *s = NVKVM(obj);

    (void)type;
    s->traps_open = false;
    /*
     * ★★ The emulated GSP's state machine goes back to cold here, and this is the one
     * thing the C artifact could not do: its WPR2 latch only cleared on a full hypervisor
     * restart, which is where the bench's "each clean run needs a fresh boot" tax comes
     * from.  The archive rebuilds the value behind its own lock, so a guest reboot is a
     * reboot rather than a restart.  Empty-handle safe: this phase also runs before the
     * device has realized.
     */
    kayfabe_shim_regs_reset(s->regs);
}

static void nvkvm_reset_exit(Object *obj, ResetType type)
{
    NvkvmState *s = NVKVM(obj);

    (void)type;
    s->traps_open = true;
}

static const Property nvkvm_properties[] = {
    /* ★★ All three DEFAULTS are the GA106 row's own numbers, and all three are CHECKED
     * against the chip table at realize (`nvkvm_apply_identity`).  A default that merely
     * looked plausible would realize a device whose registered apertures disagree with
     * what the emulated GSP tells the guest's resource manager, and neither side logs it.
     * These are the chip's facts spelled a second time so an operator can see them; the
     * chip row is authoritative and a mismatch is refused, never clamped. */
    DEFINE_PROP_UINT64("bar0-size", NvkvmState, bar0_size, 16 * MiB),
    DEFINE_PROP_UINT64("bar1-size", NvkvmState, bar1_size, 256 * MiB),
    DEFINE_PROP_UINT64("bar2-size", NvkvmState, bar2_size, 32 * MiB),
    /* 0 = install no reservation at realize.  Non-zero installs one at the base of the first
     * reservation register, which is what an acceptance test drives. */
    DEFINE_PROP_UINT64("window-size", NvkvmState, window_size, 0),
    DEFINE_PROP_BOOL("shareable-ram", NvkvmState, shareable_ram, true),
    /* One page holds a 256-entry vector table at 0x0 and its pending bits at 0x800, which
     * is far more than any chip row asks for. */
    DEFINE_PROP_UINT64("msix-size", NvkvmState, msix_size, 4 * KiB),
    /* 0 = the archive's chip table picks its default row.  A PCI device id selects another,
     * and an id the table does not carry is a named refusal at realize — there is
     * deliberately no nearest-neighbour fallback. */
    DEFINE_PROP_UINT32("chip-device-id", NvkvmState, chip_device_id, 0),
    /* ★ PROBE ONLY — reachability instrumentation, never a shipping path.  Unset = empty
     * = every non-silent notifier arming is refused, which is the product behaviour.
     * This is a DEVICE PROPERTY rather than an env var so the boot's own census can
     * report the set it actually ran with: three boots ran probe-off while looking
     * armed from the launching shell, and their conclusions had to be retracted. */
    DEFINE_PROP_STRING("probe-arm-notifier", NvkvmState, probe_arm_notifier),
    NVKVM_PROP_TERMINATOR
};

static void nvkvm_class_init(ObjectClass *klass, NVKVM_CLASS_DATA *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    PCIDeviceClass *k = PCI_DEVICE_CLASS(klass);
    ResettableClass *rc = RESETTABLE_CLASS(klass);

    (void)data;

    k->realize      = nvkvm_realize;
    k->exit         = nvkvm_exit;
    k->config_write = nvkvm_config_write;
    /*
     * ★ These are PLACEHOLDERS and nothing binds to them.  The class is instantiated before
     * any instance exists, so it cannot ask the archive which chip THIS device is — that is
     * a per-instance property.  `nvkvm_identity_realize` overwrites all four from the chip
     * table at realize, and refuses rather than proceeding if it cannot, so a device that
     * still reports these has failed to realize and is not on a bus.
     */
    k->vendor_id    = PCI_VENDOR_ID_QEMU;
    k->device_id    = 0x11ea;
    k->revision     = 1;
    k->class_id     = PCI_CLASS_OTHERS;

    rc->phases.enter = nvkvm_reset_enter;
    rc->phases.hold  = nvkvm_reset_hold;
    rc->phases.exit  = nvkvm_reset_exit;

    dc->desc = "kayfabe emulated GPU (memory plane + register plane)";
    device_class_set_props(dc, nvkvm_properties);
    set_bit(DEVICE_CATEGORY_MISC, dc->categories);
}

static const TypeInfo nvkvm_type_info = {
    .name          = TYPE_NVKVM,
    .parent        = TYPE_PCI_DEVICE,
    .instance_size = sizeof(NvkvmState),
    .class_init    = nvkvm_class_init,
    .interfaces    = (InterfaceInfo[]) {
        { INTERFACE_CONVENTIONAL_PCI_DEVICE },
        { },
    },
};

static void nvkvm_register_types(void)
{
    type_register_static(&nvkvm_type_info);
}

type_init(nvkvm_register_types)
