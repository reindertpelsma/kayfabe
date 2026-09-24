//! ★★★★★ **v3 `kf-chip` — the chip model on the compatibility AXES.**
//!
//! Owner, 2026-09-13: *"stop maintaining per exact GPU die constants … derive them from the host
//! using unprivileged userspace or automatically from ogkm source or fabricated that satisfy ogkm
//! anyways, and only maintain things per family."* So this crate has three layers:
//!
//! 1. **The FAMILY row** — hand-maintained, one per generation ([`Family`]): host classes,
//!    MMU/USERD/doorbell regime, GSP boot style. The ONLY hand-written chip data.
//! 2. **Register offsets** — generated from ogkm `dev_*.h` per family (to come).
//! 3. **Per-die facts** — read from the host GPU through unprivileged RM queries, each carrying
//!    its provenance (to come).
//!
//! ⊘ There is no pinned generation. The family is what the HOST reports (`MC_GET_ARCH_INFO`).

pub mod host_classes;

use kf_arch::HostClasses;

/// A GPU generation — the unit that is hand-maintained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// Ampere consumer (GA10x).
    Ga10x,
    /// Ada (AD10x).
    Ad10x,
    /// Hopper (GH100).
    Gh100,
    /// Blackwell consumer (GB20x).
    Gb20x,
}

/// `NV2080_CTRL_CMD_MC_GET_ARCH_INFO` — NON_PRIVILEGED (`ogkm-580: ctrl2080mc.h:61`, flags `0x1050b`).
pub const NV2080_CTRL_CMD_MC_GET_ARCH_INFO: u32 = 0x2080_1701;
/// `sizeof(NV2080_CTRL_MC_GET_ARCH_INFO_PARAMS)` — `{architecture, implementation, revision,
/// NvU8 subRevision}` + 3 pad.
pub const MC_GET_ARCH_INFO_SIZE: usize = 16;

/// `NV2080_CTRL_MC_ARCH_INFO_ARCHITECTURE_*` (`ogkm-580: ctrl2080mc.h:77-83`).
pub mod arch {
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
    /// An architecture this tree has no family row for.
    UnknownArchitecture(u32),
    /// Ampere, but an implementation outside the consumer GA10x parts (GA100 is datacenter
    /// Ampere with a different class set).
    AmpereNotGa10x(u32),
    /// Blackwell datacenter: no family row yet.
    BlackwellDatacenter,
    /// An integrated (SoC) part — `GA10B`/`AD10B`/`GB20B` `0xB`, `GB20C` `0xC`, `GH100_SOC` `1`
    /// (`ogkm-580: ctrl2080mc.h:114-151`). No vidmem: every family row assumes a framebuffer.
    Integrated {
        /// `MC_GET_ARCH_INFO` architecture.
        architecture: u32,
        /// `MC_GET_ARCH_INFO` implementation.
        implementation: u32,
    },
    /// An implementation id outside the discrete range this family row was written for.
    UnlistedImplementation {
        /// `MC_GET_ARCH_INFO` architecture.
        architecture: u32,
        /// `MC_GET_ARCH_INFO` implementation.
        implementation: u32,
    },
}

impl Family {
    /// Every family with a row.
    pub const ALL: [Family; 4] = [Family::Ga10x, Family::Ad10x, Family::Gh100, Family::Gb20x];

    /// The family for what the host reported. ⊘ Never a nearest-guess.
    ///
    /// # Errors
    /// [`FamilyRefusal`], by name.
    pub fn from_arch(architecture: u32, implementation: u32) -> Result<Family, FamilyRefusal> {
        // Implementation ids are `ogkm-580: ctrl2080mc.h:106-151`. ⊘ Integrated parts are refused
        // BEFORE the family match (review w826 #8: `>= 2` used to admit GA10B `0xB`).
        let integrated = match architecture {
            arch::GA100 | arch::AD100 | arch::GB200 => matches!(implementation, 0xB | 0xC),
            arch::GH100 => implementation == 1,
            _ => false,
        };
        if integrated {
            return Err(FamilyRefusal::Integrated { architecture, implementation });
        }
        let unlisted = || FamilyRefusal::UnlistedImplementation { architecture, implementation };
        match architecture {
            // GA100 (0) is datacenter Ampere with a different class set; GA102..GA107 are 2..=7.
            arch::GA100 if (2..=7).contains(&implementation) => Ok(Family::Ga10x),
            arch::GA100 if implementation == 0 => Err(FamilyRefusal::AmpereNotGa10x(implementation)),
            // AD102..AD107 are 2..=7 (AD100/AD000/AD101 are 0/1: never shipped discrete).
            arch::AD100 if (2..=7).contains(&implementation) => Ok(Family::Ad10x),
            arch::GH100 if implementation == 0 => Ok(Family::Gh100),
            // GB202..GB207 are 2..=7.
            arch::GB200 if (2..=7).contains(&implementation) => Ok(Family::Gb20x),
            arch::GB100 => Err(FamilyRefusal::BlackwellDatacenter),
            arch::GA100 | arch::AD100 | arch::GH100 | arch::GB200 => Err(unlisted()),
            other => Err(FamilyRefusal::UnknownArchitecture(other)),
        }
    }

    /// This family's host classes.
    #[must_use]
    pub fn host_classes(self) -> &'static dyn HostClasses {
        match self {
            Family::Ga10x => &host_classes::Ga10xHostClasses,
            Family::Ad10x => &host_classes::Ad10xHostClasses,
            Family::Gh100 => &host_classes::Gh100HostClasses,
            Family::Gb20x => &host_classes::Gb20xHostClasses,
        }
    }
}

/// Decode an `MC_GET_ARCH_INFO` reply into `(architecture, implementation)`.
#[must_use]
pub fn decode_arch_info(buf: &[u8; MC_GET_ARCH_INFO_SIZE]) -> (u32, u32) {
    let w = |o: usize| u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
    (w(0), w(4))
}
