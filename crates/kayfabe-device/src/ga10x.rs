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

use kayfabe_abi::vbios::VbiosWire;
use kayfabe_arch::gsp::{GspModel, GspObservation, GspReg, LibosRegionLayout};

use crate::{BootReg, ChipProfile, PtimerRegs, RomWindow};

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
/// *input* to [`WPR2_LO_UP`] below, and whichever plane ends up serving it
/// (`mode2_gsp_port_plan.md` §11-O1) must use [`FB_SIZE_MB`] and not a second literal —
/// the C artifact keeps exactly one (`C: src/qemu/nvkvm_gpu_emul.c:1546` answers this
/// address with `NVKVM_FB_SIZE_MB`, the same constant its WPR2 values were sized from).
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

/// The usable framebuffer this emulated GA10x advertises, in MiB.
///
/// ★ **A chip/board parameter, and the only free variable in the WPR2 layout.** 12 GiB
/// matches the C artifact's `NVKVM_FB_SIZE_MB` (`C: src/qemu/mode2_regs_ga10x.h:62`),
/// which is the RTX 3060 the oracle ran on.
pub const FB_SIZE_MB: u64 = 12288;

/// `DRF_SIZE(NV_PRAMIN)` — the VGA workspace the driver reserves at the top of FB.
/// `NV_PRAMIN` is `0x007FFFFF:0x00700000`
/// (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_ram.h:26`), so 1 MiB.
const PRAMIN_SIZE: u64 = 0x0010_0000;
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

/// `gspFwWprEnd` — the top of the WPR2 region, as `kgspPopulateWprMeta_TU102` computes it
/// (`ogkm-580: kernel_gsp_tu102.c:761, 776`). The WPR end margin is zero unless a regkey
/// sets it (`kgspGetWprEndMargin_IMPL`, `ogkm-580: kernel_gsp.c:5637`), and no MMU lock is
/// advertised, so `vbiosReservedOffset` is the VGA workspace offset.
const fn gsp_fw_wpr_end() -> u64 {
    let fb_size = FB_SIZE_MB << 20;
    let vga_workspace_offset = fb_size - PRAMIN_SIZE;
    // NV_ALIGN_DOWN64(x, WPR_ALIGNMENT)
    vga_workspace_offset & !(WPR_ALIGNMENT - 1)
}

/// `frtsOffset` (`ogkm-580: kernel_gsp_tu102.c:779`) — the address the driver expects to
/// read back out of `NV_PFB_PRI_MMU_WPR2_ADDR_LO`.
const fn frts_offset() -> u64 {
    gsp_fw_wpr_end() - FRTS_SIZE
}

/// Pack a byte address into the `_VAL` field of a WPR2 address register.
const fn wpr2_reg(addr: u64) -> u64 {
    (addr >> WPR2_ADDR_ALIGNMENT) << WPR2_VAL_SHIFT
}

/// `NV_PFB_PRI_MMU_WPR2_ADDR_LO` once FWSEC has run. **Exact-compared** at G6.3b.
const WPR2_LO_UP: u64 = wpr2_reg(frts_offset());
/// `NV_PFB_PRI_MMU_WPR2_ADDR_HI` once FWSEC has run. Only `!= 0` is read
/// (`kgspIsWpr2Up_TU102`, `ogkm-580: kernel_gsp_tu102.c:1251-1261`), but it is derived
/// anyway so the pair describes one region rather than two independent literals.
const WPR2_HI_UP: u64 = wpr2_reg(gsp_fw_wpr_end());

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
/// A value with no fields: everything it knows is a compile-time constant of the
/// generation, and there is nothing to configure. Constructed by [`Ga10xGspModel::new`]
/// so it stays extensible without churning callers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ga10xGspModel;

impl Ga10xGspModel {
    /// The model.
    #[must_use]
    pub fn new() -> Ga10xGspModel {
        Ga10xGspModel
    }

    /// Where this model puts a register, so a harness can address one without knowing the
    /// encoding. `None` for a register with no offset on this generation.
    #[must_use]
    pub fn at(reg: GspReg) -> Option<(u8, u64)> {
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

impl GspModel for Ga10xGspModel {
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
            GspReg::GspFalconIrqmask | GspReg::GspFalconIrqdest => IRQSTAT_SWGEN0,
            // Write-1-to-clear: reads back zero.
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
            // The Booter's argument register is write-only from our side: the guest
            // latches its own value and never reads ours back as state.
            GspReg::Sec2FalconMailbox0 => 0,
            GspReg::GspQueueHead(_) => 0,
        })
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

kayfabe_util::assert_send_sync!(Ga10xGspModel);

// ── the non-GSP chip facts ────────────────────────────────────────────────────────
//
// ★★ These are NOT `GspModel` registers and must not become any. `kayfabe_arch::gsp`'s
// rule is that a register whose served value is a function of the GSP boot FSM's state
// belongs behind that seam and every other register does not — and a chip-identity
// register is a function of nothing at all. Modelled as data so the rule stays checkable
// by inspection: anything below that needed state would have to move.

/// `NV_PMC_BOOT_0` (`C: src/qemu/mode2_regs_ga10x.h:13`, from `ogkm`'s `dev_boot`).
const PMC_BOOT_0: u64 = 0x0000_0000;
/// `NV_PMC_BOOT_1`. Read back **zero**: `VGPU = REAL`, i.e. this device advertises no
/// virtualization of its own (`C: src/qemu/nvkvm_gpu_emul.c:1503`).
const PMC_BOOT_1: u64 = 0x0000_0004;
/// `NV_PMC_BOOT_42` (`C: mode2_regs_ga10x.h:15`).
const PMC_BOOT_42: u64 = 0x0000_0A00;

/// `NV_PTIMER_TIME_PRIV_LEVEL_MASK` (`C: src/qemu/mode2_regs_ga10x.h:44`).
const PTIMER_TIME_PRIV_LEVEL_MASK: u64 = 0x0000_9430;
/// Every privilege level allowed to write the counter.
///
/// ★ `tmrSetCurrentTime_GV100` tests `_WRITE_PROTECTION_LEVEL0_ENABLE` in this register and,
/// when it does not hold, takes an arm that prints `ERROR: Write to PTIMER attempted even
/// though Level 0 PLM is disabled.` and trips `NV_ASSERT(0)`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/timer/arch/volta/timer_gv100.c:56-82`). Answering
/// zero is not a stop — the caller's status is discarded at
/// `ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4107` — but it is two lines of
/// assert noise in every boot log, and an assert nobody expects to fire is an assert nobody
/// reads. All-ones is the C artifact's answer
/// (`C: src/qemu/nvkvm_gpu_emul.c:1531`), and it makes the driver take the arm that writes
/// the counter instead. Those writes land on no source and are counted as unclaimed, which
/// is the honest outcome: the counter this device serves is free-running and cannot be set.
const PTIMER_PLM_ALL_LEVELS: u32 = 0xFFFF_FFFF;

/// `NV_VIRTUAL_FUNCTION_TIME_0` — the free-running nanosecond counter's low half, at
/// `NV_VIRTUAL_FUNCTION`'s base `0x00B8_0000` plus `0x3_0080`
/// (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_vm.h:28, 224`;
/// `C: src/qemu/mode2_regs_ga10x.h:39`).
const VIRTUAL_FUNCTION_TIME_0: u64 = 0x00BB_0080;
/// `NV_VIRTUAL_FUNCTION_TIME_1` — its high half
/// (`ogkm-580: .../tu102/dev_vm.h:226`; `C: mode2_regs_ga10x.h:40`).
const VIRTUAL_FUNCTION_TIME_1: u64 = 0x00BB_0084;

/// `NV_PMC_BOOT_0` for GA106 stepping A1: `ARCHITECTURE_0[28:24] = 0x17` (the GA100
/// family), `IMPLEMENTATION[23:20] = 6`, `MAJOR_REVISION[7:4]`/`MINOR_REVISION[3:0]` =
/// `0xA1` (`C: src/qemu/nvkvm_gpu_emul.c:64-72, 95`).
const PMC_BOOT_0_GA106_A1: u32 = 0x1760_00A1;
/// `NV_PMC_BOOT_42` for the same part: `ARCHITECTURE[29:24] = 0x17`,
/// `IMPLEMENTATION[23:20] = 6`, `MAJOR_REVISION[19:16] = 0xA`, `MINOR_REVISION[15:12] = 1`
/// ⇒ `CHIP_ID[29:20] = 0x176` (`C: nvkvm_gpu_emul.c:70-72, 96`).
const PMC_BOOT_42_GA106_A1: u32 = 0x176A_1000;

/// The registers that are constants of this silicon.
static GA106_BOOT_REGS: &[BootReg] = &[
    BootReg {
        off: PMC_BOOT_0,
        value: PMC_BOOT_0_GA106_A1,
        name: "NV_PMC_BOOT_0",
    },
    BootReg {
        off: PMC_BOOT_1,
        value: 0,
        name: "NV_PMC_BOOT_1",
    },
    BootReg {
        off: PMC_BOOT_42,
        value: PMC_BOOT_42_GA106_A1,
        name: "NV_PMC_BOOT_42",
    },
    BootReg {
        off: PTIMER_TIME_PRIV_LEVEL_MASK,
        value: PTIMER_PLM_ALL_LEVELS,
        name: "NV_PTIMER_TIME_PRIV_LEVEL_MASK",
    },
    // ★★★ The devinit constant [`USABLE_FB_SIZE_IN_MB_ADDR`]'s own comment predicted would
    // need a home: *"whichever plane ends up serving it must use `FB_SIZE_MB` and not a
    // second literal"*. This is that plane, and this is that constant — the WPR2 values the
    // GSP model serves are derived from the same one, so the two cannot desynchronise.
    //
    // It is a `BootReg` and not a `GspReg` for the reason stated there: its served value is
    // a function of no boot state. `assert_disjoint` is what makes that split safe rather
    // than merely intended.
    BootReg {
        off: USABLE_FB_SIZE_IN_MB_ADDR,
        value: FB_SIZE_MB as u32,
        name: "NV_USABLE_FB_SIZE_IN_MB",
    },
];

/// ★★★ Where this generation's driver reads the free-running nanosecond counter.
///
/// `tmrReadTimeLoReg_TU102` / `tmrReadTimeHiReg_TU102` read it through the
/// virtual-function aperture unconditionally — on a virtual function *and* on the physical
/// one (`ogkm-580: src/nvidia/src/kernel/gpu/timer/arch/turing/timer_tu102.c:130-155`) — so
/// these are the offsets a bare-metal driver bound to this device actually touches, and
/// `NV_PTIMER_TIME_0/_1` in the `0x9400` block are not.
static GA106_PTIMER: PtimerRegs = PtimerRegs {
    lo_off: VIRTUAL_FUNCTION_TIME_0,
    hi_off: VIRTUAL_FUNCTION_TIME_1,
};

/// `NV_PROM_DATA(i) = 0x00300000 + i`
/// (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_ext_devices.h:27`;
/// `C: src/qemu/mode2_regs_ga10x.h:23-24`). The driver reads the VBIOS through this window
/// a byte or a dword at a time and caps its interest at 1 MiB.
const PROM_DATA_BASE: u64 = 0x0030_0000;
/// See [`PROM_DATA_BASE`].
const PROM_DATA_SIZE: u64 = 0x0010_0000;

/// The register aperture's size — 16 MiB, as a real GA10x reports
/// (`C: src/qemu/nvkvm_gpu_emul.c:97`).
const REGS_APERTURE_LEN: u64 = 16 << 20;

/// How many message-signalled vectors the emulated device offers. The C artifact's
/// number, and its reasoning: *"room for PMC top-level + per-engine"*
/// (`C: src/qemu/nvkvm_gpu_emul.c:127`).
const MSIX_VECTORS: u16 = 8;

/// ★ **The GA106 row.** Everything above, selected.
///
/// The PCI identity is deliberately *incomplete* here: the vendor id and class code are
/// read from this device id's [`kayfabe_abi::vbios::VbiosProfile`] instead, so the ROM this
/// device serves and the identity it claims cannot disagree. See [`crate::identity_for`].
pub static GA106: ChipProfile = ChipProfile {
    name: "GA106",
    // RTX 3060 LHR — the part the C artifact emulated and the identity the bench's
    // `x-nvidia-identity` experiment claimed (`C: src/qemu/nvkvm_gpu_emul.c:91`).
    pci_device_id: 0x2504,
    pci_revision: 0xA1,
    // MSI, matching the host subsystem id the C's dumped ROM's PCIR block carried
    // (`C: nvkvm_gpu_emul.c:92-93`).
    pci_subsystem_vendor_id: 0x1462,
    pci_subsystem_id: 0x397D,
    regs_aperture_len: REGS_APERTURE_LEN,
    boot_regs: GA106_BOOT_REGS,
    ptimer: GA106_PTIMER,
    rom_window: RomWindow {
        base: PROM_DATA_BASE,
        len: PROM_DATA_SIZE,
    },
    vbios_wire: VbiosWire::Tu102Bit,
    msix_vectors: MSIX_VECTORS,
    gsp_model: || Box::new(Ga10xGspModel::new()),
};
