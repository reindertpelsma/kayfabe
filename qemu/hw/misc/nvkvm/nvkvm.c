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
#include "qemu/module.h"
#include "qemu/range.h"
#include "qemu/units.h"
#include "qom/object.h"
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

struct NvkvmState {
    PCIDevice parent_obj;

    /* --- properties ------------------------------------------------------------- */
    uint64_t bar0_size;
    uint64_t bar1_size;
    uint64_t bar2_size;
    uint64_t msix_size;
    uint64_t window_size;
    bool     shareable_ram;
    /* 0 = the chip table's default row.  A hex PCI device id selects another. */
    uint32_t chip_device_id;

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
    uint64_t reservation_touches;
    uint64_t irq_requests_dropped;
};

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
    if (w.raise_status_irq) {
        s->irq_requests_dropped++;
        if (!s->irq_refusal_reported) {
            s->irq_refusal_reported = true;
            /*
             * ★★ A NAMED REFUSAL, not silence, and the distinction is the whole point.
             *
             * The interrupt CAPABILITY is real — this device offers a message-signalled
             * vector table and a guest driver's probe-time gate is satisfied by it.
             * DELIVERY is not wired: raising a vector from here would call the
             * hypervisor's notify path underneath a region this device has taken the
             * global-lock opt-out on, and that inversion is invisible to every gate in
             * this tree.  A guest that blocks on an event we never deliver hangs, so the
             * one thing this must not do is hang QUIETLY.
             */
            warn_report("nvkvm: the emulated GSP asked for its status-queue interrupt and "
                        "this device does not deliver vectors yet. The capability exists so "
                        "the guest's driver can bind; delivery is a later stage. A guest "
                        "that waits on this event will not be woken by us.");
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
 * supposed to be reached, backed by nothing, counted as `reservation_touches`.  The
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

static uint64_t nvkvm_reservation_read(void *opaque, hwaddr addr, unsigned size)
{
    NvkvmState *s = opaque;

    (void)addr;
    (void)size;
    /* Reached only where the archive's slot does not cover the range — an observe-tier hole,
     * or a reservation that was never installed.  Counted so "the shadow is missing" is a
     * number rather than a suspicion. */
    s->reservation_touches++;
    return 0;
}

static void nvkvm_reservation_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    NvkvmState *s = opaque;

    (void)addr;
    (void)val;
    (void)size;
    s->reservation_touches++;
}

static const MemoryRegionOps nvkvm_reservation_ops = {
    .read       = nvkvm_reservation_read,
    .write      = nvkvm_reservation_write,
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
    { "nvkvm-bar1-window", 1, 1, NVKVM_KIND_RESERVATION, true,
      offsetof(NvkvmState, bar1_size), &nvkvm_reservation_ops },
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

static int32_t nvkvm_op_signal_msix(void *dev, uint16_t vector)
{
    (void)dev;
    (void)vector;
    /* Not wired at this stage.  A named refusal is a smaller lie than a function that returns
     * success without raising anything. */
    return KAYFABE_E_UNSUPPORTED;
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

    if (s->window_size != 0) {
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
                " writes), fb refusals %" PRIu64 ", translated-window drops %" PRIu64
                "r/%" PRIu64 "w, resident %" PRIu64 " bytes",
                a.fb_reads, a.fb_writes, a.bar0_window_reads, a.bar0_window_writes,
                a.fb_refusals, a.fb_window_reads, a.fb_window_writes, a.fb_resident_bytes);

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

            /* %.*s, never %s: the tag is NUL-PADDED and a name that exactly fills the
             * array carries no terminator. */
            info_report("nvkvm:   bridge refusal %.*s x%" PRIu64,
                        len, (const char *)r->tag, r->count);
        }
    }
}

static void nvkvm_exit_notify(Notifier *n, void *data)
{
    NvkvmState *s = container_of(n, NvkvmState, exit_notifier);

    (void)data;
    nvkvm_report_registers(s);
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
        int32_t rc = kayfabe_shim_regs_create((uint16_t)s->chip_device_id, &handle,
                                              &msg, &msg_len);

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
