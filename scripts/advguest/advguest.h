/* SPDX-License-Identifier: GPL-2.0
 *
 * advguest.h — the emulated GPU's guest-visible surface, as ADDRESSES a hostile
 * kernel driver would poke, with a citation on every number.
 *
 * ⊘ NOTHING here is hand-guessed. Every offset is either (a) a kayfabe source line
 * that decides where the archive decodes it, or (b) the vendor's own open-driver
 * header the archive cites for the same fact. Where the two must agree, both are
 * cited, because "citing the oracle is not the oracle being right" — a wrong pointer
 * in a header a whole suite is built on is checked by nobody.
 *
 * Citation rule (this repo's CI gate): a bare `ogkm:` fails. Use `ogkm-580:` for the
 * bench's own 580.159.04 tree (governs when the two disagree) or `ogkm-610:` for
 * 610.43.02. `C:` is the retired QEMU artifact. `[src]` = read from source, not run.
 */
#ifndef ADVGUEST_H
#define ADVGUEST_H

/* ── PCI identity ──────────────────────────────────────────────────────────────
 * kayfabe presents an RTX 3060. `crates/kayfabe-abi/src/chipinfo.rs:270` — "3060
 * (device 0x2504, vendor 0x10de) is the third witness". Set into config space by
 * `qemu/hw/misc/nvkvm/nvkvm.c:3569-3570` (pci_config_set_vendor_id/device_id). */
#define ADV_PCI_VENDOR   0x10DEu
#define ADV_PCI_DEVICE   0x2504u

/* BAR map — `qemu/hw/misc/nvkvm/nvkvm.c:1064-1088` (nvkvm_regions[]).
 *   BAR0  = register aperture, 16 MiB, NVKVM_KIND_CUT (mostly dead space, tiled).
 *           default size `nvkvm.c:3828` DEFINE_PROP_UINT64("bar0-size", 16*MiB).
 *   BAR1  = FB window, 64-bit prefetch, GMMU-translated (TRAP). run_fast_guest.sh
 *           passes bar1-size=128 MiB.
 *   BAR2  = instance window, 64-bit prefetch, GMMU-translated (TRAP), 32 MiB
 *           (run_fast_guest.sh: bar2-size=33554432). This is PCI BAR index 3
 *           because BAR1 is 64-bit and consumes two slots. BAR index 5 = MSI-X. */
#define ADV_BAR0_INDEX   0
#define ADV_BAR1_INDEX   1
#define ADV_BAR2_INDEX   3
#define ADV_BAR0_SIZE    (16u * 1024u * 1024u)

/* ── The VF register windows inside BAR0 ───────────────────────────────────────
 * kayfabe advertises the USERMODE (VIRTUAL_FUNCTION) window at 0x00BB0000
 * (`crates/kayfabe-device/src/ga10x.rs:1407` VIRTUAL_FUNCTION_BASE = 0x00BB0000,
 * served as regBases[NV_REG_BASE_USERMODE]). The PRIV block sits 0x30000 BELOW it
 * (`crates/kayfabe-device/src/mmuinval.rs:54-56`, USERMODE_ABOVE_PRIV = 0x30000):
 *   PRIV base = USERMODE base − 0x30000 = 0x00B80000. */
#define ADV_VF_USERMODE_BASE   0x00BB0000u
#define ADV_VF_PRIV_BASE       0x00B80000u

/* Doorbell: usermode base + 0x90.
 * `crates/kayfabe-device/src/doorbell.rs:243` USERMODE_DOORBELL_OFF = 0x90;
 * ogkm-580: src/common/inc/swref/published/turing/tu102/dev_vm.h:229
 *   NV_VIRTUAL_FUNCTION_DOORBELL = 0x30090, DRF_BASE(NV_VIRTUAL_FUNCTION)=0x30000. */
#define ADV_DOORBELL           (ADV_VF_USERMODE_BASE + 0x90u)   /* 0x00BB0090 */

/* The doorbell token's channel field is bits 11:0 (the VF doorbell vector).
 * `crates/kayfabe-device/src/dbtable.rs:349` — NV_CTRL_VF_DOORBELL_VECTOR is 11:0.
 * A written dword's low 12 bits index dbtable::Route; higher bits are not the
 * channel selector. */
#define ADV_DOORBELL_VECTOR_MASK   0xFFFu

/* TIME_0/TIME_1 — the readable liveness registers. A dead MMIO device reads all-ones;
 * a live one reads a nanosecond clock that advances.
 * ogkm-580: turing/tu102/dev_vm.h:224-227 NV_VIRTUAL_FUNCTION_TIME_0=0x30080,
 *   NV_VIRTUAL_FUNCTION_TIME_1=0x30084. Absolute = usermode base + 0x80/0x84. */
#define ADV_VF_TIME_0          (ADV_VF_USERMODE_BASE + 0x80u)   /* 0x00BB0080 */
#define ADV_VF_TIME_1          (ADV_VF_USERMODE_BASE + 0x84u)   /* 0x00BB0084 */

/* MMU invalidate — PRIV block. `crates/kayfabe-device/src/mmuinval.rs:125-132`;
 * ogkm-580: turing/tu102/dev_vm.h:120,128,131. */
#define ADV_MMU_INVALIDATE          (ADV_VF_PRIV_BASE + 0x30B0u) /* 0x00B830B0 */
#define ADV_MMU_INVALIDATE_PDB      (ADV_VF_PRIV_BASE + 0x30A0u) /* 0x00B830A0 */
#define ADV_MMU_INVALIDATE_UPPER    (ADV_VF_PRIV_BASE + 0x30A4u) /* 0x00B830A4 */
/* bitfields — ogkm-580: turing/tu102/dev_vm.h:132,135,121,125 */
#define ADV_INV_ALL_VA       (1u << 0)     /* _MMU_INVALIDATE_ALL_VA          0:0 */
#define ADV_INV_ALL_PDB      (1u << 1)     /* _MMU_INVALIDATE_ALL_PDB         1:1 */
#define ADV_INV_PDB_APERTURE_SYS  (1u << 1)/* _MMU_INVALIDATE_PDB_APERTURE    1:1 */
/* PDB addr field is 31:4, holding the address already shifted right by
 * PDB_ADDR_ALIGNMENT=0xc (ogkm-580: dev_vm.h:125-127). So reg = (gpa>>12)<<4. */
#define ADV_INV_PDB_ADDR(gpa)   ((((u32)((gpa) >> 12)) << 4))

/* GSP command-queue heads — BAR0 0x110c00, stride 8, count 8.
 * `crates/kayfabe-device/src/ga10x.rs:94-96` QUEUE_HEAD0=0x0011_0c00,
 *   QUEUE_HEAD_COUNT=8; ogkm-580 comment ga10x.rs:62 NV_PGSP_QUEUE_HEAD(i). */
#define ADV_GSP_QUEUE_HEAD0    0x0011_0C00u
#define ADV_GSP_QUEUE_HEAD_CNT 8u

/* ── USERD layout (guest RAM) ──────────────────────────────────────────────────
 * ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_ram.h:37-38.
 *   NV_RAMUSERD_GP_GET at word 34 = byte 0x88, GP_PUT at word 35 = byte 0x8C.
 * The 0x8C GP_PUT offset is the same one the shim keys its BAR1 USERD guess on,
 * `qemu/hw/misc/nvkvm/nvkvm.c:165,197`. */
#define ADV_USERD_GP_GET_OFF   0x88u
#define ADV_USERD_GP_PUT_OFF   0x8Cu
#define ADV_USERD_BYTES        0x200u   /* one page-ish; RAMUSERD fits well within */

/* ── GPFIFO entry (a pair of dwords) ──────────────────────────────────────────
 * ogkm-580: src/common/sdk/nvidia/inc/class/clc36f.h:263-278.
 *   GP_ENTRY0: FETCH 0:0, GET 31:2 (pushbuffer GPU-VA bits 31:2)
 *   GP_ENTRY1: GET_HI 7:0, PRIV 8:8, LEVEL 9:9, LENGTH 30:10, SYNC 31:31 */
static inline u32 adv_gp_entry0(u64 pb_gpuva)
{
	return (u32)(pb_gpuva & 0xFFFFFFFCu);           /* GET 31:2, FETCH=0 */
}
static inline u32 adv_gp_entry1(u64 pb_gpuva, u32 len_dwords)
{
	return ((u32)(pb_gpuva >> 32) & 0xFFu)          /* GET_HI 7:0 */
	     | ((len_dwords & 0x1FFFFFu) << 10);        /* LENGTH 30:10 (21 bits) */
}

/* ── The copy-engine class B0B5 method stream ─────────────────────────────────
 * ogkm-580: src/common/sdk/nvidia/inc/class/cla0b5.h.
 * These are the methods a real CeUtils pushbuffer is built from
 * (ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:1053-1091). */
#define ADV_B0B5_SET_SEMAPHORE_A     0x0240u
#define ADV_B0B5_SET_SEMAPHORE_B     0x0244u
#define ADV_B0B5_SET_SEMAPHORE_PAYLOAD 0x0248u
#define ADV_B0B5_SET_SRC_PHYS_MODE   0x0260u
#define ADV_B0B5_SET_DST_PHYS_MODE   0x0264u
#define ADV_B0B5_LAUNCH_DMA          0x0300u
#define ADV_B0B5_OFFSET_IN_UPPER     0x0400u
#define ADV_B0B5_OFFSET_IN_LOWER     0x0404u
#define ADV_B0B5_OFFSET_OUT_UPPER    0x0408u
#define ADV_B0B5_OFFSET_OUT_LOWER    0x040Cu
#define ADV_B0B5_LINE_LENGTH_IN      0x0418u
/* LAUNCH_DMA fields — cla0b5.h:66-102 */
#define ADV_LDMA_XFER_PIPELINED      (1u << 0)  /* DATA_TRANSFER_TYPE 1:0 = PIPELINED */
#define ADV_LDMA_SEM_ONE_WORD        (1u << 3)  /* SEMAPHORE_TYPE 4:3 = RELEASE_ONE_WORD */
#define ADV_LDMA_SRC_TYPE_PHYSICAL   (1u << 12) /* SRC_TYPE 12:12 = PHYSICAL */
#define ADV_LDMA_DST_TYPE_PHYSICAL   (1u << 13) /* DST_TYPE 13:13 = PHYSICAL */
/* SET_{SRC,DST}_PHYS_MODE_TARGET 1:0 — cla0b5.h:57-65. 3 is not enumerated. */
#define ADV_PHYS_TARGET_LOCAL_FB     0u
#define ADV_PHYS_TARGET_COHERENT     1u
#define ADV_PHYS_TARGET_UNMODELED    3u  /* reserved: no aperture kayfabe models */

/* FIFO DMA method header — an incrementing method group.
 * `crates/kayfabe-abi/src/submit.rs:2070-2085` method_header_inc():
 *   (method>>2) | (subchannel<<13) | (count<<16) | (1<<29)
 * (NVC56F_DMA SEC_OP_INC_METHOD). Same encoding clc36f-class pushbuffers use. */
static inline u32 adv_method_hdr_inc(u32 subch, u32 method, u32 count)
{
	return ((method >> 2) & 0xFFFu) | ((subch & 0x7u) << 13)
	     | ((count & 0x1FFFu) << 16) | (1u << 29);
}
#define ADV_SUBCH_COPY   4u   /* the copy engine's conventional subchannel */

#endif /* ADVGUEST_H */
