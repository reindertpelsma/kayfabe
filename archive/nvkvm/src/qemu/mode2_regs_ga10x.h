/* mode2_regs_ga10x.h — GA10x (Ampere) register offsets, GMMU VER2 format, and
 * chip constants for the Mode-2 emulated GPU.  ALL arch-specific magic numbers
 * live here (not scattered in nvkvm_gpu_emul.c) so the device is portable to
 * other archs: add mode2_regs_<arch>.h and select by chip.  These are intended
 * to be AUTO-GENERATED from the open driver's swref headers
 * (tools/mode2_gen_regs.py — see docs/design/mode2_device_data_model.md);
 * hand-maintained for now.  Glossary: docs/design/nvidia_gpu_internals.md §1.1.
 */
#ifndef MODE2_REGS_GA10X_H
#define MODE2_REGS_GA10X_H

/* M0 — chip identity (dev_boot) */
#define NV_PMC_BOOT_0   0x00000000u
#define NV_PMC_BOOT_1   0x00000004u
#define NV_PMC_BOOT_42  0x00000A00u

/* M1 — GFW boot completion (PGC6 AON secure-scratch, ga102) */
#define NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK 0x00118128u
#define NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT      0x00118234u
#define NV_PGC6_GFW_BOOT_PROGRESS_COMPLETED                 0x000000FFu

/* M2 — VBIOS PROM window (NV_PROM_DATA) */
#define NV_PROM_DATA_BASE 0x00300000u
#define NV_PROM_DATA_SIZE 0x00100000u   /* 1 MiB */

/* M2 — GSP/SEC2 falcon bring-up (NV_PGSP 0x110000, NV_PSEC 0x840000) */
#define NV_PGSP_FALCON_CPUCTL                  0x00110100u
#define NV_PFALCON_FALCON_CPUCTL_HALTED_TRUE   0x00000010u  /* bit 4 */
#define NV_PGSP_FALCON_HWCFG2                  0x001100F4u
#define NV_PFALCON_FALCON_HWCFG2_RISCV_ENABLE_VAL 0x00000400u /* bit 10 */

/* M3 — GSP-RPC mailboxes (LibOS init-args GPA) + LibOS region layout */
#define NV_PGSP_FALCON_MAILBOX0      0x00110040u
#define NV_PGSP_FALCON_MAILBOX1      0x00110044u
#define LIBOS_REGION_STRIDE          32u
#define LIBOS_REGION_LOC_SYSMEM      1u

/* PTIMER — GPU ns clock (GA10x relocated to 0xbb0000) */
#define NV_PTIMER_TIME_0_GA10X       0x00BB0080u
#define NV_PTIMER_TIME_1_GA10X       0x00BB0084u
/* PTIMER PRIV_LEVEL_MASK (legacy PRI offset, read directly — NOT relocated).
 * tmrSetCurrentTime_GV100 asserts unless WRITE_PROTECTION_LEVEL0 (bit4) = ENABLE,
 * so report all privilege levels granted (fully lowered). */
#define NV_PTIMER_TIME_PRIV_LEVEL_MASK 0x00009430u

/* M3 — Falcon DMA (FWSEC/Booter ucode load) + SEC2 falcon */
#define NV_PFALCON_DMATRFCMD_IDLE_VAL 0x00000002u  /* IDLE=TRUE|FULL=FALSE */
#define NV_PGSP_FALCON_DMATRFCMD     0x00110118u
#define NV_PSEC_FALCON_DMATRFCMD     0x00840118u
#define NV_PSEC_FALCON_CPUCTL        0x00840100u
#define NV_PSEC_FALCON_HWCFG2        0x008400F4u
#define NV_PSEC_FALCON_MAILBOX0      0x00840040u /* Booter Load/Unload arg low; =0xff on NORMAL Unload */

/* M3 — WPR2 (Write-Protect Region 2 in FB), dev_fb.h */
#define NV_PFB_PRI_MMU_WPR2_ADDR_LO  0x001FA824u
#define NV_PFB_PRI_MMU_WPR2_ADDR_HI  0x001FA828u
#define NVKVM_WPR2_LO_VAL            0x02FFE000u  /* _VAL=0x2FFE00 (12 GiB FB) */
#define NVKVM_WPR2_HI_VAL            0x02FFF000u  /* _VAL=0x2FFF00 */

/* M3 — usable FB size in MiB (NV_USABLE_FB_SIZE_IN_MB) */
#define NV_USABLE_FB_SIZE_IN_MB      0x001183A4u
#define NVKVM_FB_SIZE_MB             12288u  /* 12 GiB (RTX 3060) */

/* M3 — GSP RISC-V active (RISCV base 0x111000 + 0x388, _ACTIVE_STAT bit7) */
#define NV_PGSP_RISCV_CPUCTL                   0x00111388u
#define NV_PRISCV_RISCV_CPUCTL_ACTIVE_STAT_VAL 0x00000080u  /* bit 7 */

/* M3 — GSP cmd-queue doorbell (NV_PGSP_QUEUE_HEAD(0)) */
#define NVKVM_GSP_QUEUE_HEAD0        0x00110C00u

/* Display fuse — NV_FUSE_STATUS_OPT_DISPLAY (dev_fuse.h). _DATA bit0: ENABLE=0,
 * DISABLE=1.  gpuFuseSupportsDisplay_HAL tests _DATA==_ENABLE; reporting bit0=1
 * (fused-off) makes kdispStatePreInitLocked return NV_ERR_NOT_SUPPORTED, so the
 * driver cleanly skips ALL display init — we advertise a compute-only displayless
 * GPU (like A100/H100), which is the right model for Mode-2 compute. */
#define NV_FUSE_STATUS_OPT_DISPLAY        0x00820C04u
#define NVKVM_FUSE_OPT_DISPLAY_DISABLED   0x00000001u  /* _DATA=DISABLE */

/* M6 — BAR0 PRAMIN window + BAR2 bind register(s) */
#define NVKVM_PRAMIN_BASE   0x00700000u
#define NVKVM_PRAMIN_SIZE   0x00100000u      /* 1 MiB window */
#define NVKVM_BAR0_WINDOW   0x00001700u      /* NV_PBUS_BAR0_WINDOW; BASE[23:0]<<16 */
#define NVKVM_PBUS_BAR2_BLOCK 0x00001714u    /* Maxwell BAR2 block reg */
/* Turing/Ampere: NV_VIRTUAL_FUNCTION_PRIV_BAR2_BLOCK = VF base 0xB80000 + 0xF48.
 * (NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET = 0xB80000.) PTR[27:0]<<12=instblk,
 * MODE bit31 (1=VIRTUAL/0=PHYSICAL). NOTE: on bare-metal GSP-client the CPU bind
 * is a no-op (GSP binds BAR2); the PDB comes from GspStaticConfigInfo instead. */
#define NVKVM_VF_BAR2_BLOCK 0x00B80F48u
/* M5 — work-submit doorbell: kfifoUpdateUsermodeDoorbell_GA100 does
 * GPU_VREG_WR32(NV_VIRTUAL_FUNCTION_DOORBELL=0x30090, token).  GPU_VREG adds
 * sriovState.virtualRegPhysOffset = DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)
 * = 0xB80000 for bare-metal (gpuGetVirtRegPhysOffset_TU102), so the ABSOLUTE BAR0
 * offset is 0xB80000 + 0x30090 = 0xBB0090 (CONFIRMED in trace: WR 0xbb0090 <-
 * 0x00010002).  Token: DOORBELL_VECTOR[11:0]=chId, RUNLIST_ID[22:16].  Ringing it
 * = the guest submitted work on (runlist,chId); the emulator must execute the
 * channel (GPFIFO->pushbuffer->CE semaphore release) so
 * channelWaitForFinishPayload (ce_utils.c:349) stops timing out. */
#define NVKVM_VF_DOORBELL   0x00BB0090u
#define NVKVM_DOORBELL_CHID(tok)     ((tok) & 0xFFFu)
#define NVKVM_DOORBELL_RUNLIST(tok)  (((tok) >> 16) & 0x7Fu)

/* M7 — CPU interrupt tree (NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_*, VF base 0xB80000).
 * The driver tests IRQ delivery (osVerifySystemEnvironment/_osVerifyInterrupts):
 * it writes the SW-intr/doorbell vector (=129) to LEAF_TRIGGER, expecting the HW
 * to set the leaf+top pending bits and raise an MSI so the ISR fires.  TOP(0) bit
 * = subtree; LEAF(i) bit = vector%32; subtree = (vector/32)/2; leaf reg = vec/32. */
#define NVKVM_VF_INTR_LEAF0          0x00B81000u  /* LEAF(i)=+i*4, i<8 (RW1C pending) */
#define NVKVM_VF_INTR_LEAF_EN_SET0   0x00B81200u
#define NVKVM_VF_INTR_LEAF_EN_CLR0   0x00B81400u
#define NVKVM_VF_INTR_TOP0           0x00B81600u  /* TOP(i)=+i*4, i<1 */
#define NVKVM_VF_INTR_TOP_EN_SET0    0x00B81608u
#define NVKVM_VF_INTR_TOP_EN_CLR0    0x00B81610u
#define NVKVM_VF_INTR_LEAF_TRIGGER   0x00B81640u
#define NVKVM_VF_INTR_NLEAF          8u
#define NVKVM_SW_INTR_VECTOR         129u  /* NV_CTRL_CPU_DOORBELL_VECTORID_VALUE_CONSTANT */
#define NVKVM_BAR2_BLOCK_PTR_SHIFT 12         /* instblk addr = PTR << 12 */
#define NVKVM_BAR2_BLOCK_MODE_VIRTUAL 0x80000000u /* MODE bit31 */

/* M6 — GMMU VER2 page-table format (GA10x, Pascal-format; dev_mmu.h).
 * PTE/PDE ADDRESS_VID = bits 32:8, <<12.  DUAL_PDE (PD0) is 16B: small sub-table
 * bits 96:72 <<12, big sub-table bits 32:4 <<8.  PTE VALID = bit0. */
#define NVKVM_VER2_ADDR_VID(e)  ((((e) >> 8) & ((1ull << 25) - 1)) << 12)
/* per-level VA bit ranges {hi,lo}: PD3[48:47] PD2[46:38] PD1[37:29] PD0[28:21]
 * (16B dual PDE) PT_small[20:12] (4 KiB) PT_big[20:16] (64 KiB) */
#define NVKVM_VER2_PD0_VA_LO    21
#define NVKVM_VER2_PD0_VA_BITS  8
#define NVKVM_VER2_PT_SMALL_LO  12
#define NVKVM_VER2_PT_SMALL_BITS 9
#define NVKVM_VER2_PT_BIG_LO    16
#define NVKVM_VER2_PT_BIG_BITS  5

/* M6 — instance-block PAGE_DIR_BASE field offsets (dev_ram.h NV_RAMIN):
 * word128 @ 0x200 (TARGET[1:0], LO[31:12]); word129 @ 0x204 (HI[31:0]). */
#define NVKVM_RAMIN_PDB_LO_OFF  0x200
#define NVKVM_RAMIN_PDB_HI_OFF  0x204

/* M6 — GspStaticConfigInfo.bar2PdeBase byte offset (verified via host printk).
 * The GSP reports the BAR2 page-dir base here; we replay it for GET_GSP_STATIC_INFO
 * and use it as the GMMU walk root. */
#define NVKVM_GSPSTATIC_BAR2PDEBASE_OFF 1672

#endif /* MODE2_REGS_GA10X_H */
