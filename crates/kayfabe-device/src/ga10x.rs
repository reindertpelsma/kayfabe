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

use kayfabe_abi::bifstatic::BifStaticRow;
use kayfabe_abi::chipinfo::{ChipInfoRow, RegBaseRow, reg_base};
use kayfabe_abi::confcompute::ConfComputeRow;
use kayfabe_abi::deviceinfo::{DeviceInfoRow, DevicePriBase, EnginePriBase};
use kayfabe_abi::falconinfo::FalconInventoryRow;
use kayfabe_abi::fifochannels::FifoChannelsRow;
use kayfabe_abi::gmmustatic::GmmuStaticRow;
use kayfabe_abi::gspstaticinfo::FbRegion;
use kayfabe_abi::inittables::{FifoDeviceEntry, INTR_CATEGORY_COUNT, IntrTableEntry};
use kayfabe_abi::memsysconfig::{ComptagAllocationPolicy, MemorySystemRow, RAM_TYPE_GDDR6};
use kayfabe_abi::pcibars::PciBarRow;
use kayfabe_abi::regaccessmap::RegisterAccessMapRow;
use kayfabe_abi::vbios::VbiosWire;
use kayfabe_arch::gsp::{BootSequence, GspModel, GspObservation, GspReg, LibosRegionLayout};
use kayfabe_gsp::FalconSecureBooterBoot;

use crate::{BootReg, ChipProfile, PtimerRegs, RegSpan, RomWindow};

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
/// The same range's **base**, i.e. where the window is reached through the register
/// aperture rather than how much framebuffer it reserves. Two uses of one constant: the
/// low bound of `NV_PRAMIN` (`dev_ram.h:26`) and the window offset the C artifact decodes
/// (`C: src/qemu/mode2_regs_ga10x.h:80-81`).
const PRAMIN_BASE: u64 = 0x0070_0000;
/// ★★★ `NV_PBUS_BAR0_WINDOW` — the register that **positions** the window above
/// (`ogkm-580: src/common/inc/swref/published/maxwell/gm107/dev_bus.h:43`, and the same
/// offset the C artifact decodes at `C: src/qemu/mode2_regs_ga10x.h`'s
/// `NVKVM_BAR0_WINDOW`).
///
/// ⊘ The two constants are one mechanism and the chip row carries both, because a row with
/// only the aperture is refused at realize — see
/// [`crate::ChipError::WindowWithoutItsRegister`] for the boot that motivates it.
const BAR0_WINDOW_REG: u64 = 0x0000_1700;
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
/// Its one field is the **boot sequence it selects** (task #121). A model is a per-GPU
/// *value*, not a compile-time choice, and holding the sequence as a field rather than
/// returning a process-wide singleton is what keeps it that way: two `GpuId`s can hold
/// two models selecting two different sequences without anything in this crate changing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ga10xGspModel {
    boot: FalconSecureBooterBoot,
}

impl Ga10xGspModel {
    /// The model.
    #[must_use]
    pub fn new() -> Ga10xGspModel {
        Ga10xGspModel {
            boot: FalconSecureBooterBoot::new(),
        }
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

// ---------------------------------------------------------------------------------------
// The two init tables the guest's RM cannot start without.
// ---------------------------------------------------------------------------------------

/// ★★★ **The engines this device advertises — and the ones it deliberately does not.**
///
/// Every value here is a byte the C artifact put on the wire to a stock 580.159.04 guest
/// during the hermetic cold boot, read back out of the committed capture
/// (`nvidia-gpu-passthrough/traces/mode2_c_reference/cap1b_coldboot_hermetic_d6.rec.zst`,
/// the single `cmd=0x20801112` reply at record 142049, `paramsSize=3212`,
/// `numEntries=10`). They are not synthesised and they are not guessed: that reply is in
/// turn a replay of a real RTX 3060's own answer, captured through the same control
/// (`C: src/qemu/mode2_devinfo_ga106.h`).
///
/// ## ★★★ What was left out, and why leaving it out is the honest answer
///
/// The C advertised **ten** engines. This table carries **six**. An engine in this list is
/// an engine the guest's RM will go on to *use*, and a use we cannot serve fails later,
/// deeper, and attributed to the wrong thing. So the four engines this port has no plane
/// for are absent rather than present-and-hollow:
///
/// | omitted | why |
/// |---|---|
/// | `SEC2` | a security falcon; nothing in this port drives one, and we *are* the GSP, so the kernel does not need it to boot |
/// | `NVENC0` | video encode — no encode path exists here |
/// | `NVDEC0` | video decode — likewise |
/// | `OFA` | optical flow — likewise |
///
/// `CE4` is absent for a different and older reason: the C's own wire table drops it,
/// because the host's `GET_ENGINES_V2` exposes only `CE0..CE3`. (The C's *other*, unused
/// captured blob — `ctl_20801112` in `src/qemu/mode2_initctrl_ga106.h` — has eleven
/// entries including `CE4`; the ten-entry table is the one that reached a guest.)
///
/// What remains is exactly the north-star path: `GR0` for compute, `CE0..CE3` for the data
/// plane, and `SOFTWARE`, which is not an engine at all — it is RM's own pseudo-engine, is
/// the one entry with `IS_HOST_DRIVEN_ENGINE = 0`, and is the entry
/// `kfifoGetHostDeviceInfoTable_KERNEL` special-cases when it reserves PBDMA fault ids
/// (`ogkm-580: kernel_fifo.c:2160-2166`).
///
/// ## ⚠ Two fields here are uninitialised on real silicon, and are reproduced anyway
///
/// `engineData[7]` (`ENGINE_INFO_TYPE_INTR`) holds capture noise on several rows, and the
/// `SOFTWARE` row is noise from `RESET` onward — the same three magic words recur across
/// unrelated entries. That is what a real GA106's GSP answered and what a real driver
/// accepted, so it is reproduced verbatim rather than tidied: zeroing a field the oracle
/// did not zero is a redesign with no measurement behind it, and this port's rule is to
/// reproduce the C and subtract only its *named* bugs.
pub static GA106_ENGINES: &[FifoDeviceEntry] = &[
    FifoDeviceEntry {
        name: "GR0",
        engine_data: [
            0xd334df00, // [ 0] ENG_DESC
            0x00000000, // [ 1] FIFO_TAG
            0x00000001, // [ 2] RM_ENGINE_TYPE
            0x00000000, // [ 3] RUNLIST
            0x00000040, // [ 4] MMU_FAULT_ID
            0x00000033, // [ 5] RC_MASK
            0x0000000c, // [ 6] RESET
            0x00000000, // [ 7] INTR
            0x00000054, // [ 8] MC
            0x00000000, // [ 9] DEV_TYPE_ENUM
            0x00000000, // [10] INSTANCE_ID
            0x00c00000, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000000, // [13] RUNLIST_ENGINE_ID
            0x00c20000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000000, 0x00000001],
        pbdma_fault_ids: [0x00000020, 0x00000021],
        num_pbdmas: 2,
    },
    FifoDeviceEntry {
        name: "CE0",
        engine_data: [
            0x793ceb00, // [ 0] ENG_DESC
            0x00000001, // [ 1] FIFO_TAG
            0x00000009, // [ 2] RM_ENGINE_TYPE
            0x00000000, // [ 3] RUNLIST
            0x0000000f, // [ 4] MMU_FAULT_ID
            0x00000017, // [ 5] RC_MASK
            0x00000002, // [ 6] RESET
            0x00000000, // [ 7] INTR
            0x0000000f, // [ 8] MC
            0x00000013, // [ 9] DEV_TYPE_ENUM
            0x00000000, // [10] INSTANCE_ID
            0x00c00000, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000001, // [13] RUNLIST_ENGINE_ID
            0x00c20000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000000, 0x00000001],
        pbdma_fault_ids: [0x00000020, 0x00000021],
        num_pbdmas: 1,
    },
    FifoDeviceEntry {
        name: "CE1",
        engine_data: [
            0x793ceb01, // [ 0] ENG_DESC
            0x00000002, // [ 1] FIFO_TAG
            0x0000000a, // [ 2] RM_ENGINE_TYPE
            0x00000000, // [ 3] RUNLIST
            0x00000010, // [ 4] MMU_FAULT_ID
            0x00000018, // [ 5] RC_MASK
            0x00000003, // [ 6] RESET
            0x82300100, // [ 7] INTR
            0x00000010, // [ 8] MC
            0x00000013, // [ 9] DEV_TYPE_ENUM
            0x00000001, // [10] INSTANCE_ID
            0x00c00000, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000002, // [13] RUNLIST_ENGINE_ID
            0x00c20000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000001, 0x00000001],
        pbdma_fault_ids: [0x00000020, 0x00000021],
        num_pbdmas: 1,
    },
    FifoDeviceEntry {
        name: "CE2",
        engine_data: [
            0x793ceb02, // [ 0] ENG_DESC
            0x00000003, // [ 1] FIFO_TAG
            0x0000000b, // [ 2] RM_ENGINE_TYPE
            0x00000001, // [ 3] RUNLIST
            0x00000011, // [ 4] MMU_FAULT_ID
            0x00000019, // [ 5] RC_MASK
            0x00000004, // [ 6] RESET
            0x77f2058f, // [ 7] INTR
            0x00000011, // [ 8] MC
            0x00000013, // [ 9] DEV_TYPE_ENUM
            0x00000002, // [10] INSTANCE_ID
            0x00c00400, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000000, // [13] RUNLIST_ENGINE_ID
            0x00c22000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000005, 0x00000001],
        pbdma_fault_ids: [0x00000022, 0x00000021],
        num_pbdmas: 1,
    },
    FifoDeviceEntry {
        name: "CE3",
        engine_data: [
            0x793ceb03, // [ 0] ENG_DESC
            0x00000004, // [ 1] FIFO_TAG
            0x0000000c, // [ 2] RM_ENGINE_TYPE
            0x00000002, // [ 3] RUNLIST
            0x00000012, // [ 4] MMU_FAULT_ID
            0x0000001a, // [ 5] RC_MASK
            0x00000005, // [ 6] RESET
            0x018e0102, // [ 7] INTR
            0x00000012, // [ 8] MC
            0x00000013, // [ 9] DEV_TYPE_ENUM
            0x00000003, // [10] INSTANCE_ID
            0x00c00800, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000000, // [13] RUNLIST_ENGINE_ID
            0x00c24000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000006, 0x00000001],
        pbdma_fault_ids: [0x00000023, 0x00000021],
        num_pbdmas: 1,
    },
    FifoDeviceEntry {
        name: "SOFTWARE",
        engine_data: [
            0x95a6f500, // [ 0] ENG_DESC
            0xffffffff, // [ 1] FIFO_TAG
            0x0000002d, // [ 2] RM_ENGINE_TYPE
            0x00000007, // [ 3] RUNLIST
            0xffffffff, // [ 4] MMU_FAULT_ID
            0x00000000, // [ 5] RC_MASK
            0x82300810, // [ 6] RESET
            0x77f2058f, // [ 7] INTR
            0x018e000f, // [ 8] MC
            0x00000000, // [ 9] DEV_TYPE_ENUM
            0x82300100, // [10] INSTANCE_ID
            0x77f2058f, // [11] RUNLIST_PRI_BASE
            0x00000000, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000000, // [13] RUNLIST_ENGINE_ID
            0x82300710, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000008, 0x00000001],
        pbdma_fault_ids: [0x82300100, 0x77f2058f],
        num_pbdmas: 1,
    },
];

/// The kernel interrupt table, reproduced verbatim from the same capture (the single
/// `cmd=0x20800a5c` reply at record 142056, `paramsSize=2112`, `tableLen=24`).
///
/// ## ★★ Why this one is NOT trimmed to match [`GA106_ENGINES`]
///
/// It looks like the engine table and is not one. Its key is `MC_ENGINE_IDX_*`, RM's own
/// interrupt index space, and an entry is a statement about *where a vector would arrive*,
/// not a claim that an engine exists — so the "advertise only what we can serve" rule that
/// pruned four engines above has nothing to prune here. This port delivers **no**
/// interrupts at all, so no row is a promise; and rows 14-23 carry only non-stalling
/// vectors, which is the GSP-offload split showing through (stall vectors are the GSP's).
///
/// Trimming also looks like the riskier move, on a **reading** rather than an observation:
/// the C's comment on this exact reply states that the `UVM_OWNED` subtree mask must equal
/// the access-counter vector's subtree or `intrCacheIntrFields_TU102` asserts
/// (`C: src/qemu/nvkvm_gpu_emul.c:3122-3146`). ⚠ That assert has **not been observed to
/// fire** from this port — the claim is the C author's, recorded as theirs.
///
/// ★ `vectorStall = 0x9b` on `engineIdx 50` (`MC_ENGINE_IDX_GSP`) is the vector this
/// device would raise for its own message-queue doorbell.
pub static GA106_INTR_TABLE: &[IntrTableEntry] = &[
    IntrTableEntry {
        engine_idx: 59,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x00000040,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 62,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x00000083,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 60,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x00000048,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 73,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x00000081,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 156,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 157,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 158,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 159,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 160,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 161,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 162,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 163,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 50,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x0000009b,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 2,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x0000009a,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 84,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000000,
    },
    IntrTableEntry {
        engine_idx: 47,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000002,
    },
    IntrTableEntry {
        engine_idx: 38,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000001,
    },
    IntrTableEntry {
        engine_idx: 65,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000003,
    },
    IntrTableEntry {
        engine_idx: 15,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 16,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 17,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000007,
    },
    IntrTableEntry {
        engine_idx: 18,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000008,
    },
    IntrTableEntry {
        engine_idx: 19,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x0000000a,
    },
    IntrTableEntry {
        engine_idx: 81,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000009,
    },
];

/// `subtreeMap[NV2080_INTR_CATEGORY_*]`, indexed `DEFAULT, ESCHED_DRIVEN_ENGINE,
/// ESCHED_DRIVEN_ENGINE_NOTIFICATION, RUNLIST, RUNLIST_NOTIFICATION, UVM_OWNED,
/// UVM_SHARED`.
///
/// ★ Two oracles agree on these seven words, which is worth recording because they are the
/// part of the reply that was *not* captured from silicon: the C synthesises them in code,
/// and the bytes it put on the wire match the values in its own captured
/// `ctl_20800a5c` blob exactly. `UVM_OWNED = 0x2` is the load-bearing one.
pub static GA106_INTR_SUBTREE_MAP: [u64; INTR_CATEGORY_COUNT] = [0x0, 0x8, 0x1, 0x0, 0x0, 0x2, 0x4];

// ---------------------------------------------------------------------------------------
// The FB region table — the memory this device says it has, and can be asked for.
// ---------------------------------------------------------------------------------------

/// The framebuffer this device advertises, in bytes. [`FB_SIZE_MB`], once.
pub const GA106_FB_LENGTH: u64 = FB_SIZE_MB << 20;

/// The contiguous carve-out at the **top** of FB that a GA10x GSP keeps for itself.
///
/// ★ Read off the oracle's own answer, not chosen: in
/// `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` record 141977 — the 1792-byte
/// `GspStaticConfigInfo` an RTX 3060's GSP posted — regions 2, 3 and 4 are contiguous,
/// each carries `reserved == its own size`, and together they span
/// `[0x2_EFBE_0000, 0x2_FFFF_FFFF]` of a 12 GiB board. That is the top `0x1042_0000`
/// bytes, and it is the number below. (Regions 0 and 1 of that capture are the low
/// reserved area and the usable heap; see [`GA106_FB_REGIONS`] for why only one of the
/// two survives here.)
///
/// ⚠ It is deliberately **larger** than the WPR2 + FRTS + VGA-workspace range this device
/// publishes in `NV_PFB_PRI_MMU_WPR2_ADDR_{LO,HI}`. Over-reserving costs heap; under-
/// reserving hands the guest memory its own firmware sits in. The const assertion below
/// pins the direction.
const FW_CARVE_OUT_BYTES: u64 = 0x1042_0000;

/// Where [`FW_CARVE_OUT_BYTES`] starts, given the FB this device advertises.
const FW_CARVE_OUT_BASE: u64 = GA106_FB_LENGTH - FW_CARVE_OUT_BYTES;

// ★★ The two publications must not drift: `frts_offset()` is what the guest's FWSEC check
// exact-compares against `NV_PFB_PRI_MMU_WPR2_ADDR_LO`, so the byte it names is firmware
// territory and must fall inside the reserved region. A build error here means the FB size
// and the WPR2 layout have been changed independently of each other.
const _: () = assert!(FW_CARVE_OUT_BASE < frts_offset());
const _: () = assert!(gsp_fw_wpr_end() < GA106_FB_LENGTH);

/// ★★★ **The FB regions this device serves, and what backs each.**
///
/// Two, both derived from [`FB_SIZE_MB`]:
///
/// | # | range | `reserved` | what backs it |
/// |---|---|---|---|
/// | 0 | `0 .. FW_CARVE_OUT_BASE-1` | 0 — **usable** | the framebuffer this device already advertises at `NV_USABLE_FB_SIZE_IN_MB` ([`USABLE_FB_SIZE_IN_MB_ADDR`]); one constant, and the guest is told it twice |
/// | 1 | `FW_CARVE_OUT_BASE .. top` | its own size — **reserved** | the WPR2/FRTS/VGA-workspace layout this device publishes in `NV_PFB_PRI_MMU_WPR2_ADDR_{LO,HI}`, derived from [`gsp_fw_wpr_end`] and [`frts_offset`] and const-asserted to lie inside |
///
/// ## ⊘ What was omitted from the oracle's five, and why
///
/// | oracle region | disposition |
/// |---|---|
/// | 0: `[0, 0x0310_FFFF]`, 49 MiB, reserved | **dropped.** It backs the real GSP-RM's own low allocations. This GSP has none — nothing of ours lives below the carve-out — so reserving it would be 49 MiB withheld to imitate a firmware that is not running. |
/// | 1: `[0x0311_0000, 0x2_EFBD_FFFF]`, usable | **kept, and widened** to start at 0, absorbing the dropped region 0. |
/// | 2, 3, 4: `[0x2_EFBE_0000, 0x2_FFFF_FFFF]`, all reserved | **merged into one row.** The split is the real GSP's internal heap/BAR2-page-directory/WPR bookkeeping, and RM treats all three identically — `reserved != 0` is the only bit it reads (`ogkm-580: mem_mgr_gsp_client.c:96-100`). Three rows describing one property is three chances to disagree. |
///
/// ## ⚠ What this table promises that is not yet built
///
/// Region 0 says every byte below the carve-out can be allocated. The data plane does not
/// back guest FB yet, so what is really being stated is the *address space* the guest may
/// place allocations in — which is exactly what the register plane already claims. The
/// first allocation that must resolve to real memory is a later rung, and it will read
/// this table rather than restate it.
pub static GA106_FB_REGIONS: &[FbRegion] = &[
    FbRegion {
        base: 0,
        limit: FW_CARVE_OUT_BASE - 1,
        reserved: 0,
        // The oracle's usable region reports 6, compressed and ISO both allowed. There is
        // one region to compare against, so the number is only ever equal to itself.
        performance: 6,
        support_compressed: true,
        support_iso: true,
        protected: false,
    },
    FbRegion {
        base: FW_CARVE_OUT_BASE,
        limit: GA106_FB_LENGTH - 1,
        // `reserved` is the region's own size: RM makes the whole region reserved on any
        // non-zero value, and stating the size is what makes `reservedMemSize` add up.
        reserved: FW_CARVE_OUT_BYTES,
        performance: 0,
        support_compressed: false,
        support_iso: false,
        protected: false,
    },
];

// ---------------------------------------------------------------------------------------
// The base-address registers — what this device presents, and therefore what it may say.
// ---------------------------------------------------------------------------------------

/// The framebuffer window's length. **PCI slots 1+2** — a 64-bit prefetchable window.
///
/// ★ 256 MiB is a real RTX 3060's BAR1 with resizable-BAR off, it is what the bench boots
/// with (`-device nvkvm-gpu,bar1-size=268435456`), and it is what the oracle's own answer
/// carries (`C: src/qemu/mode2_initctrl_ga106.h:5397-5399`, `barSize = 256`,
/// `barSizeBytes = 0x1000_0000`). `identity_for` hands it to the shell, which refuses to
/// realize if its registration disagrees.
const FB_WINDOW_LEN: u64 = 256 << 20;

/// The instance / RM-`BAR2` GMMU window's length. **PCI slots 3+4**, also 64-bit.
///
/// 32 MiB, which is `BUS_BAR2_APERTURE_MB` (`ogkm-580: g_kern_bus_nvoc.h:178`), the bench's
/// `bar2-size=33554432`, and the oracle's `barSize = 32` / `barSizeBytes = 0x200_0000`.
const INST_WINDOW_LEN: u64 = 32 << 20;

/// ★★ **The four base-address registers this device presents, in RM's index order.**
///
/// `pciBarCount = 4` is not a choice: `kbusInitPciBars_GM107` sets `totalPciBars =
/// BUS_NUM_BARS` for every non-AMODEL part (`ogkm-580: kern_bus_gm107.c:4715-4718`), and
/// the physical side sends exactly that (`kern_bus_ctrl.c:640`). The oracle's captured
/// reply agrees — `pciBarCount = 4` with row 3 all zeros.
///
/// ⊘ Row 3 is the **I/O BAR, and it is absent**: `size_bytes = 0`. A GA106 has none, this
/// device registers none, and RM's own encoding for an absent BAR is a zeroed entry. It is
/// listed rather than truncated because `pciBarCount` is what RM loops to and 4 is the
/// count a classic dGPU reports; a three-row table would be a different, smaller claim.
///
/// ⚠ PCI slot 5 carries the message-signalled interrupt table, which is the hypervisor's
/// own region and not one of RM's four logical BARs. It has no row here on purpose.
pub static GA106_PCI_BARS: &[PciBarRow] = &[
    PciBarRow {
        name: "registers",
        size_bytes: REGS_APERTURE_LEN,
    },
    PciBarRow {
        name: "framebuffer-window",
        size_bytes: FB_WINDOW_LEN,
    },
    PciBarRow {
        name: "instance-window",
        size_bytes: INST_WINDOW_LEN,
    },
    PciBarRow {
        name: "io",
        size_bytes: 0,
    },
];

// The table must state the count RM's own physical side would. A row added or dropped
// without meaning to change that claim is a build error.
const _: () = assert!(GA106_PCI_BARS.len() == kayfabe_abi::pcibars::bus_bar::NUM);

// ── The chip-info reply's per-chip half (task #132) ────────────────────────────────

/// Where `NV_VIRTUAL_FUNCTION`'s registers sit in *this* device's base-address register 0.
///
/// `GPU_GET_VREG_OFFSET(pGpu, a) = pGpu->sriovState.virtualRegPhysOffset + a`
/// (`ogkm-580: src/nvidia/generated/g_gpu_access_nvoc.h:269`), where the offset is
/// `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET) = 0x00B8_0000` on a physical function
/// and zero only on an SR-IOV virtual one (`ogkm-580:
/// src/nvidia/src/kernel/gpu/arch/turing/kern_gpu_tu102.c:92-101`), and
/// `DRF_BASE(NV_VIRTUAL_FUNCTION) = 0x0003_0000`
/// (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_vm.h:27-28`).
/// `0x00B8_0000 + 0x0003_0000 = 0x00BB_0000` — which is also, exactly, the value the
/// oracle's RTX 3060 reported in `regBases[NV_REG_BASE_USERMODE]`.
///
/// ⚠ This device is **not** an SR-IOV virtual function and does not claim to be, so the
/// physical-function arm is the right one. See `docs/design/mode2_vgpu_posture_decision.md`
/// in the C tree for the posture that decision belongs to.
const VIRTUAL_FUNCTION_BASE: u32 = 0x00BB_0000;

/// `DRF_SIZE(NVC361)` — how much of the aperture `kfifoConstructUsermodeMemdescs_GV100`
/// asks for (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/volta/kernel_fifo_gv100.c:175`).
/// `NVC361` is `0x0081ffff:0x00810000` (`ogkm-580:
/// src/common/sdk/nvidia/inc/class/clc361.h:29`), i.e. 64 KiB.
const VIRTUAL_FUNCTION_LEN: u32 = 0x1_0000;

// ★★ The advertisement and the thing it advertises, tied together at compile time. The
// register base below is a promise that this window is one this device answers; the only
// offset inside it that any source claims is the free-running counter, so if the counter
// ever moved out of the window the promise would become a lie silently. It cannot now.
const _: () = assert!(VIRTUAL_FUNCTION_TIME_0 >= VIRTUAL_FUNCTION_BASE as u64);
const _: () =
    assert!(VIRTUAL_FUNCTION_TIME_1 < (VIRTUAL_FUNCTION_BASE as u64 + VIRTUAL_FUNCTION_LEN as u64));

/// ★★★ **The register groups this device names, and the three it deliberately does not.**
///
/// `regBases[]` has sixteen slots and RM defines four
/// (`ogkm-580: src/nvidia/generated/g_gpu_nvoc.h:5828-5833`). This table has **one row**,
/// and the emptiness is the decision: an index with no row is encoded as
/// [`kayfabe_abi::chipinfo::REG_BASE_UNSUPPORTED`], which `gpuGetRegBaseOffset_FWCLIENT`
/// turns straight back into `NV_ERR_NOT_SUPPORTED` at whichever call site wanted it
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_gspclient.c:315-326`).
///
/// ⊘ The oracle's board named all four. It had silicon behind every one of them; this
/// device has a register plane that decodes a listed set of offsets and answers **zero**
/// everywhere else (`crate::plane`), and a zero read out of a register group we advertised
/// is exactly the *plausible wrong answer* that plane's own docs argue is worse than a
/// refusal. So the oracle settles the layout and the encoding, and this table is decided
/// on what we can serve.
///
/// ### Served — `NV_REG_BASE_USERMODE` at [`VIRTUAL_FUNCTION_BASE`]
///
/// ★ Not optional: `kfifoStateInit` calls `kfifoConstructUsermodeMemdescs_HAL` under
/// `NV_ASSERT_OK_OR_RETURN` (`ogkm-580: kernel_fifo_init.c:232`), which asks for this base
/// under `NV_ASSERT_OK_OR_RETURN` again (`ogkm-580: kernel_fifo_gv100.c:369`). A device
/// that declined it could not finish `RmInitAdapter`. And it is a window this device really
/// answers — the free-running nanosecond counter [`GA106_PTIMER`] serves lives inside it,
/// as the `const _` assertions above check.
///
/// ### Not named — and what each omission actually costs
///
/// - **`NV_REG_BASE_GR` (1).** `[measured]` grep of `ogkm-580`: the identifier occurs
///   **once**, at its own `#define`. Nothing reads it, so naming it would advertise a
///   4 MiB window to nobody.
/// - **`NV_REG_BASE_TIMER` (2).** Its readers hand `0x9000` to a client that then maps the
///   timer block and reads `NV_PTIMER_TIME_0/_1` out of it
///   (`ogkm-580: src/nvidia/src/kernel/gpu/timer/timer.c:1712-1734`,
///   `subdevice_ctrl_timer_kernel.c:287`). ⊘ This device does **not** serve those: see
///   [`GA106_PTIMER`], whose whole point is that this generation's driver reads the counter
///   through the virtual-function aperture instead, and whose doc says the `0x9400` block
///   is not served. Advertising the group would hand a client a **stopped clock** —
///   `crate::plane`'s named worst case, arrived at from the other direction.
/// - **`NV_REG_BASE_MASTER` (3).** Its one reader maps `GF100_SUBDEVICE_MASTER` read-only
///   for a client (`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/generic_engine.c:118-135`).
///   Three registers of that block are in [`GA106_BOOT_REGS`] and the rest read zero, so
///   the same argument applies with less force — it is left out because *partly* serving a
///   client mapping is not serving it, and because nothing on the initialisation path asks.
///
/// ⚠ `[inferred]`, not measured: all three arguments are greps of the two **open** trees. A
/// closed client (NVML, `nvidia-smi`) can allocate `NV01_TIMER` or `GF100_SUBDEVICE_MASTER`
/// and this port cannot grep it. If one is ever observed to, the refusal surfaces *as a
/// named `NV_ERR_NOT_SUPPORTED` at that client's own call*, and the fix is a row here plus
/// the register model behind it — in that order.
static GA106_REG_BASES: &[RegBaseRow] = &[RegBaseRow {
    index: reg_base::USERMODE,
    offset: VIRTUAL_FUNCTION_BASE,
    name: "NV_VIRTUAL_FUNCTION (usermode work submission)",
}];

/// The two silicon facts the chip-info reply carries that the PCI identity does not.
///
/// ★ `chip_sub_rev = 0` and `is_cmp_sku = false` are the values the oracle's RTX 3060's own
/// physical RM reported for **this exact part** (`C: mode2_initctrl_ga106.h:3356`, bytes 0
/// and 8), which is the one thing a capture of the silicon we claim to be can adjudicate.
/// Neither is derivable from a register this port serves: `gpuGetChipSubRev_HAL` and
/// `gpuGetIsCmpSku_HAL` have no implementation in the open tree at all — only the
/// `_FWCLIENT` readers that consume this reply.
///
/// ⊘ Neither is branched on. `chipSubRev` reaches exactly one place,
/// `NV2080_CTRL_CMD_MC_GET_ARCH_INFO`'s `subRevision`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mc/kernel_mc.c:66`); `isCmpSku` reaches exactly
/// one, `NV2080_CTRL_GPU_INFO_INDEX_CMP_SKU`
/// (`ogkm-580: subdevice_ctrl_gpu_kernel.c:491-501`). Both report to a client and decide
/// nothing — which is why taking them from the capture is cheap to be wrong about, and is
/// stated here rather than assumed.
pub static GA106_CHIP_INFO: ChipInfoRow = ChipInfoRow {
    chip_sub_rev: 0,
    is_cmp_sku: false,
    reg_bases: GA106_REG_BASES,
};

/// ★★★ **This device publishes no user register access map — and that is the policy, not a
/// gap.**
///
/// The reply is a bitmap with one bit per 32-bit register of BAR0, and a set bit means
/// *unprivileged guest userspace may `NV2080_CTRL_CMD_GPU_EXEC_REG_OPS` this offset*
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_access.c:1632-1638`, the map's only reader).
/// [`crate::plane`] decodes a listed set of offsets and answers **zero** everywhere else, so
/// a set bit over an offset this device does not model is a plausible wrong answer handed to
/// a client with no refusal anywhere.
///
/// ⊘ **The oracle's board published one and it is not usable here.** Its reply carried
/// `userRegisterAccessMapSize = 0x80000` and a 1269-byte gzip stream that inflates to
/// 524 288 bytes permitting **6809 ranges, 37 743 registers**
/// (`C: src/qemu/mode2_initctrl_ga106.h:3363`, reconstructed from
/// `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` records 142000-142002 — see
/// [`kayfabe_abi::regaccessmap`]). Among them is `0x9400`, the PTIMER block whose *whole
/// point* in [`GA106_PTIMER`] is that this device does not serve it and that
/// [`GA106_REG_BASES`] therefore refuses `NV_REG_BASE_TIMER` rather than hand a client a
/// stopped clock. Replaying the oracle's map would make that same promise 6809 times, one
/// layer lower and with no refusal vocabulary at all.
///
/// ★★ What [`RegisterAccessMapRow::NOT_PUBLISHED`] gets instead is **RM's own** answer:
/// `userRegisterAccessMapSize == 0` takes the `LEVEL_INFO` *"User Register Access Map
/// unsupported for this chip"* arm and returns `NV_OK`
/// (`ogkm-580: gpu_register_access_map.c:261-267`), leaving `pUserRegisterAccessMap` `NULL`
/// so that `gpuGetUserRegisterAccessPermissions_IMPL` answers `NV_FALSE` to everything
/// (`:141-152`). It advertises nothing and it fails **closed**, which is the direction a
/// permission answer should default when the thing it grants access to is not built.
///
/// ⊘ The cost is stated in [`kayfabe_abi::regaccessmap`]'s docs rather than discovered:
/// `bRmProfilingPrivileged` is left `NV_FALSE` (no reader on this port's path — its one
/// reader is inside `if (IS_VIRTUAL || hypervisorIsVgxHyper())`), and the guest's
/// framebuffer reservation estimate is 512 KiB smaller.
///
/// ★ Publishing one later is a *bitmap this port authors from the offsets [`crate::plane`]
/// really decodes*, not a copy of the capture — which is why the row carries the compressed
/// bytes rather than a boolean.
pub static GA106_USER_REGISTER_ACCESS_MAP: RegisterAccessMapRow =
    RegisterAccessMapRow::NOT_PUBLISHED;

/// ★★★ **This device constructs no generic kernel falcons — and that overrules the
/// oracle.**
///
/// The C artifact replays a captured GA106 reply naming **eight**: `OBJFBFLCN` at
/// `0x9a4000`, `FECS` at `0x409000`, `GPCCS` at `0x41a000`, `OBJBSP` (NVDEC0) at
/// `0x848000`, `Pmu` at `0x10a000`, `OBJMSENC` (NVENC0) at `0x1c8000`, `OBJSEC2` at
/// `0x840000` and `OBJOFA` at `0x844000` (`C: src/qemu/mode2_initctrl_ga106.h:3878`;
/// `[measured]` byte-identical to record 142009 of
/// `traces/cap1b_coldboot_hermetic_d6.rec`).
///
/// ★★ **An entry is an instruction to construct, not a fact to report.** Each one RM
/// accepts becomes a `GenericKernelFalcon` whose every register access is
/// `registerBase + offset` handed straight to BAR0 MMIO
/// (`ogkm-580: kernel_falcon_tu102.c:45-76`), and which is registered as an `IntrService`
/// (`ogkm-580: gpu.c:5559-5568`) so those reads are reachable from the interrupt path.
///
/// `[measured]` by `tests/constructed_falcon_info.rs`: **seven of the eight bases have no
/// model at all**, and the eighth — `NV_PSEC = 0x840000` — decodes exactly three offsets
/// (`+0x040` mailbox0, `+0x100` cpuctl, `+0x118` dmatrfcmd), the FWSEC secure-booter
/// handshake this module scripts. `_HWCFG2`, `_IRQSTAT`, `_IRQMASK` and `_IRQDEST` — what
/// an `IntrService`-registered falcon reads first — are modelled nowhere.
/// [`crate::plane::RegPlane`] answers an unclaimed register with a defaulted **zero**,
/// which its own docs call *"a wrong value rather than a fabricated mapping"*. Advertising
/// these eight would hand RM eight microcontrollers reporting an all-zero state.
///
/// ⚠ That three-register exception is a correction, not a footnote: the first draft of this
/// row claimed *no* base was modelled and its own test went red on `NV_PSEC`. The true
/// statement is *"none is modelled as a falcon"*. And RM skips the `ENG_SEC2` row outright
/// wherever a `KernelSec2` exists (`ogkm-580: gpu.c:5384-5390`), so even the oracle's list
/// is conditional.
///
/// ★ What it costs is deferred and **named at the right place**, not silent: `[inferred]`
/// from `ogkm-580: channel_descendant.c:221-234`, a guest allocating a
/// SEC2/NVDEC/NVENC/NVJPG/OFA engine-class object gets `NV_ERR_INVALID_CLASS` and
/// *"engine is missing for class 0x%x"* — a refusal at the call that wanted the engine.
/// Nothing at boot requires any of them; see [`kayfabe_abi::falconinfo`].
///
/// ★★ Naming one later is an inventory this port authors from the register windows
/// [`crate::plane`] really decodes — which is why the row carries entries rather than a
/// boolean. `FECS` and `GPCCS` are the two the compute path will eventually need, and they
/// are the two whose windows are emptiest today.
pub static GA106_CONSTRUCTED_FALCONS: FalconInventoryRow = FalconInventoryRow::NONE;

/// ★★★ **What this device says about its memory system — and the one row where
/// *reproducing the oracle* is the honest answer rather than the lazy one.**
///
/// Every value is a real RTX 3060's own reply, captured from the host's GSP and replayed by
/// the C artifact (`C: src/qemu/mode2_initctrl_ga106.h:5391`, `ctl_20800a1c`, registered at
/// `:6255` as `{0x20800a1cu, 0x0u, 40u, 40u, …}` — forty bytes captured for a forty-byte
/// struct, nothing trimmed). [`kayfabe_abi::memsysconfig`]'s docs decode them byte by byte
/// and show the four independent internal agreements that make the layout trustworthy.
///
/// ★★ **Contrast with [`GA106_CONSTRUCTED_FALCONS`] and [`GA106_USER_REGISTER_ACCESS_MAP`],
/// which overrule the oracle.** Those two replies are *instructions*: an entry tells RM to
/// construct a microcontroller, a set bit tells RM a register is safe to hand to
/// unprivileged userspace — and this port cannot back either, so it declines. This reply is
/// a *description* of memory the guest is going to be given anyway: the framebuffer
/// [`GA106_FB_REGIONS`] already promises, the `PRAMIN` window [`crate::plane`] already
/// decodes, the L2 the guest never addresses directly. Reproducing the silicon's own
/// numbers is not a promise this port cannot keep; inventing rounder ones would be.
///
/// ⊘ Two of the fields are nonetheless *ours* and not the oracle's, in the sense that they
/// are checked against what this device serves rather than copied blind:
/// `fbpa_present = true` is the only value
/// [`kayfabe_abi::memsysconfig::MemorySystemError::FbpaAbsent`] permits, because the other
/// one would re-place RM's BAR0 window off `l2_cache_size` instead of the fixed `PRAMIN`
/// span in [`GA106`]; and `comprPageShift` is not here at all, because the encoder derives
/// it from `compr_page_size` so the pair cannot drift.
///
/// ★ `ecc_fbpa_enabled = false` is the oracle's value and the one this port can support:
/// an RTX 3060 has no ECC, and the heap's blacklist walk
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/heap.c:3920,3996`) reads the field to
/// decide whether to go looking for retired pages. This device retires none.
pub static GA106_MEMORY_SYSTEM: MemorySystemRow = MemorySystemRow {
    // The oracle's byte 2, and the mode in which hardware owns the comptaglines rather than
    // RM — which is the only one this port could back, since it models no comptag allocator.
    comptag_policy: ComptagAllocationPolicy::Raw,
    disable_compbit_backing: false,
    disable_post_l2_compression: false,
    ecc_fbpa_enabled: false,
    l2_prefill: false,
    // 0x24_0000 = 2.25 MiB = 24 slices x 96 KiB, which is `ltc_count * lts_per_ltc_count`
    // two fields down. The capture agrees with itself.
    l2_cache_size: 0x0024_0000,
    fbpa_present: true,
    // 64 KiB; the encoder derives `comprPageShift = 16` from it.
    compr_page_size: 0x0001_0000,
    ram_type: RAM_TYPE_GDDR6,
    ltc_count: 6,
    lts_per_ltc_count: 4,
};

/// ★★★ **Where each advertised engine's PRI block begins — and the whole of what this chip
/// has to state for the DEVICE_INFO2 reply.**
///
/// Six statements for the six rows of [`GA106_ENGINES`], because
/// [`kayfabe_abi::deviceinfo`] refuses an engine it is told nothing about. Everything else
/// in that reply — `faultId`, `instanceId`, `typeEnum`, `resetId`, `isEngine`, `rlEngId`,
/// `runlistPriBase` — is projected out of [`GA106_ENGINES`] and is deliberately **not**
/// restated here.
///
/// `[measured]`, and the run is the **C artifact's rather than this port's**: every base
/// below is read out of its captured `ctl_20800a40`
/// (`C: src/qemu/mode2_initctrl_ga106.h:4028`, `nvidia-gpu-passthrough` rev `018e492`),
/// itself a replay of a real RTX 3060's own GSP reply.
/// `tests/internal_device_info_table.rs` re-checks the whole 24580-byte projection against
/// those bytes, so a base changed here goes red against the capture rather than silently.
///
/// ⚠ **The four copy engines share one base, and that is the capture's own answer, not a
/// copy-paste.** `0x104000` is the `NV_PCE` block; `CE0..CE3` are logical copy engines
/// carved out of it, which is why `instanceId` and not `devicePriBase` is what tells them
/// apart on the wire. A reader who "fixed" this into four spaced bases would be inventing
/// silicon.
///
/// ★ `SOFTWARE` is [`DevicePriBase::NotADevice`]: RM's own pseudo-engine, the one row of
/// [`GA106_ENGINES`] with `IS_HOST_DRIVEN_ENGINE = 0`, absent from the oracle's DEVICE_INFO2
/// table, and — being the row that carries the capture noise `0x77f2058f` — the row this
/// port most needs kept off a field RM files as a register base. The exclusion is that
/// marking, never a name check; see [`kayfabe_abi::deviceinfo`] for the three independent
/// guards behind it.
pub static GA106_DEVICE_INFO: DeviceInfoRow = DeviceInfoRow {
    pri_bases: &[
        EnginePriBase {
            engine: "GR0",
            pri_base: DevicePriBase::At(0x0040_0000),
        },
        EnginePriBase {
            engine: "CE0",
            pri_base: DevicePriBase::At(0x0010_4000),
        },
        EnginePriBase {
            engine: "CE1",
            pri_base: DevicePriBase::At(0x0010_4000),
        },
        EnginePriBase {
            engine: "CE2",
            pri_base: DevicePriBase::At(0x0010_4000),
        },
        EnginePriBase {
            engine: "CE3",
            pri_base: DevicePriBase::At(0x0010_4000),
        },
        EnginePriBase {
            engine: "SOFTWARE",
            pri_base: DevicePriBase::NotADevice,
        },
    ],
};

/// ★★ **What a GA106 says about its confidential-compute plane: nothing is trusted.**
///
/// `[measured]`, and the run is the **C artifact's rather than this port's**: the oracle's
/// captured reply table carries `{0x20800af3u, 0x0u, 2u, 0u, ctl_20800af3}`
/// (`C: src/qemu/mode2_initctrl_ga106.h:6216`, `nvidia-gpu-passthrough` rev `018e492`) with
/// an **empty** `ctl_20800af3[]` at `:39-40`. The capture trims trailing zeros, so a real
/// RTX 3060's own GSP answered `NV_OK` with both bits clear.
///
/// ⊘ Written down rather than defaulted, because the fail-open value and the true value are
/// the same two bytes — see [`kayfabe_abi::confcompute`]. Both bits clear is RM **refusing**
/// to map compute-protected vidmem through BAR1
/// (`ogkm-580: src/nvidia/src/kernel/rmapi/mapping_cpu.c:227-235`), and that refusal is one
/// this port wants kept: it serves no CPR aperture.
pub static GA106_CONF_COMPUTE: ConfComputeRow = ConfComputeRow {
    bar1_trusted: false,
    pcie_trusted: false,
};

/// ★★ **What a GA106 says about its bus interface: four `NvBool`s, all clear.**
///
/// `[measured]`, and the run is the **C artifact's rather than this port's**:
/// `{0x20800aacu, 0x0u, 4u, 0u, ctl_20800aac}` (`C: src/qemu/mode2_initctrl_ga106.h:6258`,
/// rev `018e492`) with an **empty** `ctl_20800aac[]` at `:5413-5414`.
///
/// ⚠ `bPcieGen4Capable = false` on a board that is Gen4-capable is the oracle's own answer,
/// carried verbatim. This port has no independent source for it, and nothing in the open 580
/// tree reads the property it sets — `[inferred]` from a grep of the open trees only. See
/// [`kayfabe_abi::bifstatic`].
///
/// The other three are statements about *this device* and not about the oracle's board:
/// there is no chip-to-chip link, one PCI function, and no power-state transition for a PMU
/// to restore configuration space across.
pub static GA106_BIF_STATIC: BifStaticRow = BifStaticRow {
    pcie_gen4_capable: false,
    c2c_link_up: false,
    device_multi_function: false,
    gcx_pmu_cfg_space_restore: false,
};

/// ★★★ **2048 channel ids per runlist** — and refusing to say so halts the boot.
///
/// `[measured]`, and the run is the **C artifact's rather than this port's**:
/// `{0x20800a61u, 0x0u, 8u, 8u, ctl_20800a61}` (`C: src/qemu/mode2_initctrl_ga106.h:6210`,
/// rev `018e492`), whose eight bytes are `runlistId = 0` then `numChannels = 0x0800`
/// (`:12-14`). A real RTX 3060's own GSP answered this control 2048.
///
/// ⊘ `[inferred]`: that every runlist has the same count. The oracle's table is keyed on the
/// command alone, so all four of `cap1b`'s asks got the same 2048 and a stock guest booted
/// through it. See [`kayfabe_abi::fifochannels`] for what a chip with differing runlists
/// would need instead.
pub static GA106_FIFO_CHANNELS: FifoChannelsRow = FifoChannelsRow {
    channels_per_runlist: 0x0800,
};

/// ★★★ **The two MMU fault buffers a GA106's GSP sizes** — and the only row in this file
/// whose absence is a **use-after-free** rather than an amputation.
///
/// `[measured]`, and the run is the **C artifact's rather than this port's**:
/// `{0x20800a59u, 0x0u, 16u, 16u, ctl_20800a59}` (`C: src/qemu/mode2_initctrl_ga106.h:6260`,
/// `nvidia-gpu-passthrough` rev `018e492`) — `psize` 16, captured `dlen` 16, nothing trimmed
/// — whose sixteen bytes are at `:5418-5419`.
///
/// ★ Both sizes are exact multiples of `NVC369_BUF_SIZE` = 32 (6 272 and 36 961 fault
/// packets), which is the arithmetic RM performs on them and therefore an independent check
/// on the field order rather than a restatement of it. See [`kayfabe_abi::gmmustatic`].
pub static GA106_GMMU_STATIC: GmmuStaticRow = GmmuStaticRow {
    replayable_size: 0x0003_1000,
    replayable_shadow_metadata_size: 0,
    non_replayable_size: 0x0012_0c20,
    non_replayable_shadow_metadata_size: 0,
};

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
    pramin_window: RegSpan {
        base: PRAMIN_BASE,
        len: PRAMIN_SIZE,
    },
    bar0_window_reg: BAR0_WINDOW_REG,
    vbios_wire: VbiosWire::Tu102Bit,
    msix_vectors: MSIX_VECTORS,
    gsp_model: || Box::new(Ga10xGspModel::new()),
    engines: GA106_ENGINES,
    intr_table: GA106_INTR_TABLE,
    intr_subtree_map: GA106_INTR_SUBTREE_MAP,
    fb_regions: GA106_FB_REGIONS,
    pci_bars: GA106_PCI_BARS,
    chip_info: GA106_CHIP_INFO,
    user_register_access_map: GA106_USER_REGISTER_ACCESS_MAP,
    constructed_falcons: GA106_CONSTRUCTED_FALCONS,
    memory_system: GA106_MEMORY_SYSTEM,
    device_info: GA106_DEVICE_INFO,
    conf_compute: GA106_CONF_COMPUTE,
    bif_static: GA106_BIF_STATIC,
    fifo_channels: GA106_FIFO_CHANNELS,
    gmmu_static: GA106_GMMU_STATIC,
    fb_length: GA106_FB_LENGTH,
};

// ── The MMU fault-code table (Axis B, task #111) ───────────────────────────────────

/// ★★ **The `NV_PFAULT_*` encoding for this generation** — the Axis-B half of the
/// simulated-fault emitter (`docs/design/simulated_gpu_fault.md` §6).
///
/// The values come from the *chip's* hardware-reference header, which is where they
/// belong and why they cannot be in a logic crate:
/// `ogkm-580: kernel-open/nvidia-uvm/hwref/ampere/ga100/dev_fault.h:224-239`
/// (`NV_PFAULT_FAULT_TYPE_*`) and `:459-472` (`NV_PFAULT_ACCESS_TYPE_*`).
///
/// ★ The header is `ampere/ga100`'s and this chip is a GA10x consumer part. That is not
/// a substitution: the driver reaches the same numbers through the same path, because
/// this generation's whole GMMU fault HAL dispatches to the Turing/Volta
/// implementations — `kgmmuGetFaultRegisterMappings` resolves to `_TU102` for
/// `TU102…GA107` alike (`ogkm-580: src/nvidia/generated/g_kern_gmmu_nvoc.c:775-786`),
/// and the fault-type decode used on this part is `kgmmuGetFaultType_GV100`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:1699-1780`).
/// The Pascal, Turing, Ampere and Blackwell copies of `dev_fault.h` all carry `PDE = 0`
/// and `PTE = 2` at the same field width.
///
/// ★★★ **The range is closed, and out-of-range is not a lint.** `kgmmuGetFaultType_GV100`
/// returns `NV_ERR_INVALID_ARGUMENT` from its `default:` arm, which fails
/// `kgmmuParseFaultPacket_GV100`'s `NV_ASSERT_OR_RETURN`
/// (`ogkm-580: kern_gmmu_gv100.c:1851-1852`) and aborts the guest's whole drain loop
/// (`:2004-2005`) — so one bad code does not merely lose its own fault, it stops the
/// guest servicing the faults behind it. `MmuFaultCause`'s vocabulary is closed for that
/// reason and this table is total over it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Ga10xFaultCodes;

impl kayfabe_arch::fault::MmuFaultCodes for Ga10xFaultCodes {
    fn fault_type(&self, cause: kayfabe_arch::fault::MmuFaultCause) -> u32 {
        use kayfabe_arch::fault::MmuFaultCause as C;
        match cause {
            // `NV_PFAULT_FAULT_TYPE_PDE` = 0x0 (`dev_fault.h:224`).
            C::NothingMapped => 0x0,
            // `NV_PFAULT_FAULT_TYPE_RO_VIOLATION` = 0x6 (`dev_fault.h:230`). The guest's
            // own decoder maps it to `fault_write`, i.e. "a write to a read-only page"
            // (`ogkm-580: kern_gmmu_gv100.c:1699-1780`).
            C::PermissionViolation => 0x6,
        }
    }

    fn access_type(&self, access: kayfabe_arch::fault::MmuFaultAccess) -> u32 {
        use kayfabe_arch::fault::MmuFaultAccess as A;
        match access {
            // `NV_PFAULT_ACCESS_TYPE_VIRT_READ` = 0x0 (`dev_fault.h:463`).
            A::Read => 0x0,
            // `NV_PFAULT_ACCESS_TYPE_VIRT_WRITE` = 0x1 (`:464`).
            A::Write => 0x1,
            // `NV_PFAULT_ACCESS_TYPE_VIRT_ATOMIC` = 0x2 (`:465`). The header gives the
            // same value a second name, `VIRT_ATOMIC_STRONG` (`:466`); the weak flavour
            // is a different code (0x4, `:468`) and this port does not distinguish them,
            // so it reports the strong one rather than choosing at random.
            A::Atomic => 0x2,
        }
    }
}
