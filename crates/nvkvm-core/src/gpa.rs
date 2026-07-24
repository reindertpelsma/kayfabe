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
}

impl GpaSpace {
    /// A window carving fixed-size arenas of `arena_len` bytes.
    #[must_use]
    pub fn new(window: Range<u64>, arena_len: u64) -> Self {
        assert!(arena_len > 0 && window.start < window.end);
        let next = window.start;
        GpaSpace { window, arena_len, next }
    }

    /// Carve the next disjoint arena. (Freed-arena recycling is a later concern —
    /// tracked with the reap/quiesce design, lesson L10.)
    pub fn carve(&mut self) -> Result<GpaArena, GpaError> {
        let start = self.next;
        let end = start.checked_add(self.arena_len).ok_or(GpaError::WindowExhausted)?;
        if end > self.window.end {
            return Err(GpaError::WindowExhausted);
        }
        self.next = end;
        Ok(GpaArena { range: start..end, cursor: start })
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
}
