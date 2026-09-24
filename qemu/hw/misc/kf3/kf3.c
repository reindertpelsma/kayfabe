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
 *    The only trapped piece left in BAR1 is the Hopper+ doorbell page (counted until P5).
 *  - MSI-X lives in its own BAR; interrupts arrive with P5 (irqfd, never a BQL-taking notify).
 */
#include "qemu/osdep.h"
#include "hw/pci/pci.h"
#include "hw/pci/pci_device.h"
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
#include "kf3.h"

#if QEMU_VERSION_MAJOR < 10 || (QEMU_VERSION_MAJOR == 10 && QEMU_VERSION_MINOR < 2)
#error "kf3-gpu requires QEMU 10.2+ (memory_region_enable_lockless_io keeps the vCPU path BQL-free)"
#endif

#define TYPE_KF3 "kf3-gpu"
OBJECT_DECLARE_SIMPLE_TYPE(Kf3State, KF3)

#define KF3_MAX_PIECES 64
#define KF3_MSIX_BAR 5

typedef struct Kf3Piece {
    struct Kf3State *s;
    uint64_t base;
    MemoryRegion mr;
} Kf3Piece;

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
    MemoryListener listener;
};

/* ── BAR0 ───────────────────────────────────────────────────────────────────────────────── */

static uint64_t kf3_piece_read(void *opaque, hwaddr addr, unsigned size)
{
    /* A ROM device in romd mode serves reads from its RAM; this is reached only for a HOLE piece
     * (a PIO data port whose read must exit — none on Turing..Ada). */
    return 0;
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

static bool kf3_host_rom(Kf3State *s, Kf3Piece *p, const char *name, uint64_t len, Error **errp)
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
    p->mr.ops = &kf3_piece_ops;
    p->mr.opaque = p;
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

/* ── BAR1 / BAR2 ──────────────────────────────────────────────────────────────────────────
 * Plain RAM (Rust's windows). The ops below serve only a TRAPPED page (the Hopper+ BAR1
 * doorbell), counted until P5 wires it. */

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
    pci_register_bar(pci, 1, PCI_BASE_ADDRESS_SPACE_MEMORY | PCI_BASE_ADDRESS_MEM_TYPE_64 |
                     PCI_BASE_ADDRESS_MEM_PREFETCH, &s->bar1);
    pci_register_bar(pci, 3, PCI_BASE_ADDRESS_SPACE_MEMORY | PCI_BASE_ADDRESS_MEM_TYPE_64 |
                     PCI_BASE_ADDRESS_MEM_PREFETCH, &s->bar2);

    if (s->msix_vectors > 0) {
        memory_region_init(&s->msix_bar, OBJECT(s), "kf3-msix", 0x4000);
        if (msix_init(pci, s->msix_vectors, &s->msix_bar, KF3_MSIX_BAR, 0x0, &s->msix_bar,
                      KF3_MSIX_BAR, 0x2000, 0, errp) < 0) {
            return;
        }
        pci_register_bar(pci, KF3_MSIX_BAR, PCI_BASE_ADDRESS_SPACE_MEMORY, &s->msix_bar);
        for (unsigned i = 0; i < s->msix_vectors; i++) {
            msix_vector_use(pci, i);
        }
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
        info_report("%s bar12_reads=%" PRIu64 " bar12_writes=%" PRIu64, st,
                    qatomic_read(&s->bar12_reads), qatomic_read(&s->bar12_writes));
        memory_listener_unregister(&s->listener);
        kf3_unrealize(s->h);
    }
    if (s->msix_vectors > 0) {
        msix_uninit(pci, &s->msix_bar, &s->msix_bar);
    }
}

static const Property kf3_properties[] = {
    DEFINE_PROP_UINT32("gpu-minor", Kf3State, gpu_minor, 0),
    DEFINE_PROP_UINT64("fb-mb", Kf3State, fb_mb, 8192),
    DEFINE_PROP_UINT64("bar1-size", Kf3State, bar1_size, 256 * MiB),
    DEFINE_PROP_UINT64("bar2-size", Kf3State, bar2_size, 32 * MiB),
    DEFINE_PROP_UINT32("msix-vectors", Kf3State, msix_vectors, 32),
    DEFINE_PROP_STRING("guest-driver", Kf3State, guest_driver),
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
