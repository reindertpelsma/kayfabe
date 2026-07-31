//! Axis B: the **AD10x** (Ada) register model — the *"a generation is a table row"* case.
//!
//! ## Why every constant below equals GA10x's, and why that is the measurement
//!
//! Ada's GSP boot HAL dispatches to the Turing/Ampere implementations for the whole
//! sequence: `kgspBootstrap_TU102` is the Turing→Ada regime (FWSEC, then boot-args in
//! `MAILBOX0/1`, then the SEC2 Booter Load that raises WPR2), and the register block
//! bases do not move. Concretely, at `ogkm-580`:
//!
//! - `ada/ad102/` ships **no** `dev_gsp.h` and **no** `dev_falcon_v4.h` — the Ada build
//!   consumes `ampere/ga102/dev_gsp.h` and the Turing falcon block, so
//!   `NV_PGSP_FALCON_MAILBOX0` is the same `0x110040` and `NV_PGSP_QUEUE_HEAD(i)` the
//!   same `0x110c00+(i)*8`.
//! - `NV_PRISCV_RISCV_CPUCTL` is `0x388` at `ampere/ga102/dev_riscv_pri.h:32`, over the
//!   `NV_FALCON2_GSP_BASE = 0x00111000` of `ampere/ga102/dev_falcon_second_pri.h:26`;
//!   Ada inherits both.
//! - `NV_PFB_PRI_MMU_WPR2_ADDR_LO/HI` and the `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05`
//!   GFW-boot pair are likewise the Ampere definitions.
//!
//! So for **the register set `GspReg` actually models**, AD10x and GA10x are identical,
//! and this module is a second row rather than a second implementation. The four things
//! that genuinely move across TU→GA→AD (the GSP debug-fuse address, the `IMEMC`/`DMEMC`
//! field width, `NV_PRISCV_RISCV_CPUCTL`'s move from `0x268`, and `BCR_CTRL` appearing at
//! `0x668`) are **all outside** `GspReg`'s vocabulary today, which is why they cost
//! nothing here — and that is a fact about the current seam's coverage, not a claim that
//! Ada is free in general.
//!
//! ⚠ **This is the easy member of the universe.** See the crate docs: the honest stress
//! case is [`crate::gh100`], and it is reported separately.

use kayfabe_arch::gsp::{BootSequence, GspModel, GspObservation, GspReg, LibosRegionLayout};
use kayfabe_arch::ids::{ClassId, ControlCmd, VChid};
use kayfabe_arch::{Arch, DoorbellTarget, GmmuFmt, ObjectKind, PushbufferAbi, UserdModel};
use kayfabe_gsp::FalconSecureBooterBoot;
use kayfabe_mocks::MockArch;

// ── BAR0 offsets (`ogkm-580`, via the Ampere headers Ada consumes) ────────────────

/// GSP falcon block base (`ogkm-580: ampere/ga102/dev_gsp.h:25`, `NV_PGSP`).
const PGSP: u64 = 0x0011_0000;
/// SEC2 falcon block base.
const PSEC: u64 = 0x0084_0000;

/// `NV_PFALCON_FALCON_IRQSCLR` — falcon-relative
/// (`ogkm-580: turing/tu102/dev_falcon_v4.h`).
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

/// `NV_PGSP_QUEUE_HEAD(0)`; stride 8 (`ogkm-580: ampere/ga102/dev_gsp.h:38-39`).
const QUEUE_HEAD0: u64 = 0x0011_0c00;
/// `NV_PGSP_QUEUE_HEAD__SIZE_1`.
const QUEUE_HEAD_COUNT: u64 = 8;

/// GSP RISC-V `CPUCTL`: `NV_FALCON2_GSP_BASE` `0x00111000`
/// (`ogkm-580: ampere/ga102/dev_falcon_second_pri.h:26`) + `NV_PRISCV_RISCV_CPUCTL`
/// `0x388` (`ogkm-580: ampere/ga102/dev_riscv_pri.h:32`).
const GSP_RISCV_CPUCTL: u64 = 0x0011_1388;
/// `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK`
/// (`ogkm-580: ampere/ga102/dev_gc6_island.h:27`).
const GFW_BOOT_PLM: u64 = 0x0011_8128;
/// `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT`
/// (`ogkm-580: ampere/ga102/dev_gc6_island_addendum.h:30`).
const GFW_BOOT_PROGRESS: u64 = 0x0011_8234;
/// `NV_PFB_PRI_MMU_WPR2_ADDR_LO`.
const WPR2_ADDR_LO: u64 = 0x001F_A824;
/// `NV_PFB_PRI_MMU_WPR2_ADDR_HI`.
const WPR2_ADDR_HI: u64 = 0x001F_A828;

// ── encodings ─────────────────────────────────────────────────────────────────────

/// `NV_PGC6_GFW_BOOT_PROGRESS_COMPLETED`.
const GFW_BOOT_COMPLETED: u64 = 0xFF;
/// The privilege-level mask the guest requires fully lowered. All levels granted.
const GFW_BOOT_PLM_LOWERED: u64 = 0xFFFF_FFFF;
/// `NV_PFALCON_FALCON_CPUCTL_STARTCPU` — bit 1.
const CPUCTL_STARTCPU: u64 = 0x2;
/// `NV_PFALCON_FALCON_CPUCTL_HALTED_TRUE` — bit 4.
const CPUCTL_HALTED: u64 = 0x10;
/// `NV_PFALCON_FALCON_HWCFG2_RISCV_ENABLE` — bit 10.
const HWCFG2_RISCV_ENABLE: u64 = 0x400;
/// `NV_PFALCON_FALCON_DMATRFCMD` reporting `IDLE=TRUE|FULL=FALSE`.
const DMATRFCMD_IDLE: u64 = 0x2;
/// `NV_PRISCV_RISCV_CPUCTL_ACTIVE_STAT` — bit 7.
const RISCV_CPUCTL_ACTIVE: u64 = 0x80;
/// `NV_PFALCON_FALCON_IRQSTAT_SWGEN0` — bit 6.
const IRQSTAT_SWGEN0: u64 = 1 << 6;
/// The SEC2 Booter argument that means **Unload**. A generation-local convention.
const SEC2_BOOTER_UNLOAD: u32 = 0xff;

/// ★ **INVENTED, and marked as such.** The WPR2 region this model advertises once FWSEC
/// has run. It is a *chip parameter* (which region a given board carves out), not a
/// protocol constant: the guest's own test is `_VAL != 0` on the HI register
/// (`kgspIsWpr2Up_TU102`, `ogkm-580: kernel_gsp_tu102.c:1251-1261`), so only
/// zero-vs-nonzero is load-bearing. No Ada card was measured for this number.
const WPR2_LO_UP: u64 = 0x02FF_E000;
/// See [`WPR2_LO_UP`] — equally invented, equally only nonzero-load-bearing.
const WPR2_HI_UP: u64 = 0x02FF_F000;

/// The teardown sentinel `MAILBOX0` must report once fn-47 has been serviced.
/// 580 tests exact equality (`ogkm-580: kernel_gsp_tu102.c:1225-1239`), so this value
/// **replaces** the mailbox shadow and is never OR-ed onto it.
const PROCESSOR_SUSPENDED: u64 = 0x8000_0000;

/// `"RMARGS"`, little-endian ASCII in an 8-byte LibOS region id.
const RMARGS_ID: u64 = 0x0000_524d_4152_4753;

/// The AD10x (Ada) GSP register model.
///
/// Its one field is the boot sequence it selects — a per-instance value, so which regime
/// a GPU boots under is a property of *that GPU*, never of the build.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ad10xGspModel {
    boot: FalconSecureBooterBoot,
}

impl Ad10xGspModel {
    /// The model.
    #[must_use]
    pub fn new() -> Ad10xGspModel {
        Ad10xGspModel {
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

impl GspModel for Ad10xGspModel {
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
            GspReg::GspFalconCpuctl | GspReg::Sec2FalconCpuctl => CPUCTL_HALTED,
            GspReg::GspFalconHwcfg2 => HWCFG2_RISCV_ENABLE,
            GspReg::GspFalconDmatrfcmd | GspReg::Sec2FalconDmatrfcmd => DMATRFCMD_IDLE,
            GspReg::GspFalconMailbox0 => {
                if obs.suspended {
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
            GspReg::Sec2FalconMailbox0 => 0,
            GspReg::GspQueueHead(_) => 0,
        })
    }

    /// ★ Ada is inside the falcon/secure-booter regime and NVIDIA's generated HAL says so
    /// in-tree: it binds the Turing/Ampere implementations across `TU102…AD107`
    /// (`ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:2374-2385`). This model and
    /// `kayfabe_device::ga10x::Ga10xGspModel` therefore select the **same** sequence value
    /// from two different crates — shared by selection, not by inheritance.
    fn boot_sequence(&self) -> &dyn BootSequence {
        &self.boot
    }

    fn libos_region_layout(&self) -> LibosRegionLayout {
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
/// [`Ad10xGspModel`].
///
/// ★ Composed the same way `kayfabe-crec`'s GA10x arch is, deliberately: the point of
/// this crate is to measure a *second* row against the *first* one, and swapping the
/// composition strategy would change what the measurement compares.
#[derive(Debug)]
pub struct Ad10xArch {
    inner: MockArch,
    gsp: Ad10xGspModel,
}

impl Default for Ad10xArch {
    fn default() -> Ad10xArch {
        Ad10xArch::new()
    }
}

impl Ad10xArch {
    /// The architecture.
    #[must_use]
    pub fn new() -> Ad10xArch {
        Ad10xArch {
            inner: MockArch::new(),
            gsp: Ad10xGspModel::new(),
        }
    }
}

impl Arch for Ad10xArch {
    fn name(&self) -> &'static str {
        "AD10x (AD106)"
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

kayfabe_util::assert_send_sync!(Ad10xGspModel);
