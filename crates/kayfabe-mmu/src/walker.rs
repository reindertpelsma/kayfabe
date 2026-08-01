//! # The page-table decoder — `#102` stage C3
//!
//! The walk *algorithm* is core (one loop, all regimes); the entry *format* is Axis-B
//! ([`kayfabe_arch::GmmuFmt`]). This module is the algorithm, and nothing in it names a
//! bit position, a page size or a level number: it asks the format for the geometry
//! ([`kayfabe_arch::GmmuFmt::level_shift`]) and for the meaning of an entry
//! ([`kayfabe_arch::GmmuFmt::decode_entry`]), and turns the pair into *(virtual address
//! → backing)* facts the address table can be forward-populated with.
//!
//! ## The three laws it is written to, all of them load-bearing
//!
//! 1. ★★★ **MISS = FAULT, never a guess.** [`FbRead`] answering `false` is a *loud*
//!    [`WalkFault::Unbacked`], not zeros. The C's `nvkvm_pt_rd64` returns `0` for an
//!    unreadable page (`C: nvkvm_gpu_emul.c:4891-4904`), which decodes as `Invalid` and
//!    is therefore indistinguishable from a genuinely empty slot — a page we could not
//!    read is *forwarded*, never guessed into a capture. Nothing here ever resolves a
//!    physical address backwards into a virtual one either: the table is
//!    forward-populated and this is the thing that populates it.
//!
//! 2. ★★★ **Decode is not policy.** The 512 MiB leaf that #13 was about must
//!    **decode faithfully here** and be dropped **by policy** at the binding site
//!    ([`leaf_disposition`]). The C does exactly this in three places — `walk_pdb_root`
//!    *resolves* it (`C: :4949`), `pt_enum` *skips* it (`C: :8649`) and
//!    `cpt_decode_page` *skips* it (`C: :8733`) — and both skips carry the same reason,
//!    *"its only known producer is the CeUtils whole-FB identity alias"*. That is a
//!    statement about a **producer**, not a property of a big leaf. Collapsing the two
//!    halves into one is how #13's round-4 silent drop was built.
//!
//! 3. ★★ **Every leaf size the regime enumerates decodes; one it does not is a LOUD
//!    fault** ([`WalkFault::UnknownLeafSize`]). #13's corollary L3, restated as a check
//!    rather than as a comment.
//!
//! ## Where the bytes come from
//!
//! [`FbRead`] is the seam and it has one production implementation
//! (`kayfabe_fwd::IsolateFb`), over the **isolate's mapping of the fabricated aperture**
//! (`eight_blockers_resolved.md` §12.2). The core holds the address table and decides
//! *what*; the isolate holds bytes and does *it*. Nothing in these crates ever stores
//! page content — the rejected Option 3 (§11.6) is unrepresentable here, because
//! [`FbRead`] has no method that hands content *in*.

use kayfabe_arch::ids::{GpuVa, Pdb};
use kayfabe_arch::{Aperture, GmmuFmt, PageSize, PteDecode};

use crate::{AddressFault, AddressTable, Binding};

/// Abstract page-table byte source (a synthetic FB image in tests; the isolate's
/// fabricated-aperture mapping in production). Keeps the walker pure.
pub trait FbRead {
    /// Read `buf.len()` bytes at physical address `phys`; `false` if unbacked.
    ///
    /// ★ **`&mut self`, and that is not incidental.** The production implementation is a
    /// *connection*, not an image: every read is a round trip to the isolate that holds
    /// the aperture. Typing it `&self` would force interior mutability into the one
    /// place the whole design is trying to keep honest, and would hide from every caller
    /// that this call **blocks** — which is exactly the fact R1 needs visible, since a
    /// decode pass must not run under a ranked lock.
    ///
    /// ★★ `false` means **this source cannot serve the range**, and callers turn it into
    /// [`WalkFault::Unbacked`]. It must never be spelled as "zeros": a zero-filled page
    /// decodes as a full page of invalid entries, which is a page that legitimately maps
    /// nothing, and the two are opposite facts.
    fn read(&mut self, phys: u64, buf: &mut [u8]) -> bool;
}

/// Result of a (future) walk. Placeholder shape for the milestone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkResult {
    /// VA resolves to a leaf.
    Mapped {
        /// Physical address of the page.
        phys: u64,
    },
    /// The walk hit an invalid entry — a loud fault at the commit point.
    Fault {
        /// Directory level (0 = root) where the walk faulted.
        level: u8,
        /// The PDB being walked.
        pdb: Pdb,
    },
}

/// ★★★ **One page-table page, and everything needed to decode it.**
///
/// The C's `m2_cpt` entry (`C: nvkvm_gpu_emul.c:8714-8720`) minus the ownership fields,
/// which live one layer up. `level` and `vabase` are **not** derivable from `phys`: a
/// page of eight-byte words is the same bytes at every level, and what it *means* depends
/// entirely on where in the tree it hangs. That is why they travel with it.
///
/// ★ Where the metadata comes from is the thing §11.2 named as the hard part: it is
/// **forward-populated, from the root down.** Level 0 is a declared fact — a PDB *is*
/// its own root page — and every [`decode_page`] then hands each child its level and its
/// `vabase`. Nothing reverse-derives it, and nothing sweeps for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PtPage {
    /// Physical address of the table page.
    pub phys: u64,
    /// Which aperture `phys` is in.
    pub aperture: Aperture,
    /// The level this page sits at, in the format's own numbering (0 = root).
    pub level: u8,
    /// The virtual address the page's entry 0 describes.
    pub vabase: u64,
}

/// One leaf mapping recovered from a page-table page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedLeaf {
    /// The virtual address it maps.
    pub va: GpuVa,
    /// What it maps to.
    pub phys: u64,
    /// Which aperture `phys` is in.
    pub aperture: Aperture,
    /// How many bytes it maps.
    pub size: PageSize,
    /// The guest's read-only bit, carried so a binding cannot silently widen rights.
    pub read_only: bool,
    /// ★ The level the leaf was found at. Carried because a leaf's provenance is what
    /// makes a diagnostic about it actionable, and because a leaf that has forgotten
    /// where it came from cannot be asked where it came from.
    pub level: u8,
}

/// What one page-table page turned out to contain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageDecode {
    /// Sub-tables this page points at, each already carrying its own level and `vabase`.
    ///
    /// ★ A **dual** slot contributes **two** entries here with the same `vabase`, because
    /// one 16-byte slot names two sub-tables ([`PteDecode::Pde::also`]). A consumer that
    /// assumes one child per slot is `#13`'s silent drop one level up the tree.
    pub children: Vec<PtPage>,
    /// Leaf mappings this page defines. **Includes** the ones policy will drop.
    pub leaves: Vec<DecodedLeaf>,
    /// ★★ Virtual addresses of slots the guest declared **SPARSE**, ascending.
    ///
    /// Kept as addresses rather than as a count, and kept apart from
    /// [`PageDecode::invalid`], because sparse is a *declaration* and invalid is an
    /// absence (`reachability_on_transition.md` §3.6). A shadow that cannot tell them
    /// apart cannot tell valid→sparse (an unmap) from invalid→sparse (nothing).
    pub sparse: Vec<u64>,
    /// Slots that decoded to [`PteDecode::Invalid`] — a page that maps nothing there.
    /// Counted rather than discarded so "this page was empty" and "this page was never
    /// read" are different observations.
    pub invalid: usize,
}

/// Why a decode refused. Every variant is LOUD; none has a fallback value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkFault {
    /// ★★★ The byte source could not serve this page — **MISS, and a miss is a FAULT**.
    /// The subtree under it is not decoded and nothing is bound for it. It is forwarded,
    /// never guessed.
    Unbacked {
        /// The page that could not be read.
        phys: u64,
        /// The level it was believed to be at.
        level: u8,
    },
    /// The format has no such level. An un-enumerated level is a fault, never a stride
    /// this crate invents (see [`kayfabe_arch::GmmuFmt::level_shift`]).
    NoSuchLevel {
        /// The level asked for.
        level: u8,
    },
    /// The level's geometry is not one a table page can hold — a zero-width entry, an
    /// entry wider than [`kayfabe_arch::GmmuFmt::decode_entry`]'s `u128` can carry, a
    /// zero entry count, an impossible stride, or a count × width that overflows. Guest
    /// input never reaches this (it is a claim the *format* made), but a bad adapter must
    /// fail loudly rather than read out of bounds of its own arithmetic.
    BadGeometry {
        /// The level whose geometry was refused.
        level: u8,
    },
    /// ★★ A leaf claiming a size the regime does not enumerate (#13's corollary L3).
    /// Never rounded to the nearest real size, never dropped quietly.
    UnknownLeafSize {
        /// The virtual address of the offending leaf.
        va: u64,
        /// The size it claimed.
        size: PageSize,
    },
    /// A slot's virtual address does not fit in 64 bits at this level's stride — a
    /// format claiming, e.g., 512 entries at shift 60.
    VaOverflow {
        /// The level whose arithmetic overflowed.
        level: u8,
    },
    /// The descent's entry budget ran out. Loud, because the alternative is a partial
    /// capture presented as a complete one — the C bounds the same walk with
    /// `budget = 300000` (`C: nvkvm_gpu_emul.c:8759`) and simply stops.
    BudgetExhausted,
    /// The descent hit its depth limit. A page pointing at itself is a guest-writable
    /// cycle, and a decoder that follows it never returns.
    TooDeep {
        /// The page the descent refused to follow.
        phys: u64,
    },
}

/// The largest entry width [`decode_page`] will read into a `u128`.
const MAX_ENTRY_BYTES: u8 = 16;

/// ★★★ **Decode ONE page-table page** — the primitive `#102` stage C3 exists to build.
///
/// Reads the whole table in a single [`FbRead::read`] (one round trip to the aperture,
/// not one per entry), then decodes every slot through the format. Returns the page's
/// children and its leaves; **descends into nothing** — [`decode_subtree`] is the
/// descending caller, and keeping them apart is what makes the budget and the
/// cycle-refusal expressible at all.
///
/// ★ It is the *direct* decode #13's fix rests on: *"decode each dirtied page DIRECTLY —
/// from the page itself, NOT a root walk … the guest fills a leaf PT page and links it
/// under the root a SEPARATE push later, so at this release a root walk of the PDB can't
/// yet reach the page, but the page itself already holds committed PTEs"*
/// (`C: nvkvm_gpu_emul.c:8676-8690`).
///
/// # Errors
/// Any [`WalkFault`]. In particular [`WalkFault::Unbacked`] when the source cannot serve
/// the page — which is a fault and not an empty page.
pub fn decode_page(
    fmt: &dyn GmmuFmt,
    fb: &mut dyn FbRead,
    page: PtPage,
) -> Result<PageDecode, WalkFault> {
    let geo = fmt
        .level_shift(page.level)
        .ok_or(WalkFault::NoSuchLevel { level: page.level })?;
    let width = fmt.entry_size(page.level);
    if width == 0 || width > MAX_ENTRY_BYTES || geo.entries == 0 || geo.shift >= 64 {
        return Err(WalkFault::BadGeometry { level: page.level });
    }
    let bytes = u64::from(geo.entries)
        .checked_mul(u64::from(width))
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(WalkFault::BadGeometry { level: page.level })?;

    let mut image = vec![0u8; bytes];
    if !fb.read(page.phys, &mut image) {
        return Err(WalkFault::Unbacked {
            phys: page.phys,
            level: page.level,
        });
    }

    let sizes = fmt.page_sizes();
    let mut out = PageDecode::default();
    for i in 0..geo.entries {
        let at = (i as usize) * (width as usize);
        let mut raw = [0u8; MAX_ENTRY_BYTES as usize];
        raw[..width as usize].copy_from_slice(&image[at..at + width as usize]);
        // Slot `i` of a table at this level describes `vabase | (i << shift)`. Checked,
        // because `shift` and `entries` are both the format's claims and a format that
        // claims 512 entries at shift 60 must fault rather than wrap into VA 0.
        let index_va = u64::from(i)
            .checked_shl(u32::from(geo.shift))
            .filter(|v| v >> geo.shift == u64::from(i))
            .ok_or(WalkFault::VaOverflow { level: page.level })?;
        let va = page.vabase | index_va;

        match fmt.decode_entry(page.level, u128::from_le_bytes(raw)) {
            PteDecode::Invalid => out.invalid += 1,
            // ★★ A third state, carried as a third state. See [`PageDecode::sparse`].
            PteDecode::Sparse => out.sparse.push(va),
            PteDecode::Pde { edge, also } => {
                // A null sub-table pointer is not a sub-table. The C guards every descent
                // with the same truthiness (`C: :8615`, `:8623`, `:8636`).
                //
                // ★★ BOTH halves of a dual slot are followed. Taking `edge` and dropping
                // `also` would lose a whole sub-tree with no diagnostic — the shape #13
                // was, at the 16-byte level.
                for e in [Some(edge), also].into_iter().flatten() {
                    if e.next != 0 {
                        out.children.push(PtPage {
                            phys: e.next,
                            aperture: e.aperture,
                            level: e.child_level,
                            vabase: va,
                        });
                    }
                }
            }
            PteDecode::Leaf {
                phys,
                aperture,
                size,
                read_only,
            } => {
                // ★★ #13's corollary L3: a leaf whose size the regime does not enumerate
                // is a LOUD fault. Not clamped, not skipped — the whole cost of #13 was
                // an un-enumerated leaf being treated as "not a mapping".
                if !sizes.contains(&size) {
                    return Err(WalkFault::UnknownLeafSize { va, size });
                }
                out.leaves.push(DecodedLeaf {
                    va: GpuVa(va),
                    phys,
                    aperture,
                    size,
                    read_only,
                    level: page.level,
                });
            }
        }
    }
    Ok(out)
}

/// Maximum descent depth. The format's own level count bounds a *legal* tree; this bounds
/// an **illegal** one, because the page a PDE points at is guest-written and may point
/// back at its own parent.
pub const MAX_WALK_DEPTH: u8 = 16;

/// What a bounded descent produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubtreeDecode {
    /// Every leaf found, in ascending virtual-address order within each page and in
    /// depth-first order across pages. **Includes** the ones policy will drop.
    pub leaves: Vec<DecodedLeaf>,
    /// Every page actually decoded, with the level and `vabase` it was decoded at — the
    /// forward-populated metadata chain, for the caller to remember so that a later
    /// *direct* decode of one of these pages knows what it is.
    pub visited: Vec<PtPage>,
    /// ★★★ **Each visited page beside WHAT IT CONTAINED**, in visit order.
    ///
    /// [`SubtreeDecode::leaves`] is these pages' leaves flattened, and flattening is
    /// lossy in the one way `reachability_on_transition.md` needs: it forgets **which
    /// page** a leaf came out of, and therefore whether that page was witnessed and
    /// whether it is reachable. It also forgets the edges and the sparse slots entirely.
    /// A reachability shadow is a statement about pages, so it is fed from here.
    ///
    /// Kept **alongside** the flattened views rather than replacing them: a consumer that
    /// only wants "every leaf under this root" should not have to re-flatten, and the two
    /// cannot disagree because one is built from the other.
    pub decodes: Vec<(PtPage, PageDecode)>,
    /// ★★★ Branches that could not be decoded, **kept and returned rather than
    /// absorbed**. A subtree that faulted contributes no leaves and is not silently a
    /// subtree with no mappings.
    pub faults: Vec<WalkFault>,
    /// Slots that decoded to nothing, summed over every page visited.
    pub invalid: usize,
}

/// ★★ **Descend from `root`, bounded** — the C's `nvkvm_m2_pt_enum` (`C: :8635`), with
/// its silent failure modes made loud.
///
/// `budget` counts **entries examined**, exactly as the C's does, so a sparse tree costs
/// what it contains rather than what it could contain.
///
/// ★ **A fault on one branch does not discard another branch's leaves.** The leaves that
/// decoded are real facts about memory the guest really did map, and throwing them away
/// because a sibling was unreadable would be #13's silent drop with a different cause.
/// The faults come back in [`SubtreeDecode::faults`] for the caller to surface; they are
/// never turned into zeros.
///
/// # Errors
/// [`WalkFault::BudgetExhausted`] — and only that. Everything else is per-branch and
/// lands in [`SubtreeDecode::faults`], because the budget is the one condition under
/// which the *whole* result is untrustworthy.
pub fn decode_subtree(
    fmt: &dyn GmmuFmt,
    fb: &mut dyn FbRead,
    root: PtPage,
    budget: u32,
) -> Result<SubtreeDecode, WalkFault> {
    let mut out = SubtreeDecode::default();
    let mut left = budget;
    // Explicit stack: the depth bound is the guard against a guest-built cycle, and a
    // recursive form would exhaust the host stack before it reached the bound.
    let mut stack: Vec<(PtPage, u8)> = vec![(root, 0)];
    while let Some((page, depth)) = stack.pop() {
        if depth >= MAX_WALK_DEPTH {
            out.faults.push(WalkFault::TooDeep { phys: page.phys });
            continue;
        }
        let cost = fmt.level_shift(page.level).map_or(1, |g| g.entries);
        if left < cost {
            return Err(WalkFault::BudgetExhausted);
        }
        left -= cost;
        match decode_page(fmt, fb, page) {
            Ok(d) => {
                out.visited.push(page);
                out.invalid += d.invalid;
                out.leaves.extend(d.leaves.iter().copied());
                // Pushed in reverse so the pop order is ascending in virtual address — a
                // deterministic visit order is what makes a `leaves` comparison in a test
                // an assertion rather than a coin flip.
                for c in d.children.iter().rev() {
                    stack.push((*c, depth + 1));
                }
                out.decodes.push((page, d));
            }
            Err(e) => out.faults.push(e),
        }
    }
    Ok(out)
}

/// Why the binding site refused a leaf the walker decoded correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// ★★★ **The whole-framebuffer identity alias.**
    ///
    /// The guest kernel-RM's copy-engine utility identity-maps the *entire* framebuffer
    /// heap into its own address space **at the regime's largest page size** and then
    /// issues its page-table writes as virtual-destination copies through it
    /// (`C: nvkvm_gpu_emul.c:4936-4952`). The walker must resolve that mapping — not
    /// resolving it is literally what #13 was (*"chan_execute silently DROPPED every such
    /// PT write"*) — and the binding site must not back it, because an alias of the whole
    /// heap is not a compute working set (`C: :8649`, `:8733`, same reason at both).
    WholeFbIdentityAlias,
}

/// What the binding site does with one decoded leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafDisposition {
    /// Forward-populate the address table with it.
    Bind,
    /// Decoded faithfully, not bound — with the reason.
    Drop(DropReason),
}

/// ★★★ **THE POLICY, and it is the ONLY thing in this module that is one.**
///
/// [`decode_page`] resolves every leaf the format can express, including the alias; this
/// says which of them the address plane should stand behind. The separation is §11.7's
/// standing instruction, re-verified against all three C sites this stage: `walk_pdb_root`
/// **resolves** the 512 MiB leaf (`C: :4949`), while `pt_enum` (`C: :8649`) and
/// `cpt_decode_page` (`C: :8733`) **skip** it — and the two skips give a reason about its
/// *producer*, not about its size class.
///
/// ★ The predicate is *"this leaf maps the regime's largest enumerated page size"*,
/// because that is the C's own identification of the alias — *"identity-maps the WHOLE FB
/// heap into its own VAS at the largest page size"* (`C: :4941-4944`) — rather than a
/// hard-coded 512 MiB, which would be one chip's number sitting in a logic crate. A
/// regime that grows a larger leaf moves this predicate with it, which is the correct
/// coupling: the alias is defined as *the biggest thing this MMU can map*, not as a
/// constant.
///
/// ★★ It deliberately does **not** consult the address table, the aperture, or anything
/// about the destination. A policy that varied with what we happened to have published
/// would make the drop unpredictable from the guest's own actions, and this drop has to
/// be explainable to a person reading a trace.
#[must_use]
pub fn leaf_disposition(fmt: &dyn GmmuFmt, leaf: &DecodedLeaf) -> LeafDisposition {
    match fmt.page_sizes().iter().max() {
        Some(&largest) if leaf.size == largest => {
            LeafDisposition::Drop(DropReason::WholeFbIdentityAlias)
        }
        _ => LeafDisposition::Bind,
    }
}

/// Why one leaf could not be forward-populated into the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopulateRefusal {
    /// The table refused the bind. Carries the table's own fault verbatim.
    Refused {
        /// The leaf's virtual address.
        va: GpuVa,
        /// What the table said.
        fault: AddressFault,
    },
    /// ★★★ The leaf re-points a range that is **host-published**.
    ///
    /// Rebinding it here would drop a live [`crate::HostBacking`] on the floor: the host
    /// object would still be allocated and still mapped into that `Vas`'s host VAS, and
    /// no core state would name it any more. That is the G1 defect, and it is worse than
    /// G1 because the mapping stays *live* — hardware would keep resolving the old page.
    /// Unpublishing needs a worker and an unmap verb, i.e. the forwarding plane, so the
    /// table says so and refuses rather than guessing.
    RepointsPublished {
        /// The leaf's virtual address.
        va: GpuVa,
        /// What it now claims to map.
        phys: u64,
    },
    /// The leaf's range overlaps a *differently shaped* live binding — it starts inside
    /// one, or is a different length. Loud, because silently resizing a neighbour is how
    /// the `ALREADY-MAPPED` collision class hides.
    StraddlesLiveBinding {
        /// The leaf's virtual address.
        va: GpuVa,
    },
    /// ★★★ **An UNBIND of a range that is host-published**
    /// (`reachability_on_transition.md` §6).
    ///
    /// The mirror image of [`PopulateRefusal::RepointsPublished`], and it gets its own
    /// variant because it is a different act with the same consequence: dropping the
    /// range from the table would leave the host object still allocated and still mapped
    /// into that address space's host VAS, with **no core state naming it**. That is
    /// worse than a leak — hardware would keep resolving it — and it is what a teardown
    /// (`reachability_on_transition.md` §3.3) would do to a published range if the
    /// unbind were performed rather than refused.
    ///
    /// Unpublishing needs a worker and an unmap verb, i.e. the forwarding plane. So the
    /// refusal is the answer, and the binding stays.
    UnbindsPublished {
        /// The virtual address that would have been dropped.
        va: GpuVa,
    },
}

/// What forward-populating a set of leaves did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PopulateOutcome {
    /// Leaves bound into a range that was free.
    pub bound: usize,
    /// Leaves that named exactly what the table already said. Counted, not re-bound —
    /// re-binding would be a loud overlap for a fact that has not changed.
    pub unchanged: usize,
    /// Leaves that re-pointed an existing, **unpublished** binding.
    pub repointed: usize,
    /// ★★ Leaves the walker decoded and [`leaf_disposition`] dropped, with the reason.
    /// Recorded rather than counted: *"we saw it and declined it"* is the assertion that
    /// separates this stage's two halves, and a bare counter cannot carry the reason.
    pub dropped: Vec<(GpuVa, DropReason)>,
    /// Leaves that could not be applied. Loud; the caller propagates.
    pub refusals: Vec<PopulateRefusal>,
}

/// ★★★ **Forward-populate `table` from decoded leaves** — the join between the decoder
/// and the address table, and the site [`leaf_disposition`] runs at.
///
/// Every leaf arrives as a *declaration*: [`Binding::host`] is `None`, because a decode
/// says what the guest's own page tables now claim and says nothing about whether host
/// memory exists behind it. Publication is a separate verb on the forwarding plane, and
/// keeping them separate is what lets `Fabricated` and `HostBacked` be different answers
/// to the representability question at all.
///
/// ★ It is **total and non-throwing**: every leaf produces exactly one outcome, and a
/// refusal on one does not abandon the rest. A populate that stopped at the first
/// conflicting leaf would drop the tail of a legitimate remap — `#13 CE-DROP` rebuilt at
/// a different layer.
pub fn populate(
    fmt: &dyn GmmuFmt,
    table: &mut AddressTable,
    pdb: Pdb,
    leaves: &[DecodedLeaf],
) -> PopulateOutcome {
    let mut out = PopulateOutcome::default();
    for leaf in leaves {
        if let LeafDisposition::Drop(why) = leaf_disposition(fmt, leaf) {
            out.dropped.push((leaf.va, why));
            continue;
        }
        let want = Binding {
            phys: leaf.phys,
            aperture: leaf.aperture,
            host: None,
        };
        match table.binding_at(leaf.va) {
            // Exactly this range is already bound.
            Some((start, len, have)) if start == leaf.va.0 && len == leaf.size.0 => {
                // ★★ The comparison is over the DECLARATION — `(phys, aperture)` — and
                // deliberately not over the whole [`Binding`]. A decode says what the
                // guest's page tables claim; it says nothing at all about whether host
                // memory has been published behind that claim, and [`Binding::host`] is
                // the answer to that second question. Comparing the whole value would
                // make every re-decode of an already-published range look like a re-point
                // and refuse it — the guest would map a buffer, we would back it, and the
                // next page-table write in the same address space would start failing.
                if (have.phys, have.aperture) == (want.phys, want.aperture) {
                    out.unchanged += 1;
                } else if have.host.is_some() {
                    out.refusals.push(PopulateRefusal::RepointsPublished {
                        va: leaf.va,
                        phys: leaf.phys,
                    });
                } else {
                    // Unmap eager, then re-bind: the table's own discipline, and the
                    // reason a re-point is not an "insert over".
                    table.unbind(leaf.va);
                    match table.bind(pdb, leaf.va, leaf.size.0, want) {
                        Ok(()) => out.repointed += 1,
                        Err(fault) => out
                            .refusals
                            .push(PopulateRefusal::Refused { va: leaf.va, fault }),
                    }
                }
            }
            // Something else lives here, shaped differently.
            Some(_) => out
                .refusals
                .push(PopulateRefusal::StraddlesLiveBinding { va: leaf.va }),
            // Nothing at the start VA — but the range may still run into a neighbour,
            // which `bind` refuses as an overlap. That refusal is the answer; pre-checking
            // it here would be the same question asked twice, in two places that can drift.
            None => match table.bind(pdb, leaf.va, leaf.size.0, want) {
                Ok(()) => out.bound += 1,
                Err(fault) => out
                    .refusals
                    .push(PopulateRefusal::Refused { va: leaf.va, fault }),
            },
        }
    }
    out
}
