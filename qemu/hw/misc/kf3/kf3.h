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
 * surfaces, so an archive from either branch must be refused here: one new number above both. */
/* ★ 11 (2026-10-03, v3-broker, display step 3 — docs/design/V3_DISPLAY.md §8): kf3_realize gains
 * display_broker, and the broker relay's surface (Kf3BrokerEvent, the two verbs, kf3_broker_*). */
#define KF3_ABI 11

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

/* ★ ABI 11: one input event from the display broker, already bounded by Rust (kf_broker::Input).
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

uint32_t kf3_abi_version(void);
/* ★ ABI 8: `display` (0/1) — the virtual NVDisplay (docs/design/V3_DISPLAY.md). */
/* ★ ABI 11: `display_broker` (0/1) — back the display's frames with memfds the broker can receive. */
int32_t kf3_realize(uint32_t gpu_minor, uint64_t fb_mb, uint64_t bar1_bytes, uint64_t bar2_bytes,
                    const char *guest_driver, uint32_t display, uint32_t display_broker, void **out,
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
/* ★ ABI 11 (display step 3): the display-broker relay. Main loop only, BQL held. Rust owns the
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
void kf3_unrealize(void *h);
#endif
