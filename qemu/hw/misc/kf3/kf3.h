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
 * broker's SURFACE event (KF3_BROKER_SURFACE). v3-dispsw-exp takes 13 when it merges. */
#define KF3_ABI 12
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
    uint8_t bar, how, pad[6];   /* how: 0 plain RAM, 1 shadow+write trap, 2 host passthrough, 3 hole */
    uint64_t base, len;
} Kf3Region;

/* ★ ABI 10 (v3-display2's 9): one frame of the virtual display (display=on). `data` stays valid and unwritten until
 * the next kf3_display_frame call. format: 1 xrgb8888, 2 xbgr8888, 3 rgb565, 4 x2rgb10, 5 x2bgr10. */
typedef struct Kf3Frame {
    uint8_t* data;   /* (spelled for the wire-mirror census) */
    uint32_t width, height, stride, format;
    uint64_t serial;
} Kf3Frame;

/* ★ ABI 12: one input event from the display broker, already bounded by Rust (kf_broker::Input).
 * kind: KF3_BROKER_* below. */
typedef struct Kf3BrokerEvent {
    uint32_t kind;
    int32_t x, y;   /* key/button code + pressed; x, y; dx, dy; wheel +1 up / -1 down; grab; force */
    uint32_t w0, w1; /* the absolute range (KF3_BROKER_ABS) */
} Kf3BrokerEvent;
#define KF3_BROKER_KEY 1
#define KF3_BROKER_BTN 2
#define KF3_BROKER_ABS 3
#define KF3_BROKER_REL 4
#define KF3_BROKER_WHEEL 5
#define KF3_BROKER_GRAB 6
#define KF3_BROKER_CLOSE 7
#define KF3_BROKER_SURFACE 8   /* x, y = the broker window's size; w0 = its refresh in mHz (0: unknown) */

/* ★ ABI 12 (docs/design/V3_DISPLAY.md §8.13): the guest's cursor for the console while a
 * cursor-capable broker hovers. what: KF3_CURSOR_DEFINE (width x height + hot spot; width 0 = the
 * hidden cursor; pixels from kf3_display_cursor_pixels), KF3_CURSOR_MOUSE (x, y, on). */
typedef struct Kf3Cursor {
    uint32_t what;
    uint32_t width, height, hot_x, hot_y;
    int32_t x, y;
    uint32_t on;
} Kf3Cursor;
#define KF3_CURSOR_DEFINE 1
#define KF3_CURSOR_MOUSE 2
#define KF3_CURSOR_MAX_DIM 256

uint32_t kf3_abi_version(void);
/* ★ ABI 8: `display` (0/1) — the virtual NVDisplay (docs/design/V3_DISPLAY.md).
 * ★ ABI 11: `gop` (0/1) — the boot display (§4.11); needs display=1.
 * ★ ABI 12: `display_broker` — bit 0 backs the display's frames for the broker, bits 1-2 are
 * display-broker-vram (§8.11). */
int32_t kf3_realize(uint32_t gpu_minor, uint64_t fb_mb, uint64_t bar1_bytes, uint64_t bar2_bytes,
                    const char *guest_driver, uint32_t display, uint32_t gop, uint32_t display_broker,
                    void **out,
                    char *err, size_t err_len);
int32_t kf3_identity(void *h, Kf3Identity *out);
/* ★ ABI 7: config-space words the guest reads by config cycle (Hopper+ PCIe link caps). */
int32_t kf3_config_word(void *h, uint32_t idx, uint16_t *off, uint32_t *val);
int32_t kf3_usermode_view(void *h, void **ptr, uint64_t *len);
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
 * (QEMU_CLOCK_REALTIME ms, -1 = none) back only from inside these entries. kf3_broker_ready: `fd`
 * is the socket, the frame eventfd, or -1 for the timer; returns the events written to `out`. */
typedef void (*Kf3BrokerWatchFn)(void *opaque, int32_t fd, uint32_t read, uint32_t write);
typedef void (*Kf3BrokerTimerFn)(void *opaque, int64_t deadline_ms);
int32_t kf3_broker_start(void *h, const char *path, int64_t extra_uid, Kf3BrokerWatchFn watch,
                         Kf3BrokerTimerFn timer, void *opaque, uint64_t now_ms, char *err,
                         size_t err_len);
int32_t kf3_broker_frame_fd(void *h);
int32_t kf3_broker_ready(void *h, int32_t fd, uint32_t rd, uint32_t wr, uint64_t now_ms,
                         Kf3BrokerEvent *out, uint32_t cap);
void kf3_broker_stop(void *h);
/* ★ ABI 12 (§8.13): the console's cursor in hover (main loop, gfx_update). kf3_display_cursor
 * returns out->what (0: nothing to do); after a DEFINE with width > 0, kf3_display_cursor_pixels
 * fills exactly width*height QEMUCursor words (0xAARRGGBB, straight alpha) or returns -1. */
int32_t kf3_display_cursor(void *h, Kf3Cursor *out);
int32_t kf3_display_cursor_pixels(void *h, uint32_t *data, uint32_t words);
/* ★ ABI 12 (display step 3c): the console's ui_info — a resize hint for `head` (main loop). */
int32_t kf3_display_ui_info(void *h, uint32_t head, uint32_t width, uint32_t height, uint32_t refresh_mhz);
/* ★ ABI 11 (docs/design/V3_DISPLAY.md §4.11.6): the boot display's option ROM (gop=1) — the
 * embedded GOP driver wrapped with this device's ids and its KFGP descriptor. 0 and *rom, *rom_len
 * (valid for the process; the device copies them into its ROM BAR), or -1 (gop=0). */
int32_t kf3_option_rom(void *h, const uint8_t **rom, uint64_t *rom_len);
void kf3_unrealize(void *h);
#endif
