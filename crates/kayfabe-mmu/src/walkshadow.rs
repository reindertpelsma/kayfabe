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

// ─────────────────────────────────────────────────────────────────────────────────────────
// ★★★★★ THE LIVE HALF — building an image the walk kernel can actually be pointed at.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// ⊘⊘⊘ **WHY THE GUEST'S FRAMEBUFFER CANNOT BE HANDED TO THE KERNEL AS IT STANDS.**
///
/// The kernel addresses page-table pages as **offsets into one flat window** —
/// `KfArgs::win = { base, len }`, and every dereference is `win.base + gpga` bounds-checked
/// against `win.len` (`cuda/walk/kf_walk.cu:346,357`). So a window that answers for the
/// guest's tables at their own GPGA must be **as long as the highest table page's address**.
///
/// `[measured, w730]` the page arena's high-water on a full raw-client boot is
/// `span_pages=3087533` — the guest's own RM puts its tables at the **top** of a 12 GiB
/// framebuffer, ~11.78 GiB up. `[corroborated]` `cuda/walk/corpus/real_leaves.txt`, w725's
/// capture of a real driver's tables, has its five address-space roots at
/// `0x2efa4c000 .. 0x2f1cac000` — the same place.
///
/// ⇒ An identity window is an **11.8 GiB device allocation on a 12 GiB board**, competing
/// with the guest's own forwarded video memory. It is not affordable, and w725 hit exactly
/// this wall from the other side: *"the tables live across ~12 GiB of framebuffer, which no
/// test buffer can hold"*.
///
/// ★ Its answer is this one. **Relocate**: copy the table pages into a compact image and
/// rewrite the **address field of directory entries** to point at the new homes, leaving every
/// leaf PTE byte for byte untouched — because a leaf's target is *reported* and never
/// followed. The reported `va` and `gpga` are therefore the guest's own numbers and compare
/// directly against the host walk's.
///
/// # ⊘⊘ WHAT RELOCATION COSTS THE DIFFERENTIAL, stated where the number is read
///
/// The image holds exactly the pages the **host walk visited**. ⇒ the kernel's *reach* is
/// clipped to the host's reach, and it cannot find a subtree the host never entered.
///
/// - [`DisagreementKind::MissingInKernel`] — **fully live.** The kernel is given every byte
///   the host had, so *"the host found this mapping and the kernel did not"* is still a
///   statement about the two decoders. This is the serious kind, and it is not weakened.
/// - [`DisagreementKind::ExtraInKernel`] — **live within the visited pages** (a slot the
///   kernel reads as a leaf and the host did not), **foreclosed beyond them**.
/// - `gpga` / `len` / flags — fully live.
///
/// ⚠ Stated in the census line itself, beside [`COMPARED_FLAGS`], for the same reason: a
/// zero read without its scope is read as more than it is.
#[derive(Debug, Clone)]
pub struct ShadowImage {
    /// The compact image. Slot 0 is reserved and zero; table pages start at slot 1.
    pub bytes: Vec<u8>,
    /// `(the address space's real pdb, the root's address in this image)`, ascending by the
    /// second. ⊘ Both halves are kept because the report comes back keyed by the *relocated*
    /// root and the comparison is per real address space.
    pub roots: Vec<(u64, u64)>,
    /// Table pages the image holds (excluding the reserved zero slot).
    pub pages: usize,
    /// ★★ Directory edges pointed at the reserved zero slot because the host walk never
    /// visited the page they name — the **measured** size of the reach clipping above.
    /// ⊘ Counted rather than refused: a budgeted or faulted host walk legitimately leaves
    /// subtrees unentered, and refusing the whole image for that would make the shadow
    /// unreachable on exactly the boots it is most wanted.
    pub absent_edges: u64,
    /// Directory edges naming a sub-table in **system memory**, left encoded as the guest
    /// wrote them. The kernel refuses a non-vidmem table page by name
    /// (`KFWR_R_FOREIGN_AP`) and never follows it; the host walk does follow it. ⚠ A
    /// non-zero count here predicts `missing_in_kernel` that is a property of the two
    /// walkers' *scope*, not of their decoding.
    pub sysmem_edges: u64,
}

/// The reserved slot every unvisited directory edge is pointed at: 4 KiB of zeros, which
/// decodes as a table with nothing in it.
///
/// ⊘ Zero rather than out-of-range. Out-of-range would fire `KFWR_R_OOB` and mark the whole
/// report refused, so a single unentered subtree would make every refresh unreadable; zero
/// makes the kernel see exactly what the host saw there — nothing — and
/// [`ShadowImage::absent_edges`] is what says so out loud.
pub const ABSENT_SLOT: u64 = 0;

/// Why an image could not be built. ⊘ Every arm names the thing that refused, because the
/// census prints these and *"the shadow did not run"* is not an actionable sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShadowRefusal {
    /// The host walk visited no vidmem table page, so there is nothing to compare.
    NoPages,
    /// More table pages than the image budget allows.
    TooManyPages {
        /// What the walk visited.
        pages: usize,
        /// What the budget allows.
        cap: usize,
    },
    /// The framebuffer would not serve a page the host walk had just read.
    Unreadable {
        /// Its address.
        phys: u64,
    },
    /// One physical page was visited at two different levels, so it has no single decode.
    AmbiguousLevel {
        /// Its address.
        phys: u64,
    },
    /// The format has no geometry for a level the walk visited.
    BadGeometry {
        /// The level.
        level: u8,
    },
    /// [`kayfabe_arch::Relocated::Refused`], verbatim.
    Relocate(&'static str),
    /// An address space's root page is not in the image.
    RootMissing {
        /// The page-directory base.
        pdb: u64,
    },
}

impl ShadowRefusal {
    /// A short, stable name for a census column.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ShadowRefusal::NoPages => "no_pages",
            ShadowRefusal::TooManyPages { .. } => "too_many_pages",
            ShadowRefusal::Unreadable { .. } => "unreadable_page",
            ShadowRefusal::AmbiguousLevel { .. } => "ambiguous_level",
            ShadowRefusal::BadGeometry { .. } => "bad_geometry",
            ShadowRefusal::Relocate(_) => "relocate_refused",
            ShadowRefusal::RootMissing { .. } => "root_missing",
        }
    }
}

/// How many table pages one image may hold. 4096 pages is 16 MiB — comfortably above the
/// `[w724c]` measurement of **7.3 MiB** of resident tables, and far below anything that
/// would make the device allocation interesting.
pub const PAGE_CAP: usize = 4096;

/// Bytes in one table page. ⊘ The whole design rests on the walk kernel and this crate
/// agreeing that a table page is 4 KiB, which every VER2 level does.
pub const IMAGE_PAGE: usize = 4096;

/// ★★★★★ **BUILD THE COMPACT IMAGE** the walk kernel is pointed at.
///
/// `vases` is one entry per address space: its page-directory base and the pages the host
/// walk actually visited under it ([`crate::walker::SubtreeDecode::visited`]).
///
/// # Errors
/// [`ShadowRefusal`], naming what refused.
pub fn build_image(
    fmt: &dyn kayfabe_arch::GmmuFmt,
    fb: &mut dyn crate::walker::FbRead,
    vases: &[(u64, &[crate::walker::PtPage])],
    page_cap: usize,
) -> Result<ShadowImage, ShadowRefusal> {
    use std::collections::BTreeMap;

    // ⊘ Keyed by address, because one page may be reached from two address spaces and must
    // get ONE home — two copies would let the kernel and the host disagree about a page that
    // is the same page.
    let mut level_of: BTreeMap<u64, u8> = BTreeMap::new();
    for (_, visited) in vases {
        for p in *visited {
            if p.aperture != Aperture::Vidmem {
                // A table page in guest system memory is not in the framebuffer and the
                // kernel refuses it by name; it is deliberately not in the image, and the
                // directory entry naming it keeps its own encoding so that refusal fires.
                continue;
            }
            let base = p.phys & !0xfff;
            match level_of.get(&base) {
                Some(l) if *l != p.level => {
                    return Err(ShadowRefusal::AmbiguousLevel { phys: base })
                }
                _ => {
                    level_of.insert(base, p.level);
                }
            }
        }
    }
    if level_of.is_empty() {
        return Err(ShadowRefusal::NoPages);
    }
    if level_of.len() > page_cap {
        return Err(ShadowRefusal::TooManyPages {
            pages: level_of.len(),
            cap: page_cap,
        });
    }

    // Slot 1 upwards; slot 0 stays zero and is where every unvisited edge is sent.
    let home_of: BTreeMap<u64, u64> = level_of
        .keys()
        .enumerate()
        .map(|(i, &phys)| (phys, ((i + 1) * IMAGE_PAGE) as u64))
        .collect();

    let mut bytes = vec![0u8; (level_of.len() + 1) * IMAGE_PAGE];
    let absent = std::cell::Cell::new(0u64);
    let home = |old: u64| -> Option<u64> {
        if old & 0xfff != 0 {
            // An unaligned sub-table pointer is not a page we could have visited. It goes to
            // the absent slot like any other edge we cannot follow — and the kernel's own
            // `KFWR_R_UNALIGNED` is not the right instrument here, because it would fire on
            // OUR rewrite rather than on the guest's encoding.
            absent.set(absent.get() + 1);
            return Some(ABSENT_SLOT);
        }
        match home_of.get(&old) {
            Some(&h) => Some(h),
            None => {
                absent.set(absent.get() + 1);
                Some(ABSENT_SLOT)
            }
        }
    };

    let mut sysmem_edges = 0u64;
    for (&phys, &level) in &level_of {
        let slot = home_of[&phys] as usize;
        let es = usize::from(fmt.entry_size(level));
        let Some(geom) = fmt.level_shift(level) else {
            return Err(ShadowRefusal::BadGeometry { level });
        };
        if es == 0 || es > 16 {
            return Err(ShadowRefusal::BadGeometry { level });
        }
        // ★★ **Exactly the bytes the table HAS**, not a whole page. `LevelShift::entries` is a
        // count read off the level's own virtual-address bits — VER2's big-page table holds
        // **32** entries in a page that could hold 512 — and a 4 KiB read of it would demand
        // 3 840 bytes nobody has to be able to serve. ⊘ The walk kernel sizes its loads the
        // same way, so the image holds neither more nor less than either walker reads.
        let need = (geom.entries as usize).saturating_mul(es).min(IMAGE_PAGE);
        let page = &mut bytes[slot..slot + IMAGE_PAGE];
        if need == 0 || !fb.read_in(phys, Aperture::Vidmem, &mut page[..need]) {
            return Err(ShadowRefusal::Unreadable { phys });
        }
        for i in 0..geom.entries as usize {
            let off = i * es;
            if off + es > need {
                break;
            }
            let mut w = [0u8; 16];
            w[..es].copy_from_slice(&page[off..off + es]);
            let raw = u128::from_le_bytes(w);
            // ⊘ The sysmem census is taken from the DECODE, not from the relocator, because
            // the relocator's answer for a sysmem edge is `Unchanged` — the same answer it
            // gives a leaf — and the two are different facts.
            if let kayfabe_arch::PteDecode::Pde { edge, also } = fmt.decode_entry(level, raw) {
                for e in [Some(edge), also].into_iter().flatten() {
                    if e.aperture != Aperture::Vidmem {
                        sysmem_edges += 1;
                    }
                }
            }
            match fmt.relocate_entry(level, raw, &home) {
                kayfabe_arch::Relocated::Unchanged => {}
                kayfabe_arch::Relocated::Moved(v) => {
                    page[off..off + es].copy_from_slice(&v.to_le_bytes()[..es]);
                }
                kayfabe_arch::Relocated::Refused(why) => {
                    return Err(ShadowRefusal::Relocate(why))
                }
            }
        }
    }

    let mut roots = Vec::with_capacity(vases.len());
    for (pdb, _) in vases {
        let base = pdb & !0xfff;
        let Some(&h) = home_of.get(&base) else {
            return Err(ShadowRefusal::RootMissing { pdb: *pdb });
        };
        roots.push((*pdb, h));
    }
    // ★ The kernel refuses an unsorted pdb list by name (`KFWR_R_PDB_UNSORTED`), and two
    // address spaces may legitimately share a root page.
    roots.sort_by_key(|(_, h)| *h);
    roots.dedup_by_key(|(_, h)| *h);

    Ok(ShadowImage {
        bytes,
        roots,
        pages: level_of.len(),
        absent_edges: absent.get(),
        sysmem_edges,
    })
}
