//! Axis B, for real: the **GA10x** register model.
//!
//! `kayfabe-gsp` contains no offset, no bit position and no generation name — that is
//! CLAUDE.md rule 1 and the reason `GspModel` exists. This module is where those numbers
//! are *allowed* to live, and it is the first non-fake implementation of that seam: the
//! conformance suite drives the FSM through two deliberately-fake models, and this one
//! drives it through the chip the recorded capture was taken on.
//!
//! ## ★★ It MOVED here on 2026-07-31, and the move is the point
//!
//! This module used to be `kayfabe_crec::ga10x`, i.e. inside the crate whose job is the
//! **trace differential**. That was fine while nothing shipped: the only consumer was the
//! oracle replay. Stage Q4 wires a real guest's trapped register accesses into the same
//! FSM, and a register map reachable only from a test harness cannot serve one — while a
//! *second*, production copy of the same offsets would be two descriptions of one chip that
//! can disagree, which is the failure this repository's whole VBIOS argument is about.
//!
//! So there is one map, it lives in a crate a shipped archive can depend on, and
//! `kayfabe_crec::ga10x` re-exports it. The consequence worth stating: the 359 062-record
//! `cap1` differential now runs against **the same bytes the guest gets**.
//!
//! ## ★★ Every constant here is DERIVED, never read off the trace
//!
//! This is a differential harness. If the register model were tuned until the served
//! values matched the capture, the differential would be measuring nothing — it would be
//! a very expensive way to copy 359 062 records. So each offset and each encoding below
//! carries the source it came from, and all of them are one of:
//!
//! - `ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_gsp.h` and
//!   `.../dev_falcon_v4.h` — NVIDIA's own published register definitions, at the tag the
//!   bench runs (`ogkm_is_versioned`: the vendored 610.43.02 tree is *not* the spec);
//! - `C: src/qemu/mode2_regs_ga10x.h` — the C artifact's arch header, which is itself a
//!   transcription of those swref headers and says so;
//! - a *chip parameter* the C chose (the WPR2 geometry it advertises, the FB size), which
//!   is a value and not a protocol.
//!
//! Where the two disagree the header is cited and the disagreement is the finding. The
//! one place this model deliberately declines to answer is a register `GspReg` has no
//! variant for; `decode_reg` returns `None` there, which the FSM treats as *"another
//! model owns this offset"* and never as a defaulted zero (plan §11-O1, still open).

use kf_arch::gsp::{BootSequence, GspModel, GspObservation, GspReg, LibosRegionLayout};
use kf_gsp::FalconSecureBooterBoot;


// ── BAR0 offsets ──────────────────────────────────────────────────────────────────
// `ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_gsp.h:27,29,38`
// (`NV_PGSP_FALCON_MAILBOX0/1`, `NV_PGSP_QUEUE_HEAD(i) = 0x110c00+(i)*8`), the falcon
// register block at `.../dev_falcon_v4.h`, and `C: src/qemu/mode2_regs_ga10x.h` for the
// two bases (`NV_PGSP` = 0x110000, `NV_PSEC` = 0x840000) and the PGC6/PFB offsets.

/// GSP falcon block base.
const PGSP: u64 = 0x0011_0000;
/// SEC2 falcon block base.
const PSEC: u64 = 0x0084_0000;

/// `NV_PFALCON_FALCON_IRQSCLR` — falcon-relative (`ogkm-580: dev_falcon_v4.h`).
const FALCON_IRQSCLR: u64 = 0x004;
/// `NV_PFALCON_FALCON_IRQSTAT`.
const FALCON_IRQSTAT: u64 = 0x008;
/// `NV_PFALCON_FALCON_IRQMASK`.
const FALCON_IRQMASK: u64 = 0x018;
/// `NV_PFALCON_FALCON_IRQDEST`.
const FALCON_IRQDEST: u64 = 0x01c;
/// `NV_PFALCON_FALCON_MAILBOX0`.
const FALCON_MAILBOX0: u64 = 0x040;
/// `NV_PFALCON_FALCON_MAILBOX1`.
const FALCON_MAILBOX1: u64 = 0x044;
/// `NV_PFALCON_FALCON_HWCFG2`.
const FALCON_HWCFG2: u64 = 0x0f4;
/// `NV_PFALCON_FALCON_CPUCTL`.
const FALCON_CPUCTL: u64 = 0x100;
/// `NV_PFALCON_FALCON_DMATRFCMD`.
const FALCON_DMATRFCMD: u64 = 0x118;

/// `NV_PGSP_QUEUE_HEAD(0)`; stride 8, `__SIZE_1 = 8`
/// (`ogkm-580: dev_gsp.h:38-39`). The C hard-codes queue 0
/// (`C: src/qemu/mode2_regs_ga10x.h:69`); this model decodes all eight, because the
/// register is an array and a guest that rings queue 1 must not be answered as queue 0.
const QUEUE_HEAD0: u64 = 0x0011_0c00;
/// `NV_PGSP_QUEUE_HEAD__SIZE_1`.
const QUEUE_HEAD_COUNT: u64 = 8;

/// GSP RISC-V `CPUCTL`: RISCV base 0x111000 + 0x388
/// (`C: src/qemu/mode2_regs_ga10x.h`, `NV_PGSP_RISCV_CPUCTL`).
const GSP_RISCV_CPUCTL: u64 = 0x0011_1388;
/// ★★★★★ `NV_PRISCV_RISCV_IRQMASK`: `NV_FALCON2_GSP_BASE` (`0x0011_1000`) + `0x528`
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_riscv_pri.h:27,28`).
/// `C: src/qemu/nvkvm_gpu_emul.c:1572` answers this exact offset with `1 << 6`.
/// See [`kf_arch::gsp::GspReg::GspRiscvIrqmask`] for why a defaulted zero here
/// silently disarms the guest's entire GSP interrupt service.
const GSP_RISCV_IRQMASK: u64 = 0x0011_1528;
/// `NV_PRISCV_RISCV_IRQDEST`: `NV_FALCON2_GSP_BASE` + `0x52c`
/// (`ogkm-580: dev_riscv_pri.h:27,29`); `C: nvkvm_gpu_emul.c:1573`.
const GSP_RISCV_IRQDEST: u64 = 0x0011_152c;
/// `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK`.
const GFW_BOOT_PLM: u64 = 0x0011_8128;
/// `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT`.
const GFW_BOOT_PROGRESS: u64 = 0x0011_8234;
/// `NV_PFB_PRI_MMU_WPR2_ADDR_LO`.
const WPR2_ADDR_LO: u64 = 0x001F_A824;
/// `NV_PFB_PRI_MMU_WPR2_ADDR_HI`.
const WPR2_ADDR_HI: u64 = 0x001F_A828;

// ── encodings ─────────────────────────────────────────────────────────────────────

/// `NV_PGC6_GFW_BOOT_PROGRESS_COMPLETED`. The guest polls for this
/// (`gpuWaitForGfwBootComplete_TU102`, `ogkm-580: kern_gpu_tu102.c:381-469`).
const GFW_BOOT_COMPLETED: u64 = 0xFF;
/// The privilege-level mask the guest requires fully lowered before it trusts the
/// progress value (same citation). All levels granted.
const GFW_BOOT_PLM_LOWERED: u64 = 0xFFFF_FFFF;
/// `NV_PFALCON_FALCON_CPUCTL_STARTCPU` — bit 1. The write that starts a falcon.
const CPUCTL_STARTCPU: u64 = 0x2;
/// `NV_PFALCON_FALCON_CPUCTL_HALTED_TRUE` — bit 4
/// (`C: src/qemu/mode2_regs_ga10x.h`).
const CPUCTL_HALTED: u64 = 0x10;
/// `NV_PFALCON_FALCON_HWCFG2_RISCV_ENABLE` — bit 10 on GA10x. The reader is
/// `kflcnIsRiscvCpuEnabled_TU102`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/turing/kernel_falcon_tu102.c:130-132`).
const HWCFG2_RISCV_ENABLE: u64 = 0x400;
/// `NV_PFALCON_FALCON_DMATRFCMD` reporting `IDLE=TRUE|FULL=FALSE` — the ucode-load DMA
/// has always already finished, because there is no ucode.
const DMATRFCMD_IDLE: u64 = 0x2;
/// `NV_PRISCV_RISCV_CPUCTL_ACTIVE_STAT` — bit 7. The reader is
/// `kflcnIsRiscvActive_GA102`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/arch/ampere/kernel_falcon_ga102.c:53-55`).
const RISCV_CPUCTL_ACTIVE: u64 = 0x80;
/// `NV_PFALCON_FALCON_IRQSTAT_SWGEN0` — bit 6 (`C: nvkvm_gpu_emul.c:4193-4200`).
const IRQSTAT_SWGEN0: u64 = 1 << 6;
/// The SEC2 Booter argument that means **Unload** on GA10x: `SEC2 MAILBOX0 == 0xff`
/// (`C: nvkvm_gpu_emul.c:4222-4234`). A generation-local convention, not a protocol,
/// which is why `GspModel` asks for a predicate.
const SEC2_BOOTER_UNLOAD: u32 = 0xff;

/// `NV_USABLE_FB_SIZE_IN_MB` = `NV_PGC6_AON_SECURE_SCRATCH_GROUP_42`
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga102/dev_gc6_island_addendum.h:33`;
/// ★ Ampere+ only — the Turing headers do not publish it).
///
/// ⚠ **This model does not decode it, on purpose.** `GspModel`'s stated rule is that a
/// register belongs to the GSP plane only if its served value is a function of the boot
/// FSM's state, and this one is a devinit constant. It is recorded here because it is the
/// *input* to the WPR2 derivation below, and whichever plane serves it must answer the SAME
/// size the model was built with ([`FalconGspModel::with_fb_size_mb`]) — never a second literal.
///
/// ★ Measured 2026-07-31: the C's `cap1_coldboot_hermetic` capture contains **exactly 3**
/// reads of this address. Teaching this model to decode it moves every positional golden
/// in `tests/cap1_differential.rs` by +3, which is how the count above was established.
pub const USABLE_FB_SIZE_IN_MB_ADDR: u64 = 0x0011_83A4;

// ── the WPR2 layout, DERIVED from the advertised FB size ──────────────────────────
//
// ★★★ CORRECTED 2026-07-31. These two used to be hand-written constants documented as
// "only zero-vs-nonzero is load-bearing", citing `kgspIsWpr2Up_TU102`. That reader only
// looks at the HI register. The LO register's reader is `kgspExecuteFwsec_TU102`
// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c:514-524`)
// and it is an **exact compare** against the driver's own arithmetic:
//
//     if (wpr2LoVal != (NvU32)(pPreparedCmd->frtsOffset >> NV_PFB_PRI_MMU_WPR2_ADDR_LO_ALIGNMENT))
//         "failed to execute FWSEC for FRTS: WPR2 initialized at an unexpected location"
//
// So the value is a *function of `NV_USABLE_FB_SIZE_IN_MB`*, and writing it as a literal
// couples two registers with nothing to hold them together. It is now computed by the same
// chain the driver walks, so changing the FB size cannot desynchronise them.
// `docs/design/gsp_boot_gate_spec.md` §1 gates G5.2/G6.3b carry the full derivation.

/// `DRF_SIZE(NV_PRAMIN)` — the VGA workspace the driver reserves at the top of FB.
/// `NV_PRAMIN` is `0x007FFFFF:0x00700000`
/// (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_ram.h:26`), so 1 MiB.
const PRAMIN_SIZE: u64 = 0x0010_0000;
/// The same range's **base**, i.e. where the window is reached through the register
/// aperture rather than how much framebuffer it reserves. Two uses of one constant: the
/// low bound of `NV_PRAMIN` (`dev_ram.h:26`) and the window offset the C artifact decodes
/// (`C: src/qemu/mode2_regs_ga10x.h:80-81`).
pub const PRAMIN_BASE: u64 = 0x0070_0000;
/// ★★★ `NV_PBUS_BAR0_WINDOW` — the register that **positions** the window above
/// (`ogkm-580: src/common/inc/swref/published/maxwell/gm107/dev_bus.h:43`, and the same
/// offset the C artifact decodes at `C: src/qemu/mode2_regs_ga10x.h`'s
/// `NVKVM_BAR0_WINDOW`).
///
/// ⊘ The two constants are one mechanism and the chip row carries both, because a row with
/// only the aperture is refused at realize — see
/// [`crate::ChipError::WindowWithoutItsRegister`] for the boot that motivates it.
pub const BAR0_WINDOW_REG: u64 = 0x0000_1700;
/// `kgspGetFrtsSize_TU102` — 1 MiB on Turing through Ada
/// (`ogkm-580: .../gsp/arch/turing/kernel_gsp_frts_tu102.c:49-58`; GA100 and GB10B are 0).
const FRTS_SIZE: u64 = 0x0010_0000;
/// The 128 KiB alignment `kgspPopulateWprMeta_TU102` rounds the WPR end down to
/// (`ogkm-580: kernel_gsp_tu102.c:776`, literal `0x20000`).
const WPR_ALIGNMENT: u64 = 0x2_0000;
/// `NV_PFB_PRI_MMU_WPR2_ADDR_{LO,HI}_ALIGNMENT` — the address is `_VAL << 12`
/// (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_fb.h:36, 39`).
const WPR2_ADDR_ALIGNMENT: u32 = 0xc;
/// `NV_PFB_PRI_MMU_WPR2_ADDR_{LO,HI}_VAL` is bits `31:4` (`dev_fb.h:35, 38`).
const WPR2_VAL_SHIFT: u32 = 4;

/// ★★★★★ **THE SAME DERIVATION, AS A FUNCTION OF THE ADVERTISED SIZE — w696e.**
///
/// ★ v3 (w826): the size is ALWAYS a parameter. Constraint 15 — *"the advertised framebuffer size
/// is derived from the reservation that succeeded, never asserted ahead of it"* — and constraint
/// 12 (no per-die constants) made the old `FB_SIZE_MB = 12288` (GA106) default a defect: the
/// compile-time constant and its closures are deleted; the size is what the store holds.
#[must_use]
pub const fn gsp_fw_wpr_end_for(fb_size_mb: u64) -> u64 {
    let fb_size = fb_size_mb << 20;
    let vga_workspace_offset = fb_size - PRAMIN_SIZE;
    // NV_ALIGN_DOWN64(x, WPR_ALIGNMENT)
    vga_workspace_offset & !(WPR_ALIGNMENT - 1)
}

/// The advertised framebuffer length in bytes, as a function of the advertised size.
/// See [`gsp_fw_wpr_end_for`] for why the parameter exists.
#[must_use]
pub const fn fb_length_for(fb_size_mb: u64) -> u64 {
    fb_size_mb << 20
}

/// The same, as a function of the advertised size. See [`gsp_fw_wpr_end_for`].
#[must_use]
pub const fn frts_offset_for(fb_size_mb: u64) -> u64 {
    gsp_fw_wpr_end_for(fb_size_mb) - FRTS_SIZE
}

/// Pack a byte address into the `_VAL` field of a WPR2 address register.
/// The WPR2 register encoding of address `addr` (`_VAL` = `addr >> 12` at `31:4`).
#[must_use]
pub const fn wpr2_reg(addr: u64) -> u64 {
    (addr >> WPR2_ADDR_ALIGNMENT) << WPR2_VAL_SHIFT
}

/// The teardown sentinel `MAILBOX0` must report once fn-47 has been serviced.
///
/// ★★ **580 governs and it tests exact equality**: `_kgspIsProcessorSuspended` is
/// `return (mailbox == 0x80000000)` with the constant inlined
/// (`ogkm-580: kernel_gsp_tu102.c:1225-1239`; the symbol
/// `INTERRUPT_PROCESSOR_SUSPENDED_VALUE` does not exist at that tag). 610 masks instead
/// (`ogkm-610: :333, 348`). So this value **replaces** the mailbox shadow and is never
/// OR-ed onto it — a shadow still holding a boot-args low half with bit 31 set would read
/// as suspended at 610 and hang the teardown poll forever at 580.
const PROCESSOR_SUSPENDED: u64 = 0x8000_0000;

/// `"RMARGS"`, little-endian ASCII in an 8-byte LibOS region id
/// (`C: nvkvm_gpu_emul.c:3408`).
pub const RMARGS_ID: u64 = 0x0000_524d_4152_4753;

/// The GA10x GSP register model.
///
/// Its one field is the **boot sequence it selects** (task #121). A model is a per-GPU
/// *value*, not a compile-time choice, and holding the sequence as a field rather than
/// returning a process-wide singleton is what keeps it that way: two `GpuId`s can hold
/// two models selecting two different sequences without anything in this crate changing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FalconGspModel {
    boot: FalconSecureBooterBoot,
    /// ★★★★★ **The advertised framebuffer size this model answers WPR2 for — w696h.**
    ///
    /// ⊘ `Wpr2AddrLo`/`Wpr2AddrHi` are served by [`GspModel::encode`], not from
    /// [`ChipProfile::boot_regs`], so patching the register TABLE for a new framebuffer size
    /// would leave these two answering the compiled-in one. Two sources for one fact, silently
    /// disagreeing — and the guest would read a WPR2 range outside the framebuffer it was told
    /// it has. ⇒ the size lives here too, and [`Self::new`] keeps the shipped default so no
    /// existing caller changes.
    fb_size_mb: u64,
}

impl FalconGspModel {
    /// The model, answering WPR2 for a **given** advertised framebuffer size.
    ///
    /// ★ `[measured w696g]` a guest booted against 6144 MiB — half the shipped 12288 and the
    /// size this part can actually reserve — grades `(P)`. This is the seam that lets the
    /// operator's `vidmem` argument reach the two registers that would otherwise keep
    /// answering the compile-time constant.
    #[must_use]
    pub fn with_fb_size_mb(fb_size_mb: u64) -> FalconGspModel {
        FalconGspModel {
            boot: FalconSecureBooterBoot::new(),
            fb_size_mb,
        }
    }

    /// Where this model puts a register, so a harness can address one without knowing the
    /// encoding. `None` for a register with no offset on this generation.
    #[must_use]
    pub fn reg_at(reg: GspReg) -> Option<(u8, u64)> {
        let off = match reg {
            GspReg::GfwBootProgress => GFW_BOOT_PROGRESS,
            GspReg::GfwBootPlm => GFW_BOOT_PLM,
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
            GspReg::Sec2FalconCpuctl => PSEC + FALCON_CPUCTL,
            GspReg::Sec2FalconMailbox0 => PSEC + FALCON_MAILBOX0,
            GspReg::Sec2FalconDmatrfcmd => PSEC + FALCON_DMATRFCMD,
            GspReg::Wpr2AddrLo => WPR2_ADDR_LO,
            GspReg::Wpr2AddrHi => WPR2_ADDR_HI,
            GspReg::GspQueueHead(i) if u64::from(i) < QUEUE_HEAD_COUNT => {
                QUEUE_HEAD0 + u64::from(i) * 8
            }
            GspReg::GspQueueHead(_) => return None,
        };
        Some((0, off))
    }
}

impl GspModel for FalconGspModel {
    fn at(&self, reg: GspReg) -> Option<(u8, u64)> {
        FalconGspModel::reg_at(reg)
    }

    fn decode_reg(&self, bar: u8, off: u64) -> Option<GspReg> {
        if bar != 0 {
            return None;
        }
        if (QUEUE_HEAD0..QUEUE_HEAD0 + QUEUE_HEAD_COUNT * 8).contains(&off)
            && (off - QUEUE_HEAD0).is_multiple_of(8)
        {
            return Some(GspReg::GspQueueHead(((off - QUEUE_HEAD0) / 8) as u8));
        }
        Some(match off {
            GFW_BOOT_PROGRESS => GspReg::GfwBootProgress,
            GFW_BOOT_PLM => GspReg::GfwBootPlm,
            GSP_RISCV_CPUCTL => GspReg::GspRiscvCpuctl,
            GSP_RISCV_IRQMASK => GspReg::GspRiscvIrqmask,
            GSP_RISCV_IRQDEST => GspReg::GspRiscvIrqdest,
            WPR2_ADDR_LO => GspReg::Wpr2AddrLo,
            WPR2_ADDR_HI => GspReg::Wpr2AddrHi,
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
                (PSEC, FALCON_CPUCTL) => GspReg::Sec2FalconCpuctl,
                (PSEC, FALCON_MAILBOX0) => GspReg::Sec2FalconMailbox0,
                (PSEC, FALCON_DMATRFCMD) => GspReg::Sec2FalconDmatrfcmd,
                _ => return None,
            },
        })
    }

    fn is_startcpu(&self, value: u64) -> bool {
        value & CPUCTL_STARTCPU != 0
    }

    fn is_booter_unload(&self, sec2_mailbox0: u32) -> bool {
        sec2_mailbox0 == SEC2_BOOTER_UNLOAD
    }

    fn is_swgen0_clear(&self, value: u64) -> bool {
        value & IRQSTAT_SWGEN0 != 0
    }

    fn encode(&self, reg: GspReg, obs: &GspObservation) -> Option<u64> {
        Some(match reg {
            GspReg::GfwBootProgress => GFW_BOOT_COMPLETED,
            GspReg::GfwBootPlm => GFW_BOOT_PLM_LOWERED,
            // Both falcons are always HALTED: there is no ucode, so the core never runs
            // and the guest's `kflcnIsFalconHalted` gate is satisfied immediately.
            GspReg::GspFalconCpuctl | GspReg::Sec2FalconCpuctl => CPUCTL_HALTED,
            GspReg::GspFalconHwcfg2 => HWCFG2_RISCV_ENABLE,
            GspReg::GspFalconDmatrfcmd | GspReg::Sec2FalconDmatrfcmd => DMATRFCMD_IDLE,
            GspReg::GspFalconMailbox0 => {
                if obs.suspended {
                    // ★ REPLACE, never OR. See `PROCESSOR_SUSPENDED`.
                    PROCESSOR_SUSPENDED
                } else {
                    u64::from(obs.boot_args_lo)
                }
            }
            GspReg::GspFalconMailbox1 => u64::from(obs.boot_args_hi),
            GspReg::GspFalconIrqstat => {
                if obs.swgen0_pending {
                    IRQSTAT_SWGEN0
                } else {
                    0
                }
            }
            // ★★★★★ §16.77 — the falcon PAIR and the RISC-V PAIR both advertise SWGEN0,
            // because `kflcnGetPendingHostInterrupts` reads ONE pair or the OTHER depending
            // on a mode latched at bootstrap, and answering only the falcon pair made the
            // RISC-V AND collapse to zero. `C: nvkvm_gpu_emul.c:1570-1573` answers all four.
            GspReg::GspFalconIrqmask
            | GspReg::GspFalconIrqdest
            | GspReg::GspRiscvIrqmask
            | GspReg::GspRiscvIrqdest => IRQSTAT_SWGEN0,
            // Write-1-to-clear: reads back zero.
            GspReg::GspFalconIrqsclr => 0,
            GspReg::GspRiscvCpuctl => {
                if obs.riscv_active {
                    RISCV_CPUCTL_ACTIVE
                } else {
                    0
                }
            }
            // Derived from THIS MODEL'S size (`FalconGspModel::fb_size_mb`) — there is no other.
            GspReg::Wpr2AddrLo => {
                if obs.wpr2_up {
                    wpr2_reg(frts_offset_for(self.fb_size_mb))
                } else {
                    0
                }
            }
            GspReg::Wpr2AddrHi => {
                if obs.wpr2_up {
                    wpr2_reg(gsp_fw_wpr_end_for(self.fb_size_mb))
                } else {
                    0
                }
            }
            // The Booter's argument register is write-only from our side: the guest
            // latches its own value and never reads ours back as state.
            GspReg::Sec2FalconMailbox0 => 0,
            GspReg::GspQueueHead(_) => 0,
        })
    }

    /// ★ This generation is inside the falcon/secure-booter regime: NVIDIA's own
    /// generated HAL binds those implementations for every function the shared `GspReg`
    /// vocabulary models across `TU102…AD107`
    /// (`ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:2374-2385`). Selecting the shared
    /// implementation is one line and states the fact; inheriting it by omission would
    /// state nothing, which is why [`GspModel::boot_sequence`] has no default.
    fn boot_sequence(&self) -> &dyn BootSequence {
        &self.boot
    }

    fn libos_region_layout(&self) -> LibosRegionLayout {
        LibosRegionLayout {
            // `{ LibosAddress id8; LibosAddress pa; LibosAddress size; NvU8 kind; NvU8 loc; }`
            // = 32 bytes with alignment
            // (`ogkm-580: src/common/uproc/os/common/include/libos_init_args.h:49-56`);
            // the C's `LIBOS_REGION_STRIDE 32` agrees.
            entry_stride: 32,
            id_offset: 0,
            pa_offset: 8,
            size_offset: 16,
            // ★ GSP-D9: the LibOS init-args array's own declared maximum, 4096 entries
            // (`ogkm-580: src/common/uproc/os/common/include/libos_init_args.h:31` — the
            // `#define` on that line is the only place it is stated), where the C caps its
            // scan at 16 and stops at the first zero entry (`C:3388-3407`).
            //
            // ★★ The symbol is cited by FILE AND LINE rather than spelled out, and that is
            // the VMM-vocabulary gate working rather than being worked around. NVIDIA's
            // name for it happens to contain one of the hypervisor API identifiers that
            // gate refuses, this crate is deliberately IN that gate's scope, and the gate
            // has no allowlist by design. Being allowed to name a chip is not being allowed
            // to name a hypervisor's API; a file-and-line citation is what an unambiguous
            // reference costs here, and it is not lossy.
            //
            // ★ Worth knowing before the next person "fixes" this: the FIRST attempt to
            // explain the above tripped the same gate, because the explanation spelled the
            // token out. The rule is lexical, not editorial, and prose is in scope.
            max_entries: 4096,
            rmargs_id: RMARGS_ID,
        }
    }
}
