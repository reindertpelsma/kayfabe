//! Capabilities an arm requires, and the host facts they are checked against.
//!
//! **Derive, never capture** (`AGENTS.md`). Nothing here names a die, a driver or a generation. A
//! capability is satisfied or refused by comparing what the *host answered* (`MC_GET_ARCH_INFO`, the
//! class list, the page size, `/proc`) with what the arm needs. The one family-shaped fact, the GMMU
//! small page, comes from `kf_chip::Family::mmu_format()` (a family row maintained by hand in
//! `kf-chip`, generated from ogkm); an architecture that row does not know prints `family=unknown`
//! and falls back to the page size the host reported, as the design says (V3_RAW_CLIENT.md §4.6).

use crate::outcome::NotRunReason;
use kf_chip::{DieId, Family, MmuFormat};

/// A guest-allocatable VA band the host reserved for us: `[base, base + len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaBand {
    /// First byte.
    pub base: u64,
    /// Length in bytes.
    pub len: u64,
}

/// What the host told us, gathered once at start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFacts {
    /// `MC_GET_ARCH_INFO.architecture`.
    pub architecture: u32,
    /// `MC_GET_ARCH_INFO.implementation`.
    pub implementation: u32,
    /// The RM driver version string the session gated on.
    pub driver: String,
    /// `open`, `proprietary` or `unknown` (from the driver's `/proc` version text).
    pub kmod: String,
    /// The kernel's page size.
    pub host_page_bytes: u64,
    /// The copy-engine class the host derived, if the host has one.
    pub ce_class: Option<u32>,
    /// The guest VA band, if the host reserved one.
    pub va_band: Option<VaBand>,
    /// Whether a present page's frame number reads non-zero (`None` = not probed).
    pub pagemap_pfn_visible: Option<bool>,
}

impl HostFacts {
    /// The die as the host reports it.
    #[must_use]
    pub fn die(&self) -> DieId {
        DieId::new(self.architecture, self.implementation)
    }

    /// The family row for this host, or `None` for an architecture the tree has no row for.
    #[must_use]
    pub fn family(&self) -> Option<Family> {
        self.die().family().ok()
    }

    /// The GMMU format of the family, if known.
    #[must_use]
    pub fn mmu(&self) -> Option<MmuFormat> {
        self.family().map(Family::mmu_format)
    }

    /// The smallest GMMU page: the family's, or the kernel page size when the family is unknown.
    #[must_use]
    pub fn grain_bytes(&self) -> u64 {
        self.mmu()
            .map_or(self.host_page_bytes, MmuFormat::small_page_bytes)
    }
}

/// What an arm needs from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// The GMMU small page, the kernel page and the arm's leaf are the same size.
    SmallPageMatchesLeaf,
    /// A copy-engine class for the correctness oracle.
    CopyEngine,
    /// A guest VA band big enough for the workload.
    GuestVaBand,
    /// Page frame numbers are readable (`/proc/self/pagemap` with `CAP_SYS_ADMIN`).
    PagemapPfn,
}

impl Capability {
    /// Stable name for `--list-arms`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Capability::SmallPageMatchesLeaf => "small-page-matches-leaf",
            Capability::CopyEngine => "copy-engine",
            Capability::GuestVaBand => "guest-va-band",
            Capability::PagemapPfn => "pagemap-pfn",
        }
    }
}

/// What the arm's parameters demand of the facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    /// The leaf (page) size the arm maps by.
    pub leaf_bytes: u64,
    /// Bytes of VA band the workload and its staging need.
    pub va_bytes: u64,
}

/// A capability the host does not provide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    /// What was missing.
    pub cap: Capability,
    /// The reason word.
    pub reason: NotRunReason,
    /// What the host answered (one token).
    pub detail: String,
}

/// Check `cap` against `facts`.
///
/// # Errors
/// [`Missing`], naming the reason.
pub fn check(cap: Capability, facts: &HostFacts, needs: &Needs) -> Result<(), Missing> {
    let miss = |reason, detail: String| Missing {
        cap,
        reason,
        detail,
    };
    match cap {
        Capability::SmallPageMatchesLeaf => {
            let grain = facts.grain_bytes();
            if grain == needs.leaf_bytes && facts.host_page_bytes == needs.leaf_bytes {
                Ok(())
            } else {
                Err(miss(
                    NotRunReason::GrainMismatch,
                    format!(
                        "gmmu_small={grain} kernel_page={} leaf={}",
                        facts.host_page_bytes, needs.leaf_bytes
                    ),
                ))
            }
        }
        Capability::CopyEngine => match facts.ce_class {
            Some(c) if c != 0 => Ok(()),
            _ => Err(miss(
                NotRunReason::NoCe,
                "host class list has no copy engine".into(),
            )),
        },
        Capability::GuestVaBand => match facts.va_band {
            None => Err(miss(
                NotRunReason::NoGuestVaBand,
                "host reserved no guest VA band".into(),
            )),
            Some(b) if b.len < needs.va_bytes => Err(miss(
                NotRunReason::NoGuestVaBand,
                format!("band={} need={}", b.len, needs.va_bytes),
            )),
            Some(_) => Ok(()),
        },
        Capability::PagemapPfn => match facts.pagemap_pfn_visible {
            Some(true) => Ok(()),
            Some(false) => Err(miss(
                NotRunReason::PagemapPfnHidden,
                "present pages read frame 0 (needs CAP_SYS_ADMIN)".into(),
            )),
            None => Err(miss(
                NotRunReason::ProcUnreadable,
                "pagemap was not probed".into(),
            )),
        },
    }
}

/// Check every capability in `required`, returning the first that is missing.
///
/// # Errors
/// The first [`Missing`].
pub fn check_all(required: &[Capability], facts: &HostFacts, needs: &Needs) -> Result<(), Missing> {
    required.iter().try_for_each(|&c| check(c, facts, needs))
}

#[cfg(test)]
pub(crate) fn test_facts() -> HostFacts {
    use kf_chip::arch;
    HostFacts {
        architecture: arch::GA100,
        implementation: 0x6,
        driver: "580.159.04".into(),
        kmod: "open".into(),
        host_page_bytes: 4096,
        ce_class: Some(0xc7b5),
        va_band: Some(VaBand {
            base: (1 << 32) + (1 << 29),
            len: 1 << 38,
        }),
        pagemap_pfn_visible: Some(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_chip::arch;

    const NEEDS: Needs = Needs {
        leaf_bytes: 4096,
        va_bytes: 400 << 20,
    };

    #[test]
    fn a_complete_host_satisfies_every_capability() {
        let f = test_facts();
        let all = [
            Capability::SmallPageMatchesLeaf,
            Capability::CopyEngine,
            Capability::GuestVaBand,
            Capability::PagemapPfn,
        ];
        assert_eq!(check_all(&all, &f, &NEEDS), Ok(()));
    }

    #[test]
    fn every_family_derives_its_grain_from_the_row_not_from_this_file() {
        for fam in Family::ALL {
            let want = fam.mmu_format().small_page_bytes();
            let (a, i) = match fam {
                Family::Turing => (arch::TU100, 0x6),
                Family::Ampere => (arch::GA100, 0x6),
                Family::Ada => (arch::AD100, 0x4),
                Family::Hopper => (arch::GH100, 0),
                Family::Blackwell => (arch::GB200, 0x2),
            };
            let f = HostFacts {
                architecture: a,
                implementation: i,
                ..test_facts()
            };
            assert_eq!(f.family(), Some(fam));
            assert_eq!(f.grain_bytes(), want);
        }
    }

    #[test]
    fn an_unknown_architecture_falls_back_to_the_reported_page_and_is_not_a_default_die() {
        let f = HostFacts {
            architecture: 0x1234,
            implementation: 1,
            ..test_facts()
        };
        assert_eq!(f.family(), None);
        assert_eq!(f.grain_bytes(), 4096);
        assert_eq!(check(Capability::SmallPageMatchesLeaf, &f, &NEEDS), Ok(()));
        let f64k = HostFacts {
            host_page_bytes: 65536,
            ..f
        };
        let e = check(Capability::SmallPageMatchesLeaf, &f64k, &NEEDS).unwrap_err();
        assert_eq!(e.reason, NotRunReason::GrainMismatch);
    }

    #[test]
    fn a_64k_kernel_page_on_a_known_family_is_a_grain_mismatch() {
        let f = HostFacts {
            host_page_bytes: 65536,
            ..test_facts()
        };
        let e = check(Capability::SmallPageMatchesLeaf, &f, &NEEDS).unwrap_err();
        assert_eq!(e.reason, NotRunReason::GrainMismatch);
        assert!(e.detail.contains("kernel_page=65536"));
    }

    #[test]
    fn each_missing_capability_names_its_reason() {
        let f = HostFacts {
            ce_class: None,
            ..test_facts()
        };
        assert_eq!(
            check(Capability::CopyEngine, &f, &NEEDS)
                .unwrap_err()
                .reason,
            NotRunReason::NoCe
        );
        let f = HostFacts {
            ce_class: Some(0),
            ..test_facts()
        };
        assert_eq!(
            check(Capability::CopyEngine, &f, &NEEDS)
                .unwrap_err()
                .reason,
            NotRunReason::NoCe
        );
        let f = HostFacts {
            va_band: None,
            ..test_facts()
        };
        assert_eq!(
            check(Capability::GuestVaBand, &f, &NEEDS)
                .unwrap_err()
                .reason,
            NotRunReason::NoGuestVaBand
        );
        let f = HostFacts {
            va_band: Some(VaBand {
                base: 0,
                len: 1 << 20,
            }),
            ..test_facts()
        };
        assert!(
            check(Capability::GuestVaBand, &f, &NEEDS)
                .unwrap_err()
                .detail
                .contains("need=")
        );
        let f = HostFacts {
            pagemap_pfn_visible: Some(false),
            ..test_facts()
        };
        assert_eq!(
            check(Capability::PagemapPfn, &f, &NEEDS)
                .unwrap_err()
                .reason,
            NotRunReason::PagemapPfnHidden
        );
        let f = HostFacts {
            pagemap_pfn_visible: None,
            ..test_facts()
        };
        assert_eq!(
            check(Capability::PagemapPfn, &f, &NEEDS)
                .unwrap_err()
                .reason,
            NotRunReason::ProcUnreadable
        );
    }
}
