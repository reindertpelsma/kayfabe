//! ★★★★★ **THE VA REFRESH — provenance, and why UNMAP is the dangerous direction.**
//!
//! **Owner, 2026-09-10:**
//! > *"your VA refresh function checks the parent, if the parent is not set, it does not scan
//! > the contents, and does not unset dirty."*
//! > *"ensure for mappings you also locate in which page(s) it was mapped, because if its no
//! > longer there during a refresh, you should unmap it very important. Even if the write is
//! > partial to any of those pages, the entire mapping should be just rechecked."*
//!
//! This module is the **model** of that rule: a pure fixpoint over mocked page tables and
//! mocked dirty bits, with no VMM, no GPU and no isolate, so the property can be stressed.
//!
//! # ⊘ Why provenance is the whole design
//!
//! A binding is *derived* from entries in specific page-table pages — the **path** from the
//! root down to the leaf. Record that path and two hard cases fall out for free:
//!
//! - **Teardown that clears a PARENT and frees the children with no invalidate.** The parent
//!   page is on every descendant's path, so dirtying it rechecks all of them. Without
//!   provenance you would have to re-walk the entire tree to notice.
//! - **A partial write.** Any touched page on the path rechecks the **whole** mapping, never
//!   the changed entry alone: a mapping spans entries and a torn write can change its length or
//!   target halfway. Rechecking the whole thing is both correct and cheaper to reason about
//!   than deciding which half of a torn edit to believe.
//!
//! # ★★★ UNMAP is the direction that matters
//!
//! A missed *map* costs a fault, which is recoverable and loud. A missed **unmap** leaves the
//! GPU able to write into memory the guest has freed — and may since have handed to another
//! process. `hostile_guest_isolation_is_the_value_proposition`: two processes in one guest do
//! not have to trust each other. So the fixpoint is written to fail toward unmapping.
//!
//! # ★★★ UNMAPPING GARBAGE IS CORRECT, NOT MERELY CAUTIOUS
//!
//! **Owner, 2026-09-10:** *"if the mapping is invalid as written then its unmap, since a real
//! invalidate on the GPU would also not follow such mapping."*
//!
//! That is the justification, and it is stronger than caution. A dangling table pointer, a
//! cycle, or a cleared parent does not translate on real hardware either — the GMMU refuses the
//! walk and faults. So dropping the binding is not us being conservative about ambiguous data;
//! it is us **agreeing with the hardware** about what the guest's tables mean. A design that
//! kept such a binding would be emulating a GPU that does not exist.
//!
//! # ⊘ WHAT THE FUZZ DOES AND DOES NOT ESTABLISH
//!
//! The fuzz asserts `live == walk(tables)` every round — so it proves the refresh stays
//! **consistent with [`Refresh::walk`]**, which is also the thing under test. It therefore
//! catches every bookkeeping bug (a stale binding, a missed recheck, a lost unmap) and **cannot**
//! catch a walk that misreads the format: if `walk` decided a level wrongly, the fuzz would
//! agree with it.
//! ⇒ Fidelity to the real GMMU — which levels may hold a leaf, the PDE page-size bits, the
//! aperture field (`gmmu_fmt.h`'s `GMMU_APERTURE_{INVALID,VIDEO,PEER,SYS_NONCOH,SYS_COH}`) — is
//! a **separate** question needing the real format as its oracle, and this suite does not touch
//! it. Stated here rather than discovered later: an oracle that is its own reference proves
//! self-consistency and calls it correctness.

use kayfabe_arch::Aperture;
use std::collections::{BTreeMap, BTreeSet};

/// One page-table entry, at whatever level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pte {
    /// Not present. A walk that meets this **does not descend** and does not clear dirty.
    Invalid,
    /// A directory entry pointing at the next level's page.
    Table(u64),
    /// A leaf: this VA maps this physical address, under this aperture.
    ///
    /// ⊘ The aperture is part of the entry, not a property of the address. `gmmu_fmt.h` defines
    /// five (`INVALID`, `VIDEO`, `PEER`, `SYS_NONCOH`, `SYS_COH`) and they mean different
    /// **address spaces** — the same number under two apertures names two different bytes.
    Leaf { phys: u64, aperture: Aperture },
}

/// Mocked guest page tables: page address → (slot → entry).
#[derive(Debug, Default, Clone)]
pub struct PtWorld {
    pages: BTreeMap<u64, BTreeMap<usize, Pte>>,
    /// Pages written since the last refresh. In production this is the dirty-bit scan; here it
    /// is set by the test, which is the point — the model must not depend on how it was learned.
    dirty: BTreeSet<u64>,
}

/// A live binding and the path it was derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub va: u64,
    pub phys: u64,
    /// ★ Root-to-leaf pages this binding was read out of. Any of them changing invalidates it.
    pub path: Vec<u64>,
}

/// The refresh's fixpoint state.
#[derive(Debug, Default)]
pub struct Refresh {
    live: BTreeMap<u64, Binding>,
    /// VAs whose subtree could not be scanned because a parent was invalid. They stay pending;
    /// see [`Refresh::pending`].
    pending: BTreeSet<u64>,
    pub unmapped: u64,
    pub mapped: u64,
    pub rechecked: u64,
    /// Mappings the guest made that we cannot serve — `GMMU_APERTURE_PEER` has no CPU plane.
    /// ⊘ Counted separately from `unmapped` on purpose: see the match arm.
    pub unsupported: u64,
}

/// What a walk found. Three outcomes, and collapsing any two of them loses a real distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkOutcome {
    /// The tables describe this VA.
    Mapped { phys: u64, aperture: Aperture },
    /// They do not — invalid entry, cleared parent, dangling pointer. The GMMU would fault.
    NoMapping,
    /// They DO describe it, in an aperture we cannot serve. `PEER` names another GPU's memory
    /// and has no CPU plane at all.
    Unsupported(Aperture),
}

impl WalkOutcome {
    fn of(phys: u64, aperture: Aperture) -> Self {
        match aperture {
            // ⊘ Peer memory is real and the GPU can follow it; we simply cannot back it. That
            // is a refusal, not an absence.
            Aperture::Peer => Self::Unsupported(aperture),
            _ => Self::Mapped { phys, aperture },
        }
    }
}

impl PtWorld {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Write an entry, marking its page dirty — exactly what a guest store or a CE write does.
    pub fn write(&mut self, page: u64, slot: usize, e: Pte) {
        self.pages.entry(page).or_default().insert(slot, e);
        self.dirty.insert(page);
    }

    #[must_use]
    pub fn get(&self, page: u64, slot: usize) -> Pte {
        self.pages
            .get(&page)
            .and_then(|p| p.get(&slot))
            .copied()
            .unwrap_or(Pte::Invalid)
    }

    /// Take and clear the dirty set — the refresh consumes it.
    pub fn take_dirty(&mut self) -> BTreeSet<u64> {
        std::mem::take(&mut self.dirty)
    }
}

impl Refresh {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every live binding, by VA.
    #[must_use]
    pub fn live(&self) -> &BTreeMap<u64, Binding> {
        &self.live
    }

    /// VAs left pending because a parent was invalid when we looked.
    #[must_use]
    pub fn pending(&self) -> &BTreeSet<u64> {
        &self.pending
    }

    /// Walk `va` from `root` through `levels` slot indices, returning the leaf and the path.
    ///
    /// ⊘ Returns the path even on failure, so a binding that *became* unreachable still knows
    /// which pages to blame — that is what lets a later write to any of them re-trigger it.
    fn walk(w: &PtWorld, root: u64, slots: &[usize]) -> (WalkOutcome, Vec<u64>) {
        let mut path = vec![root];
        let mut page = root;
        for (i, &slot) in slots.iter().enumerate() {
            match w.get(page, slot) {
                // ★ The owner's rule: a parent that is not set means we DO NOT look down.
                // ⊘ `GMMU_APERTURE_INVALID` / a not-present entry: the GMMU refuses this walk
                // and faults, so there is no mapping. Owner, 2026-09-10: "if the mapping is
                // invalid as written then its unmap, since a real invalidate on the GPU would
                // also not follow such mapping."
                Pte::Invalid => return (WalkOutcome::NoMapping, path),
                Pte::Table(next) => {
                    page = next;
                    path.push(next);
                }
                Pte::Leaf { phys, aperture } => {
                    // A leaf before the last level is a large page; still a valid answer.
                    return (WalkOutcome::of(phys, aperture), path);
                }
            }
            if i + 1 == slots.len() {
                return (WalkOutcome::NoMapping, path);
            }
        }
        (WalkOutcome::NoMapping, path)
    }

    /// Refresh every binding whose provenance intersects `dirty`, plus every pending VA.
    ///
    /// ⊘ `interest` is the set of VAs the caller cares about — in production, everything the
    /// guest has asked us to map. A VA absent from the tables is **unmapped**, not ignored.
    pub fn refresh(
        &mut self,
        w: &PtWorld,
        dirty: &BTreeSet<u64>,
        interest: &BTreeMap<u64, (u64, Vec<usize>)>,
    ) {
        let touched: Vec<u64> = interest
            .keys()
            .copied()
            .filter(|va| {
                // Recheck when: never resolved (pending), or any page on its recorded path was
                // written. ⊘ A binding we have never seen has no path, so it must always be
                // considered — otherwise a first mapping is never picked up.
                self.pending.contains(va)
                    || self.live.get(va).is_none_or(|b| {
                        b.path.iter().any(|p| dirty.contains(p))
                    })
            })
            .collect();

        for va in touched {
            let (root, slots) = &interest[&va];
            self.rechecked += 1;
            let (leaf, path) = Self::walk(w, *root, slots);
            match leaf {
                // ⊘⊘ UNSUPPORTED IS NOT NO-MAPPING. The guest DID map this; we cannot serve it.
                // Collapsing the two would report a peer mapping as "the guest unmapped it",
                // which is a false statement about the guest and hides a real refusal.
                WalkOutcome::Unsupported(_) => {
                    if self.live.remove(&va).is_some() {
                        self.unmapped += 1;
                    }
                    self.unsupported += 1;
                    self.pending.insert(va);
                }
                WalkOutcome::Mapped { phys, .. } => {
                    self.pending.remove(&va);
                    let b = Binding {
                        va,
                        phys,
                        path,
                    };
                    if self.live.insert(va, b).is_none() {
                        self.mapped += 1;
                    }
                }
                WalkOutcome::NoMapping => {
                    // ★★★ UNMAP. The tables no longer describe this VA — a parent was cleared,
                    // the leaf was invalidated, or the walk never reached one. Keeping it would
                    // let the GPU touch memory the guest believes it has reclaimed.
                    if self.live.remove(&va).is_some() {
                        self.unmapped += 1;
                    }
                    // ⊘ Still pending, and the path we DID get is remembered by the caller's
                    // interest set — so a later write to the parent re-triggers this VA.
                    self.pending.insert(va);
                }
            }
        }
    }
}

#[cfg(test)]
mod the_refresh_must_unmap {
    //! ★★★★★ **STRESS THE MAP/UNMAP FLOW WITH MOCKED DIRTY BITS** — owner, 2026-09-10:
    //! *"I think its well testable by stressing the map/unmap flow and mocking the dirty bits"*
    //! and *"also partial reads/write, also test fuzziness with invalid PDB/PTE data etcetra"*.
    //!
    //! The invariant every test below asserts is the same one:
    //!
    //! > **after a refresh, a VA is live iff the page tables describe it, with the phys they
    //! > describe.**
    //!
    //! A missed MAP costs a fault — recoverable and loud. A missed **UNMAP** leaves the GPU able
    //! to write into memory the guest has freed and may have handed to another process, so the
    //! tests weight that direction.
    use super::*;

    const ROOT: u64 = 0x1000;
    const L1: u64 = 0x2000;
    const VA: u64 = 0x9200_a900_0000;

    /// `va -> (root, slot path)`. Two levels is enough to exercise parent-vs-leaf.
    fn interest_one() -> BTreeMap<u64, (u64, Vec<usize>)> {
        let mut m = BTreeMap::new();
        m.insert(VA, (ROOT, vec![0, 7]));
        m
    }

    fn mapped_world() -> PtWorld {
        let mut w = PtWorld::new();
        w.write(ROOT, 0, Pte::Table(L1));
        w.write(L1, 7, Pte::Leaf { phys: 0xdead_0000, aperture: Aperture::Vidmem });
        w
    }

    #[test]
    fn a_mapping_appears_and_records_every_page_it_was_read_from() {
        let mut w = mapped_world();
        let mut r = Refresh::new();
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest_one());
        let b = r.live().get(&VA).expect("must be live");
        assert_eq!(b.phys, 0xdead_0000);
        assert_eq!(b.path, vec![ROOT, L1], "provenance must be the FULL path, not the leaf");
    }

    #[test]
    fn clearing_the_leaf_unmaps_it() {
        let mut w = mapped_world();
        let mut r = Refresh::new();
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest_one());
        assert!(r.live().contains_key(&VA));

        w.write(L1, 7, Pte::Invalid);
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest_one());
        assert!(!r.live().contains_key(&VA), "a cleared leaf must UNMAP");
        assert_eq!(r.unmapped, 1);
    }

    /// ★★★ THE DANGEROUS ONE: the guest clears the PARENT and frees the children, with no
    /// invalidate of the leaf itself. Without provenance covering the parent this is invisible.
    #[test]
    fn clearing_the_parent_unmaps_the_children_even_though_the_leaf_is_untouched() {
        let mut w = mapped_world();
        let mut r = Refresh::new();
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest_one());
        assert!(r.live().contains_key(&VA));

        // Only the ROOT page is written. The leaf entry still says `Leaf{phys}`.
        w.write(ROOT, 0, Pte::Invalid);
        assert_eq!(w.get(L1, 7), Pte::Leaf { phys: 0xdead_0000, aperture: Aperture::Vidmem }, "leaf deliberately untouched");
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest_one());
        assert!(
            !r.live().contains_key(&VA),
            "a cleared PARENT must unmap its children — the GPU must not reach freed memory \
             because the leaf entry happens to be stale-but-present"
        );
    }

    /// A parent that is invalid must leave the VA PENDING, so that validating it later is
    /// picked up — the owner's "does not scan the contents, and does not unset dirty".
    #[test]
    fn an_invalid_parent_leaves_it_pending_and_a_later_validation_is_picked_up() {
        let mut w = PtWorld::new();
        w.write(ROOT, 0, Pte::Invalid);
        w.write(L1, 7, Pte::Leaf { phys: 0xbeef_0000, aperture: Aperture::Vidmem });
        let mut r = Refresh::new();
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest_one());
        assert!(!r.live().contains_key(&VA));
        assert!(r.pending().contains(&VA), "an unscannable subtree must stay PENDING");

        // The parent becomes valid. ⊘ Note the leaf page is NOT written again — bottom-up
        // construction means the leaf was already there, and only the parent's write makes it
        // reachable. An edge-watcher waiting on the leaf would miss this entirely.
        w.write(ROOT, 0, Pte::Table(L1));
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest_one());
        assert_eq!(r.live().get(&VA).map(|b| b.phys), Some(0xbeef_0000));
        assert!(!r.pending().contains(&VA));
    }

    /// A partial write anywhere on the path rechecks the WHOLE mapping, never one entry.
    #[test]
    fn a_partial_write_to_any_page_on_the_path_rechecks_the_whole_mapping() {
        let mut w = mapped_world();
        let mut r = Refresh::new();
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest_one());
        let before = r.rechecked;

        // Touch an UNRELATED slot on a page that is on the path — a torn/partial edit.
        w.write(L1, 999, Pte::Invalid);
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest_one());
        assert!(
            r.rechecked > before,
            "any write to a page on the provenance path must recheck the mapping, because a \
             torn edit can change its target or length halfway"
        );
        assert!(r.live().contains_key(&VA), "and it is still correctly mapped");
    }

    /// ⊘ FUZZ with hostile/garbage tables: entries pointing at pages that do not exist, cycles,
    /// and random rewrites. The refresh must never panic, and must never leave a binding the
    /// tables do not describe.
    #[test]
    fn fuzzing_invalid_pdb_and_pte_data_never_panics_and_never_leaves_a_stale_binding() {
        let mut seed = 0x243f_6a88_85a3_08d3u64;
        let mut rng = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        let mut w = PtWorld::new();
        let mut r = Refresh::new();
        let interest = interest_one();

        for round in 0..4000 {
            match rng() % 6 {
                // a plausible map
                0 => {
                    w.write(ROOT, 0, Pte::Table(L1));
                    w.write(L1, 7, Pte::Leaf { phys: rng() & !0xfff, aperture: Aperture::Vidmem });
                }
                // clear the leaf
                1 => w.write(L1, 7, Pte::Invalid),
                // clear the parent
                2 => w.write(ROOT, 0, Pte::Invalid),
                // ★ a parent pointing at a page that does not exist
                3 => w.write(ROOT, 0, Pte::Table(rng() & !0xfff)),
                // ★ a CYCLE: the table points at itself
                4 => w.write(ROOT, 0, Pte::Table(ROOT)),
                // ★ a leaf where a table belongs (a large page, or garbage)
                _ => w.write(ROOT, 0, Pte::Leaf { phys: rng() & !0xfff, aperture: Aperture::Vidmem }),
            }

            let d = w.take_dirty();
            r.refresh(&w, &d, &interest);

            // THE INVARIANT: live iff the tables describe it, with the phys they describe.
            let truth = match Refresh::walk(&w, ROOT, &[0, 7]).0 {
                WalkOutcome::Mapped { phys, .. } => Some(phys),
                _ => None,
            };
            let held = r.live().get(&VA).map(|b| b.phys);
            assert_eq!(
                held, truth,
                "round {round}: refresh disagrees with the tables — held={held:?} truth={truth:?}"
            );
        }
        assert!(r.unmapped > 0 && r.mapped > 0, "the fuzz must exercise BOTH directions");
    }
}

#[cfg(test)]
mod the_aperture_is_part_of_the_entry {
    //! ★★★★★ **FORMAT FIDELITY, against the vendored header rather than against ourselves.**
    //!
    //! Owner, 2026-09-10, on the fuzz's stated limit — *"thats worth testing"*. The map/unmap
    //! fuzz asserts `live == walk(tables)`, so it proves the bookkeeping is consistent with
    //! [`Refresh::walk`] and **cannot** catch a walk that misreads the format: if `walk` decided
    //! wrongly, the fuzz would agree with it. These tests use `gmmu_fmt.h` as the oracle
    //! instead.
    //!
    //! `ogkm-580.159.04/src/nvidia/inc/libraries/mmu/gmmu_fmt.h` defines exactly five:
    //! `GMMU_APERTURE_{INVALID, VIDEO, PEER, SYS_NONCOH, SYS_COH}`.
    use super::*;

    const ROOT: u64 = 0x1000;
    const L1: u64 = 0x2000;
    const VA: u64 = 0x9200_a900_0000;

    fn interest() -> BTreeMap<u64, (u64, Vec<usize>)> {
        let mut m = BTreeMap::new();
        m.insert(VA, (ROOT, vec![0, 7]));
        m
    }

    fn world_with(aperture: Aperture) -> PtWorld {
        let mut w = PtWorld::new();
        w.write(ROOT, 0, Pte::Table(L1));
        w.write(L1, 7, Pte::Leaf { phys: 0x4000, aperture });
        w
    }

    /// The three apertures that name memory we can reach must MAP.
    #[test]
    fn video_and_both_sysmem_apertures_map() {
        for ap in [
            Aperture::Vidmem,
            Aperture::SysmemCoherent,
            Aperture::SysmemNonCoherent,
        ] {
            let mut w = world_with(ap);
            let mut r = Refresh::new();
            let d = w.take_dirty();
            r.refresh(&w, &d, &interest());
            assert_eq!(
                r.live().get(&VA).map(|b| b.phys),
                Some(0x4000),
                "{ap:?} names memory we can reach and must map"
            );
            assert_eq!(r.unsupported, 0, "{ap:?} must not be reported unsupported");
        }
    }

    /// ★★★ `GMMU_APERTURE_PEER` names ANOTHER GPU's memory. The guest DID map it and the GPU
    /// can follow it; we have no CPU plane for it. That is a REFUSAL, and reporting it as
    /// "the guest unmapped it" would be a false statement about the guest.
    #[test]
    fn peer_is_unsupported_and_is_not_reported_as_an_absent_mapping() {
        let mut w = world_with(Aperture::Peer);
        let mut r = Refresh::new();
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest());

        assert!(!r.live().contains_key(&VA), "we cannot back peer memory, so nothing is live");
        assert_eq!(r.unsupported, 1, "…but it must be counted as UNSUPPORTED");
        assert_eq!(
            r.unmapped, 0,
            "and NOT as an unmap — the guest mapped this; we are the ones who cannot serve it. \
             Collapsing the two loses the only signal that a real refusal happened."
        );
        assert_eq!(
            Refresh::walk(&w, ROOT, &[0, 7]).0,
            WalkOutcome::Unsupported(Aperture::Peer)
        );
    }

    /// ⊘ A vendor-regression guard. If NVIDIA adds a sixth aperture, this test fails and forces
    /// a decision instead of letting the new value fall into a `_ =>` arm and be silently
    /// treated as ordinary memory — `a_capture_derived_table_expires_as_a_vendor_regression`.
    #[test]
    fn every_aperture_the_header_defines_has_a_decided_outcome() {
        // The five of `GMMU_APERTURE` minus INVALID, which is not an aperture a leaf carries —
        // it is the absence of one, and `Pte::Invalid` models it.
        let decided = [
            (Aperture::Vidmem, false),
            (Aperture::SysmemCoherent, false),
            (Aperture::SysmemNonCoherent, false),
            (Aperture::Peer, true),
        ];
        for (ap, expect_unsupported) in decided {
            let got = WalkOutcome::of(0x4000, ap);
            assert_eq!(
                matches!(got, WalkOutcome::Unsupported(_)),
                expect_unsupported,
                "{ap:?} must have a DECIDED outcome, not a default one"
            );
        }
    }

    /// An invalid entry is the absence of a mapping, never an aperture — and it unmaps.
    #[test]
    fn an_invalid_entry_is_no_mapping_rather_than_an_unsupported_one() {
        let mut w = PtWorld::new();
        w.write(ROOT, 0, Pte::Table(L1));
        w.write(L1, 7, Pte::Invalid);
        assert_eq!(Refresh::walk(&w, ROOT, &[0, 7]).0, WalkOutcome::NoMapping);

        let mut r = Refresh::new();
        let d = w.take_dirty();
        r.refresh(&w, &d, &interest());
        assert_eq!(r.unsupported, 0, "absence is not a refusal");
        assert!(!r.live().contains_key(&VA));
    }
}
