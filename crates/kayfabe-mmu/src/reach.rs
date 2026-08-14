//! # The reachability shadow — `resume_from_fault.md` §7 **step 4**
//!
//! Full model, its citations and its residue: `reachability_on_transition.md`. This is the
//! implementation, and the two sentences it exists to make executable are:
//!
//! > **A leaf binds iff it is REACHABLE and WITNESSED.**
//! > **Transitions are a DIFF over page-table CONTENT, not a pattern match over stores.**
//!
//! ## Why content and not edges
//!
//! The owner's *trap-on-transition* instinct — *"only trap on the final mark: valid ⇒ sync,
//! invalid ⇒ dealloc"* — is the right economics and the wrong object. Four of the seven holes
//! `resume_from_fault.md` §6 put to it are cases where a mapping becomes reachable or
//! unreachable **without the entry passing through the state an edge-watcher waits for**:
//!
//! - a leaf goes `VALID=1` while its parent directory entry is still invalid, so the leaf's
//!   own edge is not the moment it becomes reachable — *"write entries bottom up, so that they
//!   are valid once they're inserted into the tree"*, `ogkm-580:
//!   kernel-open/nvidia-uvm/uvm_mmu.c:771-782` (hole 1);
//! - a teardown clears the **parent** and frees the children's backing store with no invalidate
//!   between, so hundreds of leaves are unmapped and none is ever written invalid — `ogkm-580:
//!   src/nvidia/src/libraries/mmu/mmu_walk.c:1509-1552` (hole 3);
//! - a re-map moves a leaf's physical address without passing through invalid at all —
//!   `ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/virt_mem_allocator_gm107.c:2602-2606`
//!   (hole 4);
//! - a directory slot that becomes a 512 MiB leaf unlinks its whole sub-tree while nothing goes
//!   invalid anywhere (holes 3 and 7 composed).
//!
//! This module therefore never asks what a store *did*. It holds each page's decoded slot map,
//! and each pass recomputes two sets — which pages are reachable from the root, and which leaves
//! should be bound — and emits the **difference** against what it last told the address table.
//! Every one of the four cases above is then the same case.
//!
//! ## The two gates, and the argument that lets them differ
//!
//! **REACHABLE** is the closure from the root over the page-directory edges this shadow holds.
//! **WITNESSED** is the guest having been seen to write the page ([`ReachShadow::witness`]).
//! Only the *bind* gate needs both; reachability is deliberately permissive, and the reason it
//! can be is:
//!
//! > A directory entry read out of allocator residue can only make some *other* page reachable.
//! > That page is itself unwitnessed, so nothing in it binds. Residue cannot produce a binding.
//!
//! That is what closes hole 2 — `mmuWalkReserveEntries(..., bInvalidate = NV_FALSE)` leaves a
//! level reachable with uninitialised backing store (`ogkm-580:
//! src/nvidia/src/libraries/mmu/mmu_walk_reserve.c:57-63`, `:85`) — without a stricter
//! reachability rule that would make the guest's own tree unknowable.
//!
//! ## What this is NOT
//!
//! It holds **no page content**, only decoded slots. It is not a cache
//! [`crate::AddressTable::resolve`] may consult: `resolve` is unchanged and still MISS = FAULT,
//! and this module's entire output is a set of bind/unbind calls into that table. A leaf that is
//! reachable but unwitnessed stays a miss — which is a fault, which is
//! `resume_from_fault.md` §7 step 5, and is the price §6.1 named.

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_arch::ids::{GpuVa, Pdb};
use kayfabe_arch::{Aperture, GmmuFmt, PageSize};

use crate::walker::{
    DecodedLeaf, DropReason, LeafDisposition, PageDecode, PtPage, leaf_disposition,
};

/// How many page-table pages ONE shadow will hold.
///
/// Guest-influenced: the guest chooses how many pages its address space has, and §3.3 keeps
/// never-reachable pages indefinitely so an orphan can still be linked later. So it is bounded,
/// and reaching the bound is a loud [`ReachFault::TooManyPages`] rather than an eviction — an
/// eviction rule would silently delete the orphan the design exists to keep.
pub const MAX_SHADOW_PAGES: usize = 1 << 14;

/// How many non-invalid slots ONE shadow will hold, across every page.
///
/// The second bound is not redundant with [`MAX_SHADOW_PAGES`]: a page holds up to 512 slots, so
/// the page count alone permits eight million entries. This is the one that bounds memory.
pub const MAX_SHADOW_SLOTS: usize = 1 << 20;

/// Why the shadow refused. Every variant is LOUD and none has a fallback value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReachFault {
    /// ★★★ **The shadow's root is not the address space's page-directory base**
    /// (`reachability_on_transition.md` §3.5, hole 5).
    ///
    /// A page-directory rebind re-points an entire address space with **zero entry writes**
    /// (the guest's `SET_PAGE_DIRECTORY`), so a shadow that outlived one would answer
    /// reachability for a tree that is no longer installed. The structural defence is that a
    /// `Vas` is keyed by its PDB and a rebind mints a fresh one, shadow included; this is the
    /// standing audit that the structure held — the same argument
    /// [`crate::AddressTable::audit_identity`] makes for the identity law, for the same reason:
    /// a law with only a constructor check is one refactor away from being a law about nothing.
    RootMismatch {
        /// The address space the shadow was asked about.
        pdb: Pdb,
        /// The root the shadow was constructed with.
        root: u64,
    },
    /// [`MAX_SHADOW_PAGES`] reached. The page is **not admitted** — see [`ReachFault::TooManySlots`]
    /// for why refusal is whole-page.
    TooManyPages {
        /// The page that was refused.
        phys: u64,
    },
    /// [`MAX_SHADOW_SLOTS`] reached.
    ///
    /// ★ The page is refused **whole**, never truncated. Admitting half a page's slots would
    /// make the desired set a statement about a page-table page that never existed, and the
    /// next diff would unbind the half that was not admitted — a wrong unmap manufactured by
    /// our own bookkeeping rather than by anything the guest did.
    TooManySlots {
        /// The page that was refused.
        phys: u64,
    },
}

/// What one slot of a page-table page says. An **invalid** slot is not represented: it is the
/// absence of an entry, and absence is how the diff sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// A page-directory edge. A dual slot contributes two `Pde` slots at the same address, so
    /// they are keyed by `(slot va, child phys)` rather than by address alone.
    Pde { child: u64 },
    /// A leaf mapping.
    Leaf(DecodedLeaf),
    /// The guest's explicit "deliberately nothing here" (`reachability_on_transition.md` §3.6).
    Sparse,
}

/// One page-table page as the shadow remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShadowPage {
    /// Where it is, what level it is, what virtual address its entry 0 describes.
    page: PtPage,
    /// Its non-invalid slots, keyed by `(slot virtual address, discriminator)`. The
    /// discriminator is the child's physical address for an edge and `0` otherwise, which is
    /// what lets one 16-byte dual slot hold two edges without either overwriting the other.
    slots: BTreeMap<(u64, u64), Slot>,
    /// ★★★ Has this page **ever** been reachable from the root?
    ///
    /// The whole of hole 3's retirement rule turns on this bit, and getting it wrong in the
    /// obvious direction rebuilds `#13`. Retirement is keyed on *was reachable and is not* —
    /// a page that has fallen out of the tree. A page that was **never** reachable is the
    /// guest's own build order (a leaf table filled before anything points at it) and must be
    /// **kept**, because the link is coming.
    ever_reachable: bool,
}

/// A binding the shadow has told the address table about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Published {
    phys: u64,
    aperture: Aperture,
    size: PageSize,
    read_only: bool,
}

impl Published {
    fn of(l: &DecodedLeaf) -> Self {
        Published {
            phys: l.phys,
            aperture: l.aperture,
            size: l.size,
            read_only: l.read_only,
        }
    }
    /// Same address, same aperture, same extent — i.e. the address table would answer
    /// identically. Deliberately **excludes** `read_only`, which is why
    /// [`Settlement::protection_changes`] exists.
    fn same_mapping(self, other: Self) -> bool {
        (self.phys, self.aperture, self.size) == (other.phys, other.aperture, other.size)
    }
}

/// ★★★ **Two decoded leaves at ONE virtual address, with different shapes** — recorded at
/// the exact statement that discards one of them.
///
/// Carries both, and the page each came from, because *"which producer described this VA
/// twice"* is the only actionable half: on GA10x the expected answer is one dual PD0 slot's
/// big-page and small-page sub-tables (levels 4 and 5), and anything else is a different bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeCollision {
    /// The virtual address both leaves claim.
    pub va: GpuVa,
    /// The leaf that **survives** into the desired set (the later insert — this preserves the
    /// behaviour that was already there, and does **not** assert it is the right one).
    pub kept: CollidingLeaf,
    /// The leaf that is **discarded**.
    pub dropped: CollidingLeaf,
}

/// One side of a [`ShapeCollision`], reduced to the fields that distinguish it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollidingLeaf {
    /// Its extent.
    pub size: PageSize,
    /// The level it was decoded at — its provenance, and on GA10x its sub-table.
    pub level: u8,
    /// What it says backs `va`.
    pub phys: u64,
    /// The page-table page it was read out of.
    pub from_page: u64,
}

/// What one [`ReachShadow::settle`] decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settlement {
    /// Leaves that must be forward-populated into the table — newly reachable-and-witnessed,
    /// or re-pointed. Handed to [`crate::walker::populate`], which owns the table's refusal
    /// vocabulary.
    pub binds: Vec<DecodedLeaf>,
    /// Virtual addresses that must leave the table, ascending. A teardown (hole 3), a
    /// valid→sparse transition (hole 6), or a leaf that stopped being reachable.
    pub unbinds: Vec<GpuVa>,
    /// ★★ Pages retired from the shadow because they **were** reachable and no longer are.
    /// The caller must also drop their level metadata, or the next write to a recycled page is
    /// decoded as a page table (`reachability_on_transition.md` §3.3).
    pub retired: Vec<u64>,
    /// ★★ Slots whose **only** change is the read-only bit.
    ///
    /// A mapping change that grants or withdraws access without moving an address. It is
    /// reported and never folded into "unchanged": [`crate::Binding`] carries no rights, so
    /// this is *seen, named, counted, not modelled* — the honest state, and a fidelity gap
    /// rather than an isolation one, since the rights are the guest's own declarations about
    /// its own address space.
    pub protection_changes: Vec<GpuVa>,
    /// Leaves in a **reachable** page that is not witnessed. They stay a MISS — §6.1's rule,
    /// and the input to `resume_from_fault.md` §7 step 5.
    pub unwitnessed: usize,
    /// Leaves in a **witnessed** page that is not reachable — the orphan, waiting for its link.
    pub unreachable: usize,
    /// Leaves decoded faithfully and declined by [`crate::walker::leaf_disposition`].
    pub dropped: Vec<(GpuVa, DropReason)>,
    /// Slots the guest declared sparse, across every reachable-and-witnessed page.
    pub sparse: usize,
    /// ★★★★★ **THE THIRD OUTCOME, NAMED** — leaves that were decoded, were reachable, were
    /// witnessed, passed [`crate::walker::leaf_disposition`] … and then **lost a key
    /// collision inside this function** and were never proposed to the table at all.
    ///
    /// # ⊘⊘ Why this exists, and what it cost to not have it
    ///
    /// The desired set is a `BTreeMap` keyed on the leaf's **virtual address alone**. That is
    /// correct for the diff (the table holds one shape per address) and **silently lossy** for
    /// the input: a GA10x PD0 slot is a *dual* PDE, so one 2 MiB VA range is described by a
    /// 64 KiB big-page table **and** a 4 KiB small-page table, and a VA covered by a valid
    /// entry in both yields **two leaves at the same base address with different sizes**. The
    /// second `insert` overwrote the first, returned the old value, and the old value was
    /// dropped on the floor — no counter, no refusal, no log line.
    ///
    /// `[measured, w276b_on]` the sweep reported `bound=0 … refusals=255` while hardware
    /// faulted at an address that was **neither bound nor refused** — three outcomes where the
    /// instrument could name two. This is the third. It is *recorded rather than fixed*: which
    /// of two overlapping guest declarations wins is a **hardware precedence** question, and
    /// making up an answer here would be the silent overwrite again with a comment on it.
    ///
    /// ⚠ Same disease as `w270` (*"the key was the VA, not the extent"*) one layer up.
    pub shape_collisions: Vec<ShapeCollision>,
    /// Leaves that arrived twice, **byte-identical**. Not a collision: two pages describing
    /// the same mapping the same way is one fact seen twice, and folding it into
    /// [`Self::shape_collisions`] would make a benign duplicate read as a contradiction.
    pub duplicate_leaves: usize,
    /// ★★★★★ Leaves in [`Self::binds`]-or-already-published that are desired **only because a
    /// sweep admitted their page** — i.e. the page was never witnessed being written.
    ///
    /// ⊘ This is the price of [`ReachShadow::witness_swept`] as a number. A rung that turns the
    /// sweep on and cannot say how much of its published set came from the relaxation has not
    /// measured the relaxation, only its side effects.
    pub swept_binds: usize,
}

/// ★★★ **The reachability shadow of ONE address space.**
///
/// Owned by that address space's `Vas` and keyed by nothing — its identity is the object it
/// hangs on, which is what makes a page-directory rebind unable to carry one over (§3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachShadow {
    /// The address space's page-directory base page. Level 0, and a **declared** fact: a PDB
    /// *is* its own root page, reachable whether or not anything has been read out of it.
    root: u64,
    /// Every page-table page this shadow has been shown.
    pages: BTreeMap<u64, ShadowPage>,
    /// Pages the guest was seen to write. Cumulative, and it must be, because the dirty set
    /// that feeds it is *drained*: a page witnessed in one pass and linked in the next would
    /// otherwise bind nothing.
    witnessed: BTreeSet<u64>,
    /// ★★★★★ **Pages admitted by a whole-VAS SWEEP** — the C's `enum_gr_sysmem`
    /// (`C: nvkvm_gpu_emul.c:583-591`), and the ONE relaxation of the witness rule this port
    /// carries. See [`ReachShadow::witness_swept`] for what it costs and what pays for it.
    ///
    /// ⊘ Held **apart** from [`Self::witnessed`] and never merged into it, for the reason the
    /// campaign keeps re-learning: a relaxation folded into the thing it relaxes is a
    /// relaxation nobody can measure afterwards. [`Settlement::swept_binds`] is the number
    /// that says how much of the published set exists **only** because of it, and it can only
    /// exist while the two sets are two sets.
    swept: BTreeSet<u64>,
    /// What this shadow last told the address table, keyed by virtual address. The diff is
    /// against this and not against the table, because the table also holds bindings from other
    /// populate sources (`Vas::rpc_bound`, `Vas::promote_bound`) that are not ours to unbind.
    published: BTreeMap<u64, Published>,
    /// Live slot count across [`ReachShadow::pages`], maintained rather than recomputed.
    slots: usize,
}

impl ReachShadow {
    /// An empty shadow rooted at `root` — the address space's page-directory base.
    #[must_use]
    pub fn new(root: u64) -> Self {
        ReachShadow {
            root,
            pages: BTreeMap::new(),
            witnessed: BTreeSet::new(),
            swept: BTreeSet::new(),
            published: BTreeMap::new(),
            slots: 0,
        }
    }

    /// The page-directory base this shadow was built for.
    #[must_use]
    pub fn root(&self) -> u64 {
        self.root
    }

    /// ★★★ **Hole 5's standing audit**: is this shadow the shadow of `pdb`'s address space?
    ///
    /// # Errors
    /// [`ReachFault::RootMismatch`] — see that variant for why the check exists even though a
    /// `Vas` is keyed by its PDB and constructs its own shadow.
    pub fn audit_root(&self, pdb: Pdb) -> Result<(), ReachFault> {
        if self.root == pdb.0 & !0xfff {
            Ok(())
        } else {
            Err(ReachFault::RootMismatch {
                pdb,
                root: self.root,
            })
        }
    }

    /// ★★ **WITNESS** — record that the guest was seen to write page-table page `phys`.
    ///
    /// Called at the **drain** of the dirty set, for every page drained, including ones whose
    /// level is not yet known and which are therefore deferred rather than decoded. Deferring
    /// the decode is correct (the bytes are already ours); deferring the *witness* would lose
    /// it, because the dirty set is consumed.
    pub fn witness(&mut self, phys: u64) {
        self.witnessed.insert(phys);
    }

    /// Has the guest been seen to write this page?
    #[must_use]
    pub fn is_witnessed(&self, phys: u64) -> bool {
        self.witnessed.contains(&phys)
    }

    /// ★★★★★ **SWEPT** — admit `phys` because a **whole-VAS walk descended to it from this
    /// address space's own installed page-directory root**.
    ///
    /// # ⊘⊘ This is a DELIBERATE RELAXATION of the witness rule. Read why before calling it.
    ///
    /// [`Settlement::unwitnessed`] is not a defect: a reachable-but-unwitnessed leaf is a MISS
    /// on purpose, and the refusal is *"the whole of hole 2's resolution and the reason residue
    /// cannot bind"*. A sweep cannot satisfy it — every page a root descent reaches was, by
    /// construction, never seen being written — so a root-seeded pass **publishes nothing**
    /// unless the gate moves (`[measured, w275]`, from this file's own source).
    ///
    /// ## What the relaxation is, stated as narrowly as it is implemented
    ///
    /// > A page is `swept` iff a walk **starting at `self.root`**, over **the guest's own page
    /// > directories**, reached it. Nothing else may call this.
    ///
    /// It is *not* "read whatever the guest points at": the seed is the address space's own
    /// installed PDB, the descent is bounded ([`crate::walker::MAX_WALK_DEPTH`], the decode
    /// budget), and every byte still comes through the same aperture-checked reader, whose
    /// failures stay [`crate::walker::WalkFault`]s and are never decoded to zeros.
    ///
    /// ## What it costs, and what pays for it — do not paper this over
    ///
    /// `mode2_address_table.md` §6 refuses a miss-walk because the state may be **mid-update**
    /// (a torn multi-level walk resolves to the wrong physical page). A doorbell is the guest's
    /// own commit point for the work it submits, so the hazard does not apply to *that
    /// submission's* reachable set — but it **does** still apply to unrelated regions of the
    /// same address space, which another guest thread may be rewriting right now. That residual
    /// is accepted knowingly (owner ruling, 2026-08-12) and is bounded by the other half of the
    /// C's design, which is why this function is useless on its own:
    ///
    /// > A page that was mid-update when swept was **by definition being written**, so it lands
    /// > in the dirty set, so the next doorbell re-sweeps it. The torn window is bounded and
    /// > self-healing.
    ///
    /// ⇒ **A one-shot sweep without the dirty-driven re-sweep does not carry this argument.**
    /// The re-sweep half lives in `kayfabe_fwd::plan_pt_sweep`; build both or neither.
    ///
    /// ⚠ Residue is the price. Hole 2's guarantee — *"a directory entry read out of allocator
    /// residue can only make some other page reachable, and that page is unwitnessed, so
    /// nothing in it binds"* — is exactly what this trades away: a sweep marks the residue page
    /// swept and its leaves become bindable. The mitigation is the same self-healing loop, plus
    /// the fact that every such leaf must still pass [`crate::walker::leaf_disposition`] and
    /// the address table's own refusals.
    pub fn witness_swept(&mut self, phys: u64) {
        self.swept.insert(phys);
    }

    /// Was this page admitted by a sweep (and not by a witnessed guest write)?
    #[must_use]
    pub fn is_swept(&self, phys: u64) -> bool {
        self.swept.contains(&phys)
    }

    /// ★★ How many pages are admitted **only** by the sweep relaxation.
    ///
    /// ⊘ Reported for [`ReachShadow::witnessed_len`]'s reason, one plane over: *"the sweep
    /// published N bindings"* and *"the sweep is the ONLY reason N bindings exist"* are
    /// different claims, and a single total cannot carry the second.
    #[must_use]
    pub fn swept_only_len(&self) -> usize {
        self.swept.difference(&self.witnessed).count()
    }

    /// How many page-table pages this shadow holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// ★★★ How many pages the guest has been **seen to write**.
    ///
    /// ⊘ Reported because [`Settlement::unwitnessed`] is a *count of leaves*, and a leaf count
    /// cannot say whether the witness set is empty (nothing ever reached this shadow) or merely
    /// missing the one page in question. Those are different defects with different fixes:
    /// the first is a transport that does not cover this writer, the second is an ordering gap.
    #[must_use]
    pub fn witnessed_len(&self) -> usize {
        self.witnessed.len()
    }

    /// ★★ Up to `n` witnessed pages, ascending — **so a report can show this predicate
    /// answering YES about something.**
    ///
    /// ⊘ The whole reason it exists: `is_witnessed(p) == false` printed alone is
    /// indistinguishable from an instrument that cannot say `true`. A sample from the same
    /// set turns the negative into a measurement (`a_test_can_be_correct_and_unable_to_fail`).
    #[must_use]
    pub fn witnessed_sample(&self, n: usize) -> Vec<u64> {
        self.witnessed.iter().copied().take(n).collect()
    }

    /// Is the shadow empty? (A fresh address space's is.)
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// ★★ Did the guest declare `va` **sparse** in a page this shadow holds?
    ///
    /// ⊘ This has **no consumer**. It exists because sparse's guest-visible semantics — on real
    /// silicon a sparse mapping drops writes and returns zeros rather than faulting — is not
    /// modelled anywhere in this port, and a query is the smallest honest way to say *"we know
    /// which addresses those are, and we do not yet act on it"*. Saying so is worth more than
    /// shipping it and implying a consumer.
    #[must_use]
    pub fn sparse_at(&self, va: GpuVa) -> bool {
        self.pages
            .values()
            .any(|p| matches!(p.slots.get(&(va.0, 0)), Some(Slot::Sparse)))
    }

    /// ★★★★★ **ARM 2.1 — does THE GUEST describe a mapping covering `va`?**
    ///
    /// Searches the **reachable** pages only, because reachability from the address space's own
    /// installed page-directory root is what makes a decoded leaf the guest's description rather
    /// than our residue. A leaf sitting in an orphan table is deliberately not a hit: the guest
    /// has not linked it, so at this instant the guest does not describe that address.
    ///
    /// ⊘ **What a `None` is and is not.** It says *this shadow holds no reachable leaf covering
    /// `va`*. That is only a statement about the guest to the extent the shadow is a complete
    /// picture of the guest — i.e. after a **completed** whole-VAS sweep. Read beside a
    /// truncated or never-run sweep it says nothing at all, which is why every caller must print
    /// the sweep's state on the same line.
    ///
    /// ⚠ Linear in the shadow. It is a diagnostic, asked once per fault, never on a hot path.
    #[must_use]
    pub fn leaf_covering(&self, va: GpuVa) -> Option<DecodedLeaf> {
        let reachable = self.reachable();
        self.pages
            .iter()
            .filter(|(phys, _)| reachable.contains(phys))
            .flat_map(|(_, p)| p.slots.values())
            .find_map(|s| match s {
                Slot::Leaf(l) if l.va.0 <= va.0 && va.0 - l.va.0 < l.size.0 => Some(*l),
                _ => None,
            })
    }

    /// ★★ Every reachable leaf's `(va, bytes)`, ascending — the **coalesced** picture of what
    /// this address space describes.
    ///
    /// ⊘ Coalescing is what makes it printable: a `cuCtxCreate` VAS is thousands of 4 KiB leaves
    /// in a handful of runs, and a per-leaf dump is a log nobody reads. Adjacent leaves merge
    /// only when they are **contiguous in VA**; two runs separated by one unmapped page stay two
    /// runs, because *"is there a hole here"* is the whole question a reader brings to this.
    #[must_use]
    pub fn reachable_ranges(&self) -> Vec<(u64, u64)> {
        let reachable = self.reachable();
        let mut leaves: Vec<(u64, u64)> = self
            .pages
            .iter()
            .filter(|(phys, _)| reachable.contains(phys))
            .flat_map(|(_, p)| p.slots.values())
            .filter_map(|s| match s {
                Slot::Leaf(l) => Some((l.va.0, l.size.0)),
                _ => None,
            })
            .collect();
        leaves.sort_unstable();
        let mut out: Vec<(u64, u64)> = Vec::new();
        for (va, len) in leaves {
            match out.last_mut() {
                Some((s, l)) if *s + *l == va => *l += len,
                // ⊘ An exact duplicate (the same leaf reported twice) is absorbed rather than
                // extending the run: it is one mapping, and adding its length would report a
                // range the guest never described.
                Some((s, l)) if va >= *s && va + len <= *s + *l => {}
                _ => out.push((va, len)),
            }
        }
        out
    }

    /// ★★ **OBSERVE** — replace what the shadow knows about one page with what a decode of it
    /// just found.
    ///
    /// Replacement and not merge, deliberately: a decode reads the **whole** page, so a slot
    /// the decode did not report is a slot that is now invalid. Merging would make an entry the
    /// guest cleared immortal.
    ///
    /// # Errors
    /// [`ReachFault::TooManyPages`] / [`ReachFault::TooManySlots`] — the page is refused whole
    /// and the shadow is left exactly as it was.
    pub fn observe(&mut self, page: PtPage, decode: &PageDecode) -> Result<(), ReachFault> {
        let mut slots: BTreeMap<(u64, u64), Slot> = BTreeMap::new();
        for c in &decode.children {
            // A dual slot names two sub-tables at ONE virtual address; keying on the child's
            // physical address is what keeps the second from overwriting the first.
            slots.insert((c.vabase, c.phys), Slot::Pde { child: c.phys });
        }
        for l in &decode.leaves {
            slots.insert((l.va.0, 0), Slot::Leaf(*l));
        }
        for &va in &decode.sparse {
            slots.insert((va, 0), Slot::Sparse);
        }

        let known = self.pages.get(&page.phys);
        if known.is_none() && self.pages.len() >= MAX_SHADOW_PAGES {
            return Err(ReachFault::TooManyPages { phys: page.phys });
        }
        let was = known.map_or(0, |p| p.slots.len());
        let after = self.slots - was + slots.len();
        if after > MAX_SHADOW_SLOTS {
            return Err(ReachFault::TooManySlots { phys: page.phys });
        }
        self.slots = after;
        let ever = known.is_some_and(|p| p.ever_reachable);
        self.pages.insert(
            page.phys,
            ShadowPage {
                page,
                slots,
                ever_reachable: ever,
            },
        );
        Ok(())
    }

    /// The closure of pages reachable from the root over the edges this shadow holds.
    ///
    /// The root is in it by declaration. Bounded by the number of pages held, and every page is
    /// expanded at most once, so a guest-built cycle terminates rather than needing a depth cap
    /// here — the cap that matters lives in the decoder, where the reading happens.
    fn reachable(&self) -> BTreeSet<u64> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![self.root];
        seen.insert(self.root);
        while let Some(phys) = stack.pop() {
            let Some(p) = self.pages.get(&phys) else {
                continue;
            };
            for s in p.slots.values() {
                if let Slot::Pde { child } = s
                    && seen.insert(*child)
                {
                    stack.push(*child);
                }
            }
        }
        seen
    }

    /// ★★★ **SETTLE** — recompute reachability and the desired binding set, and return the
    /// difference against what this shadow last published.
    ///
    /// Runs **once per pass**, not once per observed page. Both halves are O(the shadow), so
    /// per-page would make a pass quadratic in a guest-influenced quantity; and once per pass is
    /// also the semantically right point, because a shadow that emitted a transition halfway
    /// through would be answering with a tree the guest had not finished writing.
    ///
    /// Three things happen here and they are in this order for a reason:
    /// 1. retire pages that **were** reachable and no longer are (hole 3), which is what makes
    ///    their leaves leave the desired set rather than needing to be hunted for;
    /// 2. build the desired set from pages that are reachable **and** witnessed;
    /// 3. diff, which is where holes 1, 4, 6 and 7 all turn into the same two lists.
    pub fn settle(&mut self, fmt: &dyn GmmuFmt) -> Settlement {
        let mut out = Settlement::default();

        // --- 1. RETIREMENT ------------------------------------------------------------
        let reachable = self.reachable();
        let retire: Vec<u64> = self
            .pages
            .iter()
            .filter(|(phys, p)| p.ever_reachable && !reachable.contains(phys))
            .map(|(phys, _)| *phys)
            .collect();
        for phys in retire {
            if let Some(p) = self.pages.remove(&phys) {
                self.slots -= p.slots.len();
            }
            // ★ The witness goes with it. The page's bytes are about to be something else
            // entirely — `_mmuWalkPdeRelease` frees the backing store right after clearing the
            // parent — so a stale witness would let the NEXT thing written there bind as if it
            // were a page table we had been watching all along.
            self.witnessed.remove(&phys);
            // ★ And the sweep's admission goes with it for the SAME reason, not a similar one.
            // A retired page's bytes are about to be something else; leaving it in `swept`
            // would let the next thing written there bind as if a root descent had just
            // reached it, which is the residue hazard this relaxation already pays for once.
            self.swept.remove(&phys);
            out.retired.push(phys);
        }
        // ★ ONE closure is enough, and the reason is worth stating because "retire, then
        // recompute, then retire again" is the obvious shape. A retired page was, by
        // definition, NOT in the closure — so the walk never traversed its edges, so deleting
        // it cannot change the closure. The whole cascade (root loses its edge to A; A and
        // everything under it goes) falls out of the single closure above.
        for (phys, p) in &mut self.pages {
            p.ever_reachable |= reachable.contains(phys);
        }

        // --- 2. THE DESIRED SET -------------------------------------------------------
        // ★ The value carries the page the leaf was read out of, so a key collision can name
        // BOTH producers. Without it the displaced side of a collision is anonymous, and
        // *"which producer described this VA twice"* is the only actionable half.
        let mut desired: BTreeMap<u64, (DecodedLeaf, u64)> = BTreeMap::new();
        for (phys, p) in &self.pages {
            let live = reachable.contains(phys);
            let written = self.witnessed.contains(phys);
            // ★★★★★ THE ONE RELAXATION, and it is spelled out here rather than hidden in a
            // helper so that a reader of the gate sees both admissions at once. See
            // [`ReachShadow::witness_swept`] for the full argument and the accepted residual.
            let swept_only = !written && self.swept.contains(phys);
            let seen = written || swept_only;
            for s in p.slots.values() {
                match s {
                    Slot::Sparse if live && seen => out.sparse += 1,
                    Slot::Leaf(l) => {
                        if !seen {
                            // Reachable-but-unwitnessed: a MISS, on purpose. This is the whole
                            // of hole 2's resolution and the reason residue cannot bind.
                            if live {
                                out.unwitnessed += 1;
                            }
                            continue;
                        }
                        if !live {
                            out.unreachable += 1;
                            continue;
                        }
                        match leaf_disposition(fmt, l) {
                            LeafDisposition::Drop(why) => out.dropped.push((l.va, why)),
                            LeafDisposition::Bind => {
                                // ★★ Counted BEFORE the insert and against the *page's*
                                // admission, so it answers "how much of the desired set exists
                                // only because the gate was relaxed" — the number the owner's
                                // ruling has to be judged on. ⊘ It is a count of DESIRED
                                // leaves, not of binds the table accepted; the table's own
                                // refusals are downstream and are reported there.
                                if swept_only {
                                    out.swept_binds += 1;
                                }
                                // ★★★★★ THE THIRD OUTCOME, at the statement that creates it.
                                // `insert` returns what it displaced; the old code threw that
                                // away, which is how a decoded, reachable, witnessed, policy-
                                // approved leaf could vanish with no counter naming it. See
                                // [`Settlement::shape_collisions`].
                                //
                                // ⊘ Behaviour is UNCHANGED — the later leaf still wins. The
                                // only new thing is that the loss is now *stated*. Choosing a
                                // winner is a hardware-precedence question and inventing one
                                // here would be the same silent overwrite with a comment on it.
                                if let Some((prev, prev_page)) = desired.insert(l.va.0, (*l, *phys))
                                {
                                    if prev == *l {
                                        out.duplicate_leaves += 1;
                                    } else {
                                        out.shape_collisions.push(ShapeCollision {
                                            va: l.va,
                                            kept: CollidingLeaf {
                                                size: l.size,
                                                level: l.level,
                                                phys: l.phys,
                                                from_page: *phys,
                                            },
                                            dropped: CollidingLeaf {
                                                size: prev.size,
                                                level: prev.level,
                                                phys: prev.phys,
                                                from_page: prev_page,
                                            },
                                        });
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // --- 3. THE DIFF --------------------------------------------------------------
        for (&va, (l, _from_page)) in &desired {
            let want = Published::of(l);
            match self.published.get(&va) {
                Some(&have) if have == want => {}
                Some(&have) if have.same_mapping(want) => {
                    // Only the rights moved. The table cannot carry them, so this is reported
                    // rather than performed — and reported rather than silently "unchanged".
                    out.protection_changes.push(GpuVa(va));
                    self.published.insert(va, want);
                }
                Some(_) => {
                    // A re-map, a resize, or an aperture move. All three need the old range out
                    // of the table before the new one goes in, so all three are an unbind
                    // followed by a bind — which is also the table's own discipline
                    // (unmap eager), not a convenience.
                    out.unbinds.push(GpuVa(va));
                    out.binds.push(*l);
                }
                None => out.binds.push(*l),
            }
        }
        for &va in self.published.keys() {
            if !desired.contains_key(&va) {
                out.unbinds.push(GpuVa(va));
            }
        }
        out.unbinds.sort_unstable();
        out.unbinds.dedup();
        out
    }

    /// ★ Record that the address table **accepted** a bind, so the next diff compares against
    /// what the table actually holds.
    ///
    /// Split from [`ReachShadow::settle`] because a bind can be refused — a range that is
    /// host-published, an overlap — and a shadow that assumed its own settlement landed would
    /// stop proposing the bind that never happened. The caller applies, then reports back.
    pub fn confirm_bind(&mut self, leaf: &DecodedLeaf) {
        self.published.insert(leaf.va.0, Published::of(leaf));
    }

    /// Record that the address table accepted an unbind (or that the range was not ours).
    pub fn confirm_unbind(&mut self, va: GpuVa) {
        self.published.remove(&va.0);
    }

    /// How many bindings this shadow believes it has in the table. Test-facing: *"we told the
    /// table N things"* is otherwise only observable by re-deriving it.
    #[must_use]
    pub fn published_len(&self) -> usize {
        self.published.len()
    }
}

/// ★★★ **APPLY a settlement to the address table** — the one place a reachability decision
/// becomes a table mutation.
///
/// Order is **unbind before bind**, and it is not a convenience: a re-map, a resize and an
/// aperture move all arrive as an unbind of the old range plus a bind of the new one, and the
/// table refuses an overlapping bind ([`crate::AddressFault::Overlap`]) by design.
///
/// Binding itself is delegated to [`crate::walker::populate`] rather than reimplemented, so the
/// refusal vocabulary has exactly one owner and the "a decode declares, it does not publish"
/// comparison (`(phys, aperture)`, never the whole [`crate::Binding`]) cannot drift into a
/// second copy.
///
/// ★★ One unbind is **refused rather than performed**: a range whose binding is host-published.
/// See [`crate::walker::PopulateRefusal::UnbindsPublished`].
pub fn apply_settlement(
    fmt: &dyn GmmuFmt,
    table: &mut crate::AddressTable,
    shadow: &mut ReachShadow,
    pdb: Pdb,
    settlement: &Settlement,
) -> ApplyOutcome {
    apply_settlement_as(fmt, table, shadow, pdb, settlement, PublishedUnbind::Refuse)
}

/// ★★★★★ **w329 — [`apply_settlement`] with the host-published unbind's policy as a
/// parameter**, because *"refuse it"* and *"revoke it"* are both correct answers and which one
/// is right is not a fact this crate can settle.
///
/// # ⊘ Why the refusal existed, and why it is still the default
///
/// `walker.rs`'s `UnbindsPublished` is not an oversight: *"dropping the range from the table
/// would leave the host object still allocated and still mapped into that address space's host
/// VAS, with **no core state naming it**. That is worse than a leak."* That argument is
/// **entirely about the caller** — it is true of a caller that drops the row and forgets, and
/// false of one that takes the host half away with it. So the policy belongs to the caller, and
/// [`PublishedUnbind::Refuse`] stays the default for every caller that has no release path.
///
/// # ★★★ What [`PublishedUnbind::RevokeWholeJoins`] revokes, and what it still refuses
///
/// A row qualifies only when **all four** hold, and each one closes a named hazard:
///
/// | condition | the hazard it closes |
/// |---|---|
/// | `start == va.0` and the row is the whole tabled extent | the **partial extent**: a proposal naming part of a larger binding would revoke bytes nobody proposed. `w291`'s merge was bounded to exact-extent rows for this reason. |
/// | [`crate::HostBacking::frees_object`] | the **double free**: an [`crate::HostExtent::Slice`] names an arena object that serves sibling bindings at other offsets, so freeing it here destroys what the last one owns. This is the same predicate `kayfabe_fwd::unpublish_backing` already gates its `free` on. |
/// | `bytes == BackingBytes::JoinsGuestWindow` | the **wrong plane**: `ShadowsGuestMemory` is ruling 3's refused chain and a published-GPA row is the arena case above; only the join is 1 leaf : 1 whole object. |
/// | the caller asked for it | the **orphan**: the row's host half leaves in [`ApplyOutcome::revoked`] and the caller must dispose of it. A caller that ignores the field has traded a frozen row for a leaked object. |
///
/// ⊘ **Everything else keeps the refusal, byte for byte.** A row that is host-published and
/// fails any condition above still pushes `UnbindsPublished` and still stays in the table.
///
/// # ⚠ The residual this does NOT close, stated because it is real
///
/// The bytes behind a revoked join are **not carried anywhere**. That is correct for the case
/// this exists for — a framebuffer frame no page table names is, to this device, an unallocated
/// frame, and an unallocated frame's truth is *"no page, reads zero"*, the same answer
/// `FbStore::device_reset` leaves — and it is **lossy** if the guest still reaches the same
/// frame through a second alias it did not unmap. That case is counted rather than assumed
/// absent: see [`ApplyOutcome::revoked_still_desired`].
pub fn apply_settlement_as(
    fmt: &dyn GmmuFmt,
    table: &mut crate::AddressTable,
    shadow: &mut ReachShadow,
    pdb: Pdb,
    settlement: &Settlement,
    policy: PublishedUnbind,
) -> ApplyOutcome {
    let mut out = ApplyOutcome::default();
    // ★ The set of framebuffer offsets this very settlement wants bound, so a revocation of a
    // frame the guest is re-describing in the SAME pass is visible as such. ⊘ It does not gate
    // the revocation — the re-description is at a different VA and needs the join released
    // before it can take one of its own — it makes the lossy sub-case countable.
    let redescribed: BTreeSet<u64> = if policy == PublishedUnbind::Refuse {
        BTreeSet::new()
    } else {
        settlement.binds.iter().map(|l| l.phys).collect()
    };
    for &va in &settlement.unbinds {
        match table.binding_at(va) {
            Some((start, tlen, b)) if b.host.is_some() => {
                let h = b.host.expect("checked in the guard");
                let whole_row = start == va.0;
                let qualifies = policy == PublishedUnbind::RevokeWholeJoins
                    && whole_row
                    && h.frees_object()
                    && h.bytes() == crate::BackingBytes::JoinsGuestWindow;
                if qualifies {
                    // ★★★ The row leaves the table and its host half leaves WITH it, in one
                    // statement. There is no instant at which the range is untabled and the
                    // caller has not been handed the object — which is precisely the state
                    // `walker.rs:956-972` refused to create.
                    table.unbind(va);
                    shadow.confirm_unbind(va);
                    out.unbound += 1;
                    if redescribed.contains(&b.phys) {
                        out.revoked_still_desired += 1;
                    }
                    out.revoked.push(RevokedPublication {
                        va,
                        len: tlen,
                        phys: b.phys,
                        host: h,
                    });
                    continue;
                }
                out.refusals
                    .push(crate::walker::PopulateRefusal::UnbindsPublished { va });
                // ★ The shadow is NOT told the unbind happened, because it did not. Next pass
                // proposes it again, which is what a refusal that the forwarding plane can
                // later clear should do.
            }
            Some(_) => {
                table.unbind(va);
                shadow.confirm_unbind(va);
                out.unbound += 1;
            }
            // Not in the table at all — another populate source's range, or already gone.
            // Dropping our belief about it is still right: we no longer claim it.
            None => shadow.confirm_unbind(va),
        }
    }
    let applied = crate::walker::populate(fmt, table, pdb, &settlement.binds);
    for l in &settlement.binds {
        // Only confirm what the table now actually agrees with. A leaf the populate refused is
        // left unconfirmed so the next settle proposes it again.
        if table
            .binding_at(l.va)
            .is_some_and(|(s, len, b)| s == l.va.0 && len == l.size.0 && b.phys == l.phys)
        {
            shadow.confirm_bind(l);
        }
    }
    out.bound = applied.bound;
    out.unchanged = applied.unchanged;
    out.repointed = applied.repointed;
    out.dropped = applied.dropped;
    out.refusals.extend(applied.refusals);
    out
}

/// ★★★★★ **w329 — what [`apply_settlement_as`] does with an unbind of a host-published row.**
///
/// Two answers, both correct, and the difference is entirely a property of the **caller**:
/// whether it has a synchronous release path for the host object the row names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PublishedUnbind {
    /// ⊘ Refuse the unbind and leave the binding — `UnbindsPublished`. The default, and the
    /// only safe answer for a caller with nowhere to send the host half.
    #[default]
    Refuse,
    /// ★ Perform the unbind for **whole-object joined leaves only**, and hand the host half
    /// back in [`ApplyOutcome::revoked`]. See [`apply_settlement_as`] for the four conditions.
    RevokeWholeJoins,
}

/// ★★★★★ **w329 — one host-published row that left the table**, with everything needed to
/// release it and nothing that would let a caller re-derive it wrongly.
///
/// ⊘ The [`crate::HostBacking`] is carried **whole** rather than decomposed into
/// `(host_va, memory)`: `frees_object()` is the predicate that separates a whole object from an
/// arena slice, and a caller handed only the two fields would have to re-decide it — a second
/// source of truth beside a complete value, which this tree has paid for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevokedPublication {
    /// The guest VA the row was bound at — the table's own key, not a re-derivation.
    pub va: GpuVa,
    /// The row's tabled length. Equal to the leaf's, because a partial extent is refused.
    pub len: u64,
    /// The framebuffer offset the row named — the key the join is installed under in
    /// `kayfabe_device::FbStore`, and the reason the two halves can be released together.
    pub phys: u64,
    /// The host materialization to release: `host_va()` to unmap, `memory()` to free, and
    /// `frees_object()` already true by the qualification above.
    pub host: crate::HostBacking,
}

/// What [`apply_settlement`] did to the table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// Leaves bound into a range that was free.
    pub bound: usize,
    /// Leaves that restated a binding already in the table.
    pub unchanged: usize,
    /// Leaves that re-pointed an existing, unpublished binding.
    pub repointed: usize,
    /// Ranges actually removed from the table.
    pub unbound: usize,
    /// Leaves decoded faithfully and declined by policy, with the reason.
    pub dropped: Vec<(GpuVa, DropReason)>,
    /// ★★★★★ **w329 — the host-published rows this pass REVOKED**, each carrying the host
    /// object the caller must now release. Always empty under [`PublishedUnbind::Refuse`].
    ///
    /// ⊘ **A caller that ignores this has not kept the old behaviour — it has leaked.** The
    /// row is already out of the table when this is returned; the object it named is reachable
    /// through nothing else.
    pub revoked: Vec<RevokedPublication>,
    /// ★ How many of [`ApplyOutcome::revoked`] name a framebuffer offset **this same
    /// settlement also wants bound** — i.e. the guest moved the frame to another VA rather
    /// than releasing it. Counted because it is the one sub-case where dropping the join's
    /// bytes is lossy, and *"it never happens"* is a claim that needs a number.
    pub revoked_still_desired: usize,
    /// Everything the table or this function refused. Loud; the caller propagates.
    pub refusals: Vec<crate::walker::PopulateRefusal>,
}

kayfabe_util::assert_send_sync!(ReachShadow, Settlement, ReachFault, ApplyOutcome);
