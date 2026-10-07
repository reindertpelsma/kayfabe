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
/// One run of [`IntervalMap::spans`]' partition: `(start, len, answer)`, where the answer is
/// `None` for a hole and `Some((value, offset-of-this-run-within-its-range))` for a covered
/// run.
///
/// ⊘ A named type rather than the tuple written out, because the third element is where the
/// meaning is: a run generally begins *inside* a range, and a caller that took the value
/// without the offset would hold the description of a different byte. See
/// [`IntervalMap::spans`] for why the offset is returned rather than re-derived.
pub type SpanRun<'a, V> = (u64, u64, Option<(&'a V, u64)>);

impl<V> Default for IntervalMap<V> {
    fn default() -> Self {
        IntervalMap {
            ranges: BTreeMap::new(),
        }
    }
}

impl<V> IntervalMap<V> {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        IntervalMap {
            ranges: BTreeMap::new(),
        }
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
            return Err(IntervalError::Overlap {
                existing_start: ps,
                existing_len: plen,
            });
        }
        // Successor may overlap from the right.
        if let Some((&ns, &(nlen, _))) = self.ranges.range(start..).next()
            && ns < end
        {
            return Err(IntervalError::Overlap {
                existing_start: ns,
                existing_len: nlen,
            });
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
        if point < start.checked_add(len)? {
            Some((start, len, v))
        } else {
            None
        }
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

    /// The first range that **starts at or after** `at`, if any. The scan-forward
    /// primitive [`IntervalMap::spans`] needs to size a hole without walking the whole
    /// map — `BTreeMap::range` makes it `O(log n)`, where a `find` over
    /// [`IntervalMap::iter`] is `O(n)` per hole.
    #[must_use]
    pub fn next_from(&self, at: u64) -> Option<(u64, u64, &V)> {
        self.ranges
            .range(at..)
            .next()
            .map(|(&s, &(l, ref v))| (s, l, v))
    }

    /// ★★★ **THE range algebra's one primitive** — partition `[start, start+len)` into
    /// the maximal runs over which this map's answer is CONSTANT: each run either lies
    /// inside exactly one range (`Some`) or is a hole (`None`).
    ///
    /// This lives on the container, not on one of its users, because
    /// `gpga_address_space.md` §5 rules that the GPGA allocator and the copy-engine
    /// operand split must share **one** range type: *"Build it once. If the CE split and
    /// the GPGA allocator grow two different range types, that is the smell that the
    /// construct was missed."* `kayfabe_mmu::AddressTable::spans` was the first user and
    /// now delegates here; `kayfabe_mmu::gpga` is the second.
    ///
    /// # Guarantees (all pinned by test)
    /// - Ascending, contiguous, non-overlapping, **no zero-length span**.
    /// - **Total**: the spans cover the effective range EXACTLY. A partition that is not
    ///   total is silently a dropped sub-operation.
    /// - Never panics on hostile input, never allocates unboundedly.
    ///
    /// ★ **A wrapping range is CLIPPED at the top of the address space, never wrapped.**
    /// `start + len` is computed in `u128` and the effective end is `min(start+len, 2^64)`.
    /// Honouring the wrap would let a hostile length reach a range at the BOTTOM of the
    /// space from a request aimed at the top. `len == 0` yields no spans: an empty request
    /// is empty, not a fault.
    ///
    /// ★★★ **A covered span carries its OFFSET INTO THE RANGE** — `(value, span.start −
    /// range.start)`. A span generally begins *inside* a range rather than at its base, so a
    /// caller holding only the value holds the description of a different byte; recovering
    /// the offset with a second `lookup` is both a wasted probe and a place to be wrong.
    /// `[measured 2026-08-08]` the copy-engine executor wrote at its span's **virtual**
    /// address precisely because the physical one was not available at this seam
    /// (`execution_plane_increments.md` §14.14 REFUTED 4), so the offset is returned rather
    /// than left to be re-derived.
    #[must_use]
    pub fn spans(&self, start: u64, len: u64) -> Vec<SpanRun<'_, V>> {
        let begin = u128::from(start);
        let end = (begin + u128::from(len)).min(1u128 << 64);
        let mut out = Vec::new();
        let mut at = begin;
        while at < end {
            let here = at as u64;
            match self.lookup(here) {
                Some((r_start, r_len, v)) => {
                    let r_end = u128::from(r_start) + u128::from(r_len);
                    let run_end = r_end.min(end);
                    out.push((here, (run_end - at) as u64, Some((v, here - r_start))));
                    at = run_end;
                }
                None => {
                    // The hole runs until the next range that STARTS at or after `here`,
                    // or to the end of the request. `next_from` is O(log n); a per-byte
                    // probe would be the C's O(n) overlay scan that ate 42% of a run.
                    let next = self
                        .next_from(here)
                        .map_or(end, |(s, _, _)| u128::from(s).min(end));
                    out.push((here, (next - at) as u64, None));
                    at = next;
                }
            }
        }
        out
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
        assert_eq!(
            m.lookup(0x1000),
            None,
            "removed range is a MISS (unmap eager)"
        );
    }

    #[test]
    fn overlap_is_a_loud_error_never_silent() {
        let mut m = IntervalMap::new();
        m.insert(0x1000, 0x1000, ()).unwrap();
        // Left, right, containing, contained: all must refuse.
        for (s, l) in [
            (0x800, 0x900),
            (0x1fff, 0x10),
            (0x0, 0x10000),
            (0x1400, 0x100),
        ] {
            assert_eq!(
                m.insert(s, l, ()),
                Err(IntervalError::Overlap {
                    existing_start: 0x1000,
                    existing_len: 0x1000
                }),
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
        assert_eq!(
            m.insert(0x1000, 0, ()),
            Err(IntervalError::Empty),
            "zero-len refused"
        );
        assert_eq!(
            m.insert(u64::MAX - 0x100, 0x1000, ()),
            Err(IntervalError::Wraps),
            "a range wrapping u64 is refused, not a panic"
        );
        assert_eq!(
            m.insert(u64::MAX, 1, ()),
            Err(IntervalError::Wraps),
            "last-byte wrap refused"
        );
        // The maximal non-wrapping range (ending exactly at u64::MAX) is accepted.
        m.insert(u64::MAX - 0x1000, 0x1000, ()).unwrap();
    }
}
