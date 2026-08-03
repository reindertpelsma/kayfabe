/*
 * nv_rpctrace.h — on-the-wire format of the GSP-RM RPC message-queue trace,
 * and the hook prototype that `src/nvidia/.../message_queue_cpu.c` calls.
 *
 * SPDX-License-Identifier: MIT
 * Part of `kayfabe`. ⊘ Nothing in this file is copied from NVIDIA's tree; it is
 * dropped INTO a checkout of `open-gpu-kernel-modules` 580.159.04 by
 * `scripts/rpctrace/build_instrumented.sh`, which also applies `rpctrace.patch`.
 *
 * ★ ONE FILE, TWO DROP SITES. The build script copies this header to BOTH
 * `kernel-open/nvidia/` (the OS layer, which implements the recorder) and
 * `src/nvidia/inc/kernel/gpu/gsp/` (the RM, which calls the hooks). Those two
 * trees are compiled with different flags and cannot share an include path, so
 * the alternative was to repeat the constants and the prototype at the call
 * site — i.e. to create exactly the silent-drift seam this project keeps getting
 * bitten by. Copying one file twice cannot drift; the script re-copies on every
 * build and verifies both copies against this one.
 *
 * `NV_KERNEL_INTERFACE_LAYER` is defined by `kernel-open/Kbuild` and NOT by the
 * RM build, which is how the file knows which half it is being read for.
 *
 * ────────────────────────────────────────────────────────────────────────────
 * ★★★ THE ONE INVARIANT: A RECORD HAS ITS BYTES, OR IT DOES NOT EXIST.
 *
 * This recorder exists because the C artifact's captured control table
 * (`../nvidia-gpu-passthrough/src/qemu/mode2_initctrl_ga106.h`) has 56 rows of
 * which 11 (19.6%) carry `dlen = 0` — a declared length with no body — and every
 * one of those checked against a real GA106 is CONTRADICTED. `0x20802a08`
 * decodes from its empty row as size 0 where hardware answers 20480, and RM DMAs
 * into a buffer of exactly that size.
 *
 * So `cap_len` below is not "how many bytes the driver said there were". It is
 * "how many bytes FOLLOW THIS HEADER", written by the same all-or-nothing append
 * that wrote the header. There is no code path that advances the write cursor
 * having copied fewer bytes than `cap_len`, and no code path that emits a record
 * with `cap_len == 0`. The declared RPC length is carried SEPARATELY, in
 * `rpc_len`, precisely so that a disagreement between the two is visible to the
 * decoder rather than silently reconciled here.
 *
 * ★★ FILL-AND-STOP, NOT OVERWRITE. A conventional ring drops the OLDEST record
 * when it is full. For a boot trace that is the worst possible choice: the value
 * of this capture is its ordered prefix, and the consumer's diff is positional
 * (`nvkvm_m2_rec`'s rule — never sample, never cap, because one drop shifts every
 * later index). So the ring fills and then REFUSES, counting what it refused.
 * `n_dropped > 0` makes the trace unreplayable and the decoder rejects it.
 * ────────────────────────────────────────────────────────────────────────────
 */

#ifndef _NV_RPCTRACE_H_
#define _NV_RPCTRACE_H_

#if defined(NV_KERNEL_INTERFACE_LAYER)
#include <linux/types.h>
#endif

/* "NVRT" little-endian. */
#define NV_RPCTRACE_FILE_MAGIC   0x5452564EU
/* "RPCR" little-endian — repeated per record so a decoder can prove alignment. */
#define NV_RPCTRACE_REC_MAGIC    0x52435052U
#define NV_RPCTRACE_VERSION      1U

/* Directions. */
#define NV_RPCTRACE_DIR_CPU_TO_GSP  0U   /* GspMsgQueueSendCommand    */
#define NV_RPCTRACE_DIR_GSP_TO_CPU  1U   /* GspMsgQueueReceiveStatus  */

/* Per-record flags. */
#define NV_RPCTRACE_F_CC_ENABLED    0x0001U /* gpuIsCCFeatureEnabled() was true at
                                             * capture time. The hooks are placed so
                                             * that the bytes are PLAINTEXT either
                                             * way; this flag records that the
                                             * placement actually mattered here. */
#define NV_RPCTRACE_F_LEN_DISAGREE  0x0002U /* cap_len != HDR + rpc_len. The element
                                             * bytes are still all present; the
                                             * DECLARED length is what disagrees. */
#define NV_RPCTRACE_F_NOT_SENT      0x0004U /* CPU→GSP only: the element was composed
                                             * and captured, but the transport did not
                                             * complete (see `outcome`). It was NOT put
                                             * on the wire. A replay must skip it. */

/* File-level flags. */
#define NV_RPCTRACE_FF_OVERFLOWED   0x0001U /* ring hit its capacity; n_dropped > 0 */
#define NV_RPCTRACE_FF_DISABLED     0x0002U /* recorder was never armed (size 0) */

#define NV_RPCTRACE_VERSION_STR_LEN 32

/*
 * ★ THE HOOK, and why its prototype is outside the guard below.
 *
 * `message_queue_cpu.c` lives in `src/nvidia`, which is compiled OS-agnostically
 * and links to the OS layer by undefined symbol (`exports_link_command.txt`).
 * It gets the constants above and these three prototypes — nothing else. Every
 * argument is a plain C type so that the declaration the RM compiles and the
 * definition the kernel layer compiles cannot disagree about a width.
 *
 * `nv_rpctrace_record` returns the record's `seq`, or NV_RPCTRACE_NO_REC if
 * nothing was recorded.
 */
#define NV_RPCTRACE_NO_REC 0xFFFFFFFFU

unsigned int nv_rpctrace_record(unsigned int dir,
                                unsigned int rpc_fn,
                                unsigned int rpc_len,
                                unsigned int elem_seq,
                                unsigned int rpc_status,
                                unsigned int flags,
                                const void  *elem,
                                unsigned int cap_len);

/* Stamp the transport outcome onto a record already written. No-op unless `seq`
 * is still the most recent record. */
void nv_rpctrace_set_outcome(unsigned int seq, unsigned int status);

/* A GSP→CPU read that failed after all retries: there is no element, so there is
 * no record — only a counted hole the decoder must surface. */
void nv_rpctrace_note_rx_error(unsigned int status);

#if defined(NV_KERNEL_INTERFACE_LAYER)

/*
 * Emitted ONCE, at offset 0 of /proc/driver/nvidia/rpctrace. Snapshotted at
 * open(2) so the file a reader sees is internally consistent even if the driver
 * keeps recording underneath.
 */
struct nv_rpctrace_file_hdr {
    __u32 magic;              /* NV_RPCTRACE_FILE_MAGIC */
    __u32 version;            /* NV_RPCTRACE_VERSION    */
    __u32 file_hdr_size;      /* sizeof(struct nv_rpctrace_file_hdr) */
    __u32 rec_hdr_size;       /* sizeof(struct nv_rpctrace_rec_hdr)  */
    __u64 capacity;           /* ring bytes                          */
    __u64 used;               /* ring bytes that follow this header  */
    __u64 n_records;          /* records in `used`                   */
    __u64 n_payload_bytes;    /* sum of cap_len over those records   */
    __u64 n_dropped;          /* ★ records REFUSED for want of room  */
    __u64 n_dropped_bytes;
    __u64 n_refused_empty;    /* ★ calls with no bytes to record — see the header
                               * comment. These never become records. */
    __u64 n_rx_failed;        /* receives that failed after retries: a HOLE in the
                               * GSP→CPU stream with no bytes behind it. */
    __u64 t0_ns;              /* ktime_get_ns() at recorder init */
    __u32 flags;              /* NV_RPCTRACE_FF_* */
    __u32 reserved0;
    char  drv_version[NV_RPCTRACE_VERSION_STR_LEN]; /* NV_VERSION_STRING, NUL-padded */
};

/*
 * Immediately followed by `cap_len` bytes of the COMPLETE GSP_MSG_QUEUE_ELEMENT
 * (authTagBuffer .. rpc header .. rpc payload), then zero padding to the next
 * 8-byte boundary. `cap_len` counts the element bytes only, not the padding.
 */
struct nv_rpctrace_rec_hdr {
    __u32 magic;       /* NV_RPCTRACE_REC_MAGIC */
    __u16 dir;         /* NV_RPCTRACE_DIR_*     */
    __u16 flags;       /* NV_RPCTRACE_F_*       */
    __u32 seq;         /* recorder's own monotonic counter, both directions */
    __u32 elem_seq;    /* GSP_MSG_QUEUE_ELEMENT.seqNum (per-direction, driver's) */
    __u64 ts_ns;       /* ktime_get_ns() — monotonic */
    __u32 rpc_fn;      /* rpc_message_header_v.function   */
    __u32 rpc_len;     /* rpc_message_header_v.length — DECLARED, may disagree */
    __u32 rpc_status;  /* rpc_message_header_v.rpc_result */
    __u32 outcome;     /* NV_STATUS of the transport operation; 0 == NV_OK */
    __u32 cap_len;     /* ★ bytes that FOLLOW. Always the true count. Never 0. */
    __u32 reserved0;
};

struct proc_dir_entry;
int  nv_rpctrace_procfs_init(struct proc_dir_entry *parent);
void nv_rpctrace_shutdown(void);

#endif /* NV_KERNEL_INTERFACE_LAYER */

#endif /* _NV_RPCTRACE_H_ */
