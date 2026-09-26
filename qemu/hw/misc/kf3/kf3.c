/*
 * kf3-gpu — the v3 kayfabe device (THE_ARCHITECTURE_v3.md §1-§2, docs/design/V3_BUILD.md).
 *
 * The C half is deliberately small: present the PCI function, build BAR0 from the Rust memory map,
 * route BAR0 WRITES to kf3_bar0_write, and tell Rust where guest RAM lives. Everything that decides
 * anything is in crates/kf-qemu (libkf_qemu.a).
 *
 *  - BAR0 reads NEVER exit: shadow pieces are ROM devices (reads from RAM Rust fills and the
 *    register drainer re-publishes; writes trap). Registered lockless (QEMU >= 10.2), so a trapped
 *    write reaches Rust without the BQL.
 *  - PRAMIN, BAR1 and BAR3 (RM's BAR2) are disposition-A RAM (P4): each is ONE host range Rust
 *    created and never unmaps (kf3_bar_ram), registered as a ram_device region — one memslot, no
 *    exit either way. What each page shows (a store view, guest RAM, or per-BAR scratch — never a
 *    hole) is re-pointed inside it by Rust with mmap(MAP_FIXED); QEMU never learns of a re-point.
 *    ⊘ No BAR1 page traps at setup. Hopper+ BAR1 usermode (doorbell) views are placed where the
 *    guest's own BAR1 PTEs put them: Rust's VA thread asks for a write-trapped overlay at that
 *    offset (kf3_bar1_overlay, a main-loop bottom half) before the guest's invalidate clears
 *    (docs/design/V3_BAR1_DOORBELL.md).
 *  - MSI-X lives in its own BAR. Interrupts (P5, V3_P5_PORT_MAP.md §2.7): Rust owns one eventfd
 *    per vector (kf3_irq_fd); this device registers each as a KVM irqfd on the vector's MSI route
 *    when the guest unmasks it (msix vector notifiers, virtio-pci's pattern). A raise is then one
 *    write(2) from any Rust thread — never a BQL-taking msix_notify.
 */
#include "qemu/osdep.h"
#include "hw/pci/pci.h"
#include "hw/pci/pci_device.h"
#include "hw/pci/msi.h"
#include "hw/pci/msix.h"
#include "hw/qdev-properties.h"
#include "qapi/error.h"
#include "qemu/error-report.h"
#include "qemu/module.h"
#include "qemu/units.h"
#include "system/ram_addr.h"
#include "qom/object.h"
#include "system/memory.h"
#include "system/address-spaces.h"
#include "system/kvm.h"
#include "qemu/event_notifier.h"
#include "qemu/main-loop.h"
#include "qemu/thread.h"
#include "kf3.h"

#if QEMU_VERSION_MAJOR < 10 || (QEMU_VERSION_MAJOR == 10 && QEMU_VERSION_MINOR < 2)
#error "kf3-gpu requires QEMU 10.2+ (memory_region_enable_lockless_io keeps the vCPU path BQL-free)"
#endif

#define TYPE_KF3 "kf3-gpu"
OBJECT_DECLARE_SIMPLE_TYPE(Kf3State, KF3)

#define KF3_MAX_PIECES 64
#define KF3_MSIX_BAR 5
#define KF3_MAX_VECTORS 32
/* Hopper+ BAR1 usermode-view overlays: the `bar1-overlays` property's default
 * (crates/kf-qemu mem.rs BAR1_OVERLAY_SLOTS). */
#define KF3_BAR1_OVERLAYS_DEFAULT 64

typedef struct Kf3Vec {
    EventNotifier e;   /* wraps the Rust-owned eventfd (never closed here) */
    int virq;          /* the KVM MSI route while the vector is in use, else -1 */
    bool have;
} Kf3Vec;

typedef struct Kf3Piece {
    struct Kf3State *s;
    uint64_t base;
    MemoryRegion mr;
} Kf3Piece;

/* One BAR1 usermode-view overlay: an ALIAS of the 64 KiB usermode ROM device, re-pointed and
 * (un)mapped at runtime. ⊘ Created once at realize and never destroyed (docs/devel/memory.rst:
 * do not create or destroy regions during a device's lifetime) — only added/removed. */
typedef struct Kf3Bar1Ov {
    MemoryRegion alias;
    bool live;
    uint64_t base, len, vf_rel;
} Kf3Bar1Ov;

struct Kf3State {
    PCIDevice parent_obj;
    /* properties */
    uint32_t gpu_minor;
    uint64_t fb_mb;
    uint64_t bar1_size;
    uint64_t bar2_size;
    uint32_t msix_vectors;
    char *guest_driver;
    /* state */
    void *h;
    MemoryRegion bar0;
    Kf3Piece pieces[KF3_MAX_PIECES];
    unsigned n_pieces;
    MemoryRegion bar1, bar2, msix_bar;
    Kf3Piece bar_pieces[2][4];
    unsigned n_bar_pieces[2];
    uint64_t bar12_reads, bar12_writes;
    /* Hopper+: the usermode page as a ROM device (reads = the host's live usermode window, writes
     * trap lockless to kf3_bar1_usermode_write), and the alias pool that places it in BAR1. */
    Kf3Piece bar1_um;
    uint64_t bar1_um_len;
    uint32_t bar1_overlays;          /* property: the alias pool's size */
    Kf3Bar1Ov *bar1_ov;              /* bar1_overlays slots, allocated once at realize */
    QemuMutex bar1_q_lock;           /* guards bar1_q only; never held across anything else */
    GQueue bar1_q;                   /* Kf3OvReq, FIFO: applied in submission order */
    QEMUBH *bar1_bh;
    uint64_t bar1_ov_applied, bar1_ov_failed;
    MemoryListener listener;
    Kf3Vec vec[KF3_MAX_VECTORS];
    uint64_t irq_routes, irq_route_fail;
    /* w827 trap bench (property dummy-bar, default off): two do-nothing MMIO pages in the MSI-X
     * BAR (every other BAR index is taken: BAR1/BAR3 are 64-bit and consume 2 and 4). */
    bool dummy_bar;
    MemoryRegion dummy_pages[3];
    EventNotifier dummy_efd;   /* page 2's KVM ioeventfd — nobody reads it */
};

/* ── BAR0 ───────────────────────────────────────────────────────────────────────────────── */

static uint64_t kf3_piece_read(void *opaque, hwaddr addr, unsigned size)
{
    /* A ROM device in romd mode serves reads from its RAM; this is reached only for a HOLE piece
     * (a register whose READ has a side effect — none on Turing..Ada). ★ w828: Rust serves it —
     * the Hopper+ memop token registers, and every other register on the page from its shadow. */
    Kf3Piece *p = opaque;
    return kf3_bar0_read(p->s->h, p->base + addr, size);
}

static void kf3_piece_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    Kf3Piece *p = opaque;
    kf3_bar0_write(p->s->h, p->base + addr, val, size);
}

static const MemoryRegionOps kf3_piece_ops = {
    .read = kf3_piece_read,
    .write = kf3_piece_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 8 },
    .impl = { .min_access_size = 1, .max_access_size = 8 },
};

/* A ROM device over pages we did not allocate: memory_region_init_rom_device_nomigrate, with the
 * RAM block taken from the host usermode window instead of qemu_ram_alloc. The window outlives
 * the region (the Rust device lives for the process). */
static void kf3_host_rom_free(MemoryRegion *mr)
{
    /* The RAM block only: its pages are the host window (RAM_PREALLOC), never freed here. */
    qemu_ram_free(mr->ram_block);
}

static bool kf3_host_rom_ops(Kf3State *s, Kf3Piece *p, const char *name, uint64_t len,
                             const MemoryRegionOps *ops, void *opaque, Error **errp)
{
    void *host = NULL;
    uint64_t have = 0;
    Error *err = NULL;

    if (kf3_usermode_view(s->h, &host, &have) != 0 || !host || have < len) {
        error_setg(errp, "kf3: no host usermode window for the %" PRIu64 "-byte passthrough page",
                   len);
        return false;
    }
    memory_region_init(&p->mr, OBJECT(s), name, len);
    p->mr.ops = ops;
    p->mr.opaque = opaque;
    p->mr.terminates = true;
    p->mr.rom_device = true;
    p->mr.destructor = kf3_host_rom_free;
    p->mr.ram_block = qemu_ram_alloc_from_ptr(len, host, &p->mr, &err);
    if (err) {
        error_propagate(errp, err);
        return false;
    }
    return true;
}

static bool kf3_host_rom(Kf3State *s, Kf3Piece *p, const char *name, uint64_t len, Error **errp)
{
    return kf3_host_rom_ops(s, p, name, len, &kf3_piece_ops, p, errp);
}

static bool kf3_bar0_build(Kf3State *s, uint64_t size, Error **errp)
{
    Kf3Region regs[KF3_MAX_PIECES * 2];
    int64_t n = kf3_memory_map(s->h, s->bar1_size, s->bar2_size, regs, G_N_ELEMENTS(regs));
    int64_t i;

    if (n < 0 || n > (int64_t)G_N_ELEMENTS(regs)) {
        error_setg(errp, "kf3: the memory map has %" PRId64 " regions (capacity %zu)", n,
                   G_N_ELEMENTS(regs));
        return false;
    }
    memory_region_init(&s->bar0, OBJECT(s), "kf3-bar0", size);
    for (i = 0; i < n; i++) {
        Kf3Region *r = &regs[i];
        Kf3Piece *p;
        g_autofree char *name = NULL;

        if (r->bar != 0) {
            continue;
        }
        if (s->n_pieces >= KF3_MAX_PIECES) {
            error_setg(errp, "kf3: more than %d BAR0 pieces", KF3_MAX_PIECES);
            return false;
        }
        p = &s->pieces[s->n_pieces++];
        p->s = s;
        p->base = r->base;
        name = g_strdup_printf("kf3-bar0@%" PRIx64 "/how%u", r->base, r->how);
        if (r->how == 3) {
            /* HOLE: a read that must exit (Hopper+ FSP EMEM). Trapping IO both ways. */
            memory_region_init_io(&p->mr, OBJECT(s), &kf3_piece_ops, p, name, r->len);
        } else if (r->how == 2) {
            /* HOST PASSTHROUGH (§53.1 C): a ROM device whose RAM IS the host's usermode window —
             * reads hit the live microsecond counter with no exit, writes (the doorbell) trap.
             * ⊘ A static shadow here froze the timer and every RM timeout spun forever. */
            if (!kf3_host_rom(s, p, name, r->len, errp)) {
                return false;
            }
        } else if (r->how == 0) {
            /* PLAIN RAM (§53.1 A) — PRAMIN: Rust's window, re-pointed inside the trapped
             * window-base write. ram_device: KVM maps it, and kf3_is_guest_ram skips it. */
            void *ptr = NULL;
            if (kf3_bar_ram(s->h, 0, r->base, r->len, &ptr) != 0 || !ptr) {
                error_setg(errp, "kf3: no host window for the plain-RAM piece at 0x%" PRIx64, r->base);
                return false;
            }
            memory_region_init_ram_device_ptr(&p->mr, OBJECT(s), name, r->len, ptr);
            memory_region_add_subregion(&s->bar0, r->base, &p->mr);
            continue;
        } else {
            /* Shadow (1): reads from RAM with no exit, writes trap. */
            if (!memory_region_init_rom_device_nomigrate(&p->mr, OBJECT(s), &kf3_piece_ops, p,
                                                         name, r->len, errp)) {
                return false;
            }
            if (kf3_shadow_attach(s->h, r->base, memory_region_get_ram_ptr(&p->mr), r->len) != 0) {
                error_setg(errp, "kf3: Rust refused the shadow piece at 0x%" PRIx64, r->base);
                return false;
            }
            memory_region_set_dirty(&p->mr, 0, r->len);
        }
        memory_region_enable_lockless_io(&p->mr);
        memory_region_add_subregion(&s->bar0, r->base, &p->mr);
    }
    kf3_shadow_seal(s->h);
    return true;
}

/* ── w827 trap bench: the most dummy trap possible ──────────────────────────────────────────
 * Page 0 (MSI-X BAR + KF3_DUMMY_OFF) is lockless, page 1 (+ 0x1000) takes the BQL; both reads
 * return 0 and both writes do nothing — no Rust call, no atomics, nothing. The guest times them
 * next to our own trapped BAR0 writes: dummy ≈ ours means the cost is the exit (nesting); dummy
 * << ours means it is our path or a lock. */
#define KF3_DUMMY_OFF 0x8000

static uint64_t kf3_dummy_read(void *opaque, hwaddr addr, unsigned size)
{
    return 0;
}

static void kf3_dummy_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
}

static const MemoryRegionOps kf3_dummy_ops = {
    .read = kf3_dummy_read,
    .write = kf3_dummy_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 8 },
    .impl = { .min_access_size = 1, .max_access_size = 8 },
};

/* ── BAR1 / BAR2 ──────────────────────────────────────────────────────────────────────────
 * Plain RAM (Rust's windows). The ops below would serve a trapped piece; the memory map has none
 * for BAR1/BAR2 on any family (the Hopper+ doorbell views are runtime overlays, below). */

static uint64_t kf3_bar12_read(void *opaque, hwaddr addr, unsigned size)
{
    Kf3State *s = opaque;
    qatomic_inc(&s->bar12_reads);
    return 0;
}

static void kf3_bar12_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    Kf3State *s = opaque;
    qatomic_inc(&s->bar12_writes);
}

static const MemoryRegionOps kf3_bar12_ops = {
    .read = kf3_bar12_read,
    .write = kf3_bar12_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 8 },
    .impl = { .min_access_size = 1, .max_access_size = 8 },
};

/* Build BAR `bar` (1, or 2 = PCI BAR3) from the Rust memory map: plain-RAM pieces over Rust's
 * window, a trapped piece (Hopper+ BAR1 doorbell page) as counted IO. */
static bool kf3_bar_build(Kf3State *s, unsigned bar, MemoryRegion *mr, uint64_t size, Error **errp)
{
    Kf3Region regs[KF3_MAX_PIECES * 2];
    int64_t n = kf3_memory_map(s->h, s->bar1_size, s->bar2_size, regs, G_N_ELEMENTS(regs));
    int64_t i;
    unsigned k = bar - 1;

    memory_region_init(mr, OBJECT(s), bar == 1 ? "kf3-bar1" : "kf3-bar2", size);
    for (i = 0; i < n && i < (int64_t)G_N_ELEMENTS(regs); i++) {
        Kf3Region *r = &regs[i];
        Kf3Piece *p;
        g_autofree char *name = NULL;
        if (r->bar != bar) {
            continue;
        }
        if (s->n_bar_pieces[k] >= G_N_ELEMENTS(s->bar_pieces[k])) {
            error_setg(errp, "kf3: BAR%u has more than %zu pieces", bar, G_N_ELEMENTS(s->bar_pieces[k]));
            return false;
        }
        p = &s->bar_pieces[k][s->n_bar_pieces[k]++];
        p->s = s;
        p->base = r->base;
        name = g_strdup_printf("kf3-bar%u@%" PRIx64 "/how%u", bar, r->base, r->how);
        if (r->how == 0) {
            void *ptr = NULL;
            if (kf3_bar_ram(s->h, bar, r->base, r->len, &ptr) != 0 || !ptr) {
                error_setg(errp, "kf3: no host window for BAR%u 0x%" PRIx64 "+0x%" PRIx64, bar, r->base, r->len);
                return false;
            }
            memory_region_init_ram_device_ptr(&p->mr, OBJECT(s), name, r->len, ptr);
        } else {
            memory_region_init_io(&p->mr, OBJECT(s), &kf3_bar12_ops, s, name, r->len);
        }
        memory_region_add_subregion(mr, r->base, &p->mr);
    }
    return true;
}

/* ── Hopper+ BAR1 usermode views (docs/design/V3_BAR1_DOORBELL.md §4) ─────────────────────
 * The guest RM maps the usermode page into BAR1 wherever ITS BAR1 allocator chooses (bBar1Mapping,
 * usermode_api.c:94-98 → kbusMapFbAperture_GM107). Rust learns the place from the walked BAR1 PTE
 * (SYS_COH + SMSKED_MESSAGE kind, address = the VF register offset) and calls kf3_bar1_overlay from
 * its VA thread; the change is made by a main-loop bottom half (the BQL owner) and the VA thread
 * waits for it, bounded, so the guest's BAR1 invalidate clears only once KVM traps the view. */

static uint64_t kf3_bar1_um_read(void *opaque, hwaddr addr, unsigned size)
{
    return 0; /* romd: reads are served from the host window's RAM and never reach here */
}

static void kf3_bar1_um_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    Kf3State *s = opaque;
    /* addr is the offset inside the usermode page: the alias carries the view's vf_rel. */
    kf3_bar1_usermode_write(s->h, addr, val, size);
}

static const MemoryRegionOps kf3_bar1_um_ops = {
    .read = kf3_bar1_um_read,
    .write = kf3_bar1_um_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 8 },
    .impl = { .min_access_size = 1, .max_access_size = 8 },
};

typedef struct Kf3OvReq {
    uint64_t seq;
    uint32_t op;
    uint64_t base, len, vf_rel;
} Kf3OvReq;

/* Main loop, BQL held. Idempotent both ways. */
static int32_t kf3_ov_apply(Kf3State *s, uint32_t op, uint64_t base, uint64_t len, uint64_t vf_rel)
{
    Kf3Bar1Ov *hit = NULL, *free_slot = NULL;
    unsigned i;

    for (i = 0; i < s->bar1_overlays; i++) {
        Kf3Bar1Ov *o = &s->bar1_ov[i];
        if (o->live && o->base == base) {
            hit = o;
        } else if (!o->live && !free_slot) {
            free_slot = o;
        }
    }
    if (op == 0) {
        if (hit) {
            memory_region_del_subregion(&s->bar1, &hit->alias);
            hit->live = false;
        }
        return 0;
    }
    if (hit) {
        return (hit->len == len && hit->vf_rel == vf_rel) ? 0 : -EEXIST;
    }
    if (!free_slot) {
        return -ENOSPC;
    }
    if (len == 0 || vf_rel + len > s->bar1_um_len || base + len > s->bar1_size ||
        ((base | len | vf_rel) & 0xfff)) {
        return -EINVAL;
    }
    memory_region_transaction_begin();
    memory_region_set_alias_offset(&free_slot->alias, vf_rel);
    memory_region_set_size(&free_slot->alias, len);
    /* Priority 1: above the BAR1 window's plain-RAM piece, which keeps its own pages beneath. */
    memory_region_add_subregion_overlap(&s->bar1, base, &free_slot->alias, 1);
    memory_region_transaction_commit();
    free_slot->live = true;
    free_slot->base = base;
    free_slot->len = len;
    free_slot->vf_rel = vf_rel;
    return 0;
}

/* Main loop, BQL held: drain the queue in FIFO order (an unmap queued before a re-map of the same
 * BAR1 VA is applied first), then post each result back to Rust, which wakes its VA thread. */
static void kf3_ov_bh(void *opaque)
{
    Kf3State *s = opaque;
    for (;;) {
        Kf3OvReq *r;
        int32_t rc;
        qemu_mutex_lock(&s->bar1_q_lock);
        r = g_queue_pop_head(&s->bar1_q);
        qemu_mutex_unlock(&s->bar1_q_lock);
        if (!r) {
            return;
        }
        rc = kf3_ov_apply(s, r->op, r->base, r->len, r->vf_rel);
        if (rc == 0) {
            s->bar1_ov_applied++;
        } else {
            s->bar1_ov_failed++;
        }
        kf3_bar1_overlay_done(s->h, r->seq, rc);
        g_free(r);
    }
}

/* Rust's OverlayFn — the VA-manager thread, never a vCPU. ⊘ NEVER WAITS (ruling 2026-09-26 (5)):
 * a thread blocking on the BQL holder is the deadlock class the blocking invariants forbid. It
 * queues and schedules the bottom half; Rust holds only that invalidate's clear until
 * kf3_bar1_overlay_done reports the change. */
static int32_t kf3_bar1_overlay(void *opaque, uint64_t seq, uint32_t op, uint64_t base, uint64_t len,
                                uint64_t vf_rel)
{
    Kf3State *s = opaque;
    Kf3OvReq *r = g_new0(Kf3OvReq, 1);

    r->seq = seq;
    r->op = op;
    r->base = base;
    r->len = len;
    r->vf_rel = vf_rel;
    qemu_mutex_lock(&s->bar1_q_lock);
    g_queue_push_tail(&s->bar1_q, r);
    qemu_mutex_unlock(&s->bar1_q_lock);
    qemu_bh_schedule(s->bar1_bh);
    return 0;
}

static bool kf3_bar1_views_build(Kf3State *s, Error **errp)
{
    void *host = NULL;
    uint64_t have = 0;
    unsigned i;

    if (kf3_bar1_follows_guest(s->h) != 1) {
        return true; /* Turing … Ada: no BAR1 usermode view exists */
    }
    if (kf3_usermode_view(s->h, &host, &have) != 0 || have < 0x10000) {
        error_setg(errp, "kf3: the host usermode window is %" PRIu64 " bytes; a BAR1 view needs 64 KiB", have);
        return false;
    }
    s->bar1_um_len = 0x10000;
    s->bar1_um.s = s;
    if (!kf3_host_rom_ops(s, &s->bar1_um, "kf3-bar1-usermode", s->bar1_um_len, &kf3_bar1_um_ops, s, errp)) {
        return false;
    }
    memory_region_enable_lockless_io(&s->bar1_um.mr);
    if (s->bar1_overlays == 0) {
        error_setg(errp, "kf3: bar1-overlays must be at least 1 on a family with BAR1 doorbell views");
        return false;
    }
    /* Allocated once and never freed: the device lives for the process, and its regions may not
     * be destroyed during its lifetime (docs/devel/memory.rst). */
    s->bar1_ov = g_new0(Kf3Bar1Ov, s->bar1_overlays);
    for (i = 0; i < s->bar1_overlays; i++) {
        g_autofree char *name = g_strdup_printf("kf3-bar1-doorbell-view%u", i);
        memory_region_init_alias(&s->bar1_ov[i].alias, OBJECT(s), name, &s->bar1_um.mr, 0, 0x1000);
    }
    qemu_mutex_init(&s->bar1_q_lock);
    g_queue_init(&s->bar1_q);
    s->bar1_bh = qemu_bh_new(kf3_ov_bh, s);
    if (kf3_set_bar1_overlay(s->h, kf3_bar1_overlay, s, s->bar1_overlays) != 0) {
        error_setg(errp, "kf3: Rust refused the BAR1 overlay verb");
        return false;
    }
    return true;
}

/* ── guest RAM → Rust ──────────────────────────────────────────────────────────────────── */

static bool kf3_is_guest_ram(MemoryRegionSection *sec)
{
    return memory_region_is_ram(sec->mr) && !memory_region_is_rom(sec->mr) &&
           !memory_region_is_ram_device(sec->mr) && sec->mr->ram_block != NULL;
}

static void kf3_region_add(MemoryListener *l, MemoryRegionSection *sec)
{
    Kf3State *s = container_of(l, Kf3State, listener);
    uint8_t *hva;
    if (!kf3_is_guest_ram(sec)) {
        return;
    }
    hva = (uint8_t *)memory_region_get_ram_ptr(sec->mr) + sec->offset_within_region;
    /* The backend's fd and the file offset of this section (memory-backend-memfd): Rust maps
     * guest RAM from it into a CPU window when a walked leaf or the PRAMIN target is sysmem. */
    kf3_ram_add(s->h, sec->offset_within_address_space, hva, int128_get64(sec->size),
                memory_region_get_fd(sec->mr),
                qemu_ram_get_fd_offset(sec->mr->ram_block) + sec->offset_within_region);
}

static void kf3_region_del(MemoryListener *l, MemoryRegionSection *sec)
{
    Kf3State *s = container_of(l, Kf3State, listener);
    if (kf3_is_guest_ram(sec)) {
        kf3_ram_del(s->h, sec->offset_within_address_space);
    }
}

/* ── MSI-X → KVM irqfd ───────────────────────────────────────────────────────────────────── */

static int kf3_vector_use(PCIDevice *pci, unsigned v, MSIMessage msg)
{
    Kf3State *s = KF3(pci);
    Kf3Vec *x;
    int virq;

    if (v >= KF3_MAX_VECTORS || !s->vec[v].have) {
        return 0;
    }
    x = &s->vec[v];
    if (x->virq >= 0) {
        if (kvm_irqchip_update_msi_route(kvm_state, x->virq, msg, pci) < 0) {
            s->irq_route_fail++;
            return -1;
        }
        kvm_irqchip_commit_routes(kvm_state);
        return 0;
    }
    {
        KVMRouteChange c = kvm_irqchip_begin_route_changes(kvm_state);
        virq = kvm_irqchip_add_msi_route(&c, v, pci);
        if (virq < 0) {
            s->irq_route_fail++;
            return virq;
        }
        kvm_irqchip_commit_route_changes(&c);
    }
    /* A raise that arrived while the vector was masked is still counted in the eventfd; KVM
     * injects it on assignment (irqfd polls the eventfd when it is registered). */
    if (kvm_irqchip_add_irqfd_notifier_gsi(kvm_state, &x->e, NULL, virq) < 0) {
        kvm_irqchip_release_virq(kvm_state, virq);
        s->irq_route_fail++;
        return -1;
    }
    x->virq = virq;
    s->irq_routes++;
    return 0;
}

static void kf3_vector_release(PCIDevice *pci, unsigned v)
{
    Kf3State *s = KF3(pci);
    Kf3Vec *x;

    if (v >= KF3_MAX_VECTORS) {
        return;
    }
    x = &s->vec[v];
    if (x->virq >= 0) {
        kvm_irqchip_remove_irqfd_notifier_gsi(kvm_state, &x->e, x->virq);
        kvm_irqchip_release_virq(kvm_state, x->virq);
        x->virq = -1;
    }
}

/* ── realize / exit ─────────────────────────────────────────────────────────────────────── */

static void kf3_dev_realize(PCIDevice *pci, Error **errp)
{
    Kf3State *s = KF3(pci);
    Kf3Identity id;
    char err[512] = "";
    uint8_t *c = pci->config;

    if (!kvm_enabled()) {
        error_setg(errp, "kf3: requires -accel kvm");
        return;
    }
    if (kf3_abi_version() != KF3_ABI) {
        error_setg(errp, "kf3: archive ABI %u, device ABI %u", kf3_abi_version(), KF3_ABI);
        return;
    }
    if (kf3_realize(s->gpu_minor, s->fb_mb, s->bar1_size, s->bar2_size, s->guest_driver, &s->h, err, sizeof(err)) != 0) {
        error_setg(errp, "kf3: realize refused: %s", err);
        return;
    }
    if (kf3_identity(s->h, &id) != 0) {
        error_setg(errp, "kf3: no identity");
        return;
    }
    /* The host's own identity, so the guest's stock driver binds. */
    pci_config_set_vendor_id(c, id.vendor);
    pci_config_set_device_id(c, id.device);
    pci_config_set_revision(c, id.revision);
    pci_config_set_class(c, (uint16_t)(id.class_code >> 8));
    c[PCI_CLASS_PROG] = (uint8_t)id.class_code;
    pci_set_word(c + PCI_SUBSYSTEM_VENDOR_ID, id.subsystem_vendor);
    pci_set_word(c + PCI_SUBSYSTEM_ID, id.subsystem);
    c[PCI_INTERRUPT_PIN] = 1;

    if (!kf3_bar0_build(s, id.bar0_bytes, errp)) {
        return;
    }
    pci_register_bar(pci, 0, PCI_BASE_ADDRESS_SPACE_MEMORY, &s->bar0);

    if (!kf3_bar_build(s, 1, &s->bar1, s->bar1_size, errp) ||
        !kf3_bar_build(s, 2, &s->bar2, s->bar2_size, errp)) {
        return;
    }
    if (!kf3_bar1_views_build(s, errp)) {
        return;
    }
    pci_register_bar(pci, 1, PCI_BASE_ADDRESS_SPACE_MEMORY | PCI_BASE_ADDRESS_MEM_TYPE_64 |
                     PCI_BASE_ADDRESS_MEM_PREFETCH, &s->bar1);
    pci_register_bar(pci, 3, PCI_BASE_ADDRESS_SPACE_MEMORY | PCI_BASE_ADDRESS_MEM_TYPE_64 |
                     PCI_BASE_ADDRESS_MEM_PREFETCH, &s->bar2);

    if (s->msix_vectors > 0) {
        memory_region_init(&s->msix_bar, OBJECT(s), "kf3-msix", s->dummy_bar ? 0x10000 : 0x4000);
        if (s->dummy_bar) {
            memory_region_init_io(&s->dummy_pages[0], OBJECT(s), &kf3_dummy_ops, s, "kf3-dummy-lockless", 0x1000);
            memory_region_enable_lockless_io(&s->dummy_pages[0]);
            memory_region_add_subregion(&s->msix_bar, KF3_DUMMY_OFF, &s->dummy_pages[0]);
            memory_region_init_io(&s->dummy_pages[1], OBJECT(s), &kf3_dummy_ops, s, "kf3-dummy-bql", 0x1000);
            memory_region_add_subregion(&s->msix_bar, KF3_DUMMY_OFF + 0x1000, &s->dummy_pages[1]);
            /* Page 2: a write at +0 is a KVM ioeventfd (handled in the host kernel, no exit to
             * QEMU) — the floor an in-kernel doorbell could reach on this host. */
            memory_region_init_io(&s->dummy_pages[2], OBJECT(s), &kf3_dummy_ops, s, "kf3-dummy-ioeventfd", 0x1000);
            memory_region_enable_lockless_io(&s->dummy_pages[2]);
            if (event_notifier_init(&s->dummy_efd, 0) == 0) {
                memory_region_add_eventfd(&s->dummy_pages[2], 0, 4, false, 0, &s->dummy_efd);
            }
            memory_region_add_subregion(&s->msix_bar, KF3_DUMMY_OFF + 0x2000, &s->dummy_pages[2]);
        }
        if (msix_init(pci, s->msix_vectors, &s->msix_bar, KF3_MSIX_BAR, 0x0, &s->msix_bar,
                      KF3_MSIX_BAR, 0x2000, 0, errp) < 0) {
            return;
        }
        pci_register_bar(pci, KF3_MSIX_BAR, PCI_BASE_ADDRESS_SPACE_MEMORY, &s->msix_bar);
        for (unsigned i = 0; i < s->msix_vectors; i++) {
            msix_vector_use(pci, i);
        }
        /* ⊘ No irqfd, no interrupts: refuse rather than fall back to a BQL-taking notify. */
        if (!kvm_msi_via_irqfd_enabled()) {
            error_setg(errp, "kf3: KVM MSI-via-irqfd is not available (kernel irqchip required)");
            return;
        }
        for (unsigned i = 0; i < s->msix_vectors && i < KF3_MAX_VECTORS; i++) {
            int fd = kf3_irq_fd(s->h, i);
            s->vec[i].virq = -1;
            if (fd >= 0) {
                event_notifier_init_fd(&s->vec[i].e, fd);
                s->vec[i].have = true;
            }
        }
        if (msix_set_vector_notifiers(pci, kf3_vector_use, kf3_vector_release, NULL) < 0) {
            error_setg(errp, "kf3: MSI-X vector notifiers refused");
            return;
        }
    }

    /* ★ ABI 7: config-space words the guest's RM reads by CONFIG CYCLE (Hopper+ read the PCIe
     * link capabilities there, not through the BAR0 XVE mirror). Read-only (no wmask), and refused
     * by name if a capability already owns the bytes. */
    for (uint32_t i = 0;; i++) {
        uint16_t off;
        uint32_t val;
        if (kf3_config_word(s->h, i, &off, &val) != 0) {
            break;
        }
        if (off < PCI_CONFIG_HEADER_SIZE || off + 4 > PCI_CONFIG_SPACE_SIZE || (off & 3) != 0 ||
            pci->used[off] || pci->used[off + 1] || pci->used[off + 2] || pci->used[off + 3]) {
            error_setg(errp, "kf3: config word %u at 0x%x collides with the header or a capability", i, off);
            return;
        }
        pci_set_long(c + off, val);
    }

    s->listener = (MemoryListener){
        .name = "kf3-guest-ram",
        .region_add = kf3_region_add,
        .region_del = kf3_region_del,
        .priority = MEMORY_LISTENER_PRIORITY_MIN,
    };
    memory_listener_register(&s->listener, &address_space_memory);

    kf3_status(s->h, err, sizeof(err));
    info_report("%s (BAR0 pieces=%u)", err, s->n_pieces);
}

static void kf3_dev_exit(PCIDevice *pci)
{
    Kf3State *s = KF3(pci);
    char st[512] = "";
    if (s->h) {
        kf3_status(s->h, st, sizeof(st));
        info_report("%s bar12_reads=%" PRIu64 " bar12_writes=%" PRIu64 " irq_routes=%" PRIu64
                    " irq_route_fail=%" PRIu64 " bar1_ov_applied=%" PRIu64 " bar1_ov_failed=%" PRIu64, st,
                    qatomic_read(&s->bar12_reads), qatomic_read(&s->bar12_writes),
                    s->irq_routes, s->irq_route_fail, s->bar1_ov_applied, s->bar1_ov_failed);
        memory_listener_unregister(&s->listener);
        kf3_unrealize(s->h);
    }
    if (s->msix_vectors > 0) {
        msix_unset_vector_notifiers(pci);
        msix_uninit(pci, &s->msix_bar, &s->msix_bar);
    }
}

static const Property kf3_properties[] = {
    DEFINE_PROP_UINT32("gpu-minor", Kf3State, gpu_minor, 0),
    DEFINE_PROP_UINT64("fb-mb", Kf3State, fb_mb, 8192),
    DEFINE_PROP_UINT64("bar1-size", Kf3State, bar1_size, 256 * MiB),
    /* Hopper+ only: how many BAR1 doorbell views can be trapped at once (~1 per CUDA process). */
    DEFINE_PROP_UINT32("bar1-overlays", Kf3State, bar1_overlays, KF3_BAR1_OVERLAYS_DEFAULT),
    DEFINE_PROP_UINT64("bar2-size", Kf3State, bar2_size, 32 * MiB),
    DEFINE_PROP_UINT32("msix-vectors", Kf3State, msix_vectors, 32),
    DEFINE_PROP_STRING("guest-driver", Kf3State, guest_driver),
    DEFINE_PROP_BOOL("dummy-bar", Kf3State, dummy_bar, false),
};

static void kf3_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    PCIDeviceClass *k = PCI_DEVICE_CLASS(klass);
    k->realize = kf3_dev_realize;
    k->exit = kf3_dev_exit;
    k->vendor_id = 0x10de;
    k->device_id = 0xffff;   /* replaced at realize with the host's own */
    k->class_id = PCI_CLASS_DISPLAY_VGA;
    device_class_set_props(dc, kf3_properties);
    set_bit(DEVICE_CATEGORY_DISPLAY, dc->categories);
    dc->desc = "kayfabe v3 GPU (stock NVIDIA driver on a real host GPU)";
}

static const TypeInfo kf3_type_info = {
    .name = TYPE_KF3,
    .parent = TYPE_PCI_DEVICE,
    .instance_size = sizeof(Kf3State),
    .class_init = kf3_class_init,
    .interfaces = (const InterfaceInfo[]){ { INTERFACE_CONVENTIONAL_PCI_DEVICE }, { } },
};

static void kf3_register_types(void)
{
    type_register_static(&kf3_type_info);
}

type_init(kf3_register_types)
