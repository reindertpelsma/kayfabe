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

/// ★★★★★ **THE KERNEL'S RUNS, REDUCED TO THE FIELDS BOTH WALKERS DECODE, THEN CANONICALISED
/// — and the order of those two steps is the whole function.**
///
/// ⊘⊘⊘ `[measured w731, live boot, vast 51067717]` this is what its absence looked like:
///
/// ```text
/// HOST[0] va=0x120000000 gpga=0x0        len=0x100000000 flags=0x0
/// KERN[0] va=0x120000000 gpga=0x0        len=0xefc00000  flags=0x90200
/// KERN[1] va=0x20fc00000 gpga=0xefc00000 len=0x10400000  flags=0x60200
/// ```
///
/// One 4 GiB mapping on the host; **two** on the kernel, contiguous in VA and in GPGA, and
/// **identical in every flag either side is allowed to compare** (`0x90200 & 0xf == 0` and
/// `0x60200 & 0xf == 0`). The bits that differ are ones the host walker **does not decode at
/// all** — this is w725's finding arriving from the other side: *"runs if KIND/COMPTAGLINE
/// were part of the identity: 3"*.
///
/// ⚠ [`walkdiff::canonical`] coalesces on **equal flags**, so canonicalising the raw runs
/// preserves a boundary that exists only in fields outside [`COMPARED_FLAGS`]. The census
/// then reported `len_differs` + `extra_in_kernel` for a mapping the two sides **agree**
/// about: **100 disagreements over 65 comparisons, none of them real.**
///
/// ⇒ **Mask first, then coalesce.** Masking afterwards cannot help: the boundary is already
/// baked into the run list by then.
///
/// ★ And this is not a narrowing bought to make a number green — it is [`COMPARED_FLAGS`]'s
/// existing rule applied one step earlier. Comparing runs cut by fields only one walker reads
/// measures a difference between what two decoders *looked at*, which the module header
/// already refuses in the comparison and had failed to refuse in the canonicalisation.
#[must_use]
pub fn kernel_runs_as_compared(runs: &[Run]) -> Vec<Run> {
    let masked: Vec<Run> = runs
        .iter()
        .map(|r| Run {
            flags: r.flags & COMPARED_FLAGS,
            ..*r
        })
        .collect();
    walkdiff::canonical(&masked)
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
    /// ★★★ **Sweeps the shadow declined to run, by name.** ⊘ Not folded into
    /// [`Self::kernel_unavailable`]: *"this sweep is on a vCPU thread"*, *"the image was too
    /// big"* and *"the isolate refused"* send a reader to three different places, and a
    /// single number sends them nowhere. A boot whose census is empty has to be able to say
    /// **why** it is empty.
    pub skipped: std::collections::BTreeMap<&'static str, u64>,
    /// Table pages in the largest image built this boot.
    pub image_pages_max: u64,
    /// Bytes staged across the whole boot — what the shadow cost the wire.
    pub staged_bytes: u64,
    /// Directory edges sent to the reserved absent slot, summed. ★ The measured size of the
    /// **reach clipping**: the kernel is given the pages the host visited and no others.
    pub absent_edges: u64,
    /// Directory edges naming a sysmem sub-table, summed. The kernel refuses those by name;
    /// the host walk follows them. ⚠ Predicts `missing_in_kernel` that is a scope difference.
    pub sysmem_edges: u64,
    /// ★★★★★ **§6 STEP 2 — address spaces whose published leaves came from the KERNEL.**
    ///
    /// ⊘ Zero on a shadow-arm boot by construction, and zero on a swap-arm boot means the
    /// swap **never decided anything**: read [`Self::fell_back`] before reading
    /// [`Self::by_kind`], because a census with no disagreements and no decisions is a
    /// census of the old path.
    pub decided: u64,
    /// ★★★ Address spaces the swap arm **refused to decide**, by name — every one of them a
    /// fall back to the host walk. See [`SwapRefusal`].
    pub fell_back: std::collections::BTreeMap<&'static str, u64>,
    /// Whether the swap arm was selected at all. ⊘ Carried in the census rather than inferred
    /// from [`Self::decided`] being zero: *"the arm was off"* and *"the arm was on and refused
    /// every time"* are different boots and a reader must not have to guess which.
    pub swap_armed: bool,
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

    /// A sweep the shadow declined, named. ⊘ Counted **and** counted as unavailable, because
    /// a declined sweep is one on which the two walkers were not compared and the vacuity
    /// arms must see it as such.
    pub fn note_skipped(&mut self, why: &'static str) {
        *self.skipped.entry(why).or_insert(0) += 1;
        self.kernel_unavailable += 1;
    }

    /// ★★★ One address space whose leaves the kernel decided.
    pub fn note_decided(&mut self) {
        self.decided += 1;
    }

    /// ★★★ One address space the swap refused to decide, named. ⊘ Named and not counted in
    /// one number: *"they disagreed"*, *"two address spaces share a root"* and *"the host
    /// walk produced an unclassed leaf"* send a reader to three different places.
    pub fn note_fell_back(&mut self, why: &'static str) {
        *self.fell_back.entry(why).or_insert(0) += 1;
    }

    /// Arm the swap column, so a zero can be told from an absence.
    pub fn note_swap_armed(&mut self) {
        self.swap_armed = true;
    }

    /// What one image cost and what it clipped.
    pub fn note_image(&mut self, pages: u64, staged: u64, absent: u64, sysmem: u64) {
        self.image_pages_max = self.image_pages_max.max(pages);
        self.staged_bytes += staged;
        self.absent_edges += absent;
        self.sysmem_edges += sysmem;
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
        let skipped = if self.skipped.is_empty() {
            "none".to_string()
        } else {
            self.skipped
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let fell = if self.fell_back.is_empty() {
            "none".to_string()
        } else {
            self.fell_back
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        // ★★★★★ **THE SWAP'S OWN VERDICT, AND IT IS SEPARATE FROM THE AGREEMENT VERDICT.**
        //
        // ⊘ An agreeing census does not say the swap ran. `[the shape this tree keeps
        // paying for]` a boot whose shadow agreed 65 times and whose swap decided **zero**
        // times would print `★★★ AGREEMENT` and be read as the swap being proven — while
        // every published leaf came from the host walk exactly as before. The two sentences
        // are therefore printed side by side, and the swap's says `⊘⊘ VACUOUS` when the arm
        // was on and nothing was decided.
        let swap = if !self.swap_armed {
            "⊘ SWAP DISARMED — the arm was `on` (shadow), so the kernel decided nothing and \
             was never on the publish path. This census is about AGREEMENT only."
                .to_string()
        } else if self.decided == 0 {
            "⊘⊘ SWAP VACUOUS — the arm was `swap` and the kernel decided NOTHING. Every \
             published leaf came from the host walk. Read `fell_back[…]`: it names why, and \
             an empty one means the shadow never even ran (read `skipped[…]`)."
                .to_string()
        } else {
            format!(
                "★★★ SWAP LIVE — {} address spaces had their published leaves' target, \
                 aperture and writability taken from the KERNEL's report; {} fell back to \
                 the host walk. ⊘ SCOPE: the page structure (`visited`, `children`, \
                 `sparse`, `invalid`) is still the HOST walk's and cannot move until the \
                 kernel reports pages — so this does NOT license §7's deletion.",
                self.decided,
                self.fell_back.values().sum::<u64>()
            )
        };
        format!(
            "WALK-SHADOW compared={} kernel_unavailable={} skipped[{}] host_runs={} \
             kernel_runs={} host_unclassed={} compared_flags={:#x} image[pages_max={} \
             staged_bytes={} absent_edges={} sysmem_edges={}] disagreements={} by_kind[{}] \
             first[{}] swap_armed={} decided={} fell_back[{}] ⇒ {} ⇒ {} ⊘ SCOPE: the kernel \
             walks a RELOCATED COPY holding exactly the pages the host walk visited (the \
             guest's tables sit ~11.8 GiB up and no device buffer can span that), so \
             `missing_in_kernel` is fully live and `extra_in_kernel` is live only WITHIN \
             those pages.",
            self.compared,
            self.kernel_unavailable,
            skipped,
            self.host_runs,
            self.kernel_runs,
            self.host_unclassed,
            COMPARED_FLAGS,
            self.image_pages_max,
            self.staged_bytes,
            self.absent_edges,
            self.sysmem_edges,
            total,
            by,
            first,
            self.swap_armed,
            self.decided,
            fell,
            verdict,
            swap
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
    use std::collections::{BTreeMap, BTreeSet};

    // ★★★★★ **A TABLE IS NOT A PAGE, AND CONFLATING THEM LOSES WHOLE SUBTREES.**
    //
    // `[measured w731, live boot]` a VER2 **big-page table is 32 entries — 256 BYTES** — and
    // the dual PDE's big half names it with `_ADDRESS_SHIFT = 8`, i.e. **256-byte
    // granularity**. `PD3` is smaller still: four entries, 32 bytes. So a table legitimately
    // sits at a **non-page-aligned** address, and keying the image by `phys & !0xfff` throws
    // that offset away: the edge that named it was then sent to the absent slot, the kernel
    // found nothing under that root, and the census reported `missing_in_kernel` for a
    // mapping the host had decoded perfectly (`absent_edges=53`, `missing_in_kernel=30`).
    //
    // ⇒ **tables** are keyed by their own address; **pages** are what the image holds; and
    // `home` preserves the offset within the page.
    let mut level_of: BTreeMap<u64, u8> = BTreeMap::new();
    for (_, visited) in vases {
        for p in *visited {
            if p.aperture != Aperture::Vidmem {
                // A table page in guest system memory is not in the framebuffer and the
                // kernel refuses it by name; it is deliberately not in the image, and the
                // directory entry naming it keeps its own encoding so that refusal fires.
                continue;
            }
            match level_of.get(&p.phys) {
                Some(l) if *l != p.level => {
                    return Err(ShadowRefusal::AmbiguousLevel { phys: p.phys })
                }
                _ => {
                    level_of.insert(p.phys, p.level);
                }
            }
        }
    }
    if level_of.is_empty() {
        return Err(ShadowRefusal::NoPages);
    }
    let pages: BTreeSet<u64> = level_of.keys().map(|a| a & !0xfff).collect();
    if pages.len() > page_cap {
        return Err(ShadowRefusal::TooManyPages {
            pages: pages.len(),
            cap: page_cap,
        });
    }

    // Slot 1 upwards; slot 0 stays zero and is where every unfollowable edge is sent.
    let home_of: BTreeMap<u64, u64> = pages
        .iter()
        .enumerate()
        .map(|(i, &base)| (base, ((i + 1) * IMAGE_PAGE) as u64))
        .collect();

    let mut bytes = vec![0u8; (pages.len() + 1) * IMAGE_PAGE];
    let absent = std::cell::Cell::new(0u64);
    // ⊘ **The offset within the page is preserved**, because a sub-page table's address is
    // not its page's address. See the block above for the boot this cost.
    let home = |old: u64| -> Option<u64> {
        match home_of.get(&(old & !0xfff)) {
            Some(&h) => Some(h + (old & 0xfff)),
            None => {
                absent.set(absent.get() + 1);
                Some(ABSENT_SLOT)
            }
        }
    };

    // ⊘ Whole pages, not "as many bytes as the table at the base needs": one page can hold
    // sixteen big-page tables, and each of them is named independently.
    for (&base, &slot) in &home_of {
        let dst = slot as usize;
        if !fb.read_in(base, Aperture::Vidmem, &mut bytes[dst..dst + IMAGE_PAGE]) {
            return Err(ShadowRefusal::Unreadable { phys: base });
        }
    }

    let mut sysmem_edges = 0u64;
    for (&addr, &level) in &level_of {
        let es = usize::from(fmt.entry_size(level));
        let Some(geom) = fmt.level_shift(level) else {
            return Err(ShadowRefusal::BadGeometry { level });
        };
        if es == 0 || es > 16 {
            return Err(ShadowRefusal::BadGeometry { level });
        }
        let at = (home_of[&(addr & !0xfff)] + (addr & 0xfff)) as usize;
        let need = (geom.entries as usize).saturating_mul(es);
        for i in 0..geom.entries as usize {
            let off = at + i * es;
            if off + es > at + need || off + es > bytes.len() {
                break;
            }
            let mut w = [0u8; 16];
            w[..es].copy_from_slice(&bytes[off..off + es]);
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
                    bytes[off..off + es].copy_from_slice(&v.to_le_bytes()[..es]);
                }
                kayfabe_arch::Relocated::Refused(why) => {
                    return Err(ShadowRefusal::Relocate(why))
                }
            }
        }
    }

    let mut roots = Vec::with_capacity(vases.len());
    for (pdb, _) in vases {
        let Some(&h) = home_of.get(&(pdb & !0xfff)) else {
            return Err(ShadowRefusal::RootMissing { pdb: *pdb });
        };
        roots.push((*pdb, h + (pdb & 0xfff)));
    }
    // ★ The kernel refuses an unsorted pdb list by name (`KFWR_R_PDB_UNSORTED`), and two
    // address spaces may legitimately share a root.
    roots.sort_by_key(|(_, h)| *h);
    roots.dedup_by_key(|(_, h)| *h);

    Ok(ShadowImage {
        bytes,
        roots,
        pages: pages.len(),
        absent_edges: absent.get(),
        sysmem_edges,
    })
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// ★★★★★ THE SWAP — `SINGLE_STORE_PLAN.md` §6 step 2. The kernel DECIDES; the host FALLS BACK.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// ⊘⊘⊘ **WHAT THE SWAP CAN AND CANNOT MOVE, MEASURED FROM THE CONSUMERS — read this before
/// reading the census.**
///
/// §6's survey says the three halves of a [`crate::walker::SubtreeDecode`] are all consumed
/// and names the leaf path as *"`leaves` → `Settlement` → `AddressTable::bind`"*. ⊘ **That is
/// not the consumer.** `[read 2026-09-15, w732, from the bodies]`
/// `kayfabe_fwd::commit_pt_decode_with` — the only thing the sweep commits through — touches
/// `SubtreeDecode::leaves` **nowhere**. What reaches the address table is
/// `SubtreeDecode::decodes[*].1.leaves`, the **per-page** leaves, by way of
/// `ReachShadow::observe` → `settle` → `apply_settlement_as`. The flattened `leaves` field has
/// exactly three readers in the tree, and all three are elsewhere: `ceresolve`, the BAR
/// mirror's `window_leaves`, and this module's own comparison.
///
/// ⇒ **A swap that replaced only the flattened `leaves` would change nothing and would still
/// have read as done.** The substitution below therefore rewrites the leaves **inside each
/// page's decode**, which is where a bind comes from.
///
/// # ★★★ AND THE HOST WALK CANNOT LEAVE THE PATH YET — three reasons, all structural
///
/// 1. **The image is built from the host walk's `visited` set.** [`build_image`] takes
///    `(pdb, &[PtPage])` and needs each page's **level** to know its entry size and geometry.
///    The kernel cannot be pointed at anything until the host walk has said which pages exist
///    and what level each is. ⊘ This is a property of **relocation**, so it expires with §3 —
///    an identity window needs no page list.
/// 2. **The publish path is keyed on pages.** `children`, `sparse` and `invalid` are the
///    reachability vocabulary (`Admit::{Witnessed, Swept}`, `PublishedUnbind`), and the
///    kernel's report is `MapRun`s — *coalesced runs, with no pages in them at all*.
/// 3. **`DecodedLeaf::level` has no kernel counterpart.** It is kept from the host leaf the
///    kernel's value is written onto, which is exact because the match is on `(va, size)`.
///
/// ⇒ What this increment moves is the **provenance of every published leaf's target,
/// aperture and writability**. What it does not move is the page structure. Saying that
/// plainly is the point: a swap reported as "the kernel now walks" would be read as licensing
/// §7's deletion of the host walker, and it does not.
///
/// # ⊘⊘ AND THE SUBSTITUTION IS OBSERVATIONALLY NEUTRAL BY CONSTRUCTION, DELIBERATELY
///
/// It is applied **only when [`compare`] is empty**, and under agreement the two leaf sets are
/// the same set — so the bytes written are the bytes that were there. ⚠ That is not a reason
/// to skip it: the *dependency* moves, the fallback is what makes a future disagreement loud
/// at the moment it happens rather than at the next census, and
/// [`SwapRefusal::TargetChanged`] is the assertion that the neutrality held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwapRefusal {
    /// The two walkers disagreed. ⚠ **The loud one.** Under the swap this is the kernel
    /// deciding something the host walk decided differently, and the host wins.
    Disagreed(usize),
    /// The host walk produced leaves with no page-size class, so the set that was compared
    /// **lost members** and agreement over it does not cover the whole decode.
    HostUnclassed(usize),
    /// Several address spaces in this sweep share one root page, so the kernel's single
    /// answer for that root is the **union** of their leaves and cannot be split back.
    /// ⊘ `[w555]` *"11 of 12 VA spaces never declare a root, so they SHARE the key `Pdb(0)`"*.
    SharedRoot {
        /// How many tasks contributed to the root.
        tasks: usize,
    },
    /// A host leaf is at a virtual address no kernel run of its class covers. ⊘ Cannot
    /// happen after an empty [`compare`]; checked anyway, because the substitution's totality
    /// is the property that makes it safe and an unchecked invariant is not one.
    LeafUncovered {
        /// Where.
        va: u64,
    },
    /// The kernel covers a different number of bytes than the host's leaves place. ⊘ The
    /// other half of totality: *"every host leaf got a kernel value"* does not say *"every
    /// kernel byte was used"*.
    BytesUnplaced {
        /// What the kernel's runs cover.
        kernel: u64,
        /// What the host's distinct leaves cover.
        host: u64,
    },
    /// A kernel run carries an aperture code no [`Aperture`] decodes.
    BadAperture {
        /// The code.
        code: u32,
    },
    /// ★★★ The substitution would have **changed a published target** even though the two
    /// walkers agreed. ⊘ That is arithmetically impossible and therefore worth asserting:
    /// it can only mean the comparison and the substitution are keyed differently, which is
    /// the exact defect class (`a probe that shares the allocator is not an observer`) this
    /// tree keeps paying for.
    TargetChanged {
        /// Where.
        va: u64,
    },
}

impl SwapRefusal {
    /// A short name for a census column.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            SwapRefusal::Disagreed(_) => "disagreed",
            SwapRefusal::HostUnclassed(_) => "host_unclassed",
            SwapRefusal::SharedRoot { .. } => "shared_root",
            SwapRefusal::LeafUncovered { .. } => "leaf_uncovered",
            SwapRefusal::BytesUnplaced { .. } => "bytes_unplaced",
            SwapRefusal::BadAperture { .. } => "bad_aperture",
            SwapRefusal::TargetChanged { .. } => "target_changed",
        }
    }
}

/// One of ours for the kernel's aperture code. ⊘ `None` rather than a default: an
/// unrecognised code is a fact about the report, and filing it under vidmem would bind a
/// system-memory target into the framebuffer's aperture.
fn ap_of(code: u32) -> Option<Aperture> {
    match code {
        0 => Some(Aperture::Vidmem),
        1 => Some(Aperture::Peer),
        2 => Some(Aperture::SysmemCoherent),
        3 => Some(Aperture::SysmemNonCoherent),
        _ => None,
    }
}

/// What one kernel run says about a leaf inside it.
fn covering(kernel: &[Run], va: u64, class: PageClass) -> Option<&Run> {
    kernel
        .iter()
        .find(|r| r.class == class && r.va <= va && va - r.va < r.len)
}

/// ★★★★★ **REWRITE ONE ADDRESS SPACE'S DECODE SO ITS LEAVES CARRY THE KERNEL'S ANSWER.**
///
/// `host` is the comparison's host side and `kernel` its kernel side — both already
/// [`walkdiff::canonical`], both already passed through the same masking, and
/// [`compare`] must already have returned empty. `d` is the decode those leaves came from.
///
/// Returns a decode whose per-page leaves (and flattened `leaves`, kept consistent with them)
/// carry `phys`, `aperture` and `read_only` **derived from the kernel's runs**, with `va`,
/// `size` and `level` kept from the host leaf they were matched to.
///
/// # Errors
/// [`SwapRefusal`], naming what refused. ⊘ Every arm is a **fall back to the host walk**, and
/// the caller must say so out loud: a swap that quietly declined would be a boot that measured
/// the old path while its census said otherwise.
pub fn substitute(
    d: &crate::walker::SubtreeDecode,
    host: &[Run],
    kernel: &[Run],
    host_unclassed: usize,
) -> Result<crate::walker::SubtreeDecode, SwapRefusal> {
    if host_unclassed != 0 {
        return Err(SwapRefusal::HostUnclassed(host_unclassed));
    }
    // ⊘⊘ **BEFORE THE COMPARISON, NOT AFTER IT.** An aperture code no [`Aperture`] decodes is
    // a fact about the REPORT, and the comparison would file it under `flags_differ` — a
    // disagreement between two decoders — which sends a reader to the walkers when the
    // answer is that the report is malformed. ⚠ Checking it downstream also made the arm
    // **unreachable**, because `compare` fires first on the same bits; an arm that cannot be
    // reached is not a refusal, it is dead code that reads as one.
    for r in kernel {
        let code = (r.flags >> RF_AP_SHIFT) & RF_AP_MASK;
        if ap_of(code).is_none() {
            return Err(SwapRefusal::BadAperture { code });
        }
    }
    let disagreements = compare(host, kernel);
    if !disagreements.is_empty() {
        return Err(SwapRefusal::Disagreed(disagreements.len()));
    }

    // ⊘ **TOTALITY, BOTH WAYS.** `placed` is keyed by `(class, va)` so a leaf the host found
    // in two pages — a shape collision — counts its bytes once, which is what the kernel's
    // coalesced runs count.
    let mut placed: std::collections::BTreeSet<(PageClass, u64)> = std::collections::BTreeSet::new();
    let rewrite = |l: &DecodedLeaf| -> Result<DecodedLeaf, SwapRefusal> {
        let Some(class) = class_of(l.size.0) else {
            // Unreachable after the `host_unclassed` guard, and refused rather than passed
            // through: a leaf with no class was never in the comparison.
            return Err(SwapRefusal::LeafUncovered { va: l.va.0 });
        };
        let Some(r) = covering(kernel, l.va.0, class) else {
            return Err(SwapRefusal::LeafUncovered { va: l.va.0 });
        };
        let code = (r.flags >> RF_AP_SHIFT) & RF_AP_MASK;
        let Some(aperture) = ap_of(code) else {
            return Err(SwapRefusal::BadAperture { code });
        };
        let out = DecodedLeaf {
            va: l.va,
            phys: r.gpga + (l.va.0 - r.va),
            aperture,
            size: l.size,
            read_only: r.flags & RF_READ_ONLY != 0,
            level: l.level,
        };
        // ★★★ The neutrality assertion. See [`SwapRefusal::TargetChanged`].
        if out.phys != l.phys || out.aperture != l.aperture || out.read_only != l.read_only {
            return Err(SwapRefusal::TargetChanged { va: l.va.0 });
        }
        Ok(out)
    };

    let mut decodes = Vec::with_capacity(d.decodes.len());
    for (page, decode) in &d.decodes {
        let mut leaves = Vec::with_capacity(decode.leaves.len());
        for l in &decode.leaves {
            let out = rewrite(l)?;
            if let Some(class) = class_of(l.size.0) {
                placed.insert((class, l.va.0));
            }
            leaves.push(out);
        }
        decodes.push((
            *page,
            crate::walker::PageDecode {
                children: decode.children.clone(),
                leaves,
                sparse: decode.sparse.clone(),
                invalid: decode.invalid,
            },
        ));
    }
    let mut flat = Vec::with_capacity(d.leaves.len());
    for l in &d.leaves {
        flat.push(rewrite(l)?);
    }

    let kernel_bytes: u64 = kernel.iter().map(|r| r.len).sum();
    let host_bytes: u64 = placed.iter().map(|(c, _)| c.bytes()).sum();
    if kernel_bytes != host_bytes {
        return Err(SwapRefusal::BytesUnplaced {
            kernel: kernel_bytes,
            host: host_bytes,
        });
    }

    Ok(crate::walker::SubtreeDecode {
        leaves: flat,
        visited: d.visited.clone(),
        decodes,
        faults: d.faults.clone(),
        invalid: d.invalid,
    })
}
