//! ★ **The die axis, derived — never captured** (lane C, 2026-10-14).
//!
//! A die is named by the two words the host's own `MC_GET_ARCH_INFO` returns
//! (`NV2080_CTRL_CMD_MC_GET_ARCH_INFO`, NON_PRIVILEGED): `architecture` and `implementation`.
//! The names below are ogkm's own `NV2080_CTRL_MC_ARCH_INFO_IMPLEMENTATION_*` defines
//! (`ogkm-595.84: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080mc.h:94-150`), tier 1 of the
//! constants ruling (§AF/§AH): nothing here is measured or hand-chosen.
//!
//! The die is the KEY of the published measured table (`kf_rm::consttab`): a measured row is
//! served only to the die it was measured on (`docs/design/V3_CONSTANT_STRATEGY.md` §3, "never a
//! neighbour's row"). A die whose implementation ogkm does not name has no name: it still runs
//! (every derived fact comes from its host), and every table lookup for it refuses by name.

use crate::{Family, FamilyRefusal, arch};

/// ★ One die, as the host reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DieId {
    /// `MC_GET_ARCH_INFO.architecture` (`NV2080_CTRL_MC_ARCH_INFO_ARCHITECTURE_*`).
    pub architecture: u32,
    /// `MC_GET_ARCH_INFO.implementation`.
    pub implementation: u32,
}

/// `(architecture, implementation, name)` for every DISCRETE die ogkm 595.84 names
/// (`ctrl2080mc.h:94-150`). ⊘ The `*00`/`*000` placeholders, the integrated parts
/// (`GA10B`, `GB10B`, `GB20B`, `GB20C`, `GH100_SOC`, `T23x`) and `AD101`/`AD000` — which share
/// implementation `0x1` in the header, so the pair names neither — are left out on purpose.
/// `GA100` and `GH100` and `GB100`/`GB200` are implementation `0`, which the header also uses
/// for the family placeholders `TU100`/`AD100`; only the three that are real dies are listed.
pub const DIE_NAMES: &[(u32, u32, &str)] = &[
    (arch::TU100, 0x2, "TU102"),
    (arch::TU100, 0x4, "TU104"),
    (arch::TU100, 0x6, "TU106"),
    (arch::TU100, 0x8, "TU116"),
    (arch::TU100, 0x7, "TU117"),
    (arch::GA100, 0x0, "GA100"),
    (arch::GA100, 0x2, "GA102"),
    (arch::GA100, 0x3, "GA103"),
    (arch::GA100, 0x4, "GA104"),
    (arch::GA100, 0x6, "GA106"),
    (arch::GA100, 0x7, "GA107"),
    (arch::GH100, 0x0, "GH100"),
    (arch::AD100, 0x2, "AD102"),
    (arch::AD100, 0x3, "AD103"),
    (arch::AD100, 0x4, "AD104"),
    (arch::AD100, 0x6, "AD106"),
    (arch::AD100, 0x7, "AD107"),
    (arch::GB100, 0x0, "GB100"),
    (arch::GB100, 0x2, "GB102"),
    (arch::GB100, 0x3, "GB110"),
    (arch::GB100, 0x4, "GB112"),
    (arch::GB200, 0x2, "GB202"),
    (arch::GB200, 0x3, "GB203"),
    (arch::GB200, 0x5, "GB205"),
    (arch::GB200, 0x6, "GB206"),
    (arch::GB200, 0x7, "GB207"),
];

impl DieId {
    /// The die from the two `MC_GET_ARCH_INFO` words.
    #[must_use]
    pub const fn new(architecture: u32, implementation: u32) -> DieId {
        DieId {
            architecture,
            implementation,
        }
    }

    /// ogkm's name for this die, or `None` when ogkm names none (then every table lookup refuses).
    #[must_use]
    pub fn name(self) -> Option<&'static str> {
        DIE_NAMES
            .iter()
            .find(|(a, i, _)| *a == self.architecture && *i == self.implementation)
            .map(|(_, _, n)| *n)
    }

    /// The die ogkm names `name` (exact, upper case), if any.
    #[must_use]
    pub fn from_name(name: &str) -> Option<DieId> {
        DIE_NAMES
            .iter()
            .find(|(_, _, n)| *n == name)
            .map(|(a, i, _)| DieId::new(*a, *i))
    }

    /// The family of this die ([`Family::from_arch`]).
    ///
    /// # Errors
    /// [`FamilyRefusal`], by name.
    pub fn family(self) -> Result<Family, FamilyRefusal> {
        Family::from_arch(self.architecture, self.implementation)
    }
}

impl std::fmt::Display for DieId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(
                f,
                "arch {:#x} impl {:#x} (no ogkm name)",
                self.architecture, self.implementation
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every listed name round-trips, is unique, and its family is the one its prefix says.
    #[test]
    fn every_name_round_trips_to_its_family() {
        for (a, i, n) in DIE_NAMES {
            let d = DieId::new(*a, *i);
            assert_eq!(d.name(), Some(*n));
            assert_eq!(DieId::from_name(n), Some(d));
            assert_eq!(
                DIE_NAMES.iter().filter(|(_, _, m)| m == n).count(),
                1,
                "{n} listed twice"
            );
            let want = match &n[..2] {
                "TU" => Family::Turing,
                "GA" => Family::Ampere,
                "AD" => Family::Ada,
                "GH" => Family::Hopper,
                "GB" => Family::Blackwell,
                other => panic!("unknown prefix {other}"),
            };
            assert_eq!(d.family(), Ok(want), "{n}");
        }
    }

    /// ⊘ Ambiguous or integrated implementations have no name (AD101/AD000 share 0x1).
    #[test]
    fn ambiguous_and_integrated_parts_are_unnamed() {
        assert_eq!(DieId::new(arch::AD100, 0x1).name(), None);
        assert_eq!(DieId::new(arch::GA100, 0xB).name(), None);
        assert_eq!(DieId::new(arch::GB200, 0xB).name(), None);
        assert_eq!(DieId::new(0x150, 0x0).name(), None);
        assert!(
            DieId::new(arch::AD100, 0x1)
                .to_string()
                .contains("no ogkm name")
        );
    }

    /// The four dies kayfabe has hardware evidence for, by the host's words.
    #[test]
    fn the_benched_dies_decode() {
        assert_eq!(DieId::new(arch::GA100, 6).to_string(), "GA106");
        assert_eq!(DieId::new(arch::AD100, 4).to_string(), "AD104");
        assert_eq!(DieId::new(arch::TU100, 8).to_string(), "TU116");
        assert_eq!(DieId::new(arch::GB200, 5).to_string(), "GB205");
    }
}
