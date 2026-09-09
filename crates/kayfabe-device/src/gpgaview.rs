//! ★★★★★ **ONE GPGA, MANY VIEWS — the registry that makes aliasing a FACT.**
//!
//! **Owner, 2026-09-09:** *"remember same gpga be mapped multiple times in vmm, multiple times
//! in same isolate, across isolates, in scratchpad, and just be allocated with no references."*
//!
//! Five states, one range, all legal and all simultaneous. The tree has been treating that
//! multiplicity as an accident to be discovered in a debugger; here it is a recorded fact.
//!
//! # ⊘ Why this exists at all — two defects, both already paid for
//!
//! - **A bare address carries no lifetime.** `SparseFb::release_join` *dropped the bytes a join
//!   held*; only `release_join_carrying_bytes` preserves them. Anything holding the old address
//!   kept a pointer to nothing.
//! - **An unrecorded alias corrupts silently.** The LLM's sixteen tokens of garbage were a frame
//!   host-backed at **one** VA while the guest aliased **seventeen** at **two**. Nothing printed
//!   the alias, so nothing could contradict the assumption that there was one.
//!
//! ⇒ Views are handed out as **guards**, and every live view of a range is enumerable.
//!
//! # ⊘ What this is NOT
//!
//! It does not map anything. It is the *bookkeeping* — who holds a view of what, in which
//! space — so the mapping layer can stay backing-specific while callers stay backing-agnostic.
//! Keeping the registry separate from the mapper is deliberate: the registry is pure and
//! testable without a VMM, an isolate, or a GPU.

use std::collections::BTreeMap;

/// Where a view lives. The same GPGA may be viewed in several of these at once, and several
/// times within one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViewSpace {
    /// The VMM's own address space (a CPU-side view — fake FB returns host RAM; real FB maps
    /// the MMIO window).
    Vmm,
    /// One isolate's address space. Isolates are separate processes, so `Isolate(1)` and
    /// `Isolate(2)` are unrelated spaces that may both view one range.
    Isolate(u32),
    /// The scratchpad — where real GPU work is staged.
    Scratchpad,
    /// The guest's own MMIO aperture (BAR1). ⊘ Fake FB may legitimately appear here at several
    /// addresses at once; that is the aliasing the census exists to print.
    GuestMmio,
}

/// A live view of a GPGA range. Dropping it releases the view.
///
/// ⊘ `#[must_use]`: a view created and immediately dropped is almost always a bug at the call
/// site — the caller wanted the address, took the guard, and let the lifetime end underneath it.
#[derive(Debug)]
#[must_use = "a view guard released immediately makes the address it returned dangle"]
pub struct ViewGuard {
    space: ViewSpace,
    gpga: u64,
    len: u64,
    /// The address the view is reachable at IN ITS OWN SPACE. Meaningless across spaces.
    at: u64,
    /// ⊘ Whether this guard has been released. Read by [`ViewGuard::is_live`] rather than
    /// deleted: a guard that was released and then used is a real bug, and keeping the flag is
    /// what lets a caller assert against it instead of dereferencing a stale address.
    live: bool,
}

impl ViewGuard {
    /// The address this view is reachable at, in its own space.
    #[must_use]
    pub const fn at(&self) -> u64 {
        self.at
    }
    /// Which space this view lives in.
    #[must_use]
    pub const fn space(&self) -> ViewSpace {
        self.space
    }
    /// The GPGA this is a view OF.
    #[must_use]
    pub const fn gpga(&self) -> u64 {
        self.gpga
    }
    /// Bytes viewed.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.len
    }
    /// Whether this guard is still held. `false` after [`GpgaViews::release`].
    #[must_use]
    pub const fn is_live(&self) -> bool {
        self.live
    }
    /// ⊘ Clippy asks for this beside `len`; a zero-length view is refused at `map`, so it is
    /// always false — stated rather than omitted so nobody adds a zero-length path later.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// One recorded view, for the census.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub space: ViewSpace,
    pub at: u64,
    pub len: u64,
}

/// A range's backing, and every live view of it.
#[derive(Debug, Default, Clone)]
struct Entry {
    /// ⊘ A range may exist with NO views — the owner's fifth case, "just be allocated with no
    /// references". That is not a leak and must not be reaped as one.
    views: Vec<View>,
    /// Whether the backing was explicitly allocated. Distinguishes "allocated, unreferenced"
    /// from "never existed", which a `views.is_empty()` test alone cannot.
    allocated: bool,
}

/// Who holds a view of which GPGA, in which space.
#[derive(Debug, Default)]
pub struct GpgaViews {
    by_gpga: BTreeMap<u64, Entry>,
    /// Views whose `Drop` ran after the table was gone. Counted, never panicked — a teardown
    /// ordering issue must not abort a boot.
    orphan_releases: u64,
}

/// Why a `map` was refused. ⊘ Named rather than `Option`: "you asked for nothing" and "that
/// range does not exist" are different mistakes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewRefused {
    /// A zero-length view is meaningless and is almost always a computed-length bug upstream.
    ZeroLength,
}

impl GpgaViews {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `gpga` exists with a backing, with no views yet.
    ///
    /// ⊘ The owner's fifth case. An allocated range with zero views is a legitimate steady
    /// state, and telling it apart from "never existed" is why `allocated` is a field rather
    /// than an inference from an empty view list.
    pub fn allocate(&mut self, gpga: u64) {
        self.by_gpga.entry(gpga).or_default().allocated = true;
    }

    /// Take a view of `gpga` in `space`, reachable at `at`.
    ///
    /// # Errors
    /// [`ViewRefused::ZeroLength`] for `len == 0`.
    pub fn map(
        &mut self,
        gpga: u64,
        len: u64,
        space: ViewSpace,
        at: u64,
    ) -> Result<ViewGuard, ViewRefused> {
        if len == 0 {
            return Err(ViewRefused::ZeroLength);
        }
        let e = self.by_gpga.entry(gpga).or_default();
        e.views.push(View { space, at, len });
        Ok(ViewGuard {
            space,
            gpga,
            len,
            at,
            live: true,
        })
    }

    /// Release a view. Called by [`ViewGuard`]'s owner; see [`GpgaViews::release`].
    fn drop_view(&mut self, g: &ViewGuard) {
        let Some(e) = self.by_gpga.get_mut(&g.gpga) else {
            self.orphan_releases += 1;
            return;
        };
        if let Some(i) = e
            .views
            .iter()
            .position(|v| v.space == g.space && v.at == g.at && v.len == g.len)
        {
            e.views.remove(i);
        } else {
            self.orphan_releases += 1;
        }
    }

    /// Release a guard against this table.
    ///
    /// ⊘ Explicit rather than a `Drop` impl on the guard: the guard would need a back-reference
    /// to the table, which means shared ownership and a lock taken at drop time — on whatever
    /// thread happens to drop it, possibly a vCPU inside a trap. Releasing explicitly keeps the
    /// table's locking the caller's decision, which is the whole point of the blocking rules.
    pub fn release(&mut self, mut g: ViewGuard) {
        self.drop_view(&g);
        g.live = false;
    }

    /// Every live view of `gpga`.
    #[must_use]
    pub fn views_of(&self, gpga: u64) -> &[View] {
        self.by_gpga.get(&gpga).map_or(&[], |e| e.views.as_slice())
    }

    /// How many live views `gpga` has, across every space.
    #[must_use]
    pub fn refcount(&self, gpga: u64) -> usize {
        self.views_of(gpga).len()
    }

    /// Ranges viewed from more than one place — the aliasing the LLM corruption turned on.
    ///
    /// ⊘ Aliasing is **legal**, so this is a census and not a refusal. What was missing before
    /// was not a prohibition but the ability to say it was happening.
    #[must_use]
    pub fn aliased(&self) -> Vec<(u64, usize)> {
        self.by_gpga
            .iter()
            .filter(|(_, e)| e.views.len() > 1)
            .map(|(g, e)| (*g, e.views.len()))
            .collect()
    }

    /// Allocated ranges with no live view — legitimate, and worth printing so it is never
    /// mistaken for a leak.
    #[must_use]
    pub fn allocated_unviewed(&self) -> Vec<u64> {
        self.by_gpga
            .iter()
            .filter(|(_, e)| e.allocated && e.views.is_empty())
            .map(|(g, _)| *g)
            .collect()
    }

    /// One line for the boot report.
    #[must_use]
    pub fn census(&self) -> String {
        let aliased = self.aliased();
        let unviewed = self.allocated_unviewed();
        format!(
            "GPGA-VIEWS ranges={} aliased={} allocated_unviewed={} orphan_releases={}{} \
             ⊘ aliasing is LEGAL — this names it so an assumption of one view can be \
             contradicted; allocated_unviewed is a steady state, NOT a leak",
            self.by_gpga.len(),
            aliased.len(),
            unviewed.len(),
            self.orphan_releases,
            if aliased.is_empty() {
                String::new()
            } else {
                format!(
                    " [{}]",
                    aliased
                        .iter()
                        .take(6)
                        .map(|(g, n)| format!("{g:#x}×{n}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            }
        )
    }
}

#[cfg(test)]
mod one_gpga_many_views {
    //! ★★★★★ **THE OWNER'S FIVE CASES, 2026-09-09**, each a test:
    //!
    //! > *"same gpga be mapped multiple times in vmm, multiple times in same isolate, across
    //! > isolates, in scratchpad, and just be allocated with no references."*
    //!
    //! All five are legal and may hold simultaneously. A design that treats any of them as an
    //! error is wrong about the hardware, and one that cannot SEE them is how the LLM's
    //! aliasing corruption went unnoticed for a campaign.
    use super::*;

    const G: u64 = 0x9200_a900_0000;

    #[test]
    fn one_range_viewed_in_every_space_at_once() {
        let mut t = GpgaViews::new();
        let a = t.map(G, 0x1000, ViewSpace::Vmm, 0x7f00_0000).unwrap();
        let b = t.map(G, 0x1000, ViewSpace::Vmm, 0x7f10_0000).unwrap();
        let c = t.map(G, 0x1000, ViewSpace::Isolate(1), 0x40_0000).unwrap();
        let d = t.map(G, 0x1000, ViewSpace::Isolate(1), 0x41_0000).unwrap();
        let e = t.map(G, 0x1000, ViewSpace::Isolate(2), 0x40_0000).unwrap();
        let f = t.map(G, 0x1000, ViewSpace::Scratchpad, 0x10_0000).unwrap();
        let h = t.map(G, 0x1000, ViewSpace::GuestMmio, 0xe000_0000).unwrap();

        // twice in the VMM, twice in ONE isolate, once in a SECOND isolate, scratchpad, MMIO
        assert_eq!(t.refcount(G), 7);
        assert_eq!(t.aliased(), vec![(G, 7)]);

        // ⊘ Two views in the same space at different addresses are DISTINCT, not a duplicate:
        // that is exactly the fake-FB-mapped-twice case, and collapsing them would recreate
        // the bug this table exists to expose.
        let vmm: Vec<_> = t
            .views_of(G)
            .iter()
            .filter(|v| v.space == ViewSpace::Vmm)
            .map(|v| v.at)
            .collect();
        assert_eq!(vmm, vec![0x7f00_0000, 0x7f10_0000]);

        for g in [a, b, c, d, e, f, h] {
            t.release(g);
        }
        assert_eq!(t.refcount(G), 0);
        assert_eq!(t.aliased(), vec![]);
    }

    /// The fifth case. ⊘ Zero views is a legitimate steady state and must be distinguishable
    /// from "never existed" — a reaper that cannot tell them apart frees live backings.
    #[test]
    fn allocated_with_no_references_is_a_state_not_a_leak() {
        let mut t = GpgaViews::new();
        t.allocate(G);
        assert_eq!(t.refcount(G), 0);
        assert_eq!(t.allocated_unviewed(), vec![G]);
        assert!(t.census().contains("allocated_unviewed=1"));

        // …and a never-allocated range is NOT reported as an unviewed allocation.
        assert!(!t.allocated_unviewed().contains(&0xdead_0000));
    }

    /// Releasing one view of a range must not disturb its siblings — the refcount bug that
    /// would surface as "the mapping vanished under a thread that still held it".
    #[test]
    fn releasing_one_view_leaves_the_others_alone() {
        let mut t = GpgaViews::new();
        let a = t.map(G, 0x1000, ViewSpace::Isolate(1), 0x1000).unwrap();
        let b = t.map(G, 0x1000, ViewSpace::Isolate(1), 0x2000).unwrap();
        t.release(a);
        assert_eq!(t.refcount(G), 1);
        assert_eq!(t.views_of(G)[0].at, 0x2000, "the survivor must be b, not a");
        t.release(b);
        assert_eq!(t.refcount(G), 0);
    }

    /// ⊘ A zero-length view is refused BY NAME. It is nearly always a computed length that came
    /// out zero upstream, and silently returning a valid-looking guard over nothing is how such
    /// a bug reaches a memcpy.
    #[test]
    fn a_zero_length_view_is_refused_by_name() {
        let mut t = GpgaViews::new();
        assert_eq!(
            t.map(G, 0, ViewSpace::Vmm, 0x1000).err(),
            Some(ViewRefused::ZeroLength)
        );
        assert_eq!(t.refcount(G), 0);
    }

    /// ⊘ A release against a table that never knew the view is COUNTED, not panicked: teardown
    /// ordering must not abort a boot, and a silent ignore would hide a real accounting bug.
    #[test]
    fn an_unknown_release_is_counted_rather_than_ignored_or_fatal() {
        let mut t = GpgaViews::new();
        let g = t.map(G, 0x1000, ViewSpace::Vmm, 0x1000).unwrap();
        let mut other = GpgaViews::new();
        other.release(g);
        assert!(other.census().contains("orphan_releases=1"), "{}", other.census());
    }
}
