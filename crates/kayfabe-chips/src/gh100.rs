//! Axis B: the **GH100** (Hopper) GSP model and boot sequence — the *second boot
//! ordering*.
//!
//! ## What this module is for
//!
//! It is not a working Hopper port and does not pretend to be one. It exists to answer a
//! question about this workspace's seams by making the edit: **can a second architecture
//! be implemented by adding alongside, without modifying the first one's?**
//!
//! ## The measurement, in two parts
//!
//! ### 2026-07-30 (#118): the answer was NO, and the reason was named
//!
//! Hopper's *offsets* are largely Ampere's — `NV_PGSP_FALCON_MAILBOX0` is still
//! `0x110040` (`ogkm-580: hopper/gh100/dev_gsp.h:26`), `NV_PFB_PRI_MMU_WPR2_ADDR_LO/HI`
//! are still `0x001FA824`/`0x001FA828` (`ogkm-580: hopper/gh100/dev_fb.h:43,48`), and
//! `NV_PRISCV_RISCV_CPUCTL` is still `0x388`
//! (`ogkm-580: hopper/gh100/dev_riscv_pri.h:58`). If a generation were only offsets, this
//! would be another table row — and it is not offsets.
//!
//! The blocker was never that a generation has arch-specific code. It was that the
//! arch-specific code had **nowhere to go**: `GspFsm::mmio_write` matched [`GspReg`] arm
//! by arm and *encoded the falcon/secure-booter ordering in the match itself*, so a
//! generation reaching the same truths through different registers could be expressed
//! only by editing that match — in `kayfabe-gsp`, a logic crate, and in the ordering
//! GA10x boots on today.
//!
//! ### 2026-07-31 (#121): the seam was built, and the answer is YES
//!
//! [`Gh100FspBoot`] is this generation's boot ordering, implemented as a
//! [`BootSequence`] living in this crate at this crate's offsets.
//!
//! ★★★ **The acceptance measurement, and how to re-take it.** The commit that landed this
//! sequence sits directly on the seam commit `e5c0f45`, and
//! `git diff --numstat e5c0f45..HEAD` names exactly **two** files: this one and
//! `tests/tests/arch_axis_second_generation.rs`. Lines changed in the falcon regime's
//! implementation — `crates/kayfabe-gsp/src/seq.rs`,
//! `crates/kayfabe-device/src/ga10x.rs`, `crates/kayfabe-chips/src/ad10x.rs` — and in the
//! logic crates `kayfabe-arch` and `kayfabe-gsp`: **zero**. That is the criterion,
//! executed rather than argued.
//!
//! See [`Gh100FspBoot`] for what the driver actually does and for the honest list of what
//! this fixture does not model.
//!
//! ## What is really absent on this generation, and what only looked absent
//!
//! | [`GspReg`] variant | on GH100 | evidence (`ogkm-580`) |
//! |---|---|---|
//! | [`GspReg::GfwBootProgress`], [`GspReg::GfwBootPlm`] | **absent** | `hopper/gh100/dev_gc6_island*.h` define no `SECURE_SCRATCH_GROUP_05` (they define `GROUP_20`, the Confidential-Compute mode word, instead). NVIDIA's own generated HAL binds `gpuWaitForGfwBootComplete_TU102` for exactly `TU102…AD107` and the not-supported stub `_5baef9` for everything else (`src/nvidia/generated/g_gpu_nvoc.c:2374-2385`). The GFW-boot poll **does not run on this chip.** |
//! | [`GspReg::Sec2FalconCpuctl`], [`GspReg::Sec2FalconMailbox0`] | **not the boot path** | `kgspBootstrap_GH100` boots the GSP-FMC through **FSP** (`kfspSendBootCommands_HAL`) and only falls back to SEC2 behind `PDB_PROP_KSEC2_BOOT_GSPFMC` (`kernel_gsp_gh100.c:833-876`). There is no Booter Load/Unload argument convention to latch. |
//! | [`GspReg::GspQueueHead`] | ★ **PRESENT — the #118 reading was wrong** | see [`QUEUE_HEAD0`]. `kgspSetCmdQueueHead` is halified, and the *Turing* binding is the fallback for every chip that is not a VF or a Tegra part (`generated/g_kernel_gsp_nvoc.c:664-679`), so GH100 writes `NV_PGSP_QUEUE_HEAD(i)` on every RPC send. "This chip's header does not define the symbol" is a fact about headers, not about which registers the driver writes. |
//!
//! ## What this model deliberately does NOT do
//!
//! It does not fake a `GfwBootProgress` or a SEC2 Booter offset to make the FSM run.
//! Answering a register the silicon does not have is exactly the *"defaulted zero"* the
//! `GspModel` seam exists to refuse: `decode_reg` returns `None` (*"another model owns
//! this offset"*) and `encode` returns `None` (a `RegisterUnserviceable` fault). A green
//! obtained by inventing registers would be a measurement of nothing.

use kayfabe_arch::gsp::{
    ArchBootState, BootContext, BootSequence, BootStageDesc, BootStep, BootStepKind, BootSteps,
    GspModel, GspObservation, GspReg, LibosRegionLayout, RegWrite,
};
use kayfabe_arch::ids::{ClassId, ControlCmd, VChid};
use kayfabe_arch::{Arch, DoorbellTarget, GmmuFmt, ObjectKind, PushbufferAbi, UserdModel};
use kayfabe_mocks::MockArch;

// ── BAR0 offsets (`ogkm-580: src/common/inc/swref/published/hopper/gh100/`) ───────

/// GSP falcon block base — `NV_PGSP` is `0x113fff:0x110000`
/// (`ogkm-580: hopper/gh100/dev_gsp.h:25`).
const PGSP: u64 = 0x0011_0000;

/// `NV_PFALCON_FALCON_IRQSCLR` (`ogkm-580: hopper/gh100/dev_falcon_v4.h`).
const FALCON_IRQSCLR: u64 = 0x004;
/// `NV_PFALCON_FALCON_IRQSTAT`.
const FALCON_IRQSTAT: u64 = 0x008;
/// `NV_PFALCON_FALCON_IRQMASK`.
const FALCON_IRQMASK: u64 = 0x018;
/// `NV_PFALCON_FALCON_IRQDEST`.
const FALCON_IRQDEST: u64 = 0x01c;
/// `NV_PFALCON_FALCON_MAILBOX0` — `0x110040` absolute
/// (`ogkm-580: hopper/gh100/dev_gsp.h:26`, `dev_falcon_v4.h:27`).
const FALCON_MAILBOX0: u64 = 0x040;
/// `NV_PFALCON_FALCON_MAILBOX1` — `0x110044` (`ogkm-580: hopper/gh100/dev_gsp.h:29`).
const FALCON_MAILBOX1: u64 = 0x044;
/// `NV_PFALCON_FALCON_HWCFG2` (`ogkm-580: hopper/gh100/dev_falcon_v4.h:39`).
const FALCON_HWCFG2: u64 = 0x0f4;
/// `NV_PFALCON_FALCON_CPUCTL`.
const FALCON_CPUCTL: u64 = 0x100;
/// `NV_PFALCON_FALCON_DMATRFCMD`.
const FALCON_DMATRFCMD: u64 = 0x118;

/// GSP RISC-V `CPUCTL` = RISC-V register base + `NV_PRISCV_RISCV_CPUCTL` `0x388`
/// (`ogkm-580: hopper/gh100/dev_riscv_pri.h:58`).
///
/// ★ **The BASE is ASSUMED, not read.** `NV_FALCON2_GSP_BASE` is defined at
/// `turing/tu102`, `ampere/ga100` and `ampere/ga102` (all `0x00111000`) and is **not**
/// defined under `hopper/gh100/` in the vendored tree. This model reuses the Ampere base
/// because the fixture needs *an* offset to decode; nothing in this crate's conclusion
/// depends on it, and it is flagged so it is never mistaken for a measurement.
const GSP_RISCV_CPUCTL: u64 = 0x0011_1388;

/// `NV_PFB_PRI_MMU_WPR2_ADDR_LO` (`ogkm-580: hopper/gh100/dev_fb.h:43`).
const WPR2_ADDR_LO: u64 = 0x001F_A824;
/// `NV_PFB_PRI_MMU_WPR2_ADDR_HI` (`ogkm-580: hopper/gh100/dev_fb.h:48`).
const WPR2_ADDR_HI: u64 = 0x001F_A828;

/// `NV_PGSP_QUEUE_HEAD(0)` — the GSP command-queue doorbell.
///
/// ★★★ **CORRECTED 2026-07-31, and the correction is a lesson about reading a HAL.** The
/// #118 measurement recorded this register as *absent* on GH100 because
/// `hopper/gh100/dev_gsp.h` does not define `NV_PGSP_QUEUE_HEAD` and "the only writer in
/// the tree is the Turing HAL". Both halves are literally true; the inference from them is
/// **wrong**. `kgspSetCmdQueueHead` is halified with three bindings and the Turing one is
/// the *fallback for every chip*: only `RmVariantHal: VF` and `ChipHal: T234D | T264D` get
/// the not-supported stub, so **GH100 binds `kgspSetCmdQueueHead_TU102`**
/// (`ogkm-580: src/nvidia/generated/g_kernel_gsp_nvoc.c:664-679`), which writes
/// `NV_PGSP_QUEUE_HEAD(queueIdx)` (`ogkm-580: kernel_gsp_tu102.c:343-357`) —
/// `0x110c00+(i)*8`, defined in the *Turing/Ampere* header the Turing HAL includes
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_gsp.h:38`), and reached on
/// every RPC send (`ogkm-580: kernel_gsp.c:425`). "This chip's header does not define the
/// symbol" is a statement about headers, not about which registers the driver writes.
const QUEUE_HEAD0: u64 = 0x0011_0C00;
/// How many command queues `NV_PGSP_QUEUE_HEAD__SIZE_1` declares
/// (`ogkm-580: ampere/ga102/dev_gsp.h:39`).
const QUEUE_HEAD_COUNT: u64 = 8;

// ── the FSP command channel (`ogkm-580: hopper/gh100/dev_fsp_pri.h`) ──────────────
//
// ★★ These are the registers that make this generation a real test of the seam: no
// `GspReg` variant names any of them, they are what its boot is *driven by*, and the
// boot-sequence seam is the only place they can be answered from.

/// `NV_PFSP_EMEMC(FSP_EMEM_CHANNEL_RM)` — the EMEM window's cursor.
/// `0x008F2ac0+(i)*8` at `dev_fsp_pri.h:26`; RM's channel is 0
/// (`ogkm-580: src/nvidia/arch/nvalloc/common/inc/fsp/fsp_emem_channels.h:34`).
const PFSP_EMEMC: u64 = 0x008F_2AC0;
/// `NV_PFSP_EMEMD(FSP_EMEM_CHANNEL_RM)` — the EMEM window's data port
/// (`ogkm-580: dev_fsp_pri.h:40`).
const PFSP_EMEMD: u64 = 0x008F_2AC4;
/// `NV_PFSP_QUEUE_HEAD(FSP_EMEM_CHANNEL_RM)` (`ogkm-580: dev_fsp_pri.h:53`).
const PFSP_QUEUE_HEAD: u64 = 0x008F_2C00;
/// `NV_PFSP_QUEUE_TAIL(FSP_EMEM_CHANNEL_RM)` (`ogkm-580: dev_fsp_pri.h:57`).
const PFSP_QUEUE_TAIL: u64 = 0x008F_2C04;
/// `NV_PFSP_MSGQ_HEAD(FSP_EMEM_CHANNEL_RM)` (`ogkm-580: dev_fsp_pri.h:44`).
const PFSP_MSGQ_HEAD: u64 = 0x008F_2C80;
/// `NV_PFSP_MSGQ_TAIL(FSP_EMEM_CHANNEL_RM)` (`ogkm-580: dev_fsp_pri.h:48`).
const PFSP_MSGQ_TAIL: u64 = 0x008F_2C84;

/// `NV_PFSP_EMEMC_OFFS` is `7:2` — a DWORD offset within a block
/// (`ogkm-580: dev_fsp_pri.h:28`).
const EMEMC_OFFS_SHIFT: u32 = 2;
/// The `7:2` field is six bits wide.
const EMEMC_OFFS_MASK: u64 = 0x3F;
/// `NV_PFSP_EMEMC_BLK` is `15:8` (`ogkm-580: dev_fsp_pri.h:30`).
const EMEMC_BLK_SHIFT: u32 = 8;
/// The `15:8` field is eight bits wide.
const EMEMC_BLK_MASK: u64 = 0xFF;
/// `NV_PFSP_EMEMC_AINCW` is `24:24` — auto-increment the cursor on every write
/// (`ogkm-580: dev_fsp_pri.h:32`).
const EMEMC_AINCW: u64 = 1 << 24;
/// `DWORDS_PER_EMEM_BLOCK` (`ogkm-580: kern_fsp_gh100.c:57`).
const DWORDS_PER_EMEM_BLOCK: u64 = 64;

/// Byte offset, **within the EMEM window**, of the COT payload's
/// `gspBootArgsSysmemOffset` — the boot-args guest-physical address on this generation.
///
/// Two DWORDs of packet header (MCTP + NVDM,
/// `ogkm-580: src/nvidia/src/kernel/gpu/fsp/kern_fsp.c:51-52, 488-506`) followed by a
/// `#pragma pack(1)` `NVDM_PAYLOAD_COT`, whose fields sum to 852 bytes before this one
/// (`ogkm-580: src/nvidia/inc/kernel/gpu/fsp/kern_fsp_cot_payload.h:36-55`: `2+2+8+8+4+
/// 8+4` then `hash384[12]`, `publicKey[96]` and `signature[96]` as `NvU32` arrays).
/// 8 + 852 = 860, and the whole 868-byte packet fits the 1024-byte RM channel
/// (`FSP_EMEM_CHANNEL_RM_SIZE`, `ogkm-580: fsp_emem_channels.h:37`), so it is a **single**
/// packet and no reassembly is involved.
const COT_BOOT_ARGS_OFF: usize = 8 + 852;

/// [`ArchBootState`] latch holding the EMEM window cursor, as a **byte** offset.
const LATCH_EMEM_CURSOR: usize = 0;
/// [`ArchBootState`] latch holding whether the cursor auto-increments on write.
const LATCH_EMEM_AINCW: usize = 1;

// ── encodings ─────────────────────────────────────────────────────────────────────

/// `NV_PFALCON_FALCON_CPUCTL_STARTCPU` — bit 1.
const CPUCTL_STARTCPU: u64 = 0x2;
/// `NV_PFALCON_FALCON_CPUCTL_HALTED_TRUE` — bit 4.
const CPUCTL_HALTED: u64 = 0x10;
/// `NV_PFALCON_FALCON_HWCFG2_RISCV_ENABLE`.
const HWCFG2_RISCV_ENABLE: u64 = 0x400;
/// `NV_PFALCON_FALCON_DMATRFCMD` reporting `IDLE=TRUE|FULL=FALSE`.
const DMATRFCMD_IDLE: u64 = 0x2;
/// `NV_PRISCV_RISCV_CPUCTL_ACTIVE_STAT` — bit 7.
const RISCV_CPUCTL_ACTIVE: u64 = 0x80;
/// `NV_PFALCON_FALCON_IRQSTAT_SWGEN0` — bit 6.
const IRQSTAT_SWGEN0: u64 = 1 << 6;
/// `NV_PFALCON_FALCON_HWCFG2_RISCV_BR_PRIV_LOCKDOWN` is `13:13`, and `_LOCK` is `1`
/// (`ogkm-580: hopper/gh100/dev_falcon_v4.h:79-81`).
const HWCFG2_BR_PRIV_LOCKDOWN: u64 = 1 << 13;

/// ★ **INVENTED** chip parameter — see `ad10x`'s note. Only nonzero-vs-zero is
/// load-bearing, and on this generation not even that is reliable:
/// `kgspIsWpr2Up_GH100` returns `NV_FALSE` **unconditionally** when Confidential Compute
/// is enabled, because the BAR0 decoupler may make the MMU registers unreadable
/// (`ogkm-580: kernel_gsp_gh100.c:220-236`). A CC-on Hopper guest therefore never
/// believes this register at all — another observable the current model has no way to
/// express, since `GspObservation` carries no CC state.
const WPR2_LO_UP: u64 = 0x02FF_E000;
/// See [`WPR2_LO_UP`].
const WPR2_HI_UP: u64 = 0x02FF_F000;

/// `"RMARGS"`, little-endian ASCII in an 8-byte LibOS region id.
const RMARGS_ID: u64 = 0x0000_524d_4152_4753;

/// ★★★ **The boot events this generation has that no [`GspReg`] variant names — and how
/// each is served now.**
///
/// This list used to be called `MISSING_TRANSITIONS` and it was the enumerated *cost* of a
/// second architecture: every item was said to need a new variant in `kayfabe-arch` and a
/// new `match` arm in `kayfabe-gsp`, both logic crates. Task #121 built the seam instead,
/// and three of the four are now served by [`Gh100FspBoot`] — this generation's own
/// [`BootSequence`], living in this crate, at this crate's offsets, with **zero** edits to
/// the falcon/secure-booter one. The events did not go away; what went away is their being
/// unhookable.
///
/// ⚠ The fourth entry is still genuinely unmodelled, and it is left in the list saying so.
/// A list that only records solved problems is a trophy cabinet.
///
/// Each tuple is `(event, evidence and current disposition)`.
pub const ARCH_LOCAL_BOOT_EVENTS: &[(&str, &str)] = &[
    (
        "FSP secure-boot command",
        "`kgspBootstrap_GH100` -> `kfspSendBootCommands_HAL` streams a Chain-of-Trust \
         payload through `NV_PFSP_EMEMD` and rings `NV_PFSP_QUEUE_HEAD` \
         (ogkm-580: hopper/gh100/dev_fsp_pri.h:26,40,53). There is no falcon STARTCPU to \
         observe: the boot request is a MESSAGE. SERVED: `Gh100FspBoot::on_write` \
         decodes the EMEM window and turns the head write into StartProcessor + \
         FirmwareLoaded + PublishBootArgs.",
    ),
    (
        "GSP target-mask release",
        "`kfspWaitForGspTargetMaskReleased_HAL`, polled after the boot command \
         (ogkm-580: kern_fsp_gh100.c:1096-1130) by reading NV_PGSP_FALCON_HWCFG2 and \
         rejecting zero and the 0xBADF41xx priv-error pattern. SERVED: `Gh100GspModel` \
         answers a non-zero HWCFG2 at every stage.",
    ),
    (
        "lockdown release / FMC error",
        "`gpuTimeoutCondWait(_kgspLockdownReleasedOrFmcError)` \
         (ogkm-580: kernel_gsp_gh100.c:489-535) reads RISCV_BR_PRIV_LOCKDOWN off the SAME \
         HWCFG2 the falcon regime reads RISCV_ENABLE off, and treats a NON-ZERO \
         NV_PFALCON_FALCON_MAILBOX0 as a boot ERROR — the OPPOSITE polarity to the falcon \
         regime, where MAILBOX0 carries the boot-args low half. SERVED: `GspObservation` \
         now carries the stage, so one model answers both bits, and this model's MAILBOX0 \
         reads 0.",
    ),
    (
        "Confidential-Compute WPR2 suppression",
        "STILL UNMODELLED. `kgspIsWpr2Up_GH100` returns NV_FALSE unconditionally under CC \
         (ogkm-580: kernel_gsp_gh100.c:220-236). `GspObservation` has no CC flag, so the \
         model cannot vary its WPR2 answer on it. This one is NOT a sequencing question — \
         it is a missing observable — and the boot-sequence seam does not address it.",
    ),
];

// ══════════════════════════════════════════════════════════════════════════════════════
// The GH100 boot SEQUENCE — added ALONGSIDE the falcon/secure-booter one (task #121)
// ══════════════════════════════════════════════════════════════════════════════════════

/// The **FSP command-queue** boot regime.
///
/// ★★★ **This type is the measurement.** It is a second architecture's boot ordering,
/// written as an implementation of [`BootSequence`] rather than as edits to the first
/// one's, and adding it changed **zero lines** of `kayfabe_gsp::seq` (the falcon/
/// secure-booter regime), of `kayfabe_device::ga10x` or of [`Ad10xGspModel`]. Before the
/// seam existed the same work meant new `GspReg` variants in `kayfabe-arch` and new
/// `match` arms in `kayfabe-gsp` — both logic crates, and both editing the ordering that
/// GA10x boots on today.
///
/// ## What the driver does, and therefore what this answers
///
/// `kgspBootstrap_GH100` (`ogkm-580:
/// src/nvidia/src/kernel/gpu/gsp/arch/hopper/kernel_gsp_gh100.c:833-935`) does **not**
/// run FWSEC and does **not** run a SEC2 Booter Load. It sends a Chain-of-Trust command
/// to the FSP and then waits:
///
/// 1. the COT payload is streamed DWORD by DWORD through `NV_PFSP_EMEMD`, at a cursor
///    held in `NV_PFSP_EMEMC` (`ogkm-580: kern_fsp_gh100.c:600-645`);
/// 2. `NV_PFSP_QUEUE_TAIL` then `NV_PFSP_QUEUE_HEAD` are written — **head last, because
///    it is the write that interrupts FSP** (`ogkm-580: kern_fsp_gh100.c:86-99`, whose
///    comment says exactly that);
/// 3. FSP boots the GSP-FMC, which raises FRTS and starts GSP-RM. There is no second
///    "load the firmware" register poke: one command does both;
/// 4. RM polls `NV_PGSP_FALCON_HWCFG2` for target-mask release
///    (`_kfspIsGspTargetMaskReleased`, `ogkm-580: kern_fsp_gh100.c:1096-1114`) and then
///    for `RISCV_BR_PRIV_LOCKDOWN_UNLOCK` (`_kgspLockdownReleasedOrFmcError`,
///    `ogkm-580: kernel_gsp_gh100.c:489-535`);
/// 5. the LibOS boot-args address is **inside the COT payload**
///    (`gspBootArgsSysmemOffset`), not in a mailbox pair: `kgspProgramLibosBootArgsAddr`
///    is called only from the Turing and Ampere bootstrap functions
///    (`ogkm-580: kernel_gsp_tu102.c:533, 933`, `kernel_gsp_ga102.c:162`), none of which
///    this chip runs.
///
/// So this generation reaches [`BootStep::StartProcessor`],
/// [`BootStep::FirmwareLoaded`] and [`BootStep::PublishBootArgs`] from **one** register
/// write, where the falcon regime needs four. That is precisely the difference a
/// `match GspReg` in the FSM could not express.
///
/// ## ⊘ What this is NOT
///
/// A Hopper port. Nothing here has touched Hopper silicon, and the honest limits are:
///
/// - **No FSP reply.** `kfspSendAndReadMessage` waits for a response packet in the MSGQ;
///   this model serves an always-drained queue (`head == tail == 0`, which is what the
///   driver's own comment says FSP does after each packet,
///   `ogkm-580: kern_fsp_gh100.c:241-256`) and never posts one.
/// - **No multi-packet reassembly.** The COT payload is 868 bytes including headers and
///   the RM channel is 1024, so the boot command is single-packet by construction
///   (`ogkm-580: kern_fsp.c:485-491`); a longer NVDM message would be split and this
///   model would read the wrong offset. It refuses instead of guessing: a window that
///   never reached [`COT_BOOT_ARGS_OFF`] yields **no** publish step.
/// - **No `AINCR`.** Read auto-increment is not modelled, because
///   [`BootSequence::on_read`] takes `&self` and the read path here exists only so a
///   driver can read back what it wrote.
/// - **No teardown.** There is no Booter Unload to observe on this regime, so nothing
///   emits [`BootStep::Teardown`]. That is an absence in the *sequence*, which is now a
///   thing an implementation is allowed to have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gh100FspBoot;

impl Gh100FspBoot {
    /// The regime.
    #[must_use]
    pub fn new() -> Gh100FspBoot {
        Gh100FspBoot
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

/// See [`BootSequence::stages`] — **three** stages where the falcon regime declares five,
/// and the difference is the whole point.
const GH100_STAGES: &[BootStageDesc] = &[
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

impl BootSequence for Gh100FspBoot {
    fn stages(&self) -> &'static [BootStageDesc] {
        GH100_STAGES
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
        // ── this generation's own registers, which no `GspReg` variant names ──
        #[allow(clippy::cast_possible_truncation)]
        match off {
            PFSP_EMEMC => {
                let (cursor, aincw) = Gh100FspBoot::decode_ememc(val);
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
            // Written *before* the head, and on its own it means nothing: the driver's
            // own comment is that the head write is the one that interrupts FSP.
            PFSP_QUEUE_TAIL => return BootSteps::none(),
            PFSP_QUEUE_HEAD => {
                // The interrupting write. Our FSP consumes the packet synchronously.
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
            _ => {}
        }
        // ── registers this generation shares with the falcon regime ──
        let Some(reg) = w.reg else {
            return BootSteps::none();
        };
        match reg {
            // Reached on every RPC send through `kgspSetCmdQueueHead_TU102`, which is
            // this chip's binding too — see [`QUEUE_HEAD0`].
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
            // so both queues always read drained — which is exactly the predicate
            // `kfspCanSendPacket_GH100` gates the next send on.
            PFSP_QUEUE_HEAD | PFSP_QUEUE_TAIL | PFSP_MSGQ_HEAD | PFSP_MSGQ_TAIL => Some(0),
            PFSP_EMEMC => Some(Gh100FspBoot::encode_ememc(
                state.latch(LATCH_EMEM_CURSOR),
                state.latch(LATCH_EMEM_AINCW) != 0,
            )),
            #[allow(clippy::cast_possible_truncation)]
            PFSP_EMEMD => Some(u64::from(
                state
                    .window_read_u32(state.latch(LATCH_EMEM_CURSOR) as usize)
                    .unwrap_or(0),
            )),
            _ => None,
        }
    }
}

kayfabe_util::assert_send_sync!(Gh100FspBoot);

/// The GH100 (Hopper) GSP register model — see the module docs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gh100GspModel {
    boot: Gh100FspBoot,
}

impl Gh100GspModel {
    /// The model.
    #[must_use]
    pub fn new() -> Gh100GspModel {
        Gh100GspModel {
            boot: Gh100FspBoot::new(),
        }
    }

    /// Where this model puts a register. **`None` is the interesting answer**: it means
    /// this generation has no such register, and four variants return it.
    #[must_use]
    pub fn at(reg: GspReg) -> Option<(u8, u64)> {
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
            GspReg::Wpr2AddrLo => WPR2_ADDR_LO,
            GspReg::Wpr2AddrHi => WPR2_ADDR_HI,
            // ★ CORRECTED: present after all, because the Turing HAL is this chip's HAL
            // for `kgspSetCmdQueueHead`. See [`QUEUE_HEAD0`].
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

impl GspModel for Gh100GspModel {
    fn decode_reg(&self, bar: u8, off: u64) -> Option<GspReg> {
        if bar != 0 {
            return None;
        }
        Some(match off {
            GSP_RISCV_CPUCTL => GspReg::GspRiscvCpuctl,
            WPR2_ADDR_LO => GspReg::Wpr2AddrLo,
            WPR2_ADDR_HI => GspReg::Wpr2AddrHi,
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
            // ★★ **One register, three readers, and they read different bits at
            // different stages** — which is why `GspObservation` had to grow a `stage`.
            // `_kfspIsGspTargetMaskReleased` requires the whole word to be non-zero and
            // not the `0xBADF41xx` priv-error pattern
            // (`ogkm-580: kern_fsp_gh100.c:1096-1114`); `_kgspLockdownReleasedOrFmcError`
            // requires `RISCV_BR_PRIV_LOCKDOWN_UNLOCK`
            // (`ogkm-580: kernel_gsp_gh100.c:530-534`); and the falcon regime reads
            // `RISCV_ENABLE` off the same offset. We answer all three: the word is always
            // non-zero, and lockdown reads LOCKED until the FSP command has booted the
            // FMC.
            GspReg::GspFalconHwcfg2 => {
                if obs.stage.wpr2_up() {
                    HWCFG2_RISCV_ENABLE
                } else {
                    HWCFG2_RISCV_ENABLE | HWCFG2_BR_PRIV_LOCKDOWN
                }
            }
            GspReg::GspFalconDmatrfcmd => DMATRFCMD_IDLE,
            // ★ Same register, opposite meaning to the falcon regime — see
            // [`ARCH_LOCAL_BOOT_EVENTS`]. Echoing the boot-args low half here is what the
            // Turing model does; on this generation a non-zero MAILBOX0 is how the
            // GSP-FMC reports a boot ERROR (`ogkm-580: kernel_gsp_gh100.c:552,562`).
            // The model cannot serve both meanings from one `GspObservation` field, so
            // it serves the one this generation's boot poll requires and the boot-args
            // echo is LOST.
            GspReg::GspFalconMailbox0 => 0,
            GspReg::GspFalconMailbox1 => u64::from(obs.boot_args_hi),
            GspReg::GspFalconIrqstat => {
                if obs.swgen0_pending {
                    IRQSTAT_SWGEN0
                } else {
                    0
                }
            }
            GspReg::GspFalconIrqmask | GspReg::GspFalconIrqdest => IRQSTAT_SWGEN0,
            GspReg::GspFalconIrqsclr => 0,
            GspReg::GspRiscvCpuctl => {
                if obs.riscv_active {
                    RISCV_CPUCTL_ACTIVE
                } else {
                    0
                }
            }
            GspReg::Wpr2AddrLo => {
                if obs.wpr2_up {
                    WPR2_LO_UP
                } else {
                    0
                }
            }
            GspReg::Wpr2AddrHi => {
                if obs.wpr2_up {
                    WPR2_HI_UP
                } else {
                    0
                }
            }
            // Write-only from our side, exactly as on the falcon regime.
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

    /// ★★★ **The second boot ordering in the tree.** Not the falcon/secure-booter one,
    /// not a variation on it, and not reachable by defaulting: see [`Gh100FspBoot`].
    fn boot_sequence(&self) -> &dyn BootSequence {
        &self.boot
    }

    fn libos_region_layout(&self) -> LibosRegionLayout {
        // The LibOS region descriptor is a driver-side structure, not a chip register,
        // and `libos_init_args.h` is architecture-independent — so this half genuinely
        // is the same on both generations.
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

/// An [`Arch`] that is `MockArch` in every respect except that its GSP is
/// [`Gh100GspModel`]. See the module docs — this is a fixture, not a port.
#[derive(Debug)]
pub struct Gh100Arch {
    inner: MockArch,
    gsp: Gh100GspModel,
}

impl Default for Gh100Arch {
    fn default() -> Gh100Arch {
        Gh100Arch::new()
    }
}

impl Gh100Arch {
    /// The architecture.
    #[must_use]
    pub fn new() -> Gh100Arch {
        Gh100Arch {
            inner: MockArch::new(),
            gsp: Gh100GspModel::new(),
        }
    }
}

impl Arch for Gh100Arch {
    fn name(&self) -> &'static str {
        "GH100 (fixture)"
    }
    fn classify(&self, class: ClassId) -> ObjectKind {
        self.inner.classify(class)
    }
    fn vchid_from_userd_flags(&self, flags: u32) -> VChid {
        self.inner.vchid_from_userd_flags(flags)
    }
    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget> {
        self.inner.decode_doorbell(token)
    }
    fn mmu(&self) -> &dyn GmmuFmt {
        self.inner.mmu()
    }
    fn userd(&self) -> &dyn UserdModel {
        self.inner.userd()
    }
    fn is_case2_control(&self, cmd: ControlCmd) -> bool {
        self.inner.is_case2_control(cmd)
    }
    fn pushbuffer(&self) -> &dyn PushbufferAbi {
        self.inner.pushbuffer()
    }
    fn gsp(&self) -> Option<&dyn GspModel> {
        Some(&self.gsp)
    }
}

kayfabe_util::assert_send_sync!(Gh100GspModel);
