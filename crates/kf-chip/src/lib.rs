//! ★★★★★ **v3 `kf-chip` — the chip model on the compatibility AXES.**
//!
//! Owner, 2026-09-13: *"stop maintaining per exact GPU die constants … derive them from the host
//! using unprivileged userspace or automatically from ogkm source or fabricated that satisfy ogkm
//! anyways, and only maintain things per family."* So this crate has three layers:
//!
//! 1. **The FAMILY row** — a few enum answers per family ([`Family::mmu_format`],
//!    [`Family::boot_style`], its GSP model). Class sets are GENERATED from ogkm ([`classes`]);
//!    the host classes are DERIVED from the host's own class list ([`host_classes`]).
//!    ★ Every family is first-class (owner, w826): no family is the reference the others hang off.
//! 2. **Register offsets** — generated from ogkm `dev_*.h` per family (to come).
//! 3. **Per-die facts** — read from the host GPU through unprivileged RM queries, each carrying
//!    its provenance (to come).
//!
//! ⊘ There is no pinned generation. The family is what the HOST reports (`MC_GET_ARCH_INFO`).

pub mod classes;
pub mod falcon_gsp;
pub mod host_classes;

pub use classes::{ClassSet, Kind, classes_for};
pub use host_classes::{DerivedHostClasses, HostLacksKind};

/// ★ A GPU family — ogkm's own axis (`MC_GET_ARCH_INFO`'s architecture). ⊘ NOT a die group: GA100
/// and GA10x are both Ampere, GB100 and GB202 both Blackwell; their differences are per-die FACTS
/// (the host's class list, its token layout, its sizes), never a new row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// Turing (TU10x/TU11x) — the GSP floor.
    Turing,
    /// Ampere (GA100, GA10x).
    Ampere,
    /// Ada (AD10x).
    Ada,
    /// Hopper (GH100).
    Hopper,
    /// Blackwell (GB100/GB102/GB11x datacenter, GB20x consumer).
    Blackwell,
}

/// The page-table format a family's GMMU walks — what the walk kernel's descriptor is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmuFormat {
    /// `NV_MMU_VER2` — 4 directory levels + PT (Pascal … Ada).
    Ver2,
    /// `NV_MMU_VER3` — one more directory level, moved fields (Hopper, Blackwell).
    Ver3,
}

/// How the family's GSP comes up — which boot sequence the emulated GSP plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootStyle {
    /// The falcon / SEC2-booter sequence (Turing … Ada).
    FalconSecureBooter,
    /// The FSP command queue (Hopper, Blackwell).
    Fsp,
}

/// `NV2080_CTRL_CMD_MC_GET_ARCH_INFO` — NON_PRIVILEGED (`ogkm-580: ctrl2080mc.h:61`, flags `0x1050b`).
pub const NV2080_CTRL_CMD_MC_GET_ARCH_INFO: u32 = 0x2080_1701;
/// `sizeof(NV2080_CTRL_MC_GET_ARCH_INFO_PARAMS)` — `{architecture, implementation, revision,
/// NvU8 subRevision}` + 3 pad.
pub const MC_GET_ARCH_INFO_SIZE: usize = 16;

/// `NV2080_CTRL_MC_ARCH_INFO_ARCHITECTURE_*` (`ogkm-580: ctrl2080mc.h:77-83`).
pub mod arch {
    /// Turing.
    pub const TU100: u32 = 0x160;
    /// Ampere.
    pub const GA100: u32 = 0x170;
    /// Hopper.
    pub const GH100: u32 = 0x180;
    /// Ada.
    pub const AD100: u32 = 0x190;
    /// Blackwell datacenter.
    pub const GB100: u32 = 0x1A0;
    /// Blackwell consumer.
    pub const GB200: u32 = 0x1B0;
}

/// Why no family was chosen — named, never a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyRefusal {
    /// An architecture this tree has no family for (pre-Turing: no GSP; or newer than ogkm).
    UnknownArchitecture(u32),
    /// An integrated (SoC) part — `GA10B`/`AD10B`/`GB20B` `0xB`, `GB20C` `0xC`, `GH100_SOC` `1`
    /// (`ogkm-580: ctrl2080mc.h:114-151`): no vidmem, and the store IS vidmem.
    Integrated {
        /// `MC_GET_ARCH_INFO` architecture.
        architecture: u32,
        /// `MC_GET_ARCH_INFO` implementation.
        implementation: u32,
    },
}

impl Family {
    /// Every family.
    pub const ALL: [Family; 5] = [Family::Turing, Family::Ampere, Family::Ada, Family::Hopper, Family::Blackwell];

    /// The family for what the host reported. ⊘ Any DISCRETE implementation of a known architecture
    /// is accepted — a new die of a known family needs no edit (its facts come from the host). Only
    /// integrated parts are refused: they have no vidmem.
    ///
    /// # Errors
    /// [`FamilyRefusal`], by name.
    pub fn from_arch(architecture: u32, implementation: u32) -> Result<Family, FamilyRefusal> {
        let integrated = match architecture {
            arch::GA100 | arch::AD100 | arch::GB200 => matches!(implementation, 0xB | 0xC),
            arch::GH100 => implementation == 1,
            _ => false,
        };
        if integrated {
            return Err(FamilyRefusal::Integrated { architecture, implementation });
        }
        match architecture {
            arch::TU100 => Ok(Family::Turing),
            arch::GA100 => Ok(Family::Ampere),
            arch::AD100 => Ok(Family::Ada),
            arch::GH100 => Ok(Family::Hopper),
            arch::GB100 | arch::GB200 => Ok(Family::Blackwell),
            other => Err(FamilyRefusal::UnknownArchitecture(other)),
        }
    }

    /// This family's generated engine class sets.
    #[must_use]
    pub fn classes(self) -> &'static ClassSet {
        classes_for(self)
    }

    /// The page-table format (`dev_mmu.h` per family: VER2 through Ada, VER3 from Hopper).
    #[must_use]
    pub const fn mmu_format(self) -> MmuFormat {
        match self {
            Family::Turing | Family::Ampere | Family::Ada => MmuFormat::Ver2,
            Family::Hopper | Family::Blackwell => MmuFormat::Ver3,
        }
    }

    /// How the GSP boots (`kgspBootstrap_*` HAL per family: falcon/booter through Ada, FSP after).
    #[must_use]
    pub const fn boot_style(self) -> BootStyle {
        match self {
            Family::Turing | Family::Ampere | Family::Ada => BootStyle::FalconSecureBooter,
            Family::Hopper | Family::Blackwell => BootStyle::Fsp,
        }
    }

    /// ★ The host classes for THIS die: the family's generated set ∩ the host's own class list.
    ///
    /// # Errors
    /// [`HostLacksKind`] when the host lists no class of a required kind.
    pub fn host_classes(self, host_classlist: &[u32]) -> Result<DerivedHostClasses, HostLacksKind> {
        DerivedHostClasses::choose(self, host_classlist)
    }

    /// ★ This family's GSP register model, for a framebuffer of `fb_size_mb` — the size the store
    /// actually holds (constraint 15), never a per-die constant.
    ///
    /// # Errors
    /// [`RowUnbuilt`] while a family's model is still being ported (named, with what remains).
    pub fn gsp_model(self, fb_size_mb: u64) -> Result<Box<dyn kf_arch::gsp::GspModel>, RowUnbuilt> {
        match self {
            // Ada's GSP boot HAL dispatches to the `_TU102`/`_GA102` bodies for the whole sequence;
            // every `GspReg` is at the same offset with the same encoding (old kayfabe-chips/ad10x.rs).
            Family::Ampere | Family::Ada => Ok(Box::new(falcon_gsp::FalconGspModel::with_fb_size_mb(fb_size_mb))),
            Family::Turing => Err(RowUnbuilt {
                family: self,
                what: "GSP model: _TU102 boot has no FWSEC-FRTS; the WPR2 derivation must be re-checked",
            }),
            Family::Hopper | Family::Blackwell => Err(RowUnbuilt {
                family: self,
                what: "GSP model: the shared FSP model is being ported (old gh100.rs/gb20x.rs)",
            }),
        }
    }
}

/// A family whose model is still being ported — refused by name while it is, never mocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowUnbuilt {
    /// The family.
    pub family: Family,
    /// What remains.
    pub what: &'static str,
}

/// Decode an `MC_GET_ARCH_INFO` reply into `(architecture, implementation)`.
#[must_use]
pub fn decode_arch_info(buf: &[u8; MC_GET_ARCH_INFO_SIZE]) -> (u32, u32) {
    let w = |o: usize| u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
    (w(0), w(4))
}

/// ★ The one chooser `kf_host::HostRm::open` takes: family from the host's architecture, host
/// classes from the host's own class list. Every caller passes this — never a local closure.
///
/// # Errors
/// The refusal, by name.
pub fn choose_host_classes(
    architecture: u32,
    implementation: u32,
    host_classlist: &[u32],
) -> Result<Box<dyn kf_arch::HostClasses>, String> {
    let family = Family::from_arch(architecture, implementation).map_err(|e| format!("{e:?}"))?;
    let chosen = family.host_classes(host_classlist).map_err(|e| format!("{e:?}"))?;
    Ok(Box::new(chosen))
}
