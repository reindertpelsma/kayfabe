//! Axis B: the **GH100** (Hopper) register model — the *refutation fixture*.
//!
//! ## What this module is for
//!
//! It is not a working Hopper port and does not pretend to be one. It is the honest
//! stress case for the claim *"adding a GPU generation is a table row with zero
//! logic-crate edits"*, written far enough to make the failure **mechanical and pinned by
//! a test** rather than an argument.
//!
//! ## The measurement
//!
//! Hopper's *offsets* are largely Ampere's — `NV_PGSP_FALCON_MAILBOX0` is still
//! `0x110040` (`ogkm-580: hopper/gh100/dev_gsp.h:26`), `NV_PFB_PRI_MMU_WPR2_ADDR_LO/HI`
//! are still `0x001FA824`/`0x001FA828` (`ogkm-580: hopper/gh100/dev_fb.h:43,48`), and
//! `NV_PRISCV_RISCV_CPUCTL` is still `0x388`
//! (`ogkm-580: hopper/gh100/dev_riscv_pri.h:58`). If a generation were only offsets, this
//! would be another table row.
//!
//! It is not offsets. **Four of the eighteen [`GspReg`] variants have no register on this
//! generation at all**, and three of them are ones the boot FSM's `mmio_write` dispatcher
//! fires transitions on. Each absence is read from the vendored tree, not inferred:
//!
//! | [`GspReg`] variant | on GH100 | evidence (`ogkm-580`) |
//! |---|---|---|
//! | [`GspReg::GfwBootProgress`], [`GspReg::GfwBootPlm`] | **absent** | `hopper/gh100/dev_gc6_island*.h` define no `SECURE_SCRATCH_GROUP_05` (they define `GROUP_20`, the Confidential-Compute mode word, instead). NVIDIA's own generated HAL binds `gpuWaitForGfwBootComplete_TU102` for exactly `TU102…AD107` and the not-supported stub `_5baef9` for everything else (`src/nvidia/generated/g_gpu_nvoc.c:2374-2385`). The GFW-boot poll **does not run on this chip.** |
//! | [`GspReg::GspQueueHead`] | **absent** | `hopper/gh100/dev_gsp.h` defines no `NV_PGSP_QUEUE_HEAD`; it has `NV_PGSP_MAILBOX(i)` at `0x110804` and the `EMEMC`/`EMEMD` window. The only writer of `NV_PGSP_QUEUE_HEAD` in the tree is the Turing HAL (`kernel_gsp_tu102.c:351,354`). |
//! | [`GspReg::Sec2FalconCpuctl`], [`GspReg::Sec2FalconMailbox0`] | **not the boot path** | `kgspBootstrap_GH100` boots the GSP-FMC through **FSP** (`kfspSendBootCommands_HAL`) — an EMEM command queue at `NV_PFSP_MSGQ_HEAD(i) = 0x008F2c80+(i)*8` (`hopper/gh100/dev_fsp_pri.h:44`) — and only falls back to SEC2 behind `PDB_PROP_KSEC2_BOOT_GSPFMC` (`kernel_gsp_gh100.c:833-876`). There is no Booter Load/Unload argument convention to latch. |
//!
//! ## Why that is a **logic-crate** failure and not an unbuilt adapter
//!
//! The FSM does not consume `GspReg` abstractly. `kayfabe-gsp`'s `GspFsm::mmio_write`
//! matches the enum arm by arm and *encodes the Turing boot ordering in the match itself*
//! — `Sec2FalconMailbox0` latches the Booter argument, `Sec2FalconCpuctl` + `is_startcpu`
//! decides Load-vs-Unload and fires `E4`/`E5`, `GspQueueHead(_)` is the doorbell that
//! drives `E7`. `kayfabe-gsp` is a **logic crate** — it is in all three of `ci.yml`'s
//! `pure=` lists, including the generation-name gate's, whose stated rule is that a
//! chip's facts cost an adapter and *zero* logic-crate edits.
//!
//! So the seam holds the *values* (offsets, bit positions, sentinels) but not the
//! *sequence*. Hopper's boot has stages Turing's does not (FSP secure-boot wait, GSP
//! target-mask release, lockdown release) and lacks stages Turing's has (GFW-boot poll,
//! SEC2 Booter Load). Expressing it needs new `GspReg` variants — in `kayfabe-arch`, also
//! a logic crate — and new `match` arms in `kayfabe-gsp`. That is the bolt-on, and it is
//! named in [`MISSING_TRANSITIONS`].
//!
//! ## What this model deliberately does NOT do
//!
//! It does not fake a `GfwBootProgress` or a `GspQueueHead` offset to make the FSM run.
//! Answering a register the silicon does not have is exactly the *"defaulted zero"* the
//! `GspModel` seam exists to refuse: `decode_reg` returns `None` (*"another model owns
//! this offset"*) and `encode` returns `None` (a `RegisterUnserviceable` fault). A green
//! obtained by inventing four registers would be a measurement of nothing.

use kayfabe_arch::gsp::{
    BootSequence, GspModel, GspObservation, GspReg, LibosRegionLayout, NoBootSequence,
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

/// ★★★ **The bolt-on, enumerated.** Boot events GH100 has that no [`GspReg`] variant can
/// name, so no `GspModel` implementation can report them and the FSM cannot transition on
/// them.
///
/// This is the concrete cost of the second architecture, and it is **not** an adapter:
/// every item needs a new variant in `kayfabe-arch` and a new arm in `kayfabe-gsp`, both
/// logic crates.
pub const MISSING_TRANSITIONS: &[(&str, &str)] = &[
    (
        "FSP secure-boot command",
        "`kgspBootstrap_GH100` -> `kfspSendBootCommands_HAL` writes an EMEM command queue \
         at `NV_PFSP_MSGQ_HEAD(i)`/`NV_PFSP_QUEUE_HEAD` (ogkm-580: hopper/gh100/\
         dev_fsp_pri.h:44). There is no falcon STARTCPU to observe: the boot request is a \
         MESSAGE, not a register poke, and `GspReg` has no variant for a queue that is \
         not the GSP command queue.",
    ),
    (
        "GSP target-mask release",
        "`kfspWaitForGspTargetMaskReleased_HAL`, polled after the boot command \
         (ogkm-580: kernel_gsp_gh100.c:877-900). No `GspReg` variant.",
    ),
    (
        "lockdown release / FMC error",
        "`gpuTimeoutCondWait(_kgspLockdownReleasedOrFmcError)` \
         (ogkm-580: kernel_gsp_gh100.c:925-935); the FMC reports errors by making \
         `NV_PFALCON_FALCON_MAILBOX0` NON-ZERO (`:552`) and success by leaving it ZERO \
         (`:562`). That is the OPPOSITE polarity to the Turing regime, where MAILBOX0 \
         carries the boot-args low half — the same register, a different meaning, and \
         `GspObservation` has one field for it.",
    ),
    (
        "Confidential-Compute WPR2 suppression",
        "`kgspIsWpr2Up_GH100` returns NV_FALSE unconditionally under CC \
         (ogkm-580: kernel_gsp_gh100.c:220-236). `GspObservation` has no CC flag, so the \
         model cannot vary its WPR2 answer on it.",
    ),
];

/// The GH100 (Hopper) GSP register model — see the module docs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gh100GspModel {
    boot: NoBootSequence,
}

impl Gh100GspModel {
    /// The model.
    #[must_use]
    pub fn new() -> Gh100GspModel {
        Gh100GspModel {
            boot: NoBootSequence,
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
            // ── the four absences, each sourced in the module docs ──
            GspReg::GfwBootProgress
            | GspReg::GfwBootPlm
            | GspReg::Sec2FalconCpuctl
            | GspReg::Sec2FalconMailbox0
            | GspReg::Sec2FalconDmatrfcmd
            | GspReg::GspQueueHead(_) => return None,
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
            GspReg::GspFalconHwcfg2 => HWCFG2_RISCV_ENABLE,
            GspReg::GspFalconDmatrfcmd => DMATRFCMD_IDLE,
            // ★ Same register, opposite meaning to the Turing regime — see
            // `MISSING_TRANSITIONS`. Echoing the boot-args low half here is what the
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
            // MISS = FAULT. This generation has no such register; a defaulted zero would
            // be a guess, and the FSM raises `RegisterUnserviceable` instead.
            GspReg::GfwBootProgress
            | GspReg::GfwBootPlm
            | GspReg::Sec2FalconCpuctl
            | GspReg::Sec2FalconMailbox0
            | GspReg::Sec2FalconDmatrfcmd
            | GspReg::GspQueueHead(_) => return None,
        })
    }

    /// ★★★ **NOT IMPLEMENTED, and it says so rather than borrowing one.** This
    /// generation's boot is an FSP command queue, not a falcon STARTCPU + SEC2 Booter
    /// Load; selecting the falcon regime here would make the model *appear* to boot by
    /// running another generation's ordering. `NoBootSequence` declares zero stages and
    /// answers no step, so the gap is a red test rather than a plausible green.
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
