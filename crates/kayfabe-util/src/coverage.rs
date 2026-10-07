//! ★★★★★ **A COVERAGE PREDICATE — `declared ⊆ published`, computed over EVERY element.**
//!
//! # ⊘⊘ THE DEFECT THIS EXISTS TO REPLACE
//!
//! `[measured, boot w376llmd]` the address-plane census printed
//! `refused_vas=[…24 addresses…] ⚠⚠ CAPPED at 24 of 255 distinct` beside
//! `HOST-PUBLISHED [… host_rows=0 of 6254 runs=0 …]`, and **neither line can answer whether
//! publication is complete**:
//!
//! - `host_rows=N of M` is a **cardinality**. Two different row counts can describe the same
//!   covered bytes — one 64 KiB row and sixteen 4 KiB rows are the same coverage and a
//!   different number. The owner's correction states it directly: *"the only thing that
//!   matters is that the same ranges are mapped, not the amount of exercised mmaps."*
//! - The address list is a **truncated sample**, and its own warning says an address absent
//!   from it is not thereby un-refused. A sample cannot certify a universal.
//!
//! ⇒ `w377` §3 blocker (5): *"A capped list cannot distinguish a fix from a coincidence."*
//!
//! # ★★★ THE RULE THAT MAKES THIS AN INSTRUMENT AND NOT A REPORT
//!
//! **A cap may truncate what is PRINTED. It must never truncate what is COMPUTED.**
//!
//! The replaced census had the cap and the computation in the *same loop* (`.take(cap)` over
//! the run list that was also the answer), so the verdict inherited the truncation. Here the
//! two are structurally separate: [`Coverage`] is built from the whole of both sets and owns
//! exact counts; `cap` appears **only** in [`Coverage::render`], which prints
//! `showing N of M` and marks the list — never the counts — as truncated.
//!
//! ⚠ That property is *proved*, not asserted: this module's own
//! `the_print_cap_truncates_the_list_and_never_the_verdict` feeds far more intervals than the
//! cap and asserts `COVERED`, `residual_bytes` and `residual_intervals` are still exact.
//!
//! # ⊘ Purely generic (design decision #2)
//!
//! Nothing here names a GPU, a VAS, a driver or an aperture. It is interval algebra over
//! `u64` addresses: union, subset, difference. The GPU-shaped half — *which* sets are
//! `declared` and `published` — lives in `kayfabe_rt::device::SharedDevice::vas_coverage`,
//! where the two records of host-side mapping state are joined.

use core::fmt::Write as _;

/// A half-open address interval `[start, end)`.
///
/// ⊘ **`u128` endpoints over `u64` addresses, deliberately.** A hostile or merely wrong
/// `(start, len)` pair can have `start + len` exceed `u64::MAX`; every alternative to widening
/// loses information at exactly the input a boundary-1 posture must not lose it on
/// (`saturating_add` silently shortens the range, `checked_add` drops it, a panic is out of
/// the question). Widening keeps the arithmetic total and exact, and the extra 8 bytes are
/// never on a hot path — this is built once per doorbell census.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Interval {
    /// Inclusive start.
    pub start: u128,
    /// Exclusive end. Always `>= start`.
    pub end: u128,
}

impl Interval {
    /// Byte length of the interval.
    #[must_use]
    pub const fn len(&self) -> u128 {
        self.end - self.start
    }

    /// Whether the interval covers no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// Accumulates `(start, len)` pairs into an [`IntervalSet`], **counting what it drops**.
///
/// ★ Two-phase (push-many, then [`build`](Self::build)) rather than an insert-merging
/// container, for one measured reason: publication runs on the vCPU thread under a time
/// budget (`w377` §3 blocker (4), `worst_trap=1750538us`), and an in-place merging insert is
/// `O(n)` per element over a 13 348-row table. Collect-then-sort is one `O(n log n)` pass.
///
/// ⊘ **A zero-length input is counted, not silently swallowed.** A zero-length declared row
/// contributes nothing to `declared` and therefore can never appear in the residual — it is
/// invisible to the predicate *by construction*, which is the `dlen=0` mistake wearing new
/// clothes. [`zero_len`](Self::zero_len) is how the caller says so out loud.
#[derive(Debug, Clone, Default)]
pub struct IntervalSetBuilder {
    runs: Vec<Interval>,
    zero_len: usize,
}

impl IntervalSetBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A builder with room for `n` intervals reserved.
    #[must_use]
    pub fn with_capacity(n: usize) -> Self {
        IntervalSetBuilder {
            runs: Vec::with_capacity(n),
            zero_len: 0,
        }
    }

    /// Add `[start, start + len)`. A zero `len` adds nothing and bumps
    /// [`zero_len`](Self::zero_len).
    pub fn push_len(&mut self, start: u64, len: u64) {
        if len == 0 {
            self.zero_len += 1;
            return;
        }
        self.runs.push(Interval {
            start: u128::from(start),
            end: u128::from(start) + u128::from(len),
        });
    }

    /// Add a half-open interval directly. An empty or inverted interval adds nothing and
    /// bumps [`zero_len`](Self::zero_len).
    pub fn push(&mut self, iv: Interval) {
        if iv.is_empty() {
            self.zero_len += 1;
            return;
        }
        self.runs.push(iv);
    }

    /// How many zero-length (or inverted) inputs were refused.
    #[must_use]
    pub const fn zero_len(&self) -> usize {
        self.zero_len
    }

    /// Sort and merge into the canonical set.
    #[must_use]
    pub fn build(mut self) -> IntervalSet {
        self.runs.sort_unstable();
        let mut merged: Vec<Interval> = Vec::with_capacity(self.runs.len());
        for iv in self.runs {
            match merged.last_mut() {
                // ★ `<=` and not `<`: ADJACENT intervals merge. `[0,0x1000)` ∪
                // `[0x1000,0x2000)` is ONE interval covering 8 KiB, not two. A set that kept
                // them apart would report `residual_intervals=2` for a contiguous hole and
                // make two fixes look like two problems.
                Some(last) if iv.start <= last.end => last.end = last.end.max(iv.end),
                _ => merged.push(iv),
            }
        }
        IntervalSet { runs: merged }
    }
}

impl Extend<(u64, u64)> for IntervalSetBuilder {
    fn extend<T: IntoIterator<Item = (u64, u64)>>(&mut self, iter: T) {
        for (start, len) in iter {
            self.push_len(start, len);
        }
    }
}

/// A canonical set of addresses: **sorted, disjoint and non-adjacent** intervals.
///
/// Canonical means two `IntervalSet`s covering the same bytes are `==`, whatever shape their
/// inputs had. That is the whole point — it is what makes *"the same ranges are mapped"* a
/// value rather than a judgement call about row counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntervalSet {
    /// Invariant: sorted by `start`, pairwise disjoint, and no two are adjacent
    /// (`runs[i].end < runs[i + 1].start`). Every constructor goes through
    /// [`IntervalSetBuilder::build`] or produces this shape directly.
    runs: Vec<Interval>,
}

impl FromIterator<(u64, u64)> for IntervalSet {
    fn from_iter<T: IntoIterator<Item = (u64, u64)>>(iter: T) -> Self {
        let mut b = IntervalSetBuilder::new();
        b.extend(iter);
        b.build()
    }
}

impl IntervalSet {
    /// The empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The canonical intervals, ascending.
    #[must_use]
    pub fn intervals(&self) -> &[Interval] {
        &self.runs
    }

    /// How many **merged** intervals the set has. ⊘ Not the number of inputs: that is a
    /// cardinality, and cardinality is the thing this module exists to stop reporting.
    #[must_use]
    pub fn count(&self) -> usize {
        self.runs.len()
    }

    /// Total bytes covered.
    #[must_use]
    pub fn bytes(&self) -> u128 {
        self.runs.iter().map(Interval::len).sum()
    }

    /// Whether the set covers no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// `self \ other` — every byte in `self` that `other` does not cover, as a canonical set.
    ///
    /// ★ A single ordered sweep, `O(n + m)`, over two already-sorted sets. `j` only ever
    /// advances past intervals of `other` that end at or before the current interval's start,
    /// which is sound because `self.runs` is ascending.
    #[must_use]
    pub fn difference(&self, other: &IntervalSet) -> IntervalSet {
        let mut out: Vec<Interval> = Vec::new();
        let mut j = 0usize;
        for a in &self.runs {
            let mut cur = a.start;
            while j < other.runs.len() && other.runs[j].end <= cur {
                j += 1;
            }
            let mut k = j;
            while k < other.runs.len() && other.runs[k].start < a.end {
                let b = other.runs[k];
                if b.start > cur {
                    out.push(Interval {
                        start: cur,
                        end: b.start,
                    });
                }
                cur = cur.max(b.end);
                if cur >= a.end {
                    break;
                }
                k += 1;
            }
            if cur < a.end {
                out.push(Interval {
                    start: cur,
                    end: a.end,
                });
            }
        }
        // ⊘ The sweep already yields sorted, disjoint, non-adjacent pieces (consecutive
        // pieces are separated either by a positive-length interval of `other` or by the gap
        // between two non-adjacent intervals of `self`). The invariant is RE-ESTABLISHED
        // rather than assumed anyway: this type's equality is only meaningful while it holds,
        // and a hand-proof in a comment is not a checked one.
        let mut b = IntervalSetBuilder::with_capacity(out.len());
        for iv in out {
            b.push(iv);
        }
        b.build()
    }

    /// `other ⊆ self` — whether every byte of `other` is covered by `self`.
    ///
    /// ★★★ **This is what makes a REFINEMENT covered.** `declared = [0, 0x1000)` against
    /// `published = [0, 0xea000)` is `true`: the question is which BYTES are mapped, never
    /// whether the extents match. `[measured, w376llmd]` the live straddle check answers the
    /// other question and refuses 233 rows on the `InsideLarger/SameMemory/leaf0x1000/
    /// live0xea000` signature alone. **This predicate must not inherit that bug**, and
    /// `a_refinement_is_covered` is where that is checked.
    #[must_use]
    pub fn contains_set(&self, other: &IntervalSet) -> bool {
        other.difference(self).is_empty()
    }
}

/// The verdict for one address space: `declared ⊆ published`, with the residual that must go
/// to zero and the excess that must not be silent.
///
/// # ⊘ What each number is, and what it is not
///
/// - `covered` — the boolean. **Exact, always, whatever any print cap is.**
/// - `residual` — `declared \ published`. **This is the number that must reach zero.** It is
///   bytes-of-guest-intent the GPU cannot see.
/// - `excess` — `published \ declared`. Non-zero excess is **not automatically wrong**:
///   RM rounds a mapping up to the memdesc's own page size (`w377` §2), so over-cover is
///   expected and legitimate. It is reported because an over-cover we did not intend is
///   indistinguishable from one we did, until someone looks.
#[derive(Debug, Clone)]
pub struct Coverage {
    declared_bytes: u128,
    declared_intervals: usize,
    published_bytes: u128,
    published_intervals: usize,
    residual: IntervalSet,
    excess: IntervalSet,
}

impl Coverage {
    /// Compute the predicate over the **whole** of both sets.
    #[must_use]
    pub fn new(declared: &IntervalSet, published: &IntervalSet) -> Self {
        Coverage {
            declared_bytes: declared.bytes(),
            declared_intervals: declared.count(),
            published_bytes: published.bytes(),
            published_intervals: published.count(),
            residual: declared.difference(published),
            excess: published.difference(declared),
        }
    }

    /// `declared ⊆ published`. Trivially `true` when nothing is declared — see
    /// [`Self::declared_is_empty`], which is how a reader tells the trivial `true` from an
    /// earned one.
    #[must_use]
    pub fn covered(&self) -> bool {
        self.residual.is_empty()
    }

    /// Whether the declared set was empty, making [`Self::covered`] vacuous.
    #[must_use]
    pub fn declared_is_empty(&self) -> bool {
        self.declared_bytes == 0
    }

    /// Bytes the guest's mapping intent describes.
    #[must_use]
    pub const fn declared_bytes(&self) -> u128 {
        self.declared_bytes
    }

    /// Merged intervals in the declared set.
    #[must_use]
    pub const fn declared_intervals(&self) -> usize {
        self.declared_intervals
    }

    /// Bytes actually installed in the host VAS.
    #[must_use]
    pub const fn published_bytes(&self) -> u128 {
        self.published_bytes
    }

    /// Merged intervals in the published set.
    #[must_use]
    pub const fn published_intervals(&self) -> usize {
        self.published_intervals
    }

    /// **The number that must go to zero** — declared bytes not published.
    #[must_use]
    pub fn residual_bytes(&self) -> u128 {
        self.residual.bytes()
    }

    /// How many merged intervals the residual has.
    #[must_use]
    pub fn residual_intervals(&self) -> usize {
        self.residual.count()
    }

    /// Published bytes nothing declared.
    #[must_use]
    pub fn excess_bytes(&self) -> u128 {
        self.excess.bytes()
    }

    /// How many merged intervals the excess has.
    #[must_use]
    pub fn excess_intervals(&self) -> usize {
        self.excess.count()
    }

    /// The residual itself, whole and untruncated.
    #[must_use]
    pub fn residual(&self) -> &IntervalSet {
        &self.residual
    }

    /// The excess itself, whole and untruncated.
    #[must_use]
    pub fn excess(&self) -> &IntervalSet {
        &self.excess
    }

    /// Fraction of declared bytes that are published, as a percentage — `None` when nothing
    /// is declared.
    ///
    /// ⊘ `Option` rather than a `0.0`/`100.0` convention, because *"there was nothing to
    /// cover"* and *"everything was covered"* are different findings and a float cannot carry
    /// the difference. The renderer prints `n/a(declared empty)`.
    #[must_use]
    pub fn covered_pct(&self) -> Option<f64> {
        if self.declared_bytes == 0 {
            return None;
        }
        let covered = self.declared_bytes - self.residual_bytes();
        Some(100.0 * (covered as f64) / (self.declared_bytes as f64))
    }

    /// The one-line verdict.
    ///
    /// ★★★ `cap` bounds **only** the two printed interval lists. Every count on this line —
    /// `COVERED`, `residual_bytes`, `residual_intervals`, `excess_bytes`, `excess_intervals`
    /// — is computed over the whole set and is exact regardless of `cap`.
    #[must_use]
    pub fn render(&self, cap: usize) -> String {
        let pct = match self.covered_pct() {
            Some(p) => format!("{p:.4}%"),
            None => "n/a(declared empty)".to_string(),
        };
        format!(
            "COVERED={}{} declared={}B/{}i published={}B/{}i covered_pct={pct} \
             residual_bytes={} residual_intervals={} residual={} \
             excess_bytes={} excess_intervals={} excess={}",
            self.covered(),
            if self.declared_is_empty() {
                "(TRIVIAL: declared is EMPTY — nothing was asked of publication, this is not \
                 evidence that publication works)"
            } else {
                ""
            },
            self.declared_bytes,
            self.declared_intervals,
            self.published_bytes,
            self.published_intervals,
            self.residual_bytes(),
            self.residual_intervals(),
            render_intervals(&self.residual, cap),
            self.excess_bytes(),
            self.excess_intervals(),
            render_intervals(&self.excess, cap),
        )
    }
}

/// `[0x…+0x…,…] showing N of M` — the **only** place a cap is allowed to appear.
#[must_use]
pub fn render_intervals(set: &IntervalSet, cap: usize) -> String {
    let mut s = String::from("[");
    for (i, iv) in set.intervals().iter().take(cap).enumerate() {
        if i > 0 {
            s.push(',');
        }
        // `start` is a real address and fits `u64`; the `u128` width exists for the END.
        let _ = write!(s, "0x{:x}+0x{:x}", iv.start, iv.len());
    }
    s.push(']');
    let shown = set.count().min(cap);
    let _ = write!(s, " showing {shown} of {}", set.count());
    if set.count() > cap {
        s.push_str(
            " ⚠ PRINT-TRUNCATED (the counts beside this list are EXACT and were \
                    computed over ALL of it)",
        );
    }
    s
}

/// The aggregate over many address spaces.
///
/// ⊘ **Sums, never a union.** Two address spaces are two different meanings for the same
/// number, so `0x1000` in one and `0x1000` in another are different bytes and unioning them
/// would silently cover one with the other. What aggregates soundly is the byte totals and
/// the count of address spaces that pass.
#[derive(Debug, Clone, Default)]
pub struct CoverageAggregate {
    /// Address spaces folded in.
    pub vases: usize,
    /// Of those, how many satisfy `declared ⊆ published`.
    pub covered_vases: usize,
    /// Of those, how many had an empty declared set (so their `true` is vacuous).
    pub trivial_vases: usize,
    /// Total declared bytes.
    pub declared_bytes: u128,
    /// Total published bytes.
    pub published_bytes: u128,
    /// **Total residual bytes — the number that must go to zero.**
    pub residual_bytes: u128,
    /// Total residual intervals, summed across address spaces.
    pub residual_intervals: usize,
    /// Total excess bytes.
    pub excess_bytes: u128,
    /// Total excess intervals, summed across address spaces.
    pub excess_intervals: usize,
}

impl CoverageAggregate {
    /// Fold one address space's verdict in.
    pub fn add(&mut self, c: &Coverage) {
        self.vases += 1;
        if c.covered() {
            self.covered_vases += 1;
        }
        if c.declared_is_empty() {
            self.trivial_vases += 1;
        }
        self.declared_bytes += c.declared_bytes();
        self.published_bytes += c.published_bytes();
        self.residual_bytes += c.residual_bytes();
        self.residual_intervals += c.residual_intervals();
        self.excess_bytes += c.excess_bytes();
        self.excess_intervals += c.excess_intervals();
    }

    /// Whether **every** folded address space is covered.
    #[must_use]
    pub const fn covered(&self) -> bool {
        self.covered_vases == self.vases
    }

    /// The aggregate line.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "COVERED={} vases={} covered_vases={} trivial_vases={} declared={}B \
             published={}B residual_bytes={} residual_intervals={} excess_bytes={} \
             excess_intervals={}{}",
            self.covered(),
            self.vases,
            self.covered_vases,
            self.trivial_vases,
            self.declared_bytes,
            self.published_bytes,
            self.residual_bytes,
            self.residual_intervals,
            self.excess_bytes,
            self.excess_intervals,
            if self.vases == 0 {
                " (⊘ NO LIVE ADDRESS SPACE — this `true` is vacuous, it is NOT 'publication \
                 is complete')"
            } else {
                ""
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K4: u64 = 0x1000;

    fn set(runs: &[(u64, u64)]) -> IntervalSet {
        runs.iter().copied().collect()
    }

    /// ★★★ **ADJACENT AND OVERLAPPING INTERVALS MERGE.** `[0,4k) ∪ [4k,8k)` is ONE interval.
    #[test]
    fn adjacent_and_overlapping_intervals_merge() {
        let adjacent = set(&[(0, K4), (K4, K4)]);
        assert_eq!(adjacent.count(), 1, "adjacent must MERGE: {adjacent:?}");
        assert_eq!(adjacent.bytes(), u128::from(2 * K4));

        let overlapping = set(&[(0, 3 * K4), (K4, 3 * K4)]);
        assert_eq!(overlapping.count(), 1, "{overlapping:?}");
        assert_eq!(overlapping.bytes(), u128::from(4 * K4));

        // ⊘ And the negative control: a ONE-BYTE gap must NOT merge, or the merge rule is
        // "everything is one interval" and the count means nothing.
        let gapped = set(&[(0, K4), (K4 + 1, K4)]);
        assert_eq!(gapped.count(), 2, "a gap is a gap: {gapped:?}");

        // ⊘ Order of arrival is irrelevant — the set is canonical.
        assert_eq!(set(&[(K4, K4), (0, K4)]), adjacent);
    }

    /// ★★★ **`residual` empty ⟺ `COVERED`.** Both directions, on the same fixture family.
    #[test]
    fn residual_is_empty_iff_covered() {
        let declared = set(&[(0x8000, 0x4000)]);

        let exact = Coverage::new(&declared, &set(&[(0x8000, 0x4000)]));
        assert!(exact.covered());
        assert_eq!(exact.residual_bytes(), 0);
        assert_eq!(exact.residual_intervals(), 0);

        let holed = Coverage::new(&declared, &set(&[(0x8000, 0x1000), (0xa000, 0x2000)]));
        assert!(!holed.covered(), "a hole is not covered: {holed:?}");
        assert_eq!(holed.residual_bytes(), 0x1000, "{holed:?}");
        assert_eq!(holed.residual_intervals(), 1, "{holed:?}");
        assert_eq!(
            holed.residual().intervals(),
            &[Interval {
                start: 0x9000,
                end: 0xa000
            }],
            "the residual NAMES the hole, it does not merely count it"
        );

        let nothing = Coverage::new(&declared, &IntervalSet::new());
        assert!(!nothing.covered());
        assert_eq!(nothing.residual_bytes(), 0x4000);
    }

    /// ★★★★★ **A REFINEMENT COVERS — the `InsideLarger` case the live straddle check refuses.**
    ///
    /// `[measured, boot w376llmd]` 233 of 255 refusals carry the signature
    /// `InsideLarger/SameMemory/lvl5/leaf0x1000/live0xea000/pub0`. Extent mismatch is the
    /// straddle check's whole criterion; it must be **none** of this predicate's, or the
    /// instrument built to grade that bug's fix shares the bug.
    #[test]
    fn a_refinement_is_covered() {
        let declared = set(&[(0, 0x1000)]);
        let published = set(&[(0, 0xea000)]);
        let c = Coverage::new(&declared, &published);
        assert!(
            c.covered(),
            "a 4 KiB leaf inside a live 0xea000 binding IS covered: {c:?}"
        );
        assert_eq!(c.residual_bytes(), 0);
        // ★ And the over-cover is VISIBLE rather than silent — it is legitimate (RM rounds
        // up to the memdesc page size) and must still be a number a reader can see.
        assert_eq!(c.excess_bytes(), 0xea000 - 0x1000, "{c:?}");
        assert_eq!(c.excess_intervals(), 1, "{c:?}");
        assert_eq!(
            c.excess().intervals(),
            &[Interval {
                start: 0x1000,
                end: 0xea000
            }]
        );

        // ⊘ The mirror shape from the same boot: `SameStartShorter`. Declared starts where
        // the live binding starts and is shorter — also a refinement, also covered.
        let same_start_shorter =
            Coverage::new(&set(&[(0x1_0000, 0x8600)]), &set(&[(0x1_0000, 0x1_0000)]));
        assert!(same_start_shorter.covered(), "{same_start_shorter:?}");
    }

    /// ★★★★★ **THE PROPERTY THE SPEC TURNS ON: a cap truncates the LIST, never the VERDICT.**
    ///
    /// 200 declared intervals, none published, printed at `cap=3`. The rendered list carries
    /// three; `COVERED`, `residual_bytes` and `residual_intervals` are exact over all 200.
    #[test]
    fn the_print_cap_truncates_the_list_and_never_the_verdict() {
        const N: u64 = 200;
        const CAP: usize = 3;
        // Every interval separated by a 4 KiB gap so none of them merge — the count is the
        // fixture's, not the merger's.
        let declared: IntervalSet = (0..N).map(|i| (i * 2 * K4, K4)).collect();
        assert_eq!(declared.count(), N as usize, "fixture: 200 distinct runs");

        let c = Coverage::new(&declared, &IntervalSet::new());
        assert!(!c.covered());
        assert_eq!(c.residual_intervals(), N as usize, "EXACT over all 200");
        assert_eq!(c.residual_bytes(), u128::from(N * K4), "EXACT over all 200");

        let line = c.render(CAP);
        assert!(line.contains("COVERED=false"), "{line}");
        assert!(line.contains("residual_intervals=200"), "{line}");
        assert!(line.contains("residual_bytes=819200"), "{line}");
        assert!(line.contains("showing 3 of 200"), "{line}");
        assert!(line.contains("PRINT-TRUNCATED"), "{line}");
        // ⊘ Non-vacuity: the list really is short. Four commas would mean the cap did
        // nothing and the test proves nothing.
        let listed = line
            .split("residual=[")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .expect("the residual list is rendered");
        assert_eq!(listed.split(',').count(), CAP, "list is capped: {line}");

        // ★★ And the verdict is IDENTICAL at a cap that truncates nothing — the cap is not
        // an input to the computation at all.
        let uncapped = c.render(1024);
        assert!(uncapped.contains("residual_intervals=200"), "{uncapped}");
        assert!(!uncapped.contains("PRINT-TRUNCATED"), "{uncapped}");
        assert!(uncapped.contains("showing 200 of 200"), "{uncapped}");
        for line in [&line, &uncapped] {
            let verdict = line.split(" residual=").next().expect("split");
            assert!(verdict.contains("COVERED=false"), "{verdict}");
        }
    }

    /// ★★ **An empty declared set is COVERED, and the line SAYS the `true` is vacuous.**
    #[test]
    fn an_empty_declared_set_is_trivially_covered_and_says_so() {
        let c = Coverage::new(&IntervalSet::new(), &set(&[(0, K4)]));
        assert!(c.covered());
        assert!(c.declared_is_empty());
        assert_eq!(
            c.covered_pct(),
            None,
            "⊘ no division by zero, and no fake 100%"
        );
        let line = c.render(8);
        assert!(line.contains("COVERED=true(TRIVIAL"), "{line}");
        assert!(line.contains("covered_pct=n/a(declared empty)"), "{line}");
        // The published bytes are pure excess and are still reported.
        assert_eq!(c.excess_bytes(), u128::from(K4));

        // ⊘ And both-empty: still trivially true, still labelled.
        let both = Coverage::new(&IntervalSet::new(), &IntervalSet::new());
        assert!(both.covered() && both.declared_is_empty());
        assert!(both.render(8).contains("TRIVIAL"), "{}", both.render(8));
    }

    /// ★ Cardinality is not coverage — the finding this module exists for, as a value.
    #[test]
    fn sixteen_small_rows_and_one_large_row_are_the_same_coverage() {
        let many: IntervalSet = (0..16u64).map(|i| (i * K4, K4)).collect();
        let one: IntervalSet = core::iter::once((0, 16 * K4)).collect();
        assert_eq!(many, one, "16 rows and 1 row describe the SAME set");
        assert_eq!(many.count(), 1, "and both canonicalize to one interval");
        assert!(Coverage::new(&many, &one).covered());
        assert!(Coverage::new(&one, &many).covered());
    }

    /// ⊘ A `(start, len)` whose sum exceeds `u64::MAX` is kept EXACTLY, not clamped or
    /// dropped — the boundary-1 input that every narrower arithmetic loses.
    #[test]
    fn a_range_past_the_top_of_the_address_space_is_kept_exactly() {
        let s = set(&[(u64::MAX - 0xfff, 0x2000)]);
        assert_eq!(s.count(), 1);
        assert_eq!(s.bytes(), 0x2000, "no clamp, no drop: {s:?}");
        assert_eq!(s.intervals()[0].end, u128::from(u64::MAX) + 0x1001);
    }

    /// ⊘ Zero-length inputs are COUNTED, because they are invisible to the predicate.
    #[test]
    fn zero_length_inputs_are_counted_not_swallowed() {
        let mut b = IntervalSetBuilder::new();
        b.push_len(0x1000, 0);
        b.push_len(0x2000, K4);
        b.push_len(0x3000, 0);
        assert_eq!(b.zero_len(), 2);
        let s = b.build();
        assert_eq!((s.count(), s.bytes()), (1, u128::from(K4)));
    }

    /// ★ Difference across many interleaved intervals — the sweep's `j` pointer is the part
    /// that is easy to get wrong and impossible to see wrong in a one-interval fixture.
    #[test]
    fn the_difference_sweep_handles_interleaved_sets() {
        // declared: [0,0x1000) [0x4000,0x5000) [0x8000,0xc000)
        let declared = set(&[(0, K4), (0x4000, K4), (0x8000, 0x4000)]);
        // published: [0x800,0x4800) [0x9000,0xa000) [0xb000,0x20000)
        let published = set(&[(0x800, 0x4000), (0x9000, K4), (0xb000, 0x15000)]);
        let c = Coverage::new(&declared, &published);
        assert_eq!(
            c.residual().intervals(),
            &[
                Interval {
                    start: 0,
                    end: 0x800
                },
                Interval {
                    start: 0x4800,
                    end: 0x5000
                },
                Interval {
                    start: 0x8000,
                    end: 0x9000
                },
                Interval {
                    start: 0xa000,
                    end: 0xb000
                },
            ],
            "{c:?}"
        );
        assert_eq!(c.residual_bytes(), 0x800 + 0x800 + 0x1000 + 0x1000);
        assert_eq!(c.residual_intervals(), 4);
        assert!(!c.covered());
        // ⊘ And the excess is the published bytes nothing DECLARED — which is NOT the same
        // as "every byte outside declared". `[0x5000,0x8000)` is in neither set and appears
        // in neither list; a difference that returned it would be reporting the complement,
        // not the difference. (This assertion caught exactly that error in its own first
        // draft, which is why it names the interval it must NOT contain.)
        assert_eq!(
            c.excess().intervals(),
            &[
                Interval {
                    start: 0x1000,
                    end: 0x4000
                },
                Interval {
                    start: 0xc000,
                    end: 0x20000
                },
            ],
            "{c:?}"
        );
        assert!(
            !c.excess().intervals().iter().any(|iv| iv.start == 0x5000),
            "⊘ a hole in BOTH sets is in NEITHER list: {c:?}"
        );
    }

    /// ★★ The aggregate: sums, an all-covered boolean, and a vacuous-when-empty label.
    #[test]
    fn the_aggregate_sums_and_never_unions_across_address_spaces() {
        let a = Coverage::new(&set(&[(0x1000, K4)]), &set(&[(0x1000, K4)]));
        // ⊘ Same ADDRESS as `a` covers, different address space. If the aggregate unioned,
        // this residual would vanish.
        let b = Coverage::new(&set(&[(0x1000, K4)]), &IntervalSet::new());
        let mut agg = CoverageAggregate::default();
        agg.add(&a);
        agg.add(&b);
        assert!(!agg.covered(), "{agg:?}");
        assert_eq!((agg.vases, agg.covered_vases), (2, 1));
        assert_eq!(agg.residual_bytes, u128::from(K4), "{agg:?}");
        assert_eq!(agg.residual_intervals, 1);
        assert!(agg.render().contains("COVERED=false"), "{}", agg.render());

        let empty = CoverageAggregate::default();
        assert!(empty.covered(), "0 == 0");
        assert!(
            empty.render().contains("NO LIVE ADDRESS SPACE"),
            "⊘ a vacuous true must announce itself: {}",
            empty.render()
        );
    }
}
