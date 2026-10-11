/* The kf3_* surface of libkf_qemu.a (crates/kf-qemu/src/ffi_unsafe.rs). Keep in lockstep: the
 * device refuses an archive whose kf3_abi_version() differs from KF3_ABI, and
 * crates/kf-qemu/tests/wire_mirror.rs compiles every Rust layout, entry point and callback type
 * against the declarations below (a name on one side only, or a differing signature, fails it). */
#ifndef KF3_H
#define KF3_H
#include <stdint.h>
#include <stddef.h>

/* ★ 10 (2026-09-30, v3-mc22): the union of two INDEPENDENT 9s — v3-display2's frame hand-off
 * (Kf3Frame, kf3_display_frame) and v3-ioeventfd's doorbell fast path (Kf3IoeventfdFn,
 * kf3_doorbell_page_offset, kf3_set_ioeventfd, kf3_doorbell_site). The two 9s name DIFFERENT
 * surfaces, so an archive from either branch must be refused here: one new number above both.
 * ★ 11 (2026-10-03, v3-gop-kf3, docs/design/V3_DISPLAY.md §4.11): the boot display — kf3_realize
 * takes `gop`, and kf3_option_rom hands over the option ROM Rust packed for this device. */
/* ★ 12 (2026-10-03, v3-broker, display step 3 — docs/design/V3_DISPLAY.md §8): ONE number above
 * master's 11 for the whole broker surface. ⊘ The branch had numbered its steps 11, 12 and 13 before
 * master's boot display took 11; the merge folds them into this one bump. kf3_realize gains
 * display_broker (after gop), whose word carries the broker in bit 0 and display-broker-vram in bits
 * 1-2 (KF3_BROKER_VRAM_*, the GPU-copy rung, sec. 8.11); the broker relay's surface (Kf3BrokerEvent,
 * the two verbs, kf3_broker_*); kf3_display_ui_info (3c, the console's ui_info hook) and the
 * broker's SURFACE event (KF3_BROKER_SURFACE). (Its closing "v3-dispsw-exp takes 13 when it merges"
 * is CORRECTED below: 13 is v3-dispsw-exp's own number, already in box binaries.) */
/* ★ 16 (2026-10-04, v3-maxfps — docs/design/V3_DISPLAY.md sec. 8.16, OWNER_RULINGS sec. M): the
 * configurable frame-rate bound. kf3_realize gains display_max_fps (after display_broker; whole Hz,
 * 0 = unset), and the console's on-demand refresh joins the surface: kf3_display_refresh,
 * kf3_display_refresh_fd, kf3_display_refresh_drain (gfx_update is asynchronous: a screendump waits
 * for a frame no older than its request).
 * THE REGISTRY (one number per realize/surface shape that ever reached a binary; never reused):
 *   11 master (the boot display, GOP);
 *   12 v3-broker (v3-windows also took 12; it is renumbered at its merge);
 *   13 v3-dispsw-exp (x11_dispsw);
 *   14 reserved: broker-on-13 (v3-cand-1);
 *   15 v3-viommu;
 *   16 v3-maxfps (this), cut from v3-broker 82f98f42 — a merge with 13, 14 or 15 takes a new one. */
/* 17 was used by the earlier display scratch integration (different argument order).
 * 18 (2026-10-04, candidate 2): ABI 13's x11_dispsw plus ABI 16's broker/cursor/refresh.
 * The realize tail is gop, x11_dispsw, display_broker, display_max_fps. */
/* 19 (2026-10-05, Windows/P1/P2 integration): append gop_efi after
 * display_max_fps, retaining every ABI-18 display/broker entry point. */
/* 20 (2026-10-05): ABI 19 plus HostTimer disposition and kf3_timer_view.
 * The narrow Windows timer branch used 13; it cannot name this combined surface. */
/* 21 (2026-10-07, claude/code43-trace-20261007): ABI 20 plus the default-off KF3_BAR0_TRACE
 * diagnostic's read-trap verb (Kf3ReadTrapFn, kf3_set_read_trap; OWNER_RULINGS.md sec. S, the
 * diagnostic BAR0-read trap exception). */
/* 22 (2026-10-08, claude/gpu-uuid-per-vm-20261008): ABI 21 plus the per-VM GPU UUID. kf3_realize
 * gains gpu_uuid and vm_id (nullable strings) and pci_devfn after gop_efi. */
/* 24 (2026-10-09, claude/kf3-read-trace-20261008): ABI 22 plus the default-off BAR0 trace mode
 * (KF3_BAR0_READ_TRACE; OWNER_RULINGS.md sec. X): kf3_trace_mode, kf3_trace_piece, kf3_trace_admit,
 * kf3_trace_name, kf3_trace_report. 23 is another branch's (0da871c1, the input/cursor shim). */
/* ★ 23 (2026-10-08, claude/input-sink-trait-20261008; OWNER_RULINGS.md sec. V,
 * docs/design/V3_DISPLAY.md sec. 8.20): ABI 22 with the broker's input and the console cursor
 * behind kf-broker's VMM-neutral InputSink/CursorSink. This device no longer interprets events:
 * kf3_broker_start takes Kf3InputOps (its input and cursor verbs, each one QEMU call), which Rust
 * calls from inside kf3_broker_ready and kf3_display_cursor_apply. Gone: Kf3BrokerEvent and
 * KF3_BROKER_* kinds, Kf3Cursor and KF3_CURSOR_DEFINE/MOUSE, kf3_display_cursor,
 * kf3_display_cursor_pixels, kf3_display_cursor_done, and kf3_broker_ready's event array. */
/* 25 (2026-10-09, merge of both at claude/windows-reset-20261009): 24's trace verbs AND 23's
 * input/cursor verbs; the two surfaces are disjoint. */
/* 26 (2026-10-11, claude/gl-icd-crash-20261011): 25 plus the channel-budget property: kf3_realize gains
 * channel_budget (channels per runlist, the count the guest is told AND the enforced twin cap; 0 = derive
 * from the host) after pci_devfn. Rust refuses a value above the host's limit or below the minimum, by name. */
#define KF3_ABI 26
#define KF3_BROKER_ON 1u
#define KF3_BROKER_VRAM_AUTO 0u
#define KF3_BROKER_VRAM_ON 1u
#define KF3_BROKER_VRAM_OFF 2u
#define KF3_BROKER_VRAM_SHIFT 1

typedef struct Kf3Identity {
    uint16_t vendor, device, subsystem_vendor, subsystem;
    uint32_t class_code;
    uint8_t revision, pad[3];
    uint64_t bar0_bytes;
} Kf3Identity;

typedef struct Kf3Region {
    uint8_t bar, how, pad[6];   /* how: 0 RAM, 1 shadow, 2 usermode, 3 hole, 4 timer */
    uint64_t base, len;
} Kf3Region;

/* ★ ABI 10 (v3-display2's 9): one frame of the virtual display (display=on). `data` stays valid and unwritten until
 * the next kf3_display_frame call. format: 1 xrgb8888, 2 xbgr8888, 3 rgb565, 4 x2rgb10, 5 x2bgr10. */
typedef struct Kf3Frame {
    uint8_t* data;   /* (spelled for the wire-mirror census) */
    uint32_t width, height, stride, format;
    uint64_t serial;
} Kf3Frame;

/* ★ ABI 23 (OWNER_RULINGS.md sec. V, docs/design/V3_DISPLAY.md sec. 8.20): one pointing device the
 * guest has, as the pointers verb lists it. id: QEMU's mouse index (qemu_mouse_set); absolute: 1 a
 * tablet, 0 a mouse; paravirtual: 1 a virtio-input device; name: NUL-terminated or cut (log only). */
typedef struct Kf3Pointer {
    uint32_t id;
    uint8_t absolute, paravirtual, pad[2];
    char name[56];
} Kf3Pointer;
#define KF3_CURSOR_MAX_DIM 256
/* Kf3InputOps' button numbers (kf_broker::Button). */
#define KF3_BTN_LEFT 0
#define KF3_BTN_RIGHT 1
#define KF3_BTN_MIDDLE 2
#define KF3_BTN_SIDE 3
#define KF3_BTN_EXTRA 4

/* ★ ABI 23: this device's input and console-cursor verbs — QEMU's half of kf-broker's
 * VMM-neutral InputSink and CursorSink. Rust decides every policy (which keys and buttons, which
 * pointing device on grab, sync points, the missing-device warning, define/hide/move); each verb
 * is the QEMU call that carries one decision out. Called only from inside kf3_broker_ready and
 * kf3_display_cursor_apply (main loop, BQL held); none may block or re-enter a kf3_* entry.
 * relative: 1 = the relative pointing device, 0 = the absolute one. Pointer verbs (button, wheel,
 * abs, rel) are QUEUED until sync; key is a whole report. Each int32_t return is 1 = applied. */
typedef int32_t (*Kf3InKeyFn)(void *opaque, uint32_t evdev, uint32_t down);
typedef void (*Kf3InButtonFn)(void *opaque, uint32_t button, uint32_t down, uint32_t relative);
typedef void (*Kf3InWheelFn)(void *opaque, int32_t dx, int32_t dy, uint32_t relative);
typedef void (*Kf3InAbsFn)(void *opaque, uint32_t x, uint32_t y, uint32_t width, uint32_t height);
typedef void (*Kf3InRelFn)(void *opaque, int32_t dx, int32_t dy);
typedef void (*Kf3InSyncFn)(void *opaque);
typedef uint32_t (*Kf3InPointersFn)(void *opaque, Kf3Pointer *out, uint32_t cap);
typedef void (*Kf3InSelectFn)(void *opaque, uint32_t id, uint32_t relative);
typedef void (*Kf3InMissingFn)(void *opaque, uint32_t relative);
typedef void (*Kf3InCloseFn)(void *opaque, uint32_t force);
typedef void (*Kf3InResizeFn)(void *opaque, uint32_t width, uint32_t height, uint32_t refresh_mhz);
/* the console's cursor: width x height in 1..KF3_CURSOR_MAX_DIM, the hot spot inside, pixels
 * exactly width*height premultiplied 0xAARRGGBB words valid for the call */
typedef int32_t (*Kf3CurDefineFn)(void *opaque, uint32_t width, uint32_t height, uint32_t hot_x,
                                  uint32_t hot_y, const uint32_t *pixels);
typedef int32_t (*Kf3CurHideFn)(void *opaque);
typedef int32_t (*Kf3CurMoveFn)(void *opaque, int32_t x, int32_t y);
typedef uint32_t (*Kf3CurAbsoluteFn)(void *opaque);
typedef struct Kf3InputOps {
    Kf3InKeyFn key;
    Kf3InButtonFn button;
    Kf3InWheelFn wheel;
    Kf3InAbsFn abs;
    Kf3InRelFn rel;
    Kf3InSyncFn sync;
    Kf3InPointersFn pointers;
    Kf3InSelectFn select_pointer;
    Kf3InMissingFn missing_pointer;
    Kf3InCloseFn close;
    Kf3InResizeFn resize_hint;
    Kf3CurDefineFn cursor_define;
    Kf3CurHideFn cursor_hide;
    Kf3CurMoveFn cursor_move;
    Kf3CurAbsoluteFn cursor_absolute;
} Kf3InputOps;

uint32_t kf3_abi_version(void);
/* ★ ABI 8: `display` (0/1) — the virtual NVDisplay (docs/design/V3_DISPLAY.md).
 * ★ ABI 11: `gop` (0/1) — the boot display (§4.11); needs display=1.
 * ABI 18 also retains x11_dispsw (0/1), default off, requiring display=1.
 * ★ ABI 12: `display_broker` — bit 0 backs the display's frames for the broker, bits 1-2 are
 * display-broker-vram (§8.11).
 * ★ ABI 16: `display_max_fps` — the cap on every head's emulated vblank tick, whole Hz, 24..75;
 * 0 = unset (cap 75, today's EDID). Rust refuses any other value, and a non-zero one without
 * display=1, by name (§8.16).
 * ABI 19: gop_efi is a signed copy of the embedded GOP driver or NULL; needs gop=1.
 * ABI 22: gpu_uuid is the gpu-uuid property (auto|random|host|GPU-xxxxxxxx-...; NULL = auto);
 * vm_id is the VM's identity text (the vm-id property, else QEMU's -uuid, else NULL);
 * pci_devfn is the device's guest PCI devfn. Rust validates all three and refuses by name. */
int32_t kf3_realize(uint32_t gpu_minor, uint64_t fb_mb, uint64_t bar1_bytes, uint64_t bar2_bytes,
                    const char *guest_driver, uint32_t display, uint32_t gop, uint32_t x11_dispsw,
                    uint32_t display_broker,
                    uint32_t display_max_fps, const char *gop_efi,
                    const char *gpu_uuid, const char *vm_id, uint32_t pci_devfn, uint32_t channel_budget, void **out,
                    char *err, size_t err_len);
int32_t kf3_identity(void *h, Kf3Identity *out);
/* ★ ABI 7: config-space words the guest reads by config cycle (Hopper+ PCIe link caps). */
int32_t kf3_config_word(void *h, uint32_t idx, uint16_t *off, uint32_t *val);
int32_t kf3_usermode_view(void *h, void **ptr, uint64_t *len);
int32_t kf3_timer_view(void *h, void **ptr, uint64_t *len);
int64_t kf3_memory_map(void *h, uint64_t bar1, uint64_t bar2, Kf3Region *out, size_t cap);
int32_t kf3_shadow_attach(void *h, uint64_t base, uint8_t *mem, uint64_t len);
void kf3_shadow_seal(void *h);
void kf3_bar0_write(void *h, uint64_t off, uint64_t val, uint32_t width);
/* ★ w828: a BAR0 read exit (HOLE pages only: a read with a side effect). */
uint64_t kf3_bar0_read(void *h, uint64_t off, uint32_t width);
int32_t kf3_ram_add(void *h, uint64_t gpa, uint8_t *hva, uint64_t len, int32_t fd, uint64_t fd_off);
int32_t kf3_bar_ram(void *h, uint32_t bar, uint64_t base, uint64_t len, void **ptr);
void kf3_ram_del(void *h, uint64_t gpa);
void kf3_status(void *h, char *buf, size_t len);
int32_t kf3_irq_fd(void *h, uint32_t vector);
/* Hopper+ BAR1 usermode views (docs/design/V3_BAR1_DOORBELL.md). */
typedef int32_t (*Kf3OverlayFn)(void *opaque, uint64_t seq, uint32_t op, uint64_t base, uint64_t len,
                                uint64_t vf_rel);
int32_t kf3_bar1_follows_guest(void *h);
int32_t kf3_set_bar1_overlay(void *h, Kf3OverlayFn f, void *opaque, uint32_t slots);
void kf3_bar1_overlay_done(void *h, uint64_t seq, int32_t rc);
void kf3_bar1_usermode_write(void *h, uint64_t vf_rel, uint64_t val, uint32_t width);
/* ★ ABI 21, DIAGNOSTIC (KF3_BAR0_TRACE, default off; OWNER_RULINGS.md sec. S, 2026-10-07): the
 * device's BAR0 read-trap verb. on = 1: ROMD off on every shadow piece (their reads exit to
 * kf3_bar0_read, which answers from the same shadow); 0: ROMD back on. Never waits: it schedules a
 * main-loop bottom half. Rust calls it only from the register drainer, only with the flag on. */
typedef void (*Kf3ReadTrapFn)(void *opaque, uint32_t on);
int32_t kf3_set_read_trap(void *h, Kf3ReadTrapFn f, void *opaque);
/* ★ ABI 24, DIAGNOSTIC (KF3_BAR0_READ_TRACE, default off; crates/kf-qemu/src/readtrace.rs): the BAR0
 * trace mode. kf3_trace_mode: 1 on, 0 off (then NONE of the trace paths exist: ROMD on, irqfd MSI,
 * no trace call), -1 bad handle. kf3_trace_piece: 1 when a shadow piece's reads must exit.
 * kf3_trace_admit (lock-free; vCPU or main loop): kind 0 read / 1 write (a offset, b width,
 * c value), 2 MSI (a vector, b data, c address); 1 = write the record, 0 = unselected or capped.
 * kf3_trace_name: the device name in the records; kf3_trace_report: the exit report line. */
int32_t kf3_trace_mode(void *h);
uint32_t kf3_trace_piece(void *h, uint64_t base, uint64_t len);
uint32_t kf3_trace_admit(void *h, uint32_t kind, uint64_t a, uint64_t b, uint64_t c);
void kf3_trace_name(void *h, char *buf, size_t len);
void kf3_trace_report(void *h, char *buf, size_t len);
/* ★ ABI 10 (v3-display2's 9): the newest display frame, for the console's gfx_update (main thread);
 * -1 = none yet. */
int32_t kf3_display_frame(void *h, Kf3Frame *out);
/* ★ ABI 10 (v3-ioeventfd's 9; docs/design/V3_DOORBELL_IOEVENTFD.md): the doorbell fast path. The
 * device's KVM_IOEVENTFD verb (0 or -errno; any non-vCPU thread), the doorbell register's offset
 * inside the usermode page, and the doorbell sites the memory listener sees (BAR0's usermode piece,
 * Hopper+ BAR1 views). */
typedef int32_t (*Kf3IoeventfdFn)(void *opaque, uint64_t gpa, uint32_t len, uint64_t datamatch, int32_t fd,
                                  uint32_t assign);
int64_t kf3_doorbell_page_offset(void *h);
int32_t kf3_set_ioeventfd(void *h, Kf3IoeventfdFn f, void *opaque, uint32_t budget);
void kf3_doorbell_site(void *h, uint64_t gpa, uint32_t add);
/* ★ ABI 12 (display step 3): the display-broker relay. Main loop only, BQL held. Rust owns the
 * socket; it calls `watch` (fd handlers; (0, 0) BEFORE it closes the fd) and `timer`
 * (QEMU_CLOCK_REALTIME ms, -1 = none) back only from inside these entries. ★ ABI 23: `ops` is
 * this device's input and cursor verbs (copied; every one must be set). kf3_broker_ready: `fd` is
 * the socket, the frame eventfd, or -1 for the timer; it DELIVERS the input through `ops` and
 * brings the console cursor along; returns the inputs delivered. */
typedef void (*Kf3BrokerWatchFn)(void *opaque, int32_t fd, uint32_t read, uint32_t write);
typedef void (*Kf3BrokerTimerFn)(void *opaque, int64_t deadline_ms);
int32_t kf3_broker_start(void *h, const char *path, int64_t extra_uid, Kf3BrokerWatchFn watch,
                         Kf3BrokerTimerFn timer, const Kf3InputOps *ops, void *opaque,
                         uint64_t now_ms, char *err, size_t err_len);
int32_t kf3_broker_frame_fd(void *h);
int32_t kf3_broker_ready(void *h, int32_t fd, uint32_t rd, uint32_t wr, uint64_t now_ms);
void kf3_broker_stop(void *h);
/* ★ §8.13, ⊘ ABI 23: the console's cursor in hover (main loop: gfx_update after its frame, and the
 * refresh answer) — Rust decides and calls the cursor verbs given at kf3_broker_start; returns the
 * parts handed out (0: nothing, no display, or no broker). */
int32_t kf3_display_cursor_apply(void *h);
/* ★ ABI 16 (§8.16, main loop): the console's on-demand refresh. kf3_display_refresh asks for a
 * frame no older than now: 1 = the worker will make kf3_display_refresh_fd readable when the newest
 * frame is (call kf3_display_refresh_drain, show the frame, end the wait); 0 = no answer will come
 * (answer the waiter at once). kf3_display_refresh_fd is -1 without a display. */
int32_t kf3_display_refresh(void *h);
int32_t kf3_display_refresh_fd(void *h);
void kf3_display_refresh_drain(void *h);
/* ★ ABI 12 (display step 3c): the console's ui_info — a resize hint for `head` (main loop). */
int32_t kf3_display_ui_info(void *h, uint32_t head, uint32_t width, uint32_t height, uint32_t refresh_mhz);
/* ★ ABI 11 (docs/design/V3_DISPLAY.md §4.11.6): the boot display's option ROM (gop=1) — the
 * embedded GOP driver wrapped with this device's ids and its KFGP descriptor. 0 and *rom, *rom_len
 * (valid for the process; the device copies them into its ROM BAR), or -1 (gop=0). */
int32_t kf3_option_rom(void *h, const uint8_t **rom, uint64_t *rom_len);
void kf3_unrealize(void *h);
#endif
