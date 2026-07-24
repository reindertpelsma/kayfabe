//! A non-overlapping interval map over `u64` keys.
//!
//! This is the data structure behind the per-VAS address table
//! (`mode2_address_table.md`): ranges are inserted **forward** (at bind time),
//! looked up by point, and removed **eagerly**. Two properties are load-bearing:
//!
//! - **Insertion of an overlapping range is a loud error** ([`OverlapError`]),
//!   never a silent merge/override — an overlap at bind time means the guest
//!   re-pointed a range without the unbind we require first (table §5.2
//!   "unmap eager, map lazy"), or that two owners collided (the #14
//!   `ALREADY-MAPPED` class). The *caller* decides policy; the container refuses
//!   to lose information.
//! - **A lookup miss returns `None`** and the caller must treat it as a fault
//!   (MISS=FAULT, table §6) — there is no fallback resolution here by design.

use std::collections::BTreeMap;

/// Error returned when a range cannot be inserted.
///
/// Every variant is LOUD (never a silent merge/drop): the container refuses to lose
/// information. `Overlap` is the bind-time collision class; `Empty`/`Wraps` reject a
/// malformed *hostile* range (guest-controlled `start`/`len` reach this container via
/// the address table — a zero-length or `u64`-wrapping range is refused as a clean
/// error, never a panic, boundary-1 posture).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalError {
    /// The inserted range overlaps an existing one.
    Overlap {
        /// Start of the existing range that overlaps the attempted insert.
        existing_start: u64,
        /// Length of the existing range that overlaps the attempted insert.
        existing_len: u64,
    },
    /// The range had zero length (nothing to map).
    Empty,
    /// `start + len` overflows `u64` (the range wraps the address space).
    Wraps,
}

/// A map from non-overlapping `[start, start+len)` ranges to values.
///
/// Deterministic iteration order (backed by a `BTreeMap`), no interior
/// mutation surprises, purely in-memory.
#[derive(Debug, Clone)]
pub struct IntervalMap<V> {
    /// start -> (len, value); invariant: ranges are disjoint and len > 0.
    ranges: BTreeMap<u64, (u64, V)>,
}

// Manual `Default` so `V: Default` is NOT required (an empty map needs no value).
impl<V> Default for IntervalMap<V> {
    fn default() -> Self {
        IntervalMap { ranges: BTreeMap::new() }
    }
}

impl<V> IntervalMap<V> {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        IntervalMap { ranges: BTreeMap::new() }
    }

    /// Number of ranges.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// True if no ranges are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Insert `[start, start+len)` -> `value`. Fails loudly (never panics) on any of:
    /// a zero-length range ([`IntervalError::Empty`]), a range that wraps `u64`
    /// ([`IntervalError::Wraps`]), or an overlap with an existing range
    /// ([`IntervalError::Overlap`]). `start`/`len` are guest-controlled at the address
    /// table, so a malformed range is a clean `Err`, not an abort (boundary-1 posture).
    pub fn insert(&mut self, start: u64, len: u64, value: V) -> Result<(), IntervalError> {
        if len == 0 {
            return Err(IntervalError::Empty);
        }
        let end = start.checked_add(len).ok_or(IntervalError::Wraps)?;
        // Predecessor may overlap from the left.
        if let Some((&ps, &(plen, _))) = self.ranges.range(..=start).next_back()
            && let (ps, Some(pe)) = (ps, ps.checked_add(plen))
            && pe > start
        {
            return Err(IntervalError::Overlap { existing_start: ps, existing_len: plen });
        }
        // Successor may overlap from the right.
        if let Some((&ns, &(nlen, _))) = self.ranges.range(start..).next()
            && ns < end
        {
            return Err(IntervalError::Overlap { existing_start: ns, existing_len: nlen });
        }
        self.ranges.insert(start, (len, value));
        Ok(())
    }

    /// Point lookup: the range containing `point`, if any.
    ///
    /// Returns `(range_start, range_len, &value)`. A `None` here is a MISS and
    /// the caller MUST fault loudly — never fall back to a heuristic resolve.
    #[must_use]
    pub fn lookup(&self, point: u64) -> Option<(u64, u64, &V)> {
        let (&start, &(len, ref v)) = self.ranges.range(..=point).next_back()?;
        if point < start.checked_add(len)? { Some((start, len, v)) } else { None }
    }

    /// Remove the range that starts exactly at `start` (the eager-unmap path).
    /// Returns the removed value, or `None` if no range starts there.
    pub fn remove_at(&mut self, start: u64) -> Option<(u64, V)> {
        self.ranges.remove(&start)
    }

    /// Iterate ranges in ascending start order as `(start, len, &value)`.
    pub fn iter(&self) -> impl Iterator<Item = (u64, u64, &V)> {
        self.ranges.iter().map(|(&s, &(l, ref v))| (s, l, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_lookup_remove_roundtrip() {
        let mut m = IntervalMap::new();
        m.insert(0x1000, 0x1000, "a").unwrap();
        m.insert(0x3000, 0x800, "b").unwrap();
        assert_eq!(m.lookup(0x1000), Some((0x1000, 0x1000, &"a")));
        assert_eq!(m.lookup(0x1fff), Some((0x1000, 0x1000, &"a")));
        assert_eq!(m.lookup(0x2000), None, "gap between ranges is a MISS");
        assert_eq!(m.lookup(0x37ff), Some((0x3000, 0x800, &"b")));
        assert_eq!(m.remove_at(0x1000), Some((0x1000, "a")));
        assert_eq!(m.lookup(0x1000), None, "removed range is a MISS (unmap eager)");
    }

    #[test]
    fn overlap_is_a_loud_error_never_silent() {
        let mut m = IntervalMap::new();
        m.insert(0x1000, 0x1000, ()).unwrap();
        // Left, right, containing, contained: all must refuse.
        for (s, l) in [(0x800, 0x900), (0x1fff, 0x10), (0x0, 0x10000), (0x1400, 0x100)] {
            assert_eq!(
                m.insert(s, l, ()),
                Err(IntervalError::Overlap { existing_start: 0x1000, existing_len: 0x1000 }),
                "overlap ({s:#x},{l:#x}) must be refused"
            );
        }
        // Exactly adjacent is fine.
        m.insert(0x0, 0x1000, ()).unwrap();
        m.insert(0x2000, 0x1000, ()).unwrap();
    }

    /// A hostile, malformed range is a clean `Err`, never a panic: a zero-length
    /// range and a `u64`-wrapping range (guest-controlled `start`/`len` reach here
    /// through the address table) are both refused loudly.
    #[test]
    fn malformed_range_is_a_loud_error_never_a_panic() {
        let mut m: IntervalMap<()> = IntervalMap::new();
        assert_eq!(m.insert(0x1000, 0, ()), Err(IntervalError::Empty), "zero-len refused");
        assert_eq!(
            m.insert(u64::MAX - 0x100, 0x1000, ()),
            Err(IntervalError::Wraps),
            "a range wrapping u64 is refused, not a panic"
        );
        assert_eq!(m.insert(u64::MAX, 1, ()), Err(IntervalError::Wraps), "last-byte wrap refused");
        // The maximal non-wrapping range (ending exactly at u64::MAX) is accepted.
        m.insert(u64::MAX - 0x1000, 0x1000, ()).unwrap();
    }
}
