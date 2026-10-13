/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * The cell sidecar (cell.json): what a capture was made of, and nothing about whose machine it
 * was. Fixed key set (the allow-list below is also checked by a test and by guard_readonly.sh):
 * PCI vendor/device/revision of the die, the Windows build, the display driver's version string
 * as the registry gives it, the decode tag, the module and engine versions, the discovery
 * outcome and counters. No UUID, serial, VBIOS, MAC, PCI subsystem id, bus/device/function,
 * host name, user name, IP address or path.
 */
#ifndef KFWC_META_H
#define KFWC_META_H

#include "../driver/kfwc_ioctl.h"
#include <stddef.h>

typedef struct kfwc_meta {
    const char *windows_build;       /* e.g. "26100" */
    const char *windows_ubr;         /* e.g. "1742" */
    const char *windows_display_version;  /* e.g. "24H2" */
    const char *driver_version;      /* display driver DriverVersion, verbatim, e.g. "32.0.15.8088" */
    const char *decode_tag;          /* ogkm tag the cell decodes with ("" when unknown) */
    kfwc_info info;                  /* from the driver (PCI ids, BAR state, frequency) */
    kfwc_stats_out stats;            /* final counters */
    const kfwc_event *events;        /* diagnostic events drained from the driver */
    kfwc_u32 n_events;
    kfwc_u32 records_total, records_by_kind[8];
    kfwc_u32 file_ok;                /* framing + counter + END checks passed */
    kfwc_u32 end_seen;
} kfwc_meta;

/* The complete list of keys the sidecar may contain (top level and nested). */
extern const char *const kfwc_meta_allowed_keys[];

/* Copy printable ASCII only, escaping '"' and '\\'. Returns 0 when src was clean and fit. */
int kfwc_json_escape(char *dst, size_t cap, const char *src);

/* Format the sidecar. Returns bytes written (without NUL) or 0 if cap is too small. */
size_t kfwc_meta_format(char *out, size_t cap, const kfwc_meta *m);

#endif /* KFWC_META_H */
