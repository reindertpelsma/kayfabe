/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#ifndef KF_GSP_TRACE_PROTOCOL_H
#define KF_GSP_TRACE_PROTOCOL_H
#include <stdint.h>
#define KFGT_ABI 1u
#define KFGT_FILE_MAGIC 0x5457474bu /* KGWT */
#define KFGT_RECORD_MAGIC 0x5247474bu /* KGGR */
#define KFGT_SAMPLED 1u /* Always set: passive observation cannot prove completeness. */
#define KFGT_PREFIX_UNKNOWN 2u
#define KFGT_GAP_BEFORE 4u
#define KFGT_PAGE 4096u
#define KFGT_MAX_PAGES 512u
#define KFGT_MAX_QUEUE (1024u * 1024u)
#define KFGT_MAX_MESSAGE 65536u
#define KFGT_RPC_SIGNATURE 0x43505256u
#define KFGT_IOCTL_STATS 0x00226000u /* CTL_CODE(FILE_DEVICE_UNKNOWN,0x800,METHOD_BUFFERED,FILE_READ_DATA) */
#define KFGT_IOCTL_STOP  0x00226008u
#define KFGT_IOCTL_READ  0x00226004u
/* All integer fields little endian; structs deliberately have no implicit padding. */
typedef struct {
    uint32_t magic, version, header_bytes, record_header_bytes;
    uint64_t qpc_frequency, started_qpc;
    uint32_t flags, reserved;
    uint64_t reserved2[3];
} KFGT_FILE_HEADER;
typedef struct {
    uint32_t magic, header_bytes, payload_bytes, direction;
    uint64_t qpc, table_pa;
    uint32_t queue_sequence, rpc_sequence, rpc_function, rpc_result;
    uint32_t flags, missing_before, rpc_version, reserved;
} KFGT_RECORD;
typedef struct {
    uint32_t version, bytes;
    uint64_t qpc_frequency, started_qpc;
    uint64_t scanned_bytes, scan_passes, candidates, attached_tables;
    uint64_t read_failures, unstable_snapshots, invalid_elements;
    uint64_t recorded, dropped, observed_sequence_gaps, buffered_bytes;
} KFGT_STATS;
#endif
