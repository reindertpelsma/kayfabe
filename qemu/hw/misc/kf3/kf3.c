/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
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
 *    exit either way. What each page shows (a store view, guest RAM, or the window's scratch — never
 *    a hole) is re-pointed inside it by Rust with mmap(MAP_FIXED); QEMU never learns of a re-point.
 *    Scratch is one small memfd tile per window, repeated (2026-10-03, V3_P4_PORT_MAP.md Q3), so a
 *    guest touching every unmapped page costs the host at most one tile per window.
 *    ⊘ No BAR1 page traps at setup. Hopper+ BAR1 usermode (doorbell) views are placed where the
 *    guest's own BAR1 PTEs put them: Rust's VA thread asks for a write-trapped overlay at that
 *    offset (kf3_bar1_overlay, a main-loop bottom half) before the guest's invalidate clears
 *    (docs/design/V3_BAR1_DOORBELL.md).
 *  - ★ 2026-09-30, doorbell fast path (property doorbell-ioeventfd, default off;
 *    docs/design/V3_DOORBELL_IOEVENTFD.md): Rust registers one KVM ioeventfd (DATAMATCH = the
 *    guest's token) per live channel at every place the doorbell register is mapped, through
 *    kf3_ioeventfd below; its register drainer services the eventfds. Unmatched values still trap.
 *  - ★ 2026-10-03, EXPERIMENT x11-dispsw (property x11-dispsw, default off; option A, OWNER_RULINGS.md §N;
 *    docs/design/V3_DISPLAY.md, the x11-dispsw note): handed to Rust at realize (needs display=on).
 *    Everything it changes is Rust's: the guest's GF100_DISP_SW objects are twinned on the host.
 *  - MSI-X lives in its own BAR. Interrupts (P5, V3_P5_PORT_MAP.md §2.7): Rust owns one eventfd
 *    per vector (kf3_irq_fd); this device registers each as a KVM irqfd on the vector's MSI route
 *    when the guest unmasks it (msix vector notifiers, virtio-pci's pattern). A raise is then one
 *    write(2) from any Rust thread — never a BQL-taking msix_notify.
 *  - ★ 2026-10-03, the GPU-copy broker rung (property display-broker-vram=auto|on|off, ABI 12;
 *    docs/design/V3_DISPLAY.md sec. 8.11): Rust allocates kayfabe's own VRAM frame slots and hands
 *    the broker their dma-bufs; this file only passes the mode.
 *  - ★ 2026-10-03, the display broker (property display-broker, unset = off;
 *    docs/design/V3_DISPLAY.md §8): Rust (crates/kf-broker, crates/kf-qemu/src/broker.rs) owns the
 *    socket and decides everything; this file registers the fd handlers and the timer Rust asks
 *    for, and injects the broker's input through QEMU's input layer on kf3's own console.
 *    ⊘ 2026-10-08 (ABI 23, OWNER_RULINGS.md §V, §8.20): the input and cursor POLICY (ported from
 *    nvkvm-pv src/qemu/nvkvm_display_relay.c at 368d2db — relay_btn, relay_set_relative,
 *    relay_handle) moved to kf-broker behind a VMM-neutral trait; this file keeps only the verbs
 *    (Kf3InputOps), one QEMU 10.2 call each. Main loop only, BQL held; no vCPU path.
 *  - ★ 2026-10-03, the boot display (property gop, default off; docs/design/V3_DISPLAY.md §4.11):
 *    the PCI expansion ROM — the constant kf-gop UEFI driver embedded in Rust, wrapped at realize
 *    with this device's ids and its framebuffer descriptor (kf3_option_rom) — registered as the ROM
 *    BAR in the shape of pci_add_option_rom (hw/pci/pci.c). Read-only RAM: reads never exit, a write
 *    exits to QEMU core and is dropped there. Everything else gop=on changes is Rust's.
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
#include "qemu/host-utils.h"
#include "system/ram_addr.h"
#include "qom/object.h"
#include "system/memory.h"
#include "system/address-spaces.h"
#include "system/kvm.h"
#include "hw/core/cpu.h"   /* current_cpu, CPUState::kvm_fd (the DBCPL diagnostic) */
#include "qemu/atomic.h"
#include <sys/ioctl.h>
#include "system/system.h"   /* qemu_uuid, qemu_uuid_set (ABI 22: the VM identity for gpu-uuid=auto) */
#include "qemu/uuid.h"
#include "qemu/event_notifier.h"
#include "qemu/main-loop.h"
#include "block/aio.h"
#include "qemu/thread.h"
#include "ui/console.h"
#include "ui/input.h"
#include "qapi/qapi-commands-ui.h"
#include "qemu/timer.h"
#include "system/runstate.h"
#include <linux/kvm.h>
#include "kf3.h"
#include "kf3_gop.h"
/* ★ ABI 24, DIAGNOSTIC (KF3_BAR0_READ_TRACE and x-gsp-observer, both default off; OWNER_RULINGS.md
 * sec. X, docs/design/V3_BAR0_TRACE_MODE.md): the VFIO reference's OWN tracer — QEMU's vfio trace
 * events (the same generated code, backend and timestamps as vfio-pci's) and the shared GSP observer
 * (tools/vfio-gsp-observer, installed in hw/vfio by its apply.py; KF3_GSP_OBSERVER is defined by
 * meson.build only when the tree has it). */
#include "trace/trace-hw_vfio.h"
#ifdef KF3_GSP_OBSERVER
#include "../../vfio/gsp-observer.h"
#endif

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
    struct Kf3State *s; /* ★ ABI 24 trace mode only: the main-loop MSI path's owner and vector */
    unsigned nr;
} Kf3Vec;

typedef struct Kf3Piece {
    struct Kf3State *s;
    uint64_t base;
    MemoryRegion mr;
    bool shadow;   /* a shadow ROM device (how 1): the only pieces the KF3_BAR0_TRACE trap toggles */
    bool rtrace;   /* ★ ABI 24 trace mode: this shadow piece's reads exit for the whole run */
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
    /* v3-display (docs/design/V3_DISPLAY.md): the virtual NVDisplay; off = the displayless posture */
    bool display;
    /* ★ ABI 11 (docs/design/V3_DISPLAY.md §4.11): the boot display — the option ROM here, the BAR1
     * seed, the boot layer and fn 65's console region in Rust. Needs display=on. Off = today. */
    bool gop;
    /* ★ EXPERIMENT x11-dispsw (2026-10-03, default off; option A, OWNER_RULINGS.md §N; V3_DISPLAY.md): twin
     * the guest's GF100_DISP_SW objects on the host with authored params, or refuse them by name. Rust's. */
    bool x11_dispsw;
    char *gop_efi;  /* ★ ABI 19: a signed copy of the embedded GOP driver (OWNER_RULINGS §K) */
    /* ★ ABI 22 (2026-10-08): the per-VM GPU UUID. gpu_uuid = auto (default) | random | host |
     * GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx; vm_id overrides QEMU's -uuid as the VM identity that
     * auto hashes. Rust parses, validates and refuses by name (crates/kf-rm/src/gpuuid.rs). */
    char *gpu_uuid;
    char *vm_id;
    uint32_t channel_budget; /* ABI 26: channel-budget property (0 = derive from the host) */
    /* ★ ABI 10 (v3-display2's 9, M2): the console the display's frames are shown on, and the frame
     * it shows. Main thread only (gfx_update, realize, exit). */
    QemuConsole *con;
    Kf3Frame shown;
    /* ★ ABI 10 (v3-ioeventfd's 9, docs/design/V3_DOORBELL_IOEVENTFD.md): the doorbell fast path —
     * one KVM ioeventfd per live guest token, serviced by the register drainer. Default OFF until
     * measured. Main loop only (realize, the memory listener, exit), BQL held. */
    bool db_ioeventfd;
    uint32_t db_ioeventfd_max;       /* placements (tokens x doorbell sites) held at once */
    MemoryRegion *um_mr;             /* BAR0's usermode piece (the host passthrough page) */
    int64_t db_page_off;             /* the doorbell's offset inside the usermode page */
    uint64_t db_sites_added, db_sites_removed;
    MemoryRegion dummy_pages[3];
    EventNotifier dummy_efd;   /* page 2's KVM ioeventfd — nobody reads it */
    /* ★ 2026-10-03: this device holds ram_block_discard_require(true) (see realize), so no device
     * that pins guest RAM for DMA can join the VM while it lives. Released at exit, or by
     * realize's `fail` path. */
    bool discard_required;
    /* ★ ABI 12 (display step 3): the display-broker relay. Main loop only, BQL held. */
    char *display_broker;            /* property: the broker's socket path; NULL = off */
    int64_t display_broker_uid;      /* property: one more uid accepted as the broker; -1 = none */
    char *display_broker_vram;       /* ★ ABI 12: "auto" (NULL), "on" or "off" — the GPU-copy rung */
    QEMUTimer *broker_timer;
    int broker_sock;                 /* the socket Rust asked us to watch, or -1 */
    int broker_frame_fd;             /* the display worker's frame eventfd, or -1 */
    /* ★ ABI 16 (docs/design/V3_DISPLAY.md sec. 8.16, OWNER_RULINGS sec. M): the cap on every head's
     * emulated vblank tick (whole Hz; 0 = unset: 75, today's EDID). Rust validates it by name. */
    uint32_t display_max_fps;
    /* ★ ABI 16: the console's on-demand refresh (gfx_update is asynchronous) — the descriptor the
     * display worker signals when a request was served, and a 1 s backstop so a screendump never
     * waits on a worker that cannot answer. Main loop only. */
    int refresh_fd;
    QEMUTimer *refresh_timer;
    /* ★ ABI 21, DIAGNOSTIC (KF3_BAR0_TRACE, default off; OWNER_RULINGS.md sec. S, 2026-10-07): the
     * BAR0 read trap. Rust's drainer stores the wish and schedules the bottom half; the bottom half
     * flips ROMD on the shadow pieces under the BQL. Never used with the flag off. */
    QEMUBH *read_trap_bh;
    bool read_trap_want;             /* qatomic: the drainer's latest wish */
    bool read_trap_on;               /* main loop only: what is applied */
    uint64_t read_trap_flips;
    /* ★ ABI 24, DIAGNOSTIC, default off (KF3_BAR0_READ_TRACE=1 and/or x-gsp-observer=PATH): the
     * VFIO reference's tracer on this device. tr_on = Rust's kf3_trace_mode (set once at realize);
     * diag = tr_on or the observer: BAR0 uses kf3_piece_ops_trace and MSI-X goes through the main
     * loop (vfio's x-no-kvm-msix shape) instead of KVM irqfds. Both false = today's device. */
    bool tr_on;
    bool diag;
    char tr_name[40];
    Notifier tr_exit;
    char *gsp_observer_path;         /* property x-gsp-observer (vfio-pci's name) */
    uint32_t gsp_observer_seconds;   /* property x-gsp-observer-seconds (vfio-pci: 300) */
#ifdef KF3_GSP_OBSERVER
    GspObserver *obs;
#endif
};

/* ── ★ ABI 24, DIAGNOSTIC: the BAR0 trace mode (both helpers are reached only with it on) ───── */

/* The shared GSP observer, BEFORE the access is served (vfio's order). Its producer is serialized
 * by the BQL; a lockless vCPU takes it only for the observer's few trigger offsets. */
static void kf3_obs_mmio(struct Kf3State *s, hwaddr off, uint64_t val, unsigned size, bool write)
{
#ifdef KF3_GSP_OBSERVER
    if (s->obs && gsp_observer_is_trigger(off, size, write)) {
        bool locked = bql_locked();
        if (!locked) {
            bql_lock();
        }
        gsp_observer_mmio(s->obs, off, val, size, write);
        if (!locked) {
            bql_unlock();
        }
    }
#endif
}

/* The record, AFTER the access is served (vfio region.c's order), through QEMU's own vfio trace
 * events; Rust admits it (selection, record and byte caps, drop counter). */
static void kf3_trace_rw(struct Kf3State *s, hwaddr off, uint64_t val, unsigned size, bool write)
{
    if (write) {
        if (trace_event_get_state_backends(TRACE_VFIO_REGION_WRITE) &&
            kf3_trace_admit(s->h, 1, off, size, val)) {
            trace_vfio_region_write(s->tr_name, 0, off, val, size);
        }
    } else if (trace_event_get_state_backends(TRACE_VFIO_REGION_READ) &&
               kf3_trace_admit(s->h, 0, off, size, val)) {
        trace_vfio_region_read(s->tr_name, 0, off, size, val);
    }
}

/* ── BAR0 ───────────────────────────────────────────────────────────────────────────────── */

static uint64_t kf3_piece_read(void *opaque, hwaddr addr, unsigned size)
{
    /* A ROM device in romd mode serves reads from its RAM; this is reached only for a HOLE piece
     * (a register whose READ has a side effect — none on Turing..Ada). ★ w828: Rust serves it —
     * the Hopper+ memop token registers, and every other register on the page from its shadow. */
    Kf3Piece *p = opaque;
    return kf3_bar0_read(p->s->h, p->base + addr, size);
}

/* ★ 2026-10-08 DIAGNOSTIC (KF3_DOORBELL_CPL=1, default off; Windows user-mode-submission question):
 * for the first 4096 writes to the usermode page's NOTIFY_CHANNEL_PENDING (BAR0 0x810090 on Turing,
 * 0xBB0090 on Ampere..Ada), log the doorbell value with the writing vCPU's privilege level (CS.RPL from
 * KVM_GET_SREGS) and RIP (KVM_GET_REGS): a ring 3 writer is guest userspace ringing its own channel.
 * Read-only ioctls on the trapping vCPU's own fd, on its own thread, between KVM_RUNs; nothing is
 * changed and nothing waits. */
static int kf3_dbcpl_left = -1;

static void kf3_dbcpl(hwaddr off, uint64_t val)
{
    if (kf3_dbcpl_left == -1) {
        const char *e = getenv("KF3_DOORBELL_CPL");
        kf3_dbcpl_left = (e && e[0] == '1') ? 4096 : 0;
    }
    /* the usermode/VF page: 0x810000 (Turing) or 0xBB0000 = NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET +
     * 0x30000 (Ampere+; kf_trap::memmap::VF_USERMODE_PAGE); the doorbell at +0x90 */
    if (kf3_dbcpl_left <= 0 || (off != 0x810090 && off != 0xBB0090)) {
        return;
    }
    if (qatomic_fetch_dec(&kf3_dbcpl_left) <= 0) {
        return;
    }
    CPUState *cs = current_cpu;
    struct kvm_sregs sr;
    struct kvm_regs r;
    int cpl = -1;
    unsigned long long rip = 0;
    if (cs && kvm_enabled() && ioctl(cs->kvm_fd, KVM_GET_SREGS, &sr) == 0) {
        cpl = sr.cs.selector & 3;
    }
    if (cs && kvm_enabled() && ioctl(cs->kvm_fd, KVM_GET_REGS, &r) == 0) {
        rip = r.rip;
    }
    fprintf(stderr, "kf3: DBCPL off=%#llx value=%#x cpl=%d rip=%#llx\n",
            (unsigned long long)off, (unsigned)val, cpl, rip);
}

static void kf3_piece_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    Kf3Piece *p = opaque;
    kf3_dbcpl(p->base + addr, val);
    kf3_bar0_write(p->s->h, p->base + addr, val, size);
}

static const MemoryRegionOps kf3_piece_ops = {
    .read = kf3_piece_read,
    .write = kf3_piece_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 8 },
    .impl = { .min_access_size = 1, .max_access_size = 8 },
};

/* ★ ABI 24, DIAGNOSTIC: BAR0's ops in trace mode ONLY — chosen once at realize (kf3_piece_ops_for),
 * so with the mode off the vCPU runs exactly kf3_piece_read/kf3_piece_write above. The answer is
 * the same call; the shared GSP observer sees the access first and QEMU's vfio trace event records
 * it after, as vfio-pci does. */
static uint64_t kf3_piece_read_trace(void *opaque, hwaddr addr, unsigned size)
{
    Kf3Piece *p = opaque;
    uint64_t v;
    kf3_obs_mmio(p->s, p->base + addr, 0, size, false);
    v = kf3_piece_read(opaque, addr, size);
    kf3_trace_rw(p->s, p->base + addr, v, size, false);
    return v;
}

static void kf3_piece_write_trace(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    Kf3Piece *p = opaque;
    kf3_obs_mmio(p->s, p->base + addr, val, size, true);
    kf3_piece_write(opaque, addr, val, size);
    kf3_trace_rw(p->s, p->base + addr, val, size, true);
}

static const MemoryRegionOps kf3_piece_ops_trace = {
    .read = kf3_piece_read_trace,
    .write = kf3_piece_write_trace,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 8 },
    .impl = { .min_access_size = 1, .max_access_size = 8 },
};

static const MemoryRegionOps *kf3_piece_ops_for(struct Kf3State *s)
{
    return s->diag ? &kf3_piece_ops_trace : &kf3_piece_ops;
}

/* A ROM device over pages we did not allocate: memory_region_init_rom_device_nomigrate, with the
 * RAM block taken from the host usermode window instead of qemu_ram_alloc. The window outlives
 * the region (the Rust device lives for the process). */
static void kf3_host_rom_free(MemoryRegion *mr)
{
    /* The RAM block only: its pages are the host window (RAM_PREALLOC), never freed here. */
    qemu_ram_free(mr->ram_block);
}

static bool kf3_host_rom_ops(Kf3State *s, Kf3Piece *p, const char *name, uint64_t len,
                             const MemoryRegionOps *ops, void *opaque, bool timer, Error **errp)
{
    void *host = NULL;
    uint64_t have = 0;
    Error *err = NULL;

    int rc = timer ? kf3_timer_view(s->h, &host, &have) : kf3_usermode_view(s->h, &host, &have);
    if (rc != 0 || !host || have < len) {
        error_setg(errp, "kf3: no host %s window for the %" PRIu64 "-byte passthrough page",
                   timer ? "timer" : "usermode", len);
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

static bool kf3_host_rom(Kf3State *s, Kf3Piece *p, const char *name, uint64_t len, bool timer, Error **errp)
{
    return kf3_host_rom_ops(s, p, name, len, kf3_piece_ops_for(s), p, timer, errp);
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
            memory_region_init_io(&p->mr, OBJECT(s), kf3_piece_ops_for(s), p, name, r->len);
        } else if (r->how == 2 || r->how == 4) {
            /* HOST PASSTHROUGH (§53.1 C): a ROM device whose RAM IS the host's usermode window —
             * reads hit the live microsecond counter with no exit, writes (the doorbell) trap.
             * ⊘ A static shadow here froze the timer and every RM timeout spun forever. */
            if (!kf3_host_rom(s, p, name, r->len, r->how == 4, errp)) {
                return false;
            }
            if (r->how == 2) {
                s->um_mr = &p->mr;
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
            if (!memory_region_init_rom_device_nomigrate(&p->mr, OBJECT(s), kf3_piece_ops_for(s), p,
                                                         name, r->len, errp)) {
                return false;
            }
            if (kf3_shadow_attach(s->h, r->base, memory_region_get_ram_ptr(&p->mr), r->len) != 0) {
                error_setg(errp, "kf3: Rust refused the shadow piece at 0x%" PRIx64, r->base);
                return false;
            }
            memory_region_set_dirty(&p->mr, 0, r->len);
            p->shadow = true;
            /* ★ ABI 24 trace mode only: a piece overlapping a selected read range serves its
             * reads by exit for the whole run (kf3_bar0_read answers from this same shadow). */
            if (s->tr_on && kf3_trace_piece(s->h, r->base, r->len)) {
                memory_region_rom_device_set_romd(&p->mr, false);
                p->rtrace = true;
            }
        }
        memory_region_enable_lockless_io(&p->mr);
        memory_region_add_subregion(&s->bar0, r->base, &p->mr);
    }
    kf3_shadow_seal(s->h);
    return true;
}

/* ── ★ ABI 21, DIAGNOSTIC: the BAR0 read trap (KF3_BAR0_TRACE, default off) ─────────────────
 * OWNER_RULINGS.md sec. S (2026-10-07): a scoped, bounded exception to "BAR0 reads never exit".
 * ROMD off on a shadow piece routes its reads to kf3_piece_read -> kf3_bar0_read, which answers
 * from the same shadow RAM ROMD reads; the answer does not change. Rust decides when (the window
 * after a guest-kernel channel's GPFIFO_SCHEDULE, capped) and records the accesses. */
static void kf3_read_trap_bh(void *opaque)
{
    Kf3State *s = opaque;
    bool on = qatomic_read(&s->read_trap_want);
    unsigned i;

    if (on == s->read_trap_on) {
        return;
    }
    memory_region_transaction_begin();
    for (i = 0; i < s->n_pieces; i++) {
        if (s->pieces[i].shadow) {
            /* a trace-mode piece (ABI 24) stays trapped whatever the window wants */
            memory_region_rom_device_set_romd(&s->pieces[i].mr, !(on || s->pieces[i].rtrace));
        }
    }
    memory_region_transaction_commit();
    s->read_trap_on = on;
    s->read_trap_flips++;
    info_report("kf3: BAR0-TRACE read trap %s (flip %" PRIu64 ")", on ? "ON" : "OFF", s->read_trap_flips);
}

/* Rust's ReadTrapFn — the register drainer, never a vCPU. Never waits. */
static void kf3_read_trap(void *opaque, uint32_t on)
{
    Kf3State *s = opaque;

    qatomic_set(&s->read_trap_want, on != 0);
    qemu_bh_schedule(s->read_trap_bh);
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
    if (!kf3_host_rom_ops(s, &s->bar1_um, "kf3-bar1-usermode", s->bar1_um_len, &kf3_bar1_um_ops, s, false, errp)) {
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

#include "kf3_guest_ram.h"

/* ── the doorbell fast path (docs/design/V3_DOORBELL_IOEVENTFD.md) ────────────────────────────
 * Rust keeps the registrations; this file only (a) issues KVM_IOEVENTFD for it — directly, so a
 * refusal is a returned errno the token survives on the trapped path, never QEMU's abort() in
 * kvm_mem_ioeventfd_add — and (b) reports where the doorbell register is mapped: BAR0's usermode
 * piece and every Hopper+ BAR1 view alias of the usermode page. A BAR move or a decode toggle is a
 * region_del then a region_add, so registrations follow the guest's own BAR programming. */

/* Any non-vCPU thread (the channel act thread, the main loop). kvm_vm_ioctl takes no QEMU lock. */
static int32_t kf3_ioeventfd(void *opaque, uint64_t gpa, uint32_t len, uint64_t datamatch, int32_t fd,
                             uint32_t assign)
{
    struct kvm_ioeventfd iofd = {
        .datamatch = datamatch,
        .addr = gpa,
        .len = len,
        .fd = fd,
        .flags = KVM_IOEVENTFD_FLAG_DATAMATCH | (assign ? 0 : KVM_IOEVENTFD_FLAG_DEASSIGN),
    };
    int r;

    (void)opaque;
    if (!kvm_enabled()) {
        return -ENOSYS;
    }
    r = kvm_vm_ioctl(kvm_state, KVM_IOEVENTFD, &iofd);
    return r < 0 ? r : 0;
}

/* Main loop (or a vCPU in a PCI config write), BQL held: does this section map the doorbell? */
static void kf3_db_section(Kf3State *s, MemoryRegionSection *sec, bool add)
{
    uint64_t lo, hi, gpa;

    if (!s->db_ioeventfd || s->db_page_off < 0 || !s->h) {
        return;
    }
    if (sec->mr != s->um_mr && !(s->bar1_um_len && sec->mr == &s->bar1_um.mr)) {
        return;
    }
    lo = sec->offset_within_region;
    hi = lo + int128_get64(sec->size);
    if ((uint64_t)s->db_page_off < lo || (uint64_t)s->db_page_off + 4 > hi) {
        return;
    }
    gpa = sec->offset_within_address_space + ((uint64_t)s->db_page_off - lo);
    kf3_doorbell_site(s->h, gpa, add ? 1 : 0);
    if (add) {
        s->db_sites_added++;
    } else {
        s->db_sites_removed++;
    }
}

static void kf3_region_add(MemoryListener *l, MemoryRegionSection *sec)
{
    Kf3State *s = container_of(l, Kf3State, listener);
    uint8_t *hva;
    kf3_db_section(s, sec, true);
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
    kf3_db_section(s, sec, false);
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

/* ★ ABI 24, DIAGNOSTIC (trace mode / x-gsp-observer only; never registered otherwise): MSI-X through
 * the main loop in vfio's x-no-kvm-msix shape (hw/vfio/pci.c vfio_msi_interrupt): the shared GSP
 * observer samples the status queue, QEMU's vfio_msi_interrupt event records the raise, then
 * msix_notify (which sets a masked vector's pending bit). Rust raises exactly as before: one
 * write(2) on the same eventfd. Main loop, BQL held. */
static void kf3_msi_user(void *opaque)
{
    Kf3Vec *x = opaque;
    struct Kf3State *s = x->s;
    PCIDevice *pci = PCI_DEVICE(s);

    if (!event_notifier_test_and_clear(&x->e)) {
        return;
    }
#ifdef KF3_GSP_OBSERVER
    if (s->obs) {
        gsp_observer_irq(s->obs);
    }
#endif
    if (s->tr_on && trace_event_get_state_backends(TRACE_VFIO_MSI_INTERRUPT)) {
        MSIMessage m = msix_get_message(pci, x->nr);
        if (kf3_trace_admit(s->h, 2, x->nr, m.data, m.address)) {
            trace_vfio_msi_interrupt(s->tr_name, x->nr, m.address, m.data);
        }
    }
    msix_notify(pci, x->nr);
}

static void kf3_msi_user_off(struct Kf3State *s)
{
    for (unsigned i = 0; i < KF3_MAX_VECTORS; i++) {
        if (s->vec[i].s) {
            qemu_set_fd_handler(event_notifier_get_fd(&s->vec[i].e), NULL, NULL, NULL);
            s->vec[i].s = NULL;
        }
    }
}

static void kf3_trace_exit(Notifier *n, void *data)
{
    struct Kf3State *s = container_of(n, struct Kf3State, tr_exit);
    char rep[1024] = "";
    kf3_trace_report(s->h, rep, sizeof(rep));
    info_report("%s", rep);
}

/* ── realize / exit ─────────────────────────────────────────────────────────────────────── */

/* ── the virtual display's console (M2, docs/design/V3_DISPLAY.md §4.6) ─────────────────────
 * Zero-copy: the surface wraps the frame the display worker's GPU copy filled (page-locked host
 * memory Rust keeps mapped for the process). kf3_display_frame hands over the newest one; the
 * worker never writes the frame shown, so the pixels under this surface only change when a later
 * call replaces it. Main thread, BQL held — nothing here waits on the worker. */
static pixman_format_code_t kf3_pixman_format(uint32_t f)
{
    switch (f) {
    case 1: return PIXMAN_x8r8g8b8;
    case 2: return PIXMAN_x8b8g8r8;
    case 3: return PIXMAN_r5g6b5;
    case 4: return PIXMAN_x2r10g10b10;
    case 5: return PIXMAN_x2b10g10r10;
    default: return 0;
    }
}

/* ★ ABI 12 (docs/design/V3_DISPLAY.md §8.13): the guest's cursor for THIS console while a
 * cursor-capable broker hovers — the frames then carry none (OWNER_RULINGS §O), so the console gets
 * it through QEMU's cursor API and a VNC client draws it as a real pointer (the coordinator's
 * decision of 2026-10-04). ⊘ ABI 23 (OWNER_RULINGS §V, §8.20): Rust decides what, when and
 * whether (kf_broker::ConsoleCursor::apply: following the frame the console SHOWS, paced; the
 * hidden image for none; a move only under an ABSOLUTE pointer, since GTK's gd_mouse_set warps
 * the HOST pointer whenever the console's input is relative, ui/gtk.c:447-467) and calls these
 * four verbs; each is QEMU's call and reports whether it was applied (a part not applied is
 * retried). Ownership per QEMU 10.2.4: cursor_alloc and cursor_builtin_hidden return a cursor with
 * one reference (ui/cursor.c:93-108), or NULL; dpy_cursor_define takes its own
 * (ui/console.c:961-980), so ours is dropped right after. The pixels are kayfabe's own copy of the
 * image (never guest memory), exactly width x height words, copied into what this function
 * allocated. Main thread, BQL held; nothing here waits. */
static int32_t kf3_cur_define(void *opaque, uint32_t width, uint32_t height, uint32_t hot_x,
                              uint32_t hot_y, const uint32_t *pixels)
{
    Kf3State *s = opaque;
    QEMUCursor *qc;

    if (!s->con || !pixels || width == 0 || height == 0 || width > KF3_CURSOR_MAX_DIM ||
        height > KF3_CURSOR_MAX_DIM || hot_x >= width || hot_y >= height) {
        return 0;
    }
    qc = cursor_alloc((uint16_t)width, (uint16_t)height);
    if (!qc) {
        return 0;
    }
    memcpy(qc->data, pixels, (size_t)width * height * sizeof(uint32_t));
    qc->hot_x = (int)hot_x;
    qc->hot_y = (int)hot_y;
    dpy_cursor_define(s->con, qc);
    cursor_unref(qc);
    return 1;
}

static int32_t kf3_cur_hide(void *opaque)
{
    Kf3State *s = opaque;
    QEMUCursor *qc = s->con ? cursor_builtin_hidden() : NULL;

    if (!qc) {
        return 0;
    }
    dpy_cursor_define(s->con, qc);
    cursor_unref(qc);
    return 1;
}

static int32_t kf3_cur_move(void *opaque, int32_t x, int32_t y)
{
    Kf3State *s = opaque;

    if (!s->con) {
        return 0;
    }
    dpy_mouse_set(s->con, x, y, true);
    return 1;
}

static uint32_t kf3_cur_absolute(void *opaque)
{
    Kf3State *s = opaque;

    return s->con && qemu_input_is_absolute(s->con);
}

static void kf3_console_cursor(Kf3State *s)
{
    kf3_display_cursor_apply(s->h);
}

/* The newest frame onto the console's surface (gfx_update). */
static void kf3_console_frame(Kf3State *s)
{
    Kf3Frame f;
    pixman_format_code_t fmt;

    if (kf3_display_frame(s->h, &f) != 0 || f.serial == s->shown.serial) {
        return;
    }
    fmt = kf3_pixman_format(f.format);
    if (!fmt || !f.data || f.width == 0 || f.height == 0 || f.stride < f.width ||
        (f.stride & 3) != 0) {
        return;
    }
    if (f.data != s->shown.data || f.width != s->shown.width || f.height != s->shown.height ||
        f.stride != s->shown.stride || f.format != s->shown.format) {
        dpy_gfx_replace_surface(s->con, qemu_create_displaysurface_from((int)f.width, (int)f.height,
                                                                         fmt, (int)f.stride, f.data));
    }
    s->shown = f;
    dpy_gfx_update_full(s->con);
}

/* ★ ABI 16 (sec. 8.16): a refresh request was answered (or its backstop expired) — show the newest
 * frame, then end every screendump waiting on this console (graphic_hw_update_done wakes the
 * coroutines queued in qemu_console_co_wait_update; with nobody waiting it does nothing). */
static void kf3_refresh_done(Kf3State *s)
{
    if (s->refresh_timer) {
        timer_del(s->refresh_timer);
    }
    if (!s->con) {
        return;
    }
    if (s->h) {
        kf3_console_frame(s);
        /* The asynchronous answer can change whether the shown frame composes
         * the cursor. Apply that frame's cursor policy before waking captures. */
        kf3_console_cursor(s);
    }
    graphic_hw_update_done(s->con);
}

static void kf3_refresh_ready(void *opaque)
{
    Kf3State *s = opaque;

    if (s->h) {
        kf3_display_refresh_drain(s->h);
    }
    kf3_refresh_done(s);
}

static void kf3_refresh_backstop(void *opaque)
{
    kf3_refresh_done(opaque);
}

#define KF3_REFRESH_BACKSTOP_MS 1000

static void kf3_gfx_update(void *opaque)
{
    Kf3State *s = opaque;

    if (!s->h) {
        if (s->con) {
            graphic_hw_update_done(s->con);
        }
        return;
    }
    kf3_console_frame(s);
    /* §8.13 (the review of 2026-10-04): AFTER the frame — the console's cursor follows the frame it
     * now shows (an image beside a frame that still composes one is two cursors); in hover a cursor
     * change makes no frame, so this runs even when no new frame came */
    kf3_console_cursor(s);
    /* ★ ABI 16 (sec. 8.16, owner decision D2): no copy is made without a flip while nobody watches,
     * so a screendump asks for a frame no older than its request. The worker answers at the console
     * head's next tick (or at once when nothing can be copied); kf3_refresh_ready then shows it and
     * ends the wait. Without an answer to come, the wait ends now. Nothing here waits. */
    if (s->refresh_fd >= 0 && kf3_display_refresh(s->h) == 1) {
        if (s->refresh_timer && !timer_pending(s->refresh_timer)) {
            timer_mod(s->refresh_timer,
                      qemu_clock_get_ms(QEMU_CLOCK_REALTIME) + KF3_REFRESH_BACKSTOP_MS);
        }
    } else if (s->con) {
        graphic_hw_update_done(s->con);
    }
}

/* ★ ABI 12 (display step 3c, docs/design/V3_DISPLAY.md §8.6): a resize hint from the UI (the
 * broker's SURFACE through dpy_set_ui_info, or a GTK/VNC frontend on the same console) after QEMU's
 * 1 s coalescing. Rust authors the new monitor's EDID on its worker and the register drainer posts
 * the hotplug; nothing here waits. */
static void kf3_ui_info(void *opaque, uint32_t head, QemuUIInfo *info)
{
    Kf3State *s = opaque;

    if (s->h && info) {
        kf3_display_ui_info(s->h, head, info->width, info->height, info->refresh_rate);
    }
}

/* ⊘ 2026-10-03 (the review of v3-broker): the ui_info hook is installed ONLY with display-broker
 * set. Installed unconditionally, it changed the console path without a broker: GTK's
 * gd_configure -> gd_set_ui_size -> dpy_set_ui_info re-authored the guest's monitor to the widget's
 * size (its startup size included), and a VNC SetDesktopSize re-moded the guest. Without a broker
 * the console is exactly the M2 one (no ui_info: dpy_ui_info_supported() is false). */
/* ★ ABI 16: gfx_update_async — QEMU then waits for graphic_hw_update_done (kf3_refresh_done) before
 * a screendump reads the surface (ui/console.c graphic_hw_update, ui/ui-qmp-cmds.c qmp_screendump). */
static const GraphicHwOps kf3_gfx_ops = {
    .gfx_update = kf3_gfx_update,
    .gfx_update_async = true,
};

static const GraphicHwOps kf3_gfx_ops_broker = {
    .gfx_update = kf3_gfx_update,
    .gfx_update_async = true,
    .ui_info = kf3_ui_info,
};

/* ── the display-broker relay (display step 3, docs/design/V3_DISPLAY.md §8) ─────────────────
 * Rust decides; this file registers what Rust asks for and carries out its input verbs. Every
 * function here runs on the main loop with the BQL held. */

static void kf3_broker_pump(Kf3State *s, int fd, bool rd, bool wr);

static void kf3_broker_sock_rd(void *opaque)
{
    Kf3State *s = opaque;
    kf3_broker_pump(s, s->broker_sock, true, false);
}

static void kf3_broker_sock_wr(void *opaque)
{
    Kf3State *s = opaque;
    kf3_broker_pump(s, s->broker_sock, false, true);
}

static void kf3_broker_frame_rd(void *opaque)
{
    Kf3State *s = opaque;
    kf3_broker_pump(s, s->broker_frame_fd, true, false);
}

static void kf3_broker_timer_cb(void *opaque)
{
    kf3_broker_pump(opaque, -1, false, false);
}

/* Rust's watch verb (Kf3BrokerWatchFn). (0, 0) arrives BEFORE Rust closes the fd, so no stale
 * descriptor number stays registered (nvkvm-pv relay.c:724-743's order). ⊘ ABI 23 (§8.20): the
 * absent-tablet check at a new connection is kf-broker's (InputPolicy::connected), through the
 * pointers verb. */
static void kf3_broker_watch(void *opaque, int32_t fd, uint32_t rd, uint32_t wr)
{
    Kf3State *s = opaque;

    if (!rd && !wr) {
        qemu_set_fd_handler(fd, NULL, NULL, NULL);
        if (s->broker_sock == fd) {
            s->broker_sock = -1;
        }
        return;
    }
    s->broker_sock = fd;
    qemu_set_fd_handler(fd, rd ? kf3_broker_sock_rd : NULL, wr ? kf3_broker_sock_wr : NULL, s);
}

/* Rust's timer verb (Kf3BrokerTimerFn): QEMU_CLOCK_REALTIME milliseconds, -1 = none. */
static void kf3_broker_timer(void *opaque, int64_t deadline_ms)
{
    Kf3State *s = opaque;

    if (!s->broker_timer) {
        return;
    }
    if (deadline_ms < 0) {
        timer_del(s->broker_timer);
    } else {
        timer_mod(s->broker_timer, deadline_ms);
    }
}

/* ── ★ ABI 23 (OWNER_RULINGS.md §V, 2026-10-08; docs/design/V3_DISPLAY.md §8.20): the input
 * verbs — kf-broker's VMM-neutral InputSink for QEMU. Every decision is Rust's (crates/kf-broker
 * src/input.rs: which keys and buttons, which pointing device on a grab and on its end, where the
 * sync points fall, the missing-device warning, close); each verb is the QEMU call that carries one
 * out, aimed at kf3's own console: qemu_input_find_handler takes a handler bound to it first
 * (-device virtio-tablet-pci,display=<kf3 id>), then any unbound one. ⊘ Until ABI 22 this file
 * interpreted Kf3BrokerEvents itself (nvkvm-pv relay.c:1101-1462, the QEMU 10.2 spelling); §8.20
 * has the old -> new table. */

static int32_t kf3_in_key(void *opaque, uint32_t evdev, uint32_t down)
{
    Kf3State *s = opaque;

    /* QEMU's linux -> qcode map is this VMM's mapping: a code it cannot map is reported back */
    if (evdev >= qemu_input_map_linux_to_qcode_len ||
        qemu_input_map_linux_to_qcode[evdev] == Q_KEY_CODE_UNMAPPED) {
        return 0;
    }
    qemu_input_event_send_key_qcode(s->con, qemu_input_linux_to_qcode(evdev), down != 0);
    return 1;
}

static const InputButton kf3_in_buttons[] = {
    [KF3_BTN_LEFT] = INPUT_BUTTON_LEFT,     [KF3_BTN_RIGHT] = INPUT_BUTTON_RIGHT,
    [KF3_BTN_MIDDLE] = INPUT_BUTTON_MIDDLE, [KF3_BTN_SIDE] = INPUT_BUTTON_SIDE,
    [KF3_BTN_EXTRA] = INPUT_BUTTON_EXTRA,
};

/* relative: with src NULL QEMU's qemu_input_find_handler skips every display-bound handler — the
 * tablet bound to kf3's console — and takes the first unbound one, the relative device the select
 * verb put in front (§8.19: motion, buttons and wheel then come from ONE guest device) */
static QemuConsole *kf3_in_src(Kf3State *s, uint32_t relative)
{
    return relative ? NULL : s->con;
}

static void kf3_in_button(void *opaque, uint32_t button, uint32_t down, uint32_t relative)
{
    Kf3State *s = opaque;

    if (button < ARRAY_SIZE(kf3_in_buttons)) {
        qemu_input_queue_btn(kf3_in_src(s, relative), kf3_in_buttons[button], down != 0);
    }
}

/* QEMU's wheel is a button: one detent is a press, a sync and a release (Rust's sync follows).
 * QEMU 10.2's virtio and USB pointers map no WHEEL_LEFT/RIGHT, so dx is not carried. */
static void kf3_in_wheel(void *opaque, int32_t dx, int32_t dy, uint32_t relative)
{
    Kf3State *s = opaque;
    InputButton b = dy > 0 ? INPUT_BUTTON_WHEEL_UP : INPUT_BUTTON_WHEEL_DOWN;

    (void)dx;
    if (dy == 0) {
        return;
    }
    qemu_input_queue_btn(kf3_in_src(s, relative), b, true);
    qemu_input_event_sync();
    qemu_input_queue_btn(kf3_in_src(s, relative), b, false);
}

/* QEMU scales value * INPUT_EVENT_ABS_MAX / range (qemu_input_scale_axis) */
static void kf3_in_abs(void *opaque, uint32_t x, uint32_t y, uint32_t width, uint32_t height)
{
    Kf3State *s = opaque;

    /* the shim re-checks what Rust validated: QEMU scales by range, and a zero or over-INT_MAX
     * range or value must never reach it (review 2026-10-08, finding 1) */
    if (!width || !height || width > (uint32_t)INT_MAX || height > (uint32_t)INT_MAX ||
        x > (uint32_t)INT_MAX || y > (uint32_t)INT_MAX) {
        return;
    }
    qemu_input_queue_abs(s->con, INPUT_AXIS_X, (int)x, 0, (int)width);
    qemu_input_queue_abs(s->con, INPUT_AXIS_Y, (int)y, 0, (int)height);
}

static void kf3_in_rel(void *opaque, int32_t dx, int32_t dy)
{
    Kf3State *s = opaque;

    qemu_input_queue_rel(s->con, INPUT_AXIS_X, dx);
    qemu_input_queue_rel(s->con, INPUT_AXIS_Y, dy);
}

static void kf3_in_sync(void *opaque)
{
    (void)opaque;
    qemu_input_event_sync();
}

/* QEMU's pointing devices; it names its virtio-input ones "QEMU Virtio Tablet/Mouse" */
static uint32_t kf3_in_pointers(void *opaque, Kf3Pointer *out, uint32_t cap)
{
    MouseInfoList *mice = qmp_query_mice(NULL), *e;
    uint32_t n = 0;

    (void)opaque;
    for (e = mice; e && n < cap; e = e->next, n++) {
        memset(&out[n], 0, sizeof(out[n]));
        out[n].id = (uint32_t)e->value->index;
        out[n].absolute = e->value->absolute;
        out[n].paravirtual = e->value->name && strstr(e->value->name, "Virtio");
        if (e->value->name) {
            g_strlcpy(out[n].name, e->value->name, sizeof(out[n].name));
        }
    }
    qapi_free_MouseInfoList(mice);
    return n;
}

static void kf3_in_select_pointer(void *opaque, uint32_t id, uint32_t relative)
{
    (void)opaque;
    (void)relative;
    qemu_mouse_set((int)id, NULL);
}

/* Rust logged what is lost; this says how to add the device on QEMU's command line */
static void kf3_in_missing_pointer(void *opaque, uint32_t relative)
{
    Kf3State *s = opaque;

    if (relative) {
        warn_report("kf3: broker: add -device virtio-mouse-pci for the grab's relative pointer");
    } else {
        warn_report("kf3: broker: add -device virtio-tablet-pci,display=%s,head=0 for the "
                    "broker's absolute pointer",
                    DEVICE(s)->id ? DEVICE(s)->id : "<this kf3-gpu's id>");
    }
}

/* force = stop now; otherwise an ACPI powerdown the guest decides on (Rust logs the repeat-ask) */
static void kf3_in_close(void *opaque, uint32_t force)
{
    (void)opaque;
    if (force) {
        qemu_system_shutdown_request(SHUTDOWN_CAUSE_HOST_UI);
    } else {
        qemu_system_powerdown_request();
    }
}

/* 3c: a resize hint through the console's ui_info (QEMU coalesces for 1 s and only a change reaches
 * kf3_ui_info); a refresh of 0 (unknown) keeps the console's */
static void kf3_in_resize_hint(void *opaque, uint32_t width, uint32_t height, uint32_t refresh_mhz)
{
    Kf3State *s = opaque;
    QemuUIInfo info;

    if (!s->con || width == 0 || height == 0 || !dpy_ui_info_supported(s->con)) {
        return;
    }
    info = *dpy_get_ui_info(s->con);
    info.width = width;
    info.height = height;
    if (refresh_mhz > 0) {
        info.refresh_rate = refresh_mhz;
    }
    dpy_set_ui_info(s->con, &info, true);
}

static const Kf3InputOps kf3_input_ops = {
    .key = kf3_in_key,
    .button = kf3_in_button,
    .wheel = kf3_in_wheel,
    .abs = kf3_in_abs,
    .rel = kf3_in_rel,
    .sync = kf3_in_sync,
    .pointers = kf3_in_pointers,
    .select_pointer = kf3_in_select_pointer,
    .missing_pointer = kf3_in_missing_pointer,
    .close = kf3_in_close,
    .resize_hint = kf3_in_resize_hint,
    .cursor_define = kf3_cur_define,
    .cursor_hide = kf3_cur_hide,
    .cursor_move = kf3_cur_move,
    .cursor_absolute = kf3_cur_absolute,
};

/* ⊘ ABI 23: Rust delivers the input through kf3_input_ops and brings the console cursor along
 * (§8.13: every cursor post is followed by a frame publish, which lands here). */
static void kf3_broker_pump(Kf3State *s, int fd, bool rd, bool wr)
{
    if (s->h) {
        kf3_broker_ready(s->h, fd, rd ? 1 : 0, wr ? 1 : 0,
                         (uint64_t)qemu_clock_get_ms(QEMU_CLOCK_REALTIME));
    }
}

/* Realize, before the option ROM and console are registered. The path rules (absolute, < sun_path, no abstract namespace)
 * and display-broker-uid's range (-1, or a uid) are Rust's and refuse here by name; an ABSENT
 * broker is not an error (retried, never a startup dependency). Rust only ARMS the first attempt
 * on the timer: it runs from the main loop, after os_setup_post's -run-with user= / -runas
 * dropped privileges, so the peer check judges against the uid QEMU runs as. */
static bool kf3_broker_realize(Kf3State *s, Error **errp)
{
    char err[512] = "";

    s->broker_sock = -1;
    s->broker_frame_fd = -1;
    if (!s->display_broker) {
        return true;
    }
    s->broker_frame_fd = kf3_broker_frame_fd(s->h);
    if (s->broker_frame_fd < 0) {
        error_setg(errp, "kf3: display-broker: the display has no broker frames");
        return false;
    }
    s->broker_timer = timer_new_ms(QEMU_CLOCK_REALTIME, kf3_broker_timer_cb, s);
    qemu_set_fd_handler(s->broker_frame_fd, kf3_broker_frame_rd, NULL, s);
    if (kf3_broker_start(s->h, s->display_broker, s->display_broker_uid, kf3_broker_watch,
                         kf3_broker_timer, &kf3_input_ops, s,
                         (uint64_t)qemu_clock_get_ms(QEMU_CLOCK_REALTIME),
                         err, sizeof(err)) != 0) {
        error_setg(errp, "kf3: %s", err);
        return false;
    }
    info_report("kf3: display broker relay on %s (input goes to this device's console)",
                s->display_broker);
    return true;
}

/* Exit, BEFORE the console closes: Rust unwatches and closes the socket, then the timer and the
 * frame handler go. */
static void kf3_broker_exit(Kf3State *s)
{
    if (!s->display_broker || !s->h) {
        return;
    }
    kf3_broker_stop(s->h);
    if (s->broker_frame_fd >= 0) {
        qemu_set_fd_handler(s->broker_frame_fd, NULL, NULL, NULL);
        s->broker_frame_fd = -1;
    }
    if (s->broker_timer) {
        timer_free(s->broker_timer);
        s->broker_timer = NULL;
    }
}

/* ★ ABI 11 — the boot display's option ROM (docs/design/V3_DISPLAY.md §4.11.6), registered the way
 * pci_add_option_rom (hw/pci/pci.c) registers a romfile: has_rom, a ROM region of pow2ceil(len)
 * (or the user's romsize), the bytes copied in, pci_register_bar(PCI_ROM_SLOT); pci_del_option_rom
 * cleans it up at unrealize. ⊘ Not vfio's trapping ROM, and NEVER a class-level pc->romfile:
 * pci_patch_ids would rewrite byte 6, which is inside this ROM's EfiSignature.
 * ⊘ CORRECTED 2026-10-03 (v3-gop): a user romfile= (even "") or rombar=0 no longer "wins with a
 * warning" here — kf3_dev_realize refuses either with gop=on before Rust realizes (kf3_gop.h), so by
 * this point gop=on means this ROM is the ROM BAR. Called last, after every step of realize that can
 * fail: QEMU's realize-failure path (pci_qdev_realize → do_pci_unregister_device) never runs
 * pci_del_option_rom, so a ROM registered before a later refusal would stay registered. */
static bool kf3_option_rom_build(Kf3State *s, PCIDevice *pci, Error **errp)
{
    const uint8_t *rom = NULL;
    uint64_t len = 0, size;

    if (!s->gop) {
        return true;
    }
    if (kf3_option_rom(s->h, &rom, &len) != 0 || !rom || len == 0) {
        error_setg(errp, "kf3: gop=on, but Rust packed no option ROM");
        return false;
    }
    if (pci->romsize != UINT32_MAX) {
        if (len > pci->romsize) {
            error_setg(errp, "kf3: the boot display's option ROM (%" PRIu64 " bytes) is too large for "
                       "romsize %u", len, pci->romsize);
            return false;
        }
        size = pci->romsize;
    } else {
        size = pow2ceil(len);
    }
    if (!memory_region_init_rom(&pci->rom, OBJECT(pci), "kf3-gpu.rom", size, errp)) {
        return false;
    }
    pci->has_rom = true;
    memcpy(memory_region_get_ram_ptr(&pci->rom), rom, len);
    pci_register_bar(pci, PCI_ROM_SLOT, 0, &pci->rom);
    info_report("kf3: boot display option ROM registered: %" PRIu64 " bytes in a %" PRIu64 "-byte ROM BAR",
                len, size);
    return true;
}

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
    /* ★ gop=on needs THIS device's ROM BAR. QEMU's own ROM properties asking otherwise are refused by
     * name here, before Rust realizes, so the Rust side never enters the boot-display posture (store
     * zeroed, BAR1 seeded, budget charged, boot layer armed, fn 65's region 0) for a ROM that would not
     * be served (kf3_gop.h). */
    {
        const char *why = kf3_gop_rom_conflict(s->gop, pci->romfile, pci->rom_bar);
        if (why) {
            error_setg(errp, "kf3: gop=on refused: %s (drop the property, or set gop=off; "
                       "docs/design/V3_DISPLAY.md §4.11.12)", why);
            return;
        }
    }
    if (s->display_broker && !s->display) {
        error_setg(errp, "kf3: display-broker needs display=on (the broker shows the virtual display)");
        return;
    }
    /* ★ ABI 12 (docs/design/V3_DISPLAY.md sec. 8.11): display-broker-vram rides in bits 1-2 */
    uint32_t broker_word = 0;
    if (s->display_broker_vram && !s->display_broker) {
        error_setg(errp, "kf3: display-broker-vram needs display-broker (it is the broker's GPU-copy rung)");
        return;
    }
    if (s->display_broker) {
        uint32_t vram = KF3_BROKER_VRAM_AUTO;
        if (s->display_broker_vram && !strcmp(s->display_broker_vram, "on")) {
            vram = KF3_BROKER_VRAM_ON;
        } else if (s->display_broker_vram && !strcmp(s->display_broker_vram, "off")) {
            vram = KF3_BROKER_VRAM_OFF;
        } else if (s->display_broker_vram && strcmp(s->display_broker_vram, "auto")) {
            error_setg(errp, "kf3: display-broker-vram=%s: the values are auto, on and off",
                       s->display_broker_vram);
            return;
        }
        broker_word = KF3_BROKER_ON | (vram << KF3_BROKER_VRAM_SHIFT);
    }
    s->refresh_fd = -1;
    s->broker_sock = -1;
    s->broker_frame_fd = -1;
    /* ★ 2026-10-03 (V3_P4_PORT_MAP.md Q3): never share a VM with a device that pins guest RAM for
     * DMA. VFIO's listener DMA-maps every ram_device region (QEMU 10.2.4 hw/vfio/listener.c:
     * 598-631): it would pin every page of PRAMIN/BAR1/BAR2 and keep IOMMU mappings of whatever
     * they showed at that moment, stale after every re-point Rust makes behind QEMU's back -- a
     * released host BAR1 aperture among them, reachable by the guest-programmed device. 10.2 has
     * no per-region opt-out (memory_region_set_skip_iommu_map arrives in QEMU 11.1). Every 10.2
     * technology that pins guest RAM first disables RAM discard, and ram_block_discard_require()
     * inhibits exactly that, in both realize orders and for hotplug: vfio legacy and iommufd,
     * vfio-user, and the nvme:// userspace block driver, which DMA-maps every RAM block through a
     * RAMBlockNotifier (util/vfio-helpers.c: qemu_vfio_open_pci disables discard before it
     * registers the notifier), so a later one fails with "Cannot set discarding of RAM broken";
     * also refused, as collateral: libblkio drivers that may pin memory (block/blkio.c), vhost-vdpa
     * (it skips ram_device sections, so it was never the hazard), SEV/SEV-ES and incoming COLO.
     * ⊘ Corrected 2026-10-03 (third review): this said nvme:// was "not caught" and "disables
     * nothing"; it disables discard first, so it is refused like VFIO.
     * ★ Taken FIRST, before kf3_realize builds anything (the host store, the RM client, the
     * threads, the windows), so a refused realize builds and holds nothing (only checks that build
     * nothing come before it: KVM, the archive ABI, and property checks); every later failure
     * goes to `fail`, which gives it back (QEMU calls no exit for a failed realize: 10.2.4
     * hw/pci/pci.c pci_qdev_realize only unregisters the device). */
    if (ram_block_discard_require(true) != 0) {
        error_setg(errp, "kf3: RAM discard is already disabled by something that pins guest RAM "
                   "(a VFIO, iommufd or vfio-user device, an nvme:// drive, a libblkio drive that may "
                   "pin memory, vhost-vdpa, SEV or COLO); kf3 refuses to share a VM with one, since "
                   "a DMA-mapping one would map kf3's BAR windows and keep stale mappings after "
                   "every re-point");
        return;
    }
    s->discard_required = true;

    /* ABI 22: the VM's identity is the vm-id property, else QEMU's -uuid when one was given. A VM
     * with neither passes NULL and Rust says so by name (gpu-uuid=auto then uses a random value). */
    char *qemu_vm_id = (!s->vm_id || !s->vm_id[0]) && qemu_uuid_set
                           ? qemu_uuid_unparse_strdup(&qemu_uuid) : NULL;
    const char *vm_id = (s->vm_id && s->vm_id[0]) ? s->vm_id : qemu_vm_id;
    int32_t realize_rc = kf3_realize(s->gpu_minor, s->fb_mb, s->bar1_size, s->bar2_size, s->guest_driver, s->display ? 1 : 0,
                    s->gop ? 1 : 0, s->x11_dispsw ? 1 : 0, broker_word, s->display_max_fps, s->gop_efi,
                    s->gpu_uuid, vm_id, (uint32_t)pci->devfn, s->channel_budget, &s->h, err, sizeof(err));
    g_free(qemu_vm_id); /* Rust copied what it keeps during the call */
    if (realize_rc != 0) {
        error_setg(errp, "kf3: realize refused: %s", err);
        goto fail;
    }
    if (kf3_identity(s->h, &id) != 0) {
        error_setg(errp, "kf3: no identity");
        goto fail;
    }
    /* ★ ABI 24, DIAGNOSTIC, default off: Rust read KF3_BAR0_READ_TRACE at realize. Off (and no
     * x-gsp-observer) = BAR0, its ops and the irqfd MSI path are built exactly as before. */
    s->tr_on = kf3_trace_mode(s->h) == 1;
    s->diag = s->tr_on || s->gsp_observer_path;
    if (s->diag && s->db_ioeventfd) {
        error_setg(errp, "kf3: the BAR0 trace and x-gsp-observer need doorbell-ioeventfd=off "
                   "(an ioeventfd doorbell write never reaches them)");
        goto fail;
    }
#ifndef KF3_GSP_OBSERVER
    if (s->gsp_observer_path) {
        error_setg(errp, "kf3: x-gsp-observer: this QEMU tree has no shared GSP observer "
                   "(install tools/vfio-gsp-observer with its apply.py, then rebuild)");
        goto fail;
    }
#endif
    if (s->tr_on) {
        kf3_trace_name(s->h, s->tr_name, sizeof(s->tr_name));
        warn_report("kf3: ★★ DIAGNOSTIC BAR0 TRACE MODE ON (KF3_BAR0_READ_TRACE=1, default off; "
                    "OWNER_RULINGS.md sec. X): selected BAR0 reads EXIT and every record goes through "
                    "QEMU's vfio trace events as %s; MSI-X goes through the main loop. Slow by design.",
                    s->tr_name);
        if (!trace_event_get_state_backends(TRACE_VFIO_REGION_READ)) {
            warn_report("kf3: BAR0 trace mode is on but the vfio_region_read trace event is not "
                        "enabled: nothing is recorded (pass -trace events=FILE,file=LOG)");
        }
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
        goto fail;
    }
    pci_register_bar(pci, 0, PCI_BASE_ADDRESS_SPACE_MEMORY, &s->bar0);
    /* ★ ABI 21: the diagnostic read-trap verb (Rust calls it only with KF3_BAR0_TRACE=1). */
    s->read_trap_bh = qemu_bh_new(kf3_read_trap_bh, s);
    if (kf3_set_read_trap(s->h, kf3_read_trap, s) != 0) {
        error_setg(errp, "kf3: Rust refused the BAR0 read-trap verb");
        goto fail;
    }

    if (!kf3_bar_build(s, 1, &s->bar1, s->bar1_size, errp) ||
        !kf3_bar_build(s, 2, &s->bar2, s->bar2_size, errp)) {
        goto fail;
    }
    if (!kf3_bar1_views_build(s, errp)) {
        goto fail;
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
            goto fail;
        }
        pci_register_bar(pci, KF3_MSIX_BAR, PCI_BASE_ADDRESS_SPACE_MEMORY, &s->msix_bar);
        for (unsigned i = 0; i < s->msix_vectors; i++) {
            msix_vector_use(pci, i);
        }
        /* ⊘ No irqfd, no interrupts: refuse rather than fall back to a BQL-taking notify.
         * (★ ABI 24: the diagnostic trace mode alone takes the main-loop path on purpose.) */
        if (!s->diag && !kvm_msi_via_irqfd_enabled()) {
            error_setg(errp, "kf3: KVM MSI-via-irqfd is not available (kernel irqchip required)");
            goto fail;
        }
        for (unsigned i = 0; i < s->msix_vectors && i < KF3_MAX_VECTORS; i++) {
            int fd = kf3_irq_fd(s->h, i);
            s->vec[i].virq = -1;
            if (fd >= 0) {
                event_notifier_init_fd(&s->vec[i].e, fd);
                s->vec[i].have = true;
                if (s->diag) {
                    s->vec[i].s = s;
                    s->vec[i].nr = i;
                    qemu_set_fd_handler(fd, kf3_msi_user, NULL, &s->vec[i]);
                }
            }
        }
        if (!s->diag &&
            msix_set_vector_notifiers(pci, kf3_vector_use, kf3_vector_release, NULL) < 0) {
            error_setg(errp, "kf3: MSI-X vector notifiers refused");
            goto fail;
        }
    }
#ifdef KF3_GSP_OBSERVER
    /* ★ ABI 24, DIAGNOSTIC: the shared GSP observer (vfio-pci's x-gsp-observer, same code) */
    if (s->gsp_observer_path) {
        s->obs = gsp_observer_open(pci, s->gsp_observer_path, s->gsp_observer_seconds, errp);
        if (!s->obs) {
            goto fail;
        }
        warn_report("kf3: ★★ DIAGNOSTIC GSP OBSERVER ON (x-gsp-observer=%s, %u s): the VFIO "
                    "reference's observer samples this guest's GSP queues", s->gsp_observer_path,
                    s->gsp_observer_seconds);
    }
#endif

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
            goto fail;
        }
        pci_set_long(c + off, val);
    }

    /* ★ ABI 10 (v3-ioeventfd's 9): the doorbell fast path, only on request. Enabled BEFORE the
     * listener registers, so its replay of the current sections reports the doorbell sites already
     * mapped. */
    s->db_page_off = -1;
    if (s->db_ioeventfd) {
        if (!s->um_mr) {
            error_setg(errp, "kf3: doorbell-ioeventfd=on, but BAR0 has no usermode passthrough piece");
            goto fail;
        }
        s->db_page_off = kf3_doorbell_page_offset(s->h);
        if (s->db_page_off < 0 || kf3_set_ioeventfd(s->h, kf3_ioeventfd, s, s->db_ioeventfd_max) != 0) {
            error_setg(errp, "kf3: Rust refused the doorbell fast path");
            goto fail;
        }
    }
    /* The relay only arms its first attempt, which runs after realize returns to the main
     * loop. Validate it before registering the ROM, listener and console: QEMU does not run
     * device exit after a refused realize. */
    if (!kf3_broker_realize(s, errp)) {
        goto fail;
    }
    /* ★ ABI 11: the boot display's option ROM (gop=on only) — after the last step of realize that can
     * fail; nothing below this line can (kf3_option_rom_build's comment says why it matters). */
    if (!kf3_option_rom_build(s, pci, errp)) {
        goto fail;
    }

    s->listener = (MemoryListener){
        .name = "kf3-guest-ram",
        .region_add = kf3_region_add,
        .region_del = kf3_region_del,
        .priority = MEMORY_LISTENER_PRIORITY_MIN,
    };
    memory_listener_register(&s->listener, &address_space_memory);

    if (s->display) {
        /* the ui_info hook (3c resize) only with a broker: without one, GTK/VNC stay as in M2 */
        s->con = graphic_console_init(DEVICE(pci), 0,
                                      s->display_broker ? &kf3_gfx_ops_broker : &kf3_gfx_ops, s);
        info_report("kf3: display console registered (head 0 of %s)",
                    DEVICE(pci)->id ? DEVICE(pci)->id : "kf3-gpu");
        /* ★ ABI 16: the on-demand refresh's answer and its backstop */
        s->refresh_fd = kf3_display_refresh_fd(s->h);
        if (s->refresh_fd >= 0) {
            /* HMP waits for screendump's coroutine with aio_poll on this context.
             * qemu_set_fd_handler uses the separate iohandler context, and ordinary
             * timers also need the outer main loop: neither can end that wait.
             * Keep both completion and backstop on the context the waiter polls. */
            s->refresh_timer = aio_timer_new(qemu_get_aio_context(), QEMU_CLOCK_REALTIME,
                                             SCALE_MS, kf3_refresh_backstop, s);
            aio_set_fd_handler(qemu_get_aio_context(), s->refresh_fd,
                               kf3_refresh_ready, NULL, NULL, NULL, s);
        }
    }

    /* ⊘ 2026-10-03 (v3-broker): the status line outgrew 512 bytes — the disp[...] and broker[...]
     * fragments were cut off at realize and exit, so the relay's counters never reached the log */
    {
        char st[4096] = "";
        kf3_status(s->h, st, sizeof(st));
        info_report("%s (BAR0 pieces=%u)", st, s->n_pieces);
    }
    if (s->tr_on) {
        /* ★ ABI 24: the trace's report at exit (registered after the last failure point) */
        s->tr_exit.notify = kf3_trace_exit;
        qemu_add_exit_notifier(&s->tr_exit);
    }
    return;

fail:
#ifdef KF3_GSP_OBSERVER
    gsp_observer_close(s->obs);
    s->obs = NULL;
#endif
    kf3_msi_user_off(s);
    kf3_broker_exit(s);
    /* Every failure after the discard requirement was taken. QEMU calls no exit for a failed
     * realize, so it is given back here. ⊘ Pre-existing and unchanged: a failure after kf3_realize
     * still leaves what kf3_realize built (s->h) in place. */
    ram_block_discard_require(false);
    s->discard_required = false;
}

static void kf3_dev_exit(PCIDevice *pci)
{
    Kf3State *s = KF3(pci);
    char st[4096] = "";
    if (s->h) {
        kf3_status(s->h, st, sizeof(st));
        info_report("%s bar12_reads=%" PRIu64 " bar12_writes=%" PRIu64 " irq_routes=%" PRIu64
                    " irq_route_fail=%" PRIu64 " bar1_ov_applied=%" PRIu64 " bar1_ov_failed=%" PRIu64
                    " db_sites_added=%" PRIu64 " db_sites_removed=%" PRIu64, st,
                    qatomic_read(&s->bar12_reads), qatomic_read(&s->bar12_writes),
                    s->irq_routes, s->irq_route_fail, s->bar1_ov_applied, s->bar1_ov_failed,
                    s->db_sites_added, s->db_sites_removed);
        memory_listener_unregister(&s->listener);
        /* ★ ABI 24, DIAGNOSTIC: the report now, and nothing traced or observed after this */
        if (s->tr_on) {
            qemu_remove_exit_notifier(&s->tr_exit);
            kf3_trace_exit(&s->tr_exit, NULL);
        }
#ifdef KF3_GSP_OBSERVER
        gsp_observer_close(s->obs);
        s->obs = NULL;
#endif
        kf3_msi_user_off(s);
        /* ★ ABI 12: the broker relay stops BEFORE the console closes (its input targets it) */
        kf3_broker_exit(s);
        /* ★ ABI 16: no refresh answer arrives after this; a screendump still waiting ends now */
        if (s->refresh_fd >= 0) {
            aio_set_fd_handler(qemu_get_aio_context(), s->refresh_fd,
                               NULL, NULL, NULL, NULL, NULL);
            s->refresh_fd = -1;
        }
        if (s->refresh_timer) {
            timer_free(s->refresh_timer);
            s->refresh_timer = NULL;
        }
        if (s->con) {
            graphic_hw_update_done(s->con);
        }
        if (s->con) {
            /* the console stops reading the display's frames before the device goes */
            graphic_console_close(s->con);
            s->con = NULL;
        }
        kf3_unrealize(s->h);
    }
    if (s->msix_vectors > 0) {
        if (!s->diag) {   /* ★ ABI 24: the trace mode never set them */
            msix_unset_vector_notifiers(pci);
        }
        msix_uninit(pci, &s->msix_bar, &s->msix_bar);
    }
    if (s->discard_required) {
        ram_block_discard_require(false);
        s->discard_required = false;
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
    DEFINE_PROP_BOOL("display", Kf3State, display, false),
    /* ★ 2026-10-03: the boot display (docs/design/V3_DISPLAY.md §4.11). OFF = today's device. */
    DEFINE_PROP_BOOL("gop", Kf3State, gop, false),
    /* ★ 2026-10-03: EXPERIMENT (V3_DISPLAY.md, the x11-dispsw note). OFF until OWNER_RULINGS §N's default-on
     * conditions hold; needs display=on. */
    DEFINE_PROP_BOOL("x11-dispsw", Kf3State, x11_dispsw, false),
    /* ★ ABI 19 (2026-10-05, Windows integration): the boot display's driver, signed — validated by Rust as the
     * embedded kf-gop plus an Authenticode signature; refused with gop=off. */
    DEFINE_PROP_STRING("gop-efi", Kf3State, gop_efi),
    /* ABI 22: see Kf3State. Unset gpu-uuid = auto. Unset vm-id falls back to -uuid (if given). */
    DEFINE_PROP_STRING("gpu-uuid", Kf3State, gpu_uuid),
    DEFINE_PROP_STRING("vm-id", Kf3State, vm_id),
    /* ★ ABI 26 (2026-10-11, docs/design/V3_CHANNEL_BUDGET.md): channels PER RUNLIST — the count the guest is told and
     * the enforced twin cap. 0 (default) = derived from the host (free channels minus a 1/8 reserve). Refused at
     * realize above what the host can give (or 2048, the token field) and below 256, by name. */
    DEFINE_PROP_UINT32("channel-budget", Kf3State, channel_budget, 0),
    /* ★ 2026-09-30: the doorbell fast path (docs/design/V3_DOORBELL_IOEVENTFD.md). OFF until measured. */
    DEFINE_PROP_BOOL("doorbell-ioeventfd", Kf3State, db_ioeventfd, false),
    DEFINE_PROP_UINT32("doorbell-ioeventfd-max", Kf3State, db_ioeventfd_max, 256),
    /* ★ 2026-10-03, display step 3 (docs/design/V3_DISPLAY.md §8): the display broker's socket
     * (absolute path; unset = off, the console alone) and one more uid accepted as the broker
     * (-1 = none; any other value outside 0..4294967294 is refused at realize). Uid 0 and QEMU's
     * effective uid AT EACH CONNECT (after -run-with user= / -runas) are always accepted; the
     * owner of the socket's directory is not (anyone can create a missing /tmp directory). */
    DEFINE_PROP_STRING("display-broker", Kf3State, display_broker),
    DEFINE_PROP_INT64("display-broker-uid", Kf3State, display_broker_uid, -1),
    /* ★ 2026-10-03, the GPU-copy rung (docs/design/V3_DISPLAY.md sec. 8.11, OWNER_RULINGS L):
     * auto (default: probe at realize, allocate kayfabe's own VRAM frame slots at the broker's
     * first yes for the block-linear pair), on (allocate at realize; a refusal fails realize),
     * off (host-memory rungs only). Never guest memory: the slots are kayfabe's. */
    DEFINE_PROP_STRING("display-broker-vram", Kf3State, display_broker_vram),
    /* ★ 2026-10-04, ABI 16 (docs/design/V3_DISPLAY.md sec. 8.16, OWNER_RULINGS sec. M): the cap on
     * every head's emulated vblank tick, whole Hz. 0 (default) = unset: a cap of 75 Hz and today's
     * EDID, byte for byte. 24..75 sets the cap and the monitor's preferred rate; below 24, above 75
     * (owner decision D3) or without display=on, realize is refused by name. */
    DEFINE_PROP_UINT32("display-max-fps", Kf3State, display_max_fps, 0),
    /* ★ ABI 24, DIAGNOSTIC (2026-10-09, OWNER_RULINGS.md sec. X; unset = off): the VFIO reference's
     * GSP observer on this device — vfio-pci's property name, the same code (hw/vfio/gsp-observer.c,
     * tools/vfio-gsp-observer) and the same output file format. A fresh path (created O_EXCL). The
     * capture window from realize, 1..7200 s (vfio-pci's is fixed at 300). */
    DEFINE_PROP_STRING("x-gsp-observer", Kf3State, gsp_observer_path),
    DEFINE_PROP_UINT32("x-gsp-observer-seconds", Kf3State, gsp_observer_seconds, 300),
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
