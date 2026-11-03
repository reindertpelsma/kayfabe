/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * Interface between the kfwc driver and its user-mode companion. Shared by both builds.
 *
 * The device is read-only in the sense the owner asked for: the companion opens it with
 * GENERIC_READ (a request for write access is refused at CREATE), records come out through
 * ReadFile, and the control codes below configure the module's OWN behaviour only. There is no
 * code that takes an address from the caller, reads an arbitrary physical address, or writes
 * anything to the GPU.
 */
#ifndef KFWC_IOCTL_H
#define KFWC_IOCTL_H

#include "../lib/kfwc_engine.h"

#define KFWC_DRIVER_ABI 3u
#define KFWC_MODULE_VERSION 1u

/* CTL_CODE(FILE_DEVICE_UNKNOWN, fn, METHOD_BUFFERED, FILE_READ_ACCESS), spelled out so that no
 * Windows header is needed to include this file; both TUs assert the equality. */
#define KFWC_CTL(fn) (0x00220000u | 0x00004000u | ((fn) << 2))
#define KFWC_IOCTL_INFO KFWC_CTL(0x800u)     /* out: kfwc_info */
#define KFWC_IOCTL_STATS KFWC_CTL(0x801u)    /* out: kfwc_stats_out */
#define KFWC_IOCTL_EVENTS KFWC_CTL(0x802u)   /* out: array of kfwc_event (drains them) */
#define KFWC_IOCTL_START KFWC_CTL(0x803u)    /* in: kfwc_start_req; resets the stream (counter 0) and starts */
#define KFWC_IOCTL_STOP KFWC_CTL(0x804u)     /* no buffers; stops, writes END; records stay readable */
#define KFWC_IOCTL_DISPLAY_CAPS KFWC_CTL(0x805u) /* no input; out: kfwc_display_caps_out */

typedef struct kfwc_display_caps_out {
    kfwc_u32 pci_vendor_id, pci_device_id, word_count, reserved;
    kfwc_u32 words[KFWC_DISPLAY_CAPS_WORDS];
} kfwc_display_caps_out;
KFWC_STATIC_ASSERT(kfwc_display_caps_out_is_fixed, sizeof(kfwc_display_caps_out) == 4112u);

#define KFWC_DEVICE_USER_PATH "\\\\.\\KayfabeWinCapture"
#define KFWC_MIN_READ_BYTES (70u * 1024u)    /* one record is at most 32 + 64 KiB */
#define KFWC_MAX_READ_BYTES (4u * 1024u * 1024u)

/* Why the BAR0 mapping is or is not available. */
#define KFWC_BAR_NONE 0u         /* no NVIDIA display device found */
#define KFWC_BAR_DISABLED 1u     /* found, memory decode off or not in D0: registers are not read */
#define KFWC_BAR_UNSTABLE 2u     /* BAR value changed between reads or looked invalid */
#define KFWC_BAR_MAPPED 3u       /* mapped read-only */
#define KFWC_BAR_MAP_FAILED 4u

typedef struct kfwc_info {          /* 64 bytes */
    kfwc_u32 driver_abi, engine_abi, module_version, gpu_count;
    kfwc_u32 pci_vendor_id, pci_device_id, pci_revision_id, bar_state;
    kfwc_u32 running, ring_bytes, ram_ranges, reserved0;
    kfwc_u64 qpc_frequency, started_qpc;
} kfwc_info;
KFWC_STATIC_ASSERT(kfwc_info_is_64, sizeof(kfwc_info) == 64);

#define KFWC_BACKOFF_OFF 0xFFFFFFFFu
#define KFWC_BACKOFF_INITIAL_MS_DEFAULT 5000u
#define KFWC_BACKOFF_MAX_MS_DEFAULT 300000u

typedef struct kfwc_start_req {     /* 48 bytes */
    kfwc_u32 abi;                   /* KFWC_DRIVER_ABI */
    kfwc_u32 use_mailbox, use_scan;
    kfwc_u32 layout;                /* 0 auto, 1 pre-610, 2 610 */
    kfwc_u32 poll_us;               /* sleep when idle: 200 .. 100000 (default 1000) */
    kfwc_u32 scan_period_us;        /* sleep between scan steps: 500 .. 1000000 (default 2000) */
    kfwc_u32 scan_bytes_per_step;   /* 4096 .. 1 MiB (default 1 MiB) */
    kfwc_u32 scan_delay_ms;         /* mailbox reads zero: wait before scanning (default 5000) */
    kfwc_u32 max_msgs_per_step;     /* 1 .. 256 (default 64) */
    kfwc_u32 gpu_index;             /* which NVIDIA display device when there are several */
    kfwc_u32 scan_backoff_initial_ms; /* pause after the first completed scan pass: 0 = 5000, KFWC_BACKOFF_OFF = none */
    kfwc_u32 scan_backoff_max_ms;   /* cap of the doubling pause: 0 = 300000 */
} kfwc_start_req;
KFWC_STATIC_ASSERT(kfwc_start_req_is_48, sizeof(kfwc_start_req) == 48);

typedef struct kfwc_stats_out {     /* 33 engine words + 8 ring/driver words */
    kfwc_stats engine;
    kfwc_u64 ring_puts, ring_refused, ring_bytes_in, ring_bytes_out, ring_high_water;
    kfwc_u64 loop_iterations, bar_remaps, pci_scans;
} kfwc_stats_out;

#endif /* KFWC_IOCTL_H */
