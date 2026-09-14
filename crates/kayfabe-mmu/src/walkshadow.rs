//! ★★★★★ **SHADOW MODE FOR THE WALK KERNEL** — `SINGLE_STORE_PLAN.md` §6, step 1.
//!
//! The host walk decides, as it does today. The kernel's report is computed **alongside** and
//! compared, and the disagreements are counted by kind. Nothing here changes what is
//! published; that is the whole point.
//!
//! > **The idiom is this tree's own.** The doorbell table was wired in shadow first —
//! > consulted on every doorbell, deciding nothing, printing an agreement census — and only
//! > then armed. A differential on **real guest page tables, live** is strictly stronger than
//! > the synthetic corpus and strictly stronger than *"the raw client still passes"*, because
//! > it is the only thing that can find a disagreement only a real driver's tables produce.
//! > The `KIND` finding (§w725b) is proof that class exists.
//!
//! # ⊘⊘⊘ WHAT IS COMPARED, AND WHAT IS NOT — read this before reading a zero
//!
//! The two walkers do **not** decode the same set of fields, and pretending otherwise would
//! manufacture a disagreement on every run:
//!
//! | field | host `DecodedLeaf` | kernel `MapRun` |
//! |---|---|---|
//! | `va`, `gpga`, `len` | ✔ | ✔ |
//! | page-size class | ✔ (`size`) | ✔ (`RF_PS`) |
//! | aperture | ✔ | ✔ (`RF_AP`) |
//! | read-only | ✔ | ✔ (`RF_READ_ONLY`) |
//! | **volatile / privilege / atomic-disable** | ⊘ **not decoded** | ✔ |
//! | **KIND** (§w725b) | ⊘ **not decoded** | ✔ |
//!
//! ⇒ [`COMPARED_FLAGS`] is the intersection, and the census **prints the mask** so that a
//! reader can never take *"zero disagreements"* for *"the two walkers agree about
//! everything"*. ⚠ That distinction is the whole reason this module states it twice: a
//! comparison narrowed to make a number green, without saying so, is the defect this tree
//! keeps cataloguing.
//!
//! ★ The narrowing is **not** a weakness of the differential for its purpose. What step 2
//! needs to be safe is that the kernel finds *the same mappings, at the same addresses, of the
//! same length, in the same aperture, with the same writability*. The fields it decodes and
//! the host does not are strictly additional information, and the host cannot be wrong about
//! them because it never reads them.

use crate::walkdiff::{self, PageClass, Run};
use crate::walker::DecodedLeaf;
use kayfabe_arch::Aperture;

/// The aperture bits, as [`crate::walkreport`] spells them.
const RF_AP_SHIFT: u32 = 0;
/// Their mask.
const RF_AP_MASK: u32 = 0x7;
/// `KFWR_RF_READ_ONLY`.
const RF_READ_ONLY: u32 = 1 << 3;

/// ★★★ **The flag bits both walkers produce**, and therefore the only ones a disagreement may
/// be raised on.
///
/// ⊘ Aperture and read-only. **Not** volatile, privilege, atomic-disable or `KIND`: the host
/// walker does not decode them, so comparing them would report a difference between what two
/// decoders *looked at*, not between what the guest's tables *say*.
pub const COMPARED_FLAGS: u32 = RF_AP_MASK | RF_READ_ONLY;

/// The kernel's aperture encoding for one of ours.
fn ap_code(a: Aperture) -> u32 {
    match a {
        Aperture::Vidmem => 0,
        Aperture::Peer => 1,
        Aperture::SysmemCoherent => 2,
        Aperture::SysmemNonCoherent => 3,
    }
}

/// The page-size class of a leaf, or `None` for a size no class covers.
///
/// ⊘ `None` rather than a default: a leaf whose size this build has no class for is a fact
/// about the format, and silently filing it under 4 KiB would make a real disagreement
/// invisible by turning it into an equal comparison.
fn class_of(size: u64) -> Option<PageClass> {
    match size {
        4096 => Some(PageClass::P4K),
        65_536 => Some(PageClass::P64K),
        0x0020_0000 => Some(PageClass::P2M),
        0x2000_0000 => Some(PageClass::P512M),
        _ => None,
    }
}

/// ★★ **The host walk's leaves as [`Run`]s**, so the two descriptions can be compared at all.
///
/// One run per leaf, then [`walkdiff::canonical`] — which explodes to pages and re-coalesces,
/// so *"two descriptions of the same mapping set compare equal regardless of how the walk
/// happened to cut them into runs"*. That is what makes a comparison against a **coalescing**
/// kernel meaningful rather than a comparison of two chopping strategies.
///
/// Returns the runs and the number of leaves dropped for having no page-size class. ⊘ The
/// count is returned rather than logged: a comparison run over a set that silently lost
/// members is not a comparison, and the census has to be able to say so.
#[must_use]
pub fn leaves_as_runs(leaves: &[DecodedLeaf]) -> (Vec<Run>, usize) {
    let mut out = Vec::with_capacity(leaves.len());
    let mut dropped = 0usize;
    for l in leaves {
        let Some(class) = class_of(l.size.0) else {
            dropped += 1;
            continue;
        };
        let mut flags = ap_code(l.aperture) << RF_AP_SHIFT;
        if l.read_only {
            flags |= RF_READ_ONLY;
        }
        out.push(Run {
            va: l.va.0,
            gpga: l.phys,
            len: l.size.0,
            flags,
            class,
        });
    }
    (walkdiff::canonical(&out), dropped)
}

/// What kind of disagreement. ⊘ Five kinds and not a boolean: *"the kernel missed a mapping"*,
/// *"the kernel invented one"* and *"the two disagree about where it points"* send a reader to
/// three different places, and a single count sends them nowhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DisagreementKind {
    /// The host found a mapping the kernel did not report. ⚠ The most serious kind: under the
    /// swap this is a mapping the guest believes it has and the GPU would not be given.
    MissingInKernel,
    /// The kernel reported a mapping the host did not find.
    ExtraInKernel,
    /// Both found a mapping at this VA and class, pointing at different GPGAs.
    GpgaDiffers,
    /// Both found one, of different lengths.
    LenDiffers,
    /// Both found one, with different [`COMPARED_FLAGS`].
    FlagsDiffer,
}

impl DisagreementKind {
    /// A short name for a census column.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DisagreementKind::MissingInKernel => "missing_in_kernel",
            DisagreementKind::ExtraInKernel => "extra_in_kernel",
            DisagreementKind::GpgaDiffers => "gpga_differs",
            DisagreementKind::LenDiffers => "len_differs",
            DisagreementKind::FlagsDiffer => "flags_differ",
        }
    }
}

/// One disagreement, with enough to find it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disagreement {
    /// Which kind.
    pub kind: DisagreementKind,
    /// The virtual address it is about.
    pub va: u64,
    /// The page-size class it is about.
    pub class: PageClass,
    /// What the host said (`gpga`, `len`, masked flags) — zeroes when the host has nothing.
    pub host: (u64, u64, u32),
    /// What the kernel said. Zeroes when the kernel has nothing.
    pub kernel: (u64, u64, u32),
}

/// ★★★★★ **THE COMPARISON.** Both sides must already be [`walkdiff::canonical`].
///
/// ⊘ Keyed on `(class, va)`, because [`walkdiff`]'s own rule is that runs are diffed **within**
/// a class and never across: two runs differing only in page size are different mappings, and
/// the same VA may legitimately be covered by two classes at once.
#[must_use]
pub fn compare(host: &[Run], kernel: &[Run]) -> Vec<Disagreement> {
    use std::collections::BTreeMap;
    let key = |r: &Run| (r.class, r.va);
    let h: BTreeMap<_, _> = host.iter().map(|r| (key(r), *r)).collect();
    let k: BTreeMap<_, _> = kernel.iter().map(|r| (key(r), *r)).collect();
    let mut out = Vec::new();
    for (kk, hr) in &h {
        match k.get(kk) {
            None => out.push(Disagreement {
                kind: DisagreementKind::MissingInKernel,
                va: hr.va,
                class: hr.class,
                host: (hr.gpga, hr.len, hr.flags & COMPARED_FLAGS),
                kernel: (0, 0, 0),
            }),
            Some(kr) => {
                // ⊘ At most ONE disagreement per key, and the order is deliberate: a run at
                // the wrong GPGA is a different fault from one of the wrong length, and
                // reporting both for one run would make the by-kind census double-count.
                let kind = if hr.gpga != kr.gpga {
                    Some(DisagreementKind::GpgaDiffers)
                } else if hr.len != kr.len {
                    Some(DisagreementKind::LenDiffers)
                } else if (hr.flags & COMPARED_FLAGS) != (kr.flags & COMPARED_FLAGS) {
                    Some(DisagreementKind::FlagsDiffer)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    out.push(Disagreement {
                        kind,
                        va: hr.va,
                        class: hr.class,
                        host: (hr.gpga, hr.len, hr.flags & COMPARED_FLAGS),
                        kernel: (kr.gpga, kr.len, kr.flags & COMPARED_FLAGS),
                    });
                }
            }
        }
    }
    for (kk, kr) in &k {
        if !h.contains_key(kk) {
            out.push(Disagreement {
                kind: DisagreementKind::ExtraInKernel,
                va: kr.va,
                class: kr.class,
                host: (0, 0, 0),
                kernel: (kr.gpga, kr.len, kr.flags & COMPARED_FLAGS),
            });
        }
    }
    out
}

/// ★★★ **The shadow census.** Accumulated across a boot and printed once at teardown.
#[derive(Debug, Clone, Default)]
pub struct ShadowCensus {
    /// Refreshes on which both walkers ran and were compared.
    pub compared: u64,
    /// Refreshes on which the kernel could not be run at all. ⊘ Counted separately: a boot
    /// that never ran the kernel has zero disagreements for a reason that is not agreement.
    pub kernel_unavailable: u64,
    /// Runs the host walk produced, summed.
    pub host_runs: u64,
    /// Runs the kernel produced, summed.
    pub kernel_runs: u64,
    /// Leaves the host walk produced that had no page-size class and were dropped from the
    /// comparison. ⚠ Non-zero means the comparison was over a set that lost members.
    pub host_unclassed: u64,
    /// Disagreements, by kind.
    pub by_kind: std::collections::BTreeMap<&'static str, u64>,
    /// The first few, kept verbatim so the census is actionable and not just a tally.
    pub first: Vec<Disagreement>,
}

/// How many disagreements the census keeps verbatim.
pub const KEPT: usize = 8;

impl ShadowCensus {
    /// Fold one refresh's comparison in.
    pub fn note(&mut self, host: &[Run], kernel: &[Run], host_unclassed: usize, d: &[Disagreement]) {
        self.compared += 1;
        self.host_runs += host.len() as u64;
        self.kernel_runs += kernel.len() as u64;
        self.host_unclassed += host_unclassed as u64;
        for x in d {
            *self.by_kind.entry(x.kind.as_str()).or_insert(0) += 1;
            if self.first.len() < KEPT {
                self.first.push(*x);
            }
        }
    }

    /// A refresh on which the kernel could not run.
    pub fn note_unavailable(&mut self) {
        self.kernel_unavailable += 1;
    }

    /// Total disagreements.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.by_kind.values().sum()
    }

    /// ★★★ The line. ⊘ It states the **compared flag mask** and the **vacuity guards** in the
    /// line itself, because a zero that is read without them is read as more than it is.
    #[must_use]
    pub fn render(&self) -> String {
        let total = self.total();
        let by = if self.by_kind.is_empty() {
            "none".to_string()
        } else {
            self.by_kind
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let verdict = if self.compared == 0 {
            "⊘⊘ VACUOUS — the two walkers were never compared on this boot. This is NOT \
             agreement; it is an unarmed or unreachable shadow."
                .to_string()
        } else if self.host_runs == 0 && self.kernel_runs == 0 {
            "⊘⊘ VACUOUS — compared, and both sides were EMPTY every time. A comparison of two \
             empty sets agrees for a reason that has nothing to do with the walkers."
                .to_string()
        } else if total == 0 {
            "★★★ AGREEMENT over every compared refresh, on the fields both walkers decode. \
             ⚠ Necessary and not sufficient for the swap: see `compared_flags` — volatile, \
             privilege, atomic-disable and KIND are decoded by the KERNEL ONLY and are not in \
             this number."
                .to_string()
        } else {
            format!(
                "⊘⊘ {total} DISAGREEMENTS — the kernel may NOT replace the host walk. \
                 `missing_in_kernel` is the serious kind: under the swap those are mappings \
                 the guest believes it has and the GPU would not be given."
            )
        };
        let first = self
            .first
            .iter()
            .map(|d| {
                format!(
                    "[{} va={:#x} class={:?} host=({:#x},{:#x},{:#x}) kernel=({:#x},{:#x},{:#x})]",
                    d.kind.as_str(),
                    d.va,
                    d.class,
                    d.host.0,
                    d.host.1,
                    d.host.2,
                    d.kernel.0,
                    d.kernel.1,
                    d.kernel.2
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "WALK-SHADOW compared={} kernel_unavailable={} host_runs={} kernel_runs={} \
             host_unclassed={} compared_flags={:#x} disagreements={} by_kind[{}] first[{}] ⇒ {}",
            self.compared,
            self.kernel_unavailable,
            self.host_runs,
            self.kernel_runs,
            self.host_unclassed,
            COMPARED_FLAGS,
            total,
            by,
            first,
            verdict
        )
    }
}
