/* The kf3_* surface of libkf_qemu.a (crates/kf-qemu/src/ffi_unsafe.rs). Keep in lockstep: the
 * device refuses an archive whose kf3_abi_version() differs from KF3_ABI. */
#ifndef KF3_H
#define KF3_H
#include <stdint.h>
#include <stddef.h>

#define KF3_ABI 7

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

uint32_t kf3_abi_version(void);
int32_t kf3_realize(uint32_t gpu_minor, uint64_t fb_mb, uint64_t bar1_bytes, uint64_t bar2_bytes,
                    const char *guest_driver, void **out,
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
void kf3_unrealize(void *h);
#endif
