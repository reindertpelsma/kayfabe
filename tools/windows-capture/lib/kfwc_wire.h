/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 *
 * kfwc: wire constants of the GSP message-queue transport, and byte accessors.
 *
 * Provenance. Every offset below is the published wire ABI of the open NVIDIA GPU kernel
 * modules (ogkm, MIT) as already recorded in this repository by kf-gsp
 * (crates/kf-gsp/src/{ring,element,boot}.rs, crates/kf-abi/src/versions.rs) and by
 * tools/vfio-gsp-observer (layout-580.json, core/queue.c). No NVIDIA implementation code is
 * copied and no closed binary was consulted. Each block names its kf-gsp source.
 *
 * This header is freestanding: no libc, no stdint.h, so it compiles in a Windows kernel
 * driver (/kernel, no CRT headers) and in a Linux test build alike.
 */
#ifndef KFWC_WIRE_H
#define KFWC_WIRE_H

typedef unsigned char kfwc_u8;
typedef unsigned short kfwc_u16;
typedef unsigned int kfwc_u32;
typedef unsigned long long kfwc_u64;

#define KFWC_STATIC_ASSERT(name, cond) typedef char name[(cond) ? 1 : -1]
KFWC_STATIC_ASSERT(kfwc_u8_is_1, sizeof(kfwc_u8) == 1);
KFWC_STATIC_ASSERT(kfwc_u16_is_2, sizeof(kfwc_u16) == 2);
KFWC_STATIC_ASSERT(kfwc_u32_is_4, sizeof(kfwc_u32) == 4);
KFWC_STATIC_ASSERT(kfwc_u64_is_8, sizeof(kfwc_u64) == 8);

/* ---- byte accessors (little endian, alignment independent) ------------------------------ */
static __inline kfwc_u32 kfwc_get32(const void *p)
{
    const kfwc_u8 *b = (const kfwc_u8 *)p;
    return (kfwc_u32)b[0] | ((kfwc_u32)b[1] << 8) | ((kfwc_u32)b[2] << 16) | ((kfwc_u32)b[3] << 24);
}
static __inline kfwc_u64 kfwc_get64(const void *p)
{
    return (kfwc_u64)kfwc_get32(p) | ((kfwc_u64)kfwc_get32((const kfwc_u8 *)p + 4) << 32);
}
static __inline void kfwc_put32(void *p, kfwc_u32 v)
{
    kfwc_u8 *b = (kfwc_u8 *)p;
    b[0] = (kfwc_u8)v;
    b[1] = (kfwc_u8)(v >> 8);
    b[2] = (kfwc_u8)(v >> 16);
    b[3] = (kfwc_u8)(v >> 24);
}
static __inline void kfwc_put64(void *p, kfwc_u64 v)
{
    kfwc_put32(p, (kfwc_u32)v);
    kfwc_put32((kfwc_u8 *)p + 4, (kfwc_u32)(v >> 32));
}

/* ---- msgq (src/common/shared/msgq at ogkm-580; kf-gsp ring.rs) --------------------------- */
#define KFWC_PAGE 4096u              /* RM_PAGE_SIZE; also GSP_MSG_QUEUE_ELEMENT_SIZE_MIN */
#define KFWC_MSGQ_VERSION 0u         /* MSGQ_VERSION */
#define KFWC_MSGQ_FLAG_SWAP_RX 1u    /* MSGQ_FLAGS_SWAP_RX */
/* msgqTxHeader: eight little-endian u32 (kf-gsp TxHeader). */
#define KFWC_TXH_VERSION 0u
#define KFWC_TXH_SIZE 4u             /* whole backing store, bytes */
#define KFWC_TXH_MSGSIZE 8u
#define KFWC_TXH_MSGCOUNT 12u
#define KFWC_TXH_WRITEPTR 16u
#define KFWC_TXH_FLAGS 20u
#define KFWC_TXH_RXHDROFF 24u
#define KFWC_TXH_ENTRYOFF 28u
#define KFWC_TXH_BYTES 32u
#define KFWC_RXH_BYTES 4u            /* msgqRxHeader: readPtr */
#define KFWC_QUEUE_MIN_BYTES (2u * KFWC_PAGE)
#define KFWC_QUEUE_MAX_BYTES (1024u * 1024u)   /* bound, the default queue is 0x40000 */

/* ---- queue element (message_queue_priv.h; kf-gsp element.rs, kf-abi GspElementWire) ------ */
/* Pre-610 (r535/r570/580/590/595): 48-byte header, elemCount@40, RPC envelope @48.        */
/* 610.43.02 and later: 16-byte header with MCTP/NVDM words, no elemCount, RPC @16.        */
#define KFWC_LAYOUT_PRE610 1u
#define KFWC_LAYOUT_610 2u
#define KFWC_NO_FIELD 0xFFFFu
typedef struct kfwc_layout {
    kfwc_u32 id;
    kfwc_u16 hdr_size;
    kfwc_u16 checksum_off;
    kfwc_u16 seq_off;
    kfwc_u16 count_off;             /* KFWC_NO_FIELD when the version has no elemCount */
    kfwc_u16 mctp_off, nvdm_off;    /* KFWC_NO_FIELD when absent */
    kfwc_u32 mctp_word, nvdm_word;  /* the words a conforming sender writes (610 only) */
} kfwc_layout;

/* rpc_message_header_v: eight u32, 32 bytes (kf-gsp encode_envelope). */
#define KFWC_RPC_VERSION 0x03000000u
#define KFWC_RPC_SIGNATURE 0x43505256u
#define KFWC_RPC_BYTES 32u
#define KFWC_RPC_OFF_VERSION 0u
#define KFWC_RPC_OFF_SIGNATURE 4u
#define KFWC_RPC_OFF_LENGTH 8u
#define KFWC_RPC_OFF_FUNCTION 12u
#define KFWC_RPC_OFF_RESULT 16u
#define KFWC_RPC_OFF_RESULT_PRIVATE 20u
#define KFWC_RPC_OFF_SEQUENCE 24u
#define KFWC_ELEM_SIZE_MAX (16u * KFWC_PAGE)   /* queueElementSizeMax = RM_PAGE_SIZE * 16 */
#define KFWC_MAX_RUN_ELEMS 16u

/* ---- LibOS init args and the message-queue init args (kf-gsp boot.rs publish) ------------ */
/* libos_init_args.h: {id8 @0, pa @8, size @16, kind u8 @24, loc u8 @25}, 32-byte stride.  */
#define KFWC_LIBOS_STRIDE 32u
#define KFWC_LIBOS_ID 0u
#define KFWC_LIBOS_PA 8u
#define KFWC_LIBOS_SIZE 16u
#define KFWC_LIBOS_KIND 24u
#define KFWC_LIBOS_LOC 25u
#define KFWC_LIBOS_KIND_CONTIGUOUS 1u    /* layout-580.json "contiguous" */
#define KFWC_LIBOS_LOC_SYSMEM 1u         /* layout-580.json "sysmem" */
#define KFWC_LIBOS_ARRAY_BYTES 4096u     /* the bounded 4096-byte array (observer README) */
#define KFWC_LIBOS_MAX_ENTRIES (KFWC_LIBOS_ARRAY_BYTES / KFWC_LIBOS_STRIDE)
#define KFWC_RMARGS_ID 0x0000524d41524753ull   /* "RMARGS", id built MSB first (kf-chip RMARGS_ID) */
/* OGKM580.65.06 gspifpub.h / kgspBootstrapGspFmc_GH100. Compiler-derived
 * by tools/vfio-gsp-observer/layout.c (307159f2623d3bf45feb9177bd2da52ffbc5ddf9).
 * Hopper/Blackwell MAILBOX0/1 may name this structure instead of LibOS. */
#define KFWC_FMC_TARGET 40u
#define KFWC_FMC_LIBOS 48u
#define KFWC_FMC_COHERENT_SYSTEM 1u
#define KFWC_FMC_NONCOHERENT_SYSTEM 2u
/* MESSAGE_QUEUE_INIT_ARGUMENTS, first member of GSP_ARGUMENTS_CACHED (NvLength is 64-bit). */
#define KFWC_MQA_SHARED_PA 0u
#define KFWC_MQA_PTE_COUNT 8u            /* u32, then 4 pad bytes */
#define KFWC_MQA_CMD_OFF 16u
#define KFWC_MQA_STAT_OFF 24u
#define KFWC_MQA_HDR_SIZE 32u            /* queueElementHdrSize, 610 only (kf-abi NineField) */
#define KFWC_MQA_MIN_PRE610 32u
#define KFWC_MQA_MIN_610 40u

/* The self-describing page table: region page 0 holds one u64 per region page, entry 0 is
 * the table page itself. One table page = 512 entries; chained tables are refused by name. */
#define KFWC_MAX_REGION_PAGES 512u

/* ---- BAR0 (the whole read surface; ogkm dev_gsp.h / dev_falcon_v4.h, kf-chip falcon_gsp) -- */
#define KFWC_REG_PMC_BOOT_0 0x00000000u       /* liveness only (all-ones = not readable) */
#define KFWC_REG_GSP_MAILBOX0 0x00110040u     /* NV_PGSP_FALCON_MAILBOX0 (TU10x..AD10x boot args low) */
#define KFWC_REG_GSP_MAILBOX1 0x00110044u     /* NV_PGSP_FALCON_MAILBOX1 (boot args high) */
/* Privileged measurement only: NV_PDISP_FE_SW, the C573/C673/C773/CA73 capabilities page
 * (ogkm kdispGetDisplayCapsBaseAndSize; V3_DISPLAY_CAPS_DERIVATION_20261014.md). */
#define KFWC_DISPLAY_CAPS_BASE 0x00640000u
#define KFWC_DISPLAY_CAPS_WORDS 1024u
#define KFWC_DISPLAY_CAPS_BYTES (KFWC_DISPLAY_CAPS_WORDS * 4u)
#define KFWC_BAR0_MAP_BYTES (KFWC_DISPLAY_CAPS_BASE + KFWC_DISPLAY_CAPS_BYTES)
#define KFWC_MAILBOX0_SUSPENDED 0x80000000u   /* suspend sentinel (kf-chip PROCESSOR_SUSPENDED) */

#endif /* KFWC_WIRE_H */
