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
#include "hw/qdev-properties.h"
#include "hw/resettable.h"
#include "migration/blocker.h"
#include "qapi/error.h"
#include "qemu/error-report.h"
#include "qemu/module.h"
#include "qemu/range.h"
#include "qemu/units.h"
#include "qom/object.h"

#include "kayfabe_shim.h"

#define TYPE_NVKVM "nvkvm-gpu"
OBJECT_DECLARE_SIMPLE_TYPE(NvkvmState, NVKVM)

/* Kept in step with `nvkvm_regions` by a build-time assertion in nvkvm_bars_realize. */
#define NVKVM_N_REGIONS 3

typedef enum NvkvmRegionKind {
    /* Accesses trap to this device. */
    NVKVM_KIND_TRAP = 0,
    /* ★ A pure-MMIO reservation the archive shadows with its own slots.  Its callbacks are
     * not reached in normal operation; if one fires, the shadow is missing. */
    NVKVM_KIND_RESERVATION = 1,
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
    uint64_t window_size;
    bool     shareable_ram;

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

    void    *shim;         /* the archive's handle, or NULL */
    bool     shim_refused; /* refused once, loudly; never retried */
    uint64_t reset_epoch;
    bool     traps_open;

    uint64_t trap_reads;
    uint64_t trap_writes;
    uint64_t reservation_touches;
};

/* ===================================================================================
 * Region callbacks
 * =================================================================================== */

static uint64_t nvkvm_trap_read(void *opaque, hwaddr addr, unsigned size)
{
    NvkvmState *s = opaque;

    (void)addr;
    (void)size;
    s->trap_reads++;
    /*
     * ★ Nothing routes an access into the core at this stage, and this returns a constant
     * rather than a plausible register value on purpose: a guest driver must fail to
     * recognise this device rather than half-recognise it.  Wiring the dispatch is a separate
     * stage with its own acceptance.
     */
    return 0;
}

static void nvkvm_trap_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    NvkvmState *s = opaque;

    (void)addr;
    (void)val;
    (void)size;
    s->trap_writes++;
}

static const MemoryRegionOps nvkvm_trap_ops = {
    .read       = nvkvm_trap_read,
    .write      = nvkvm_trap_write,
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

/* ★★ THE table.  Complete, literal, and the only place a region is named. */
static const NvkvmRegionSpec nvkvm_regions[NVKVM_N_REGIONS] = {
    { "nvkvm-bar0-regs",   0, 0, NVKVM_KIND_TRAP,
      offsetof(NvkvmState, bar0_size), &nvkvm_trap_ops },
    { "nvkvm-bar1-window", 1, 2, NVKVM_KIND_RESERVATION,
      offsetof(NvkvmState, bar1_size), &nvkvm_reservation_ops },
    { "nvkvm-bar2-window", 2, 4, NVKVM_KIND_RESERVATION,
      offsetof(NvkvmState, bar2_size), &nvkvm_reservation_ops },
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

        nvkvm_region_init_io(s, &s->mr[i], row->ops, row->name, size);
        pci_register_bar(pci, row->pci_bar,
                         PCI_BASE_ADDRESS_SPACE_MEMORY |
                         PCI_BASE_ADDRESS_MEM_TYPE_64 |
                         PCI_BASE_ADDRESS_MEM_PREFETCH,
                         &s->mr[i]);
    }
    return true;
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

    if (s->io_inits != NVKVM_N_REGIONS) {
        error_setg(errp,
                   "nvkvm: the region table has %u rows but the constructor ran %u times; "
                   "a region was built somewhere other than nvkvm_region_init_io, which is "
                   "exactly the omission the table exists to make impossible",
                   NVKVM_N_REGIONS, s->io_inits);
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
        if (nvkvm_regions[i].pci_bar + 2 > PCI_NUM_REGIONS) {
            error_setg(errp, "nvkvm: %s is registered past the last base-address register",
                       nvkvm_regions[i].name);
            return false;
        }
        if (i > 0 && nvkvm_regions[i].pci_bar < nvkvm_regions[i - 1].pci_bar + 2) {
            error_setg(errp,
                       "nvkvm: %s is at base-address register %d and %s is at %d; these are "
                       "64-bit registers, so each consumes TWO and they overlap. PCI accepts "
                       "this silently and the device then reports two registers at one "
                       "guest-physical base",
                       nvkvm_regions[i - 1].name, nvkvm_regions[i - 1].pci_bar,
                       nvkvm_regions[i].name, nvkvm_regions[i].pci_bar);
            return false;
        }
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
        if (!s->mr[i].lockless_io) {
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
    unsigned i;
    int32_t rc;

    for (i = 0; i < NVKVM_N_REGIONS; i++) {
        bars[i].index    = nvkvm_regions[i].port_index;
        bars[i].reserved = 0;
        bars[i].base     = (uint64_t)pci->io_regions[nvkvm_regions[i].pci_bar].addr;
        bars[i].len      = nvkvm_row_size(s, &nvkvm_regions[i]);
    }
    cfg.abi_version   = KAYFABE_SHIM_ABI;
    cfg.struct_size   = (uint32_t)sizeof(cfg);
    cfg.shareable_ram = s->shareable_ram ? 1 : 0;
    cfg.n_bars        = NVKVM_N_REGIONS;
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

    info_report("nvkvm: memory plane realized (bar0=0x%" PRIx64 " bar1=0x%" PRIx64
                " bar2=0x%" PRIx64 ")",
                bars[0].base, bars[1].base, bars[2].base);
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
        pcibus_t addr = pci->io_regions[nvkvm_regions[i].pci_bar].addr;

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

    if (!nvkvm_bars_realize(s, errp)) {
        return;
    }
    if (!nvkvm_regions_selfcheck(s, errp)) {
        return;
    }
    s->traps_open = true;
}

static void nvkvm_exit(PCIDevice *pci)
{
    NvkvmState *s = NVKVM(pci);

    if (s->listening) {
        memory_listener_unregister(&s->listener);
        s->listening = false;
    }
    /* The archive withdraws the blocker and re-enables discard through the primitives, so the
     * two arms below are backstops for the case where it never realized at all. */
    if (s->shim) {
        kayfabe_shim_unrealize(s->shim);
        s->shim = NULL;
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
}

static void nvkvm_reset_exit(Object *obj, ResetType type)
{
    NvkvmState *s = NVKVM(obj);

    (void)type;
    s->traps_open = true;
}

static const Property nvkvm_properties[] = {
    DEFINE_PROP_UINT64("bar0-size", NvkvmState, bar0_size, 16 * MiB),
    DEFINE_PROP_UINT64("bar1-size", NvkvmState, bar1_size, 4 * GiB),
    DEFINE_PROP_UINT64("bar2-size", NvkvmState, bar2_size, 1 * GiB),
    /* 0 = install no reservation at realize.  Non-zero installs one at the base of the first
     * reservation register, which is what an acceptance test drives. */
    DEFINE_PROP_UINT64("window-size", NvkvmState, window_size, 0),
    DEFINE_PROP_BOOL("shareable-ram", NvkvmState, shareable_ram, true),
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
    /* Deliberately neutral identifiers: this device emulates no vendor's silicon and claiming
     * one would make a guest driver bind to something that cannot serve it. */
    k->vendor_id    = PCI_VENDOR_ID_QEMU;
    k->device_id    = 0x11ea;
    k->revision     = 1;
    k->class_id     = PCI_CLASS_OTHERS;

    rc->phases.enter = nvkvm_reset_enter;
    rc->phases.hold  = nvkvm_reset_hold;
    rc->phases.exit  = nvkvm_reset_exit;

    dc->desc = "kayfabe accelerator memory plane";
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
