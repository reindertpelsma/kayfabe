//! ★★★★★ **THE HOST-SIDE DIFF — what changed between two walks of the guest's page tables.**
//!
//! The walk kernel (`cuda/walk/`) reports **the current mapping set**, not a delta
//! (`the_walk_kernel_report_format.md` §w723). This module turns two such sets into the
//! instruction list kayfabe acts on.
//!
//! # ⊘⊘⊘ WHY THIS IS IN RUST AND NOT IN THE KERNEL
//!
//! `[w722]` The diff was written in CUDA first, as a **merge join over whole runs**, which is the
//! obvious reading of *"linear, no hashing, no allocation"*. It **does not close**: for one new run
//! covering several old ones it emits a `Remap` followed by `Unmap`s of the runs it just replaced,
//! and **applying that in order deletes the mapping it just made**.
//!
//! ⚠ **All nine single-step delta tests passed against it.** Only an accumulating round-trip found
//! it. ⇒ Diff logic is subtle enough to get wrong, so it belongs beside the model, in a language
//! where a mistake is a panic rather than silent garbage.
//!
//! # ★★★ THE PROPERTY, and it is the one worth testing
//!
//! ```text
//!     apply(prev, diff(prev, cur)) == cur          for all prev, cur
//! ```
//!
//! ★ And **order-independence**: the [`MapOp::Unmap`] set and the [`MapOp::Map`]/[`MapOp::Remap`]
//! set are **disjoint in `(va, class)` by construction**, so applying the ops in any order gives
//! the same result. That is a property of the algorithm, not of the caller's discipline — which
//! is exactly what the whole-run version lacked.
//!
//! # The algorithm — per class, at SEGMENT granularity
//!
//! Runs of different page sizes can cover the same VA (a dual PDE's two halves both describe the
//! same 2 MiB), so a diff keyed on VA alone conflates them. ⇒ Group by **page-size class** first;
//! within a class the runs are disjoint and ascending, so the two sides can be walked together and
//! cut at every boundary either side introduces.
//!
//! For each resulting segment: covered by **prev only** ⇒ `Unmap`; **cur only** ⇒ `Map`; **both,
//! differing** ⇒ `Remap`; **both, identical** ⇒ nothing.

use std::collections::BTreeMap;

/// A contiguous run of pages mapping ascending VAs to ascending GPGAs with identical flags —
/// the unit the walk kernel reports, and the unit this module diffs.
///
/// ⊘ `len` is a whole multiple of the class's page size; a run is never partial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// Start virtual address, page-aligned.
    pub va: u64,
    /// Start guest physical (framebuffer) address the run maps to.
    pub gpga: u64,
    /// Length in bytes.
    pub len: u64,
    /// Decoded flags — aperture, read-only, atomic-disable, volatile. ⊘ **Not** the page size;
    /// that is [`Run::class`], because two runs differing only in page size are different
    /// mappings and must not be coalesced or compared as one.
    pub flags: u32,
    /// The page-size class. Runs are diffed **within** a class, never across.
    pub class: PageClass,
}

/// Which leaf size a run is made of. Distinct classes may cover the same VA simultaneously.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PageClass {
    /// 4 KiB.
    P4K,
    /// 64 KiB.
    P64K,
    /// 2 MiB.
    P2M,
    /// 512 MiB — Ampere and later.
    P512M,
}

impl PageClass {
    /// The class's page size in bytes.
    #[must_use]
    pub fn bytes(self) -> u64 {
        match self {
            PageClass::P4K => 4096,
            PageClass::P64K => 64 * 1024,
            PageClass::P2M => 2 * 1024 * 1024,
            PageClass::P512M => 512 * 1024 * 1024,
        }
    }
}

/// One instruction for the publisher.
///
/// ⊘ `Remap` is kept distinct from `Unmap` + `Map` deliberately: the host can re-point a binding
/// **without a window in which the VA is unmapped**, which is the difference between a re-point
/// and a transient fault the guest can observe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapOp {
    /// The guest now maps this range and did not before.
    Map(Run),
    /// The guest no longer maps this range. Only `va`, `len` and `class` are meaningful.
    Unmap(Run),
    /// Same range, different target or flags.
    Remap(Run),
}

impl MapOp {
    /// The run this op names.
    #[must_use]
    pub fn run(&self) -> &Run {
        match self {
            MapOp::Map(r) | MapOp::Unmap(r) | MapOp::Remap(r) => r,
        }
    }
}

/// The GPGA a run maps `va` to. ⊘ A run is linear, so this is arithmetic, not a lookup.
fn gpga_at(r: &Run, va: u64) -> u64 {
    r.gpga + (va - r.va)
}

/// ★★★ **Diff two mapping sets into an instruction list.**
///
/// `prev` and `cur` need not be sorted; they are grouped and sorted here. Within a class the
/// caller's runs must be **non-overlapping** — the walk produces them that way, and an overlap
/// would mean the guest's own tables described one VA twice in one class, which the walk refuses.
///
/// Returns ops whose `Unmap` set is disjoint from the `Map`/`Remap` set in `(va, class)`, so they
/// may be applied in **any** order.
#[must_use]
pub fn diff(prev: &[Run], cur: &[Run]) -> Vec<MapOp> {
    let mut by_class: BTreeMap<PageClass, (Vec<Run>, Vec<Run>)> = BTreeMap::new();
    for r in prev {
        by_class.entry(r.class).or_default().0.push(*r);
    }
    for r in cur {
        by_class.entry(r.class).or_default().1.push(*r);
    }
    let mut out = Vec::new();
    for (_class, (mut p, mut c)) in by_class {
        p.sort_by_key(|r| r.va);
        c.sort_by_key(|r| r.va);
        diff_one_class(&p, &c, &mut out);
    }
    out
}

/// Walk two sorted, disjoint run lists together, cutting at every boundary either side introduces.
fn diff_one_class(prev: &[Run], cur: &[Run], out: &mut Vec<MapOp>) {
    // Every VA at which coverage or content can change on either side.
    let mut edges: Vec<u64> = Vec::with_capacity((prev.len() + cur.len()) * 2);
    for r in prev.iter().chain(cur.iter()) {
        edges.push(r.va);
        edges.push(r.va + r.len);
    }
    edges.sort_unstable();
    edges.dedup();

    for w in edges.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        let p = covering(prev, lo);
        let c = covering(cur, lo);
        match (p, c) {
            (None, None) => {}
            (Some(_), None) => out.push(MapOp::Unmap(seg(lo, hi, 0, 0, prev, lo))),
            (None, Some(r)) => out.push(MapOp::Map(seg(lo, hi, gpga_at(r, lo), r.flags, cur, lo))),
            (Some(pr), Some(cr)) => {
                // ⊘ Compare the DERIVED target at this segment's start, not the runs' own `gpga`:
                // two runs starting at different VAs can agree perfectly over their overlap.
                if gpga_at(pr, lo) != gpga_at(cr, lo) || pr.flags != cr.flags {
                    out.push(MapOp::Remap(seg(lo, hi, gpga_at(cr, lo), cr.flags, cur, lo)));
                }
            }
        }
    }
}

/// The run covering `va`, if any. Linear; the lists are short and this keeps the code obvious.
fn covering(runs: &[Run], va: u64) -> Option<&Run> {
    runs.iter().find(|r| va >= r.va && va < r.va + r.len)
}

/// Build the segment `[lo, hi)` carrying `gpga`/`flags`, taking the class from whichever side
/// supplied it.
fn seg(lo: u64, hi: u64, gpga: u64, flags: u32, from: &[Run], at: u64) -> Run {
    let class = covering(from, at).map_or(PageClass::P4K, |r| r.class);
    Run {
        va: lo,
        gpga,
        len: hi - lo,
        flags,
        class,
    }
}

/// Apply ops to a mapping set, for tests and for the publisher's own bookkeeping.
///
/// ★ The set is keyed by `(class, va)` at **page** granularity, which is what makes
/// order-independence checkable rather than merely asserted.
#[must_use]
pub fn apply(prev: &[Run], ops: &[MapOp]) -> Vec<Run> {
    let mut pages: BTreeMap<(PageClass, u64), (u64, u32)> = BTreeMap::new();
    for r in prev {
        explode(r, &mut pages);
    }
    for op in ops {
        match op {
            MapOp::Unmap(r) => {
                let ps = r.class.bytes();
                let mut va = r.va;
                while va < r.va + r.len {
                    pages.remove(&(r.class, va));
                    va += ps;
                }
            }
            MapOp::Map(r) | MapOp::Remap(r) => explode(r, &mut pages),
        }
    }
    coalesce(&pages)
}

fn explode(r: &Run, pages: &mut BTreeMap<(PageClass, u64), (u64, u32)>) {
    let ps = r.class.bytes();
    let mut va = r.va;
    while va < r.va + r.len {
        pages.insert((r.class, va), (gpga_at(r, va), r.flags));
        va += ps;
    }
}

/// Re-form runs from pages — consecutive VAs, consecutive GPGAs, identical flags.
fn coalesce(pages: &BTreeMap<(PageClass, u64), (u64, u32)>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for (&(class, va), &(gpga, flags)) in pages {
        let ps = class.bytes();
        match out.last_mut() {
            Some(last)
                if last.class == class
                    && last.flags == flags
                    && last.va + last.len == va
                    && last.gpga + last.len == gpga =>
            {
                last.len += ps;
            }
            _ => out.push(Run {
                va,
                gpga,
                len: ps,
                flags,
                class,
            }),
        }
    }
    out
}

/// Canonical form: exploded to pages and re-coalesced, so two descriptions of the same mapping
/// set compare equal regardless of how the walk happened to cut them into runs.
#[must_use]
pub fn canonical(runs: &[Run]) -> Vec<Run> {
    apply(runs, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(va: u64, gpga: u64, len: u64, flags: u32, class: PageClass) -> Run {
        Run { va, gpga, len, flags, class }
    }
    const K4: PageClass = PageClass::P4K;
    const K64: PageClass = PageClass::P64K;

    /// ★★★★★ **THE CLOSURE PROPERTY.** Everything else in this module is in service of it.
    fn assert_closes(prev: &[Run], cur: &[Run], what: &str) {
        let ops = diff(prev, cur);
        assert_eq!(
            canonical(&apply(prev, &ops)),
            canonical(cur),
            "{what}: applying the diff did not reproduce the new set\n  ops = {ops:#x?}"
        );
        // ★ Order-independence: the Unmap set and the Map/Remap set are disjoint by
        // construction, so a reversed application must give the same answer.
        let mut rev = ops.clone();
        rev.reverse();
        assert_eq!(
            canonical(&apply(prev, &rev)),
            canonical(cur),
            "{what}: the diff is ORDER-DEPENDENT, which the algorithm forbids"
        );
    }

    /// ⊘⊘⊘ **THE w722 BUG, as a regression test.** One new run covering several old ones. The
    /// whole-run merge join emitted `Remap` then `Unmap` of the runs it had just replaced, and
    /// applying that in order deleted the mapping it had just made. ⚠ Every single-step test
    /// passed against that version; only coverage-changing shapes like this one catch it.
    #[test]
    fn one_new_run_covering_several_old_ones_still_closes() {
        let prev = vec![
            r(0x1000, 0xA000, 0x1000, 0, K4),
            r(0x2000, 0xB000, 0x1000, 0, K4),
            r(0x3000, 0xC000, 0x1000, 0, K4),
        ];
        // One contiguous run over all three, at a different target.
        let cur = vec![r(0x1000, 0x50000, 0x3000, 0, K4)];
        assert_closes(&prev, &cur, "one run replacing three");
    }

    /// The mirror image: one old run becoming several new ones.
    #[test]
    fn one_old_run_splitting_into_several_still_closes() {
        let prev = vec![r(0x1000, 0x50000, 0x3000, 0, K4)];
        let cur = vec![
            r(0x1000, 0xA000, 0x1000, 0, K4),
            r(0x2000, 0xB000, 0x1000, 1, K4),
            r(0x3000, 0xC000, 0x1000, 0, K4),
        ];
        assert_closes(&prev, &cur, "three runs replacing one");
    }

    /// ⊘ **Two classes may cover the same VA** — a dual PDE's halves both describe one 2 MiB
    /// region. A diff keyed on VA alone conflates them; this asserts they stay separate.
    #[test]
    fn two_page_size_classes_over_one_va_are_not_conflated() {
        let prev = vec![r(0x20_0000, 0xA0_0000, 0x1000, 0, K4)];
        let cur = vec![
            r(0x20_0000, 0xA0_0000, 0x1000, 0, K4),
            r(0x20_0000, 0xF0_0000, 0x1_0000, 0, K64),
        ];
        let ops = diff(&prev, &cur);
        assert!(
            ops.iter().all(|o| !matches!(o, MapOp::Unmap(_))),
            "adding a 64K mapping must not UNMAP the 4K one at the same VA: {ops:#x?}"
        );
        assert_closes(&prev, &cur, "two classes at one VA");
    }

    /// Extending a run must `Map` only the new pages, and shrinking must `Unmap` only the tail —
    /// not `Remap` the whole extent.
    #[test]
    fn extending_and_shrinking_touch_only_the_difference() {
        let base = vec![r(0x1000, 0xA000, 0x2000, 0, K4)];
        let longer = vec![r(0x1000, 0xA000, 0x4000, 0, K4)];
        let ops = diff(&base, &longer);
        assert_eq!(ops.len(), 1, "extending should be one op: {ops:#x?}");
        assert!(matches!(ops[0], MapOp::Map(_)), "extending should MAP the tail");
        assert_eq!(ops[0].run().va, 0x3000, "and only the new pages");
        assert_closes(&base, &longer, "extend");
        assert_closes(&longer, &base, "shrink");
    }

    /// A deterministic pseudo-random mutation sequence, asserting closure at every step.
    /// ⊘ The vacuity guard matters: a sequence that never changes anything closes trivially.
    #[test]
    fn a_long_mutation_sequence_closes_at_every_step() {
        let mut seed = 0x243f_6a88_85a3_08d3u64;
        let mut rng = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut cur: Vec<Run> = Vec::new();
        let (mut changed, mut nonempty) = (0usize, 0usize);
        for step in 0..400 {
            let prev = cur.clone();
            let n = (rng() % 6) as usize;
            cur = (0..n)
                .map(|i| {
                    let class = if rng() % 3 == 0 { K64 } else { K4 };
                    let ps = class.bytes();
                    r(
                        (i as u64 + 1) * 0x10_0000 + (rng() % 4) * ps,
                        (rng() % 64) * ps,
                        ps * (1 + rng() % 3),
                        (rng() % 4) as u32,
                        class,
                    )
                })
                .collect();
            // ⊘ Drop overlaps within a class — the walk cannot produce them, so neither may this.
            cur.sort_by_key(|x| (x.class, x.va));
            cur.dedup_by(|b, a| a.class == b.class && b.va < a.va + a.len);
            if canonical(&prev) != canonical(&cur) {
                changed += 1;
            }
            if !diff(&prev, &cur).is_empty() {
                nonempty += 1;
            }
            assert_closes(&prev, &cur, &format!("step {step}"));
        }
        assert!(changed > 200, "vacuous: only {changed} steps changed the set");
        assert!(nonempty > 200, "vacuous: only {nonempty} steps produced ops");
    }

    /// ★★★ **THE KNOWN-POSITIVE.** A whole-run merge join — the w722 implementation — must FAIL
    /// closure on the shape above. Without this, a green suite would not prove the property is
    /// being tested at all.
    #[test]
    fn the_superseded_whole_run_merge_is_shown_to_break_closure() {
        // The bug, reproduced exactly: pair runs by start VA; emit Remap for a changed pair and
        // Unmap for every prev run the new one swallowed.
        fn broken(prev: &[Run], cur: &[Run]) -> Vec<MapOp> {
            let mut out = Vec::new();
            for c in cur {
                out.push(MapOp::Remap(*c));
            }
            for p in prev {
                if !cur.iter().any(|c| c.va == p.va && c.len == p.len) {
                    out.push(MapOp::Unmap(*p));
                }
            }
            out
        }
        let prev = vec![
            r(0x1000, 0xA000, 0x1000, 0, K4),
            r(0x2000, 0xB000, 0x1000, 0, K4),
            r(0x3000, 0xC000, 0x1000, 0, K4),
        ];
        let cur = vec![r(0x1000, 0x50000, 0x3000, 0, K4)];
        let got = canonical(&apply(&prev, &broken(&prev, &cur)));
        assert_ne!(
            got,
            canonical(&cur),
            "the known-positive did not fire: the whole-run merge must LOSE mappings here, or \
             this suite is not testing closure at all"
        );
        // And the real one closes on the same input.
        assert_closes(&prev, &cur, "known-positive control");
    }
}
