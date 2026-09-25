//! ★★★ **The FSP-booted GSP model — Hopper AND Blackwell, one model, a two-field row.**
//!
//! v3 (w826, owner: *"all of them need eventually parity … not too much maintenance overhead"*).
//! The old tree kept `gh100.rs` and `gb20x.rs` as two near-copies (a line diff of their models is
//! THREE hunks). They differ only where [`FspRow`] says:
//! - Blackwell gates the COT stream on `NV_THERM_I2CS_SCRATCH` (GB202 binds
//!   `kfspWaitForSecureBoot_GB202`, `ogkm-580: g_kern_fsp_nvoc.c:423-425`); Hopper does not;
//! - Blackwell's WPR2 register is DRIVER-VERSION keyed: `0x1FA824` at 580, `0x88A824` at 610 — both
//!   are decoded.
//! ⊘ And WPR2's value is DERIVED from the framebuffer size (as the falcon model does) — the old
//! models' `0x02FF_E000` was self-described as INVENTED. Only nonzero is load-bearing
//! (`kgspIsWpr2Up_TU102`: `_VAL != 0`), and a derived nonzero value is the same for every die.
//!
//! The body below is the old `kayfabe-chips/gb20x.rs` (the superset), with those three changes.

use kf_arch::gsp::{
    ArchBootState, BootContext, BootSequence, BootStageDesc, BootStep, BootStepKind, BootSteps,
    GspModel, GspObservation, GspReg, LibosRegionLayout, RegWrite,
};

// ── BAR0 offsets ──────────────────────────────────────────────────────────────────

/// GSP falcon block base. `NV_PGSP` is `0x113fff:0x110000` in a **Blackwell** header
/// (`ogkm-580: src/common/inc/swref/published/blackwell/gb20b/dev_gsp.h:27`), and GB202
/// reaches it through `kgspConfigureFalcon_GA102`'s `falconConfig.registerBase =
/// DRF_BASE(NV_PGSP)` (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/ampere/
/// kernel_gsp_ga102.c:57`).
///
/// ⊘ `blackwell/gb202/` ships no `dev_gsp.h` of its own; the two citations above are the
/// Blackwell-family header and the binding, which is the same evidence shape
/// [`crate::gh100::QUEUE_HEAD0`]'s correction records.
const PGSP: u64 = 0x0011_0000;

/// `NV_PFALCON_FALCON_IRQSCLR` — falcon-relative
/// (`ogkm-580: turing/tu102/dev_falcon_v4.h:26`).
const FALCON_IRQSCLR: u64 = 0x004;
/// `NV_PFALCON_FALCON_IRQSTAT` (`ogkm-580: turing/tu102/dev_falcon_v4.h:31`).
const FALCON_IRQSTAT: u64 = 0x008;
/// `NV_PFALCON_FALCON_IRQMASK` (`ogkm-580: turing/tu102/dev_falcon_v4.h:39`).
const FALCON_IRQMASK: u64 = 0x018;
/// `NV_PFALCON_FALCON_IRQDEST` (`ogkm-580: turing/tu102/dev_falcon_v4.h:40`).
const FALCON_IRQDEST: u64 = 0x01c;
/// `NV_PFALCON_FALCON_MAILBOX0` (`ogkm-580: turing/tu102/dev_falcon_v4.h:41`); the
/// absolute `0x110040` is republished for Blackwell at
/// `ogkm-580: blackwell/gb100/dev_gsp.h:27`.
const FALCON_MAILBOX0: u64 = 0x040;
/// `NV_PFALCON_FALCON_MAILBOX1` (`ogkm-580: turing/tu102/dev_falcon_v4.h:42`); absolute
/// `0x110044` at `ogkm-580: blackwell/gb100/dev_gsp.h:30`.
const FALCON_MAILBOX1: u64 = 0x044;
/// `NV_PFALCON_FALCON_HWCFG2` (`ogkm-580: turing/tu102/dev_falcon_v4.h:62`) — and
/// **republished unchanged in a Blackwell header**,
/// `ogkm-580: blackwell/gb20b/dev_falcon_v4.h:27`.
const FALCON_HWCFG2: u64 = 0x0f4;
/// `NV_PFALCON_FALCON_CPUCTL` (`ogkm-580: turing/tu102/dev_falcon_v4.h:46`).
const FALCON_CPUCTL: u64 = 0x100;
/// `NV_PFALCON_FALCON_DMATRFCMD` (`ogkm-580: turing/tu102/dev_falcon_v4.h:76`).
const FALCON_DMATRFCMD: u64 = 0x118;

/// `NV_PGSP_QUEUE_HEAD(0)`, stride 8 — `0x00110c00+(i)*8` in a **Blackwell** header
/// (`ogkm-580: blackwell/gb20b/dev_gsp.h:33`). GB202 writes it on every RPC send because
/// `kgspSetCmdQueueHead` binds `_TU102` for every chip that is not a VF or a Tegra part
/// (`ogkm-580: src/nvidia/generated/g_kernel_gsp_nvoc.c:675-677`).
const QUEUE_HEAD0: u64 = 0x0011_0C00;
/// `NV_PGSP_QUEUE_HEAD__SIZE_1` (`ogkm-580: blackwell/gb20b/dev_gsp.h:34`).
const QUEUE_HEAD_COUNT: u64 = 8;

/// GSP RISC-V `CPUCTL`. **Both halves are cited on this generation**, which is the one
/// place this module is better evidenced than [`crate::gh100`]:
///
/// - base `NV_FALCON2_GSP_BASE = 0x0011_1000` — GB202 binds `kgspConfigureFalcon_GA102`
///   (`ogkm-580: g_kernel_gsp_nvoc.c:578-580`), whose `riscvRegisterBase` is that symbol
///   (`ogkm-580: kernel_gsp_ga102.c:58`), defined at
///   `ogkm-580: ampere/ga102/dev_riscv_pri.h:27`;
/// - offset `0x388` — **GB202 publishes it itself**,
///   `ogkm-580: blackwell/gb202/dev_riscv_pri.h:27`.
const GSP_RISCV_CPUCTL: u64 = 0x0011_1388;
/// `NV_PRISCV_RISCV_IRQMASK` = `NV_FALCON2_GSP_BASE` + `0x528`, the offset published by
/// GB202 itself (`ogkm-580: blackwell/gb202/dev_riscv_pri.h:31`).
///
/// ⚠ This offset is generation-keyed and the generations are not close — Turing and GA100
/// publish `0x2b4`/`0x2b8` (`ogkm-580: turing/tu102/dev_riscv_pri.h:32-33`). GB202 lands on
/// the Ampere-GA102 values, and it says so in its own header rather than inheriting them
/// silently; the HAL binding agrees (`kflcnRiscvReadIntrStatus_GA102`,
/// `ogkm-580: g_kernel_falcon_nvoc.c:632-634`).
const GSP_RISCV_IRQMASK: u64 = 0x0011_1528;
/// `NV_PRISCV_RISCV_IRQDEST` = base + `0x52c`
/// (`ogkm-580: blackwell/gb202/dev_riscv_pri.h:30`).
const GSP_RISCV_IRQDEST: u64 = 0x0011_152c;
/// ★ w827: `NV_PRISCV_RISCV_BCR_CTRL` = `NV_FALCON2_GSP_BASE` + `0x668`
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_riscv_pri.h:55`).
const GSP_RISCV_BCR_CTRL: u64 = 0x0011_1668;
/// `NV_PRISCV_RISCV_BCR_CTRL_VALID_TRUE` (0:0, `dev_riscv_pri.h:56-57`).
const BCR_CTRL_VALID: u64 = 1;

/// ★★★★★ **THE WPR2 REGISTER MOVED, AND IT MOVED BY DRIVER VERSION, NOT BY CHIP.**
///
/// This is the one constant in this file that a family assumption gets wrong *and* that a
/// single-version reading gets wrong, and it was nearly written as `0x001F_A824` here on
/// the strength of the 580 tree alone. Both readings are correct; they are readings of
/// different drivers:
///
/// | driver | GB202's `kgspIsWpr2Up` binding | register it reads | address |
/// |---|---|---|---|
/// | **580.159.04** | `_GH100` (`ogkm-580: g_kernel_gsp_nvoc.c:1237-1239`, the `else` past AD107) → tail-calls `kgspIsWpr2Up_TU102` (`ogkm-580: kernel_gsp_gh100.c:236`) | `NV_PFB_PRI_MMU_WPR2_ADDR_HI` (`ogkm-580: hopper/gh100/dev_fb.h:48`) | `0x001F_A828` |
/// | **610.43.02** | `_GB100` (`ogkm-610: g_kernel_gsp_nvoc.c:1339-1341` — 610 grew a **fifth** arm and GH100 was split out of the `else` into its own) | `NV_HUBMMU0_PRI_BASE + NV_HUBMMU_PRI_MMU_WPR2_ADDR_HI` (`ogkm-610: kernel_gsp_gb100.c:338`) = `0x880000` (`ogkm-610: blackwell/gb100/hwproject.h:36`) + `0xa828` (`ogkm-610: blackwell/gb100/dev_hubmmu_base.h:94`) | **`0x0088_A828`** |
///
/// ⇒ **a 580 Blackwell guest and a 610 Blackwell guest poll two different addresses for
/// the same fact.** A profile that served only one would look healthy and hang the other
/// at `kgspWaitForGfwBootOk`, with nothing refused — the `dlen = 0` shape.
///
/// ★ So [`FspGspModel::decode_reg`] claims **both pairs**, and `encode` answers the same
/// value for either. That is not a guess: neither address exists on the other driver's
/// path, so serving both cannot be wrong for either, and refusing one *is* wrong for one.
/// [`FspGspModel::at`] has to name a single offset and names the **610** one, because
/// 610 is the newer binding and the older one is reachable through `decode_reg`.
///
/// ⊘ The LO half of the 610 pair is `NV_HUBMMU_PRI_MMU_WPR2_ADDR_LO = 0xa824`
/// (`ogkm-610: blackwell/gb100/dev_hubmmu_base.h:91`) over the same base. Only HI is read
/// by `kgspIsWpr2Up_GB100`; LO is placed for symmetry and because the guest may read it.
///
/// ⚠ Independently derived, then found already recorded: `docs/design/
/// porting_to_any_architecture.md` §3.4 names `0x88A828` as *"what DOES move"* on
/// Blackwell and calls it **one constant**. It is one constant **per driver generation** —
/// the 580 half is the part that section does not carry, because it reads 610 only.
const WPR2_ADDR_LO: u64 = 0x0088_A824;
/// See [`WPR2_ADDR_LO`]. `NV_HUBMMU_PRI_MMU_WPR2_ADDR_HI`
/// (`ogkm-610: blackwell/gb100/dev_hubmmu_base.h:94`) over `NV_HUBMMU0_PRI_BASE`
/// (`ogkm-610: blackwell/gb100/hwproject.h:36`).
const WPR2_ADDR_HI: u64 = 0x0088_A828;
/// `NV_PFB_PRI_MMU_WPR2_ADDR_LO` (`ogkm-580: hopper/gh100/dev_fb.h:43`) — the address a
/// **580-era** Blackwell guest polls. See [`WPR2_ADDR_LO`] for why both are served.
pub const WPR2_ADDR_LO_580: u64 = 0x001F_A824;
/// `NV_PFB_PRI_MMU_WPR2_ADDR_HI` (`ogkm-580: hopper/gh100/dev_fb.h:48`) — the 580-era
/// address. See [`WPR2_ADDR_LO`].
pub const WPR2_ADDR_HI_580: u64 = 0x001F_A828;

// ── the FSP command channel ───────────────────────────────────────────────────────
//
// GB202 is named EXPLICITLY in the chip mask that binds every one of these HALs to the
// `_GH100` implementation (`ogkm-580: src/nvidia/generated/g_kern_fsp_nvoc.c:322-326`
// `kfspSendPacket`, `:345-350` `kfspCanSendPacket`, `:381-386` `kfspPrepareBootCommands`,
// `:394-399` `kfspSendBootCommands`) — `ChipHal: GH100 | GB100 | GB102 | GB110 | GB112 |
// GB202 | GB203 | GB205 | GB206 | GB207`. So the transport is Hopper's, at Hopper's
// offsets, because `kern_fsp_gh100.c` includes `published/hopper/gh100/dev_fsp_pri.h`.
//
// ⊘ `blackwell/gb202/dev_fsp_pri.h` defines ONLY the two `FALCON_COMMON_SCRATCH_GROUP`
// arrays (`:27-35`) and no EMEM window at all. Its silence is not a contradiction — the
// per-chip headers carry overrides, not full copies — but it does mean nothing in the
// Blackwell headers independently confirms these six offsets.

/// `NV_PFSP_EMEMC(FSP_EMEM_CHANNEL_RM)` — `0x008F2ac0+(i)*8`
/// (`ogkm-580: hopper/gh100/dev_fsp_pri.h:26`); RM's channel is `0`
/// (`ogkm-580: src/nvidia/arch/nvalloc/common/inc/fsp/fsp_emem_channels.h:34`).
const PFSP_EMEMC: u64 = 0x008F_2AC0;
/// `NV_PFSP_EMEMD(FSP_EMEM_CHANNEL_RM)` (`ogkm-580: hopper/gh100/dev_fsp_pri.h:40`).
const PFSP_EMEMD: u64 = 0x008F_2AC4;
/// `NV_PFSP_QUEUE_HEAD(FSP_EMEM_CHANNEL_RM)` (`ogkm-580: hopper/gh100/dev_fsp_pri.h:53`).
const PFSP_QUEUE_HEAD: u64 = 0x008F_2C00;
/// `NV_PFSP_QUEUE_TAIL(FSP_EMEM_CHANNEL_RM)` (`ogkm-580: hopper/gh100/dev_fsp_pri.h:57`).
const PFSP_QUEUE_TAIL: u64 = 0x008F_2C04;
/// `NV_PFSP_MSGQ_HEAD(FSP_EMEM_CHANNEL_RM)` (`ogkm-580: hopper/gh100/dev_fsp_pri.h:44`).
const PFSP_MSGQ_HEAD: u64 = 0x008F_2C80;
/// `NV_PFSP_MSGQ_TAIL(FSP_EMEM_CHANNEL_RM)` (`ogkm-580: hopper/gh100/dev_fsp_pri.h:48`).
const PFSP_MSGQ_TAIL: u64 = 0x008F_2C84;

/// ★★★ **The GB202-specific boot register, and it MOVED.**
///
/// `NV_THERM_I2CS_SCRATCH` is `0x00ad00bc` on GB202
/// (`ogkm-580: blackwell/gb202/dev_therm.h:27`) where it is `0x000200bc` on Hopper
/// (`ogkm-580: hopper/gh100/dev_therm.h:26`) and on datacenter Blackwell
/// (`ogkm-580: blackwell/gb100/dev_therm.h:27`). `NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE`
/// aliases the whole register (`ogkm-580: blackwell/gb202/dev_therm_addendum.h:27-28`).
///
/// RM polls it *before it is allowed to send any FSP packet*: `_kfspWaitBootCond_GB202`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fsp/arch/blackwell/kern_fsp_gb202.c:45-57`),
/// driven by `kfspWaitForSecureBoot_GB202` — which GB202 binds by name
/// (`ogkm-580: g_kern_fsp_nvoc.c:423-425`, `ChipHal: GB202 | GB203 | GB205 | GB206 |
/// GB207`) — and which is the first call in `kfspPrepareAndSendBootCommands_GH100`
/// (`ogkm-580: kern_fsp_gh100.c:1299`).
///
/// ⇒ A model that leaves this offset unclaimed answers `0`, the guest reads
/// `FSP_BOOT_COMPLETE != SUCCESS` forever and **never streams a COT payload at all** —
/// so none of the FSP registers above are ever touched and the boot looks like it stopped
/// before it started.
const THERM_I2CS_SCRATCH: u64 = 0x00AD_00BC;

/// `NV_PFSP_EMEMC_OFFS` is `7:2` (`ogkm-580: hopper/gh100/dev_fsp_pri.h:28`).
const EMEMC_OFFS_SHIFT: u32 = 2;
/// The `7:2` field is six bits wide.
const EMEMC_OFFS_MASK: u64 = 0x3F;
/// `NV_PFSP_EMEMC_BLK` is `15:8` (`ogkm-580: hopper/gh100/dev_fsp_pri.h:30`).
const EMEMC_BLK_SHIFT: u32 = 8;
/// The `15:8` field is eight bits wide.
const EMEMC_BLK_MASK: u64 = 0xFF;
/// `NV_PFSP_EMEMC_AINCW` is `24:24` (`ogkm-580: hopper/gh100/dev_fsp_pri.h:32`).
const EMEMC_AINCW: u64 = 1 << 24;
/// `DWORDS_PER_EMEM_BLOCK` (`ogkm-580: kern_fsp_gh100.c:57`).
const DWORDS_PER_EMEM_BLOCK: u64 = 64;

/// Byte offset within the EMEM window of the COT payload's `gspBootArgsSysmemOffset`.
/// Identical to Hopper's because GB202 runs the *same* `kfspPrepareBootCommands_GH100`
/// over the same `NVDM_PAYLOAD_COT` — see [`crate::gh100`] for the field-by-field
/// derivation of the 852.
const COT_BOOT_ARGS_OFF: usize = 8 + 852;

/// [`ArchBootState`] latch holding the EMEM window cursor, as a byte offset.
const LATCH_EMEM_CURSOR: usize = 0;
/// [`ArchBootState`] latch holding whether the cursor auto-increments on write.
const LATCH_EMEM_AINCW: usize = 1;

// ── encodings ─────────────────────────────────────────────────────────────────────

/// `NV_PFALCON_FALCON_CPUCTL_STARTCPU` is `1:1`
/// (`ogkm-580: turing/tu102/dev_falcon_v4.h:47`).
const CPUCTL_STARTCPU: u64 = 0x2;
/// `NV_PFALCON_FALCON_CPUCTL_HALTED` is `4:4`
/// (`ogkm-580: turing/tu102/dev_falcon_v4.h:50`).
const CPUCTL_HALTED: u64 = 0x10;
/// `NV_PFALCON_FALCON_HWCFG2_RISCV` is `10:10`
/// (`ogkm-580: turing/tu102/dev_falcon_v4.h:63`); `_ENABLE` is `1`
/// (`ogkm-580: hopper/gh100/dev_falcon_v4.h:71`).
const HWCFG2_RISCV_ENABLE: u64 = 0x400;
/// `NV_PFALCON_FALCON_HWCFG2_RISCV_BR_PRIV_LOCKDOWN` is `13:13` and `_LOCK` is `1`
/// (`ogkm-580: hopper/gh100/dev_falcon_v4.h:79-80`).
const HWCFG2_BR_PRIV_LOCKDOWN: u64 = 1 << 13;
/// `NV_PFALCON_FALCON_DMATRFCMD_IDLE` is `1:1`
/// (`ogkm-580: turing/tu102/dev_falcon_v4.h:79`); reported `IDLE=TRUE|FULL=FALSE`.
const DMATRFCMD_IDLE: u64 = 0x2;
/// `NV_PRISCV_RISCV_CPUCTL_ACTIVE_STAT` is `7:7`
/// (`ogkm-580: ampere/ga102/dev_riscv_pri.h:33`, the header GB202's
/// `kflcnRiscvReadIntrStatus_GA102` binding selects).
const RISCV_CPUCTL_ACTIVE: u64 = 0x80;
/// `NV_PFALCON_FALCON_IRQSTAT_SWGEN0` is `6:6`
/// (`ogkm-580: turing/tu102/dev_falcon_v4.h:34`).
const IRQSTAT_SWGEN0: u64 = 1 << 6;

/// `NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE_STATUS_SUCCESS`
/// (`ogkm-580: blackwell/gb202/dev_therm_addendum.h:29`). The field is the whole 32-bit
/// word (`_STATUS` is `31:0`, `:28`), so the register reads exactly this value — not this
/// value OR-ed into anything.
const FSP_BOOT_COMPLETE_SUCCESS: u64 = 0x0000_00FF;

/// `"RMARGS"`, little-endian ASCII in an 8-byte LibOS region id.
const RMARGS_ID: u64 = 0x0000_524d_4152_4753;

// ⊘ The old `WPR2_LO_UP`/`WPR2_HI_UP` (`0x02FF_E000`, self-described INVENTED) are gone: WPR2 is
// derived from the framebuffer size, as the falcon model derives it.


// ══════════════════════════════════════════════════════════════════════════════════════
// The GB20x boot SEQUENCE
// ══════════════════════════════════════════════════════════════════════════════════════

/// The **FSP command-queue** boot regime as GB202 runs it: Hopper's transport, plus the
/// `NV_THERM_I2CS_SCRATCH` boot-complete gate that precedes it and the moved offset that
/// gate lives at.
///
/// ★ Written as a separate type rather than a reuse of [`crate::gh100::Gh100FspBoot`]
/// **on purpose**. The two differ in one register and one stage, and expressing that as a
/// flag on the Hopper type would put a Blackwell fact inside another generation's
/// ordering — exactly the coupling task #121 built the seam to abolish. Adding this file
/// changes zero lines of `kf_gsp::seq`, of `kayfabe_device::ga10x`, of
/// [`crate::ad10x`] or of [`crate::gh100`].
/// ★ The whole per-family difference between Hopper and Blackwell's FSP regimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FspRow {
    /// `NV_THERM_I2CS_SCRATCH` — the FSP boot-complete gate, when the family has one.
    pub therm_boot_scratch: Option<u64>,
    /// The WPR2 `(LO, HI)` register pair the driver polls.
    pub wpr2: (u64, u64),
    /// A second WPR2 pair decoded too — Blackwell's is driver-version keyed (580 vs 610).
    pub wpr2_alt: Option<(u64, u64)>,
}

impl FspRow {
    /// Hopper: no thermal gate; WPR2 at `NV_PFB_PRI_MMU_WPR2_ADDR_*` (`hopper/gh100/dev_fb.h:43-48`).
    pub const HOPPER: FspRow = FspRow {
        therm_boot_scratch: None,
        wpr2: (WPR2_ADDR_LO_580, WPR2_ADDR_HI_580),
        wpr2_alt: None,
    };
    /// Blackwell: the thermal gate, and WPR2 at the 610 HUBMMU pair AND the 580 FB pair.
    pub const BLACKWELL: FspRow = FspRow {
        therm_boot_scratch: Some(THERM_I2CS_SCRATCH),
        wpr2: (WPR2_ADDR_LO, WPR2_ADDR_HI),
        wpr2_alt: Some((WPR2_ADDR_LO_580, WPR2_ADDR_HI_580)),
    };
}

/// The FSP boot sequence, for a family's [`FspRow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FspBoot {
    therm: Option<u64>,
}

impl FspBoot {
    /// The regime.
    #[must_use]
    pub fn new(row: FspRow) -> FspBoot {
        FspBoot { therm: row.therm_boot_scratch }
    }

    /// Decode `NV_PFSP_EMEMC` into a byte cursor and an auto-increment flag.
    fn decode_ememc(value: u64) -> (u64, bool) {
        let dwords = ((value >> EMEMC_BLK_SHIFT) & EMEMC_BLK_MASK) * DWORDS_PER_EMEM_BLOCK
            + ((value >> EMEMC_OFFS_SHIFT) & EMEMC_OFFS_MASK);
        (dwords * 4, value & EMEMC_AINCW != 0)
    }

    /// The inverse, so a guest that reads `EMEMC` back sees its own cursor.
    fn encode_ememc(byte_cursor: u64, aincw: bool) -> u64 {
        let dwords = byte_cursor / 4;
        let mut v = ((dwords / DWORDS_PER_EMEM_BLOCK) & EMEMC_BLK_MASK) << EMEMC_BLK_SHIFT
            | ((dwords % DWORDS_PER_EMEM_BLOCK) & EMEMC_OFFS_MASK) << EMEMC_OFFS_SHIFT;
        if aincw {
            v |= EMEMC_AINCW;
        }
        v
    }
}

/// See [`BootSequence::stages`] — the same three the FSP regime declares. The
/// FSP-boot-complete poll is a **precondition**, not a stage: it publishes no
/// [`BootStep`], because nothing about the GSP has happened when it passes.
const FSP_STAGES: &[BootStageDesc] = &[
    BootStageDesc {
        name: "the FSP command-queue HEAD write starts the GSP-FMC and raises FRTS",
        step: BootStepKind::StartProcessor,
    },
    BootStageDesc {
        name: "the same command loads GSP-RM — there is no separate Booter Load",
        step: BootStepKind::FirmwareLoaded,
    },
    BootStageDesc {
        name: "the boot-args pointer comes from the command payload, not a mailbox pair",
        step: BootStepKind::PublishBootArgs,
    },
];

impl BootSequence for FspBoot {
    fn stages(&self) -> &'static [BootStageDesc] {
        FSP_STAGES
    }

    fn on_write(
        &self,
        model: &dyn GspModel,
        w: &RegWrite,
        _ctx: &BootContext,
        state: &mut ArchBootState,
    ) -> BootSteps {
        let (off, val) = (w.off, w.val);
        if w.bar != 0 {
            return BootSteps::none();
        }
        #[allow(clippy::cast_possible_truncation)]
        match off {
            PFSP_EMEMC => {
                let (cursor, aincw) = FspBoot::decode_ememc(val);
                state.set_latch(LATCH_EMEM_CURSOR, cursor);
                state.set_latch(LATCH_EMEM_AINCW, u64::from(aincw));
                return BootSteps::none();
            }
            PFSP_EMEMD => {
                let cursor = state.latch(LATCH_EMEM_CURSOR);
                if state.window_write_u32(cursor as usize, val as u32)
                    && state.latch(LATCH_EMEM_AINCW) != 0
                {
                    state.set_latch(LATCH_EMEM_CURSOR, cursor + 4);
                }
                return BootSteps::none();
            }
            // Written before the head, and on its own it means nothing: the driver's own
            // comment is that the head write is the one that interrupts FSP
            // (`ogkm-580: kern_fsp_gh100.c:86-99`).
            PFSP_QUEUE_TAIL => return BootSteps::none(),
            PFSP_QUEUE_HEAD => {
                let Some(gpa) = state.window_read_u64(COT_BOOT_ARGS_OFF) else {
                    // MISS = FAULT-shaped: the guest never streamed a payload this long,
                    // so there is no boot-args pointer and we invent none.
                    return BootSteps::none();
                };
                let mut steps = BootSteps::none();
                steps.push(BootStep::StartProcessor);
                steps.push(BootStep::FirmwareLoaded);
                steps.push(BootStep::PublishBootArgs(gpa));
                return steps;
            }
            // Read-only from the guest's side on the boot path; a write is not part of any
            // sequence RM runs, and we publish nothing for it.
            o if Some(o) == self.therm => return BootSteps::none(),
            _ => {}
        }
        let Some(reg) = w.reg else {
            return BootSteps::none();
        };
        match reg {
            GspReg::GspQueueHead(_) => BootSteps::one(BootStep::CommandDoorbell),
            GspReg::GspFalconIrqsclr if model.is_swgen0_clear(val) => {
                BootSteps::one(BootStep::ClearStatusIrq)
            }
            // Shadowed so a read-back is the guest's own value, but they publish nothing:
            // on this regime RM never programs them, and a non-zero `MAILBOX0` is how the
            // GSP-FMC reports a boot ERROR (`ogkm-580: kernel_gsp_gh100.c:515-535`).
            #[allow(clippy::cast_possible_truncation)]
            GspReg::GspFalconMailbox0 => BootSteps::one(BootStep::BootArgsLo(val as u32)),
            #[allow(clippy::cast_possible_truncation)]
            GspReg::GspFalconMailbox1 => BootSteps::one(BootStep::BootArgsHi(val as u32)),
            _ => BootSteps::none(),
        }
    }

    /// ★★★★★ **w567's lesson, applied at write time rather than after a silent failure.**
    /// `RegPlane::read_inner` refuses an offset whose `decode_reg` is `None` unless
    /// `may_read` claims it, and the default is `false` — so an `on_read` arm without a
    /// matching arm here is dead code that looks alive, and an unclaimed BAR0 read answers
    /// `0` with no fault and no counter.
    ///
    /// ★ Written as the SAME arms as [`Self::on_read`], in the same order, so the two
    /// cannot disagree about which offsets this regime owns.
    /// `crates/kayfabe-chips/tests/boot_sequence_read_gate.rs` sweeps the aperture and
    /// checks it.
    fn may_read(&self, bar: u8, off: u64) -> bool {
        bar == 0
            && matches!(
                off,
                PFSP_QUEUE_HEAD
                    | PFSP_QUEUE_TAIL
                    | PFSP_MSGQ_HEAD
                    | PFSP_MSGQ_TAIL
                    | PFSP_EMEMC
                    | PFSP_EMEMD
            )
            || Some(off) == self.therm
    }

    fn on_read(
        &self,
        _model: &dyn GspModel,
        bar: u8,
        off: u64,
        _ctx: &BootContext,
        state: &ArchBootState,
    ) -> Option<u64> {
        if bar != 0 {
            return None;
        }
        match off {
            // "FSP will set QUEUE_HEAD = TAIL after each packet is received"
            // (`ogkm-580: kern_fsp_gh100.c:241-256`), and our FSP consumes synchronously,
            // so both queues always read drained — the predicate
            // `kfspCanSendPacket_GH100` gates the next send on.
            PFSP_QUEUE_HEAD | PFSP_QUEUE_TAIL | PFSP_MSGQ_HEAD | PFSP_MSGQ_TAIL => Some(0),
            PFSP_EMEMC => Some(FspBoot::encode_ememc(
                state.latch(LATCH_EMEM_CURSOR),
                state.latch(LATCH_EMEM_AINCW) != 0,
            )),
            #[allow(clippy::cast_possible_truncation)]
            PFSP_EMEMD => Some(u64::from(
                state
                    .window_read_u32(state.latch(LATCH_EMEM_CURSOR) as usize)
                    .unwrap_or(0),
            )),
            // ★★★ The gate. FSP has "completed boot out of chip reset" from the instant
            // this model exists — there is no earlier observable for it to be false at,
            // because the emulated device has no bootfsm and nothing the guest can do
            // makes FSP boot later.
            //
            // ⊘ That is a DELIBERATE simplification and it is a real difference from
            // silicon, where the poll can take seconds (`kern_fsp_gb202.c:77-78` sets a
            // five-second timeout). Nothing in this port needs the wait to be observable;
            // if something ever does, the answer belongs on `GspObservation`, not here.
            o if Some(o) == self.therm => Some(FSP_BOOT_COMPLETE_SUCCESS),
            _ => None,
        }
    }
}

kf_util::assert_send_sync!(FspBoot);

/// The GB20x (consumer Blackwell) GSP register model — see the module docs.
/// ★ The FSP-booted GSP model for one family's row and one framebuffer size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FspGspModel {
    boot: FspBoot,
    row: FspRow,
    fb_size_mb: u64,
}

impl FspGspModel {
    /// The model.
    #[must_use]
    pub fn new(row: FspRow, fb_size_mb: u64) -> FspGspModel {
        FspGspModel { boot: FspBoot::new(row), row, fb_size_mb }
    }

    /// Where this model puts a register. **`None` is the interesting answer**: it means
    /// this generation has no such register, and five variants return it.
    #[must_use]
    pub fn reg_at(&self, reg: GspReg) -> Option<(u8, u64)> {
        let off = match reg {
            GspReg::GspFalconCpuctl => PGSP + FALCON_CPUCTL,
            GspReg::GspFalconHwcfg2 => PGSP + FALCON_HWCFG2,
            GspReg::GspFalconDmatrfcmd => PGSP + FALCON_DMATRFCMD,
            GspReg::GspFalconMailbox0 => PGSP + FALCON_MAILBOX0,
            GspReg::GspFalconMailbox1 => PGSP + FALCON_MAILBOX1,
            GspReg::GspFalconIrqstat => PGSP + FALCON_IRQSTAT,
            GspReg::GspFalconIrqmask => PGSP + FALCON_IRQMASK,
            GspReg::GspFalconIrqdest => PGSP + FALCON_IRQDEST,
            GspReg::GspFalconIrqsclr => PGSP + FALCON_IRQSCLR,
            GspReg::GspRiscvCpuctl => GSP_RISCV_CPUCTL,
            GspReg::GspRiscvIrqmask => GSP_RISCV_IRQMASK,
            GspReg::GspRiscvIrqdest => GSP_RISCV_IRQDEST,
            GspReg::GspRiscvBcrCtrl => GSP_RISCV_BCR_CTRL,
            GspReg::Wpr2AddrLo => self.row.wpr2.0,
            GspReg::Wpr2AddrHi => self.row.wpr2.1,
            GspReg::GspQueueHead(i) if u64::from(i) < QUEUE_HEAD_COUNT => {
                QUEUE_HEAD0 + u64::from(i) * 8
            }
            GspReg::GspQueueHead(_) => return None,
            // ── the absences, each sourced in the module docs ──
            GspReg::GfwBootProgress
            | GspReg::GfwBootPlm
            | GspReg::Sec2FalconCpuctl
            | GspReg::Sec2FalconMailbox0
            | GspReg::Sec2FalconDmatrfcmd => return None,
        };
        Some((0, off))
    }
}

impl GspModel for FspGspModel {
    fn at(&self, reg: GspReg) -> Option<(u8, u64)> {
        self.reg_at(reg)
    }

    fn decode_reg(&self, bar: u8, off: u64) -> Option<GspReg> {
        if bar != 0 {
            return None;
        }
        Some(match off {
            GSP_RISCV_CPUCTL => GspReg::GspRiscvCpuctl,
            GSP_RISCV_IRQMASK => GspReg::GspRiscvIrqmask,
            GSP_RISCV_IRQDEST => GspReg::GspRiscvIrqdest,
            GSP_RISCV_BCR_CTRL => GspReg::GspRiscvBcrCtrl,
            // ★★★★★ BOTH driver generations' WPR2 addresses — see [`WPR2_ADDR_LO`].
            o if o == self.row.wpr2.0 || self.row.wpr2_alt.is_some_and(|a| a.0 == o) => GspReg::Wpr2AddrLo,
            o if o == self.row.wpr2.1 || self.row.wpr2_alt.is_some_and(|a| a.1 == o) => GspReg::Wpr2AddrHi,
            #[allow(clippy::cast_possible_truncation)]
            q if (QUEUE_HEAD0..QUEUE_HEAD0 + QUEUE_HEAD_COUNT * 8).contains(&q)
                && (q - QUEUE_HEAD0).is_multiple_of(8) =>
            {
                GspReg::GspQueueHead(((q - QUEUE_HEAD0) / 8) as u8)
            }
            _ => match (off & !0xFFFF, off & 0xFFFF) {
                (PGSP, FALCON_IRQSCLR) => GspReg::GspFalconIrqsclr,
                (PGSP, FALCON_IRQSTAT) => GspReg::GspFalconIrqstat,
                (PGSP, FALCON_IRQMASK) => GspReg::GspFalconIrqmask,
                (PGSP, FALCON_IRQDEST) => GspReg::GspFalconIrqdest,
                (PGSP, FALCON_MAILBOX0) => GspReg::GspFalconMailbox0,
                (PGSP, FALCON_MAILBOX1) => GspReg::GspFalconMailbox1,
                (PGSP, FALCON_HWCFG2) => GspReg::GspFalconHwcfg2,
                (PGSP, FALCON_CPUCTL) => GspReg::GspFalconCpuctl,
                (PGSP, FALCON_DMATRFCMD) => GspReg::GspFalconDmatrfcmd,
                _ => return None,
            },
        })
    }

    fn is_startcpu(&self, value: u64) -> bool {
        value & CPUCTL_STARTCPU != 0
    }

    /// **Always false.** There is no SEC2 Booter Load/Unload argument convention on the
    /// FSP boot path, so there is no value of this argument that means "unload".
    /// Answering `true` for some sentinel would be inventing a protocol.
    fn is_booter_unload(&self, _sec2_mailbox0: u32) -> bool {
        false
    }

    fn is_swgen0_clear(&self, value: u64) -> bool {
        value & IRQSTAT_SWGEN0 != 0
    }

    fn encode(&self, reg: GspReg, obs: &GspObservation) -> Option<u64> {
        Some(match reg {
            GspReg::GspFalconCpuctl => CPUCTL_HALTED,
            // One register, three readers, different bits at different stages — the same
            // structure Hopper has, because it is the same `kgspBootstrap_GH100` code:
            // `_kfspIsGspTargetMaskReleased` requires the whole word non-zero and not the
            // `0xBADF41xx` priv-error pattern (`ogkm-580: kern_fsp_gh100.c:1096-1114`),
            // `_kgspLockdownReleasedOrFmcError` requires `RISCV_BR_PRIV_LOCKDOWN_UNLOCK`
            // (`ogkm-580: kernel_gsp_gh100.c:530-534`).
            GspReg::GspFalconHwcfg2 => {
                if obs.stage.wpr2_up() {
                    HWCFG2_RISCV_ENABLE
                } else {
                    HWCFG2_RISCV_ENABLE | HWCFG2_BR_PRIV_LOCKDOWN
                }
            }
            GspReg::GspFalconDmatrfcmd => DMATRFCMD_IDLE,
            // A non-zero MAILBOX0 is how the GSP-FMC reports a boot ERROR on this regime
            // (`ogkm-580: kernel_gsp_gh100.c:552,562`), so the boot-args echo the falcon
            // regime serves here is LOST rather than served with the wrong meaning.
            GspReg::GspFalconMailbox0 => 0,
            GspReg::GspFalconMailbox1 => u64::from(obs.boot_args_hi),
            GspReg::GspFalconIrqstat => {
                if obs.swgen0_pending {
                    IRQSTAT_SWGEN0
                } else {
                    0
                }
            }
            // ★★★★★ §16.77 — the falcon PAIR and the RISC-V PAIR both advertise SWGEN0.
            // `kflcnGetPendingHostInterrupts` reads ONE pair or the OTHER depending on a
            // mode latched at bootstrap, so answering only the falcon pair made the RISC-V
            // AND collapse to zero.
            GspReg::GspFalconIrqmask
            | GspReg::GspFalconIrqdest
            | GspReg::GspRiscvIrqmask
            | GspReg::GspRiscvIrqdest => IRQSTAT_SWGEN0,
            GspReg::GspFalconIrqsclr => 0,
            // ★ w827: the core the guest last selected, VALID (`kf_arch::gsp::GspReg::GspRiscvBcrCtrl`);
            // never written = the reset value (FALCON, not VALID).
            GspReg::GspRiscvBcrCtrl => obs.riscv_bcr_ctrl.map_or(0, |v| u64::from(v) | BCR_CTRL_VALID),
            GspReg::GspRiscvCpuctl => {
                if obs.riscv_active {
                    RISCV_CPUCTL_ACTIVE
                } else {
                    0
                }
            }
            GspReg::Wpr2AddrLo => {
                if obs.wpr2_up {
                    crate::falcon_gsp::wpr2_reg(crate::falcon_gsp::frts_offset_for(self.fb_size_mb))
                } else {
                    0
                }
            }
            GspReg::Wpr2AddrHi => {
                if obs.wpr2_up {
                    crate::falcon_gsp::wpr2_reg(crate::falcon_gsp::gsp_fw_wpr_end_for(self.fb_size_mb))
                } else {
                    0
                }
            }
            // Write-only from our side.
            GspReg::GspQueueHead(_) => 0,
            // MISS = FAULT. This generation has no such register; a defaulted zero would
            // be a guess, and the FSM raises `RegisterUnserviceable` instead.
            GspReg::GfwBootProgress
            | GspReg::GfwBootPlm
            | GspReg::Sec2FalconCpuctl
            | GspReg::Sec2FalconMailbox0
            | GspReg::Sec2FalconDmatrfcmd => return None,
        })
    }

    fn boot_sequence(&self) -> &dyn BootSequence {
        &self.boot
    }

    fn libos_region_layout(&self) -> LibosRegionLayout {
        // The LibOS region descriptor is a driver-side structure, not a chip register, and
        // `libos_init_args.h` is architecture-independent — so this half genuinely is the
        // same on every generation this crate models.
        LibosRegionLayout {
            entry_stride: 32,
            id_offset: 0,
            pa_offset: 8,
            size_offset: 16,
            max_entries: 4096,
            rmargs_id: RMARGS_ID,
        }
    }
}
