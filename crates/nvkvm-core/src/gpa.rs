//! Per-process GPA arenas (arch doc §4.3.3).
//!
//! [`GpaSpace`] owns the device's guest-physical window; each `Proc` receives a
//! [`GpaArena`] — a contiguous sub-range with its own allocator. Two processes'
//! identical guest VAs therefore land in **disjoint GPA (and host-backing) ranges by
//! construction** — the `back_sys ALREADY-MAPPED` collision class (#14, C cracks
//! ⚠6/⚠10) cannot occur. Arenas are sparse *reservations* (the backing VMM
//! demand-faults), so per-proc cost is address space, not RAM.

use core::ops::Range;
use nvkvm_arch::ids::Gpa;

/// Errors from the GPA window/arena allocators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpaError {
    /// The window has no room for another arena.
    WindowExhausted,
    /// The arena has no room for the requested allocation.
    ArenaExhausted {
        /// Requested length.
        len: u64,
    },
}

/// The device's guest-physical window; hands out per-proc arenas.
#[derive(Debug)]
pub struct GpaSpace {
    window: Range<u64>,
    arena_len: u64,
    next: u64,
    /// Arenas released at a reap point, ready for recycling (LIFO).
    free: Vec<Range<u64>>,
}

impl GpaSpace {
    /// A window carving fixed-size arenas of `arena_len` bytes.
    #[must_use]
    pub fn new(window: Range<u64>, arena_len: u64) -> Self {
        assert!(arena_len > 0 && window.start < window.end);
        let next = window.start;
        GpaSpace { window, arena_len, next, free: Vec::new() }
    }

    /// Carve a disjoint arena: recycle a released one first, else cut fresh from
    /// the window. Recycling is what makes the device-lifecycle sustainable — the
    /// C paid for this with #80's GPA free-list after sequential process churn
    /// exhausted the shared window (`teardown_hardening_done`).
    pub fn carve(&mut self) -> Result<GpaArena, GpaError> {
        if let Some(range) = self.free.pop() {
            return Ok(GpaArena { cursor: range.start, range });
        }
        let start = self.next;
        let end = start.checked_add(self.arena_len).ok_or(GpaError::WindowExhausted)?;
        if end > self.window.end {
            return Err(GpaError::WindowExhausted);
        }
        self.next = end;
        Ok(GpaArena { range: start..end, cursor: start })
    }

    /// This window's guest-physical range (used to mint a fresh, disjoint per-target
    /// window for another GPU — MG-6).
    #[must_use]
    pub fn window(&self) -> Range<u64> {
        self.window.clone()
    }

    /// The fixed arena size this window carves (cloned into a new target's geometry).
    #[must_use]
    pub fn arena_len(&self) -> u64 {
        self.arena_len
    }

    /// Return a carved arena to the window for recycling. Takes the arena **by
    /// value**: releasing an arena a live `Proc` still owns is unrepresentable —
    /// only a reaped proc's arena (moved out of the dropped `Proc` at the quiesce
    /// point, `Gpu::reap_retired`) can arrive here. Recycled GPAs are safe by
    /// construction: the dead proc's host mappings died with its isolate session,
    /// and its address tables died with its `Vas`es — the C's stale-backing /
    /// `ALREADY-MAPPED`-on-reuse class (#12 cont.29) has nothing to be stale.
    pub fn release(&mut self, arena: GpaArena) {
        debug_assert_eq!(arena.range.end - arena.range.start, self.arena_len);
        debug_assert!(arena.range.start >= self.window.start && arena.range.end <= self.next);
        self.free.push(arena.range);
    }
}

/// One process's private slice of the guest-physical window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpaArena {
    /// The arena's disjoint GPA range.
    pub range: Range<u64>,
    cursor: u64,
}

impl GpaArena {
    /// Bump-allocate `len` bytes at `align` (power of two).
    pub fn alloc(&mut self, len: u64, align: u64) -> Result<Gpa, GpaError> {
        assert!(align.is_power_of_two() && len > 0);
        let start = self
            .cursor
            .checked_add(align - 1)
            .map(|c| c & !(align - 1))
            .ok_or(GpaError::ArenaExhausted { len })?;
        let end = start.checked_add(len).ok_or(GpaError::ArenaExhausted { len })?;
        if end > self.range.end {
            return Err(GpaError::ArenaExhausted { len });
        }
        self.cursor = end;
        Ok(Gpa(start))
    }

    /// True if nothing was ever allocated (used to enforce the early-arm merge
    /// discipline: merging two `Proc`s is only legal while the absorbed one is
    /// untouched — lesson L9).
    #[must_use]
    pub fn is_untouched(&self) -> bool {
        self.cursor == self.range.start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// testing strategy §2.3 `t14_already_mapped_arena` (allocator half): two arenas
    /// are disjoint by construction; identical allocation sequences in each yield
    /// disjoint GPAs.
    #[test]
    fn t14_arena_disjoint_by_construction() {
        let mut space = GpaSpace::new(0x1_0000_0000..0x5_0000_0000, 0x2_0000_0000);
        let mut a = space.carve().unwrap();
        let mut b = space.carve().unwrap();
        assert!(a.range.end <= b.range.start || b.range.end <= a.range.start);
        // Identical request streams (two identical guest processes)…
        let ga = a.alloc(0x10000, 0x1000).unwrap();
        let gb = b.alloc(0x10000, 0x1000).unwrap();
        assert_ne!(ga, gb, "identical requests must land at disjoint GPAs");
        assert!(space.carve().is_err(), "window exhaustion is loud");
    }

    /// ★ Mutation-gate kill (`GpaArena::alloc` `end > range.end`→`==`/`>=`): the arena
    /// bound is inclusive of a perfect fill — an allocation whose end lands EXACTLY on
    /// `range.end` must SUCCEED (the last usable byte is available), while any allocation
    /// that would run one byte past must FAIL loudly. The prior suite only ever made
    /// small sub-arena allocations, so the exact-fill boundary was never exercised: a
    /// `>`→`>=` mutant (rejecting the perfect fill) and a `>`→`==` mutant (accepting an
    /// over-run) both survived. This pins the exact boundary from both sides.
    #[test]
    fn arena_alloc_boundary_exact_fill_ok_overrun_loud() {
        let mut space = GpaSpace::new(0..0x10000, 0x10000);
        let mut a = space.carve().unwrap();
        let len = a.range.end - a.range.start; // exactly the whole arena
        // Exact fill: end == range.end must SUCCEED (kills `>`→`>=`).
        let g = a.alloc(len, 0x1000).expect("an exact-fill allocation is legal");
        assert_eq!(g, Gpa(a.range.start));
        // Now the arena is full; a further byte over-runs and must FAIL loudly
        // (kills `>`→`==`, which would accept a one-past-end allocation).
        assert!(
            matches!(a.alloc(1, 1), Err(GpaError::ArenaExhausted { .. })),
            "an allocation past the exact fill is a loud fault, never silently accepted",
        );

        // And an over-run in one shot (len one past the arena) is also loud — the `==`
        // mutant would only reject the exact fill, letting this strictly-larger request
        // through.
        let mut space2 = GpaSpace::new(0..0x10000, 0x10000);
        let mut b = space2.carve().unwrap();
        let over = (b.range.end - b.range.start) + 0x1000;
        assert!(
            matches!(b.alloc(over, 0x1000), Err(GpaError::ArenaExhausted { .. })),
            "an allocation strictly past range.end is a loud fault",
        );
    }

    /// ★ Mutation-gate kill (`GpaArena::is_untouched`→`true`): the early-arm merge
    /// discipline (lesson L9) depends on this predicate distinguishing a pristine arena
    /// from one that has carved even a single allocation — merging is legal ONLY while
    /// the absorbed proc is untouched. The prior suite never asserted it, so a
    /// "always untouched" mutant survived (it would let a touched proc be silently
    /// merge-absorbed). A fresh arena is untouched; after ANY alloc it is not.
    #[test]
    fn is_untouched_flips_on_first_allocation() {
        let mut space = GpaSpace::new(0..0x10000, 0x10000);
        let mut a = space.carve().unwrap();
        assert!(a.is_untouched(), "a freshly carved arena has allocated nothing");
        a.alloc(0x1000, 0x1000).unwrap();
        assert!(!a.is_untouched(), "after one allocation the arena is touched");
    }
}
