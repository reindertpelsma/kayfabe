/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#include "kfwc_meta.h"
#include <stdio.h>
#include <string.h>

const char *const kfwc_meta_allowed_keys[] = {
    "schema", "module", "version", "engine_abi", "windows", "build", "ubr", "display_version",
    "display_driver", "driver_version", "decode_tag", "die", "pci_vendor_id", "pci_device_id",
    "pci_revision_id", "discovery", "bar0", "gpu_count", "ring", "bytes", "capture", "records",
    "end_seen", "file_ok", "by_kind", "bind", "elements", "loss_overflow", "loss_unreadable",
    "loss_unknown", "end", "stats", "steps", "attaches", "detaches", "messages_cmd", "messages_status",
    "lost_messages_exact", "loss_records", "torn_reads", "invalid_elements", "unsupported",
    "read_failures", "resyncs", "sink_dropped", "records_refused", "scan_bytes", "scan_passes",
    "scan_candidates", "mailbox_reads", "mailbox_status", "method", "layout", "seq_gap_messages",
    "lapped_messages", "ring_refused", "ring_high_water", "events", "code", "generation", "data_hex",
    "ticks_per_second", "started_qpc", 0};

int kfwc_json_escape(char *dst, size_t cap, const char *src)
{
    size_t n = 0;
    int dirty = 0;
    if (!cap)
        return 1;
    for (; *src; src++) {
        unsigned char c = (unsigned char)*src;
        if (c < 0x20 || c >= 0x7f) {
            dirty = 1;
            continue;
        }
        if (c == '"' || c == '\\') {
            if (n + 2 >= cap) {
                dirty = 1;
                break;
            }
            dst[n++] = '\\';
        } else if (n + 1 >= cap) {
            dirty = 1;
            break;
        }
        dst[n++] = (char)c;
    }
    dst[n] = 0;
    return dirty;
}

size_t kfwc_meta_format(char *out, size_t cap, const kfwc_meta *m)
{
    char bld[64], ubr[32], dv[64], drv[96], tag[64];
    size_t n = 0, i;
    int w;
    const kfwc_stats *s = &m->stats.engine;
    kfwc_json_escape(bld, sizeof(bld), m->windows_build ? m->windows_build : "");
    kfwc_json_escape(ubr, sizeof(ubr), m->windows_ubr ? m->windows_ubr : "");
    kfwc_json_escape(dv, sizeof(dv), m->windows_display_version ? m->windows_display_version : "");
    kfwc_json_escape(drv, sizeof(drv), m->driver_version ? m->driver_version : "");
    kfwc_json_escape(tag, sizeof(tag), m->decode_tag ? m->decode_tag : "");
#define EMIT(...)                                                              \
    do {                                                                       \
        w = snprintf(out + n, cap - n, __VA_ARGS__);                           \
        if (w < 0 || (size_t)w >= cap - n)                                     \
            return 0;                                                          \
        n += (size_t)w;                                                        \
    } while (0)
    EMIT("{\n \"schema\": \"kfwc-cell/1\",\n");
    EMIT(" \"module\": {\"version\": %u, \"engine_abi\": %u},\n", (unsigned)m->info.module_version,
         (unsigned)m->info.engine_abi);
    EMIT(" \"windows\": {\"build\": \"%s\", \"ubr\": \"%s\", \"display_version\": \"%s\"},\n", bld, ubr, dv);
    EMIT(" \"display_driver\": {\"driver_version\": \"%s\"},\n", drv);
    EMIT(" \"decode_tag\": \"%s\",\n", tag);
    EMIT(" \"die\": {\"pci_vendor_id\": \"0x%04x\", \"pci_device_id\": \"0x%04x\", \"pci_revision_id\": \"0x%02x\"},\n",
         (unsigned)m->info.pci_vendor_id, (unsigned)m->info.pci_device_id, (unsigned)m->info.pci_revision_id);
    EMIT(" \"discovery\": {\"method\": %llu, \"layout\": %llu, \"bar0\": %u, \"gpu_count\": %u, "
         "\"mailbox_reads\": %llu, \"mailbox_status\": %llu, \"scan_bytes\": %llu, \"scan_passes\": %llu, "
         "\"scan_candidates\": %llu},\n",
         (unsigned long long)s->method, (unsigned long long)s->layout, (unsigned)m->info.bar_state,
         (unsigned)m->info.gpu_count, (unsigned long long)s->mailbox_reads, (unsigned long long)s->mailbox_status,
         (unsigned long long)s->scan_bytes, (unsigned long long)s->scan_passes,
         (unsigned long long)s->scan_candidates);
    EMIT(" \"ring\": {\"bytes\": %u, \"ring_refused\": %llu, \"ring_high_water\": %llu},\n",
         (unsigned)m->info.ring_bytes, (unsigned long long)m->stats.ring_refused,
         (unsigned long long)m->stats.ring_high_water);
    EMIT(" \"capture\": {\"records\": %u, \"end_seen\": %u, \"file_ok\": %u, \"by_kind\": {\"bind\": %u, "
         "\"elements\": %u, \"loss_overflow\": %u, \"loss_unreadable\": %u, \"loss_unknown\": %u, \"end\": %u}},\n",
         (unsigned)m->records_total, (unsigned)m->end_seen, (unsigned)m->file_ok, (unsigned)m->records_by_kind[1],
         (unsigned)m->records_by_kind[2], (unsigned)m->records_by_kind[3], (unsigned)m->records_by_kind[4],
         (unsigned)m->records_by_kind[5], (unsigned)m->records_by_kind[6]);
    EMIT(" \"stats\": {\"steps\": %llu, \"attaches\": %llu, \"detaches\": %llu, \"messages_cmd\": %llu, "
         "\"messages_status\": %llu, \"lost_messages_exact\": %llu, \"seq_gap_messages\": %llu, "
         "\"lapped_messages\": %llu, \"loss_records\": %llu, \"torn_reads\": %llu, \"invalid_elements\": %llu, "
         "\"unsupported\": %llu, \"read_failures\": %llu, \"resyncs\": %llu, \"sink_dropped\": %llu, "
         "\"records_refused\": %llu},\n",
         (unsigned long long)s->steps, (unsigned long long)s->attaches, (unsigned long long)s->detaches,
         (unsigned long long)s->messages[0], (unsigned long long)s->messages[1],
         (unsigned long long)s->lost_messages_exact, (unsigned long long)s->seq_gap_messages,
         (unsigned long long)s->lapped_messages, (unsigned long long)s->loss_records,
         (unsigned long long)s->torn_reads, (unsigned long long)s->invalid_elements,
         (unsigned long long)s->unsupported, (unsigned long long)s->read_failures, (unsigned long long)s->resyncs,
         (unsigned long long)s->sink_dropped, (unsigned long long)s->records_refused);
    EMIT(" \"events\": [");
    for (i = 0; i < m->n_events; i++) {
        char hex[2 * 48 + 1];
        size_t k;
        for (k = 0; k < 48; k++)
            snprintf(hex + 2 * k, 3, "%02x", m->events[i].data[k]);
        EMIT("%s\n  {\"code\": %u, \"generation\": %u, \"data_hex\": \"%s\"}", i ? "," : "",
             (unsigned)m->events[i].code, (unsigned)m->events[i].generation, hex);
    }
    EMIT("%s],\n", m->n_events ? "\n " : "");
    EMIT(" \"ticks_per_second\": %llu, \"started_qpc\": %llu\n}\n", (unsigned long long)m->info.qpc_frequency,
         (unsigned long long)m->info.started_qpc);
#undef EMIT
    return n;
}
